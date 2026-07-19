use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use cakesplitter_core::{
    CancellationToken, CoreError, MAX_PACKAGE_INSPECTION_SERIALIZED_BYTES, MergeCheckpointEvent,
    MergeResumeData, PackageBinding, Progress, ResumableMergeOptions, ResumableSplitOptions,
    SplitCheckpointEvent, SplitResumeData, capture_package_binding, default_created_at,
    inspect_package_bound, merge_package_resumable_bound_with_progress,
    remove_owned_incomplete_file, split_file_resumable_with_progress, validate_existing_directory,
    validate_existing_regular_file,
};
use cakesplitter_format::{
    MAX_SAFE_INTEGER, MAX_SLICE_COUNT, expected_slice_count, validate_portable_filename,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    MAX_QUEUED_TASKS,
    model::{
        DesktopPreferences, InspectionSummary, ProcessingPlan, RecoveryCheckpoint, TaskFailure,
        TaskProgress, TaskRecord, TaskResult, TaskSnapshot, TaskSpec, TaskStatus,
    },
    store::{StoreError, TaskStore},
};

const DISK_SPACE_MARGIN_BYTES: u64 = 16 * 1024 * 1024;
const PAUSE_ACK_TIMEOUT: Duration = Duration::from_secs(10);
const PROGRESS_WRITE_INTERVAL: Duration = Duration::from_millis(250);

type Listener = dyn Fn(TaskSnapshot) + Send + Sync + 'static;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("task ID is invalid")]
    InvalidTaskId,
    #[error("task is not active")]
    NotActive,
    #[error("task pause was not acknowledged within the safe timeout")]
    PauseTimeout,
    #[error("slice size must be between 1 and {MAX_SAFE_INTEGER} bytes")]
    InvalidSliceSize,
    #[error("planned Slice count exceeds the supported maximum of {MAX_SLICE_COUNT}")]
    SliceLimit,
    #[error("required space is {required} bytes but only {available} bytes are available")]
    InsufficientSpace { required: u64, available: u64 },
    #[error("task command is not valid in the current state")]
    InvalidState,
    #[error("task queue is unavailable")]
    QueueUnavailable,
    #[error("active tasks did not stop within the safe cleanup timeout")]
    TasksStopping,
}

#[derive(Clone)]
pub struct TaskEngine {
    inner: Arc<EngineInner>,
    sender: mpsc::SyncSender<String>,
}

struct EngineInner {
    store: Arc<TaskStore>,
    application_version: String,
    controls: Mutex<HashMap<String, CancellationToken>>,
    admission: Mutex<()>,
    listener: Arc<Listener>,
}

impl TaskEngine {
    pub fn open(
        app_data_directory: &Path,
        application_version: impl Into<String>,
        listener: impl Fn(TaskSnapshot) + Send + Sync + 'static,
    ) -> Result<Self, EngineError> {
        let store = Arc::new(TaskStore::open(app_data_directory)?);
        store.recover_after_restart()?;
        let (sender, receiver) = mpsc::sync_channel(MAX_QUEUED_TASKS);
        let inner = Arc::new(EngineInner {
            store,
            application_version: application_version.into(),
            controls: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            listener: Arc::new(listener),
        });
        let worker = Arc::clone(&inner);
        thread::Builder::new()
            .name("cakesplitter-task-worker".to_owned())
            .spawn(move || worker_loop(worker, receiver))
            .map_err(|_| EngineError::QueueUnavailable)?;
        let engine = Self { inner, sender };
        for record in engine.inner.store.list()? {
            if record.status == TaskStatus::Queued {
                engine.send(&record.id)?;
            }
        }
        Ok(engine)
    }

    pub fn store(&self) -> &TaskStore {
        &self.inner.store
    }

    pub fn startup_recovery_report(&self) -> crate::model::StartupRecoveryReport {
        self.inner.store.startup_recovery_report()
    }

    pub fn plan_split(
        &self,
        source_path: &Path,
        output_directory: &Path,
        slice_size: u64,
    ) -> Result<ProcessingPlan, EngineError> {
        validate_existing_regular_file(source_path)?;
        validate_existing_directory(output_directory)?;
        if slice_size == 0 || slice_size > MAX_SAFE_INTEGER {
            return Err(EngineError::InvalidSliceSize);
        }
        let total_bytes = source_path
            .metadata()
            .map_err(|source| CoreError::Io {
                path: source_path.to_path_buf(),
                source,
            })?
            .len();
        if total_bytes > MAX_SAFE_INTEGER {
            return Err(EngineError::InvalidSliceSize);
        }
        let slice_count = expected_slice_count(total_bytes, slice_size);
        if slice_count > MAX_SLICE_COUNT {
            return Err(EngineError::SliceLimit);
        }
        let required_free_bytes = total_bytes
            .checked_add(DISK_SPACE_MARGIN_BYTES)
            .ok_or(EngineError::InvalidSliceSize)?;
        Ok(ProcessingPlan {
            total_bytes,
            slice_size,
            slice_count,
            required_free_bytes,
        })
    }

