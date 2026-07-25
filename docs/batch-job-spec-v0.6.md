# CakeSplitter v0.6 Batch Job specification (current in `v0.8.1`)

The filename preserves the v0.6 specification lineage. The current private
application version is `0.8.1`; Batch Job schema version `1`
and Cake Package format `1.0` remain unchanged.

The Batch Job schema is version `1`. It is independent from development
application version `0.8.1`, CLI output schema `1`, and Cake Package format
`1.0`.
The machine-readable schema is [specs/batch-job.schema.json](../specs/batch-job.schema.json).

## Commands

```text
cakesplitter batch validate JOB.json
cakesplitter batch plan JOB.json
cakesplitter batch run JOB.json [--state RUN.json]
cakesplitter batch resume RUN.json [--job-spec JOB.json] [--retry-failed] [--retry-cancelled]
cakesplitter batch status RUN.json
```

All commands support the existing `--format human|json|jsonl` modes. Validate,
plan, and status do not process files. Plan performs operation preflight and
reports conflicts and space limits without creating output or run state.

## Structure

```json
{
  "schemaVersion": 1,
  "name": "nightly-package-check",
  "failurePolicy": "stop",
  "operations": [
    {
      "id": "verify-package",
      "command": "verify",
      "package": "./package/file.cake.json"
    }
  ]
}
```

Supported operations are `split`, `merge`, `inspect`, and `verify`. Split uses
`file`, exactly one of `sliceSize` or `sliceCount`, and optional `outputDir`.
Merge uses `package`, `output`, and optional complete `slices`. Inspect and
verify use `package`, optional complete `slices`, and `manifestOnly` only for
inspect. Each operation may request a redacted JSON or Markdown receipt with
`receipt: {"path": "...", "format": "json|markdown"}`.

## Limits

| Limit | Value |
| --- | ---: |
| Job specification | 8 MiB |
| Operations | 1,000 |
| Dependencies per operation | 128 |
| Total dependency edges | 10,000 |
| Operation ID | 128 UTF-8 bytes |
| Job name | 256 UTF-8 bytes |
| Description | 2,000 UTF-8 bytes |
| Metadata | 64 KiB |
| Selected Slices per operation | 50,000 |
| Receipt path | 200 UTF-8 bytes |
| JSON nesting | 24 levels |
| Persisted state events | 10,000 |
| Diagnostic/progress samples per operation | 20 |
| Retained completed run states | 32 |
| Active batch executions | 1 per process |

Oversized input is rejected before operation planning or filesystem access.
Operations are never truncated to fit a limit.

## Dependencies and policies

Dependencies are explicit operation IDs. The graph rejects duplicate IDs,
unknown dependencies, and cycles. Topological order is deterministic: when
multiple operations are eligible, specification order wins. A dependent
operation never starts before all dependencies complete successfully.

`stop` halts new work after the first failure or cancellation. Remaining
operations are persisted as blocked or not-started. `continue-independent`
continues operations whose dependencies succeeded and blocks dependents of a
failure. Batch statuses are `completed`, `completed-with-failures`, `failed`,
`cancelled`, and `interrupted`.

Execution is sequential with one active file-processing operation. There is no
shell, executable, script, HTTP, plugin, recursive batch, daemon, or cloud
execution capability. JSONL progress and warning events from an operation are
sampled to the bounded diagnostic limit rather than buffered without a bound.

## Machine-readable output

Batch JSON uses the CLI schema 1 envelope with `command: "batch"`. A loaded
Job result adds top-level `runId`, `jobName`, `jobSpecDigest`,
`failurePolicy`, `operationCounts`, and `operations`. The same metadata remains
available inside the command-specific `result` summary for compatibility with
existing consumers. A batch JSONL stream uses a stable top-level `runId` on
every event and emits `batch-*` and `operation-*` lifecycle events. Its
sequence starts at one, is contiguous and monotonic, and ends with exactly one
terminal event.

## Paths and resume

Relative paths resolve against `workingDirectory`, or the Job specification's
parent when omitted. Paths are literal data: no environment variables, `~`,
wildcards, command substitution, or shell expansion are performed. Existing
CakeSplitter reparse-point, filename, identity, collision, and integrity rules
remain authoritative.

Run state stores schema versions, run ID, normalized Job digest, operation
states, attempt counts, redacted result summaries, and bounded event count. It
is checksummed, written through a temporary file and atomic replacement, and
guarded by a local writer lock. Corrupt or stale state fails closed.

Resume reloads the same normalized specification and requires the SHA-256
digest and deterministic operation order to match. Completed operations are
not rerun; their persisted native identity evidence is revalidated first.
Running/interrupted operations may recover through the existing core controls.
Failed operations require `--retry-failed` and must be retryable; cancelled
operations require `--retry-cancelled`.

Ctrl+C stops admission, preserves completed operations, persists a cancelled
state, and emits one terminal `batch-cancelled` JSONL event. A process crash
leaves a running state that is treated as interrupted on the next resume.
