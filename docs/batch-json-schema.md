# Batch output schema (`v0.7.0`)

Batch output reuses CLI schema version `1` and keeps the Batch Job schema
version separate at `1`. JSON output contains one final envelope with
`command: "batch"`. For a loaded Job, the envelope adds top-level `runId`,
`jobName`, `jobSpecDigest`, `failurePolicy`, `operationCounts`, and
`operations`; the command-specific `result` retains the detailed run summary.
JSONL uses the same CLI envelope fields as other commands, adds a stable
top-level `runId` to every batch event, and uses contiguous monotonic
`sequence` values.

Example final result:

```json
{
  "schemaVersion": 1,
  "applicationVersion": "0.7.0",
  "command": "batch",
  "status": "completed",
  "startedAt": "2026-07-23T12:00:00.000Z",
  "completedAt": "2026-07-23T12:00:00.100Z",
  "durationMs": 100,
  "runId": "ff7cb026-f7ec-4d17-a3e4-8083217ec688",
  "jobName": "nightly-package-check",
  "jobSpecDigest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "failurePolicy": "stop",
  "operationCounts": { "completed": 1 },
  "operations": [{ "id": "verify-package", "command": "verify", "status": "completed", "attemptCount": 1, "result": { "verified": true }, "error": null }],
  "result": {
    "runSchemaVersion": 1,
    "batchJobSchemaVersion": 1,
    "cakePackageFormat": "1.0",
    "runId": "ff7cb026-f7ec-4d17-a3e4-8083217ec688",
    "jobName": "nightly-package-check",
    "jobSpecDigest": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "failurePolicy": "stop",
    "terminalState": "completed",
    "operationCounts": { "completed": 1 },
    "operations": [{ "id": "verify-package", "command": "verify", "status": "completed", "attemptCount": 1, "result": { "verified": true }, "error": null }]
  },
  "warnings": [],
  "error": null
}
```

Batch JSONL terminal events are exactly one of `batch-completed`,
`batch-failed`, `batch-cancelled`, or `batch-interrupted`. Operation events
include their operation ID; all batch events and payloads include the stable
`runId`. Progress and warning events are bounded samples (at most 20 per
operation).
