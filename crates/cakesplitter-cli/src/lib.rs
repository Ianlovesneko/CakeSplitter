#![allow(
    clippy::result_large_err,
    reason = "CliError intentionally carries the structured machine-readable diagnostic envelope"
)]

mod cli;
mod contract;
mod error;
mod planning;
mod receipt;
mod terminal;

use std::{cell::RefCell, ffi::OsString, fs, io::Write, path::PathBuf};

use cakesplitter_core::{
    CancellationToken, CoreError, MergeCheckpointEvent, NativeFileIdentity, Progress,
    ResumableMergeOptions, ResumableSplitOptions, SplitCheckpointEvent, fingerprint_directory,
    merge_package_resumable_bound_with_progress, remove_owned_incomplete_file,
    split_file_resumable_with_progress,
};
use cakesplitter_desktop_runtime::{masked_path, redact_text};
use cakesplitter_format::FORMAT_VERSION;
use chrono::{SecondsFormat, Utc};
use clap::{CommandFactory, Parser, error::ErrorKind};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    cli::{
        Cli, Command, InspectArgs, MergeArgs, OutputFormat, PlanCommand, ReceiptArgs, SplitArgs,
        VerifyArgs, requested_output_format,
    },
    contract::{OperationOutcome, OutputSession, render_parse_error},
    error::{CliError, EXIT_INTERNAL},
    planning::{
        canonical_existing_directory, ensure_merge_ready, ensure_split_ready, manifest_only,
        plan_merge, plan_split, prepare_package,
    },
    receipt::export_receipt,
    terminal::{terminal_path, terminal_safe},
};

pub use cli::OutputFormat as CliOutputFormat;
pub use contract::CLI_SCHEMA_VERSION;
pub use error::{CliError as StructuredCliError, CliErrorCategory};

pub fn run<W: Write, E: Write>(
    arguments: Vec<OsString>,
    stdout: &mut W,
    stderr: &mut E,
    cancellation: CancellationToken,
) -> u8 {
    let requested_format = requested_output_format(&arguments);
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                return render_clap_display(requested_format, error, stdout, stderr);
            }
            let technical = terminal_safe(&redact_text(&error.to_string()));
            if requested_format == OutputFormat::Human {
                return if write!(stderr, "{technical}").is_ok() {
                    u8::try_from(error.exit_code()).unwrap_or(EXIT_INTERNAL)
                } else {
                    EXIT_INTERNAL
                };
            }
            let structured = CliError::usage(
                "invalid_arguments",
                "The command arguments are invalid.",
                technical,
            );
            return render_parse_error(
                requested_format,
                inferred_command(&error.to_string()),
                stdout,
                stderr,
                structured,
            );
        }
    };

    let command_name = cli.command.name();
    let operation_id = Uuid::new_v4().to_string();
    let mut session = match OutputSession::new(
        cli.format,
        command_name,
        operation_id,
        stdout,
        stderr,
        cli.verbose,
    ) {
        Ok(session) => session,
        Err(_) => return EXIT_INTERNAL,
    };
    let result = execute(&cli.command, &mut session, &cancellation);
    match result {
        Ok(outcome) => session.finish_success(&outcome).unwrap_or(EXIT_INTERNAL),
        Err(error) => session.finish_error(error).unwrap_or(EXIT_INTERNAL),
    }
}

fn execute<W: Write, E: Write>(
    command: &Command,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    match command {
        Command::Split(arguments) => execute_split(arguments, session, cancellation),
        Command::Merge(arguments) => execute_merge(arguments, session, cancellation),
        Command::Inspect(arguments) => execute_inspect(arguments, session, cancellation),
        Command::Verify(arguments) => execute_verify(arguments, session, cancellation),
        Command::Plan { command } => execute_plan(command, session, cancellation),
        Command::Version => Ok(OperationOutcome::new(
            json!({
                "applicationVersion": env!("CARGO_PKG_VERSION"),
                "cliSchemaVersion": CLI_SCHEMA_VERSION,
                "cakePackageFormat": FORMAT_VERSION
            }),
            format!(
                "CakeSplitter {} · CLI schema {} · Cake Package {}",
                env!("CARGO_PKG_VERSION"),
                CLI_SCHEMA_VERSION,
                FORMAT_VERSION
            ),
        )),
        Command::Help => {
            let help = Cli::command().render_long_help().to_string();
            Ok(OperationOutcome::new(json!({ "help": help }), help))
        }
    }
}