    pub fn enqueue_split(
        &self,
        source_path: PathBuf,
        output_directory: PathBuf,
        slice_size: u64,
    ) -> Result<TaskSnapshot, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.store.ensure_admission_available()?;
        let plan = self.plan_split(&source_path, &output_directory, slice_size)?;
        ensure_space(&output_directory, plan.required_free_bytes)?;
        let display_name = filename(&source_path)?;
        let destination_name = directory_label(&output_directory);
        let epoch = self.inner.store.epoch()?;
        let spec = TaskSpec::Split {
            source_path,
            output_directory,
            slice_size,
            package_id: Uuid::new_v4().to_string(),
            created_at: default_created_at(),
        };
        self.enqueue_record(TaskRecord::new(
            &self.inner.application_version,
            epoch,
            display_name,
            destination_name,
            spec,
            plan,
        ))
    }

    pub fn enqueue_merge(
        &self,
        manifest_path: PathBuf,
        output_path: PathBuf,
    ) -> Result<TaskSnapshot, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.store.ensure_admission_available()?;
        let package_binding = capture_package_binding(&manifest_path, &CancellationToken::new())?;
        let manifest = &package_binding.manifest;
        let parent = output_path
            .parent()
            .ok_or_else(|| CoreError::UnsafeFilesystemPath(output_path.clone()))?;
        validate_existing_directory(parent)?;
        validate_portable_filename(
            output_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(CoreError::NonUtf8Filename)?,
        )
        .map_err(CoreError::from)?;
        let required_free_bytes = manifest
            .original
            .size
            .checked_add(DISK_SPACE_MARGIN_BYTES)
            .ok_or(EngineError::InvalidSliceSize)?;
        ensure_space(parent, required_free_bytes)?;
        let plan = ProcessingPlan {
            total_bytes: manifest.original.size,
            slice_size: manifest.target_slice_size,
            slice_count: manifest.slice_count,
            required_free_bytes,
        };
        let epoch = self.inner.store.epoch()?;
        let display_name = manifest.original.filename.clone();
        let destination_name = Some(filename(&output_path)?);
        let spec = TaskSpec::Merge {
            manifest_path,
            output_path,
            package_binding,
        };
        self.enqueue_record(TaskRecord::new(
            &self.inner.application_version,
            epoch,
            display_name,
            destination_name,
            spec,
            plan,
        ))
    }

    pub fn enqueue_inspect(
        &self,
        manifest_path: PathBuf,
        verify_hashes: bool,
    ) -> Result<TaskSnapshot, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.store.ensure_admission_available()?;
        let package_binding = capture_package_binding(&manifest_path, &CancellationToken::new())?;
        let epoch = self.inner.store.epoch()?;
        let display_name = package_binding.manifest.original.filename.clone();
        let plan = ProcessingPlan {
            total_bytes: package_binding.manifest.original.size,
            slice_size: package_binding.manifest.target_slice_size,
            slice_count: package_binding.manifest.slice_count,
            required_free_bytes: 0,
        };
        self.enqueue_record(TaskRecord::new(
            &self.inner.application_version,
            epoch,
            display_name,
            None,
            TaskSpec::Inspect {
                manifest_path,
                verify_hashes,
                package_binding,
            },
            plan,
        ))
    }

    pub fn enqueue_verify(&self, manifest_path: PathBuf) -> Result<TaskSnapshot, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.store.ensure_admission_available()?;
        let package_binding = capture_package_binding(&manifest_path, &CancellationToken::new())?;
        let epoch = self.inner.store.epoch()?;
        let display_name = package_binding.manifest.original.filename.clone();
        let plan = ProcessingPlan {
            total_bytes: package_binding.manifest.original.size,
            slice_size: package_binding.manifest.target_slice_size,
            slice_count: package_binding.manifest.slice_count,
            required_free_bytes: 0,
        };
        self.enqueue_record(TaskRecord::new(
            &self.inner.application_version,
            epoch,
            display_name,
            None,
            TaskSpec::Verify {
                manifest_path,
                package_binding,
            },
            plan,
        ))
    }

    pub fn list_tasks(&self) -> Result<Vec<TaskSnapshot>, EngineError> {
        Ok(self
            .inner
            .store
            .list()?
            .into_iter()
            .map(|record| record.snapshot())
            .collect())
    }

    pub fn pause_task(&self, task_id: &str) -> Result<TaskSnapshot, EngineError> {
        validate_task_id(task_id)?;
        let token = self.active_token(task_id)?;
        let record = self.inner.store.get(task_id)?;
        let paused = self.update(task_id, record.epoch, |task| {
            task.transition(TaskStatus::Pausing)
                .map_err(|_| StoreError::InvalidTransition)
        })?;
        token.pause();
        if !token.wait_until_paused(PAUSE_ACK_TIMEOUT) {
            return Err(EngineError::PauseTimeout);
        }
        let paused = self.update(task_id, paused.epoch, |task| {
            task.transition(TaskStatus::Paused)
                .map_err(|_| StoreError::InvalidTransition)
        })?;
        Ok(paused.snapshot())
    }

    pub fn resume_task(&self, task_id: &str) -> Result<TaskSnapshot, EngineError> {
        validate_task_id(task_id)?;
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = self.inner.store.get(task_id)?;
        if record.status == TaskStatus::Paused {
            let token = self.active_token(task_id)?;
            let resuming = self.update(task_id, record.epoch, |task| {
                task.transition(TaskStatus::Resuming)
                    .map_err(|_| StoreError::InvalidTransition)
            })?;
            token.resume();
            let running = self.update(task_id, resuming.epoch, |task| {
                task.transition(TaskStatus::Running)
                    .map_err(|_| StoreError::InvalidTransition)
            })?;
            return Ok(running.snapshot());
        }
        if matches!(
            record.status,
            TaskStatus::Interrupted
                | TaskStatus::PermissionRequired
                | TaskStatus::Failed
                | TaskStatus::Cancelled
        ) {
            if record.status.is_terminal() {
                self.inner.store.ensure_admission_available()?;
            }
            let queued = self.update(task_id, record.epoch, |task| {
                task.failure = None;
                task.transition(TaskStatus::Queued)
                    .map_err(|_| StoreError::InvalidTransition)
            })?;
            self.send(task_id)?;
            return Ok(queued.snapshot());
        }
        Err(EngineError::InvalidState)
    }

    pub fn retry_task(&self, task_id: &str) -> Result<TaskSnapshot, EngineError> {
        self.resume_task(task_id)
    }

    pub fn cancel_task(&self, task_id: &str) -> Result<TaskSnapshot, EngineError> {
        validate_task_id(task_id)?;
        let record = self.inner.store.get(task_id)?;
        if let Ok(token) = self.active_token(task_id) {
            let cancelling = self.update(task_id, record.epoch, |task| {
                task.transition(TaskStatus::Cancelling)
                    .map_err(|_| StoreError::InvalidTransition)
            })?;
            token.cancel();
            return Ok(cancelling.snapshot());
        }
        let cancelled = self.update(task_id, record.epoch, |task| {
            task.transition(TaskStatus::Cancelled)
                .map_err(|_| StoreError::InvalidTransition)
        })?;
        Ok(cancelled.snapshot())
    }

    pub fn remove_task(&self, task_id: &str) -> Result<(), EngineError> {
        validate_task_id(task_id)?;
        let record = self.inner.store.get(task_id)?;
        if !record.status.is_terminal() {
            return Err(EngineError::InvalidState);
        }
        cleanup_incomplete(&record)?;
        self.inner.store.remove_terminal(task_id)?;
        Ok(())
    }

    pub fn clear_all(&self) -> Result<(), EngineError> {
        let controls = self
            .inner
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for token in controls.values() {
            token.cancel();
        }
        drop(controls);
        let deadline = Instant::now() + PAUSE_ACK_TIMEOUT;
        while !self.active_tasks().is_empty() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if !self.active_tasks().is_empty() {
            return Err(EngineError::TasksStopping);
        }
        for record in self.inner.store.list()? {
            if record.status != TaskStatus::Completed {
                cleanup_incomplete(&record)?;
            }
        }
        self.inner.store.clear_all()?;
        Ok(())
    }

    pub fn active_tasks(&self) -> Vec<String> {
        self.inner
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    pub fn preferences(&self) -> Result<DesktopPreferences, EngineError> {
        Ok(self.inner.store.preferences()?)
    }

    pub fn save_preferences(
        &self,
        preferences: &DesktopPreferences,
    ) -> Result<DesktopPreferences, EngineError> {
        if preferences.default_slice_size == 0 || preferences.default_slice_size > MAX_SAFE_INTEGER
        {
            return Err(EngineError::InvalidSliceSize);
        }
        Ok(self.inner.store.save_preferences(preferences)?)
    }

    pub fn interrupt_all(&self) -> Result<Vec<TaskSnapshot>, EngineError> {
        let controls = self
            .inner
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|(id, token)| (id.clone(), token.clone()))
            .collect::<Vec<_>>();
        let mut interrupted = Vec::with_capacity(controls.len());
        for (id, token) in controls {
            let record = self.inner.store.get(&id)?;
            if matches!(
                record.status,
                TaskStatus::Running
                    | TaskStatus::Pausing
                    | TaskStatus::Paused
                    | TaskStatus::Resuming
                    | TaskStatus::Cancelling
            ) {
                let record = self.update(&id, record.epoch, |task| {
                    task.transition(TaskStatus::Interrupted)
                        .map_err(|_| StoreError::InvalidTransition)
                })?;
                interrupted.push(record.snapshot());
            }
            token.cancel();
        }
        Ok(interrupted)
    }

    fn enqueue_record(&self, mut record: TaskRecord) -> Result<TaskSnapshot, EngineError> {
        record
            .transition(TaskStatus::Queued)
            .map_err(|_| EngineError::InvalidState)?;
        let record = self.inner.store.insert(record)?;
        if let Err(error) = self.send(&record.id) {
            self.inner.store.remove_failed_admission(&record.id)?;
            return Err(error);
        }
        (self.inner.listener)(record.snapshot());
        Ok(record.snapshot())
    }

    fn send(&self, task_id: &str) -> Result<(), EngineError> {
        self.sender
            .send(task_id.to_owned())
            .map_err(|_| EngineError::QueueUnavailable)
    }

    fn active_token(&self, task_id: &str) -> Result<CancellationToken, EngineError> {
        self.inner
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(task_id)
            .cloned()
            .ok_or(EngineError::NotActive)
    }

    fn update<F>(&self, task_id: &str, epoch: u64, change: F) -> Result<TaskRecord, EngineError>
    where
        F: FnOnce(&mut TaskRecord) -> Result<(), StoreError>,
    {
        let record = self.inner.store.mutate(task_id, epoch, change)?;
        (self.inner.listener)(record.snapshot());
        Ok(record)
    }
}

