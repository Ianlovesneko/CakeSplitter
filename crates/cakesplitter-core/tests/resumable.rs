use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use cakesplitter_core::{
    CancellationToken, CoreError, DirectoryFingerprint, MergeCheckpointEvent, MergeResumeData,
    PartialCheckpoint, ResumableMergeOptions, ResumableSplitOptions, SliceCheckpoint,
    SourceFingerprint, SplitCheckpointEvent, SplitOptions, default_created_at,
    merge_package_resumable_with_progress, planned_slice_range, split_file,
    split_file_resumable_with_progress,
};
use tempfile::tempdir;
use uuid::Uuid;

#[derive(Default)]
struct SplitEvidence {
    source: Option<SourceFingerprint>,
    directory: Option<DirectoryFingerprint>,
    baseline: Option<String>,
    completed: Vec<SliceCheckpoint>,
    active: Option<PartialCheckpoint>,
}

#[test]
fn planner_handles_offsets_above_four_gibibytes_without_allocating_the_file() {
    const GIB: u64 = 1024 * 1024 * 1024;
    let total_size = 5 * GIB + 123;

    assert_eq!(
        planned_slice_range(total_size, GIB, 5).unwrap(),
        (4 * GIB, GIB)
    );
    assert_eq!(
        planned_slice_range(total_size, GIB, 6).unwrap(),
        (5 * GIB, 123)
    );
}

#[test]
fn planner_rejects_unsafe_arithmetic_and_unbounded_slice_plans() {
    assert!(planned_slice_range(u64::MAX, 1, 1).is_err());
    assert!(planned_slice_range(1, 0, 1).is_err());
    assert!(planned_slice_range(50_001, 1, 1).is_err());
    assert!(planned_slice_range(5, 2, 0).is_err());
    assert!(planned_slice_range(5, 2, 4).is_err());
}

#[test]
fn split_resumes_at_a_verified_slice_boundary_without_rewriting_completed_slices() {
    let root = tempdir().unwrap();
    let source = root.path().join("resume-source.bin");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    fs::write(&source, (0..96_u8).collect::<Vec<_>>()).unwrap();
    let task_id = Uuid::new_v4().to_string();
    let package_id = Uuid::new_v4().to_string();
    let token = CancellationToken::new();
    let evidence = Arc::new(Mutex::new(SplitEvidence::default()));
    let evidence_for_callback = Arc::clone(&evidence);
    let cancellation = token.clone();

    let result = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id: task_id.clone(),
            package_id: package_id.clone(),
            created_at: default_created_at(),
            slice_size: 16,
            output_dir: package.clone(),
            cancellation: token,
            resume: None,
        },
        move |progress| {
            if progress.bytes_processed > 16 {
                cancellation.cancel();
            }
        },
        move |event| update_split_evidence(&evidence_for_callback, event),
    );
    assert!(matches!(result, Err(CoreError::Cancelled)));
    assert!(!package.join("resume-source.bin.cake.json").exists());
    let before = evidence.lock().unwrap();
    assert_eq!(before.completed.len(), 1);
    assert!(before.active.is_some());
    let first_slice = package.join(&before.completed[0].entry.filename);
    let first_identity = before.completed[0].identity;
    drop(before);

    let snapshot = evidence.lock().unwrap();
    let resume = cakesplitter_core::SplitResumeData {
        source: snapshot.source.clone().unwrap(),
        output_directory: snapshot.directory.clone().unwrap(),
        baseline_sha256: snapshot.baseline.clone().unwrap(),
        completed: snapshot.completed.clone(),
        active_partial: snapshot.active.clone(),
        published_manifest_sha256: None,
    };
    drop(snapshot);
    let manifest = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id,
            package_id,
            created_at: default_created_at(),
            slice_size: 16,
            output_dir: package.clone(),
            cancellation: CancellationToken::new(),
            resume: Some(resume),
        },
        |_| {},
        |_| {},
    )
    .unwrap();
    assert!(manifest.exists());
    let completed_first = cakesplitter_core::fingerprint_file(&first_slice).unwrap();
    assert_eq!(completed_first.identity, first_identity);
}

