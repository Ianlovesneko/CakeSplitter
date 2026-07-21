use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use cakesplitter_core::{CoreError, DirectoryFingerprint, DirectoryIdentityAuthority};
use cakesplitter_format::FORMAT_VERSION;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    MAX_DIAGNOSTIC_FILE_BYTES, MAX_DIAGNOSTIC_TASKS, MAX_RECEIPT_BYTES, StorageSummary,
    TaskOperation, TaskRecord, TaskResult, TaskSpec, TaskStatus, masked_path, redact_text,
    sanitize_label,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReceiptFormat {
    Markdown,
    Json,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub path: PathBuf,
    pub display_name: String,
    pub bytes_written: u64,
    pub kind: String,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("the selected task cannot be exported in its current state")]
    TaskNotExportable,
    #[error("the export destination already exists")]
    Collision,
    #[error("the export exceeds the supported size limit")]
    SizeLimit,
    #[error("the export path is unsafe")]
    UnsafePath,
    #[error("the export destination changed during publication")]
    DestinationIdentityChanged,
    #[error("export I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("export serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationReceipt {
    cakesplitter_version: String,
    cake_package_format: String,
    task_id: String,
    operation: String,
    started_at: Option<String>,
    ended_at: Option<String>,
    duration_ms: Option<u64>,
    source: String,
    destination: Option<String>,
    file_size: u64,
    slice_size: u64,
    slice_count: u64,
    result: String,
    sha256: Option<String>,
    warnings: Vec<String>,
    error_code: Option<String>,
    recovery_status: String,
    validation_summary: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticTaskSummary {
    task_id: String,
    operation: String,
    status: String,
    priority: String,
    display_name: String,
    destination_name: Option<String>,
    bytes_processed: u64,
    total_bytes: u64,
    slice_count: u64,
    attempt_count: u32,
    error_code: Option<String>,
    updated_at: String,
}

pub fn export_operation_receipt(
    record: &TaskRecord,
    output_path: &Path,
    expected_parent: &DirectoryFingerprint,
    format: ReceiptFormat,
    include_path_detail: bool,
) -> Result<ExportSummary, ExportError> {
    if !matches!(record.status, TaskStatus::Completed | TaskStatus::Failed) {
        return Err(ExportError::TaskNotExportable);
    }
    let parent = output_path.parent().ok_or(ExportError::UnsafePath)?;
    let authority = acquire_export_authority(parent, expected_parent)?;
    validate_future_file(output_path)?;
    let receipt = build_receipt(record, include_path_detail);
    let bytes = match format {
        ReceiptFormat::Json => serde_json::to_vec_pretty(&receipt)?,
        ReceiptFormat::Markdown => receipt_markdown(&receipt).into_bytes(),
    };
    if bytes.len() > MAX_RECEIPT_BYTES {
        return Err(ExportError::SizeLimit);
    }
    authority.revalidate().map_err(map_identity_error)?;
    write_new_file(output_path, &bytes)?;
    Ok(ExportSummary {
        path: output_path.to_path_buf(),
        display_name: output_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(sanitize_label)
            .unwrap_or_else(|| "operation-receipt".to_owned()),
        bytes_written: bytes.len() as u64,
        kind: match format {
            ReceiptFormat::Markdown => "receipt-markdown",
            ReceiptFormat::Json => "receipt-json",
        }
        .to_owned(),
    })
}

pub fn export_diagnostic_bundle(
    output_parent: &Path,
    expected_parent: &DirectoryFingerprint,
    application_version: &str,
    records: &[TaskRecord],
    storage: &StorageSummary,
) -> Result<ExportSummary, ExportError> {
    let authority = acquire_export_authority(output_parent, expected_parent)?;
    authority.revalidate().map_err(map_identity_error)?;
    let directory_name = format!(
        "cakesplitter-diagnostic-bundle-{}",
        Utc::now().format("%Y%m%d-%H%M%S")
    );
    let directory = output_parent.join(&directory_name);
    match fs::create_dir(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ExportError::Collision);
        }
        Err(error) => return Err(error.into()),
    }

    let result = write_diagnostic_files(
        &directory,
        &authority,
        application_version,
        records,
        storage,
    );
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&directory);
        return Err(error);
    }
    let bytes_written = directory_bytes(&directory)?;
    Ok(ExportSummary {
        path: directory,
        display_name: directory_name,
        bytes_written,
        kind: "diagnostic-bundle".to_owned(),
    })
}

