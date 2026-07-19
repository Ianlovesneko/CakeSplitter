use std::{
    fs, thread,
    time::{Duration, Instant},
};

use cakesplitter_core::{
    CancellationToken, MAX_PACKAGE_DIAGNOSTIC_ENTRIES, SplitOptions, capture_package_binding,
    split_file,
};
use cakesplitter_desktop_runtime::{EngineError, StoreError, TaskEngine, TaskSnapshot, TaskStatus};
use cakesplitter_desktop_runtime::{ProcessingPlan, TaskRecord, TaskSpec, TaskStore};
use tempfile::tempdir;

fn wait_for(engine: &TaskEngine, id: &str, wanted: TaskStatus) -> TaskSnapshot {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let task = engine
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|task| task.id == id)
            .unwrap();
        if task.status == wanted {
            return task;
        }
        assert!(
            !matches!(
                task.status,
                TaskStatus::Failed | TaskStatus::PermissionRequired
            ),
            "task failed unexpectedly: {:?}",
            task.failure
        );
        assert!(Instant::now() < deadline, "task did not reach {wanted:?}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn queue_streams_split_and_merge_through_the_shared_rust_core() {
    let root = tempdir().unwrap();
    let app_data = root.path().join("app-data");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let source = root.path().join("desktop-source.bin");
    let original = (0..=255_u8).cycle().take(16 * 1024).collect::<Vec<_>>();
    fs::write(&source, &original).unwrap();

    let engine = TaskEngine::open(&app_data, "0.4.0-dev", |_| {}).unwrap();
    let split = engine.enqueue_split(source, package.clone(), 1024).unwrap();
    let completed_split = wait_for(&engine, &split.id, TaskStatus::Completed);
    assert_eq!(
        completed_split.progress.bytes_processed,
        original.len() as u64
    );
    let manifest = package.join("desktop-source.bin.cake.json");
    assert!(manifest.is_file());

    let rebuilt = root.path().join("desktop-rebuilt.bin");
    let merge = engine.enqueue_merge(manifest, rebuilt.clone()).unwrap();
    wait_for(&engine, &merge.id, TaskStatus::Completed);
    assert_eq!(fs::read(rebuilt).unwrap(), original);
}

#[test]
fn clear_all_remains_empty_after_a_queued_worker_observes_the_new_epoch() {
    let root = tempdir().unwrap();
    let app_data = root.path().join("app-data");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let source = root.path().join("queued.bin");
    fs::write(&source, vec![0x5a; 8 * 1024 * 1024]).unwrap();
    let engine = TaskEngine::open(&app_data, "0.4.0-dev", |_| {}).unwrap();
    engine.enqueue_split(source, package, 1024 * 1024).unwrap();
    engine.clear_all().unwrap();
    thread::sleep(Duration::from_millis(200));
    assert!(engine.list_tasks().unwrap().is_empty());
}

#[test]
fn active_pause_cancel_and_slice_boundary_retry_complete_without_stale_partials() {
    let root = tempdir().unwrap();
    let app_data = root.path().join("app-data");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let source = root.path().join("pause-recovery.bin");
    let file = fs::File::create(&source).unwrap();
    file.set_len(128 * 1024 * 1024).unwrap();
    drop(file);

    let engine = TaskEngine::open(&app_data, "0.4.0-dev", |_| {}).unwrap();
    let task = engine
        .enqueue_split(source, package.clone(), 1024 * 1024)
        .unwrap();
    wait_for(&engine, &task.id, TaskStatus::Running);
    let paused = engine.pause_task(&task.id).unwrap();
    assert_eq!(paused.status, TaskStatus::Paused);
    let resumed = engine.resume_task(&task.id).unwrap();
    assert_eq!(resumed.status, TaskStatus::Running);

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let current = engine
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|candidate| candidate.id == task.id)
            .unwrap();
        if current.status == TaskStatus::Running && current.progress.bytes_processed > 0 {
            break;
        }
        assert_ne!(
            current.status,
            TaskStatus::Completed,
            "task completed before cancellation checkpoint"
        );
        assert!(
            Instant::now() < deadline,
            "task did not reach a persisted Slice checkpoint"
        );
        thread::sleep(Duration::from_millis(10));
    }
    engine.cancel_task(&task.id).unwrap();
    wait_for(&engine, &task.id, TaskStatus::Cancelled);
    assert!(!package.join("pause-recovery.bin.cake.json").exists());

    engine.retry_task(&task.id).unwrap();
    wait_for(&engine, &task.id, TaskStatus::Completed);
    assert!(package.join("pause-recovery.bin.cake.json").is_file());
    assert!(fs::read_dir(&package).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".partial")
    }));
}