fn worker_loop(inner: Arc<EngineInner>, receiver: mpsc::Receiver<String>) {
    while let Ok(task_id) = receiver.recv() {
        let Ok(record) = inner.store.get(&task_id) else {
            continue;
        };
        if record.status != TaskStatus::Queued {
            continue;
        }
        let token = CancellationToken::new();
        inner
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(task_id.clone(), token.clone());
        let epoch = record.epoch;
        let running = update_inner(&inner, &task_id, epoch, |task| {
            task.transition(TaskStatus::Running)
                .map_err(|_| StoreError::InvalidTransition)
        });
        let result = match running {
            Ok(running) => execute_task(&inner, &running, token.clone()),
            Err(error) => Err(EngineError::Store(error)),
        };
        inner
            .controls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&task_id);

        let current = inner.store.get(&task_id);
        let Ok(current) = current else {
            continue;
        };
        let _ = match result {
            Ok(()) => update_inner(&inner, &task_id, current.epoch, |task| {
                task.progress.bytes_processed = task.progress.total_bytes;
                task.progress.stage = "Complete".to_owned();
                task.failure = None;
                task.transition(TaskStatus::Completed)
                    .map_err(|_| StoreError::InvalidTransition)
            }),
            Err(EngineError::Core(CoreError::Cancelled)) => {
                update_inner(&inner, &task_id, current.epoch, |task| {
                    if task.status != TaskStatus::Cancelling {
                        task.transition(TaskStatus::Cancelling)
                            .map_err(|_| StoreError::InvalidTransition)?;
                    }
                    task.transition(TaskStatus::Cancelled)
                        .map_err(|_| StoreError::InvalidTransition)
                })
            }
            Err(error) => {
                let (status, code, message) = classify_error(&error);
                update_inner(&inner, &task_id, current.epoch, |task| {
                    task.failure = Some(TaskFailure::bounded(code, message));
                    task.transition(status)
                        .map_err(|_| StoreError::InvalidTransition)
                })
            }
        };
    }
}

