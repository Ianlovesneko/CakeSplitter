use std::{
    fs,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use cakesplitter_core::{CancellationToken, SplitOptions, split_file};
use cakesplitter_desktop_runtime::{
    ConflictClass, ConflictType, EngineError, QueueDirection, RecoveryAction, StoreError,
    TaskEngine, TaskFailure, TaskPriority, TaskSnapshot, TaskStatus,
};
use tempfile::{TempDir, tempdir};

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
        thread::sleep(Duration::from_millis(10));
    }
}

fn paused_blocker(root: &TempDir, engine: &TaskEngine) -> TaskSnapshot {
    let source = root.path().join("scheduler-blocker.bin");
    let source_file = fs::File::create(&source).unwrap();
    source_file.set_len(128 * 1024 * 1024).unwrap();
    drop(source_file);
    let output = root.path().join("scheduler-blocker-package");
    fs::create_dir(&output).unwrap();
    let task = engine.enqueue_split(source, output, 1024 * 1024).unwrap();
    wait_for(engine, &task.id, TaskStatus::Running);
    engine.pause_task(&task.id).unwrap()
}

fn small_source(root: &TempDir, name: &str, byte: u8) -> std::path::PathBuf {
    let path = root.path().join(name);
    fs::write(&path, vec![byte; 64 * 1024]).unwrap();
    path
}

fn output_directory(root: &TempDir, name: &str) -> std::path::PathBuf {
    let path = root.path().join(name);
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn real_worker_honors_priority_fifo_and_emits_no_duplicate_execution() {
    let root = tempdir().unwrap();
    let completed = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = Arc::clone(&completed);
    let engine = TaskEngine::open(
        root.path().join("app-data").as_path(),
        "0.5.0",
        move |task| {
            if task.status == TaskStatus::Completed {
                observed.lock().unwrap().push(task.id);
            }
        },
    )
    .unwrap();
    let blocker = paused_blocker(&root, &engine);

    let low = engine
        .enqueue_split_with_priority(
            small_source(&root, "low.bin", 0x11),
            output_directory(&root, "low-package"),
            16 * 1024,
            TaskPriority::Low,
        )
        .unwrap();
    let high = engine
        .enqueue_split_with_priority(
            small_source(&root, "high.bin", 0x22),
            output_directory(&root, "high-package"),
            16 * 1024,
            TaskPriority::High,
        )
        .unwrap();
    let normal = engine
        .enqueue_split_with_priority(
            small_source(&root, "normal.bin", 0x33),
            output_directory(&root, "normal-package"),
            16 * 1024,
            TaskPriority::Normal,
        )
        .unwrap();

    let queued = engine.list_tasks().unwrap();
    assert_eq!(
        queued
            .iter()
            .find(|task| task.id == high.id)
            .unwrap()
            .queue_position,
        Some(1)
    );
    assert_eq!(
        queued
            .iter()
            .find(|task| task.id == normal.id)
            .unwrap()
            .queue_position,
        Some(2)
    );
    assert_eq!(
        queued
            .iter()
            .find(|task| task.id == low.id)
            .unwrap()
            .queue_position,
        Some(3)
    );

    engine.resume_task(&blocker.id).unwrap();
    wait_for(&engine, &low.id, TaskStatus::Completed);
    let order = completed.lock().unwrap().clone();
    let filtered = order
        .into_iter()
        .filter(|id| id != &blocker.id)
        .collect::<Vec<_>>();
    assert_eq!(filtered, vec![high.id, normal.id, low.id]);
    let snapshots = engine.list_tasks().unwrap();
    for task in snapshots {
        assert!(task.attempt_count <= 1, "task executed more than once");
    }
}

#[test]
fn queued_priority_reorder_and_cancel_use_the_authoritative_engine() {
    let root = tempdir().unwrap();
    let engine = TaskEngine::open(root.path().join("app-data").as_path(), "0.5.0", |_| {}).unwrap();
    let blocker = paused_blocker(&root, &engine);
    let first = engine
        .enqueue_split(
            small_source(&root, "first.bin", 1),
            output_directory(&root, "first-package"),
            16 * 1024,
        )
        .unwrap();
    let second = engine
        .enqueue_split(
            small_source(&root, "second.bin", 2),
            output_directory(&root, "second-package"),
            16 * 1024,
        )
        .unwrap();
    engine
        .reorder_task(&second.id, QueueDirection::Earlier)
        .unwrap();
    let second = engine
        .set_task_priority(&second.id, TaskPriority::High)
        .unwrap();
    assert_eq!(second.priority, TaskPriority::High);
    let cancelled = engine.cancel_task(&first.id).unwrap();
    assert_eq!(cancelled.status, TaskStatus::Cancelled);

    engine.resume_task(&blocker.id).unwrap();
    wait_for(&engine, &second.id, TaskStatus::Completed);
    thread::sleep(Duration::from_millis(50));
    assert!(
        !root
            .path()
            .join("first-package/first.bin.cake.json")
            .exists()
    );
    assert_eq!(
        engine
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|task| task.id == first.id)
            .unwrap()
            .attempt_count,
        0
    );
}