fn build_receipt(record: &TaskRecord, include_path_detail: bool) -> OperationReceipt {
    let (source, destination) = task_paths(record);
    let sha256 = match &record.result {
        Some(TaskResult::Split { source_sha256, .. }) => Some(source_sha256.clone()),
        Some(TaskResult::Merge { output_sha256, .. }) => Some(output_sha256.clone()),
        Some(TaskResult::Inspection { inspection }) => Some(inspection.original_sha256.clone()),
        None => None,
    };
    let warnings = record
        .preflight
        .as_ref()
        .map(|preflight| {
            preflight
                .warnings
                .iter()
                .map(|warning| redact_text(&format!("{}: {}", warning.code, warning.message)))
                .collect()
        })
        .unwrap_or_default();
    OperationReceipt {
        cakesplitter_version: record.application_version.clone(),
        cake_package_format: record.format_version.clone(),
        task_id: record.id.clone(),
        operation: operation_name(record.operation).to_owned(),
        started_at: record.started_at.clone(),
        ended_at: record.finished_at.clone(),
        duration_ms: record.duration_ms,
        source: masked_path(source, include_path_detail),
        destination: destination.map(|path| masked_path(path, include_path_detail)),
        file_size: record.plan.total_bytes,
        slice_size: record.plan.slice_size,
        slice_count: record.plan.slice_count,
        result: status_name(record.status).to_owned(),
        sha256,
        warnings,
        error_code: record.failure.as_ref().map(|failure| failure.code.clone()),
        recovery_status: if record.recovery_eligible() {
            "available-at-verified-slice-boundary"
        } else {
            "not-available"
        }
        .to_owned(),
        validation_summary: if record.status == TaskStatus::Completed {
            "The operation reached a verified final state.".to_owned()
        } else {
            "The operation failed safely; no successful final state is claimed.".to_owned()
        },
    }
}

fn receipt_markdown(receipt: &OperationReceipt) -> String {
    let warnings = if receipt.warnings.is_empty() {
        "None".to_owned()
    } else {
        receipt.warnings.join("; ")
    };
    format!(
        "# CakeSplitter Operation Receipt\n\n\
         - CakeSplitter version: `{}`\n\
         - Cake Package format: `{}`\n\
         - Task ID: `{}`\n\
         - Operation: `{}`\n\
         - Result: `{}`\n\
         - Source: `{}`\n\
         - Destination: `{}`\n\
         - File size: `{}` bytes\n\
         - Slice size: `{}` bytes\n\
         - Slice count: `{}`\n\
         - Started: `{}`\n\
         - Ended: `{}`\n\
         - Duration: `{}` ms\n\
         - SHA-256: `{}`\n\
         - Error code: `{}`\n\
         - Recovery: `{}`\n\
         - Warnings: {}\n\n\
         {}\n",
        receipt.cakesplitter_version,
        receipt.cake_package_format,
        receipt.task_id,
        receipt.operation,
        receipt.result,
        receipt.source,
        receipt.destination.as_deref().unwrap_or("Not applicable"),
        receipt.file_size,
        receipt.slice_size,
        receipt.slice_count,
        receipt.started_at.as_deref().unwrap_or("Not recorded"),
        receipt.ended_at.as_deref().unwrap_or("Not recorded"),
        receipt
            .duration_ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "Not recorded".to_owned()),
        receipt.sha256.as_deref().unwrap_or("Not available"),
        receipt.error_code.as_deref().unwrap_or("None"),
        receipt.recovery_status,
        warnings,
        receipt.validation_summary
    )
}