fn execute_task(
    inner: &Arc<EngineInner>,
    record: &TaskRecord,
    token: CancellationToken,
) -> Result<(), EngineError> {
    match &record.spec {
        TaskSpec::Split {
            source_path,
            output_directory,
            slice_size,
            package_id,
            created_at,
        } => execute_split(
            inner,
            record,
            source_path,
            output_directory,
            *slice_size,
            package_id,
            created_at,
            token,
        ),
        TaskSpec::Merge {
            manifest_path,
            output_path,
            package_binding,
        } => execute_merge(
            inner,
            record,
            manifest_path,
            output_path,
            package_binding,
            token,
        ),
        TaskSpec::Inspect {
            manifest_path,
            verify_hashes,
            package_binding,
        } => {
            let inspection =
                inspect_package_bound(manifest_path, *verify_hashes, package_binding, &token)?;
            let inspection = bounded_inspection_summary(inspection)?;
            update_inner(inner, &record.id, record.epoch, |task| {
                task.result = Some(TaskResult::Inspection { inspection });
                task.progress.stage = "Inspection complete".to_owned();
                Ok(())
            })?;
            Ok(())
        }
        TaskSpec::Verify {
            manifest_path,
            package_binding,
        } => {
            let inspection = inspect_package_bound(manifest_path, true, package_binding, &token)?;
            let inspection = bounded_inspection_summary(inspection)?;
            update_inner(inner, &record.id, record.epoch, |task| {
                task.result = Some(TaskResult::Inspection { inspection });
                task.progress.stage = "Verification complete".to_owned();
                Ok(())
            })?;
            Ok(())
        }
    }
}

