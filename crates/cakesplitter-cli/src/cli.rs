use std::path::PathBuf;

use cakesplitter_format::{MAX_SAFE_INTEGER, MAX_SLICE_COUNT};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::terminal::terminal_safe;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
    Jsonl,
}

#[derive(Debug, Parser)]
#[command(
    name = "cakesplitter",
    version,
    disable_help_subcommand = true,
    about = "Split, inspect, verify, plan, and rebuild local Cake Packages",
    long_about = "A local-only, non-interactive interface for streamed Cake Package format 1.0 workflows. Outputs never overwrite existing files."
)]
pub struct Cli {
    /// Select human output, one final JSON document, or a JSONL event stream.
    #[arg(long, global = true, value_enum, default_value_t)]
    pub format: OutputFormat,

    /// Include privacy-safe technical detail in human diagnostics.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Split one file into verified .slice files and a Manifest.
    Split(SplitArgs),
    /// Rebuild the original file and verify its final SHA-256 hash.
    Merge(MergeArgs),
    /// Inspect a Manifest or package without creating output.
    Inspect(InspectArgs),
    /// Verify every selected Slice and produce a final package verdict.
    Verify(VerifyArgs),
    /// Plan a Split or Merge without mutating the filesystem.
    Plan {
        #[command(subcommand)]
        command: PlanCommand,
    },
    /// Validate and execute a bounded local batch job.
    Batch {
        #[command(subcommand)]
        command: BatchCommand,
    },
    /// Print application, CLI schema, and Cake Package format versions.
    Version,
    /// Print command help.
    Help,
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Split(_) => "split",
            Self::Merge(_) => "merge",
            Self::Inspect(_) => "inspect",
            Self::Verify(_) => "verify",
            Self::Plan { .. } => "plan",
            Self::Batch { .. } => "batch",
            Self::Version => "version",
            Self::Help => "help",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum BatchCommand {
    /// Validate a versioned Batch Job specification without mutation.
    Validate(BatchValidateArgs),
    /// Preflight a Batch Job specification without mutation.
    Plan(BatchPlanArgs),
    /// Execute a Batch Job specification and persist bounded run state.
    Run(BatchRunArgs),
    /// Resume an interrupted or retryable Batch Job run.
    Resume(BatchResumeArgs),
    /// Inspect persisted Batch Job run state without processing.
    Status(BatchStatusArgs),
}

#[derive(Debug, Args)]
pub struct BatchValidateArgs {
    /// Versioned Batch Job specification.
    pub job_spec: PathBuf,
    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct BatchPlanArgs {
    /// Versioned Batch Job specification.
    pub job_spec: PathBuf,
    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct BatchRunArgs {
    /// Versioned Batch Job specification.
    pub job_spec: PathBuf,
    /// Persisted run-state path. Defaults beside the Job specification.
    #[arg(long)]
    pub state: Option<PathBuf>,
    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct BatchResumeArgs {
    /// Persisted Batch Job run-state file.
    pub run_state: PathBuf,
    /// Re-supply the Job specification when it has moved; its digest must match.
    #[arg(long)]
    pub job_spec: Option<PathBuf>,
    /// Explicitly retry previously failed retryable operations.
    #[arg(long)]
    pub retry_failed: bool,
    /// Explicitly retry operations cancelled by a prior run.
    #[arg(long)]
    pub retry_cancelled: bool,
    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct BatchStatusArgs {
    /// Persisted Batch Job run-state file.
    pub run_state: PathBuf,
    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Subcommand)]
pub enum PlanCommand {
    /// Plan a Split without mutation unless an explicit dry-run receipt is requested.
    Split(PlanSplitArgs),
    /// Plan a Merge without mutation unless an explicit dry-run receipt is requested.
    Merge(PlanMergeArgs),
}

#[derive(Debug, Args)]
pub struct PlanSplitArgs {
    #[command(flatten)]
    pub plan: SplitPlanArgs,

    /// Export an explicit dry-run report; this is the only Plan output file.
    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct PlanMergeArgs {
    #[command(flatten)]
    pub plan: MergePlanArgs,

    /// Export an explicit dry-run report; this is the only Plan output file.
    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct SplitArgs {
    #[command(flatten)]
    pub plan: SplitPlanArgs,

    /// Validate and plan only; produce no file output.
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct SplitPlanArgs {
    /// Source file to split.
    pub file: PathBuf,

    /// Target Slice size. Accepted units: B, KiB, MiB, GiB.
    #[arg(
        long,
        value_parser = parse_size,
        required_unless_present = "slice_count",
        conflicts_with = "slice_count"
    )]
    pub slice_size: Option<u64>,

    /// Target Slice count, bounded by the package format limit.
    #[arg(
        long,
        value_parser = parse_slice_count,
        required_unless_present = "slice_size",
        conflicts_with = "slice_size"
    )]
    pub slice_count: Option<u64>,

    /// Destination directory. Defaults to the source directory.
    #[arg(long)]
    pub output_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct MergeArgs {
    #[command(flatten)]
    pub plan: MergePlanArgs,

    /// Validate and plan only; produce no file output.
    #[arg(long)]
    pub dry_run: bool,

    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct MergePlanArgs {
    /// Cake Manifest or a package directory containing exactly one Manifest.
    pub package: PathBuf,

    /// Explicitly select one expected Slice. Repeat for the complete set.
    #[arg(long = "slice")]
    pub slices: Vec<PathBuf>,

    /// Rebuilt output path.
    #[arg(long)]
    pub output: PathBuf,
}

#[derive(Debug, Args)]
pub struct InspectArgs {
    #[command(flatten)]
    pub package: PackageArgs,

    /// Validate and return only the Manifest, without enumerating package Slices.
    #[arg(long)]
    pub manifest_only: bool,

    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    #[command(flatten)]
    pub package: PackageArgs,

    #[command(flatten)]
    pub receipt: ReceiptArgs,
}

#[derive(Debug, Args)]
pub struct PackageArgs {
    /// Cake Manifest or a package directory containing exactly one Manifest.
    pub package: PathBuf,

    /// Explicitly select one expected Slice. Repeat for the complete set.
    #[arg(long = "slice")]
    pub slices: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptFormat {
    #[default]
    Json,
    Markdown,
}

#[derive(Clone, Debug, Args)]
pub struct ReceiptArgs {
    /// Export a bounded, redacted operation receipt to this new file.
    #[arg(long)]
    pub receipt: Option<PathBuf>,

    /// Receipt format. A receipt path must also be supplied.
    #[arg(long, value_enum, default_value_t, requires = "receipt")]
    pub receipt_format: ReceiptFormat,
}

pub fn parse_size(value: &str) -> Result<u64, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with(['-', '+']) {
        return Err("size must be a positive integer followed by B, KiB, MiB, or GiB".to_owned());
    }
    let split_at = trimmed
        .find(|character: char| !character.is_ascii_digit() && character != '_')
        .unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split_at);
    if number.is_empty()
        || number.starts_with('_')
        || number.ends_with('_')
        || number.contains("__")
        || number.chars().enumerate().any(|(index, character)| {
            character == '_'
                && (index == 0
                    || index + 1 >= number.len()
                    || !number.as_bytes()[index - 1].is_ascii_digit()
                    || !number.as_bytes()[index + 1].is_ascii_digit())
        })
    {
        return Err("size digits may contain only single separators between digits".to_owned());
    }
    let normalized_number = number.replace('_', "");
    let number = normalized_number
        .parse::<u64>()
        .map_err(|_| format!("invalid size: {}", terminal_safe(value)))?;
    let multiplier = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "kib" => 1_024,
        "mib" => 1_048_576,
        "gib" => 1_073_741_824,
        _ => {
            return Err(format!(
                "unsupported or ambiguous size unit: {}; use B, KiB, MiB, or GiB",
                terminal_safe(unit.trim())
            ));
        }
    };
    number
        .checked_mul(multiplier)
        .filter(|size| *size > 0 && *size <= MAX_SAFE_INTEGER)
        .ok_or_else(|| {
            format!("size must be between 1 and {MAX_SAFE_INTEGER} bytes after unit conversion")
        })
}

pub fn parse_slice_count(value: &str) -> Result<u64, String> {
    if value.trim().starts_with(['-', '+']) {
        return Err("Slice count must be a positive integer".to_owned());
    }
    let count = value
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("invalid Slice count: {}", terminal_safe(value)))?;
    if !(1..=MAX_SLICE_COUNT).contains(&count) {
        return Err(format!(
            "Slice count must be between 1 and {MAX_SLICE_COUNT}"
        ));
    }
    Ok(count)
}

pub fn requested_output_format(arguments: &[std::ffi::OsString]) -> OutputFormat {
    let mut iter = arguments.iter().skip(1);
    while let Some(argument) = iter.next() {
        let value = argument.to_string_lossy();
        if let Some(format) = value.strip_prefix("--format=") {
            return parse_format_name(format).unwrap_or_default();
        }
        if value == "--format"
            && let Some(format) = iter.next()
        {
            return parse_format_name(&format.to_string_lossy()).unwrap_or_default();
        }
    }
    OutputFormat::Human
}

fn parse_format_name(value: &str) -> Option<OutputFormat> {
    match value.to_ascii_lowercase().as_str() {
        "human" => Some(OutputFormat::Human),
        "json" => Some(OutputFormat::Json),
        "jsonl" => Some(OutputFormat::Jsonl),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_unambiguous_binary_units() {
        assert_eq!(parse_size("10"), Ok(10));
        assert_eq!(parse_size("2 MiB"), Ok(2_097_152));
        assert_eq!(parse_size("3GiB"), Ok(3_221_225_472));
        for invalid in [
            "0", "-1", "1KB", "2mb", "1.5MiB", "GiB", "1__0B", "_10B", "10_B",
        ] {
            assert!(parse_size(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn bounds_slice_counts() {
        assert_eq!(parse_slice_count("1"), Ok(1));
        assert_eq!(parse_slice_count("50000"), Ok(50_000));
        assert!(parse_slice_count("0").is_err());
        assert!(parse_slice_count("50001").is_err());
        assert!(parse_slice_count("-1").is_err());
    }
}