#[test]
fn split_resume_rejects_a_changed_source() {
    let root = tempdir().unwrap();
    let source = root.path().join("changed.bin");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    fs::write(&source, vec![b'A'; 64]).unwrap();
    let task_id = Uuid::new_v4().to_string();
    let token = CancellationToken::new();
    let cancellation = token.clone();
    let evidence = Arc::new(Mutex::new(SplitEvidence::default()));
    let callback_evidence = Arc::clone(&evidence);
    let result = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id: task_id.clone(),
            package_id: Uuid::new_v4().to_string(),
            created_at: default_created_at(),
            slice_size: 16,
            output_dir: package.clone(),
            cancellation: token,
            resume: None,
        },
        move |progress| {
            if progress.bytes_processed > 16 {
                cancellation.cancel();
            }
        },
        move |event| update_split_evidence(&callback_evidence, event),
    );
    assert!(matches!(result, Err(CoreError::Cancelled)));
    let mut bytes = fs::read(&source).unwrap();
    bytes[0] ^= 0xff;
    fs::write(&source, bytes).unwrap();
    let snapshot = evidence.lock().unwrap();
    let resume = cakesplitter_core::SplitResumeData {
        source: snapshot.source.clone().unwrap(),
        output_directory: snapshot.directory.clone().unwrap(),
        baseline_sha256: snapshot.baseline.clone().unwrap(),
        completed: snapshot.completed.clone(),
        active_partial: snapshot.active.clone(),
        published_manifest_sha256: None,
    };
    drop(snapshot);
    let result = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id,
            package_id: Uuid::new_v4().to_string(),
            created_at: default_created_at(),
            slice_size: 16,
            output_dir: package,
            cancellation: CancellationToken::new(),
            resume: Some(resume),
        },
        |_| {},
        |_| {},
    );
    assert!(matches!(result, Err(CoreError::ResumeRejected(_))));
}

