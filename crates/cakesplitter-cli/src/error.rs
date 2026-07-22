use std::io::ErrorKind;

use cakesplitter_core::CoreError;
use cakesplitter_desktop_runtime::redact_text;
use serde::Serialize;

use crate::terminal::terminal_safe;

const MAX_CLI_ERROR_TEXT: usize = 2_000;

pub const EXIT_INTERNAL: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_INTEGRITY: u8 = 3;
pub const EXIT_CONFLICT: u8 = 4;
pub const EXIT_SOURCE: u8 = 5;
pub const EXIT_DESTINATION: u8 = 6;
pub const EXIT_PERMISSION: u8 = 7;
pub const EXIT_STORAGE: u8 = 8;
pub const EXIT_RECOVERY: u8 = 9;
pub const EXIT_CAPACITY: u8 = 10;
pub const EXIT_CANCELLED: u8 = 130;
pub const EXIT_BATCH_FAILURE: u8 = 11;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CliErrorCategory {
    Usage,
    Source,
    Destination,
    Package,
    Integrity,
    Permission,
    Storage,
    Conflict,
    Recovery,
    Capacity,
    Cancellation,
    Internal,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliError {
    pub code: String,
    pub category: CliErrorCategory,
    pub message: String,
    pub technical_message: String,
    pub retryable: bool,
    pub suggested_action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip)]
    pub exit_code: u8,
}

impl CliError {
    pub fn usage(code: &str, message: impl Into<String>, technical: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            category: CliErrorCategory::Usage,
            message: message.into(),
            technical_message: bounded_error_text(&redact_text(&technical.into())),
            retryable: false,
            suggested_action: "Correct the command arguments and run it again.".to_owned(),
            operation_id: None,
            exit_code: EXIT_USAGE,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: "internal_failure".to_owned(),
            category: CliErrorCategory::Internal,
            message: "CakeSplitter could not complete the local operation.".to_owned(),
            technical_message: bounded_error_text(&redact_text(&message)),
            retryable: false,
            suggested_action:
                "Review the local diagnostic detail and report a reproducible defect.".to_owned(),
            operation_id: None,
            exit_code: EXIT_INTERNAL,
        }
    }

    pub fn with_operation_id(mut self, operation_id: &str) -> Self {
        self.operation_id = Some(operation_id.to_owned());
        self
    }
}

impl From<CoreError> for CliError {
    fn from(error: CoreError) -> Self {
        let technical_message =
            bounded_error_text(&redact_text(&terminal_safe(&error.to_string())));
        let code = error.code().to_owned();
        let (category, message, retryable, suggested_action, exit_code) = match &error {
            CoreError::Io { source, .. } if source.kind() == ErrorKind::PermissionDenied => (
                CliErrorCategory::Permission,
                "Permission was denied for a selected local filesystem object.",
                true,
                "Grant access or choose another local destination.",
                EXIT_PERMISSION,
            ),
            CoreError::Io { source, .. } if source.kind() == ErrorKind::NotFound => (
                CliErrorCategory::Source,
                "A selected local filesystem object was not found.",
                false,
                "Check the selected path and run the command again.",
                EXIT_SOURCE,
            ),
            CoreError::Io { .. } => (
                CliErrorCategory::Storage,
                "A bounded local filesystem operation failed.",
                true,
                "Check local storage health and available space before retrying.",
                EXIT_STORAGE,
            ),
            CoreError::InvalidJson(_) | CoreError::InvalidManifest(_) => (
                CliErrorCategory::Package,
                "The Cake Manifest is malformed or unsupported.",
                false,
                "Select a valid Cake Package format 1.0 Manifest.",
                EXIT_USAGE,
            ),
            CoreError::InvalidInput(_) | CoreError::SourceChanged => (
                CliErrorCategory::Source,
                "The selected source is invalid or changed during processing.",
                false,
                "Reselect the original stable source and retry.",
                EXIT_SOURCE,
            ),
            CoreError::InvalidSliceSize => (
                CliErrorCategory::Usage,
                "The requested Slice size is outside supported bounds.",
                false,
                "Use a positive byte size with B, KiB, MiB, or GiB units.",
                EXIT_USAGE,
            ),
            CoreError::SliceLimit { .. } | CoreError::PackageEnumerationLimit { .. } => (
                CliErrorCategory::Capacity,
                "The operation exceeds a documented local capacity limit.",
                false,
                "Reduce the Slice count or package size and plan again.",
                EXIT_CAPACITY,
            ),
            CoreError::Collision(_) => (
                CliErrorCategory::Conflict,
                "A planned output already exists; CakeSplitter did not overwrite it.",
                true,
                "Choose a new output path or remove the conflicting file explicitly.",
                EXIT_CONFLICT,
            ),
            CoreError::Cancelled => (
                CliErrorCategory::Cancellation,
                "The operation was cancelled safely.",
                true,
                "Run the command again when you are ready.",
                EXIT_CANCELLED,
            ),
            CoreError::MissingSlices(_)
            | CoreError::UnexpectedSlices(_)
            | CoreError::CorruptedSlices(_)
            | CoreError::FinalHashMismatch { .. } => (
                CliErrorCategory::Integrity,
                "The Cake Package failed integrity validation.",
                false,
                "Restore the original verified Slices or recreate the package.",
                EXIT_INTEGRITY,
            ),
            CoreError::NonUtf8Filename => (
                CliErrorCategory::Source,
                "A selected filename is not valid portable UTF-8.",
                false,
                "Rename the local file using a portable filename.",
                EXIT_SOURCE,
            ),
            CoreError::StagedIdentityChanged(_)
            | CoreError::StagedContentChanged(_)
            | CoreError::AtomicFinalizationUnsupported(_)
            | CoreError::UnsafeFilesystemPath(_)
            | CoreError::DestinationIdentityChanged(_) => (
                CliErrorCategory::Destination,
                "The output destination is unsafe, unsupported, or changed identity.",
                false,
                "Choose a stable local destination without links or reparse points.",
                EXIT_DESTINATION,
            ),
            CoreError::ResumeRejected(_) => (
                CliErrorCategory::Recovery,
                "The stored operation state cannot be resumed safely.",
                false,
                "Start a new operation with the original selected objects.",
                EXIT_RECOVERY,
            ),
            CoreError::PackageIdentityChanged(_) => (
                CliErrorCategory::Package,
                "The selected Cake Package changed identity during processing.",
                false,
                "Reselect the original package and retry.",
                EXIT_INTEGRITY,
            ),
        };
        let message = match &error {
            CoreError::InvalidJson(_) | CoreError::InvalidManifest(_) => bounded_error_text(
                &format!("The Cake Manifest is malformed or unsupported: {technical_message}"),
            ),
            CoreError::MissingSlices(_)
            | CoreError::UnexpectedSlices(_)
            | CoreError::CorruptedSlices(_)
            | CoreError::FinalHashMismatch { .. } => technical_message.clone(),
            _ => message.to_owned(),
        };
        Self {
            code,
            category,
            message,
            technical_message,
            retryable,
            suggested_action: suggested_action.to_owned(),
            operation_id: None,
            exit_code,
        }
    }
}

