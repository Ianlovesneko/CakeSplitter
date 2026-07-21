# CakeSplitter CLI Contract v0.6

This document describes the implemented `0.6.0-dev` CLI checkpoint. It is a
local-only automation interface over the existing Rust core. It is not the
complete v0.6 batch system and it does not change Cake Package format `1.0`.

## Existing v0.5 inventory

| Behavior | v0.6 classification | Implemented contract |
| --- | --- | --- |
| `split FILE --slice-size SIZE [--output-dir DIR]` | Preserved and extended | Existing syntax remains valid. Target count, planning, dry-run, output modes, progress, cancellation, and receipts are added. |
| `merge MANIFEST --output FILE` | Preserved and extended | A Manifest or package directory is accepted. A complete explicit `--slice` set may be supplied. |
| `inspect MANIFEST` | Preserved and extended | Human mode keeps the v0.5 pretty-JSON inspection shape. Manifest-only and versioned machine modes are added. |
| `verify MANIFEST` | Preserved and extended | Integrity failures retain exit code `3`; structured results and optional receipts are added. |
| `--help`, `--version` | Preserved | Clap-compatible flags remain. Explicit `help` and `version` commands are added. |
| No-overwrite finalization | Preserved | Existing final output, task partial, Slice, Manifest, or receipt collisions fail closed. |
| Progress | Extended (previously undocumented) | Human progress is written to stderr; JSON suppresses it; JSONL emits versioned progress events. |
| Errors | Extended (previously undocumented) | Core codes are retained and mapped to stable categories, exit codes, retryability, and suggested actions. |
| Cancellation | Previously token-capable but not wired to the terminal | Ctrl+C cancels through the core token. Identity-owned incomplete output is cleaned without publishing success. |
| Terminal escaping | Preserved | ANSI controls and Unicode bidirectional controls remain visible escaped text. |
| Machine output | Inconsistent in v0.5; extended | `human`, `json`, and `jsonl` are explicit global modes. |
| Receipts | Previously absent in CLI | Explicit JSON or Markdown receipts reuse the v0.5 centralized redaction rules. |

No v0.5 command is deprecated in this checkpoint. The only inconsistent or
previously undocumented behaviors are the output and diagnostic details shown
above; no behavior is silently changed. Deprecated-command review found no
candidate, and unsupported future batch commands remain intentionally absent.

No command prompts, opens a dialog, launches the Desktop app, invokes a shell,
or waits for hidden input.

## Commands

```text
cakesplitter split FILE (--slice-size SIZE | --slice-count COUNT) [--output-dir DIR] [--dry-run]
cakesplitter merge PACKAGE --output FILE [--slice FILE ...] [--dry-run]
cakesplitter inspect PACKAGE [--slice FILE ...] [--manifest-only]
cakesplitter verify PACKAGE [--slice FILE ...]
cakesplitter plan split FILE (--slice-size SIZE | --slice-count COUNT) [--output-dir DIR] [--receipt FILE]
cakesplitter plan merge PACKAGE --output FILE [--slice FILE ...] [--receipt FILE]
cakesplitter version
cakesplitter help
```

Global options may appear before or after the subcommand:

```text
--format human|json|jsonl
--verbose
```

`PACKAGE` is either a `.cake.json` Manifest or a directory containing exactly
one Manifest. Repeated `--slice` values must be the complete, duplicate-free
Manifest set and must remain in the bound package directory. Same-name files
from another directory are rejected. The explicit set is an identity and
membership assertion; the core still re-enumerates and verifies the bound
package directory before any merge or inspection output is published.

Split accepts positive integer bytes or the explicit units `B`, `KiB`, `MiB`,
and `GiB`. Decimal or ambiguous units such as `KB` and `MB`, fractions, signs,
zero, overflow, and values above the cross-runtime safe integer limit are
rejected. A target count must be between 1 and 50,000, no greater than the
source byte count, and exactly representable by a uniform target Slice size.

## Plan and dry-run

`plan split` and `plan merge` are read-only. `--dry-run` on Split or Merge uses
the same planner and returns the same readiness evidence. Planning reports:

- source size, target Slice size, and expected Slice count;
- expected output names;
- required and available free space;
- warnings and conflicts;
- package integrity state for Merge;
- format compatibility limits; and
- a final `ready` value.

Planning does not create a directory, `.partial`, `.slice`, Manifest, task, or
history record. An explicitly requested `--receipt` is the sole dry-run report
exception; it creates exactly the requested new JSON or Markdown report and
has `status: "dry-run"`. Without `--receipt`, planning and `--dry-run` leave
the filesystem unchanged.