fn write_diagnostic_files(
    directory: &Path,
    authority: &DirectoryIdentityAuthority,
    application_version: &str,
    records: &[TaskRecord],
    storage: &StorageSummary,
) -> Result<(), ExportError> {
    let summaries = records
        .iter()
        .take(MAX_DIAGNOSTIC_TASKS)
        .map(|record| DiagnosticTaskSummary {
            task_id: record.id.clone(),
            operation: operation_name(record.operation).to_owned(),
            status: status_name(record.status).to_owned(),
            priority: priority_name(record.priority).to_owned(),
            display_name: sanitize_label(&record.display_name),
            destination_name: record.destination_name.as_deref().map(sanitize_label),
            bytes_processed: record.progress.bytes_processed,
            total_bytes: record.plan.total_bytes,
            slice_count: record.plan.slice_count,
            attempt_count: record.attempt_count,
            error_code: record.failure.as_ref().map(|failure| failure.code.clone()),
            updated_at: record.updated_at.clone(),
        })
        .collect::<Vec<_>>();
    let errors = records
        .iter()
        .filter_map(|record| {
            record.failure.as_ref().map(|failure| {
                serde_json::json!({
                    "taskId": record.id,
                    "code": failure.code,
                    "category": failure.category,
                    "retryable": failure.retryable,
                    "message": redact_text(&failure.message),
                    "technicalMessage": redact_text(&failure.technical_message),
                    "occurredAt": failure.occurred_at,
                })
            })
        })
        .take(20)
        .collect::<Vec<_>>();

    write_bundle_file(
        authority,
        directory,
        "README.md",
        b"# CakeSplitter Diagnostic Bundle\n\nGenerated locally after explicit user action. No file contents, native identity values, credentials, environment variables, or full paths are included by default.\n",
    )?;
    write_bundle_file(
        authority,
        directory,
        "app-summary.md",
        format!(
            "# App Summary\n\n- CakeSplitter: `{}`\n- Cake Package format: `{}`\n- Processing: local only\n- Platform: Windows x64\n",
            redact_text(application_version),
            FORMAT_VERSION
        )
        .as_bytes(),
    )?;
    write_json_file(authority, directory, "task-summary.json", &summaries)?;
    write_json_file(authority, directory, "recent-errors.json", &errors)?;
    write_json_file(
        authority,
        directory,
        "capability-report.json",
        &serde_json::json!({
            "platform": "windows-x64",
            "networkRequired": false,
            "automaticUpdates": false,
            "telemetry": false,
            "backgroundService": false,
            "resumeBoundary": "verified-slice"
        }),
    )?;
    write_json_file(authority, directory, "storage-summary.json", storage)?;
    write_bundle_file(
        authority,
        directory,
        "limits-and-settings.md",
        format!(
            "# Limits and Settings\n\n- Nonterminal task limit: 64\n- Active disk-intensive task limit: 1\n- Terminal history limit: {}\n- Terminal history age: {} days\n- Diagnostic task summaries: {}\n",
            storage.maximum_terminal_history,
            storage.terminal_history_days,
            MAX_DIAGNOSTIC_TASKS
        )
        .as_bytes(),
    )?;
    write_bundle_file(
        authority,
        directory,
        "privacy-notice.md",
        b"# Privacy Notice\n\nThis bundle stays local until you choose otherwise. Review every file before sharing it. Selected file contents, Slice contents, Manifests, full paths, usernames, environment variables, secrets, tokens, and native filesystem identities are omitted.\n",
    )?;
    Ok(())
}

fn task_paths(record: &TaskRecord) -> (&Path, Option<&Path>) {
    match &record.spec {
        TaskSpec::Split {
            source_path,
            output_directory,
            ..
        } => (source_path, Some(output_directory)),
        TaskSpec::Merge {
            manifest_path,
            output_path,
            ..
        } => (manifest_path, Some(output_path)),
        TaskSpec::Inspect { manifest_path, .. } | TaskSpec::Verify { manifest_path, .. } => {
            (manifest_path, None)
        }
    }
}

fn validate_future_file(path: &Path) -> Result<(), ExportError> {
    if !path.is_absolute()
        || path.file_name().is_none()
        || path.parent().is_none_or(|p| !p.is_dir())
    {
        return Err(ExportError::UnsafePath);
    }
    if path.exists() {
        return Err(ExportError::Collision);
    }
    Ok(())
}

fn write_json_file<T: Serialize>(
    authority: &DirectoryIdentityAuthority,
    directory: &Path,
    filename: &str,
    value: &T,
) -> Result<(), ExportError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_bundle_file(authority, directory, filename, &bytes)
}

fn write_bundle_file(
    authority: &DirectoryIdentityAuthority,
    directory: &Path,
    filename: &str,
    bytes: &[u8],
) -> Result<(), ExportError> {
    if bytes.len() > MAX_DIAGNOSTIC_FILE_BYTES {
        return Err(ExportError::SizeLimit);
    }
    authority.revalidate().map_err(map_identity_error)?;
    write_new_file(&directory.join(filename), bytes)
}

fn acquire_export_authority(
    parent: &Path,
    expected: &DirectoryFingerprint,
) -> Result<DirectoryIdentityAuthority, ExportError> {
    DirectoryIdentityAuthority::acquire(parent, Some(expected)).map_err(map_identity_error)
}