fn bounded_error_text(value: &str) -> String {
    value.chars().take(MAX_CLI_ERROR_TEXT).collect()
}

#[cfg(test)]
mod tests {
    use std::{io, path::PathBuf};

    use cakesplitter_format::ManifestError;

    use super::*;

    #[test]
    fn core_errors_map_to_stable_categories_and_exit_codes() {
        let cases = [
            (
                CoreError::Io {
                    path: PathBuf::from("private"),
                    source: io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
                },
                CliErrorCategory::Permission,
                EXIT_PERMISSION,
            ),
            (
                CoreError::SourceChanged,
                CliErrorCategory::Source,
                EXIT_SOURCE,
            ),
            (
                CoreError::DestinationIdentityChanged(PathBuf::from("destination")),
                CliErrorCategory::Destination,
                EXIT_DESTINATION,
            ),
            (
                CoreError::InvalidManifest(ManifestError::UnsupportedVersion("9".to_owned())),
                CliErrorCategory::Package,
                EXIT_USAGE,
            ),
            (
                CoreError::MissingSlices(vec!["missing.slice".to_owned()]),
                CliErrorCategory::Integrity,
                EXIT_INTEGRITY,
            ),
            (
                CoreError::PackageIdentityChanged(PathBuf::from("package")),
                CliErrorCategory::Package,
                EXIT_INTEGRITY,
            ),
            (
                CoreError::ResumeRejected("rebound".to_owned()),
                CliErrorCategory::Recovery,
                EXIT_RECOVERY,
            ),
            (
                CoreError::SliceLimit {
                    actual: 50_001,
                    maximum: 50_000,
                },
                CliErrorCategory::Capacity,
                EXIT_CAPACITY,
            ),
            (
                CoreError::Cancelled,
                CliErrorCategory::Cancellation,
                EXIT_CANCELLED,
            ),
        ];
        for (error, category, exit_code) in cases {
            let mapped = CliError::from(error);
            assert_eq!(mapped.category, category);
            assert_eq!(mapped.exit_code, exit_code);
            assert!(!mapped.code.is_empty());
            assert!(!mapped.message.is_empty());
            assert!(!mapped.suggested_action.is_empty());
        }

        let internal = CliError::internal("internal detail");
        assert_eq!(internal.category, CliErrorCategory::Internal);
        assert_eq!(internal.exit_code, EXIT_INTERNAL);
    }
}
