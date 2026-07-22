//! Bounded, local-only Batch Job workflows for the v0.6 CLI checkpoint.
//!
//! This module deliberately keeps the batch layer above the existing CLI/core
//! operations. It validates one complete specification before allocating a run,
//! executes operations sequentially, and persists a checksummed state envelope.

use std::{
    collections::{HashMap, HashSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use cakesplitter_core::{
    CancellationToken, CoreError, DirectoryFingerprint, PackageBinding, SourceFingerprint,
    capture_package_binding, fingerprint_directory, fingerprint_file,
};
use cakesplitter_format::{FORMAT_VERSION, MAX_SLICE_COUNT, validate_portable_filename};
use chrono::{SecondsFormat, Utc};
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    apply_receipt,
    cli::{
        BatchCommand, BatchPlanArgs, BatchResumeArgs, BatchRunArgs, BatchStatusArgs,
        BatchValidateArgs, Command, InspectArgs, MergeArgs, MergePlanArgs, OutputFormat,
        PackageArgs, ReceiptArgs, ReceiptFormat, SplitArgs, SplitPlanArgs, VerifyArgs,
    },
    contract::{CLI_SCHEMA_VERSION, OperationOutcome, OutputSession},
    error::{
        CliError, CliErrorCategory, EXIT_BATCH_FAILURE, EXIT_CANCELLED, EXIT_CAPACITY,
        EXIT_CONFLICT, EXIT_RECOVERY,
    },
    planning::{
        absolute_path, canonical_existing_directory, canonical_existing_file, plan_merge,
        plan_split, prepare_package,
    },
};

pub const BATCH_JOB_SCHEMA_VERSION: u32 = 1;
pub const BATCH_STATE_SCHEMA_VERSION: u32 = 1;
pub const MAX_JOB_SPEC_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_BATCH_OPERATIONS: usize = 1_000;
pub const MAX_DEPENDENCIES_PER_OPERATION: usize = 128;
pub const MAX_DEPENDENCY_EDGES: usize = 10_000;
pub const MAX_OPERATION_ID_BYTES: usize = 128;
pub const MAX_JOB_NAME_BYTES: usize = 256;
pub const MAX_METADATA_BYTES: usize = 64 * 1024;
pub const MAX_JSON_DEPTH: usize = 24;
pub const MAX_PERSISTED_EVENTS: u64 = 10_000;
pub const MAX_DIAGNOSTIC_SAMPLES: usize = 20;
pub const MAX_RETAINED_COMPLETED_RUNS: usize = 32;
pub const MAX_OPERATION_ATTEMPTS: u32 = 8;
pub const MAX_ACTIVE_BATCHES: usize = 1;

static ACTIVE_BATCH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailurePolicy {
    Stop,
    ContinueIndependent,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchReceiptSpec {
    path: PathBuf,
    #[serde(default)]
    format: ReceiptFormat,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchJobSpec {
    schema_version: u32,
    name: String,
    failure_policy: FailurePolicy,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    working_directory: Option<PathBuf>,
    #[serde(default)]
    receipt: Option<BatchReceiptSpec>,
    #[serde(default)]
    metadata: Option<Value>,
    operations: Vec<BatchOperationSpec>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BatchOperationSpec {
    id: String,
    command: BatchOperationKind,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default, alias = "source")]
    file: Option<PathBuf>,
    #[serde(default, alias = "manifest")]
    package: Option<PathBuf>,
    #[serde(default)]
    output: Option<PathBuf>,
    #[serde(default)]
    output_dir: Option<PathBuf>,
    #[serde(default)]
    slice_size: Option<String>,
    #[serde(default)]
    slice_count: Option<u64>,
    #[serde(default)]
    slices: Vec<PathBuf>,
    #[serde(default)]
    manifest_only: bool,
    #[serde(default)]
    receipt: Option<BatchReceiptSpec>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum BatchOperationKind {
    Split,
    Merge,
    Inspect,
    Verify,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NormalizedBatchJob {
    schema_version: u32,
    name: String,
    failure_policy: FailurePolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    working_directory: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt: Option<BatchReceiptSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
    operations: Vec<NormalizedOperation>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NormalizedOperation {
    id: String,
    command: BatchOperationKind,
    depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slice_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    slice_count: Option<u64>,
    slices: Vec<PathBuf>,
    manifest_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt: Option<BatchReceiptSpec>,
}

#[derive(Clone, Debug)]
struct LoadedJob {
    path: PathBuf,
    job: NormalizedBatchJob,
    digest: String,
    order: Vec<usize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum RunStatus {
    Running,
    Completed,
    CompletedWithFailures,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum OperationStatus {
    NotStarted,
    Running,
    Completed,
    Failed,
    Cancelled,
    Blocked,
    Interrupted,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationEvidence {
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<SourceFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<SourceFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    directory: Option<DirectoryFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package_manifest: Option<SourceFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    package_directory: Option<DirectoryFingerprint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    membership_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedOperation {
    id: String,
    command: BatchOperationKind,
    status: OperationStatus,
    attempt_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<OperationEvidence>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RunState {
    run_schema_version: u32,
    application_version: String,
    cli_schema_version: u32,
    batch_job_schema_version: u32,
    cake_package_format: String,
    run_id: String,
    job_spec_path: PathBuf,
    job_spec_digest: String,
    job_name: String,
    started_at: String,
    updated_at: String,
    failure_policy: FailurePolicy,
    execution_order: Vec<String>,
    operations: Vec<PersistedOperation>,
    event_count: u64,
    terminal_state: RunStatus,
    revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_status: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateEnvelope {
    state_schema_version: u32,
    revision: u64,
    checksum: String,
    state: RunState,
}

#[derive(Debug)]
struct Preflight {
    ready: bool,
    result: Value,
    error: Option<Value>,
}

#[derive(Default)]
struct NestedEventCapture {
    pending: Vec<u8>,
    events: Vec<Value>,
}

impl Write for NestedEventCapture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        if self.pending.len() > 64 * 1024 {
            self.pending.clear();
            return Ok(bytes.len());
        }
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line = self.pending.drain(..=position).collect::<Vec<_>>();
            if self.events.len() >= MAX_DIAGNOSTIC_SAMPLES {
                continue;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&line)
                && matches!(value["event"].as_str(), Some("progress" | "warning"))
            {
                self.events.push(value);
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ActiveBatchGuard;

impl Drop for ActiveBatchGuard {
    fn drop(&mut self) {
        ACTIVE_BATCH.store(false, Ordering::Release);
    }
}

fn acquire_active_batch() -> Result<ActiveBatchGuard, CliError> {
    ACTIVE_BATCH
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ActiveBatchGuard)
        .map_err(|_| {
            batch_capacity_error(
                "active_batch_limit",
                "one batch execution is already active",
            )
        })
}

pub(crate) fn execute<W: Write, E: Write>(
    command: &BatchCommand,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    match command {
        BatchCommand::Validate(arguments) => execute_validate(arguments, session),
        BatchCommand::Plan(arguments) => execute_plan(arguments, session, cancellation),
        BatchCommand::Run(arguments) => execute_run(arguments, session, cancellation),
        BatchCommand::Resume(arguments) => execute_resume(arguments, session, cancellation),
        BatchCommand::Status(arguments) => execute_status(arguments),
    }
}

fn execute_validate<W: Write, E: Write>(
    arguments: &BatchValidateArgs,
    session: &mut OutputSession<'_, W, E>,
) -> Result<OperationOutcome, CliError> {
    let loaded = load_job(&arguments.job_spec)?;
    let mut outcome = OperationOutcome::new(
        validation_result(&loaded),
        format!("Batch Job `{}` is valid", loaded.job.name),
    );
    apply_batch_receipt(
        &mut outcome,
        &arguments.receipt,
        loaded.job.receipt.as_ref(),
        session.operation_id(),
        "batch-validate",
    )?;
    Ok(outcome)
}

fn execute_plan<W: Write, E: Write>(
    arguments: &BatchPlanArgs,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    let loaded = load_job(&arguments.job_spec)?;
    let result = plan_job(&loaded, session, cancellation, None)?;
    let ready = result["ready"].as_bool().unwrap_or(false);
    let mut outcome = OperationOutcome::new(
        result,
        if ready {
            "Batch plan ready; no output created"
        } else {
            "Batch plan is not ready; no output created"
        },
    );
    if !ready {
        outcome = outcome.with_terminal("not-ready", "batch-failed", EXIT_BATCH_FAILURE);
    }
    apply_batch_receipt(
        &mut outcome,
        &arguments.receipt,
        loaded.job.receipt.as_ref(),
        session.operation_id(),
        "batch-plan",
    )?;
    Ok(outcome)
}

fn execute_run<W: Write, E: Write>(
    arguments: &BatchRunArgs,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    let loaded = load_job(&arguments.job_spec)?;
    let state_path = resolve_state_path(arguments.state.as_deref(), &loaded)?;
    execute_loaded_run(
        &loaded,
        &state_path,
        None,
        session,
        cancellation,
        arguments.receipt.clone(),
    )
}

fn execute_resume<W: Write, E: Write>(
    arguments: &BatchResumeArgs,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    let state_path = validate_state_path(&arguments.run_state)?;
    let state = read_state(&state_path)?;
    let spec_path = arguments
        .job_spec
        .as_deref()
        .map(absolute_path)
        .transpose()?
        .unwrap_or_else(|| state.job_spec_path.clone());
    let loaded = load_job(&spec_path)?;
    if loaded.digest != state.job_spec_digest {
        return Err(batch_recovery_error(
            "batch_spec_digest_mismatch",
            "The supplied Batch Job specification does not match the persisted run digest.",
        ));
    }
    let options = ResumeOptions {
        retry_failed: arguments.retry_failed,
        retry_cancelled: arguments.retry_cancelled,
    };
    execute_loaded_run(
        &loaded,
        &state_path,
        Some((state, options)),
        session,
        cancellation,
        arguments.receipt.clone(),
    )
}

fn execute_status(arguments: &BatchStatusArgs) -> Result<OperationOutcome, CliError> {
    let state_path = validate_state_path(&arguments.run_state)?;
    let state = read_state(&state_path)?;
    let mut outcome = OperationOutcome::new(
        state_result(&state),
        format!("Batch run {}: {:?}", state.run_id, state.terminal_state),
    );
    let receipt = arguments.receipt.clone();
    apply_batch_receipt(&mut outcome, &receipt, None, &state.run_id, "batch-status")?;
    Ok(outcome)
}

#[derive(Clone, Copy, Debug, Default)]
struct ResumeOptions {
    retry_failed: bool,
    retry_cancelled: bool,
}

fn load_job(path: &Path) -> Result<LoadedJob, CliError> {
    let path = canonical_existing_file(path)?;
    let metadata = fs::metadata(&path).map_err(|source| core_io(&path, source))?;
    if metadata.len() > MAX_JOB_SPEC_BYTES as u64 {
        return Err(batch_capacity_error(
            "batch_spec_size_limit",
            "the Batch Job specification exceeds the 8 MiB limit",
        ));
    }
    let bytes = fs::read(&path).map_err(|source| core_io(&path, source))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::usage(
            "batch_invalid_json",
            "The Batch Job specification is not valid JSON.",
            error.to_string(),
        )
    })?;
    validate_json_depth(&value, 0)?;
    let spec: BatchJobSpec = serde_json::from_value(value).map_err(|error| {
        CliError::usage(
            "batch_invalid_schema",
            "The Batch Job specification does not match schema version 1.",
            error.to_string(),
        )
    })?;
    normalize_job(path, spec)
}

fn normalize_job(path: PathBuf, spec: BatchJobSpec) -> Result<LoadedJob, CliError> {
    if spec.schema_version != BATCH_JOB_SCHEMA_VERSION {
        return Err(batch_usage_error(
            "batch_schema_version",
            format!("supported schema version is {BATCH_JOB_SCHEMA_VERSION}"),
        ));
    }
    if spec.name.trim().is_empty() || spec.name.len() > MAX_JOB_NAME_BYTES {
        return Err(batch_usage_error(
            "batch_job_name_limit",
            "job name must be non-empty and at most 256 UTF-8 bytes",
        ));
    }
    if spec
        .description
        .as_ref()
        .is_some_and(|description| description.len() > 2_000)
    {
        return Err(batch_capacity_error(
            "batch_description_limit",
            "Batch Job description exceeds the 2,000-byte limit",
        ));
    }
    if spec.operations.is_empty() || spec.operations.len() > MAX_BATCH_OPERATIONS {
        return Err(batch_capacity_error(
            "batch_operation_limit",
            "the Batch Job must contain between 1 and 1,000 operations",
        ));
    }
    if let Some(metadata) = &spec.metadata {
        let bytes =
            serde_json::to_vec(metadata).map_err(|error| CliError::internal(error.to_string()))?;
        if bytes.len() > MAX_METADATA_BYTES {
            return Err(batch_capacity_error(
                "batch_metadata_limit",
                "Batch Job metadata exceeds the 64 KiB limit",
            ));
        }
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let working_directory = match spec.working_directory.as_deref() {
        Some(directory) => {
            let resolved = resolve_path(parent, directory);
            canonical_existing_directory(&resolved)?
        }
        None => canonical_existing_directory(parent)?,
    };
    let mut operations = Vec::with_capacity(spec.operations.len());
    for operation in spec.operations {
        operations.push(normalize_operation(operation, &working_directory)?);
    }
    validate_graph(&operations)?;
    let order = graph_order(&operations)?;
    let normalized = NormalizedBatchJob {
        schema_version: spec.schema_version,
        name: spec.name,
        failure_policy: spec.failure_policy,
        description: spec.description,
        working_directory,
        receipt: spec
            .receipt
            .map(|receipt| normalize_receipt(receipt, parent))
            .transpose()?,
        metadata: spec.metadata,
        operations,
    };
    let bytes =
        serde_json::to_vec(&normalized).map_err(|error| CliError::internal(error.to_string()))?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    Ok(LoadedJob {
        path,
        job: normalized,
        digest,
        order,
    })
}

fn normalize_operation(
    operation: BatchOperationSpec,
    working_directory: &Path,
) -> Result<NormalizedOperation, CliError> {
    if operation.id.trim().is_empty() || operation.id.len() > MAX_OPERATION_ID_BYTES {
        return Err(batch_usage_error(
            "batch_operation_id_limit",
            "operation IDs must be non-empty and at most 128 UTF-8 bytes",
        ));
    }
    if operation.depends_on.len() > MAX_DEPENDENCIES_PER_OPERATION {
        return Err(batch_capacity_error(
            "batch_dependency_limit",
            "an operation may declare at most 128 dependencies",
        ));
    }
    let file = operation
        .file
        .map(|path| resolve_path(working_directory, &path));
    let package = operation
        .package
        .map(|path| resolve_path(working_directory, &path));
    let output = operation
        .output
        .map(|path| resolve_path(working_directory, &path));
    let output_dir = operation
        .output_dir
        .map(|path| resolve_path(working_directory, &path));
    if u64::try_from(operation.slices.len()).unwrap_or(u64::MAX) > MAX_SLICE_COUNT {
        return Err(batch_capacity_error(
            "batch_slice_set_limit",
            "an operation may select at most the package format slice limit",
        ));
    }
    let slices = operation
        .slices
        .into_iter()
        .map(|path| resolve_path(working_directory, &path))
        .collect::<Vec<_>>();
    let slice_size = operation
        .slice_size
        .as_deref()
        .map(|value| {
            crate::cli::parse_size(value)
                .map_err(|error| batch_usage_error("batch_slice_size", error))
        })
        .transpose()?;
    let normalized = NormalizedOperation {
        id: operation.id,
        command: operation.command,
        depends_on: operation.depends_on,
        file,
        package,
        output,
        output_dir,
        slice_size,
        slice_count: operation.slice_count,
        slices,
        manifest_only: operation.manifest_only,
        receipt: operation
            .receipt
            .map(|receipt| normalize_receipt(receipt, working_directory))
            .transpose()?,
    };
    validate_operation_shape(&normalized)?;
    Ok(normalized)
}

fn normalize_receipt(receipt: BatchReceiptSpec, base: &Path) -> Result<BatchReceiptSpec, CliError> {
    if receipt.path.as_os_str().is_empty() || receipt.path.to_string_lossy().len() > 200 {
        return Err(batch_usage_error(
            "batch_receipt_path_limit",
            "receipt paths must be non-empty and at most 200 UTF-8 bytes",
        ));
    }
    Ok(BatchReceiptSpec {
        path: resolve_path(base, &receipt.path),
        format: receipt.format,
    })
}

fn validate_operation_shape(operation: &NormalizedOperation) -> Result<(), CliError> {
    let has_slice_size = operation.slice_size.is_some();
    let has_slice_count = operation.slice_count.is_some();
    match operation.command {
        BatchOperationKind::Split => {
            if operation.file.is_none() || has_slice_size == has_slice_count {
                return Err(batch_usage_error(
                    "batch_split_arguments",
                    "split requires file and exactly one of sliceSize or sliceCount",
                ));
            }
            if operation.package.is_some()
                || operation.output.is_some()
                || !operation.slices.is_empty()
                || operation.manifest_only
            {
                return Err(batch_usage_error(
                    "batch_split_arguments",
                    "split contains fields belonging to another command",
                ));
            }
        }
        BatchOperationKind::Merge => {
            if operation.package.is_none() || operation.output.is_none() {
                return Err(batch_usage_error(
                    "batch_merge_arguments",
                    "merge requires package and output",
                ));
            }
            if operation.file.is_some()
                || operation.output_dir.is_some()
                || has_slice_size
                || has_slice_count
                || operation.manifest_only
            {
                return Err(batch_usage_error(
                    "batch_merge_arguments",
                    "merge contains fields belonging to another command",
                ));
            }
        }
        BatchOperationKind::Inspect | BatchOperationKind::Verify => {
            if operation.package.is_none() {
                return Err(batch_usage_error(
                    "batch_package_arguments",
                    "inspect and verify require package",
                ));
            }
            if operation.file.is_some()
                || operation.output.is_some()
                || operation.output_dir.is_some()
                || has_slice_size
                || has_slice_count
            {
                return Err(batch_usage_error(
                    "batch_package_arguments",
                    "package operations contain fields belonging to another command",
                ));
            }
            if operation.command == BatchOperationKind::Verify && operation.manifest_only {
                return Err(batch_usage_error(
                    "batch_verify_arguments",
                    "verify cannot use manifestOnly",
                ));
            }
        }
    }
    if operation
        .slice_count
        .is_some_and(|count| count == 0 || count > MAX_SLICE_COUNT)
    {
        return Err(batch_capacity_error(
            "batch_slice_count_limit",
            "sliceCount must be between 1 and the package format limit",
        ));
    }
    Ok(())
}

fn validate_graph(operations: &[NormalizedOperation]) -> Result<(), CliError> {
    let mut ids = HashSet::with_capacity(operations.len());
    let mut edges = 0usize;
    for operation in operations {
        if !ids.insert(operation.id.as_str()) {
            return Err(batch_usage_error(
                "batch_duplicate_operation_id",
                "operation IDs must be unique",
            ));
        }
        edges = edges.saturating_add(operation.depends_on.len());
    }
    if edges > MAX_DEPENDENCY_EDGES {
        return Err(batch_capacity_error(
            "batch_dependency_edge_limit",
            "the Batch Job exceeds the 10,000 dependency-edge limit",
        ));
    }
    for operation in operations {
        let mut dependencies = HashSet::with_capacity(operation.depends_on.len());
        for dependency in &operation.depends_on {
            if !dependencies.insert(dependency.as_str()) {
                return Err(batch_usage_error(
                    "batch_duplicate_dependency",
                    format!("operation {} repeats a dependency", operation.id),
                ));
            }
            if !ids.contains(dependency.as_str()) {
                return Err(batch_usage_error(
                    "batch_unknown_dependency",
                    format!("operation {} names an unknown dependency", operation.id),
                ));
            }
            if dependency == &operation.id {
                return Err(batch_usage_error(
                    "batch_dependency_cycle",
                    "an operation cannot depend on itself",
                ));
            }
        }
    }
    Ok(())
}

fn graph_order(operations: &[NormalizedOperation]) -> Result<Vec<usize>, CliError> {
    let indices = operations
        .iter()
        .enumerate()
        .map(|(index, operation)| (operation.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut indegree = vec![0usize; operations.len()];
    let mut dependents = vec![Vec::new(); operations.len()];
    for (index, operation) in operations.iter().enumerate() {
        indegree[index] = operation.depends_on.len();
        for dependency in &operation.depends_on {
            dependents[indices[dependency.as_str()]].push(index);
        }
    }
    let mut order = Vec::with_capacity(operations.len());
    let mut ready = Vec::new();
    for (index, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            ready.push(index);
        }
    }
    while let Some(index) = ready.first().copied() {
        ready.remove(0);
        order.push(index);
        for dependent in &dependents[index] {
            indegree[*dependent] -= 1;
            if indegree[*dependent] == 0 {
                let position = ready
                    .iter()
                    .position(|candidate| *candidate > *dependent)
                    .unwrap_or(ready.len());
                ready.insert(position, *dependent);
            }
        }
    }
    if order.len() != operations.len() {
        return Err(batch_usage_error(
            "batch_dependency_cycle",
            "the Batch Job dependency graph contains a cycle",
        ));
    }
    Ok(order)
}

fn validate_json_depth(value: &Value, depth: usize) -> Result<(), CliError> {
    if depth > MAX_JSON_DEPTH {
        return Err(batch_capacity_error(
            "batch_json_depth_limit",
            "the Batch Job JSON nesting exceeds the supported limit",
        ));
    }
    match value {
        Value::Array(values) => values
            .iter()
            .try_for_each(|value| validate_json_depth(value, depth + 1)),
        Value::Object(values) => values
            .values()
            .try_for_each(|value| validate_json_depth(value, depth + 1)),
        _ => Ok(()),
    }
}

fn resolve_path(base: &Path, path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    absolute_path(&path).unwrap_or(path)
}

fn validation_result(loaded: &LoadedJob) -> Value {
    json!({
        "batchJobSchemaVersion": BATCH_JOB_SCHEMA_VERSION,
        "cliSchemaVersion": CLI_SCHEMA_VERSION,
        "cakePackageFormat": FORMAT_VERSION,
        "applicationVersion": env!("CARGO_PKG_VERSION"),
        "jobName": loaded.job.name,
        "jobSpecDigest": loaded.digest,
        "failurePolicy": loaded.job.failure_policy,
        "operationCount": loaded.job.operations.len(),
        "executionOrder": loaded.order.iter().map(|index| loaded.job.operations[*index].id.clone()).collect::<Vec<_>>(),
        "limits": batch_limits(),
        "workingDirectory": masked_batch_path(&loaded.job.working_directory)
    })
}

fn batch_limits() -> Value {
    json!({
        "maximumSpecificationBytes": MAX_JOB_SPEC_BYTES,
        "maximumOperations": MAX_BATCH_OPERATIONS,
        "maximumDependenciesPerOperation": MAX_DEPENDENCIES_PER_OPERATION,
        "maximumDependencyEdges": MAX_DEPENDENCY_EDGES,
        "maximumOperationIdBytes": MAX_OPERATION_ID_BYTES,
        "maximumJobNameBytes": MAX_JOB_NAME_BYTES,
        "maximumDescriptionBytes": 2000,
        "maximumMetadataBytes": MAX_METADATA_BYTES,
        "maximumSelectedSlices": MAX_SLICE_COUNT,
        "maximumJsonDepth": MAX_JSON_DEPTH,
        "maximumPersistedEvents": MAX_PERSISTED_EVENTS,
        "maximumDiagnosticSamples": MAX_DIAGNOSTIC_SAMPLES,
        "maximumRetainedCompletedRuns": MAX_RETAINED_COMPLETED_RUNS,
        "maximumActiveBatches": MAX_ACTIVE_BATCHES
    })
}

fn plan_job<W: Write, E: Write>(
    loaded: &LoadedJob,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
    completed: Option<&HashSet<String>>,
) -> Result<Value, CliError> {
    let mut statuses = vec!["not-started".to_owned(); loaded.job.operations.len()];
    let mut operation_plans = Vec::with_capacity(loaded.order.len());
    let mut ready_by_id = HashMap::new();
    for (position, index) in loaded.order.iter().enumerate() {
        if cancellation.is_cancelled() {
            return Err(CliError::from(CoreError::Cancelled));
        }
        let operation = &loaded.job.operations[*index];
        if completed.is_some_and(|completed| completed.contains(&operation.id)) {
            statuses[*index] = "completed".to_owned();
            ready_by_id.insert(operation.id.clone(), true);
            operation_plans.push(json!({
                "id": operation.id,
                "command": operation.command,
                "dependsOn": operation.depends_on,
                "order": position + 1,
                "status": "completed",
                "ready": true,
                "plan": { "alreadyCompleted": true }
            }));
            continue;
        }
        let dependencies_ready = operation
            .depends_on
            .iter()
            .all(|dependency| ready_by_id.get(dependency).copied().unwrap_or(false));
        if !dependencies_ready {
            statuses[*index] = "blocked".to_owned();
            ready_by_id.insert(operation.id.clone(), false);
            operation_plans.push(json!({
                "id": operation.id,
                "command": operation.command,
                "dependsOn": operation.depends_on,
                "order": position + 1,
                "status": "blocked",
                "ready": false,
                "reason": "dependency-not-ready"
            }));
            continue;
        }
        let preflight = preflight_operation_safe(operation, cancellation);
        let status = if preflight.ready {
            "ready"
        } else {
            "not-ready"
        };
        statuses[*index] = status.to_owned();
        ready_by_id.insert(operation.id.clone(), preflight.ready);
        let mut result = json!({
            "id": operation.id,
            "command": operation.command,
            "dependsOn": operation.depends_on,
            "order": position + 1,
            "status": status,
            "ready": preflight.ready,
            "plan": preflight.result
        });
        if let Some(error) = preflight.error {
            result["error"] = error;
        }
        operation_plans.push(result);
        session.emit_batch_event(
            "batch-preflight",
            json!({ "runId": session.operation_id(), "jobName": loaded.job.name, "operationId": operation.id, "status": status }),
        )?;
    }
    let ready = statuses
        .iter()
        .all(|status| matches!(status.as_str(), "ready" | "completed"));
    Ok(json!({
        "batchJobSchemaVersion": BATCH_JOB_SCHEMA_VERSION,
        "jobName": loaded.job.name,
        "jobSpecDigest": loaded.digest,
        "failurePolicy": loaded.job.failure_policy,
        "ready": ready,
        "executionOrder": loaded.order.iter().map(|index| loaded.job.operations[*index].id.clone()).collect::<Vec<_>>(),
        "operationCounts": count_statuses(&statuses),
        "operations": operation_plans,
        "limits": batch_limits(),
        "workingDirectory": masked_batch_path(&loaded.job.working_directory),
        "warnings": [],
        "cakePackageFormat": FORMAT_VERSION
    }))
}

fn preflight_operation(
    operation: &NormalizedOperation,
    cancellation: &CancellationToken,
) -> Result<Preflight, CliError> {
    let result = match operation.command {
        BatchOperationKind::Split => {
            let prepared = plan_split(&SplitPlanArgs {
                file: operation.file.clone().expect("validated split file"),
                slice_size: operation.slice_size,
                slice_count: operation.slice_count,
                output_dir: operation.output_dir.clone(),
            })?;
            (prepared.ready, prepared.plan)
        }
        BatchOperationKind::Merge => {
            let prepared = plan_merge(
                &MergePlanArgs {
                    package: operation.package.clone().expect("validated merge package"),
                    slices: operation.slices.clone(),
                    output: operation.output.clone().expect("validated merge output"),
                },
                cancellation,
            )?;
            (prepared.ready, prepared.plan)
        }
        BatchOperationKind::Inspect | BatchOperationKind::Verify => {
            let package = prepare_package(
                &PackageArgs {
                    package: operation.package.clone().expect("validated package"),
                    slices: operation.slices.clone(),
                },
                operation.command == BatchOperationKind::Verify,
                cancellation,
            )?;
            let ready = package.inspection.missing.is_empty()
                && package.inspection.corrupted.is_empty()
                && package.inspection.unexpected.is_empty();
            (
                ready,
                json!({
                    "type": match operation.command { BatchOperationKind::Inspect => "inspect", BatchOperationKind::Verify => "verify", _ => "package" },
                    "manifestFilename": package.manifest_path.file_name().and_then(|name| name.to_str()).unwrap_or("<manifest>"),
                    "expectedSliceCount": package.inspection.expected_slice_count,
                    "foundSliceCount": package.inspection.found_slice_count,
                    "missing": package.inspection.missing,
                    "corrupted": package.inspection.corrupted,
                    "unexpected": package.inspection.unexpected,
                    "verified": package.inspection.verified,
                    "ready": ready
                }),
            )
        }
    };
    Ok(Preflight {
        ready: result.0,
        result: result.1,
        error: None,
    })
}

fn preflight_operation_safe(
    operation: &NormalizedOperation,
    cancellation: &CancellationToken,
) -> Preflight {
    match preflight_operation(operation, cancellation) {
        Ok(preflight) => preflight,
        Err(error) => Preflight {
            ready: false,
            result: json!({}),
            error: Some(error_value(&error)),
        },
    }
}

fn count_statuses(statuses: &[String]) -> Value {
    let mut counts = HashMap::<&str, usize>::new();
    for status in statuses {
        *counts.entry(status.as_str()).or_default() += 1;
    }
    json!(counts)
}

fn masked_batch_path(path: &Path) -> String {
    cakesplitter_desktop_runtime::masked_path(path, false)
}

fn execute_loaded_run<W: Write, E: Write>(
    loaded: &LoadedJob,
    state_path: &Path,
    resume: Option<(RunState, ResumeOptions)>,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
    receipt: ReceiptArgs,
) -> Result<OperationOutcome, CliError> {
    let _active = acquire_active_batch()?;
    let (mut state, resume_options) = match resume {
        Some((mut state, options)) => {
            prepare_resume_state(&mut state, loaded, options, cancellation)?;
            (state, options)
        }
        None => {
            enforce_retained_run_limit(state_path.parent().unwrap_or_else(|| Path::new(".")))?;
            let state = new_run_state(loaded, session.operation_id());
            create_state(state_path, &state)?;
            (state, ResumeOptions::default())
        }
    };

    session.emit_batch_event(
        "batch-started",
        json!({ "runId": state.run_id, "jobName": loaded.job.name, "jobSpecDigest": loaded.digest }),
    )?;

    if cancellation.is_cancelled() {
        mark_cancelled(&mut state);
        state.terminal_state = RunStatus::Cancelled;
        state.updated_at = now();
        state.event_count = state.event_count.saturating_add(1);
        persist_state(state_path, &mut state)?;
        return batch_outcome(
            &state,
            "Batch cancelled before the first operation",
            "cancelled",
            EXIT_CANCELLED,
            "batch-cancelled",
            &receipt,
            loaded.job.receipt.as_ref(),
            session,
        );
    }

    let completed = state
        .operations
        .iter()
        .filter(|operation| operation.status == OperationStatus::Completed)
        .map(|operation| operation.id.clone())
        .collect::<HashSet<_>>();
    let plan = match plan_job(loaded, session, cancellation, Some(&completed)) {
        Ok(plan) => plan,
        Err(error) if cancellation.is_cancelled() || error.exit_code == EXIT_CANCELLED => {
            mark_cancelled(&mut state);
            state.terminal_state = RunStatus::Cancelled;
            state.updated_at = now();
            state.event_count = state.event_count.saturating_add(1);
            persist_state(state_path, &mut state)?;
            return batch_outcome(
                &state,
                "Batch cancelled during preflight",
                "cancelled",
                EXIT_CANCELLED,
                "batch-cancelled",
                &receipt,
                loaded.job.receipt.as_ref(),
                session,
            );
        }
        Err(error) => return Err(error),
    };
    if !plan["ready"].as_bool().unwrap_or(false) {
        let errors = plan["operations"].as_array().cloned().unwrap_or_default();
        for operation in &mut state.operations {
            let planned = errors.iter().find(|value| value["id"] == operation.id);
            if let Some(planned) = planned {
                if planned["status"] == "blocked" {
                    operation.status = OperationStatus::Blocked;
                } else if planned.get("error").is_some() || planned["ready"] == false {
                    operation.status = OperationStatus::Failed;
                    operation.error = planned.get("error").cloned();
                }
            }
        }
        state.terminal_state = RunStatus::Failed;
        state.updated_at = now();
        state.event_count = state.event_count.saturating_add(1);
        persist_state(state_path, &mut state)?;
        return batch_outcome(
            &state,
            "Batch preflight failed; no operation started",
            "failed",
            EXIT_BATCH_FAILURE,
            "batch-failed",
            &receipt,
            loaded.job.receipt.as_ref(),
            session,
        );
    }

    if cancellation.is_cancelled() {
        mark_cancelled(&mut state);
        state.terminal_state = RunStatus::Cancelled;
        state.updated_at = now();
        persist_state(state_path, &mut state)?;
        return batch_outcome(
            &state,
            "Batch cancelled before the first operation",
            "cancelled",
            EXIT_CANCELLED,
            "batch-cancelled",
            &receipt,
            loaded.job.receipt.as_ref(),
            session,
        );
    }

    let id_to_index = loaded
        .job
        .operations
        .iter()
        .enumerate()
        .map(|(index, operation)| (operation.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut failure_seen = false;
    let mut cancelled = false;
    for index in &loaded.order {
        if cancellation.is_cancelled() {
            cancelled = true;
            mark_cancelled(&mut state);
            break;
        }
        let operation = &loaded.job.operations[*index];
        let state_index = state
            .operations
            .iter()
            .position(|candidate| candidate.id == operation.id)
            .ok_or_else(|| {
                batch_recovery_error(
                    "batch_state_operation_missing",
                    "run state is missing an operation",
                )
            })?;
        let state_status = state.operations[state_index].status;
        if state_status == OperationStatus::Completed {
            session.emit_batch_event(
                "operation-completed",
                json!({ "runId": state.run_id, "operationId": operation.id, "status": "already-completed" }),
            )?;
            continue;
        }
        let dependency_failed = operation.depends_on.iter().any(|dependency| {
            let dependency_index = id_to_index[dependency.as_str()];
            !matches!(
                state.operations[dependency_index].status,
                OperationStatus::Completed
            )
        });
        if dependency_failed {
            state.operations[state_index].status = OperationStatus::Blocked;
            state.operations[state_index].error = Some(
                json!({ "code": "batch_dependency_failed", "message": "a dependency did not complete successfully" }),
            );
            state.event_count = state.event_count.saturating_add(1);
            state.updated_at = now();
            persist_state(state_path, &mut state)?;
            session.emit_batch_event(
                "operation-blocked",
                json!({ "runId": state.run_id, "operationId": operation.id, "reason": "dependency-failed" }),
            )?;
            continue;
        }
        if state_status == OperationStatus::Failed && !resume_options.retry_failed {
            failure_seen = true;
            continue;
        }
        if state_status == OperationStatus::Cancelled && !resume_options.retry_cancelled {
            cancelled = true;
            break;
        }
        if state.operations[state_index].attempt_count >= MAX_OPERATION_ATTEMPTS {
            return Err(batch_capacity_error(
                "batch_operation_attempt_limit",
                "an operation has reached the bounded retry-attempt limit",
            ));
        }
        state.operations[state_index].status = OperationStatus::Running;
        state.operations[state_index].attempt_count = state.operations[state_index]
            .attempt_count
            .saturating_add(1);
        let attempt = state.operations[state_index].attempt_count;
        state.event_count = state.event_count.saturating_add(1);
        state.updated_at = now();
        persist_state(state_path, &mut state)?;
        session.emit_batch_event(
            "operation-started",
            json!({ "runId": state.run_id, "operationId": operation.id, "attempt": attempt }),
        )?;
        let result = execute_batch_operation(operation, session, cancellation);
        match result {
            Ok(outcome) => {
                let evidence = capture_evidence(operation)?;
                let state_operation = state
                    .operations
                    .iter_mut()
                    .find(|candidate| candidate.id == operation.id)
                    .expect("operation was present before execution");
                state_operation.status = OperationStatus::Completed;
                state_operation.result = Some(outcome.result.clone());
                state_operation.error = None;
                state_operation.evidence = Some(evidence);
                state.event_count = state.event_count.saturating_add(1);
                state.updated_at = now();
                persist_state(state_path, &mut state)?;
                session.emit_batch_event(
                    "operation-completed",
                    json!({ "runId": state.run_id, "operationId": operation.id, "status": "completed", "result": outcome.result }),
                )?;
            }
            Err(error) => {
                let error_value = error_value(&error);
                let state_operation = state
                    .operations
                    .iter_mut()
                    .find(|candidate| candidate.id == operation.id)
                    .expect("operation was present before execution");
                state_operation.error = Some(error_value.clone());
                state_operation.status = if error.exit_code == EXIT_CANCELLED {
                    OperationStatus::Cancelled
                } else {
                    OperationStatus::Failed
                };
                state.event_count = state.event_count.saturating_add(1);
                state.updated_at = now();
                persist_state(state_path, &mut state)?;
                session.emit_batch_event(
                    if error.exit_code == EXIT_CANCELLED { "operation-cancelled" } else { "operation-failed" },
                    json!({ "runId": state.run_id, "operationId": operation.id, "error": error_value }),
                )?;
                if error.exit_code == EXIT_CANCELLED {
                    cancelled = true;
                    break;
                }
                failure_seen = true;
                if loaded.job.failure_policy == FailurePolicy::Stop {
                    break;
                }
            }
        }
    }

    if failure_seen && loaded.job.failure_policy == FailurePolicy::Stop {
        for operation in &mut state.operations {
            if matches!(
                operation.status,
                OperationStatus::NotStarted | OperationStatus::Running
            ) {
                operation.status = OperationStatus::Blocked;
                operation.error = Some(json!({
                    "code": "batch_stop_policy",
                    "message": "the stop failure policy prevented this operation from starting"
                }));
            }
        }
    }
    if cancelled {
        mark_cancelled(&mut state);
        state.terminal_state = RunStatus::Cancelled;
    } else if failure_seen
        || state.operations.iter().any(|operation| {
            matches!(
                operation.status,
                OperationStatus::Failed | OperationStatus::Blocked
            )
        })
    {
        state.terminal_state = if loaded.job.failure_policy == FailurePolicy::ContinueIndependent {
            RunStatus::CompletedWithFailures
        } else {
            RunStatus::Failed
        };
    } else {
        state.terminal_state = RunStatus::Completed;
    }
    state.updated_at = now();
    state.event_count = state.event_count.saturating_add(1);
    persist_state(state_path, &mut state)?;
    let (status, event, exit_code, message) = match state.terminal_state {
        RunStatus::Completed => (
            "completed",
            "batch-completed",
            0,
            "Batch completed successfully",
        ),
        RunStatus::CompletedWithFailures => (
            "completed-with-failures",
            "batch-failed",
            EXIT_BATCH_FAILURE,
            "Batch completed with failures",
        ),
        RunStatus::Cancelled => (
            "cancelled",
            "batch-cancelled",
            EXIT_CANCELLED,
            "Batch cancelled safely",
        ),
        RunStatus::Failed => (
            "failed",
            "batch-failed",
            EXIT_BATCH_FAILURE,
            "Batch failed; completed results were preserved",
        ),
        RunStatus::Running | RunStatus::Interrupted => (
            "interrupted",
            "batch-interrupted",
            EXIT_BATCH_FAILURE,
            "Batch was interrupted; resume is required",
        ),
    };
    let mut outcome =
        batch_summary_outcome(&state, message).with_terminal(status, event, exit_code);
    let receipt_result = receipt_for_batch(&receipt, loaded.job.receipt.as_ref());
    apply_batch_receipt(&mut outcome, &receipt_result, None, &state.run_id, "batch")?;
    state.receipt_status = outcome.result.get("receipt").cloned();
    state.updated_at = now();
    state.event_count = state.event_count.saturating_add(1);
    persist_state(state_path, &mut state)?;
    Ok(outcome)
}

fn new_run_state(loaded: &LoadedJob, run_id: &str) -> RunState {
    let operations = loaded
        .job
        .operations
        .iter()
        .map(|operation| PersistedOperation {
            id: operation.id.clone(),
            command: operation.command,
            status: OperationStatus::NotStarted,
            attempt_count: 0,
            result: None,
            error: None,
            evidence: None,
        })
        .collect();
    RunState {
        run_schema_version: BATCH_STATE_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").to_owned(),
        cli_schema_version: CLI_SCHEMA_VERSION,
        batch_job_schema_version: BATCH_JOB_SCHEMA_VERSION,
        cake_package_format: FORMAT_VERSION.to_owned(),
        run_id: run_id.to_owned(),
        job_spec_path: loaded.path.clone(),
        job_spec_digest: loaded.digest.clone(),
        job_name: loaded.job.name.clone(),
        started_at: now(),
        updated_at: now(),
        failure_policy: loaded.job.failure_policy,
        execution_order: loaded
            .order
            .iter()
            .map(|index| loaded.job.operations[*index].id.clone())
            .collect(),
        operations,
        event_count: 0,
        terminal_state: RunStatus::Running,
        revision: 0,
        receipt_status: None,
    }
}

fn resolve_state_path(requested: Option<&Path>, loaded: &LoadedJob) -> Result<PathBuf, CliError> {
    if let Some(path) = requested {
        return validate_state_output_path(path);
    }
    let stem = loaded
        .job
        .name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    validate_state_output_path(
        &loaded
            .path
            .with_file_name(format!("{stem}.cakesplitter.run.json")),
    )
}

fn validate_state_output_path(path: &Path) -> Result<PathBuf, CliError> {
    let path = absolute_path(path)?;
    let parent = path.parent().ok_or_else(|| {
        batch_destination_error(
            "batch_state_parent",
            "run-state path has no parent directory",
        )
    })?;
    canonical_existing_directory(parent)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            batch_destination_error(
                "batch_state_filename",
                "run-state filename must be portable UTF-8",
            )
        })?;
    validate_portable_filename(name).map_err(|error| CliError::from(CoreError::from(error)))?;
    Ok(path)
}

fn validate_state_path(path: &Path) -> Result<PathBuf, CliError> {
    let path = validate_state_output_path(path)?;
    if !path.exists() {
        return Err(batch_recovery_error(
            "batch_state_missing",
            "the persisted Batch run state does not exist",
        ));
    }
    canonical_existing_file(&path)
}

fn enforce_retained_run_limit(parent: &Path) -> Result<(), CliError> {
    let mut completed = 0usize;
    let entries = fs::read_dir(parent).map_err(|source| core_io(parent, source))?;
    for entry in entries.take(MAX_RETAINED_COMPLETED_RUNS + 1) {
        let entry = entry.map_err(|source| core_io(parent, source))?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if !name.ends_with(".cakesplitter.run.json") {
            continue;
        }
        if let Ok(state) = read_state(&entry.path()) {
            if matches!(
                state.terminal_state,
                RunStatus::Completed | RunStatus::CompletedWithFailures
            ) {
                completed = completed.saturating_add(1);
            }
        }
    }
    if completed >= MAX_RETAINED_COMPLETED_RUNS {
        return Err(batch_capacity_error(
            "batch_retained_run_limit",
            "the retained completed-run limit has been reached",
        ));
    }
    Ok(())
}

fn create_state(path: &Path, state: &RunState) -> Result<(), CliError> {
    let envelope = make_envelope(state)?;
    let bytes = serde_json::to_vec_pretty(&envelope)
        .map_err(|error| CliError::internal(error.to_string()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                batch_conflict_error(
                    "batch_state_collision",
                    "the requested run-state file already exists",
                )
            } else {
                core_io(path, source)
            }
        })?;
    if let Err(error) = file
        .write_all(&bytes)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(core_io(path, error));
    }
    Ok(())
}

fn read_state(path: &Path) -> Result<RunState, CliError> {
    let bytes = fs::read(path).map_err(|source| core_io(path, source))?;
    let envelope: StateEnvelope = serde_json::from_slice(&bytes)
        .map_err(|error| batch_recovery_error("batch_state_corrupt", error.to_string()))?;
    if envelope.state_schema_version != BATCH_STATE_SCHEMA_VERSION
        || envelope.state.run_schema_version != BATCH_STATE_SCHEMA_VERSION
        || envelope.state.cli_schema_version != CLI_SCHEMA_VERSION
        || envelope.state.batch_job_schema_version != BATCH_JOB_SCHEMA_VERSION
        || envelope.state.cake_package_format != FORMAT_VERSION
        || envelope.revision != envelope.state.revision
    {
        return Err(batch_recovery_error(
            "batch_state_unsupported",
            "the persisted run-state schema is unsupported",
        ));
    }
    let checksum = state_checksum(&envelope.state)?;
    if checksum != envelope.checksum {
        return Err(batch_recovery_error(
            "batch_state_corrupt",
            "the persisted run-state checksum does not match",
        ));
    }
    Ok(envelope.state)
}

fn persist_state(path: &Path, state: &mut RunState) -> Result<(), CliError> {
    if state.event_count > MAX_PERSISTED_EVENTS {
        return Err(batch_capacity_error(
            "batch_event_limit",
            "the persisted run has exceeded the bounded event limit",
        ));
    }
    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| core_io(&lock_path, source))?;
    FileExt::try_lock(&lock).map_err(|_| {
        batch_recovery_error(
            "batch_state_stale",
            "another process is writing this run state",
        )
    })?;
    let current = read_state(path)?;
    if current.revision != state.revision {
        let _ = FileExt::unlock(&lock);
        return Err(batch_recovery_error(
            "batch_state_stale",
            "the run state changed since it was read",
        ));
    }
    state.revision = state.revision.saturating_add(1);
    let envelope = make_envelope(state)?;
    let bytes = serde_json::to_vec_pretty(&envelope)
        .map_err(|error| CliError::internal(error.to_string()))?;
    let temporary = path.with_extension(format!("run.json.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| core_io(&temporary, source))?;
        file.write_all(&bytes)
            .and_then(|_| file.flush())
            .and_then(|_| file.sync_all())
            .map_err(|source| core_io(&temporary, source))?;
        atomic_replace(&temporary, path).map_err(|source| core_io(path, source))
    })();
    let _ = fs::remove_file(&temporary);
    let _ = FileExt::unlock(&lock);
    result
}

fn make_envelope(state: &RunState) -> Result<StateEnvelope, CliError> {
    Ok(StateEnvelope {
        state_schema_version: BATCH_STATE_SCHEMA_VERSION,
        revision: state.revision,
        checksum: state_checksum(state)?,
        state: state.clone(),
    })
}

fn state_checksum(state: &RunState) -> Result<String, CliError> {
    let bytes = serde_json::to_vec(state).map_err(|error| CliError::internal(error.to_string()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let ok = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

fn prepare_resume_state(
    state: &mut RunState,
    loaded: &LoadedJob,
    options: ResumeOptions,
    cancellation: &CancellationToken,
) -> Result<(), CliError> {
    if state.job_spec_digest != loaded.digest
        || state.job_name != loaded.job.name
        || state.execution_order
            != loaded
                .order
                .iter()
                .map(|index| loaded.job.operations[*index].id.clone())
                .collect::<Vec<_>>()
    {
        return Err(batch_recovery_error(
            "batch_spec_digest_mismatch",
            "the persisted run does not match the normalized Batch Job specification",
        ));
    }
    if state.event_count > MAX_PERSISTED_EVENTS {
        return Err(batch_capacity_error(
            "batch_event_limit",
            "the persisted run has exceeded the bounded event limit",
        ));
    }
    let index_by_id = loaded
        .job
        .operations
        .iter()
        .enumerate()
        .map(|(index, operation)| (operation.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    for persisted in &mut state.operations {
        if persisted.status == OperationStatus::Completed {
            validate_completed_operation(
                &loaded.job.operations[index_by_id[persisted.id.as_str()]],
                persisted.evidence.as_ref(),
            )?;
        } else if persisted.status == OperationStatus::Running
            || persisted.status == OperationStatus::Interrupted
        {
            persisted.status = OperationStatus::NotStarted;
        } else if persisted.status == OperationStatus::Failed {
            let retryable = persisted
                .error
                .as_ref()
                .and_then(|error| error["retryable"].as_bool())
                .unwrap_or(false);
            if options.retry_failed && retryable {
                persisted.status = OperationStatus::NotStarted;
            }
        } else if persisted.status == OperationStatus::Cancelled && options.retry_cancelled {
            persisted.status = OperationStatus::NotStarted;
        }
    }
    if cancellation.is_cancelled() {
        return Err(CliError::from(CoreError::Cancelled));
    }
    state.terminal_state = RunStatus::Running;
    state.updated_at = now();
    Ok(())
}

fn validate_completed_operation(
    operation: &NormalizedOperation,
    evidence: Option<&OperationEvidence>,
) -> Result<(), CliError> {
    let evidence = evidence.ok_or_else(|| {
        batch_recovery_error(
            "batch_completed_binding_missing",
            "completed operation has no persisted identity evidence",
        )
    })?;
    match operation.command {
        BatchOperationKind::Split => {
            let source = operation.file.as_deref().ok_or_else(|| {
                batch_recovery_error(
                    "batch_completed_binding",
                    "completed split is missing its source",
                )
            })?;
            let source_fingerprint = fingerprint_file(source).map_err(|error| {
                batch_recovery_error("batch_input_identity_changed", error.to_string())
            })?;
            let output_dir = operation
                .output_dir
                .as_deref()
                .or_else(|| source.parent())
                .ok_or_else(|| {
                    batch_recovery_error(
                        "batch_completed_binding",
                        "completed split is missing its output directory",
                    )
                })?;
            let directory_fingerprint = fingerprint_directory(output_dir).map_err(|error| {
                batch_recovery_error("batch_output_identity_changed", error.to_string())
            })?;
            if evidence.source.as_ref() != Some(&source_fingerprint)
                || evidence.directory.as_ref() != Some(&directory_fingerprint)
            {
                return Err(batch_recovery_error(
                    "batch_completed_binding_changed",
                    "completed split input or output identity changed",
                ));
            }
        }
        BatchOperationKind::Merge => {
            let output = operation.output.as_deref().ok_or_else(|| {
                batch_recovery_error(
                    "batch_completed_binding",
                    "completed merge is missing its output",
                )
            })?;
            let output_fingerprint = fingerprint_file(output).map_err(|error| {
                batch_recovery_error("batch_output_identity_changed", error.to_string())
            })?;
            let package = operation.package.as_deref().ok_or_else(|| {
                batch_recovery_error(
                    "batch_completed_binding",
                    "completed merge is missing its package",
                )
            })?;
            let binding = capture_package_binding(
                &resolve_manifest_for_batch(package)?,
                &CancellationToken::new(),
            )
            .map_err(|error| {
                batch_recovery_error("batch_package_identity_changed", error.to_string())
            })?;
            let current = evidence_from_binding(&binding);
            if evidence.output.as_ref() != Some(&output_fingerprint)
                || current.package_directory != evidence.package_directory
                || current.manifest_sha256 != evidence.manifest_sha256
                || current.membership_sha256 != evidence.membership_sha256
            {
                return Err(batch_recovery_error(
                    "batch_completed_binding_changed",
                    "completed merge input or output identity changed",
                ));
            }
        }
        BatchOperationKind::Inspect | BatchOperationKind::Verify => {
            let package = operation.package.as_deref().ok_or_else(|| {
                batch_recovery_error(
                    "batch_completed_binding",
                    "completed package operation is missing its package",
                )
            })?;
            let manifest = resolve_manifest_for_batch(package)?;
            let binding =
                capture_package_binding(&manifest, &CancellationToken::new()).map_err(|error| {
                    batch_recovery_error("batch_package_identity_changed", error.to_string())
                })?;
            let current = evidence_from_binding(&binding);
            if current.package_directory != evidence.package_directory
                || current.package_manifest != evidence.package_manifest
                || current.manifest_sha256 != evidence.manifest_sha256
                || current.membership_sha256 != evidence.membership_sha256
            {
                return Err(batch_recovery_error(
                    "batch_completed_binding_changed",
                    "completed package identity changed",
                ));
            }
        }
    }
    Ok(())
}

fn resolve_manifest_for_batch(package: &Path) -> Result<PathBuf, CliError> {
    let package = absolute_path(package)?;
    if package.is_dir() {
        let directory = canonical_existing_directory(&package)?;
        cakesplitter_core::find_package_manifest(&directory, &CancellationToken::new())
            .map_err(CliError::from)
    } else {
        canonical_existing_file(&package)
    }
}

fn execute_batch_operation<W: Write, E: Write>(
    operation: &NormalizedOperation,
    session: &mut OutputSession<'_, W, E>,
    cancellation: &CancellationToken,
) -> Result<OperationOutcome, CliError> {
    session.emit_batch_event(
        "operation-ready",
        json!({ "runId": session.operation_id(), "operationId": operation.id }),
    )?;
    let command = operation_command(operation);
    let mut nested_stdout = NestedEventCapture::default();
    let mut nested_stderr = Vec::new();
    let nested_id = Uuid::new_v4().to_string();
    let mut nested = OutputSession::new(
        OutputFormat::Jsonl,
        operation_name(operation.command),
        nested_id,
        &mut nested_stdout,
        &mut nested_stderr,
        false,
    )?;
    let result = match command {
        Command::Split(arguments) => crate::execute_split(&arguments, &mut nested, cancellation),
        Command::Merge(arguments) => crate::execute_merge(&arguments, &mut nested, cancellation),
        Command::Inspect(arguments) => {
            crate::execute_inspect(&arguments, &mut nested, cancellation)
        }
        Command::Verify(arguments) => crate::execute_verify(&arguments, &mut nested, cancellation),
        _ => Err(batch_usage_error(
            "batch_unsupported_command",
            "batch dispatch accepted an unsupported command",
        )),
    };
    drop(nested);
    for value in nested_stdout.events {
        match value["event"].as_str() {
            Some("progress") => {
                session.emit_batch_event(
                        "operation-progress",
                        json!({ "runId": session.operation_id(), "operationId": operation.id, "payload": value["payload"] }),
                    )?;
            }
            Some("warning") => {
                session.emit_batch_event(
                        "operation-warning",
                        json!({ "runId": session.operation_id(), "operationId": operation.id, "payload": value["payload"] }),
                    )?;
            }
            _ => {}
        }
    }
    result
}

fn operation_command(operation: &NormalizedOperation) -> Command {
    let receipt = operation.receipt.as_ref().map_or_else(
        || ReceiptArgs {
            receipt: None,
            receipt_format: ReceiptFormat::Json,
        },
        |receipt| ReceiptArgs {
            receipt: Some(receipt.path.clone()),
            receipt_format: receipt.format,
        },
    );
    match operation.command {
        BatchOperationKind::Split => Command::Split(SplitArgs {
            plan: SplitPlanArgs {
                file: operation.file.clone().expect("validated split file"),
                slice_size: operation.slice_size,
                slice_count: operation.slice_count,
                output_dir: operation.output_dir.clone(),
            },
            dry_run: false,
            receipt,
        }),
        BatchOperationKind::Merge => Command::Merge(MergeArgs {
            plan: MergePlanArgs {
                package: operation.package.clone().expect("validated merge package"),
                slices: operation.slices.clone(),
                output: operation.output.clone().expect("validated merge output"),
            },
            dry_run: false,
            receipt,
        }),
        BatchOperationKind::Inspect => Command::Inspect(InspectArgs {
            package: PackageArgs {
                package: operation
                    .package
                    .clone()
                    .expect("validated inspect package"),
                slices: operation.slices.clone(),
            },
            manifest_only: operation.manifest_only,
            receipt,
        }),
        BatchOperationKind::Verify => Command::Verify(VerifyArgs {
            package: PackageArgs {
                package: operation.package.clone().expect("validated verify package"),
                slices: operation.slices.clone(),
            },
            receipt,
        }),
    }
}

fn operation_name(command: BatchOperationKind) -> &'static str {
    match command {
        BatchOperationKind::Split => "split",
        BatchOperationKind::Merge => "merge",
        BatchOperationKind::Inspect => "inspect",
        BatchOperationKind::Verify => "verify",
    }
}

fn capture_evidence(operation: &NormalizedOperation) -> Result<OperationEvidence, CliError> {
    match operation.command {
        BatchOperationKind::Split => {
            let source = operation
                .file
                .as_deref()
                .ok_or_else(|| batch_recovery_error("batch_evidence", "split source missing"))?;
            let output_dir = operation
                .output_dir
                .as_deref()
                .or_else(|| source.parent())
                .ok_or_else(|| {
                    batch_recovery_error("batch_evidence", "split output directory missing")
                })?;
            Ok(OperationEvidence {
                source: Some(fingerprint_file(source).map_err(CliError::from)?),
                directory: Some(fingerprint_directory(output_dir).map_err(CliError::from)?),
                ..OperationEvidence::default()
            })
        }
        BatchOperationKind::Merge => {
            let output = operation
                .output
                .as_deref()
                .ok_or_else(|| batch_recovery_error("batch_evidence", "merge output missing"))?;
            let package = operation
                .package
                .as_deref()
                .ok_or_else(|| batch_recovery_error("batch_evidence", "merge package missing"))?;
            let manifest = resolve_manifest_for_batch(package)?;
            let binding = capture_package_binding(&manifest, &CancellationToken::new())
                .map_err(CliError::from)?;
            let mut evidence = evidence_from_binding(&binding);
            evidence.output = Some(fingerprint_file(output).map_err(CliError::from)?);
            Ok(evidence)
        }
        BatchOperationKind::Inspect | BatchOperationKind::Verify => {
            let package = operation
                .package
                .as_deref()
                .ok_or_else(|| batch_recovery_error("batch_evidence", "package missing"))?;
            let manifest = resolve_manifest_for_batch(package)?;
            let binding = capture_package_binding(&manifest, &CancellationToken::new())
                .map_err(CliError::from)?;
            Ok(evidence_from_binding(&binding))
        }
    }
}

fn evidence_from_binding(binding: &PackageBinding) -> OperationEvidence {
    OperationEvidence {
        package_manifest: Some(binding.manifest_identity.clone()),
        package_directory: Some(binding.package_directory.clone()),
        manifest_sha256: Some(binding.manifest_sha256.clone()),
        membership_sha256: Some(binding.membership_sha256.clone()),
        ..OperationEvidence::default()
    }
}

fn batch_summary_outcome(state: &RunState, message: &str) -> OperationOutcome {
    OperationOutcome::new(state_result(state), message)
}

#[allow(clippy::too_many_arguments)]
fn batch_outcome<W: Write, E: Write>(
    state: &RunState,
    message: &str,
    status: &str,
    exit_code: u8,
    event: &str,
    receipt: &ReceiptArgs,
    spec_receipt: Option<&BatchReceiptSpec>,
    session: &OutputSession<'_, W, E>,
) -> Result<OperationOutcome, CliError> {
    let mut outcome = batch_summary_outcome(state, message).with_terminal(status, event, exit_code);
    let receipt = receipt_for_batch(receipt, spec_receipt);
    apply_batch_receipt(&mut outcome, &receipt, None, &state.run_id, "batch")?;
    let _ = session;
    Ok(outcome)
}

fn receipt_for_batch(cli: &ReceiptArgs, spec: Option<&BatchReceiptSpec>) -> ReceiptArgs {
    if cli.receipt.is_some() {
        return cli.clone();
    }
    spec.map_or_else(
        || ReceiptArgs {
            receipt: None,
            receipt_format: ReceiptFormat::Json,
        },
        |receipt| ReceiptArgs {
            receipt: Some(receipt.path.clone()),
            receipt_format: receipt.format,
        },
    )
}

fn apply_batch_receipt(
    outcome: &mut OperationOutcome,
    cli: &ReceiptArgs,
    spec: Option<&BatchReceiptSpec>,
    operation_id: &str,
    command: &str,
) -> Result<(), CliError> {
    let receipt = receipt_for_batch(cli, spec);
    apply_receipt(outcome, &receipt, command, operation_id, "completed")
}

fn state_result(state: &RunState) -> Value {
    let operations = state
        .operations
        .iter()
        .map(|operation| {
            json!({
                "id": operation.id,
                "command": operation.command,
                "status": operation.status,
                "attemptCount": operation.attempt_count,
                "result": operation.result,
                "error": operation.error
            })
        })
        .collect::<Vec<_>>();
    let statuses = state
        .operations
        .iter()
        .map(|operation| format_status(operation.status))
        .collect::<Vec<_>>();
    json!({
        "runSchemaVersion": state.run_schema_version,
        "applicationVersion": state.application_version,
        "cliSchemaVersion": state.cli_schema_version,
        "batchJobSchemaVersion": state.batch_job_schema_version,
        "cakePackageFormat": state.cake_package_format,
        "runId": state.run_id,
        "jobName": state.job_name,
        "jobSpecDigest": state.job_spec_digest,
        "failurePolicy": state.failure_policy,
        "executionOrder": state.execution_order,
        "terminalState": state.terminal_state,
        "operationCounts": count_statuses(&statuses),
        "operations": operations,
        "eventCount": state.event_count,
        "startedAt": state.started_at,
        "updatedAt": state.updated_at,
        "receiptStatus": state.receipt_status,
        "limits": batch_limits()
    })
}

fn format_status(status: OperationStatus) -> String {
    match status {
        OperationStatus::NotStarted => "not-started",
        OperationStatus::Running => "running",
        OperationStatus::Completed => "completed",
        OperationStatus::Failed => "failed",
        OperationStatus::Cancelled => "cancelled",
        OperationStatus::Blocked => "blocked",
        OperationStatus::Interrupted => "interrupted",
    }
    .to_owned()
}

fn mark_cancelled(state: &mut RunState) {
    for operation in &mut state.operations {
        if matches!(
            operation.status,
            OperationStatus::NotStarted | OperationStatus::Running
        ) {
            operation.status = OperationStatus::Cancelled;
        }
    }
}

fn error_value(error: &CliError) -> Value {
    serde_json::to_value(error).unwrap_or_else(|_| {
        json!({
            "code": "internal_failure",
            "category": "internal",
            "message": "the batch error could not be serialized",
            "retryable": false
        })
    })
}

fn batch_usage_error(code: &str, technical: impl Into<String>) -> CliError {
    CliError::usage(code, "The Batch Job specification is invalid.", technical)
}

fn batch_capacity_error(code: &str, technical: impl Into<String>) -> CliError {
    CliError {
        code: code.to_owned(),
        category: CliErrorCategory::Capacity,
        message: "The Batch Job exceeds a documented local capacity limit.".to_owned(),
        technical_message: technical.into(),
        retryable: false,
        suggested_action: "Reduce the Batch Job size or retained run count and retry.".to_owned(),
        operation_id: None,
        exit_code: EXIT_CAPACITY,
    }
}

fn batch_recovery_error(code: &str, technical: impl Into<String>) -> CliError {
    CliError {
        code: code.to_owned(),
        category: CliErrorCategory::Recovery,
        message: "The persisted Batch Job cannot be resumed safely.".to_owned(),
        technical_message: technical.into(),
        retryable: false,
        suggested_action: "Inspect the run state and restart with an unchanged Job specification."
            .to_owned(),
        operation_id: None,
        exit_code: EXIT_RECOVERY,
    }
}

fn batch_conflict_error(code: &str, technical: impl Into<String>) -> CliError {
    CliError {
        code: code.to_owned(),
        category: CliErrorCategory::Conflict,
        message: "The requested Batch Job state already exists and was not overwritten.".to_owned(),
        technical_message: technical.into(),
        retryable: true,
        suggested_action: "Choose a new run-state path or inspect the existing run explicitly."
            .to_owned(),
        operation_id: None,
        exit_code: EXIT_CONFLICT,
    }
}

fn batch_destination_error(code: &str, message: &str) -> CliError {
    CliError {
        code: code.to_owned(),
        category: CliErrorCategory::Destination,
        message: message.to_owned(),
        technical_message: message.to_owned(),
        retryable: false,
        suggested_action: "Choose a stable local directory for the run state.".to_owned(),
        operation_id: None,
        exit_code: crate::error::EXIT_DESTINATION,
    }
}

fn core_io(path: &Path, source: std::io::Error) -> CliError {
    CliError::from(CoreError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn pre_cancelled_batch_persists_cancelled_state_and_one_terminal_jsonl_event() {
        let root = tempdir().unwrap();
        let source = root.path().join("source.bin");
        let output = root.path().join("output");
        let state = root.path().join("run.json");
        let spec = root.path().join("job.json");
        fs::write(&source, b"cancel before first").unwrap();
        fs::create_dir(&output).unwrap();
        fs::write(
            &spec,
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": 1,
                "name": "cancel-before-first",
                "failurePolicy": "stop",
                "operations": [{
                    "id": "split",
                    "command": "split",
                    "file": source,
                    "sliceSize": "4B",
                    "outputDir": output
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let command = BatchCommand::Run(BatchRunArgs {
            job_spec: spec,
            state: Some(state.clone()),
            receipt: ReceiptArgs {
                receipt: None,
                receipt_format: ReceiptFormat::Json,
            },
        });
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut session = OutputSession::new(
            OutputFormat::Jsonl,
            "batch",
            Uuid::new_v4().to_string(),
            &mut stdout,
            &mut stderr,
            false,
        )
        .unwrap();
        let outcome = execute(&command, &mut session, &cancellation).unwrap();
        assert_eq!(outcome.terminal_status, "cancelled");
        assert_eq!(outcome.exit_code, EXIT_CANCELLED);
        assert_eq!(session.finish_success(&outcome).unwrap(), EXIT_CANCELLED);
        assert!(stderr.is_empty());

        let events = String::from_utf8(stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event["event"].as_str(),
                    Some(
                        "batch-completed"
                            | "batch-failed"
                            | "batch-cancelled"
                            | "batch-interrupted"
                    )
                ))
                .count(),
            1
        );
        assert_eq!(events.last().unwrap()["event"], "batch-cancelled");
        assert_eq!(
            events.last().unwrap()["runId"],
            events.last().unwrap()["payload"]["runId"]
        );
        let stored: Value = serde_json::from_slice(&fs::read(state).unwrap()).unwrap();
        assert_eq!(stored["state"]["terminalState"], "cancelled");
        assert_eq!(stored["state"]["operations"][0]["status"], "cancelled");
        assert!(!output.join("source.bin.cake.json").exists());
    }
}