fn map_identity_error(error: CoreError) -> ExportError {
    match error {
        CoreError::UnsafeFilesystemPath(_) => ExportError::UnsafePath,
        CoreError::DestinationIdentityChanged(_) => ExportError::DestinationIdentityChanged,
        _ => ExportError::UnsafePath,
    }
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ExportError::Collision);
        }
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error.into());
    }
    Ok(())
}

fn directory_bytes(directory: &Path) -> Result<u64, ExportError> {
    let mut total = 0_u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        total = total
            .checked_add(entry.metadata()?.len())
            .ok_or(ExportError::SizeLimit)?;
    }
    Ok(total)
}

fn operation_name(operation: TaskOperation) -> &'static str {
    match operation {
        TaskOperation::Split => "split",
        TaskOperation::Merge => "merge",
        TaskOperation::Inspect => "inspect",
        TaskOperation::Verify => "verify",
    }
}

fn status_name(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Planned => "planned",
        TaskStatus::Queued => "queued",
        TaskStatus::Running => "running",
        TaskStatus::Pausing => "pausing",
        TaskStatus::Paused => "paused",
        TaskStatus::Resuming => "resuming",
        TaskStatus::Cancelling => "cancelling",
        TaskStatus::Cancelled => "cancelled",
        TaskStatus::Interrupted => "interrupted",
        TaskStatus::PermissionRequired => "permission-required",
        TaskStatus::Failed => "failed",
        TaskStatus::Completed => "completed",
    }
}

fn priority_name(priority: crate::TaskPriority) -> &'static str {
    match priority {
        crate::TaskPriority::High => "high",
        crate::TaskPriority::Normal => "normal",
        crate::TaskPriority::Low => "low",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::{ProcessingPlan, TaskFailure, TaskPriority};

    fn completed_record(root: &Path) -> TaskRecord {
        let mut record = TaskRecord::new(
            "0.5.0",
            1,
            "sample.bin".to_owned(),
            Some("output".to_owned()),
            TaskSpec::Split {
                source_path: root.join("source.bin"),
                output_directory: root.join("output"),
                slice_size: 4,
                package_id: uuid::Uuid::new_v4().to_string(),
                created_at: crate::now(),
            },
            ProcessingPlan {
                total_bytes: 8,
                slice_size: 4,
                slice_count: 2,
                required_free_bytes: 8,
                minimum_required_bytes: 8,
                recommended_free_bytes: 16,
                available_free_bytes: 32,
                temporary_bytes: 8,
                recovery_overhead_bytes: 0,
                expected_output_count: 3,
            },
        );
        record.priority = TaskPriority::High;
        record.status = TaskStatus::Completed;
        record.result = Some(TaskResult::Split {
            manifest_filename: "sample.bin.cake.json".to_owned(),
            source_sha256: "a".repeat(64),
        });
        record.started_at = Some(crate::now());
        record.finished_at = Some(crate::now());
        record.duration_ms = Some(10);
        record
    }

    #[test]
    fn receipts_mask_paths_and_never_overwrite() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("output")).unwrap();
        let record = completed_record(root.path());
        let path = root.path().join("receipt.json");
        let parent_identity = cakesplitter_core::fingerprint_directory(root.path()).unwrap();
        export_operation_receipt(&record, &path, &parent_identity, ReceiptFormat::Json, false)
            .unwrap();
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("…\\\\source.bin"));
        assert!(!contents.contains(&root.path().to_string_lossy().to_string()));
        assert!(matches!(
            export_operation_receipt(&record, &path, &parent_identity, ReceiptFormat::Json, false),
            Err(ExportError::Collision)
        ));
    }

    #[test]
    fn diagnostic_bundle_has_fixed_safe_contents() {
        let root = tempdir().unwrap();
        let mut record = completed_record(root.path());
        record.failure = Some(TaskFailure::classified(
            "permission_required",
            "Contact person@example.test",
            r"C:\Users\Private Name\secret.bin token=secret-value",
            crate::ErrorCategory::Permission,
            true,
            crate::RecoveryAction::Retry,
            1,
        ));
        let summary = export_diagnostic_bundle(
            root.path(),
            &cakesplitter_core::fingerprint_directory(root.path()).unwrap(),
            "0.5.0",
            &[record],
            &StorageSummary::default(),
        )
        .unwrap();
        let names = fs::read_dir(&summary.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 8);
        let errors = fs::read_to_string(summary.path.join("recent-errors.json")).unwrap();
        for forbidden in ["Private Name", "person@example.test", "secret-value"] {
            assert!(!errors.contains(forbidden));
        }
    }
}