fn bounded_inspection_summary(
    inspection: cakesplitter_core::PackageInspection,
) -> Result<InspectionSummary, EngineError> {
    let summary = InspectionSummary::from(inspection);
    let bytes = serde_json::to_vec(&summary)
        .map_err(|error| EngineError::Core(CoreError::InvalidJson(error)))?;
    if bytes.len() > MAX_PACKAGE_INSPECTION_SERIALIZED_BYTES {
        return Err(EngineError::Core(CoreError::PackageEnumerationLimit {
            resource: "serialized inspection response bytes",
            maximum: MAX_PACKAGE_INSPECTION_SERIALIZED_BYTES,
        }));
    }
    Ok(summary)
}

#[allow(clippy::too_many_arguments)]
fn execute_split(
    inner: &Arc<EngineInner>,
    record: &TaskRecord,
    source_path: &Path,
    output_directory: &Path,
    slice_size: u64,
    package_id: &str,
    created_at: &str,
    token: CancellationToken,
) -> Result<(), EngineError> {
    validate_existing_regular_file(source_path)?;
    validate_existing_directory(output_directory)?;
    ensure_space(output_directory, remaining_required(record))?;
    let resume = match &record.checkpoint {
        Some(RecoveryCheckpoint::Split(value)) => Some(value.clone()),
        None => None,
        _ => return Err(EngineError::InvalidState),
    };
    let progress_gate = Arc::new(Mutex::new(Instant::now() - PROGRESS_WRITE_INTERVAL));
    let progress_inner = Arc::clone(inner);
    let progress_id = record.id.clone();
    let progress_gate_clone = Arc::clone(&progress_gate);
    let checkpoint_inner = Arc::clone(inner);
    let checkpoint_id = record.id.clone();
    let epoch = record.epoch;
    let progress_token = token.clone();
    let checkpoint_token = token.clone();
    let result = split_file_resumable_with_progress(
        source_path,
        &ResumableSplitOptions {
            task_id: record.id.clone(),
            package_id: package_id.to_owned(),
            created_at: created_at.to_owned(),
            slice_size,
            output_dir: output_directory.to_path_buf(),
            cancellation: token,
            resume,
        },
        move |progress| {
            if should_persist_progress(&progress_gate_clone, &progress)
                && update_progress(&progress_inner, &progress_id, epoch, progress).is_err()
            {
                progress_token.cancel();
            }
        },
        move |event| {
            if update_split_checkpoint(&checkpoint_inner, &checkpoint_id, epoch, event).is_err() {
                checkpoint_token.cancel();
            }
        },
    );
    result?;
    Ok(())
}

fn execute_merge(
    inner: &Arc<EngineInner>,
    record: &TaskRecord,
    manifest_path: &Path,
    output_path: &Path,
    package_binding: &PackageBinding,
    token: CancellationToken,
) -> Result<(), EngineError> {
    validate_existing_regular_file(manifest_path)?;
    let parent = output_path
        .parent()
        .ok_or_else(|| CoreError::UnsafeFilesystemPath(output_path.to_path_buf()))?;
    validate_existing_directory(parent)?;
    ensure_space(parent, remaining_required(record))?;
    let resume = match &record.checkpoint {
        Some(RecoveryCheckpoint::Merge(value)) => Some(value.clone()),
        None => None,
        _ => return Err(EngineError::InvalidState),
    };
    let output_filename = filename(output_path)?;
    let output_sha256 = package_binding.manifest.original.sha256.clone();
    let progress_gate = Arc::new(Mutex::new(Instant::now() - PROGRESS_WRITE_INTERVAL));
    let progress_inner = Arc::clone(inner);
    let progress_id = record.id.clone();
    let progress_gate_clone = Arc::clone(&progress_gate);
    let checkpoint_inner = Arc::clone(inner);
    let checkpoint_id = record.id.clone();
    let epoch = record.epoch;
    let progress_token = token.clone();
    let checkpoint_token = token.clone();
    merge_package_resumable_bound_with_progress(
        manifest_path,
        output_path,
        &ResumableMergeOptions {
            task_id: record.id.clone(),
            cancellation: token,
            resume,
        },
        package_binding,
        move |progress| {
            if should_persist_progress(&progress_gate_clone, &progress)
                && update_progress(&progress_inner, &progress_id, epoch, progress).is_err()
            {
                progress_token.cancel();
            }
        },
        move |event| {
            if update_merge_checkpoint(&checkpoint_inner, &checkpoint_id, epoch, event).is_err() {
                checkpoint_token.cancel();
            }
        },
    )?;
    update_inner(inner, &record.id, epoch, |task| {
        task.result = Some(TaskResult::Merge {
            output_filename,
            output_sha256,
        });
        Ok(())
    })?;
    Ok(())
}

