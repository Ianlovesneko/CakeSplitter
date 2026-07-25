# CLI JSON Schema

The v0.6 contract lineage introduced CLI schema version `1`; the current
public pre-release application version is `0.8.1`. This schema version is
independent from application version `0.8.1` and Cake Package format
version `1.0`.

Authoritative schema files:

- [`cli-final-result.schema.json`](../specs/cli-final-result.schema.json)
- [`cli-jsonl-event.schema.json`](../specs/cli-jsonl-event.schema.json)
- [`cli-error.schema.json`](../specs/cli-error.schema.json)

The TypeScript runtime validators and interfaces live in
`packages/shared-types/src/index.ts` and reject unknown or malformed contract
fields.

Compatibility rules:

- additive optional fields may be introduced within schema version `1`, while
  batch-specific fields are required for loaded batch results;
- existing field meanings and terminal-state semantics do not change silently;
- removing a field, changing a field type, or changing a field's meaning
  requires a CLI schema-version increment; and
- a CLI schema increment does not imply a Cake Package format change.

## Final JSON document

`--format json` writes one document to stdout. This is the shape emitted by
`inspect empty.cake.json --manifest-only --format json` (timestamps are
operation-specific):

```json
{
  "schemaVersion": 1,
  "applicationVersion": "0.8.1",
  "command": "inspect",
  "status": "completed",
  "result": {
    "duplicateSlices": [],
    "manifest": {
      "createdAt": "2026-07-16T04:00:00Z",
      "format": "cakesplitter",
      "original": {
        "filename": "empty.bin",
        "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "size": 0
      },
      "packageId": "ff7cb026-f7ec-4d17-a3e4-8083217ec688",
      "sliceCount": 0,
      "slices": [],
      "targetSliceSize": 1048576,
      "version": "1.0"
    },
    "manifestFilename": "empty.cake.json",
    "ready": true,
    "type": "manifest"
  },
  "warnings": [],
  "error": null,
  "startedAt": "2026-07-21T12:00:00.000Z",
  "completedAt": "2026-07-21T12:00:00.012Z",
  "durationMs": 12
}
```

Duplicate or repeated Slice names are rejected while the Manifest is parsed,
before a final result document is emitted. The `duplicateSlices` field remains
in the schema for forward-compatible diagnostics and is therefore empty for
accepted v0.6 results.

Command-specific `result` objects contain the documented command evidence. The
envelope remains schema version `1`.

## JSONL events

`--format jsonl` writes one valid object per line. This is a complete
four-event one-Slice operation shape; free-space values and timestamps are
operation-specific, but every shown object is emitted by the executable:

```jsonl
{"schemaVersion":1,"event":"started","command":"split","operationId":"ff7cb026-f7ec-4d17-a3e4-8083217ec688","timestamp":"2026-07-21T12:00:00.000Z","sequence":1,"payload":{"applicationVersion":"0.8.1"}}
{"schemaVersion":1,"event":"preflight","command":"split","operationId":"ff7cb026-f7ec-4d17-a3e4-8083217ec688","timestamp":"2026-07-21T12:00:00.001Z","sequence":2,"payload":{"availableFreeSpace":1073741824,"cakePackageFormat":"1.0","compatibilityLimits":{"maximumFilenameBytes":200,"maximumManifestBytes":16777216,"maximumSafeInteger":9007199254740991,"maximumSliceCount":50000},"conflicts":[],"expectedOutputNames":["example.bin.001.slice","example.bin.cake.json"],"expectedSliceCount":1,"ready":true,"requiredFreeSpace":3,"sourceFilename":"example.bin","sourceSize":3,"targetSliceSize":3,"type":"split","warnings":[]}}
{"schemaVersion":1,"event":"progress","command":"split","operationId":"ff7cb026-f7ec-4d17-a3e4-8083217ec688","timestamp":"2026-07-21T12:00:00.010Z","sequence":3,"payload":{"operation":"split","bytesProcessed":3,"totalBytes":3,"currentSlice":1,"sliceCount":1}}
{"schemaVersion":1,"event":"completed","command":"split","operationId":"ff7cb026-f7ec-4d17-a3e4-8083217ec688","timestamp":"2026-07-21T12:00:00.020Z","sequence":4,"payload":{"durationMs":20,"result":{"cakePackageFormat":"1.0","manifestFilename":"example.bin.cake.json","outputDirectory":"…\\package","sliceCount":1,"sliceSize":3,"sourceFilename":"example.bin","sourceSha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","sourceSize":3,"type":"split"},"status":"completed","warnings":[]}}
```

An event stream has one operation ID, strictly increasing sequence values, and
exactly one terminal event: `completed`, `failed`, or `cancelled`.

Batch commands use the same envelope. Every batch event carries a top-level
`runId` and every batch payload repeats that identifier; the `batch-started`
payload additionally carries `jobName`, while the envelope also carries
`operationId`. A batch stream emits lifecycle events such as
`batch-started`, `operation-started`, `operation-progress`,
`operation-completed`, `operation-failed`, `operation-blocked`, and one
terminal `batch-completed`, `batch-failed`, or `batch-cancelled` event. The
batch job schema and bounded limits are documented in
`docs/batch-job-spec-v0.6.md` and `specs/batch-job.schema.json`. Batch final
results use `command: "batch"` and carry `runId`, `jobName`, `jobSpecDigest`,
`failurePolicy`, `operationCounts`, and `operations` at the top level. An
early validation or recovery error may carry only the established `runId`
because no Job metadata was safely loaded.

## Structured error result

Handled failures remain parseable:

```json
{
  "schemaVersion": 1,
  "applicationVersion": "0.8.1",
  "command": "split",
  "status": "failed",
  "result": null,
  "warnings": [],
  "error": {
    "code": "output_collision",
    "category": "conflict",
    "message": "A planned output already exists; CakeSplitter did not overwrite it.",
    "technicalMessage": "1 output conflict(s) detected",
    "retryable": true,
    "suggestedAction": "Choose a new output path or remove conflicts explicitly.",
    "operationId": "ff7cb026-f7ec-4d17-a3e4-8083217ec688"
  },
  "startedAt": "2026-07-21T12:00:00.000Z",
  "completedAt": "2026-07-21T12:00:00.001Z",
  "durationMs": 1
}
```

Machine output contains no ANSI decoration. Unicode bidirectional controls are
encoded as JSON Unicode escapes in the byte stream. Diagnostic strings use
centralized path, credential, environment, and secret redaction.
