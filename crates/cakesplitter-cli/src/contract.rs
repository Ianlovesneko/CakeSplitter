use std::{
    io::Write,
    time::{Duration, Instant},
};

use cakesplitter_core::Progress;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    cli::OutputFormat,
    error::CliError,
    terminal::{json_terminal_safe, terminal_safe},
};

pub const CLI_SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub struct OperationOutcome {
    pub result: Value,
    pub human_message: String,
    pub warnings: Vec<String>,
    pub terminal_status: String,
    pub terminal_event: String,
    pub exit_code: u8,
}

impl OperationOutcome {
    pub fn new(result: Value, human_message: impl Into<String>) -> Self {
        Self {
            result,
            human_message: human_message.into(),
            warnings: Vec::new(),
            terminal_status: "completed".to_owned(),
            terminal_event: "completed".to_owned(),
            exit_code: 0,
        }
    }

    pub fn with_terminal(mut self, status: &str, event: &str, exit_code: u8) -> Self {
        self.terminal_status = status.to_owned();
        self.terminal_event = event.to_owned();
        self.exit_code = exit_code;
        self
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalResult<'a> {
    schema_version: u32,
    application_version: &'static str,
    command: &'a str,
    status: &'a str,
    result: Option<&'a Value>,
    warnings: &'a [String],
    error: Option<&'a CliError>,
    started_at: String,
    completed_at: String,
    duration_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonlEvent<'a> {
    schema_version: u32,
    event: &'a str,
    command: &'a str,
    operation_id: &'a str,
    timestamp: String,
    sequence: u64,
    payload: Value,
}

pub struct OutputSession<'a, W: Write, E: Write> {
    mode: OutputFormat,
    command: String,
    operation_id: String,
    started_at: DateTime<Utc>,
    started: Instant,
    sequence: u64,
    stdout: &'a mut W,
    stderr: &'a mut E,
    verbose: bool,
    last_human_slice: Option<u64>,
}

impl<'a, W: Write, E: Write> OutputSession<'a, W, E> {
    pub fn new(
        mode: OutputFormat,
        command: &str,
        operation_id: String,
        stdout: &'a mut W,
        stderr: &'a mut E,
        verbose: bool,
    ) -> Result<Self, CliError> {
        let mut session = Self {
            mode,
            command: command.to_owned(),
            operation_id,
            started_at: Utc::now(),
            started: Instant::now(),
            sequence: 0,
            stdout,
            stderr,
            verbose,
            last_human_slice: None,
        };
        if mode == OutputFormat::Jsonl {
            let payload = if command == "batch" {
                json!({
                    "applicationVersion": env!("CARGO_PKG_VERSION"),
                    "runId": session.operation_id
                })
            } else {
                json!({ "applicationVersion": env!("CARGO_PKG_VERSION") })
            };
            session.emit_event("started", payload)?;
        }
        Ok(session)
    }

    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub fn preflight(&mut self, payload: Value) -> Result<(), CliError> {
        if self.mode == OutputFormat::Jsonl {
            self.emit_event("preflight", payload)?;
        }
        Ok(())
    }

    pub fn progress(&mut self, progress: &Progress) -> Result<(), CliError> {
        match self.mode {
            OutputFormat::Jsonl => self.emit_event(
                "progress",
                serde_json::to_value(progress)
                    .map_err(|error| CliError::internal(error.to_string()))?,
            ),
            OutputFormat::Human => {
                if self.last_human_slice == Some(progress.current_slice)
                    && progress.bytes_processed < progress.total_bytes
                {
                    return Ok(());
                }
                self.last_human_slice = Some(progress.current_slice);
                let percent = if progress.total_bytes == 0 {
                    100
                } else {
                    progress.bytes_processed.saturating_mul(100) / progress.total_bytes
                };
                writeln!(
                    self.stderr,
                    "{}: {}% · Slice {}/{}",
                    progress.operation, percent, progress.current_slice, progress.slice_count
                )
                .map_err(|error| CliError::internal(error.to_string()))
            }
            OutputFormat::Json => Ok(()),
        }
    }

    pub fn warning(&mut self, message: &str) -> Result<(), CliError> {
        match self.mode {
            OutputFormat::Jsonl => self.emit_event("warning", json!({ "message": message })),
            OutputFormat::Human => writeln!(self.stderr, "warning: {}", terminal_safe(message))
                .map_err(|error| CliError::internal(error.to_string())),
            OutputFormat::Json => Ok(()),
        }
    }