fn update_progress(
    inner: &Arc<EngineInner>,
    task_id: &str,
    epoch: u64,
    progress: Progress,
) -> Result<TaskRecord, StoreError> {
    update_inner(inner, task_id, epoch, |task| {
        task.progress = TaskProgress {
            bytes_processed: progress.bytes_processed,
            total_bytes: progress.total_bytes,
            current_slice: progress.current_slice,
            slice_count: progress.slice_count,
            stage: if progress.operation == "split" {
                "Writing Slice".to_owned()
            } else {
                "Rebuilding output".to_owned()
            },
        };
        Ok(())
    })
}

fn update_split_checkpoint(
    inner: &Arc<EngineInner>,
    task_id: &str,
    epoch: u64,
    event: SplitCheckpointEvent,
) -> Result<TaskRecord, StoreError> {
    update_inner(inner, task_id, epoch, |task| {
        match event {
            SplitCheckpointEvent::Baseline {
                source,
                output_directory,
                baseline_sha256,
            } => {
                task.source_identity = Some(source.clone());
                task.destination_identity = Some(output_directory.clone());
                task.checkpoint = Some(RecoveryCheckpoint::Split(SplitResumeData {
                    source,
                    output_directory,
                    baseline_sha256,
                    completed: Vec::new(),
                    active_partial: None,
                    published_manifest_sha256: None,
                }));
                task.progress.stage = "Source baseline verified".to_owned();
            }
            SplitCheckpointEvent::PartialCreated { partial } => {
                let Some(RecoveryCheckpoint::Split(checkpoint)) = task.checkpoint.as_mut() else {
                    return Err(StoreError::CorruptState);
                };
                checkpoint.active_partial = Some(partial);
            }
            SplitCheckpointEvent::SliceCompleted { checkpoint: slice } => {
                let Some(RecoveryCheckpoint::Split(checkpoint)) = task.checkpoint.as_mut() else {
                    return Err(StoreError::CorruptState);
                };
                checkpoint.active_partial = None;
                checkpoint.completed.push(slice);
                task.progress.stage = "Slice boundary committed".to_owned();
            }
            SplitCheckpointEvent::PartialCleared => {
                let Some(RecoveryCheckpoint::Split(checkpoint)) = task.checkpoint.as_mut() else {
                    return Err(StoreError::CorruptState);
                };
                checkpoint.active_partial = None;
            }
            SplitCheckpointEvent::ManifestPublished { filename, sha256 } => {
                let Some(RecoveryCheckpoint::Split(checkpoint)) = task.checkpoint.as_mut() else {
                    return Err(StoreError::CorruptState);
                };
                checkpoint.active_partial = None;
                checkpoint.published_manifest_sha256 = Some(sha256);
                task.result = Some(TaskResult::Split {
                    manifest_filename: filename,
                    source_sha256: checkpoint.baseline_sha256.clone(),
                });
                task.progress.stage = "Manifest published".to_owned();
            }
        }
        Ok(())
    })
}

fn update_merge_checkpoint(
    inner: &Arc<EngineInner>,
    task_id: &str,
    epoch: u64,
    event: MergeCheckpointEvent,
) -> Result<TaskRecord, StoreError> {
    update_inner(inner, task_id, epoch, |task| {
        match event {
            MergeCheckpointEvent::PartialCreated {
                output_directory,
                partial,
            } => {
                task.destination_identity = Some(output_directory.clone());
                task.checkpoint = Some(RecoveryCheckpoint::Merge(MergeResumeData {
                    output_directory,
                    partial,
                    completed_slices: 0,
                    completed_bytes: 0,
                    published_sha256: None,
                }));
                task.progress.stage = "Partial output created".to_owned();
            }
            MergeCheckpointEvent::SliceBoundary {
                completed_slices,
                completed_bytes,
            } => {
                let Some(RecoveryCheckpoint::Merge(checkpoint)) = task.checkpoint.as_mut() else {
                    return Err(StoreError::CorruptState);
                };
                checkpoint.completed_slices = completed_slices;
                checkpoint.completed_bytes = completed_bytes;
                checkpoint.partial.verified_bytes = completed_bytes;
                task.progress.stage = "Slice boundary committed".to_owned();
            }
            MergeCheckpointEvent::Published { sha256, .. } => {
                let Some(RecoveryCheckpoint::Merge(checkpoint)) = task.checkpoint.as_mut() else {
                    return Err(StoreError::CorruptState);
                };
                checkpoint.published_sha256 = Some(sha256);
                task.progress.stage = "Output published".to_owned();
            }
        }
        Ok(())
    })
}

fn update_inner<F>(
    inner: &Arc<EngineInner>,
    task_id: &str,
    epoch: u64,
    change: F,
) -> Result<TaskRecord, StoreError>
where
    F: FnOnce(&mut TaskRecord) -> Result<(), StoreError>,
{
    let record = inner.store.mutate(task_id, epoch, change)?;
    (inner.listener)(record.snapshot());
    Ok(record)
}

