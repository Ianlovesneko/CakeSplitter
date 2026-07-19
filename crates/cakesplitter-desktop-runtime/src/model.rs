use std::path::PathBuf;

use cakesplitter_core::{
    DirectoryFingerprint, MergeResumeData, PackageBinding, PackageInspection, SourceFingerprint,
    SplitResumeData,
};
use cakesplitter_format::FORMAT_VERSION;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::TASK_STATE_SCHEMA_VERSION;

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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DesktopPreferences {
    pub default_slice_size: u64,
    pub confirm_destructive_actions: bool,
    pub reduce_motion: bool,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            default_slice_size: 512 * 1024 * 1024,
            confirm_destructive_actions: true,
            reduce_motion: false,
        }
    }
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
#[serde(rename_all = "camelCase")]
pub struct ProcessingPlan {
    pub total_bytes: u64,
    pub slice_size: u64,
    pub slice_count: u64,
    pub required_free_bytes: u64,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFailure {
    pub code: String,
    pub message: String,
}

impl TaskFailure {
    pub fn bounded(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: truncate(&code.into(), 80),
            message: truncate(&message.into(), 2_000),
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
    pub format_version: String,
    pub epoch: u64,
    pub revision: u64,
    pub display_name: String,
    pub destination_name: Option<String>,
    pub spec: TaskSpec,
    pub plan: ProcessingPlan,
    pub source_identity: Option<SourceFingerprint>,
    pub destination_identity: Option<DirectoryFingerprint>,
    pub checkpoint: Option<RecoveryCheckpoint>,
    pub progress: TaskProgress,
    pub status: TaskStatus,
    pub failure: Option<TaskFailure>,
    pub result: Option<TaskResult>,
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
            format_version: FORMAT_VERSION.to_owned(),
            epoch,
            revision: 0,
            display_name,
            destination_name,
            spec,
            plan,
            source_identity: None,
            destination_identity: None,
            checkpoint: None,
            progress: TaskProgress::default(),
            status: TaskStatus::Planned,
            failure: None,
            result: None,
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
        self.checkpoint.is_some()
            && matches!(
                self.status,
                TaskStatus::Interrupted
                    | TaskStatus::PermissionRequired
                    | TaskStatus::Failed
                    | TaskStatus::Cancelled
            )
    }

    pub fn snapshot(&self) -> TaskSnapshot {
        TaskSnapshot {
            id: self.id.clone(),
            revision: self.revision,
            operation: self.operation,
            application_version: self.application_version.clone(),
            format_version: self.format_version.clone(),
            display_name: self.display_name.clone(),
            destination_name: self.destination_name.clone(),
            plan: self.plan.clone(),
            progress: self.progress.clone(),
            status: self.status,
            failure: self.failure.clone(),
            result: self.result.clone(),
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
    pub display_name: String,
    pub destination_name: Option<String>,
    pub plan: ProcessingPlan,
    pub progress: TaskProgress,
    pub status: TaskStatus,
    pub failure: Option<TaskFailure>,
    pub result: Option<TaskResult>,
    pub recovery_eligible: bool,
    pub created_at: String,
    pub updated_at: String,
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