#[test]
fn merge_resumes_at_a_verified_slice_boundary_and_rebuilds_exact_bytes() {
    let root = tempdir().unwrap();
    let source = root.path().join("merge-source.bin");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let original = (0..100_u8).collect::<Vec<_>>();
    fs::write(&source, &original).unwrap();
    let manifest = split_file(
        &source,
        &SplitOptions {
            slice_size: 16,
            output_dir: package,
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let output = root.path().join("rebuilt.bin");
    let task_id = Uuid::new_v4().to_string();
    let token = CancellationToken::new();
    let cancellation = token.clone();
    let merge_evidence = Arc::new(Mutex::new(None::<MergeResumeData>));
    let callback_evidence = Arc::clone(&merge_evidence);
    let result = merge_package_resumable_with_progress(
        &manifest,
        &output,
        &ResumableMergeOptions {
            task_id: task_id.clone(),
            cancellation: token,
            resume: None,
        },
        move |progress| {
            if progress.bytes_processed > 16 {
                cancellation.cancel();
            }
        },
        move |event| update_merge_evidence(&callback_evidence, event),
    );
    assert!(matches!(result, Err(CoreError::Cancelled)));
    assert!(!output.exists());
    let resume = merge_evidence.lock().unwrap().clone().unwrap();
    assert!(resume.completed_slices >= 1);
    assert!(resume.completed_slices < original.len().div_ceil(16) as u64);
    assert_eq!(resume.completed_bytes, resume.completed_slices * 16);

    merge_package_resumable_with_progress(
        &manifest,
        &output,
        &ResumableMergeOptions {
            task_id,
            cancellation: CancellationToken::new(),
            resume: Some(resume),
        },
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(output).unwrap(), original);
}

#[test]
fn pause_is_acknowledged_and_cancel_wakes_a_paused_operation() {
    let token = CancellationToken::new();
    token.pause();
    let worker_token = token.clone();
    let worker = thread::spawn(move || {
        let root = tempdir().unwrap();
        let source = root.path().join("paused.bin");
        let package = root.path().join("package");
        fs::create_dir(&package).unwrap();
        fs::write(&source, vec![0x55; 2 * 1024 * 1024]).unwrap();
        split_file(
            &source,
            &SplitOptions {
                slice_size: 1024 * 1024,
                output_dir: package,
                cancellation: worker_token,
            },
        )
    });
    assert!(token.wait_until_paused(Duration::from_secs(5)));
    token.cancel();
    assert!(matches!(worker.join().unwrap(), Err(CoreError::Cancelled)));
}

#[test]
fn split_reconciles_an_atomic_manifest_publish_before_or_after_its_checkpoint() {
    let root = tempdir().unwrap();
    let source = root.path().join("commit-window.bin");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    fs::write(&source, (0..64_u8).collect::<Vec<_>>()).unwrap();
    let task_id = Uuid::new_v4().to_string();
    let package_id = Uuid::new_v4().to_string();
    let created_at = default_created_at();
    let mut events = Vec::new();
    let manifest = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id: task_id.clone(),
            package_id: package_id.clone(),
            created_at: created_at.clone(),
            slice_size: 16,
            output_dir: package.clone(),
            cancellation: CancellationToken::new(),
            resume: None,
        },
        |_| {},
        |event| events.push(event),
    )
    .unwrap();

    let (source_fingerprint, output_directory, baseline_sha256) = events
        .iter()
        .find_map(|event| match event {
            SplitCheckpointEvent::Baseline {
                source,
                output_directory,
                baseline_sha256,
            } => Some((
                source.clone(),
                output_directory.clone(),
                baseline_sha256.clone(),
            )),
            _ => None,
        })
        .unwrap();
    let completed = events
        .iter()
        .filter_map(|event| match event {
            SplitCheckpointEvent::SliceCompleted { checkpoint } => Some(checkpoint.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let manifest_partial = events
        .iter()
        .filter_map(|event| match event {
            SplitCheckpointEvent::PartialCreated { partial }
                if partial.filename.contains(".cake.json.") =>
            {
                Some(partial.clone())
            }
            _ => None,
        })
        .next_back()
        .unwrap();
    let published_sha256 = events
        .iter()
        .find_map(|event| match event {
            SplitCheckpointEvent::ManifestPublished { sha256, .. } => Some(sha256.clone()),
            _ => None,
        })
        .unwrap();

    for resume in [
        cakesplitter_core::SplitResumeData {
            source: source_fingerprint.clone(),
            output_directory: output_directory.clone(),
            baseline_sha256: baseline_sha256.clone(),
            completed: completed.clone(),
            active_partial: Some(manifest_partial),
            published_manifest_sha256: None,
        },
        cakesplitter_core::SplitResumeData {
            source: source_fingerprint.clone(),
            output_directory: output_directory.clone(),
            baseline_sha256: baseline_sha256.clone(),
            completed: completed.clone(),
            active_partial: None,
            published_manifest_sha256: Some(published_sha256.clone()),
        },
    ] {
        let recovered = split_file_resumable_with_progress(
            &source,
            &ResumableSplitOptions {
                task_id: task_id.clone(),
                package_id: package_id.clone(),
                created_at: created_at.clone(),
                slice_size: 16,
                output_dir: package.clone(),
                cancellation: CancellationToken::new(),
                resume: Some(resume),
            },
            |_| {},
            |_| {},
        )
        .unwrap();
        assert_eq!(recovered, manifest);
    }
}

#[test]
fn merge_reconciles_an_atomic_output_publish_before_its_checkpoint() {
    let root = tempdir().unwrap();
    let source = root.path().join("merge-window.bin");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let original = (0..64_u8).collect::<Vec<_>>();
    fs::write(&source, &original).unwrap();
    let manifest = split_file(
        &source,
        &SplitOptions {
            slice_size: 16,
            output_dir: package,
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let output = root.path().join("rebuilt-window.bin");
    let task_id = Uuid::new_v4().to_string();
    let mut events = Vec::new();
    merge_package_resumable_with_progress(
        &manifest,
        &output,
        &ResumableMergeOptions {
            task_id: task_id.clone(),
            cancellation: CancellationToken::new(),
            resume: None,
        },
        |_| {},
        |event| events.push(event),
    )
    .unwrap();
    let (output_directory, partial) = events
        .iter()
        .find_map(|event| match event {
            MergeCheckpointEvent::PartialCreated {
                output_directory,
                partial,
            } => Some((output_directory.clone(), partial.clone())),
            _ => None,
        })
        .unwrap();
    let (completed_slices, completed_bytes) = events
        .iter()
        .filter_map(|event| match event {
            MergeCheckpointEvent::SliceBoundary {
                completed_slices,
                completed_bytes,
            } => Some((*completed_slices, *completed_bytes)),
            _ => None,
        })
        .next_back()
        .unwrap();
    merge_package_resumable_with_progress(
        &manifest,
        &output,
        &ResumableMergeOptions {
            task_id,
            cancellation: CancellationToken::new(),
            resume: Some(MergeResumeData {
                output_directory,
                partial,
                completed_slices,
                completed_bytes,
                published_sha256: None,
            }),
        },
        |_| {},
        |_| {},
    )
    .unwrap();
    assert_eq!(fs::read(output).unwrap(), original);
}

#[test]
fn incomplete_cleanup_refuses_to_delete_a_replaced_file() {
    let root = tempdir().unwrap();
    let path = root.path().join("owned.partial");
    fs::write(&path, b"owned").unwrap();
    let identity = cakesplitter_core::fingerprint_file(&path).unwrap().identity;
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"replacement").unwrap();
    let result = cakesplitter_core::remove_owned_incomplete_file(&path, identity);
    assert!(matches!(result, Err(CoreError::StagedIdentityChanged(_))));
    assert_eq!(fs::read(path).unwrap(), b"replacement");
}

#[derive(Clone, Copy)]
enum RebindBoundary {
    BeforeFirstSlice,
    BetweenSlices,
    BeforeManifestPublication,
}

#[test]
fn output_directory_rebinding_fails_closed_at_all_publication_boundaries() {
    for boundary in [
        RebindBoundary::BeforeFirstSlice,
        RebindBoundary::BetweenSlices,
        RebindBoundary::BeforeManifestPublication,
    ] {
        assert_output_rebind_fails_closed(boundary);
    }
}

#[test]
fn parent_directory_rebinding_or_inaccessibility_fails_closed() {
    let root = tempdir().unwrap();
    let parent = root.path().join("selected-parent");
    let output = parent.join("selected-output");
    let moved = root.path().join("selected-parent-original");
    fs::create_dir_all(&output).unwrap();
    let source = root.path().join("source.bin");
    fs::write(&source, vec![0x51; 3 * 1024 * 1024]).unwrap();
    let cancellation = CancellationToken::new();
    let callback_cancellation = cancellation.clone();
    let attempted = Arc::new(AtomicBool::new(false));
    let callback_attempted = Arc::clone(&attempted);
    let callback_parent = parent.clone();
    let callback_output = output.clone();
    let callback_moved = moved.clone();

    let result = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id: Uuid::new_v4().to_string(),
            package_id: Uuid::new_v4().to_string(),
            created_at: default_created_at(),
            slice_size: 1024 * 1024,
            output_dir: output.clone(),
            cancellation,
            resume: None,
        },
        |_| {},
        move |event| {
            if matches!(event, SplitCheckpointEvent::Baseline { .. })
                && !callback_attempted.swap(true, Ordering::SeqCst)
            {
                if fs::rename(&callback_parent, &callback_moved).is_ok() {
                    fs::create_dir_all(&callback_output).unwrap();
                }
                callback_cancellation.cancel();
            }
        },
    );

    assert!(attempted.load(Ordering::SeqCst));
    assert!(matches!(
        result,
        Err(CoreError::Cancelled | CoreError::DestinationIdentityChanged(_))
    ));
    assert!(!output.join("source.bin.cake.json").exists());
    if output.exists() && moved.exists() {
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    }
}

#[test]
fn destination_that_becomes_inaccessible_before_execution_fails_with_identity_error() {
    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let output = root.path().join("selected-output");
    fs::write(&source, b"source").unwrap();
    fs::create_dir(&output).unwrap();
    fs::remove_dir(&output).unwrap();

    let result = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id: Uuid::new_v4().to_string(),
            package_id: Uuid::new_v4().to_string(),
            created_at: default_created_at(),
            slice_size: 2,
            output_dir: output,
            cancellation: CancellationToken::new(),
            resume: None,
        },
        |_| {},
        |_| {},
    );
    assert!(matches!(
        result,
        Err(CoreError::DestinationIdentityChanged(_))
    ));
}