fn should_persist_progress(gate: &Mutex<Instant>, progress: &Progress) -> bool {
    let mut last = gate.lock().unwrap_or_else(|error| error.into_inner());
    if last.elapsed() >= PROGRESS_WRITE_INTERVAL || progress.bytes_processed == progress.total_bytes
    {
        *last = Instant::now();
        true
    } else {
        false
    }
}

fn ensure_space(path: &Path, required: u64) -> Result<(), EngineError> {
    let available = fs4::available_space(path).map_err(|source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    ensure_available_space(required, available)
}

fn ensure_available_space(required: u64, available: u64) -> Result<(), EngineError> {
    if available < required {
        return Err(EngineError::InsufficientSpace {
            required,
            available,
        });
    }
    Ok(())
}

fn remaining_required(record: &TaskRecord) -> u64 {
    record
        .plan
        .required_free_bytes
        .saturating_sub(record.progress.bytes_processed)
}

fn validate_task_id(task_id: &str) -> Result<(), EngineError> {
    Uuid::parse_str(task_id)
        .map(|_| ())
        .map_err(|_| EngineError::InvalidTaskId)
}

fn filename(path: &Path) -> Result<String, EngineError> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| EngineError::Core(CoreError::NonUtf8Filename))
}

fn directory_label(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .or_else(|| Some("Selected folder".to_owned()))
}

fn classify_error(error: &EngineError) -> (TaskStatus, String, String) {
    if let EngineError::Core(CoreError::Io { source, .. }) = error {
        if source.kind() == std::io::ErrorKind::PermissionDenied {
            return (
                TaskStatus::PermissionRequired,
                "permission_required".to_owned(),
                format!("Permission is required to continue this local task: {source}"),
            );
        }
    }
    let code = match error {
        EngineError::Core(core) => core.code().to_owned(),
        EngineError::InsufficientSpace { .. } => "insufficient_space".to_owned(),
        EngineError::Store(_) => "task_store_error".to_owned(),
        _ => "desktop_runtime_error".to_owned(),
    };
    (TaskStatus::Failed, code, redacted_error_message(error))
}

fn redacted_error_message(error: &EngineError) -> String {
    match error {
        EngineError::Core(CoreError::Io { source, .. }) => {
            format!("A local filesystem operation failed: {source}")
        }
        EngineError::Core(CoreError::InvalidInput(_)) => {
            "The selected input is not a regular file.".to_owned()
        }
        EngineError::Core(CoreError::Collision(_)) => {
            "The planned output already exists.".to_owned()
        }
        EngineError::Core(CoreError::StagedIdentityChanged(_)) => {
            "The incomplete output identity changed before publication.".to_owned()
        }
        EngineError::Core(CoreError::StagedContentChanged(_)) => {
            "The incomplete output content changed before publication.".to_owned()
        }
        EngineError::Core(CoreError::AtomicFinalizationUnsupported(_)) => {
            "Atomic no-replace publication is unavailable for this destination.".to_owned()
        }
        EngineError::Core(CoreError::UnsafeFilesystemPath(_)) => {
            "The selected filesystem path is unsafe or ambiguous.".to_owned()
        }
        EngineError::Core(CoreError::DestinationIdentityChanged(_)) => {
            "The selected output destination changed or could not be proven stable.".to_owned()
        }
        EngineError::Core(CoreError::PackageIdentityChanged(_)) => {
            "The selected Cake Package changed or could not be proven stable. Select it again."
                .to_owned()
        }
        EngineError::Core(CoreError::PackageEnumerationLimit { .. }) => {
            "The selected Cake Package exceeds a supported local resource limit.".to_owned()
        }
        EngineError::Store(StoreError::Io { source, .. }) => {
            format!("The local task store could not be accessed: {source}")
        }
        _ => error.to_string(),
    }
}