#[test]
fn preflight_classifies_duplicate_shared_output_and_cross_over_conflicts() {
    let root = tempdir().unwrap();
    let engine = TaskEngine::open(root.path().join("app-data").as_path(), "0.5.0", |_| {}).unwrap();
    let _blocker = paused_blocker(&root, &engine);
    let source = small_source(&root, "selected.bin", 3);
    let output = output_directory(&root, "selected-package");
    let queued = engine
        .enqueue_split(source.clone(), output.clone(), 16 * 1024)
        .unwrap();

    let duplicate = engine.preflight_split(&source, &output, 16 * 1024).unwrap();
    assert!(duplicate.conflicts.iter().any(|conflict| {
        conflict.conflicting_task_id == queued.id
            && conflict.class == ConflictClass::DuplicateTask
            && conflict.conflict_type == ConflictType::DuplicateOperation
    }));

    let shared_output = output_directory(&root, "shared-input-package");
    let shared = engine
        .preflight_split(&source, &shared_output, 32 * 1024)
        .unwrap();
    assert!(shared.conflicts.iter().any(|conflict| {
        conflict.class == ConflictClass::InformationalOverlap
            && conflict.conflict_type == ConflictType::SharedInput
    }));

    let other_source = small_source(&root, "other.bin", 4);
    let overlapping = engine
        .preflight_split(&other_source, &output, 16 * 1024)
        .unwrap();
    assert!(overlapping.conflicts.iter().any(|conflict| {
        conflict.class == ConflictClass::HardConflict
            && conflict.conflict_type == ConflictType::OverlappingOutput
    }));

    let source_inside_existing_output = output.join("future-source.bin");
    fs::write(&source_inside_existing_output, b"future").unwrap();
    let cross_over = engine
        .preflight_split(
            &source_inside_existing_output,
            &output_directory(&root, "cross-over-output"),
            1024,
        )
        .unwrap();
    assert!(cross_over.conflicts.iter().any(|conflict| {
        conflict.class == ConflictClass::HardConflict
            && conflict.conflict_type == ConflictType::SourceUsedAsDestination
    }));
}

