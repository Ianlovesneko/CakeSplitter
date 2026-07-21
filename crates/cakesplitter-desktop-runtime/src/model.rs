use std::path::PathBuf;

use cakesplitter_core::{
    DirectoryFingerprint, MergeResumeData, PackageBinding, PackageInspection, SourceFingerprint,
    SplitResumeData,
};
use cakesplitter_format::FORMAT_VERSION;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    DEFAULT_HISTORY_RETENTION_DAYS, MAX_HISTORY_RETENTION_DAYS, MAX_PREFLIGHT_WARNINGS,
    MAX_TASK_HISTORY, SCHEDULER_VERSION, TASK_STATE_SCHEMA_VERSION,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartupRecoveryState {
    Ready,
    RecoveryRequired,
    Quarantined,
    CapacityExceeded,
    UnsupportedVersion,
    Corrupt,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartupRecoveryReport {
    pub state: StartupRecoveryState,
    pub recovered_tasks: usize,
    pub quarantined_records: usize,
    pub capacity_exceeded_records: usize,
}

impl Default for StartupRecoveryReport {
    fn default() -> Self {
        Self {
            state: StartupRecoveryState::Ready,
            recovered_tasks: 0,
            quarantined_records: 0,
            capacity_exceeded_records: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopPreferences {
    pub default_slice_size: u64,
    pub confirm_destructive_actions: bool,
    pub reduce_motion: bool,
    pub maximum_terminal_history: u32,
    pub terminal_history_days: u32,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            default_slice_size: 512 * 1024 * 1024,
            confirm_destructive_actions: true,
            reduce_motion: false,
            maximum_terminal_history: MAX_TASK_HISTORY as u32,
            terminal_history_days: DEFAULT_HISTORY_RETENTION_DAYS,
        }
    }
}

impl DesktopPreferences {
    pub fn validate(&self) -> bool {
        self.default_slice_size > 0
            && self.maximum_terminal_history > 0
            && self.maximum_terminal_history <= MAX_TASK_HISTORY as u32
            && self.terminal_history_days > 0
            && self.terminal_history_days <= MAX_HISTORY_RETENTION_DAYS
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskPriority {
    High,
    #[default]
    Normal,
    Low,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueDirection {
    Earlier,
    Later,
}

impl TaskPriority {
    pub fn rank(self) -> u64 {
        match self {
            Self::High => 0,
            Self::Normal => 1,
            Self::Low => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreflightState {
    #[default]
    Ready,
    ReadyWithWarning,
    Blocked,
    ReselectionRequired,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictClass {
    #[default]
    InformationalOverlap,
    RecoverableConflict,
    DuplicateTask,
    HardConflict,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictType {
    #[default]
    SharedInput,
    SameSource,
    SameManifest,
    SamePackageDirectory,
    SameOutput,
    OverlappingOutput,
    SamePackageId,
    SourceUsedAsDestination,
    DestinationInsidePackage,
    DuplicateOperation,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskConflict {
    pub conflicting_task_id: String,
    pub class: ConflictClass,
    pub conflict_type: ConflictType,
    pub affected_resource: String,
    pub recommended_action: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PreflightWarning {
    pub code: String,
    pub message: String,
}

impl PreflightWarning {
    pub fn bounded(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: truncate(&code.into(), 80),
            message: truncate(&message.into(), 500),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct PreflightResult {
    pub state: PreflightState,
    pub checked_at: String,
    pub minimum_required_bytes: u64,
    pub recommended_free_bytes: u64,
    pub available_free_bytes: u64,
    pub temporary_bytes: u64,
    pub recovery_overhead_bytes: u64,
    pub expected_output_count: u64,
    pub warnings: Vec<PreflightWarning>,
    pub conflicts: Vec<TaskConflict>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageSummary {
    pub database_bytes: u64,
    pub active_tasks: u64,
    pub nonterminal_tasks: u64,
    pub terminal_history_tasks: u64,
    pub quarantined_records: u64,
    pub incomplete_output_references: u64,
    pub diagnostic_bundle_count: u64,
    pub maximum_terminal_history: u32,
    pub terminal_history_days: u32,
}

impl PreflightResult {
    pub fn bounded_warnings(mut self) -> Self {
        self.warnings.truncate(MAX_PREFLIGHT_WARNINGS);
        self.conflicts.truncate(MAX_PREFLIGHT_WARNINGS);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCategory {
    Source,
    Destination,
    Package,
    Integrity,
    Permission,
    Space,
    Recovery,
    Queue,
    Storage,
    Capability,
    #[default]
    Internal,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryAction {
    Retry,
    ReselectSource,
    ReselectDestination,
    ReselectPackage,
    FreeSpace,
    CloseConflictingApplication,
    RemoveConflict,
    #[default]
    None,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskOperation {
    Split,
    Merge,
    Inspect,
    Verify,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Planned,
    Queued,
    Running,
    Pausing,
    Paused,
    Resuming,
    Cancelling,
    Cancelled,
    Interrupted,
    PermissionRequired,
    Failed,
    Completed,
}

impl TaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Cancelled | Self::Failed | Self::Completed)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Planned, Self::Queued)
                | (Self::Planned, Self::Cancelled)
                | (Self::Queued, Self::Running)
                | (Self::Queued, Self::Cancelled)
                | (Self::Running, Self::Pausing)
                | (Self::Running, Self::Cancelling)
                | (Self::Running, Self::Interrupted)
                | (Self::Running, Self::PermissionRequired)
                | (Self::Running, Self::Failed)
                | (Self::Running, Self::Completed)
                | (Self::Pausing, Self::Paused)
                | (Self::Pausing, Self::Cancelling)
                | (Self::Pausing, Self::Interrupted)
                | (Self::Paused, Self::Resuming)
                | (Self::Paused, Self::Cancelling)
                | (Self::Paused, Self::Interrupted)
                | (Self::Resuming, Self::Running)
                | (Self::Resuming, Self::Cancelling)
                | (Self::Resuming, Self::Interrupted)
                | (Self::Cancelling, Self::Cancelled)
                | (Self::Cancelling, Self::Interrupted)
                | (Self::Cancelling, Self::Failed)
                | (Self::Interrupted, Self::Queued)
                | (Self::Interrupted, Self::Cancelled)
                | (Self::PermissionRequired, Self::Queued)
                | (Self::PermissionRequired, Self::Cancelled)
                | (Self::Failed, Self::Queued)
                | (Self::Failed, Self::Cancelled)
                | (Self::Cancelled, Self::Queued)
        )
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TaskSpec {
    Split {
        source_path: PathBuf,
        output_directory: PathBuf,
        slice_size: u64,
        package_id: String,
        created_at: String,
    },
    Merge {
        manifest_path: PathBuf,
        output_path: PathBuf,
        package_binding: PackageBinding,
    },
    Inspect {
        manifest_path: PathBuf,
        verify_hashes: bool,
        package_binding: PackageBinding,
    },
    Verify {
        manifest_path: PathBuf,
        package_binding: PackageBinding,
    },
}

impl TaskSpec {
    pub fn operation(&self) -> TaskOperation {
        match self {
            Self::Split { .. } => TaskOperation::Split,
            Self::Merge { .. } => TaskOperation::Merge,
            Self::Inspect { .. } => TaskOperation::Inspect,
            Self::Verify { .. } => TaskOperation::Verify,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessingPlan {
    pub total_bytes: u64,
    pub slice_size: u64,
    pub slice_count: u64,
    pub required_free_bytes: u64,
    pub minimum_required_bytes: u64,
    pub recommended_free_bytes: u64,
    pub available_free_bytes: u64,
    pub temporary_bytes: u64,
    pub recovery_overhead_bytes: u64,
    pub expected_output_count: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgress {
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub current_slice: u64,
    pub slice_count: u64,
    pub stage: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RecoveryCheckpoint {
    Split(SplitResumeData),
    Merge(MergeResumeData),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskFailure {
    pub code: String,
    pub message: String,
    pub technical_message: String,
    pub category: ErrorCategory,
    pub retryable: bool,
    pub recovery_action: RecoveryAction,
    pub occurred_at: String,
    pub attempt: u32,
}

impl Default for TaskFailure {
    fn default() -> Self {
        Self {
            code: String::new(),
            message: String::new(),
            technical_message: String::new(),
            category: ErrorCategory::Internal,
            retryable: false,
            recovery_action: RecoveryAction::None,
            occurred_at: String::new(),
            attempt: 0,
        }
    }
}

impl TaskFailure {
    pub fn bounded(code: impl Into<String>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: truncate(&code.into(), 80),
            message: truncate(&message, 2_000),
            technical_message: truncate(&message, 2_000),
            occurred_at: now(),
            ..Self::default()
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn classified(
        code: impl Into<String>,
        message: impl Into<String>,
        technical_message: impl Into<String>,
        category: ErrorCategory,
        retryable: bool,
        recovery_action: RecoveryAction,
        attempt: u32,
    ) -> Self {
        Self {
            code: truncate(&code.into(), 80),
            message: truncate(&message.into(), 2_000),
            technical_message: truncate(&technical_message.into(), 2_000),
            category,
            retryable,
            recovery_action,
            occurred_at: now(),
            attempt,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionSummary {
    pub package_id: String,
    pub format_version: String,
    pub original_filename: String,
    pub original_size: u64,
    pub original_sha256: String,
    pub expected_slice_count: u64,
    pub found_slice_count: u64,
    pub missing: Vec<String>,
    pub corrupted: Vec<String>,
    pub unexpected: Vec<String>,
    pub verified: bool,
}

impl From<PackageInspection> for InspectionSummary {
    fn from(value: PackageInspection) -> Self {
        Self {
            package_id: value.manifest.package_id,
            format_version: value.manifest.version,
            original_filename: value.manifest.original.filename,
            original_size: value.manifest.original.size,
            original_sha256: value.manifest.original.sha256,
            expected_slice_count: value.expected_slice_count,
            found_slice_count: value.found_slice_count,
            missing: value.missing,
            corrupted: value.corrupted,
            unexpected: value.unexpected,
            verified: value.verified,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TaskResult {
    Split {
        #[serde(rename = "manifestFilename", alias = "manifest_filename")]
        manifest_filename: String,
        #[serde(rename = "sourceSha256", alias = "source_sha256")]
        source_sha256: String,
    },
    Merge {
        #[serde(rename = "outputFilename", alias = "output_filename")]
        output_filename: String,
        #[serde(rename = "outputSha256", alias = "output_sha256")]
        output_sha256: String,
    },
    Inspection {
        inspection: InspectionSummary,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub operation: TaskOperation,
    pub application_version: String,
    pub schema_version: u32,
    #[serde(default = "scheduler_version")]
    pub scheduler_version: u32,
    pub format_version: String,
    pub epoch: u64,
    pub revision: u64,
    #[serde(default)]
    pub priority: TaskPriority,
    #[serde(default)]
    pub queue_order: u64,
    pub display_name: String,
    pub destination_name: Option<String>,
    pub spec: TaskSpec,
    pub plan: ProcessingPlan,
    #[serde(default)]
    pub preflight: Option<PreflightResult>,
    pub source_identity: Option<SourceFingerprint>,
    pub destination_identity: Option<DirectoryFingerprint>,
    pub checkpoint: Option<RecoveryCheckpoint>,
    pub progress: TaskProgress,
    pub status: TaskStatus,
    pub failure: Option<TaskFailure>,
    #[serde(default)]
    pub failure_history: Vec<TaskFailure>,
    pub result: Option<TaskResult>,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
}

impl TaskRecord {
    pub fn new(
        application_version: &str,
        epoch: u64,
        display_name: String,
        destination_name: Option<String>,
        spec: TaskSpec,
        plan: ProcessingPlan,
    ) -> Self {
        let now = now();
        Self {
            id: Uuid::new_v4().to_string(),
            operation: spec.operation(),
            application_version: application_version.to_owned(),
            schema_version: TASK_STATE_SCHEMA_VERSION,
            scheduler_version: SCHEDULER_VERSION,
            format_version: FORMAT_VERSION.to_owned(),
            epoch,
            revision: 0,
            priority: TaskPriority::Normal,
            queue_order: 0,
            display_name,
            destination_name,
            spec,
            plan,
            preflight: None,
            source_identity: None,
            destination_identity: None,
            checkpoint: None,
            progress: TaskProgress::default(),
            status: TaskStatus::Planned,
            failure: None,
            failure_history: Vec::new(),
            result: None,
            attempt_count: 0,
            started_at: None,
            finished_at: None,
            duration_ms: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    pub fn transition(&mut self, next: TaskStatus) -> Result<(), InvalidTransition> {
        if !self.status.can_transition_to(next) {
            return Err(InvalidTransition {
                current: self.status,
                requested: next,
            });
        }
        self.status = next;
        self.updated_at = now();
        Ok(())
    }

    pub fn recovery_eligible(&self) -> bool {
        matches!(
            self.status,
            TaskStatus::Interrupted
                | TaskStatus::PermissionRequired
                | TaskStatus::Failed
                | TaskStatus::Cancelled
        ) && (self.checkpoint.is_some()
            || self
                .failure
                .as_ref()
                .is_some_and(|failure| failure.retryable))
    }

    pub fn snapshot(&self) -> TaskSnapshot {
        self.snapshot_with_position(None)
    }

    pub fn snapshot_with_position(&self, queue_position: Option<u64>) -> TaskSnapshot {
        TaskSnapshot {
            id: self.id.clone(),
            revision: self.revision,
            operation: self.operation,
            application_version: self.application_version.clone(),
            format_version: self.format_version.clone(),
            priority: self.priority,
            queue_order: self.queue_order,
            queue_position,
            display_name: self.display_name.clone(),
            destination_name: self.destination_name.clone(),
            plan: self.plan.clone(),
            preflight: self.preflight.clone(),
            progress: self.progress.clone(),
            status: self.status,
            failure: self.failure.clone(),
            failure_history: self.failure_history.clone(),
            result: self.result.clone(),
            attempt_count: self.attempt_count,
            started_at: self.started_at.clone(),
            finished_at: self.finished_at.clone(),
            duration_ms: self.duration_ms,
            recovery_eligible: self.recovery_eligible(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    pub id: String,
    pub revision: u64,
    pub operation: TaskOperation,
    pub application_version: String,
    pub format_version: String,
    pub priority: TaskPriority,
    pub queue_order: u64,
    pub queue_position: Option<u64>,
    pub display_name: String,
    pub destination_name: Option<String>,
    pub plan: ProcessingPlan,
    pub preflight: Option<PreflightResult>,
    pub progress: TaskProgress,
    pub status: TaskStatus,
    pub failure: Option<TaskFailure>,
    pub failure_history: Vec<TaskFailure>,
    pub result: Option<TaskResult>,
    pub attempt_count: u32,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub duration_ms: Option<u64>,
    pub recovery_eligible: bool,
    pub created_at: String,
    pub updated_at: String,
}

fn scheduler_version() -> u32 {
    SCHEDULER_VERSION
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidTransition {
    pub current: TaskStatus,
    pub requested: TaskStatus,
}

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn truncate(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_completion_and_allows_recovery_transitions() {
        assert!(!TaskStatus::Queued.can_transition_to(TaskStatus::Completed));
        assert!(TaskStatus::Running.can_transition_to(TaskStatus::Completed));
        assert!(TaskStatus::Paused.can_transition_to(TaskStatus::Interrupted));
        assert!(TaskStatus::Interrupted.can_transition_to(TaskStatus::Queued));
    }

    #[test]
    fn bounded_failures_do_not_store_unlimited_diagnostic_text() {
        let failure = TaskFailure::bounded("x".repeat(200), "y".repeat(4_000));
        assert_eq!(failure.code.chars().count(), 80);
        assert_eq!(failure.message.chars().count(), 2_000);
    }

    #[test]
    fn completion_results_use_the_frontend_ipc_field_contract() {
        let split = serde_json::to_value(TaskResult::Split {
            manifest_filename: "sample.bin.cake.json".to_owned(),
            source_sha256: "a".repeat(64),
        })
        .unwrap();
        assert_eq!(
            split,
            serde_json::json!({
                "type": "split",
                "manifestFilename": "sample.bin.cake.json",
                "sourceSha256": "a".repeat(64),
            })
        );

        let legacy = serde_json::json!({
            "type": "merge",
            "output_filename": "sample.bin",
            "output_sha256": "b".repeat(64),
        });
        let decoded: TaskResult = serde_json::from_value(legacy).unwrap();
        assert!(matches!(decoded, TaskResult::Merge { .. }));
    }
}