#[test]
fn interrupted_task_reopens_and_recovers_from_a_persisted_slice_boundary() {
    let root = tempdir().unwrap();
    let app_data = root.path().join("app-data");
    let package = root.path().join("recovery-package");
    fs::create_dir(&package).unwrap();
    let source = root.path().join("restart-recovery.bin");
    fs::write(&source, vec![0x3c; 128 * 1024 * 1024]).unwrap();

    let engine = TaskEngine::open(&app_data, "0.4.0-dev", |_| {}).unwrap();
    let task = engine
        .enqueue_split(source, package.clone(), 1024 * 1024)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let current = engine.store().get(&task.id).unwrap();
        if current.status == TaskStatus::Running && current.progress.bytes_processed > 0 {
            break;
        }
        assert_ne!(current.status, TaskStatus::Completed);
        assert!(
            Instant::now() < deadline,
            "task did not reach a recovery boundary"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let interrupted = engine.interrupt_all().unwrap();
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].status, TaskStatus::Interrupted);
    drop(engine);

    let reopen_deadline = Instant::now() + Duration::from_secs(20);
    let reopened = loop {
        match TaskEngine::open(&app_data, "0.4.0-dev", |_| {}) {
            Ok(engine) => break engine,
            Err(EngineError::Store(StoreError::ActiveWriter)) => {
                assert!(
                    Instant::now() < reopen_deadline,
                    "worker lock was not released"
                );
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("unexpected reopen failure: {error}"),
        }
    };
    assert_eq!(
        reopened.store().get(&task.id).unwrap().status,
        TaskStatus::Interrupted
    );
    reopened.retry_task(&task.id).unwrap();
    wait_for(&reopened, &task.id, TaskStatus::Completed);
    assert!(package.join("restart-recovery.bin.cake.json").is_file());
}