fn execute_plan<W: Write, E: Write>(
    command: &PlanCommand,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    match command {
        PlanCommand::Split(arguments) => {
            let prepared = plan_split(&arguments.plan)?;
            session.preflight(prepared.plan.clone())?;
            let mut outcome = OperationOutcome::new(
                json!({ "dryRun": true, "plan": prepared.plan }),
                if prepared.ready {
                    format!(
                        "Split plan ready: {} Slices, no output created",
                        prepared.expected_slice_count
                    )
                } else {
                    "Split plan not ready; no output created".to_owned()
                },
            );
            apply_receipt(
                &mut outcome,
                &arguments.receipt,
                "plan",
                session.operation_id(),
                "dry-run",
            )?;
            Ok(outcome)
        }
        PlanCommand::Merge(arguments) => {
            let prepared = plan_merge(&arguments.plan, cancellation)?;
            session.preflight(prepared.plan.clone())?;
            let mut outcome = OperationOutcome::new(
                json!({ "dryRun": true, "plan": prepared.plan }),
                if prepared.ready {
                    "Merge plan ready; no output created".to_owned()
                } else {
                    "Merge plan not ready; no output created".to_owned()
                },
            );
            apply_receipt(
                &mut outcome,
                &arguments.receipt,
                "plan",
                session.operation_id(),
                "dry-run",
            )?;
            Ok(outcome)
        }
    }
}

fn execute_split<W: Write, E: Write>(
    arguments: &SplitArgs,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    let prepared = plan_split(&arguments.plan)?;
    session.preflight(prepared.plan.clone())?;
    if arguments.dry_run {
        let mut outcome = OperationOutcome::new(
            json!({ "dryRun": true, "plan": prepared.plan }),
            if prepared.ready {
                "Split dry-run ready; filesystem unchanged"
            } else {
                "Split dry-run found blocking conflicts; filesystem unchanged"
            },
        );
        apply_receipt(
            &mut outcome,
            &arguments.receipt,
            "split",
            session.operation_id(),
            "dry-run",
        )?;
        return Ok(outcome);
    }
    ensure_split_ready(&prepared)?;
    if cancellation.is_cancelled() {
        return Err(CliError::from(CoreError::Cancelled));
    }
    fs::create_dir_all(&prepared.output_directory).map_err(|error| {
        CliError::from(CoreError::Io {
            path: prepared.output_directory.clone(),
            source: error,
        })
    })?;
    let output_directory = canonical_existing_directory(&prepared.output_directory)?;
    let output_identity = fingerprint_directory(&output_directory).map_err(CliError::from)?;
    let task_id = session.operation_id().to_owned();
    let mut tracker = SplitArtifactTracker::new(output_directory.clone());
    let callback_error = RefCell::new(None::<CliError>);
    let options = ResumableSplitOptions {
        task_id,
        package_id: Uuid::new_v4().to_string(),
        created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        slice_size: prepared.slice_size,
        output_dir: output_directory.clone(),
        cancellation: cancellation.clone(),
        resume: None,
    };
    let result = split_file_resumable_with_progress(
        &prepared.source,
        &options,
        |progress| capture_progress(session, progress, &callback_error),
        |event| tracker.observe(event),
    );
    if let Some(error) = callback_error.into_inner() {
        tracker.cleanup();
        return Err(error);
    }
    let manifest_path = match result {
        Ok(path) => path,
        Err(error) => {
            tracker.cleanup();
            return Err(CliError::from(error));
        }
    };
    if fingerprint_directory(&output_directory).map_err(CliError::from)? != output_identity {
        tracker.cleanup();
        return Err(CliError::from(CoreError::DestinationIdentityChanged(
            output_directory,
        )));
    }
    let manifest = match cakesplitter_core::load_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(error) => {
            tracker.cleanup();
            return Err(CliError::from(error));
        }
    };
    let mut outcome = OperationOutcome::new(
        json!({
            "type": "split",
            "manifestFilename": manifest_path.file_name().and_then(|name| name.to_str()).unwrap_or("<manifest>"),
            "sourceFilename": manifest.original.filename,
            "sourceSize": manifest.original.size,
            "sourceSha256": manifest.original.sha256,
            "sliceSize": manifest.target_slice_size,
            "sliceCount": manifest.slice_count,
            "outputDirectory": masked_path(&prepared.output_directory, false),
            "cakePackageFormat": manifest.version
        }),
        format!("Created {}", terminal_path(&manifest_path)),
    );
    let operation_id = session.operation_id().to_owned();
    apply_receipt(
        &mut outcome,
        &arguments.receipt,
        "split",
        &operation_id,
        "completed",
    )?;
    Ok(outcome)
}

