use std::{
    collections::HashSet,
    fs,
    mem::size_of,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use cakesplitter_core::{CancellationToken, SplitOptions, split_file};
use cakesplitter_desktop_runtime::{EngineError, TaskEngine, TaskPriority, TaskStatus};
use tempfile::{TempDir, tempdir};

const ENDURANCE_TASKS: usize = 25;

fn wait_for_status(engine: &TaskEngine, id: &str, wanted: TaskStatus) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let task = engine
            .list_tasks()
            .unwrap()
            .into_iter()
            .find(|task| task.id == id)
            .unwrap();
        if task.status == wanted {
            return;
        }
        assert!(Instant::now() < deadline, "task did not reach {wanted:?}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn create_source(root: &TempDir, name: &str, bytes: usize, value: u8) -> std::path::PathBuf {
    let path = root.path().join(name);
    fs::write(&path, vec![value; bytes]).unwrap();
    path
}

fn create_directory(root: &TempDir, name: &str) -> std::path::PathBuf {
    let path = root.path().join(name);
    fs::create_dir(&path).unwrap();
    path
}

#[test]
fn controlled_25_task_queue_endurance_remains_bounded_and_correct() {
    let root = tempdir().unwrap();
    let started = Instant::now();
    let stop_monitor = Arc::new(AtomicBool::new(false));
    let peak_handles = Arc::new(AtomicU32::new(process_handle_count()));
    let monitor_stop = Arc::clone(&stop_monitor);
    let monitor_peak = Arc::clone(&peak_handles);
    let monitor = thread::spawn(move || {
        while !monitor_stop.load(Ordering::Acquire) {
            monitor_peak.fetch_max(process_handle_count(), Ordering::AcqRel);
            thread::sleep(Duration::from_millis(5));
        }
    });

    let package_a = create_directory(&root, "prebuilt-a");
    let package_b = create_directory(&root, "prebuilt-b");
    let manifest_a = split_file(
        &create_source(&root, "prebuilt-a.bin", 128 * 1024, 0xa1),
        &SplitOptions {
            slice_size: 16 * 1024,
            output_dir: package_a,
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();
    let manifest_b = split_file(
        &create_source(&root, "prebuilt-b.bin", 96 * 1024, 0xb2),
        &SplitOptions {
            slice_size: 12 * 1024,
            output_dir: package_b,
            cancellation: CancellationToken::new(),
        },
    )
    .unwrap();

    let engine = TaskEngine::open(root.path().join("app-data").as_path(), "0.5.0", |_| {}).unwrap();
    let blocker_source = root.path().join("blocker.bin");
    let blocker_file = fs::File::create(&blocker_source).unwrap();
    blocker_file.set_len(128 * 1024 * 1024).unwrap();
    drop(blocker_file);
    let blocker = engine
        .enqueue_split(
            blocker_source,
            create_directory(&root, "blocker-package"),
            1024 * 1024,
        )
        .unwrap();
    wait_for_status(&engine, &blocker.id, TaskStatus::Running);
    let pause_started = Instant::now();
    engine.pause_task(&blocker.id).unwrap();
    let pause_ack_millis = pause_started.elapsed().as_millis();

    let mut admitted_ids = vec![blocker.id.clone()];
    let mut split_tasks = Vec::new();
    for index in 0..18 {
        let bytes = [1024, 64 * 1024, 1024 * 1024][index % 3];
        let priority = [TaskPriority::High, TaskPriority::Normal, TaskPriority::Low][index % 3];
        let task = engine
            .enqueue_split_with_priority(
                create_source(
                    &root,
                    &format!("endurance-{index:02}.bin"),
                    bytes,
                    index as u8,
                ),
                create_directory(&root, &format!("endurance-package-{index:02}")),
                16 * 1024,
                priority,
            )
            .unwrap();
        admitted_ids.push(task.id.clone());
        split_tasks.push(task);
    }
    let operations = [
        engine.enqueue_inspect(manifest_a.clone(), false).unwrap(),
        engine.enqueue_verify(manifest_a.clone()).unwrap(),
        engine.enqueue_inspect(manifest_b.clone(), false).unwrap(),
        engine.enqueue_verify(manifest_b.clone()).unwrap(),
        engine
            .enqueue_merge(manifest_a, root.path().join("endurance-rebuilt-a.bin"))
            .unwrap(),
        engine
            .enqueue_merge(manifest_b, root.path().join("endurance-rebuilt-b.bin"))
            .unwrap(),
    ];
    admitted_ids.extend(operations.iter().map(|task| task.id.clone()));
    assert_eq!(admitted_ids.len(), ENDURANCE_TASKS);
    assert_eq!(
        admitted_ids.iter().collect::<HashSet<_>>().len(),
        ENDURANCE_TASKS
    );

    let cancelled_then_restarted = &split_tasks[0];
    engine.cancel_task(&cancelled_then_restarted.id).unwrap();
    engine.resume_task(&cancelled_then_restarted.id).unwrap();
    let remains_cancelled = &split_tasks[1];
    engine.cancel_task(&remains_cancelled.id).unwrap();

    let duplicate_started = Instant::now();
    for _ in 0..50 {
        assert!(matches!(
            engine.enqueue_split(
                match &engine.store().get(&split_tasks[2].id).unwrap().spec {
                    cakesplitter_desktop_runtime::TaskSpec::Split { source_path, .. } =>
                        source_path.clone(),
                    _ => unreachable!(),
                },
                match &engine.store().get(&split_tasks[2].id).unwrap().spec {
                    cakesplitter_desktop_runtime::TaskSpec::Split {
                        output_directory, ..
                    } => output_directory.clone(),
                    _ => unreachable!(),
                },
                16 * 1024,
            ),
            Err(EngineError::TaskConflict(_))
        ));
    }
    let rejected_admission_micros = duplicate_started.elapsed().as_micros() / 50;
    assert_eq!(
        engine.store().nonterminal_count().unwrap(),
        ENDURANCE_TASKS - 1
    );

    let state_update_started = Instant::now();
    for index in 0..100 {
        engine
            .store()
            .mutate(
                &split_tasks[2].id,
                engine.store().epoch().unwrap(),
                |record| {
                    record.progress.stage = format!("Queued endurance update {index}");
                    Ok(())
                },
            )
            .unwrap();
    }
    let state_update_mean_micros = state_update_started.elapsed().as_micros() / 100;

    let diagnostic_started = Instant::now();
    let diagnostics = create_directory(&root, "diagnostics");
    let diagnostics_identity = cakesplitter_core::fingerprint_directory(&diagnostics).unwrap();
    let diagnostic = engine
        .export_diagnostics(&diagnostics, &diagnostics_identity)
        .unwrap();
    let diagnostic_millis = diagnostic_started.elapsed().as_millis();
    assert!(diagnostic.path.is_dir());

    let resume_started = Instant::now();
    engine.resume_task(&blocker.id).unwrap();
    let resume_millis = resume_started.elapsed().as_millis();
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let tasks = engine.list_tasks().unwrap();
        if tasks.iter().all(|task| task.status.is_terminal()) {
            break;
        }
        assert!(Instant::now() < deadline, "endurance queue did not drain");
        thread::sleep(Duration::from_millis(20));
    }

    let tasks = engine.list_tasks().unwrap();
    assert_eq!(tasks.len(), ENDURANCE_TASKS);
    assert_eq!(
        tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Completed)
            .count(),
        24
    );
    assert_eq!(
        tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Cancelled)
            .count(),
        1
    );
    assert!(tasks.iter().all(|task| task.attempt_count <= 1));
    assert_eq!(
        tasks
            .iter()
            .find(|task| task.id == cancelled_then_restarted.id)
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    assert_eq!(engine.store().nonterminal_count().unwrap(), 0);
    assert_eq!(count_partial_files(root.path()), 0);
    let storage = engine.storage_summary().unwrap();
    assert_eq!(storage.terminal_history_tasks, ENDURANCE_TASKS as u64);
    assert_eq!(storage.nonterminal_tasks, 0);

    stop_monitor.store(true, Ordering::Release);
    monitor.join().unwrap();
    println!(
        "ENDURANCE_METRICS tasks={} duration_ms={} peak_working_set_bytes={} peak_handles={} database_bytes={} rejected_admission_mean_us={} state_update_mean_us={} diagnostic_ms={} pause_ack_ms={} resume_ms={} partial_files=0 orphan_records=0",
        ENDURANCE_TASKS,
        started.elapsed().as_millis(),
        peak_working_set_bytes(),
        peak_handles.load(Ordering::Acquire),
        storage.database_bytes,
        rejected_admission_micros,
        state_update_mean_micros,
        diagnostic_millis,
        pause_ack_millis,
        resume_millis,
    );
}

fn count_partial_files(directory: &Path) -> usize {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| {
            if path.is_dir() {
                count_partial_files(&path)
            } else {
                usize::from(path.to_string_lossy().ends_with(".partial"))
            }
        })
        .sum()
}

#[cfg(windows)]
#[repr(C)]
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
    fn GetProcessHandleCount(process: isize, count: *mut u32) -> i32;
}

#[cfg(windows)]
#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemoryCounters, size: u32)
    -> i32;
}

#[cfg(windows)]
fn process_handle_count() -> u32 {
    let mut count = 0;
    let result = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
    assert_ne!(result, 0, "GetProcessHandleCount failed");
    count
}

#[cfg(not(windows))]
fn process_handle_count() -> u32 {
    0
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
