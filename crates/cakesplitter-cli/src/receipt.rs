use std::{fs::OpenOptions, io::Write, path::Path};

use cakesplitter_core::{DirectoryIdentityAuthority, fingerprint_directory};
use cakesplitter_desktop_runtime::{redact_text, sanitize_label};
use cakesplitter_format::{FORMAT_VERSION, validate_portable_filename};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value, json};

use crate::{
    cli::ReceiptFormat,
    contract::CLI_SCHEMA_VERSION,
    error::{CliError, CliErrorCategory, EXIT_CONFLICT, EXIT_DESTINATION, EXIT_STORAGE},
    planning::absolute_path,
};

pub fn export_receipt(
    requested_path: &Path,
    format: ReceiptFormat,
    command: &str,
    operation_id: &str,
    status: &str,
    result: &Value,
    warnings: &[String],
) -> Result<Value, CliError> {
    let output_path = absolute_path(requested_path)?;
    let parent = output_path.parent().ok_or_else(|| {
        receipt_destination_error(
            "receipt_destination_invalid",
            "The receipt path must have an existing parent directory.",
        )
    })?;
    cakesplitter_core::validate_existing_directory(parent).map_err(|error| match error {
        cakesplitter_core::CoreError::Io { source, .. }
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            receipt_destination_error(
                "receipt_destination_missing",
                "The receipt parent directory does not exist.",
            )
        }
        other => CliError::from(other),
    })?;
    let filename = output_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            receipt_destination_error(
                "receipt_filename_invalid",
                "The receipt filename must be portable UTF-8.",
            )
        })?;
    validate_portable_filename(filename)
        .map_err(|error| CliError::from(cakesplitter_core::CoreError::from(error)))?;
    let output_path = parent.join(filename);
    let fingerprint = fingerprint_directory(parent).map_err(CliError::from)?;
    let authority =
        DirectoryIdentityAuthority::acquire(parent, Some(&fingerprint)).map_err(CliError::from)?;
    authority.revalidate().map_err(CliError::from)?;

    let receipt = json!({
        "schemaVersion": CLI_SCHEMA_VERSION,
        "applicationVersion": env!("CARGO_PKG_VERSION"),
        "cakePackageFormat": FORMAT_VERSION,
        "operationId": operation_id,
        "command": command,
        "status": status,
        "createdAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "result": redact_json(result),
        "warnings": warnings.iter().map(|warning| redact_text(warning)).collect::<Vec<_>>(),
        "privacy": {
            "pathsMasked": true,
            "usernamesOmitted": true,
            "environmentVariablesOmitted": true,
            "nativeFilesystemIdentitiesOmitted": true,
            "fileContentsOmitted": true,
            "sliceContentsOmitted": true
        }
    });
    let bytes = match format {
        ReceiptFormat::Json => serde_json::to_vec_pretty(&receipt).map_err(|error| {
            receipt_storage_error("receipt_serialization_failed", error.to_string())
        })?,
        ReceiptFormat::Markdown => markdown_receipt(&receipt).into_bytes(),
    };
    if bytes.len() > cakesplitter_desktop_runtime::MAX_RECEIPT_BYTES {
        return Err(receipt_storage_error(
            "receipt_size_limit",
            "The requested receipt exceeds the bounded receipt size limit.",
        ));
    }
    authority.revalidate().map_err(CliError::from)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                receipt_conflict_error()
            } else {
                receipt_storage_error("receipt_write_failed", error.to_string())
            }
        })?;
    if let Err(error) = output
        .write_all(&bytes)
        .and_then(|_| output.flush())
        .and_then(|_| output.sync_all())
    {
        drop(output);
        let _ = authority.revalidate();
        let _ = std::fs::remove_file(&output_path);
        return Err(receipt_storage_error(
            "receipt_write_failed",
            error.to_string(),
        ));
    }
    authority.revalidate().map_err(CliError::from)?;
    Ok(json!({
        "status": status,
        "format": match format { ReceiptFormat::Json => "json", ReceiptFormat::Markdown => "markdown" },
        "filename": sanitize_label(filename),
        "bytesWritten": bytes.len()
    }))
}

fn redact_json(value: &Value) -> Value {
    match value {
        Value::String(value) => Value::String(redact_text(value)),
        Value::Array(values) => Value::Array(values.iter().map(redact_json).collect()),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), redact_json(value)))
                .collect::<Map<_, _>>(),
        ),
        value => value.clone(),
    }
}

fn markdown_receipt(receipt: &Value) -> String {
    format!(
        "# CakeSplitter operation receipt\n\n- Application: `{}`\n- Cake Package format: `{}`\n- CLI schema: `{}`\n- Operation: `{}`\n- Operation ID: `{}`\n- Status: `{}`\n- Created: `{}`\n\n## Result\n\n```json\n{}\n```\n\nPaths are masked. Usernames, secrets, environment variables, native filesystem identities, file contents, and Slice contents are omitted.\n",
        receipt["applicationVersion"].as_str().unwrap_or("unknown"),
        receipt["cakePackageFormat"].as_str().unwrap_or("unknown"),
        receipt["schemaVersion"],
        receipt["command"].as_str().unwrap_or("unknown"),
        receipt["operationId"].as_str().unwrap_or("unknown"),
        receipt["status"].as_str().unwrap_or("unknown"),
        receipt["createdAt"].as_str().unwrap_or("unknown"),
        serde_json::to_string_pretty(&receipt["result"]).unwrap_or_else(|_| "null".to_owned())
    )
}

fn receipt_conflict_error() -> CliError {
    CliError {
        code: "receipt_collision".to_owned(),
        category: CliErrorCategory::Conflict,
        message: "The requested receipt already exists and was not overwritten.".to_owned(),
        technical_message: "receipt output collision".to_owned(),
        retryable: true,
        suggested_action: "Choose a new receipt path or remove the existing file explicitly."
            .to_owned(),
        operation_id: None,
        exit_code: EXIT_CONFLICT,
    }
}

fn receipt_destination_error(code: &str, message: impl Into<String>) -> CliError {
    let message = message.into();
    CliError {
        code: code.to_owned(),
        category: CliErrorCategory::Destination,
        message: message.clone(),
        technical_message: message,
        retryable: false,
        suggested_action: "Choose a stable existing local directory for the receipt.".to_owned(),
        operation_id: None,
        exit_code: EXIT_DESTINATION,
    }
}

fn receipt_storage_error(code: &str, technical: impl Into<String>) -> CliError {
    CliError {
        code: code.to_owned(),
        category: CliErrorCategory::Storage,
        message: "The operation completed, but the optional receipt was not exported.".to_owned(),
        technical_message: redact_text(&technical.into()),
        retryable: true,
        suggested_action: "Choose another new receipt path and export again.".to_owned(),
        operation_id: None,
        exit_code: EXIT_STORAGE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_redaction_removes_private_values() {
        let value = json!({
            "path": r"C:\Users\Private Name\sample.bin",
            "token": "api_key=super-secret",
            "hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        });
        let redacted = redact_json(&value);
        let text = serde_json::to_string(&redacted).unwrap();
        assert!(!text.contains("Private Name"));
        assert!(!text.contains("super-secret"));
        assert!(text.contains("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    }
}