#[test]
fn merge_output_parent_rebinding_cannot_publish_to_replacement() {
    let root = tempdir().unwrap();
    let source = root.path().join("merge-source.bin");
    let package = root.path().join("package");
    let selected_parent = root.path().join("selected-parent");
    let moved_parent = root.path().join("selected-parent-original");
    fs::write(&source, vec![0x4d; 2 * 1024 * 1024]).unwrap();
    fs::create_dir(&package).unwrap();
    fs::create_dir(&selected_parent).unwrap();
    let manifest = split_file(
        &source,
        &SplitOptions {
            slice_size: 1024 * 1024,
            output_dir: package,
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let output = selected_parent.join("rebuilt.bin");
    let cancellation = CancellationToken::new();
    let callback_cancellation = cancellation.clone();
    let attempted = Arc::new(AtomicBool::new(false));
    let callback_attempted = Arc::clone(&attempted);
    let callback_parent = selected_parent.clone();
    let callback_moved = moved_parent.clone();

    let result = merge_package_resumable_with_progress(
        &manifest,
        &output,
        &ResumableMergeOptions {
            task_id: Uuid::new_v4().to_string(),
            cancellation,
            resume: None,
        },
        |_| {},
        move |event| {
            if matches!(event, MergeCheckpointEvent::PartialCreated { .. })
                && !callback_attempted.swap(true, Ordering::SeqCst)
            {
                if fs::rename(&callback_parent, &callback_moved).is_ok() {
                    fs::create_dir(&callback_parent).unwrap();
                }
                callback_cancellation.cancel();
            }
        },
    );
    assert!(attempted.load(Ordering::SeqCst));
    assert!(matches!(
        result,
        Err(CoreError::Cancelled | CoreError::DestinationIdentityChanged(_))
    ));
    assert!(!output.exists());
    if moved_parent.exists() {
        assert_eq!(fs::read_dir(&selected_parent).unwrap().count(), 0);
    }
}

#[test]
fn recovery_refuses_a_rebound_output_directory_without_publishing() {
    let root = tempdir().unwrap();
    let source = root.path().join("resume-source.bin");
    let output = root.path().join("selected-output");
    let moved = root.path().join("selected-output-original");
    fs::write(&source, vec![0x52; 2 * 1024 * 1024]).unwrap();
    fs::create_dir(&output).unwrap();
    let evidence = Arc::new(Mutex::new(SplitEvidence::default()));
    let callback_evidence = Arc::clone(&evidence);
    let cancellation = CancellationToken::new();
    let callback_cancellation = cancellation.clone();
    let task_id = Uuid::new_v4().to_string();
    let package_id = Uuid::new_v4().to_string();

    let first = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id: task_id.clone(),
            package_id: package_id.clone(),
            created_at: default_created_at(),
            slice_size: 1024 * 1024,
            output_dir: output.clone(),
            cancellation,
            resume: None,
        },
        |_| {},
        move |event| {
            update_split_evidence(&callback_evidence, event);
            callback_cancellation.cancel();
        },
    );
    assert!(matches!(first, Err(CoreError::Cancelled)));
    let evidence = evidence.lock().unwrap();
    let resume = cakesplitter_core::SplitResumeData {
        source: evidence.source.clone().unwrap(),
        output_directory: evidence.directory.clone().unwrap(),
        baseline_sha256: evidence.baseline.clone().unwrap(),
        completed: evidence.completed.clone(),
        active_partial: evidence.active.clone(),
        published_manifest_sha256: None,
    };
    drop(evidence);
    fs::rename(&output, &moved).unwrap();
    fs::create_dir(&output).unwrap();

    let recovered = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id,
            package_id,
            created_at: default_created_at(),
            slice_size: 1024 * 1024,
            output_dir: output.clone(),
            cancellation: CancellationToken::new(),
            resume: Some(resume),
        },
        |_| {},
        |_| {},
    );
    assert!(matches!(
        recovered,
        Err(CoreError::DestinationIdentityChanged(_))
    ));
    assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    assert!(!output.join("resume-source.bin.cake.json").exists());
}

