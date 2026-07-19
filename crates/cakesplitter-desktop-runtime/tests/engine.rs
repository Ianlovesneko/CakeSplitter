use std::{
    fs, thread,
    time::{Duration, Instant},
};

use cakesplitter_desktop_runtime::{EngineError, StoreError, TaskEngine, TaskSnapshot, TaskStatus};
use tempfile::tempdir;

fn wait_for(engine: &TaskEngine, id: &str, wanted: TaskStatus) -> TaskSnapshot {
    let deadline = Instant::now() + Duration::from_secs(15);
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