fn execute_merge<W: Write, E: Write>(
    arguments: &MergeArgs,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    let prepared = plan_merge(&arguments.plan, cancellation)?;
    session.preflight(prepared.plan.clone())?;
    if arguments.dry_run {
        let mut outcome = OperationOutcome::new(
            json!({ "dryRun": true, "plan": prepared.plan }),
            if prepared.ready {
                "Merge dry-run ready; filesystem unchanged"
            } else {
                "Merge dry-run found blocking conflicts; filesystem unchanged"
            },
        );
        apply_receipt(
            &mut outcome,
            &arguments.receipt,
            "merge",
            session.operation_id(),
            "dry-run",
        )?;
        return Ok(outcome);
    }
    ensure_merge_ready(&prepared)?;
    if cancellation.is_cancelled() {
        return Err(CliError::from(CoreError::Cancelled));
    }
    let parent = prepared
        .output
        .parent()
        .ok_or_else(|| CliError::from(CoreError::UnsafeFilesystemPath(prepared.output.clone())))?;
    fs::create_dir_all(parent).map_err(|error| {
        CliError::from(CoreError::Io {
            path: parent.to_path_buf(),
            source: error,
        })
    })?;
    let parent = canonical_existing_directory(parent)?;
    let output_identity = fingerprint_directory(&parent).map_err(CliError::from)?;
    let output = parent.join(
        prepared
            .output
            .file_name()
            .ok_or_else(|| CliError::from(CoreError::NonUtf8Filename))?,
    );
    let mut tracker = MergeArtifactTracker::new(parent.clone());
    let callback_error = RefCell::new(None::<CliError>);
    let options = ResumableMergeOptions {
        task_id: session.operation_id().to_owned(),
        cancellation: cancellation.clone(),
        resume: None,
    };
    let result = merge_package_resumable_bound_with_progress(
        &prepared.package.manifest_path,
        &output,
        &options,
        &prepared.package.binding,
        |progress| capture_progress(session, progress, &callback_error),
        |event| tracker.observe(event),
    );
    if let Some(error) = callback_error.into_inner() {
        tracker.cleanup();
        return Err(error);
    }
    if let Err(error) = result {
        tracker.cleanup();
        return Err(CliError::from(error));
    }
    if fingerprint_directory(&parent).map_err(CliError::from)? != output_identity {
        tracker.cleanup();
        return Err(CliError::from(CoreError::DestinationIdentityChanged(
            parent,
        )));
    }
    let manifest = &prepared.package.inspection.manifest;
    let mut outcome = OperationOutcome::new(
        json!({
            "type": "merge",
            "outputFilename": output.file_name().and_then(|name| name.to_str()).unwrap_or("<output>"),
            "outputSize": manifest.original.size,
            "outputSha256": manifest.original.sha256,
            "sliceCount": manifest.slice_count,
            "verified": true,
            "cakePackageFormat": manifest.version
        }),
        format!("Rebuilt and verified {}", terminal_path(&output)),
    );
    let operation_id = session.operation_id().to_owned();
    apply_receipt(
        &mut outcome,
        &arguments.receipt,
        "merge",
        &operation_id,
        "completed",
    )?;
    Ok(outcome)
}