#[cfg(windows)]
#[test]
fn preexisting_directory_reparse_point_is_rejected() {
    use std::os::windows::fs::symlink_dir;

    let root = tempdir().unwrap();
    let source = root.path().join("source.bin");
    let real = root.path().join("real-output");
    let selected = root.path().join("selected-output");
    fs::write(&source, b"source").unwrap();
    fs::create_dir(&real).unwrap();
    match symlink_dir(&real, &selected) {
        Ok(()) => {
            let result = split_file_resumable_with_progress(
                &source,
                &ResumableSplitOptions {
                    task_id: Uuid::new_v4().to_string(),
                    package_id: Uuid::new_v4().to_string(),
                    created_at: default_created_at(),
                    slice_size: 2,
                    output_dir: selected,
                    cancellation: CancellationToken::new(),
                    resume: None,
                },
                |_| {},
                |_| {},
            );
            assert!(matches!(result, Err(CoreError::UnsafeFilesystemPath(_))));
            assert_eq!(fs::read_dir(real).unwrap().count(), 0);
        }
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            // The platform itself denied creation of the unsafe test fixture.
        }
        Err(error) => panic!("failed to create reparse-point fixture: {error}"),
    }
}

fn assert_output_rebind_fails_closed(boundary: RebindBoundary) {
    let root = tempdir().unwrap();
    let source = root.path().join("private-source.bin");
    let output = root.path().join("selected-output");
    let moved = root.path().join("selected-output-original");
    fs::write(&source, vec![0x53; 3 * 1024 * 1024]).unwrap();
    fs::create_dir(&output).unwrap();
    let cancellation = CancellationToken::new();
    let callback_cancellation = cancellation.clone();
    let attempted = Arc::new(AtomicBool::new(false));
    let callback_attempted = Arc::clone(&attempted);
    let callback_output = output.clone();
    let callback_moved = moved.clone();

    let result = split_file_resumable_with_progress(
        &source,
        &ResumableSplitOptions {
            task_id: Uuid::new_v4().to_string(),
            package_id: Uuid::new_v4().to_string(),
            created_at: default_created_at(),
            slice_size: 1024 * 1024,
            output_dir: output.clone(),
            cancellation,
            resume: None,
        },
        |_| {},
        move |event| {
            let trigger = match boundary {
                RebindBoundary::BeforeFirstSlice => {
                    matches!(event, SplitCheckpointEvent::Baseline { .. })
                }
                RebindBoundary::BetweenSlices => {
                    matches!(event, SplitCheckpointEvent::SliceCompleted { .. })
                }
                RebindBoundary::BeforeManifestPublication => matches!(
                    event,
                    SplitCheckpointEvent::PartialCreated { ref partial }
                        if partial.filename.contains(".cake.json.")
                ),
            };
            if trigger && !callback_attempted.swap(true, Ordering::SeqCst) {
                if fs::rename(&callback_output, &callback_moved).is_ok() {
                    fs::create_dir(&callback_output).unwrap();
                }
                callback_cancellation.cancel();
            }
        },
    );

    assert!(attempted.load(Ordering::SeqCst));
    assert!(matches!(
        result,
        Err(CoreError::Cancelled | CoreError::DestinationIdentityChanged(_))
    ));
    assert!(!output.join("private-source.bin.cake.json").exists());
    if moved.exists() {
        assert_eq!(fs::read_dir(&output).unwrap().count(), 0);
    }
}