## Output modes

Human is the default. Final human results go to stdout. Concise progress,
warnings, and failures go to stderr. Untrusted terminal and bidirectional
controls are escaped. `--verbose` adds privacy-safe technical error detail.

JSON produces exactly one final document on stdout, including for usage,
processing, integrity, and cancellation failures. JSON stdout never contains
progress, terminal decoration, or debug text. Machine-mode stderr remains
empty for handled CLI results.

JSONL produces one JSON object per stdout line. Each event contains
`schemaVersion`, `event`, `command`, `operationId`, `timestamp`, `sequence`,
and `payload`. Sequence starts at one and increases monotonically for the
operation. Exactly one final `completed`, `failed`, or `cancelled` event is
emitted. `paused` and `resumed` are reserved schema events; this first CLI
checkpoint exposes Ctrl+C cancellation but no interactive pause command.

See [CLI JSON Schema](cli-json-schema.md) for field compatibility rules.

## Exit codes

| Code | Category | Meaning |
| ---: | --- | --- |
| `0` | success | Completed operation or completed read-only plan. A plan may report `ready: false`. |
| `1` | internal | Unexpected bounded CLI failure. |
| `2` | usage / invalid Manifest | Invalid arguments, size, count, JSON, or Manifest. Preserves the v0.5 contract. |
| `3` | package / integrity | Missing, unexpected, corrupted, identity-changed, or final-hash-mismatched package. |
| `4` | conflict | Existing output or receipt. Preserves the v0.5 collision code. |
| `5` | source | Missing, invalid, non-portable, or changed source. |
| `6` | destination | Unsafe, rebound, unsupported, or identity-changed destination. |
| `7` | permission | Local filesystem permission failure. |
| `8` | storage | Local I/O or storage failure. |
| `9` | recovery | Unsafe or invalid recovery state. |
| `10` | capacity | Slice, package enumeration, or free-space capacity limit. |
| `130` | cancellation | Ctrl+C or an equivalent cancellation token. |

Every machine error includes `code`, `category`, `message`,
`technicalMessage`, `retryable`, `suggestedAction`, and `operationId` where an
operation was established. Raw platform errors are redacted before emission.

## Cancellation and partial files

Ctrl+C sets the existing core cancellation token. The core stops at a bounded
read/write checkpoint, closes handles, and never publishes an unverified final
Manifest or rebuilt output. The CLI records native identities for task-owned
partials and completed pre-Manifest Slices. On failure or cancellation it
attempts to remove only those exact identity-owned files. Replacement or
ambiguous objects are not deleted. Cancellation ends with exit code `130` and
a structured `cancelled` terminal result.

## Receipts

Split, Merge, Inspect, and Verify accept:

```text
--receipt FILE --receipt-format json|markdown
```

Receipt creation is explicit and no-replace. `--dry-run --receipt FILE` is
allowed as an explicit dry-run report; that report has `status: "dry-run"` and
is the only file a dry-run may create. Receipts mask path components and
omit usernames, secrets, environment variables, native filesystem identities,
file contents, and Slice contents. A receipt failure is a warning and a
separate `result.receipt` failure; it never changes a successfully verified
file operation into a false processing failure.

## Examples

Human Split:

```powershell
cakesplitter split .\example.bin --slice-size 64MiB --output-dir .\package
```

JSON Inspect:

```powershell
cakesplitter inspect .\package\example.bin.cake.json --format json
```

JSONL Split progress:

```powershell
cakesplitter split .\example.bin --slice-count 4 --output-dir .\package --format jsonl
```

Dry-run planning:

```powershell
cakesplitter plan split .\example.bin --slice-size 64MiB --output-dir .\package --format json
```

Receipt export:

```powershell
cakesplitter verify .\package --receipt .\verify-receipt.json --receipt-format json
```

Cancellation is the normal terminal interrupt:

```text
Ctrl+C
```

The JSON result then has `status: "cancelled"`, error category
`"cancellation"`, and exit code `130`.

## Privacy and non-goals

The CLI performs no network calls, telemetry, analytics, update checks, remote
logging, cloud fallback, plugin loading, process execution, or dynamic code
execution. It does not transmit file contents, names, paths, hashes, Manifests,
Slices, receipts, or task metadata. It is not a daemon and does not implement
the later v0.6 batch-processing goal.
