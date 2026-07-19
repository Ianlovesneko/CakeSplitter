//! Explicit, ignored 1 GiB validation profile for streamed desktop workflows.
//!
//! Run with `CAKESPLITTER_LARGE_TEST_SOURCE` set to an existing source file.

use std::{
    env,
    fs::{self, File},
    io::Read,
    mem::size_of,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use cakesplitter_core::{
    CancellationToken, CoreError, DirectoryFingerprint, PartialCheckpoint, ResumableMergeOptions,
    ResumableSplitOptions, SliceCheckpoint, SourceFingerprint, SplitCheckpointEvent,
    SplitResumeData, default_created_at, load_manifest, merge_package_resumable_with_progress,
    split_file_resumable_with_progress,
};
use cakesplitter_integrity::Sha256State;
use tempfile::tempdir_in;
use uuid::Uuid;

const GIB: u64 = 1024 * 1024 * 1024;
const SLICE_SIZE: u64 = 64 * 1024 * 1024;

#[derive(Default)]
struct SplitEvidence {
    source: Option<SourceFingerprint>,
    directory: Option<DirectoryFingerprint>,
    baseline: Option<String>,
    completed: Vec<SliceCheckpoint>,
    active: Option<PartialCheckpoint>,
}

#[test]
#[ignore = "explicit 1 GiB streamed validation profile"]
fn one_gib_split_cancel_recover_and_merge_remains_bounded() {
    let source = PathBuf::from(
        env::var_os("CAKESPLITTER_LARGE_TEST_SOURCE")
            .expect("set CAKESPLITTER_LARGE_TEST_SOURCE to the real 1 GiB fixture"),
    );
    let source = source.as_path();
    let source_size = source.metadata().unwrap().len();
    assert_eq!(
        source_size, GIB,
        "the release profile requires a real 1 GiB file"
    );

    let root = tempdir_in(source.parent().unwrap()).unwrap();
    let package = root.path().join("Package Output");
    let output = root.path().join("one-gib-profile-rebuilt.bin");
    fs::create_dir(&package).unwrap();
    let task_id = Uuid::new_v4().to_string();
    let package_id = Uuid::new_v4().to_string();
    let created_at = default_created_at();
    let token = CancellationToken::new();
    token.pause();
    let pause_token = token.clone();
    let pause_worker = thread::spawn(move || {
        assert!(pause_token.wait_until_paused(Duration::from_secs(20)));
        thread::sleep(Duration::from_millis(100));
        pause_token.resume();
    });
    let evidence = Arc::new(Mutex::new(SplitEvidence::default()));
    let callback_evidence = Arc::clone(&evidence);
    let cancel_token = token.clone();
    let total_started = Instant::now();
    let first_split_started = Instant::now();
    let first_result = split_file_resumable_with_progress(
        source,
        &ResumableSplitOptions {
            task_id: task_id.clone(),
            package_id: package_id.clone(),
            created_at: created_at.clone(),
            slice_size: SLICE_SIZE,
            output_dir: package.clone(),
            cancellation: token,
            resume: None,
        },
        |_| {},
        move |event| {
            let mut evidence = callback_evidence.lock().unwrap();
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
                SplitCheckpointEvent::PartialCreated { partial } => {
                    evidence.active = Some(partial);
                }
                SplitCheckpointEvent::SliceCompleted { checkpoint } => {
                    evidence.completed.push(checkpoint);
                    evidence.active = None;
                    if evidence.completed.len() == 2 {
                        cancel_token.cancel();
                    }
                }
                SplitCheckpointEvent::PartialCleared => evidence.active = None,
                SplitCheckpointEvent::ManifestPublished { .. } => evidence.active = None,
            }
        },
    );
    let first_split_seconds = first_split_started.elapsed().as_secs_f64();
    pause_worker.join().unwrap();
    assert!(matches!(first_result, Err(CoreError::Cancelled)));

    let snapshot = evidence.lock().unwrap();
    assert_eq!(snapshot.completed.len(), 2);
    let resume = SplitResumeData {
        source: snapshot.source.clone().unwrap(),
        output_directory: snapshot.directory.clone().unwrap(),
        baseline_sha256: snapshot.baseline.clone().unwrap(),
        completed: snapshot.completed.clone(),
        active_partial: snapshot.active.clone(),
        published_manifest_sha256: None,
    };
    drop(snapshot);

    let resumed_split_started = Instant::now();
    let manifest_path = split_file_resumable_with_progress(
        source,
        &ResumableSplitOptions {
            task_id,
            package_id,
            created_at,
            slice_size: SLICE_SIZE,
            output_dir: package,
            cancellation: CancellationToken::new(),
            resume: Some(resume),
        },
        |_| {},
        |_| {},
    )
    .unwrap();
    let resumed_split_seconds = resumed_split_started.elapsed().as_secs_f64();

    let merge_started = Instant::now();
    merge_package_resumable_with_progress(
        &manifest_path,
        &output,
        &ResumableMergeOptions {
            task_id: Uuid::new_v4().to_string(),
            cancellation: CancellationToken::new(),
            resume: None,
        },
        |_| {},
        |_| {},
    )
    .unwrap();
    let merge_seconds = merge_started.elapsed().as_secs_f64();

    let manifest = load_manifest(&manifest_path).unwrap();
    let rebuilt_hash = hash_file(&output);
    assert_eq!(output.metadata().unwrap().len(), source_size);
    assert_eq!(rebuilt_hash, manifest.original.sha256);
    assert_eq!(hash_file(source), rebuilt_hash);

    let peak_working_set = peak_working_set_bytes();
    println!("LARGE_PROFILE_SOURCE_SIZE={source_size}");
    println!("LARGE_PROFILE_SLICE_SIZE={SLICE_SIZE}");
    println!("LARGE_PROFILE_SLICE_COUNT={}", manifest.slice_count);
    println!("LARGE_PROFILE_FIRST_SPLIT_SECONDS={first_split_seconds:.3}");
    println!("LARGE_PROFILE_RESUMED_SPLIT_SECONDS={resumed_split_seconds:.3}");
    println!("LARGE_PROFILE_MERGE_SECONDS={merge_seconds:.3}");
    println!(
        "LARGE_PROFILE_TOTAL_SECONDS={:.3}",
        total_started.elapsed().as_secs_f64()
    );
    println!("LARGE_PROFILE_PEAK_WORKING_SET_BYTES={peak_working_set}");
    println!("LARGE_PROFILE_SHA256={rebuilt_hash}");
}

fn hash_file(path: &Path) -> String {
    let mut file = File::open(path).unwrap();
    let mut hasher = Sha256State::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).unwrap();
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    hasher.finish()
}

#[cfg(windows)]
#[repr(C)]
#[allow(dead_code)]
struct ProcessMemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> isize;
}

#[cfg(windows)]
#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemoryCounters, size: u32)
    -> i32;
}

#[cfg(windows)]
fn peak_working_set_bytes() -> usize {
    let mut counters = ProcessMemoryCounters {
        cb: size_of::<ProcessMemoryCounters>() as u32,
        page_fault_count: 0,
        peak_working_set_size: 0,
        working_set_size: 0,
        quota_peak_paged_pool_usage: 0,
        quota_paged_pool_usage: 0,
        quota_peak_non_paged_pool_usage: 0,
        quota_non_paged_pool_usage: 0,
        pagefile_usage: 0,
        peak_pagefile_usage: 0,
        private_usage: 0,
    };
    let result = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            size_of::<ProcessMemoryCounters>() as u32,
        )
    };
    assert_ne!(result, 0, "GetProcessMemoryInfo failed");
    counters.peak_working_set_size
}

#[cfg(not(windows))]
fn peak_working_set_bytes() -> usize {
    0
}