fn update_split_evidence(evidence: &Mutex<SplitEvidence>, event: SplitCheckpointEvent) {
    let mut evidence = evidence.lock().unwrap();
    match event {
        SplitCheckpointEvent::Baseline {
            source,
            output_directory,
            baseline_sha256,
        } => {
            evidence.source = Some(source);
            evidence.directory = Some(output_directory);
            evidence.baseline = Some(baseline_sha256);
        }
        SplitCheckpointEvent::PartialCreated { partial } => evidence.active = Some(partial),
        SplitCheckpointEvent::SliceCompleted { checkpoint } => {
            evidence.completed.push(checkpoint);
            evidence.active = None;
        }
        SplitCheckpointEvent::PartialCleared => evidence.active = None,
        SplitCheckpointEvent::ManifestPublished { .. } => evidence.active = None,
    }
}

fn update_merge_evidence(evidence: &Mutex<Option<MergeResumeData>>, event: MergeCheckpointEvent) {
    let mut evidence = evidence.lock().unwrap();
    match event {
        MergeCheckpointEvent::PartialCreated {
            output_directory,
            partial,
        } => {
            *evidence = Some(MergeResumeData {
                output_directory,
                partial,
                completed_slices: 0,
                completed_bytes: 0,
                published_sha256: None,
            });
        }
        MergeCheckpointEvent::SliceBoundary {
            completed_slices,
            completed_bytes,
        } => {
            let value = evidence.as_mut().unwrap();
            value.completed_slices = completed_slices;
            value.completed_bytes = completed_bytes;
            value.partial.verified_bytes = completed_bytes;
        }
        MergeCheckpointEvent::Published { .. } => {}
    }
}