#[test]
fn package_overlap_is_informational_but_destination_inside_package_is_blocked() {
    let root = tempdir().unwrap();
    let package = output_directory(&root, "cake-package");
    let source = small_source(&root, "package-source.bin", 5);
    let manifest = split_file(
        &source,
        &SplitOptions {
            slice_size: 16 * 1024,
            output_dir: package.clone(),
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let engine = TaskEngine::open(root.path().join("app-data").as_path(), "0.5.0", |_| {}).unwrap();
    let _blocker = paused_blocker(&root, &engine);
    engine.enqueue_inspect(manifest.clone(), false).unwrap();

    let merge_output = root.path().join("rebuilt.bin");
    let overlap = engine.preflight_merge(&manifest, &merge_output).unwrap();
    assert!(overlap.conflicts.iter().any(|conflict| {
        conflict.class == ConflictClass::InformationalOverlap
            && conflict.conflict_type == ConflictType::SharedInput
    }));

    let inside = package.join("unsafe-rebuilt.bin");
    let blocked = engine.preflight_merge(&manifest, &inside).unwrap();
    assert!(blocked.conflicts.iter().any(|conflict| {
        conflict.class == ConflictClass::HardConflict
            && conflict.conflict_type == ConflictType::DestinationInsidePackage
    }));
}

#[test]
fn retry_archives_failure_without_rebinding_and_nonretryable_failure_is_rejected() {
    let root = tempdir().unwrap();
    let engine = TaskEngine::open(root.path().join("app-data").as_path(), "0.5.0", |_| {}).unwrap();
    let _blocker = paused_blocker(&root, &engine);
    let task = engine
        .enqueue_split(
            small_source(&root, "retry.bin", 6),
            output_directory(&root, "retry-package"),
            16 * 1024,
        )
        .unwrap();
    let failed = engine
        .store()
        .mutate(&task.id, engine.store().epoch().unwrap(), |record| {
            record.transition(TaskStatus::Running).unwrap();
            record.attempt_count = 1;
            record.failure = Some(TaskFailure::classified(
                "sharing_violation",
                "Close the conflicting local application and retry.",
                "The file is temporarily locked.",
                cakesplitter_desktop_runtime::ErrorCategory::Permission,
                true,
                RecoveryAction::CloseConflictingApplication,
                1,
            ));
            record.transition(TaskStatus::Failed).unwrap();
            Ok(())
        })
        .unwrap();
    let retried = engine.retry_task(&failed.id).unwrap();
    assert_eq!(retried.status, TaskStatus::Queued);
    assert!(retried.failure.is_none());
    assert_eq!(retried.failure_history.len(), 1);
    assert_eq!(retried.attempt_count, 1);

    let nonretryable = engine
        .store()
        .mutate(&retried.id, engine.store().epoch().unwrap(), |record| {
            record.transition(TaskStatus::Running).unwrap();
            record.failure = Some(TaskFailure::classified(
                "slice_corrupted",
                "The Slice failed integrity verification.",
                "The Slice hash did not match.",
                cakesplitter_desktop_runtime::ErrorCategory::Integrity,
                false,
                RecoveryAction::None,
                2,
            ));
            record.transition(TaskStatus::Failed).unwrap();
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        engine.retry_task(&nonretryable.id),
        Err(EngineError::RetryNotAllowed)
    ));
}

#[test]
fn scheduler_shutdown_releases_the_store_and_restart_runs_queued_work_once() {
    let root = tempdir().unwrap();
    let app_data = root.path().join("app-data");
    let engine = TaskEngine::open(&app_data, "0.5.0", |_| {}).unwrap();
    let blocker = paused_blocker(&root, &engine);
    let queued = engine
        .enqueue_split(
            small_source(&root, "restart-queued.bin", 0x77),
            output_directory(&root, "restart-queued-package"),
            16 * 1024,
        )
        .unwrap();
    let interrupted = engine.interrupt_all().unwrap();
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].id, blocker.id);
    drop(engine);

    let deadline = Instant::now() + Duration::from_secs(20);
    let reopened = loop {
        match TaskEngine::open(&app_data, "0.5.0", |_| {}) {
            Ok(engine) => break engine,
            Err(EngineError::Store(StoreError::ActiveWriter)) => {
                assert!(
                    Instant::now() < deadline,
                    "scheduler did not release the store lock"
                );
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("unexpected restart error: {error}"),
        }
    };
    assert_eq!(
        reopened.store().get(&blocker.id).unwrap().status,
        TaskStatus::Interrupted
    );
    let completed = wait_for(&reopened, &queued.id, TaskStatus::Completed);
    assert_eq!(completed.attempt_count, 1);
    assert!(
        root.path()
            .join("restart-queued-package/restart-queued.bin.cake.json")
            .is_file()
    );
}
