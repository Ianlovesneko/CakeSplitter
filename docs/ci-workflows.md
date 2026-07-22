# Local CI workflow examples

These examples are intentionally provider-neutral and process files locally.
They do not configure a remote, upload selected files, or add cloud execution.

```powershell
# Validate the specification before any processing.
cakesplitter batch validate .\examples\batch\verify-package.json --format json

# Plan and fail the CI step if preflight is not ready.
cakesplitter batch plan .\examples\batch\verify-package.json --format json

# Consume one final JSON result.
cakesplitter batch run .\examples\batch\verify-package.json --state .\run.json --format json

# Stream progress without terminal decoration.
cakesplitter batch run .\examples\batch\verify-package.json --state .\run.json --format jsonl

# Resume only with an unchanged specification and explicit retry policy.
cakesplitter batch resume .\run.json --retry-failed --format json

# Inspect a redacted persisted summary.
cakesplitter batch status .\run.json --format json --receipt .\run-summary.md --receipt-format markdown
```

Exit code `0` means the batch completed successfully. `11` indicates a batch
failure or completed-with-failures verdict, `9` indicates unsafe recovery, and
`130` indicates cancellation. Existing top-level command exit codes remain
unchanged.