    pub fn finish_success(mut self, outcome: &OperationOutcome) -> Result<u8, CliError> {
        let completed = Utc::now();
        match self.mode {
            OutputFormat::Human => {
                writeln!(self.stdout, "{}", outcome.human_message)
                    .map_err(|error| CliError::internal(error.to_string()))?;
                for warning in &outcome.warnings {
                    self.warning(warning)?;
                }
            }
            OutputFormat::Json => {
                let document = FinalResult {
                    schema_version: CLI_SCHEMA_VERSION,
                    application_version: env!("CARGO_PKG_VERSION"),
                    command: &self.command,
                    status: &outcome.terminal_status,
                    result: Some(&outcome.result),
                    warnings: &outcome.warnings,
                    error: None,
                    started_at: timestamp(self.started_at),
                    completed_at: timestamp(completed),
                    duration_ms: duration_ms(self.started.elapsed()),
                };
                write_json_line(self.stdout, &document)?;
            }
            OutputFormat::Jsonl => {
                for warning in &outcome.warnings {
                    self.emit_event("warning", json!({ "message": warning }))?;
                }
                let mut payload = json!({
                    "status": outcome.terminal_status,
                    "result": outcome.result,
                    "warnings": outcome.warnings,
                    "durationMs": duration_ms(self.started.elapsed())
                });
                if self.command == "batch" {
                    payload["runId"] = Value::String(self.operation_id.clone());
                }
                self.emit_event(&outcome.terminal_event, payload)?;
            }
        }
        Ok(outcome.exit_code)
    }

    pub fn finish_error(mut self, mut error: CliError) -> Result<u8, CliError> {
        error.operation_id = Some(self.operation_id.clone());
        let completed = Utc::now();
        let terminal_status = if error.exit_code == crate::error::EXIT_CANCELLED {
            "cancelled"
        } else {
            "failed"
        };
        match self.mode {
            OutputFormat::Human => {
                writeln!(
                    self.stderr,
                    "cakesplitter: [{}] {}",
                    terminal_safe(&error.code),
                    terminal_safe(&error.message)
                )
                .map_err(|write_error| CliError::internal(write_error.to_string()))?;
                if self.verbose {
                    writeln!(
                        self.stderr,
                        "technical: {}",
                        terminal_safe(&error.technical_message)
                    )
                    .map_err(|write_error| CliError::internal(write_error.to_string()))?;
                }
            }
            OutputFormat::Json => {
                let document = FinalResult {
                    schema_version: CLI_SCHEMA_VERSION,
                    application_version: env!("CARGO_PKG_VERSION"),
                    command: &self.command,
                    status: terminal_status,
                    result: None,
                    warnings: &[],
                    error: Some(&error),
                    started_at: timestamp(self.started_at),
                    completed_at: timestamp(completed),
                    duration_ms: duration_ms(self.started.elapsed()),
                };
                write_json_line(self.stdout, &document)?;
            }
            OutputFormat::Jsonl => {
                let mut payload = json!({
                    "status": terminal_status,
                    "error": error,
                    "durationMs": duration_ms(self.started.elapsed())
                });
                if self.command == "batch" {
                    payload["runId"] = Value::String(self.operation_id.clone());
                }
                self.emit_event(terminal_status, payload)?;
            }
        }
        Ok(error.exit_code)
    }

    fn emit_event(&mut self, event: &str, payload: Value) -> Result<(), CliError> {
        self.sequence = self.sequence.saturating_add(1);
        let document = JsonlEvent {
            schema_version: CLI_SCHEMA_VERSION,
            event,
            command: &self.command,
            operation_id: &self.operation_id,
            timestamp: timestamp(Utc::now()),
            sequence: self.sequence,
            payload,
        };
        write_json_line(self.stdout, &document)
    }

    pub(crate) fn emit_batch_event(&mut self, event: &str, payload: Value) -> Result<(), CliError> {
        if self.mode == OutputFormat::Jsonl {
            self.emit_event(event, payload)?;
        }
        Ok(())
    }
}

pub fn render_parse_error<W: Write, E: Write>(
    mode: OutputFormat,
    command: &str,
    stdout: &mut W,
    stderr: &mut E,
    error: CliError,
) -> u8 {
    let operation_id = uuid::Uuid::new_v4().to_string();
    let session = match OutputSession::new(mode, command, operation_id, stdout, stderr, false) {
        Ok(session) => session,
        Err(_) => return crate::error::EXIT_INTERNAL,
    };
    match session.finish_error(error) {
        Ok(code) => code,
        Err(_) => crate::error::EXIT_INTERNAL,
    }
}

pub fn write_json_line<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<(), CliError> {
    let serialized =
        serde_json::to_string(value).map_err(|error| CliError::internal(error.to_string()))?;
    writeln!(writer, "{}", json_terminal_safe(&serialized))
        .and_then(|_| writer.flush())
        .map_err(|error| CliError::internal(error.to_string()))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
