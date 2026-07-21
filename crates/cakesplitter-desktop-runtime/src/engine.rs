use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use cakesplitter_core::{
    CancellationToken, CoreError, DirectoryFingerprint, MAX_PACKAGE_INSPECTION_SERIALIZED_BYTES,
    MergeCheckpointEvent, MergeResumeData, PackageBinding, Progress, ResumableMergeOptions,
    ResumableSplitOptions, SplitCheckpointEvent, SplitResumeData, capture_package_binding,
    default_created_at, fingerprint_directory, fingerprint_file, inspect_package_bound,
    merge_package_resumable_bound_with_progress, remove_owned_incomplete_file,
    split_file_resumable_with_progress, validate_existing_directory,
    validate_existing_regular_file,
};
use cakesplitter_format::{
    MAX_SAFE_INTEGER, MAX_SLICE_COUNT, expected_slice_count, validate_portable_filename,
};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    MAX_FAILURE_HISTORY,
    exports::{
        ExportError, ExportSummary, ReceiptFormat, export_diagnostic_bundle,
        export_operation_receipt,
    },
    model::{
        ConflictClass, ConflictType, DesktopPreferences, ErrorCategory, InspectionSummary,
        PreflightResult, PreflightState, PreflightWarning, ProcessingPlan, QueueDirection,
        RecoveryAction, RecoveryCheckpoint, StorageSummary, TaskConflict, TaskFailure,
        TaskPriority, TaskProgress, TaskRecord, TaskResult, TaskSnapshot, TaskSpec, TaskStatus,
        now,
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
    #[error("task admission conflicts with an existing nonterminal task")]
    TaskConflict(TaskConflict),
    #[error("task preflight did not reach a ready state")]
    PreflightBlocked,
    #[error("the prior failure is not safely retryable")]
    RetryNotAllowed,
    #[error("desktop retention or Slice-size settings are invalid")]
    InvalidSettings,
    #[error("an export is already in progress or the export token is invalid")]
    InvalidExport,
    #[error(transparent)]
    Export(#[from] ExportError),
}

pub struct TaskEngine {
    inner: Arc<EngineInner>,
}

struct EngineInner {
    store: Arc<TaskStore>,
    application_version: String,
    controls: Mutex<HashMap<String, CancellationToken>>,
    admission: Mutex<()>,
    listener: Arc<Listener>,
    scheduler_generation: Mutex<u64>,
    scheduler_wake: Condvar,
    clearing: AtomicBool,
    shutdown: AtomicBool,
    client_handles: AtomicU64,
    diagnostic_bundle_count: AtomicU64,
}

impl Clone for TaskEngine {
    fn clone(&self) -> Self {
        self.inner.client_handles.fetch_add(1, Ordering::AcqRel);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Drop for TaskEngine {
    fn drop(&mut self) {
        if self.inner.client_handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.inner.shutdown.store(true, Ordering::Release);
            self.notify_scheduler();
        }
    }
}

impl TaskEngine {
    pub fn open(
        app_data_directory: &Path,
        application_version: impl Into<String>,
        listener: impl Fn(TaskSnapshot) + Send + Sync + 'static,
    ) -> Result<Self, EngineError> {
        let store = Arc::new(TaskStore::open(app_data_directory)?);
        store.recover_after_restart()?;
        let inner = Arc::new(EngineInner {
            store,
            application_version: application_version.into(),
            controls: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            listener: Arc::new(listener),
            scheduler_generation: Mutex::new(0),
            scheduler_wake: Condvar::new(),
            clearing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            client_handles: AtomicU64::new(1),
            diagnostic_bundle_count: AtomicU64::new(0),
        });
        let worker = Arc::clone(&inner);
        thread::Builder::new()
            .name("cakesplitter-task-worker".to_owned())
            .spawn(move || worker_loop(worker))
            .map_err(|_| EngineError::QueueUnavailable)?;
        let engine = Self { inner };
        engine.notify_scheduler();
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
        let package_id = Uuid::new_v4().to_string();
        let (plan, _, _, _, _) =
            self.split_preflight_data(source_path, output_directory, slice_size, package_id, None)?;
        Ok(plan)
    }

    pub fn preflight_split(
        &self,
        source_path: &Path,
        output_directory: &Path,
        slice_size: u64,
    ) -> Result<PreflightResult, EngineError> {
        let package_id = Uuid::new_v4().to_string();
        let (_, preflight, _, _, _) =
            self.split_preflight_data(source_path, output_directory, slice_size, package_id, None)?;
        Ok(preflight)
    }

    pub fn preflight_merge(
        &self,
        manifest_path: &Path,
        output_path: &Path,
    ) -> Result<PreflightResult, EngineError> {
        let (_, preflight, _, _) = self.merge_preflight_data(manifest_path, output_path, None)?;
        Ok(preflight)
    }

    pub fn enqueue_split(
        &self,
        source_path: PathBuf,
        output_directory: PathBuf,
        slice_size: u64,
    ) -> Result<TaskSnapshot, EngineError> {
        self.enqueue_split_with_priority(
            source_path,
            output_directory,
            slice_size,
            TaskPriority::Normal,
        )
    }

    pub fn enqueue_split_with_priority(
        &self,
        source_path: PathBuf,
        output_directory: PathBuf,
        slice_size: u64,
        priority: TaskPriority,
    ) -> Result<TaskSnapshot, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.store.ensure_admission_available()?;
        ensure_not_clearing(&self.inner)?;
        let display_name = filename(&source_path)?;
        let destination_name = directory_label(&output_directory);
        let epoch = self.inner.store.epoch()?;
        let package_id = Uuid::new_v4().to_string();
        let (plan, preflight, source_identity, destination_identity, spec) = self
            .split_preflight_data(
                &source_path,
                &output_directory,
                slice_size,
                package_id,
                None,
            )?;
        ensure_admittable_preflight(&preflight)?;
        let mut record = TaskRecord::new(
            &self.inner.application_version,
            epoch,
            display_name,
            destination_name,
            spec,
            plan,
        );
        record.priority = priority;
        record.preflight = Some(preflight);
        record.source_identity = Some(source_identity);
        record.destination_identity = Some(destination_identity);
        self.enqueue_record(record)
    }

    pub fn enqueue_merge(
        &self,
        manifest_path: PathBuf,
        output_path: PathBuf,
    ) -> Result<TaskSnapshot, EngineError> {
        self.enqueue_merge_with_priority(manifest_path, output_path, TaskPriority::Normal)
    }

    pub fn enqueue_merge_with_priority(
        &self,
        manifest_path: PathBuf,
        output_path: PathBuf,
        priority: TaskPriority,
    ) -> Result<TaskSnapshot, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.store.ensure_admission_available()?;
        ensure_not_clearing(&self.inner)?;
        let (plan, preflight, destination_identity, spec) =
            self.merge_preflight_data(&manifest_path, &output_path, None)?;
        ensure_admittable_preflight(&preflight)?;
        let epoch = self.inner.store.epoch()?;
        let display_name = match &spec {
            TaskSpec::Merge {
                package_binding, ..
            } => package_binding.manifest.original.filename.clone(),
            _ => return Err(EngineError::InvalidState),
        };
        let destination_name = Some(filename(&output_path)?);
        let mut record = TaskRecord::new(
            &self.inner.application_version,
            epoch,
            display_name,
            destination_name,
            spec,
            plan,
        );
        record.priority = priority;
        record.preflight = Some(preflight);
        record.destination_identity = Some(destination_identity);
        if let TaskSpec::Merge {
            package_binding, ..
        } = &record.spec
        {
            record.source_identity = Some(package_binding.manifest_identity.clone());
        }
        self.enqueue_record(record)
    }

    pub fn enqueue_inspect(
        &self,
        manifest_path: PathBuf,
        verify_hashes: bool,
    ) -> Result<TaskSnapshot, EngineError> {
        self.enqueue_inspect_with_priority(manifest_path, verify_hashes, TaskPriority::Normal)
    }

    pub fn enqueue_inspect_with_priority(
        &self,
        manifest_path: PathBuf,
        verify_hashes: bool,
        priority: TaskPriority,
    ) -> Result<TaskSnapshot, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.store.ensure_admission_available()?;
        ensure_not_clearing(&self.inner)?;
        let package_binding = capture_package_binding(&manifest_path, &CancellationToken::new())?;
        let epoch = self.inner.store.epoch()?;
        let display_name = package_binding.manifest.original.filename.clone();
        let plan = ProcessingPlan {
            total_bytes: package_binding.manifest.original.size,
            slice_size: package_binding.manifest.target_slice_size,
            slice_count: package_binding.manifest.slice_count,
            required_free_bytes: 0,
            expected_output_count: 0,
            ..ProcessingPlan::default()
        };
        let spec = TaskSpec::Inspect {
            manifest_path,
            verify_hashes,
            package_binding,
        };
        let preflight = self.preflight_for_candidate(&spec, &plan, 0, None)?;
        ensure_admittable_preflight(&preflight)?;
        let mut record = TaskRecord::new(
            &self.inner.application_version,
            epoch,
            display_name,
            None,
            spec,
            plan,
        );
        record.priority = priority;
        record.preflight = Some(preflight);
        if let TaskSpec::Inspect {
            package_binding, ..
        } = &record.spec
        {
            record.source_identity = Some(package_binding.manifest_identity.clone());
        }
        self.enqueue_record(record)
    }

    pub fn enqueue_verify(&self, manifest_path: PathBuf) -> Result<TaskSnapshot, EngineError> {
        self.enqueue_verify_with_priority(manifest_path, TaskPriority::Normal)
    }

    pub fn enqueue_verify_with_priority(
        &self,
        manifest_path: PathBuf,
        priority: TaskPriority,
    ) -> Result<TaskSnapshot, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.store.ensure_admission_available()?;
        ensure_not_clearing(&self.inner)?;
        let package_binding = capture_package_binding(&manifest_path, &CancellationToken::new())?;
        let epoch = self.inner.store.epoch()?;
        let display_name = package_binding.manifest.original.filename.clone();
        let plan = ProcessingPlan {
            total_bytes: package_binding.manifest.original.size,
            slice_size: package_binding.manifest.target_slice_size,
            slice_count: package_binding.manifest.slice_count,
            required_free_bytes: 0,
            expected_output_count: 0,
            ..ProcessingPlan::default()
        };
        let spec = TaskSpec::Verify {
            manifest_path,
            package_binding,
        };
        let preflight = self.preflight_for_candidate(&spec, &plan, 0, None)?;
        ensure_admittable_preflight(&preflight)?;
        let mut record = TaskRecord::new(
            &self.inner.application_version,
            epoch,
            display_name,
            None,
            spec,
            plan,
        );
        record.priority = priority;
        record.preflight = Some(preflight);
        if let TaskSpec::Verify {
            package_binding, ..
        } = &record.spec
        {
            record.source_identity = Some(package_binding.manifest_identity.clone());
        }
        self.enqueue_record(record)
    }

    pub fn list_tasks(&self) -> Result<Vec<TaskSnapshot>, EngineError> {
        let positions = self
            .inner
            .store
            .queued_in_scheduler_order()?
            .into_iter()
            .enumerate()
            .map(|(index, record)| (record.id, index as u64 + 1))
            .collect::<HashMap<_, _>>();
        Ok(self
            .inner
            .store
            .list()?
            .into_iter()
            .map(|record| record.snapshot_with_position(positions.get(&record.id).copied()))
            .collect())
    }

    pub fn set_task_priority(
        &self,
        task_id: &str,
        priority: TaskPriority,
    ) -> Result<TaskSnapshot, EngineError> {
        validate_task_id(task_id)?;
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = self.inner.store.get(task_id)?;
        let updated = self
            .inner
            .store
            .set_priority(task_id, record.epoch, priority)?;
        (self.inner.listener)(updated.snapshot());
        self.notify_scheduler();
        Ok(updated.snapshot())
    }

    pub fn reorder_task(
        &self,
        task_id: &str,
        direction: QueueDirection,
    ) -> Result<Vec<TaskSnapshot>, EngineError> {
        validate_task_id(task_id)?;
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let records = self.inner.store.move_queued(task_id, direction)?;
        for record in &records {
            (self.inner.listener)(record.snapshot());
        }
        self.notify_scheduler();
        Ok(records
            .into_iter()
            .map(|record| record.snapshot())
            .collect())
    }

    pub fn storage_summary(&self) -> Result<StorageSummary, EngineError> {
        Ok(self
            .inner
            .store
            .storage_summary(self.inner.diagnostic_bundle_count.load(Ordering::Acquire))?)
    }

    pub fn clear_completed_history(&self) -> Result<usize, EngineError> {
        Ok(self.inner.store.clear_completed_history()?)
    }

    pub fn clear_failed_history(&self) -> Result<usize, EngineError> {
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for record in self
            .inner
            .store
            .list()?
            .into_iter()
            .filter(|record| record.status == TaskStatus::Failed)
        {
            cleanup_incomplete(&record)?;
        }
        Ok(self.inner.store.clear_failed_history()?)
    }

    pub fn clear_quarantine(&self) -> Result<usize, EngineError> {
        Ok(self.inner.store.clear_quarantine()?)
    }

    pub fn export_receipt(
        &self,
        task_id: &str,
        output_path: &Path,
        expected_parent: &DirectoryFingerprint,
        format: ReceiptFormat,
        include_path_detail: bool,
    ) -> Result<ExportSummary, EngineError> {
        validate_task_id(task_id)?;
        let record = self.inner.store.get(task_id)?;
        Ok(export_operation_receipt(
            &record,
            output_path,
            expected_parent,
            format,
            include_path_detail,
        )?)
    }

    pub fn export_diagnostics(
        &self,
        output_parent: &Path,
        expected_parent: &DirectoryFingerprint,
    ) -> Result<ExportSummary, EngineError> {
        let records = self.inner.store.list()?;
        let storage = self.storage_summary()?;
        let summary = export_diagnostic_bundle(
            output_parent,
            expected_parent,
            &self.inner.application_version,
            &records,
            &storage,
        )?;
        self.inner
            .diagnostic_bundle_count
            .fetch_add(1, Ordering::AcqRel);
        Ok(summary)
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
            TaskStatus::Interrupted | TaskStatus::PermissionRequired | TaskStatus::Cancelled
        ) {
            return self.requeue_existing(record, false);
        }
        Err(EngineError::InvalidState)
    }

    pub fn retry_task(&self, task_id: &str) -> Result<TaskSnapshot, EngineError> {
        validate_task_id(task_id)?;
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = self.inner.store.get(task_id)?;
        if matches!(
            record.status,
            TaskStatus::Cancelled | TaskStatus::Interrupted
        ) {
            return self.requeue_existing(record, false);
        }
        if !matches!(
            record.status,
            TaskStatus::Failed | TaskStatus::PermissionRequired
        ) {
            return Err(EngineError::InvalidState);
        }
        self.requeue_existing(record, true)
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
            finish_task(task);
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
        let _admission = self
            .inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inner.clearing.store(true, Ordering::Release);
        let result = (|| {
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
        })();
        self.inner.clearing.store(false, Ordering::Release);
        self.notify_scheduler();
        result
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
        if preferences.default_slice_size > MAX_SAFE_INTEGER || !preferences.validate() {
            return Err(EngineError::InvalidSettings);
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
        (self.inner.listener)(record.snapshot());
        self.notify_scheduler();
        Ok(record.snapshot())
    }

    fn notify_scheduler(&self) {
        let mut generation = self
            .inner
            .scheduler_generation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *generation = generation.wrapping_add(1);
        self.inner.scheduler_wake.notify_one();
    }

    fn split_preflight_data(
        &self,
        source_path: &Path,
        output_directory: &Path,
        slice_size: u64,
        package_id: String,
        exclude_task_id: Option<&str>,
    ) -> Result<
        (
            ProcessingPlan,
            PreflightResult,
            cakesplitter_core::SourceFingerprint,
            cakesplitter_core::DirectoryFingerprint,
            TaskSpec,
        ),
        EngineError,
    > {
        validate_existing_regular_file(source_path)?;
        validate_existing_directory(output_directory)?;
        if slice_size == 0 || slice_size > MAX_SAFE_INTEGER {
            return Err(EngineError::InvalidSliceSize);
        }
        let source_identity = fingerprint_file(source_path)?;
        let destination_identity = fingerprint_directory(output_directory)?;
        let total_bytes = source_identity.len;
        if total_bytes > MAX_SAFE_INTEGER {
            return Err(EngineError::InvalidSliceSize);
        }
        let slice_count = expected_slice_count(total_bytes, slice_size);
        if slice_count > MAX_SLICE_COUNT {
            return Err(EngineError::SliceLimit);
        }
        let recovery_overhead_bytes = recovery_overhead(slice_count)?;
        let minimum_required_bytes = total_bytes
            .checked_add(recovery_overhead_bytes)
            .ok_or(EngineError::InvalidSliceSize)?;
        let recommended_free_bytes = minimum_required_bytes
            .checked_add(DISK_SPACE_MARGIN_BYTES)
            .ok_or(EngineError::InvalidSliceSize)?;
        let available_free_bytes = available_space(output_directory)?;
        let expected_output_count = slice_count.checked_add(1).ok_or(EngineError::SliceLimit)?;
        let plan = ProcessingPlan {
            total_bytes,
            slice_size,
            slice_count,
            required_free_bytes: minimum_required_bytes,
            minimum_required_bytes,
            recommended_free_bytes,
            available_free_bytes,
            temporary_bytes: total_bytes,
            recovery_overhead_bytes,
            expected_output_count,
        };
        let spec = TaskSpec::Split {
            source_path: source_path.to_path_buf(),
            output_directory: output_directory.to_path_buf(),
            slice_size,
            package_id,
            created_at: default_created_at(),
        };
        let preflight =
            self.preflight_for_candidate(&spec, &plan, available_free_bytes, exclude_task_id)?;
        Ok((plan, preflight, source_identity, destination_identity, spec))
    }

    fn merge_preflight_data(
        &self,
        manifest_path: &Path,
        output_path: &Path,
        exclude_task_id: Option<&str>,
    ) -> Result<
        (
            ProcessingPlan,
            PreflightResult,
            cakesplitter_core::DirectoryFingerprint,
            TaskSpec,
        ),
        EngineError,
    > {
        let package_binding = capture_package_binding(manifest_path, &CancellationToken::new())?;
        let manifest = &package_binding.manifest;
        let parent = output_path
            .parent()
            .ok_or_else(|| CoreError::UnsafeFilesystemPath(output_path.to_path_buf()))?;
        validate_existing_directory(parent)?;
        validate_portable_filename(
            output_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(CoreError::NonUtf8Filename)?,
        )
        .map_err(CoreError::from)?;
        ensure_output_absent(output_path)?;
        let destination_identity = fingerprint_directory(parent)?;
        let recovery_overhead_bytes = recovery_overhead(manifest.slice_count)?;
        let minimum_required_bytes = manifest
            .original
            .size
            .checked_add(recovery_overhead_bytes)
            .ok_or(EngineError::InvalidSliceSize)?;
        let recommended_free_bytes = minimum_required_bytes
            .checked_add(DISK_SPACE_MARGIN_BYTES)
            .ok_or(EngineError::InvalidSliceSize)?;
        let available_free_bytes = available_space(parent)?;
        let plan = ProcessingPlan {
            total_bytes: manifest.original.size,
            slice_size: manifest.target_slice_size,
            slice_count: manifest.slice_count,
            required_free_bytes: minimum_required_bytes,
            minimum_required_bytes,
            recommended_free_bytes,
            available_free_bytes,
            temporary_bytes: manifest.original.size,
            recovery_overhead_bytes,
            expected_output_count: 1,
        };
        let spec = TaskSpec::Merge {
            manifest_path: manifest_path.to_path_buf(),
            output_path: output_path.to_path_buf(),
            package_binding,
        };
        let preflight =
            self.preflight_for_candidate(&spec, &plan, available_free_bytes, exclude_task_id)?;
        Ok((plan, preflight, destination_identity, spec))
    }

    fn preflight_for_candidate(
        &self,
        spec: &TaskSpec,
        plan: &ProcessingPlan,
        available_free_bytes: u64,
        exclude_task_id: Option<&str>,
    ) -> Result<PreflightResult, EngineError> {
        let conflicts = detect_conflicts(&self.inner.store.list()?, spec, exclude_task_id);
        let blocked_by_conflict = conflicts
            .iter()
            .any(|conflict| conflict.class != ConflictClass::InformationalOverlap);
        let mut warnings = conflicts
            .iter()
            .filter(|conflict| conflict.class == ConflictClass::InformationalOverlap)
            .map(|conflict| {
                PreflightWarning::bounded(
                    "informational_overlap",
                    format!(
                        "Task {} also uses {}.",
                        conflict.conflicting_task_id, conflict.affected_resource
                    ),
                )
            })
            .collect::<Vec<_>>();
        let insufficient = available_free_bytes < plan.minimum_required_bytes;
        if insufficient {
            warnings.push(PreflightWarning::bounded(
                "insufficient_space",
                "Available space is below the minimum required bytes.",
            ));
        } else if available_free_bytes < plan.recommended_free_bytes {
            warnings.push(PreflightWarning::bounded(
                "space_margin_low",
                "Available space meets the minimum but not the recommended safety margin.",
            ));
        }
        let state = if blocked_by_conflict || insufficient {
            PreflightState::Blocked
        } else if warnings.is_empty() {
            PreflightState::Ready
        } else {
            PreflightState::ReadyWithWarning
        };
        Ok(PreflightResult {
            state,
            checked_at: now(),
            minimum_required_bytes: plan.minimum_required_bytes,
            recommended_free_bytes: plan.recommended_free_bytes,
            available_free_bytes,
            temporary_bytes: plan.temporary_bytes,
            recovery_overhead_bytes: plan.recovery_overhead_bytes,
            expected_output_count: plan.expected_output_count,
            warnings,
            conflicts,
        }
        .bounded_warnings())
    }

    fn preflight_existing(&self, record: &TaskRecord) -> Result<PreflightResult, EngineError> {
        match &record.spec {
            TaskSpec::Split {
                source_path,
                output_directory,
                slice_size,
                package_id,
                ..
            } => {
                let (_, preflight, source, destination, _) = self.split_preflight_data(
                    source_path,
                    output_directory,
                    *slice_size,
                    package_id.clone(),
                    Some(&record.id),
                )?;
                if record.source_identity.as_ref() != Some(&source) {
                    return Err(EngineError::Core(CoreError::SourceChanged));
                }
                if record.destination_identity.as_ref() != Some(&destination) {
                    return Err(EngineError::Core(CoreError::DestinationIdentityChanged(
                        output_directory.clone(),
                    )));
                }
                Ok(preflight)
            }
            TaskSpec::Merge {
                manifest_path,
                output_path,
                package_binding,
            } => {
                let (_, preflight, destination, current_spec) =
                    self.merge_preflight_data(manifest_path, output_path, Some(&record.id))?;
                let TaskSpec::Merge {
                    package_binding: current_binding,
                    ..
                } = current_spec
                else {
                    return Err(EngineError::InvalidState);
                };
                if &current_binding != package_binding {
                    return Err(EngineError::Core(CoreError::PackageIdentityChanged(
                        manifest_path.clone(),
                    )));
                }
                if record.destination_identity.as_ref() != Some(&destination) {
                    return Err(EngineError::Core(CoreError::DestinationIdentityChanged(
                        output_path.clone(),
                    )));
                }
                Ok(preflight)
            }
            TaskSpec::Inspect {
                manifest_path,
                package_binding,
                ..
            }
            | TaskSpec::Verify {
                manifest_path,
                package_binding,
            } => {
                let current = capture_package_binding(manifest_path, &CancellationToken::new())?;
                if &current != package_binding {
                    return Err(EngineError::Core(CoreError::PackageIdentityChanged(
                        manifest_path.clone(),
                    )));
                }
                self.preflight_for_candidate(&record.spec, &record.plan, 0, Some(&record.id))
            }
        }
    }

    fn requeue_existing(
        &self,
        record: TaskRecord,
        require_retryable: bool,
    ) -> Result<TaskSnapshot, EngineError> {
        ensure_not_clearing(&self.inner)?;
        if record.status.is_terminal() {
            self.inner.store.ensure_admission_available()?;
        }
        let preflight = self.preflight_existing(&record)?;
        ensure_admittable_preflight(&preflight)?;
        if require_retryable
            && !record
                .failure
                .as_ref()
                .is_some_and(|failure| failure.retryable)
            && !record.failure.as_ref().is_some_and(|failure| {
                matches!(
                    failure.recovery_action,
                    RecoveryAction::ReselectSource
                        | RecoveryAction::ReselectDestination
                        | RecoveryAction::ReselectPackage
                )
            })
        {
            return Err(EngineError::RetryNotAllowed);
        }
        let queued = self.update(&record.id, record.epoch, |task| {
            if let Some(failure) = task.failure.take() {
                task.failure_history.push(failure);
                if task.failure_history.len() > MAX_FAILURE_HISTORY {
                    let overflow = task.failure_history.len() - MAX_FAILURE_HISTORY;
                    task.failure_history.drain(0..overflow);
                }
            }
            task.preflight = Some(preflight);
            task.finished_at = None;
            task.duration_ms = None;
            task.transition(TaskStatus::Queued)
                .map_err(|_| StoreError::InvalidTransition)
        })?;
        self.notify_scheduler();
        Ok(queued.snapshot())
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

fn ensure_not_clearing(inner: &EngineInner) -> Result<(), EngineError> {
    if inner.clearing.load(Ordering::Acquire) {
        Err(EngineError::TasksStopping)
    } else {
        Ok(())
    }
}

fn ensure_admittable_preflight(preflight: &PreflightResult) -> Result<(), EngineError> {
    if matches!(
        preflight.state,
        PreflightState::Ready | PreflightState::ReadyWithWarning
    ) {
        return Ok(());
    }
    if let Some(conflict) = preflight.conflicts.first() {
        return Err(EngineError::TaskConflict(conflict.clone()));
    }
    if preflight.available_free_bytes < preflight.minimum_required_bytes {
        return Err(EngineError::InsufficientSpace {
            required: preflight.minimum_required_bytes,
            available: preflight.available_free_bytes,
        });
    }
    Err(EngineError::PreflightBlocked)
}

fn recovery_overhead(slice_count: u64) -> Result<u64, EngineError> {
    slice_count
        .checked_mul(256)
        .and_then(|bytes| bytes.checked_add(4_096))
        .ok_or(EngineError::SliceLimit)
}

fn available_space(path: &Path) -> Result<u64, EngineError> {
    fs4::available_space(path).map_err(|source| {
        EngineError::Core(CoreError::Io {
            path: path.to_path_buf(),
            source,
        })
    })
}

fn ensure_output_absent(path: &Path) -> Result<(), EngineError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Err(EngineError::Core(CoreError::Collision(path.to_path_buf()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(EngineError::Core(CoreError::Io {
            path: path.to_path_buf(),
            source,
        })),
    }
}

fn preflight_runtime(
    inner: &Arc<EngineInner>,
    record: &TaskRecord,
) -> Result<PreflightResult, EngineError> {
    inner.client_handles.fetch_add(1, Ordering::AcqRel);
    let engine = TaskEngine {
        inner: Arc::clone(inner),
    };
    engine.preflight_existing(record)
}

fn finish_task(task: &mut TaskRecord) {
    let finished = now();
    task.duration_ms = task.started_at.as_ref().and_then(|started| {
        let start = chrono::DateTime::parse_from_rfc3339(started).ok()?;
        let end = chrono::DateTime::parse_from_rfc3339(&finished).ok()?;
        u64::try_from((end - start).num_milliseconds()).ok()
    });
    task.finished_at = Some(finished);
}

#[derive(Clone)]
enum OutputResource {
    File(PathBuf),
    Directory(PathBuf),
}

struct TaskResources {
    operation: crate::TaskOperation,
    input_files: Vec<(PathBuf, Option<cakesplitter_core::NativeFileIdentity>)>,
    package_directory: Option<(PathBuf, cakesplitter_core::NativeFileIdentity)>,
    package_id: Option<String>,
    output: Option<OutputResource>,
    slice_size: Option<u64>,
    verify_hashes: Option<bool>,
}

fn detect_conflicts(
    records: &[TaskRecord],
    candidate: &TaskSpec,
    exclude_task_id: Option<&str>,
) -> Vec<TaskConflict> {
    let candidate_resources = task_resources(candidate, None);
    let mut conflicts = Vec::new();

    if let TaskSpec::Split {
        source_path,
        output_directory,
        ..
    } = candidate
    {
        if path_contains(output_directory, source_path) {
            conflicts.push(conflict(
                "current-selection",
                ConflictClass::HardConflict,
                ConflictType::SourceUsedAsDestination,
                source_path,
                "Choose an output folder that does not contain the source file.",
            ));
        }
    }
    if let TaskSpec::Merge {
        output_path,
        package_binding,
        manifest_path,
    } = candidate
    {
        let package_directory = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        if path_contains(package_directory, output_path)
            || package_binding
                .slices
                .iter()
                .any(|slice| output_path.ends_with(&slice.filename))
        {
            conflicts.push(conflict(
                "current-selection",
                ConflictClass::HardConflict,
                ConflictType::DestinationInsidePackage,
                output_path,
                "Choose an output outside the selected Cake Package.",
            ));
        }
    }

    for record in records.iter().filter(|record| {
        !record.status.is_terminal() && Some(record.id.as_str()) != exclude_task_id
    }) {
        let existing = task_resources(&record.spec, Some(record));
        if equivalent_task(&candidate_resources, &existing) {
            conflicts.push(conflict(
                &record.id,
                ConflictClass::DuplicateTask,
                ConflictType::DuplicateOperation,
                candidate_primary_resource(candidate),
                "Use the existing task or remove it before adding this duplicate.",
            ));
            continue;
        }
        if outputs_overlap(
            candidate_resources.output.as_ref(),
            existing.output.as_ref(),
        ) {
            conflicts.push(conflict(
                &record.id,
                ConflictClass::HardConflict,
                ConflictType::OverlappingOutput,
                candidate_output_path(candidate)
                    .unwrap_or_else(|| candidate_primary_resource(candidate)),
                "Wait for or remove the conflicting task, then run preflight again.",
            ));
            continue;
        }
        if output_overlaps_inputs(&candidate_resources.output, &existing.input_files)
            || output_overlaps_inputs(&existing.output, &candidate_resources.input_files)
        {
            conflicts.push(conflict(
                &record.id,
                ConflictClass::HardConflict,
                ConflictType::SourceUsedAsDestination,
                candidate_primary_resource(candidate),
                "Choose resources that are not used as another task's destination.",
            ));
            continue;
        }
        if inputs_overlap(&candidate_resources.input_files, &existing.input_files) {
            conflicts.push(conflict(
                &record.id,
                ConflictClass::InformationalOverlap,
                ConflictType::SharedInput,
                candidate_primary_resource(candidate),
                "The scheduler will serialize disk-intensive access.",
            ));
        } else if package_overlap(&candidate_resources, &existing) {
            conflicts.push(conflict(
                &record.id,
                ConflictClass::InformationalOverlap,
                ConflictType::SamePackageDirectory,
                candidate_primary_resource(candidate),
                "The scheduler will serialize access to the shared Cake Package.",
            ));
        }
    }
    conflicts.sort_by_key(|value| {
        (
            std::cmp::Reverse(conflict_rank(value.class)),
            value.conflicting_task_id.clone(),
        )
    });
    conflicts
}

fn task_resources(spec: &TaskSpec, record: Option<&TaskRecord>) -> TaskResources {
    match spec {
        TaskSpec::Split {
            source_path,
            output_directory,
            slice_size,
            package_id,
            ..
        } => TaskResources {
            operation: crate::TaskOperation::Split,
            input_files: vec![(
                source_path.clone(),
                record
                    .and_then(|record| record.source_identity.as_ref())
                    .map(|fingerprint| fingerprint.identity)
                    .or_else(|| {
                        fingerprint_file(source_path)
                            .ok()
                            .map(|value| value.identity)
                    }),
            )],
            package_directory: None,
            package_id: Some(package_id.clone()),
            output: Some(OutputResource::Directory(output_directory.clone())),
            slice_size: Some(*slice_size),
            verify_hashes: None,
        },
        TaskSpec::Merge {
            manifest_path,
            output_path,
            package_binding,
        } => TaskResources {
            operation: crate::TaskOperation::Merge,
            input_files: vec![(
                manifest_path.clone(),
                Some(package_binding.manifest_identity.identity),
            )],
            package_directory: Some((
                manifest_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
                package_binding.package_directory.identity,
            )),
            package_id: Some(package_binding.manifest.package_id.clone()),
            output: Some(OutputResource::File(output_path.clone())),
            slice_size: None,
            verify_hashes: None,
        },
        TaskSpec::Inspect {
            manifest_path,
            verify_hashes,
            package_binding,
        } => TaskResources {
            operation: crate::TaskOperation::Inspect,
            input_files: vec![(
                manifest_path.clone(),
                Some(package_binding.manifest_identity.identity),
            )],
            package_directory: Some((
                manifest_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
                package_binding.package_directory.identity,
            )),
            package_id: Some(package_binding.manifest.package_id.clone()),
            output: None,
            slice_size: None,
            verify_hashes: Some(*verify_hashes),
        },
        TaskSpec::Verify {
            manifest_path,
            package_binding,
        } => TaskResources {
            operation: crate::TaskOperation::Verify,
            input_files: vec![(
                manifest_path.clone(),
                Some(package_binding.manifest_identity.identity),
            )],
            package_directory: Some((
                manifest_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf(),
                package_binding.package_directory.identity,
            )),
            package_id: Some(package_binding.manifest.package_id.clone()),
            output: None,
            slice_size: None,
            verify_hashes: Some(true),
        },
    }
}

fn equivalent_task(first: &TaskResources, second: &TaskResources) -> bool {
    first.operation == second.operation
        && first.slice_size == second.slice_size
        && first.verify_hashes == second.verify_hashes
        && first.input_files.len() == second.input_files.len()
        && inputs_overlap(&first.input_files, &second.input_files)
        && output_equal(first.output.as_ref(), second.output.as_ref())
}

fn inputs_overlap(
    first: &[(PathBuf, Option<cakesplitter_core::NativeFileIdentity>)],
    second: &[(PathBuf, Option<cakesplitter_core::NativeFileIdentity>)],
) -> bool {
    first.iter().any(|(first_path, first_identity)| {
        second.iter().any(|(second_path, second_identity)| {
            first_identity
                .zip(*second_identity)
                .is_some_and(|(first, second)| first == second)
                || normalized_path(first_path) == normalized_path(second_path)
        })
    })
}

fn package_overlap(first: &TaskResources, second: &TaskResources) -> bool {
    first
        .package_directory
        .as_ref()
        .zip(second.package_directory.as_ref())
        .is_some_and(
            |((first_path, first_identity), (second_path, second_identity))| {
                first_identity == second_identity
                    || normalized_path(first_path) == normalized_path(second_path)
            },
        )
        || first
            .package_id
            .as_ref()
            .zip(second.package_id.as_ref())
            .is_some_and(|(first, second)| first == second)
}

fn outputs_overlap(first: Option<&OutputResource>, second: Option<&OutputResource>) -> bool {
    match (first, second) {
        (Some(OutputResource::File(first)), Some(OutputResource::File(second))) => {
            normalized_path(first) == normalized_path(second)
        }
        (Some(OutputResource::Directory(first)), Some(OutputResource::Directory(second))) => {
            path_contains(first, second) || path_contains(second, first)
        }
        (Some(OutputResource::Directory(directory)), Some(OutputResource::File(file)))
        | (Some(OutputResource::File(file)), Some(OutputResource::Directory(directory))) => {
            path_contains(directory, file)
        }
        _ => false,
    }
}

fn output_equal(first: Option<&OutputResource>, second: Option<&OutputResource>) -> bool {
    match (first, second) {
        (None, None) => true,
        (Some(OutputResource::File(first)), Some(OutputResource::File(second)))
        | (Some(OutputResource::Directory(first)), Some(OutputResource::Directory(second))) => {
            normalized_path(first) == normalized_path(second)
        }
        _ => false,
    }
}

fn output_overlaps_inputs(
    output: &Option<OutputResource>,
    inputs: &[(PathBuf, Option<cakesplitter_core::NativeFileIdentity>)],
) -> bool {
    output.as_ref().is_some_and(|output| {
        inputs.iter().any(|(input, _)| match output {
            OutputResource::File(path) => normalized_path(path) == normalized_path(input),
            OutputResource::Directory(path) => path_contains(path, input),
        })
    })
}

fn path_contains(directory: &Path, candidate: &Path) -> bool {
    let directory = normalized_path(directory);
    let candidate = normalized_path(candidate);
    candidate == directory
        || candidate
            .strip_prefix(&directory)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn candidate_primary_resource(spec: &TaskSpec) -> &Path {
    match spec {
        TaskSpec::Split { source_path, .. } => source_path,
        TaskSpec::Merge { manifest_path, .. }
        | TaskSpec::Inspect { manifest_path, .. }
        | TaskSpec::Verify { manifest_path, .. } => manifest_path,
    }
}

fn candidate_output_path(spec: &TaskSpec) -> Option<&Path> {
    match spec {
        TaskSpec::Split {
            output_directory, ..
        } => Some(output_directory),
        TaskSpec::Merge { output_path, .. } => Some(output_path),
        _ => None,
    }
}

fn conflict(
    task_id: &str,
    class: ConflictClass,
    conflict_type: ConflictType,
    resource: &Path,
    action: &str,
) -> TaskConflict {
    TaskConflict {
        conflicting_task_id: task_id.to_owned(),
        class,
        conflict_type,
        affected_resource: resource
            .file_name()
            .and_then(|name| name.to_str())
            .map(crate::sanitize_label)
            .unwrap_or_else(|| "selected resource".to_owned()),
        recommended_action: action.to_owned(),
    }
}

fn conflict_rank(class: ConflictClass) -> u8 {
    match class {
        ConflictClass::InformationalOverlap => 0,
        ConflictClass::RecoverableConflict => 1,
        ConflictClass::DuplicateTask => 2,
        ConflictClass::HardConflict => 3,
    }
}

fn worker_loop(inner: Arc<EngineInner>) {
    'worker: loop {
        let record = loop {
            if inner.shutdown.load(Ordering::Acquire) {
                break 'worker;
            }
            if !inner.clearing.load(Ordering::Acquire) {
                let admission = inner
                    .admission
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let next = if inner.clearing.load(Ordering::Acquire) {
                    None
                } else {
                    inner.store.next_scheduled_task().ok().flatten()
                };
                if let Some(record) = next {
                    drop(admission);
                    break record;
                }
            }
            let generation = inner
                .scheduler_generation
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _ = inner
                .scheduler_wake
                .wait_timeout(generation, Duration::from_secs(1))
                .unwrap_or_else(|error| error.into_inner());
        };
        let task_id = record.id.clone();
        let admission = inner
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if inner.clearing.load(Ordering::Acquire) {
            continue;
        }
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
            task.attempt_count = task
                .attempt_count
                .checked_add(1)
                .ok_or(StoreError::CorruptState)?;
            task.started_at.get_or_insert_with(now);
            task.finished_at = None;
            task.duration_ms = None;
            task.transition(TaskStatus::Running)
                .map_err(|_| StoreError::InvalidTransition)
        });
        drop(admission);
        let result = match running {
            Ok(mut running) => match preflight_runtime(&inner, &running) {
                Ok(preflight) => {
                    running.preflight = Some(preflight.clone());
                    let persisted = update_inner(&inner, &task_id, epoch, |task| {
                        task.preflight = Some(preflight);
                        Ok(())
                    });
                    match persisted {
                        Ok(persisted) => execute_task(&inner, &persisted, token.clone()),
                        Err(error) => Err(EngineError::Store(error)),
                    }
                }
                Err(error) => Err(error),
            },
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
                finish_task(task);
                task.transition(TaskStatus::Completed)
                    .map_err(|_| StoreError::InvalidTransition)
            }),
            Err(EngineError::Core(CoreError::Cancelled)) => {
                if current.status == TaskStatus::Interrupted {
                    continue;
                }
                update_inner(&inner, &task_id, current.epoch, |task| {
                    if task.status != TaskStatus::Cancelling {
                        task.transition(TaskStatus::Cancelling)
                            .map_err(|_| StoreError::InvalidTransition)?;
                    }
                    finish_task(task);
                    task.transition(TaskStatus::Cancelled)
                        .map_err(|_| StoreError::InvalidTransition)
                })
            }
            Err(error) => {
                let (status, failure) = classify_error(&error, current.attempt_count);
                update_inner(&inner, &task_id, current.epoch, |task| {
                    task.failure = Some(failure);
                    if status.is_terminal() {
                        finish_task(task);
                    }
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

fn classify_error(error: &EngineError, attempt: u32) -> (TaskStatus, TaskFailure) {
    if let EngineError::Core(CoreError::Io { source, .. }) = error {
        if source.kind() == std::io::ErrorKind::PermissionDenied {
            return (
                TaskStatus::PermissionRequired,
                TaskFailure::classified(
                    "permission_required",
                    "Permission is required to continue this local task.",
                    crate::redact_text(&format!("Filesystem permission failure: {source}")),
                    ErrorCategory::Permission,
                    true,
                    RecoveryAction::Retry,
                    attempt,
                ),
            );
        }
    }
    let code = match error {
        EngineError::Core(core) => core.code().to_owned(),
        EngineError::InsufficientSpace { .. } => "insufficient_space".to_owned(),
        EngineError::TaskConflict(_) => "task_conflict".to_owned(),
        EngineError::PreflightBlocked => "preflight_blocked".to_owned(),
        EngineError::Store(_) => "task_store_error".to_owned(),
        _ => "desktop_runtime_error".to_owned(),
    };
    let (category, retryable, recovery_action) = error_policy(error);
    (
        TaskStatus::Failed,
        TaskFailure::classified(
            code,
            redacted_error_message(error),
            crate::redact_text(&error.to_string()),
            category,
            retryable,
            recovery_action,
            attempt,
        ),
    )
}

fn error_policy(error: &EngineError) -> (ErrorCategory, bool, RecoveryAction) {
    match error {
        EngineError::InsufficientSpace { .. } => {
            (ErrorCategory::Space, true, RecoveryAction::FreeSpace)
        }
        EngineError::TaskConflict(_) | EngineError::PreflightBlocked => {
            (ErrorCategory::Queue, true, RecoveryAction::RemoveConflict)
        }
        EngineError::Core(CoreError::Io { source, .. })
            if matches!(
                source.kind(),
                std::io::ErrorKind::PermissionDenied
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::Interrupted
            ) =>
        {
            (
                ErrorCategory::Permission,
                true,
                RecoveryAction::CloseConflictingApplication,
            )
        }
        EngineError::Core(CoreError::SourceChanged | CoreError::InvalidInput(_)) => {
            (ErrorCategory::Source, false, RecoveryAction::ReselectSource)
        }
        EngineError::Core(
            CoreError::DestinationIdentityChanged(_)
            | CoreError::StagedIdentityChanged(_)
            | CoreError::StagedContentChanged(_)
            | CoreError::UnsafeFilesystemPath(_),
        ) => (
            ErrorCategory::Destination,
            false,
            RecoveryAction::ReselectDestination,
        ),
        EngineError::Core(CoreError::Collision(_)) => (
            ErrorCategory::Destination,
            true,
            RecoveryAction::RemoveConflict,
        ),
        EngineError::Core(CoreError::PackageIdentityChanged(_)) => (
            ErrorCategory::Package,
            false,
            RecoveryAction::ReselectPackage,
        ),
        EngineError::Core(
            CoreError::InvalidJson(_)
            | CoreError::InvalidManifest(_)
            | CoreError::MissingSlices(_)
            | CoreError::UnexpectedSlices(_)
            | CoreError::CorruptedSlices(_)
            | CoreError::FinalHashMismatch { .. },
        ) => (ErrorCategory::Integrity, false, RecoveryAction::None),
        EngineError::Core(CoreError::ResumeRejected(_)) => (
            ErrorCategory::Recovery,
            false,
            RecoveryAction::ReselectPackage,
        ),
        EngineError::Core(CoreError::AtomicFinalizationUnsupported(_)) => {
            (ErrorCategory::Capability, false, RecoveryAction::None)
        }
        EngineError::Store(_) => (ErrorCategory::Storage, false, RecoveryAction::None),
        _ => (ErrorCategory::Internal, false, RecoveryAction::None),
    }
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
        EngineError::Core(CoreError::SourceChanged) => {
            "The selected source changed. Select the original source again.".to_owned()
        }
        EngineError::Core(CoreError::InvalidJson(_) | CoreError::InvalidManifest(_)) => {
            "The Cake Manifest is malformed or unsupported.".to_owned()
        }
        EngineError::Core(CoreError::MissingSlices(_)) => {
            "The Cake Package is incomplete because one or more Slices are missing.".to_owned()
        }
        EngineError::Core(CoreError::UnexpectedSlices(_)) => {
            "The Cake Package contains unexpected Slice files.".to_owned()
        }
        EngineError::Core(CoreError::CorruptedSlices(_)) => {
            "One or more Cake Package Slices failed SHA-256 verification.".to_owned()
        }
        EngineError::Core(CoreError::FinalHashMismatch { .. }) => {
            "The rebuilt output did not match the Manifest SHA-256.".to_owned()
        }
        EngineError::Core(CoreError::ResumeRejected(_)) => {
            "Recovery is not safe for the current local files. Reselect the original objects."
                .to_owned()
        }
        EngineError::TaskConflict(_) => {
            "Another nonterminal task conflicts with these selected resources.".to_owned()
        }
        EngineError::PreflightBlocked => {
            "Native preflight blocked this task before execution.".to_owned()
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
            let (_, failure) = classify_error(&error, 1);
            assert!(!failure.message.contains("Private Name"));
            assert!(!failure.message.contains("secret.bin"));
            assert!(!failure.technical_message.contains("Private Name"));
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
    fn preflight_distinguishes_minimum_space_from_the_recommended_margin() {
        let root = tempdir().unwrap();
        let store = Arc::new(TaskStore::open(root.path()).unwrap());
        let engine = TaskEngine {
            inner: Arc::new(EngineInner {
                store,
                application_version: "0.5.0".to_owned(),
                controls: Mutex::new(HashMap::new()),
                admission: Mutex::new(()),
                listener: Arc::new(|_| {}),
                scheduler_generation: Mutex::new(0),
                scheduler_wake: Condvar::new(),
                clearing: AtomicBool::new(false),
                shutdown: AtomicBool::new(false),
                client_handles: AtomicU64::new(1),
                diagnostic_bundle_count: AtomicU64::new(0),
            }),
        };
        let spec = TaskSpec::Split {
            source_path: PathBuf::from(r"C:\fixtures\source.bin"),
            output_directory: PathBuf::from(r"D:\fixtures\package"),
            slice_size: 1024,
            package_id: Uuid::new_v4().to_string(),
            created_at: default_created_at(),
        };
        let plan = ProcessingPlan {
            minimum_required_bytes: 100,
            recommended_free_bytes: 200,
            ..ProcessingPlan::default()
        };
        let blocked = engine
            .preflight_for_candidate(&spec, &plan, 99, None)
            .unwrap();
        assert_eq!(blocked.state, PreflightState::Blocked);
        assert!(
            blocked
                .warnings
                .iter()
                .any(|warning| warning.code == "insufficient_space")
        );
        let warning = engine
            .preflight_for_candidate(&spec, &plan, 150, None)
            .unwrap();
        assert_eq!(warning.state, PreflightState::ReadyWithWarning);
        assert!(
            warning
                .warnings
                .iter()
                .any(|warning| warning.code == "space_margin_low")
        );
        let ready = engine
            .preflight_for_candidate(&spec, &plan, 200, None)
            .unwrap();
        assert_eq!(ready.state, PreflightState::Ready);
    }

    #[test]
    fn error_taxonomy_makes_only_transient_permission_and_space_failures_retryable() {
        assert_eq!(
            error_policy(&EngineError::InsufficientSpace {
                required: 2,
                available: 1
            }),
            (ErrorCategory::Space, true, RecoveryAction::FreeSpace)
        );
        assert_eq!(
            error_policy(&EngineError::Core(CoreError::Io {
                path: PathBuf::from(r"C:\private\locked.bin"),
                source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            })),
            (
                ErrorCategory::Permission,
                true,
                RecoveryAction::CloseConflictingApplication
            )
        );
        assert_eq!(
            error_policy(&EngineError::Core(CoreError::SourceChanged)),
            (ErrorCategory::Source, false, RecoveryAction::ReselectSource)
        );
        assert_eq!(
            error_policy(&EngineError::Core(CoreError::PackageIdentityChanged(
                PathBuf::from(r"C:\private\package.cake.json")
            ))),
            (
                ErrorCategory::Package,
                false,
                RecoveryAction::ReselectPackage
            )
        );
    }

    #[test]
    fn progress_persistence_is_throttled_but_final_state_is_never_delayed() {
        let gate = Mutex::new(Instant::now());
        let partial = Progress {
            operation: "split",
            bytes_processed: 50,
            total_bytes: 100,
            current_slice: 1,
            slice_count: 2,
        };
        assert!(!should_persist_progress(&gate, &partial));
        let complete = Progress {
            bytes_processed: 100,
            ..partial.clone()
        };
        assert!(should_persist_progress(&gate, &complete));

        let elapsed_gate = Mutex::new(Instant::now() - PROGRESS_WRITE_INTERVAL);
        assert!(should_persist_progress(&elapsed_gate, &partial));
    }

    #[test]
    fn full_capacity_rejects_before_filesystem_access_or_execution_allocation() {
        let root = tempdir().unwrap();
        let store = Arc::new(TaskStore::open(root.path()).unwrap());
        for _ in 0..MAX_NONTERMINAL_TASKS {
            let epoch = store.epoch().unwrap();
            let mut record = TaskRecord::new(
                "0.4.0",
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
                    ..ProcessingPlan::default()
                },
            );
            record.transition(TaskStatus::Queued).unwrap();
            store.insert(record).unwrap();
        }
        let inner = Arc::new(EngineInner {
            store: Arc::clone(&store),
            application_version: "0.4.0".to_owned(),
            controls: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            listener: Arc::new(|_| {}),
            scheduler_generation: Mutex::new(0),
            scheduler_wake: Condvar::new(),
            clearing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            client_handles: AtomicU64::new(1),
            diagnostic_bundle_count: AtomicU64::new(0),
        });
        let engine = TaskEngine { inner };

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
        fs::write(&source, b"bounded").unwrap();
        let store = Arc::new(TaskStore::open(&app_data).unwrap());
        let inner = Arc::new(EngineInner {
            store: Arc::clone(&store),
            application_version: "0.4.0".to_owned(),
            controls: Mutex::new(HashMap::new()),
            admission: Mutex::new(()),
            listener: Arc::new(|_| {}),
            scheduler_generation: Mutex::new(0),
            scheduler_wake: Condvar::new(),
            clearing: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            client_handles: AtomicU64::new(1),
            diagnostic_bundle_count: AtomicU64::new(0),
        });
        let engine = TaskEngine { inner };
        let mut attempts = Vec::new();
        for index in 0..MAX_NONTERMINAL_TASKS * 2 {
            let engine = engine.clone();
            let source = source.clone();
            let output = root.path().join(format!("output-{index}"));
            fs::create_dir(&output).unwrap();
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