#[test]
fn queued_merge_rejects_package_rebinding_and_retry_keeps_the_original_binding() {
    let root = tempdir().unwrap();
    let package_a = root.path().join("package-a");
    let package_b = root.path().join("package-b");
    let blocker_package = root.path().join("blocker-package");
    fs::create_dir(&package_a).unwrap();
    fs::create_dir(&package_b).unwrap();
    fs::create_dir(&blocker_package).unwrap();
    let source_a = root.path().join("source-a").join("same.bin");
    let source_b = root.path().join("source-b").join("same.bin");
    fs::create_dir_all(source_a.parent().unwrap()).unwrap();
    fs::create_dir_all(source_b.parent().unwrap()).unwrap();
    let bytes_a = vec![0x41; 96];
    fs::write(&source_a, &bytes_a).unwrap();
    fs::write(&source_b, vec![0x42; 96]).unwrap();
    let manifest_a = split_file(
        &source_a,
        &SplitOptions {
            slice_size: 32,
            output_dir: package_a.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    split_file(
        &source_b,
        &SplitOptions {
            slice_size: 32,
            output_dir: package_b.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();

    let blocker = root.path().join("blocker.bin");
    let blocker_file = fs::File::create(&blocker).unwrap();
    blocker_file.set_len(128 * 1024 * 1024).unwrap();
    drop(blocker_file);
    let app_data = root.path().join("app-data");
    let engine = TaskEngine::open(&app_data, "0.4.0-dev", |_| {}).unwrap();
    let active = engine
        .enqueue_split(blocker, blocker_package, 1024 * 1024)
        .unwrap();
    wait_for(&engine, &active.id, TaskStatus::Running);
    engine.pause_task(&active.id).unwrap();

    let output = root.path().join("rebuilt.bin");
    let queued = engine
        .enqueue_merge(manifest_a.clone(), output.clone())
        .unwrap();
    let original_binding = match engine.store().get(&queued.id).unwrap().spec {
        TaskSpec::Merge {
            package_binding, ..
        } => package_binding,
        _ => unreachable!(),
    };
    let stash = root.path().join("package-a-stash");
    fs::create_dir(&stash).unwrap();
    move_entries(&package_a, &stash);
    copy_entries(&package_b, &package_a);

    engine.cancel_task(&active.id).unwrap();
    wait_for(&engine, &active.id, TaskStatus::Cancelled);
    let failed = wait_for(&engine, &queued.id, TaskStatus::Failed);
    assert_eq!(
        failed.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("package_identity_changed")
    );
    assert!(!output.exists());
    let retained_binding = match engine.store().get(&queued.id).unwrap().spec {
        TaskSpec::Merge {
            package_binding, ..
        } => package_binding,
        _ => unreachable!(),
    };
    assert_eq!(retained_binding, original_binding);

    remove_entries(&package_a);
    move_entries(&stash, &package_a);
    engine.retry_task(&queued.id).unwrap();
    wait_for(&engine, &queued.id, TaskStatus::Completed);
    assert_eq!(fs::read(output).unwrap(), bytes_a);
}

#[test]
fn startup_verify_rejects_same_name_replacement_and_stable_original_recovers() {
    let root = tempdir().unwrap();
    let app_data = root.path().join("app-data");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let source = root.path().join("restart-bind.bin");
    fs::write(&source, vec![0x51; 64]).unwrap();
    let manifest = split_file(
        &source,
        &SplitOptions {
            slice_size: 16,
            output_dir: package.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let binding = capture_package_binding(&manifest, &CancellationToken::new()).unwrap();
    let store = TaskStore::open(&app_data).unwrap();
    let mut record = TaskRecord::new(
        "0.4.0-dev",
        store.epoch().unwrap(),
        binding.manifest.original.filename.clone(),
        None,
        TaskSpec::Verify {
            manifest_path: manifest.clone(),
            package_binding: binding.clone(),
        },
        ProcessingPlan {
            total_bytes: binding.manifest.original.size,
            slice_size: binding.manifest.target_slice_size,
            slice_count: binding.manifest.slice_count,
            required_free_bytes: 0,
        },
    );
    record.transition(TaskStatus::Queued).unwrap();
    let queued = store.insert(record).unwrap();
    drop(store);

    let slice = package.join(&binding.manifest.slices[0].filename);
    let original = root.path().join("original.slice");
    fs::rename(&slice, &original).unwrap();
    fs::write(&slice, vec![0x51; 16]).unwrap();
    let engine = TaskEngine::open(&app_data, "0.4.0-dev", |_| {}).unwrap();
    let failed = wait_for(&engine, &queued.id, TaskStatus::Failed);
    assert_eq!(
        failed.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("package_identity_changed")
    );
    fs::remove_file(&slice).unwrap();
    fs::rename(&original, &slice).unwrap();
    engine.retry_task(&queued.id).unwrap();
    let completed = wait_for(&engine, &queued.id, TaskStatus::Completed);
    assert!(matches!(
        completed.result,
        Some(cakesplitter_desktop_runtime::TaskResult::Inspection { .. })
    ));
}

#[test]
fn excessive_inspection_diagnostics_fail_without_persisting_or_emitting_a_result() {
    let root = tempdir().unwrap();
    let app_data = root.path().join("app-data");
    let package = root.path().join("package");
    fs::create_dir(&package).unwrap();
    let source = root.path().join("bounded-inspection.bin");
    fs::write(&source, b"bounded").unwrap();
    let manifest = split_file(
        &source,
        &SplitOptions {
            slice_size: 1024,
            output_dir: package.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    for index in 0..=MAX_PACKAGE_DIAGNOSTIC_ENTRIES {
        fs::write(package.join(format!("unexpected-{index:04}.slice")), b"x").unwrap();
    }
    let engine = TaskEngine::open(&app_data, "0.4.0-dev", |_| {}).unwrap();
    let task = engine.enqueue_inspect(manifest, false).unwrap();
    let failed = wait_for(&engine, &task.id, TaskStatus::Failed);
    assert_eq!(
        failed.failure.as_ref().map(|failure| failure.code.as_str()),
        Some("package_enumeration_limit")
    );
    assert!(failed.result.is_none());
    assert!(engine.store().get(&task.id).unwrap().result.is_none());
}

fn move_entries(from: &std::path::Path, to: &std::path::Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        fs::rename(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn copy_entries(from: &std::path::Path, to: &std::path::Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

fn remove_entries(directory: &std::path::Path) {
    for entry in fs::read_dir(directory).unwrap() {
        fs::remove_file(entry.unwrap().path()).unwrap();
    }
}