fn execute_inspect<W: Write, E: Write>(
    arguments: &InspectArgs,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    let mut outcome = if arguments.manifest_only {
        if !arguments.package.slices.is_empty() {
            return Err(CliError::usage(
                "manifest_only_slice_conflict",
                "--manifest-only cannot be combined with explicit --slice selections.",
                "manifest-only inspection does not enumerate selected Slices",
            ));
        }
        let (manifest_path, manifest) = manifest_only(&arguments.package.package, cancellation)?;
        OperationOutcome::new(
            json!({
                "type": "manifest",
                "manifestFilename": manifest_path.file_name().and_then(|name| name.to_str()).unwrap_or("<manifest>"),
                "manifest": manifest,
                "ready": true,
                "duplicateSlices": []
            }),
            format!("Manifest valid: {} Slices declared", manifest.slice_count),
        )
    } else {
        let package = prepare_package(&arguments.package, false, cancellation)?;
        let inspection = package.inspection;
        let ready = inspection.missing.is_empty()
            && inspection.corrupted.is_empty()
            && inspection.unexpected.is_empty();
        let result = json!({
            "type": "inspection",
            "manifest": inspection.manifest,
            "expectedSliceCount": inspection.expected_slice_count,
            "foundSliceCount": inspection.found_slice_count,
            "missing": inspection.missing,
            "corrupted": inspection.corrupted,
            "duplicateSlices": [],
            "unexpected": inspection.unexpected,
            "verified": false,
            "ready": ready
        });
        let human = crate::terminal::json_terminal_safe(
            &serde_json::to_string_pretty(&result)
                .map_err(|error| CliError::internal(error.to_string()))?,
        );
        OperationOutcome::new(result, human)
    };
    let operation_id = session.operation_id().to_owned();
    apply_receipt(
        &mut outcome,
        &arguments.receipt,
        "inspect",
        &operation_id,
        "completed",
    )?;
    Ok(outcome)
}

fn execute_verify<W: Write, E: Write>(
    arguments: &VerifyArgs,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    let package = prepare_package(&arguments.package, true, cancellation)?;
    let inspection = package.inspection;
    if !inspection.missing.is_empty() {
        return Err(CliError::from(CoreError::MissingSlices(inspection.missing)));
    }
    if !inspection.corrupted.is_empty() {
        return Err(CliError::from(CoreError::CorruptedSlices(
            inspection.corrupted,
        )));
    }
    if !inspection.unexpected.is_empty() {
        return Err(CliError::from(CoreError::UnexpectedSlices(
            inspection.unexpected,
        )));
    }
    let mut outcome = OperationOutcome::new(
        json!({
            "type": "verification",
            "manifest": inspection.manifest,
            "expectedSliceCount": inspection.expected_slice_count,
            "foundSliceCount": inspection.found_slice_count,
            "missing": [],
            "corrupted": [],
            "duplicateSlices": [],
            "unexpected": [],
            "verified": true,
            "ready": true
        }),
        format!("Verified {} Slices", inspection.found_slice_count),
    );
    let operation_id = session.operation_id().to_owned();
    apply_receipt(
        &mut outcome,
        &arguments.receipt,
        "verify",
        &operation_id,
        "completed",
    )?;
    Ok(outcome)
}

fn apply_receipt(
    outcome: &mut OperationOutcome,
    arguments: &ReceiptArgs,
    command: &str,
    operation_id: &str,
    status: &str,
) -> Result<(), CliError> {
    let Some(path) = arguments.receipt.as_deref() else {
        return Ok(());
    };
    match export_receipt(
        path,
        arguments.receipt_format,
        command,
        operation_id,
        status,
        &outcome.result,
        &outcome.warnings,
    ) {
        Ok(receipt) => insert_result_field(&mut outcome.result, "receipt", receipt),
        Err(error) => {
            let warning = format!("{}: {}", error.code, error.message);
            outcome.warnings.push(warning);
            insert_result_field(
                &mut outcome.result,
                "receipt",
                json!({
                    "status": "failed",
                    "error": error
                }),
            );
        }
    }
    Ok(())
}

