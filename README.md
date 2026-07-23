# CakeSplitter

CakeSplitter is a local-first tool for splitting one file into verified Slices
and rebuilding the original byte-for-byte. `v0.7.0` is a validated local
publication candidate: it preserves the v0.6 local CLI, Batch, Windows Desktop,
and SplitTheCake Web workflows while hardening public metadata and
machine-readable contracts. Cake Package Manifest 1.0 compatibility remains
unchanged.

This is an early technical release, not a backup system or an authenticated
archival format. Keep an independent copy of important data.

## What the current development line includes

### CakeSplitter Desktop

- native Split, Merge, Inspect, and Verify workflows on Windows 10/11 x64;
- streamed Rust processing with bounded memory and SHA-256 verification;
- one active native worker, a bounded queue, pause, resume, cancellation, and
  structured failures;
- durable SQLite task state and restart recovery at verified Slice boundaries;
- source, selection, destination, and durable package identity binding;
- bounded package enumeration and startup recovery;
- disk-space checks and atomic no-replace final publication;
- explicit active-task close handling and a fenced Clear All operation; and
- a current-user NSIS installer workflow. Installer artifacts remain private
  preview artifacts and are not publicly distributed.

### SplitTheCake Web

- local Split, Merge, Inspect, Tasks, Compatibility Mode, and PWA workflows;
- a canonical offline application shell with no selected-content caching;
- runtime-validated Worker messages and bounded browser operations; and
- bidirectional Cake Package 1.0 interoperability with Rust and Desktop.

### Local CLI automation

- non-interactive Split, Merge, Inspect, Verify, and read-only planning;
- bounded Batch Job schema 1 workflows with deterministic dependencies;
- JSON and JSONL machine output with stable exit codes; and
- checksummed, resumable local run state with bounded diagnostics.

Web Direct Folder Mode remains disabled as a fail-closed security decision.
Current browser APIs do not expose the atomic no-replace primitive CakeSplitter
requires for safe publication. Compatibility Download Mode is the only Web
output mode in the current development line. See
[`docs/direct-folder-security.md`](docs/direct-folder-security.md).

## Privacy

Desktop and Web processing are local. CakeSplitter has no account, analytics,
telemetry, upload, remote checksum, crash upload, remote logging, cloud fallback,
background service, or automatic update check.

The Desktop app stores bounded task metadata in
`%LOCALAPPDATA%\io.cakesplitter.desktop\tasks.sqlite3`. Records may contain local
full paths because native restart recovery must reopen the exact selected
objects. They do not contain selected file contents. Clear All removes managed
task records and bounded quarantine diagnostics, but not user outputs; uninstall
intentionally preserves app data.
See [`docs/privacy-model.md`](docs/privacy-model.md). Public GitHub publication
is intentionally deferred until v0.8.0.

The Web app's production Content Security Policy sets `connect-src 'none'`.
Compatibility Split buffers one completed Slice at a time; Compatibility Merge
buffers the rebuilt output. Both remain subject to documented limits.

## Desktop support and distribution status

CakeSplitter Desktop is a Windows x64 local preview. No public installer or
download is available for `v0.7.0`; private maintainers may use the
separately preserved v0.6.0 artifact and its SHA-256 evidence. Any such
installer is unsigned, may trigger a Windows SmartScreen warning, installs
per-user, and must not be represented as a public release.

Detailed install, uninstall, app-data, and source-build instructions are in
[`docs/desktop-installation.md`](docs/desktop-installation.md). Supported and
unsupported platforms are listed in
[`docs/desktop-support.md`](docs/desktop-support.md).

## Run from source

Rust 1.85 or later and Node.js 20.19+ or 22.12+ are required.

```powershell
npm ci
npm run tauri:dev
```

Build the Windows installer with:

```powershell
npm --workspace @cakesplitter/desktop run tauri:build -- --bundles nsis
```

## CLI

The `0.7.0` CLI is a non-interactive local automation
interface. Human output is the default; `--format json` emits one final JSON
document and `--format jsonl` emits a monotonic event stream.

```powershell
cargo run --locked -p cakesplitter-cli -- split .\large.bin --slice-size 100MiB --output-dir .\package
cargo run --locked -p cakesplitter-cli -- plan split .\large.bin --slice-count 10 --output-dir .\package --format json
cargo run --locked -p cakesplitter-cli -- inspect .\package\large.bin.cake.json --format json
cargo run --locked -p cakesplitter-cli -- verify .\package --receipt .\verify-receipt.json
cargo run --locked -p cakesplitter-cli -- merge .\package --output .\rebuilt.bin --format jsonl
cargo run --locked -p cakesplitter-cli -- batch validate .\examples\batch\verify-package.json --format json
cargo run --locked -p cakesplitter-cli -- batch plan .\examples\batch\verify-package.json --format json
cargo run --locked -p cakesplitter-cli -- batch run .\examples\batch\verify-package.json --state .\verify-run.json --format jsonl
```