fn cleanup_incomplete(record: &TaskRecord) -> Result<(), EngineError> {
    let expected_suffix = format!(".{}.partial", record.id);
    match (&record.spec, &record.checkpoint) {
        (
            TaskSpec::Split {
                output_directory, ..
            },
            Some(RecoveryCheckpoint::Split(checkpoint)),
        ) => {
            if let Some(partial) = &checkpoint.active_partial {
                validate_partial_name(&partial.filename, &expected_suffix)?;
                remove_owned_incomplete_file(
                    &output_directory.join(&partial.filename),
                    partial.identity,
                )?;
            }
        }
        (TaskSpec::Merge { output_path, .. }, Some(RecoveryCheckpoint::Merge(checkpoint))) => {
            validate_partial_name(&checkpoint.partial.filename, &expected_suffix)?;
            let parent = output_path
                .parent()
                .ok_or_else(|| CoreError::UnsafeFilesystemPath(output_path.clone()))?;
            remove_owned_incomplete_file(
                &parent.join(&checkpoint.partial.filename),
                checkpoint.partial.identity,
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn validate_partial_name(filename: &str, expected_suffix: &str) -> Result<(), EngineError> {
    if !filename.ends_with(expected_suffix)
        || Path::new(filename).components().count() != 1
        || filename == "."
        || filename == ".."
    {
        return Err(EngineError::InvalidState);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, thread};

    use tempfile::tempdir;

    use super::*;
    use crate::MAX_NONTERMINAL_TASKS;

    #[test]
    fn persisted_task_failures_do_not_expose_private_paths() {
        let private_path = PathBuf::from(r"C:\Users\Private Name\secret.bin");
        let errors = [
            EngineError::Core(CoreError::Collision(private_path.clone())),
            EngineError::Core(CoreError::UnsafeFilesystemPath(private_path.clone())),
            EngineError::Core(CoreError::DestinationIdentityChanged(private_path.clone())),
            EngineError::Core(CoreError::Io {
                path: private_path.clone(),
                source: std::io::Error::other("device failure"),
            }),
            EngineError::Store(StoreError::Io {
                path: private_path.clone(),
                source: std::io::Error::other("database failure"),
            }),
        ];

        for error in errors {
            let (_, _, message) = classify_error(&error);
            assert!(!message.contains("Private Name"));
            assert!(!message.contains("secret.bin"));
        }
    }

    #[test]
    fn insufficient_space_fails_before_processing_starts() {
        assert!(ensure_available_space(1024, 1024).is_ok());
        assert!(matches!(
            ensure_available_space(1024, 1023),
            Err(EngineError::InsufficientSpace {
                required: 1024,
                available: 1023,
            })
        ));
    }

    #[test]
    fn full_capacity_rejects_before_filesystem_access_or_execution_allocation() {
        let root = tempdir().unwrap();
        let store = Arc::new(TaskStore::open(root.path()).unwrap());
        for _ in 0..MAX_NONTERMINAL_TASKS {
            let epoch = store.epoch().unwrap();
            let mut record = TaskRecord::new(
                "0.4.0-dev",
                epoch,
                "duplicate.bin".to_owned(),
                Some("output".to_owned()),
                TaskSpec::Split {
                    source_path: PathBuf::from(r"C:\missing\duplicate.bin"),
                    output_directory: PathBuf::from(r"C:\missing\output"),
                    slice_size: 1024,
                    package_id: Uuid::new_v4().to_string(),
                    created_at: default_created_at(),
                },
                ProcessingPlan {
                    total_bytes: 1024,
                    slice_size: 1024,
                    slice_count: 1,
                    required_free_bytes: 1024,
                },
            );
            record.transition(TaskStatus::Queued).unwrap();
            store.insert(record).unwrap();
        }
        let (sender, _receiver) = mpsc::sync_channel(MAX_QUEUED_TASKS);
        let inner = Arc::new(EngineInner {
            store: Arc::clone(&store),
            application_version: "0.4.0-dev".to_owned(),
            controls: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            listener: Arc::new(|_| {}),
        });
        let engine = TaskEngine { inner, sender };

        assert!(matches!(
            engine.enqueue_split(
                PathBuf::from(r"C:\definitely-missing\source.bin"),
                PathBuf::from(r"C:\definitely-missing\output"),
                1024,
            ),
            Err(EngineError::Store(StoreError::QueueCapacityReached { .. }))
        ));
        assert!(engine.inner.controls.lock().unwrap().is_empty());
        assert_eq!(store.nonterminal_count().unwrap(), MAX_NONTERMINAL_TASKS);
    }

    #[test]
    fn concurrent_command_path_admission_never_exceeds_native_capacity() {
        let root = tempdir().unwrap();
        let app_data = root.path().join("app-data");
        let source = root.path().join("source.bin");
        let output = root.path().join("output");
        fs::write(&source, b"bounded").unwrap();
        fs::create_dir(&output).unwrap();
        let store = Arc::new(TaskStore::open(&app_data).unwrap());
        let (sender, _receiver) = mpsc::sync_channel(MAX_QUEUED_TASKS);
        let inner = Arc::new(EngineInner {
            store: Arc::clone(&store),
            application_version: "0.4.0-dev".to_owned(),
            controls: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            listener: Arc::new(|_| {}),
        });
        let engine = TaskEngine { inner, sender };
        let mut attempts = Vec::new();
        for _ in 0..MAX_NONTERMINAL_TASKS * 2 {
            let engine = engine.clone();
            let source = source.clone();
            let output = output.clone();
            attempts.push(thread::spawn(move || {
                engine.enqueue_split(source, output, 1024).is_ok()
            }));
        }
        let admitted = attempts
            .into_iter()
            .map(|attempt| attempt.join().unwrap())
            .filter(|admitted| *admitted)
            .count();
        assert_eq!(admitted, MAX_NONTERMINAL_TASKS);
        assert_eq!(store.nonterminal_count().unwrap(), MAX_NONTERMINAL_TASKS);
        assert!(engine.inner.controls.lock().unwrap().is_empty());
    }
}