fn insert_result_field(result: &mut Value, key: &str, value: Value) {
    if let Some(object) = result.as_object_mut() {
        object.insert(key.to_owned(), value);
    }
}

fn capture_progress<W: Write, E: Write>(
    session: &mut OutputSession<'_, W, E>,
    progress: Progress,
    error: &RefCell<Option<CliError>>,
) {
    if error.borrow().is_some() {
        return;
    }
    if let Err(output_error) = session.progress(&progress) {
        *error.borrow_mut() = Some(output_error);
    }
}

struct SplitArtifactTracker {
    output_directory: PathBuf,
    active: Option<(PathBuf, NativeFileIdentity)>,
    completed: Vec<(PathBuf, NativeFileIdentity)>,
}

impl SplitArtifactTracker {
    fn new(output_directory: PathBuf) -> Self {
        Self {
            output_directory,
            active: None,
            completed: Vec::new(),
        }
    }

    fn observe(&mut self, event: SplitCheckpointEvent) {
        match event {
            SplitCheckpointEvent::PartialCreated { partial } => {
                self.active = Some((
                    self.output_directory.join(partial.filename),
                    partial.identity,
                ));
            }
            SplitCheckpointEvent::SliceCompleted { checkpoint } => {
                self.active = None;
                self.completed.push((
                    self.output_directory.join(checkpoint.entry.filename),
                    checkpoint.identity,
                ));
            }
            SplitCheckpointEvent::PartialCleared
            | SplitCheckpointEvent::ManifestPublished { .. } => {
                self.active = None;
            }
            SplitCheckpointEvent::Baseline { .. } => {}
        }
    }

    fn cleanup(&mut self) {
        if let Some((path, identity)) = self.active.take() {
            let _ = remove_owned_incomplete_file(&path, identity);
        }
        for (path, identity) in self.completed.drain(..).rev() {
            let _ = remove_owned_incomplete_file(&path, identity);
        }
    }
}

struct MergeArtifactTracker {
    output_directory: PathBuf,
    active: Option<(PathBuf, NativeFileIdentity)>,
}

impl MergeArtifactTracker {
    fn new(output_directory: PathBuf) -> Self {
        Self {
            output_directory,
            active: None,
        }
    }

    fn observe(&mut self, event: MergeCheckpointEvent) {
        match event {
            MergeCheckpointEvent::PartialCreated { partial, .. } => {
                self.active = Some((
                    self.output_directory.join(partial.filename),
                    partial.identity,
                ));
            }
            MergeCheckpointEvent::Published { .. } => self.active = None,
            MergeCheckpointEvent::SliceBoundary { .. } => {}
        }
    }

    fn cleanup(&mut self) {
        if let Some((path, identity)) = self.active.take() {
            let _ = remove_owned_incomplete_file(&path, identity);
        }
    }
}

fn render_clap_display<W: Write, E: Write>(
    mode: OutputFormat,
    error: clap::Error,
    stdout: &mut W,
    stderr: &mut E,
) -> u8 {
    let text = error.to_string();
    if mode == OutputFormat::Human {
        let safe = terminal_safe(&redact_text(&text));
        return if write!(stdout, "{safe}").is_ok() {
            0
        } else {
            EXIT_INTERNAL
        };
    }
    let command = if error.kind() == ErrorKind::DisplayVersion {
        "version"
    } else {
        "help"
    };
    let operation_id = Uuid::new_v4().to_string();
    let session = match OutputSession::new(mode, command, operation_id, stdout, stderr, false) {
        Ok(session) => session,
        Err(_) => return EXIT_INTERNAL,
    };
    session
        .finish_success(&OperationOutcome::new(json!({ "text": text }), text))
        .unwrap_or(EXIT_INTERNAL)
}

