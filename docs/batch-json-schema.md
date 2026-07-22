# Batch output schema

Batch output reuses CLI schema version `1` and keeps the Batch Job schema
version separate at `1`. JSON output contains one final envelope whose
`result` includes `runId`, `jobName`, `jobSpecDigest`, `failurePolicy`,
`operationCounts`, `operations`, and the terminal batch state. JSONL uses the
same CLI envelope fields as other commands and places `runId` in every batch
event and payload, with monotonic `sequence` values.

Example final result:

```json
{
  "schemaVersion": 1,
  "applicationVersion": "0.6.0-dev",
  "command": "batch",
  "status": "completed",
  "result": {
    "runSchemaVersion": 1,
    "batchJobSchemaVersion": 1,
    "cakePackageFormat": "1.0",
    "runId": "local-run-id",
    "jobName": "nightly-package-check",
    "jobSpecDigest": "sha256",
    "failurePolicy": "stop",
    "terminalState": "completed",
    "operationCounts": { "completed": 1 },
    "operations": [{ "id": "verify-package", "command": "verify", "status": "completed", "attemptCount": 1 }]
  },
  "warnings": [],
  "error": null
}
```

Batch JSONL terminal events are exactly one of `batch-completed`,
`batch-failed`, `batch-cancelled`, or `batch-interrupted`. Operation events
include `operationId`; all batch events and payloads include `runId`. Progress and warning
events are bounded samples (at most 20 per operation).