Size units accept bytes and the unambiguous binary units `KiB`, `MiB`, and
`GiB`. Ambiguous decimal labels such as `KB` and `MB` are rejected. `plan` and
`--dry-run` perform no output mutation. Native execution revalidates source,
package, destination, and staged-output identity, then uses an operating-system
no-replace operation. Existing outputs are never replaced.

Established exit codes remain `2` for usage/invalid Manifest, `3` for package
integrity, `4` for output conflict, and `130` for cancellation. Additional
stable source, destination, permission, storage, recovery, and capacity codes
are documented in [`docs/error-codes.md`](docs/error-codes.md).

The complete implemented contract and schemas are documented in
[`docs/cli-contract-v0.6.md`](docs/cli-contract-v0.6.md) and
[`docs/cli-json-schema.md`](docs/cli-json-schema.md).

Batch workflows are bounded, local-only, and sequential by default. Use
`batch validate`, `batch plan`, `batch run`, `batch resume`, and `batch status`
with the versioned Job specification documented in
[`docs/batch-job-spec-v0.6.md`](docs/batch-job-spec-v0.6.md). Batch execution
does not run shells, expand environment variables, discover globs, or upload
selected files.

## Web App

```powershell
npm ci
npm run dev
```

Compatibility Mode limits Split and Merge to 256 MiB per operation, Split to
1,000 downloads, and selected packages to 10,000 physical entries. Browser
memory, download behavior, and platform limits may impose lower practical
limits. CakeSplitter does not claim unlimited browser capacity. See
[`docs/browser-support.md`](docs/browser-support.md).

## Format and limits

Application development version `0.7.0` and Cake Package format version
`1.0` are separate. The format did not change in this checkpoint. See
[`specs/cake-package-format.md`](specs/cake-package-format.md).

Key portable-format limits are:

- 16 MiB UTF-8 manifest;
- JSON nesting depth 16;
- 50,000 Slices;
- 200 UTF-8 bytes per portable filename; and
- exact integers no larger than `9,007,199,254,740,991`.

Native Desktop additionally limits nonterminal tasks to 64, retained terminal
history to 500, checkpoint history to 500, and task metadata to 32 MiB per
checksummed record. Browser Compatibility Mode buffers a Slice or rebuilt
output in memory and is limited to 256 MiB and 1,000 downloads. See
[`docs/task-queue.md`](docs/task-queue.md) and
[`docs/task-history.md`](docs/task-history.md).

## Repository layout

```text
apps/desktop/                    React/Tauri Windows desktop application
apps/web/                        SplitTheCake React/Vite app and Web Worker
crates/cakesplitter-format/      Manifest types and strict validation
crates/cakesplitter-integrity/   Incremental SHA-256
crates/cakesplitter-core/        Streaming native operations
crates/cakesplitter-cli/         Command-line interface
crates/cakesplitter-desktop-runtime/  Native queue, persistence, and recovery
packages/                        Shared types, UI, and browser file-I/O policy
specs/                           Cake Package format and JSON Schema
tests/                           Compatibility and production browser tests
docs/                            Architecture, security, support, and reports
```

## Support matrix

| Surface | Status |
| --- | --- |
| SplitTheCake Web | local browser processing |
| CakeSplitter Desktop | Windows x64 local preview |
| CakeSplitter CLI | local automation |
| Batch workflows | local sequential automation |
| macOS/Linux | not currently supported |
| Web Direct Folder | intentionally disabled |
| GitHub publication | deferred until v0.8 |
| Cake Package | format 1.0 |

No public downloads currently exist. `v0.7.0` is not a public release.

## Validation

The executed candidate matrix covers Rust formatting, strict Clippy, native
queue and recovery tests, Node tests, production Microsoft Edge tests,
compatibility in both directions, Web/Desktop production builds, dependency
audits, and repository history/public-surface review. Installer lifecycle,
physical 1 GiB profiling, and fresh-clone reproduction remain v0.8 gates.

The current release reports are in
[`docs/v0.7-contract-alignment.md`](docs/v0.7-contract-alignment.md) and the
[`docs/v0.7.0-test-report.md`](docs/v0.7.0-test-report.md),
[`docs/v0.7.0-security-report.md`](docs/v0.7.0-security-report.md), and the
public-metadata policy report is in
[`docs/public-authorship-policy.md`](docs/public-authorship-policy.md).
Historical release notes are in [`docs/v0.6.0-release-notes.md`](docs/v0.6.0-release-notes.md).
Executed results are in [`docs/v0.6.0-test-report.md`](docs/v0.6.0-test-report.md)
and [`docs/v0.6.0-security-report.md`](docs/v0.6.0-security-report.md). The
bounded automation evidence is in
[`docs/v0.6-automation-validation.md`](docs/v0.6-automation-validation.md),
with the repository audit in
[`docs/v0.6.0-release-audit.md`](docs/v0.6.0-release-audit.md).

## Project policy

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md), and the
[`public security report index`](docs/public-security-report-index.md). CakeSplitter
is licensed under the [MIT License](LICENSE). `v0.7.0` is a private local
publication candidate; public publication is planned for v0.8.0.