fn inferred_command(error: &str) -> &str {
    for command in [
        "split", "merge", "inspect", "verify", "plan", "version", "help",
    ] {
        if error.contains(command) {
            return command;
        }
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_failure_is_one_parseable_document() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(
            vec![
                OsString::from("cakesplitter"),
                OsString::from("split"),
                OsString::from("--format"),
                OsString::from("json"),
            ],
            &mut stdout,
            &mut stderr,
            CancellationToken::new(),
        );
        assert_eq!(code, 2);
        assert!(stderr.is_empty());
        let value: Value = serde_json::from_slice(&stdout).unwrap();
        assert_eq!(value["schemaVersion"], CLI_SCHEMA_VERSION);
        assert_eq!(value["status"], "failed");
        assert_eq!(value["error"]["category"], "usage");
    }

    #[test]
    fn jsonl_failure_has_monotonic_sequence_and_one_terminal_event() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(
            vec![
                OsString::from("cakesplitter"),
                OsString::from("verify"),
                OsString::from("missing.cake.json"),
                OsString::from("--format=jsonl"),
            ],
            &mut stdout,
            &mut stderr,
            CancellationToken::new(),
        );
        assert_ne!(code, 0);
        assert!(stderr.is_empty());
        let events = String::from_utf8(stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(events[0]["event"], "started");
        assert_eq!(events.last().unwrap()["event"], "failed");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event["event"].as_str(),
                    Some("failed" | "cancelled" | "completed")
                ))
                .count(),
            1
        );
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["sequence"], (index + 1) as u64);
        }
    }

    #[test]
    fn pre_cancelled_split_creates_no_outputs() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.bin");
        let output = root.path().join("output");
        fs::write(&source, vec![7_u8; 1024 * 1024]).unwrap();
        fs::create_dir(&output).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(
            vec![
                OsString::from("cakesplitter"),
                OsString::from("split"),
                source.into_os_string(),
                OsString::from("--slice-size"),
                OsString::from("64KiB"),
                OsString::from("--output-dir"),
                output.clone().into_os_string(),
                OsString::from("--format=json"),
            ],
            &mut stdout,
            &mut stderr,
            cancellation,
        );
        assert_eq!(code, 130);
        assert!(fs::read_dir(output).unwrap().next().is_none());
        assert_eq!(
            serde_json::from_slice::<Value>(&stdout).unwrap()["status"],
            "cancelled"
        );
    }

    #[test]
    fn human_argument_errors_use_central_redaction() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(
            vec![
                OsString::from("cakesplitter"),
                OsString::from("split"),
                OsString::from("source.bin"),
                OsString::from("--slice-size"),
                OsString::from("token=super-secret"),
            ],
            &mut stdout,
            &mut stderr,
            CancellationToken::new(),
        );
        assert_eq!(code, 2);
        let rendered = String::from_utf8(stderr).unwrap();
        assert!(!rendered.contains("super-secret"));
    }

    #[test]
    fn pre_cancelled_merge_has_structured_terminal_result() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.bin");
        let package = root.path().join("package");
        fs::write(&source, b"merge cancellation fixture").unwrap();
        fs::create_dir(&package).unwrap();
        let mut split_stdout = Vec::new();
        let mut split_stderr = Vec::new();
        assert_eq!(
            run(
                vec![
                    OsString::from("cakesplitter"),
                    OsString::from("split"),
                    source.into_os_string(),
                    OsString::from("--slice-size"),
                    OsString::from("4B"),
                    OsString::from("--output-dir"),
                    package.clone().into_os_string(),
                ],
                &mut split_stdout,
                &mut split_stderr,
                CancellationToken::new(),
            ),
            0
        );
        let manifest = fs::read_dir(&package)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .unwrap();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let code = run(
            vec![
                OsString::from("cakesplitter"),
                OsString::from("merge"),
                manifest.into_os_string(),
                OsString::from("--output"),
                OsString::from("rebuilt.bin"),
                OsString::from("--format=jsonl"),
            ],
            &mut stdout,
            &mut stderr,
            cancellation,
        );
        assert_ne!(code, 0);
        let events = String::from_utf8(stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(events.last().unwrap()["event"], "cancelled");
        assert_eq!(
            events.last().unwrap()["payload"]["error"]["category"],
            "cancellation"
        );
    }
}
