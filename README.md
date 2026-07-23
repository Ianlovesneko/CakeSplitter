# CakeSplitter

CakeSplitter is a local-first tool for splitting one file into verified Slices
and rebuilding the original byte-for-byte. Version 0.5.0 adds operationally
Windows desktop application while preserving the browser-based SplitTheCake
workbench and Cake Package Manifest 1.0 compatibility.

This is an early technical release, not a backup system or an authenticated
archival format. Keep an independent copy of important data.

## What v0.5.0 includes

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
- a current-user NSIS installer. The v0.5.0 installer is an unsigned private
  preview and is not publicly distributed.

### SplitTheCake Web

- local Split, Merge, Inspect, Tasks, Compatibility Mode, and PWA workflows;
- a canonical offline application shell with no selected-content caching;
- runtime-validated Worker messages and bounded browser operations; and
- bidirectional Cake Package 1.0 interoperability with Rust and Desktop.

Web Direct Folder Mode remains disabled as a fail-closed security decision.
Current browser APIs do not expose the atomic no-replace primitive CakeSplitter
requires for safe publication. Compatibility Download Mode is the only Web
output mode in v0.5.0. See
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

## Install CakeSplitter Desktop

The validated v0.5.0 artifact is an unsigned Windows x64 NSIS installer. Windows
SmartScreen may display an unrecognized-publisher warning. Verify the published
SHA-256 before running it, and install only artifacts obtained from the intended
release source. Installation is per-user and does not require elevation.

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

The `0.6.0-dev` CLI checkpoint is a non-interactive local automation
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

Development application version `0.6.0-dev` and Cake Package format version
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

## Validation

The full release matrix covers Rust formatting, strict Clippy, native queue and
recovery tests, Node tests, production Microsoft Edge tests, compatibility in both
directions, real packaged Desktop workflows, installer lifecycle, a physical
1 GiB streamed profile, dependency audits, and fresh-clone reproduction.

Executed results are in [`docs/v0.5-test-report.md`](docs/v0.5-test-report.md)
and [`docs/v0.5-security-report.md`](docs/v0.5-security-report.md). The
`0.6.0-dev` CLI release-candidate evidence is in
[`docs/v0.6-test-report.md`](docs/v0.6-test-report.md),
[`docs/v0.6-automation-validation.md`](docs/v0.6-automation-validation.md),
and [`docs/v0.6-security-review.md`](docs/v0.6-security-review.md).

## Project policy

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md), and the
[`v0.5.0 release notes`](docs/v0.5.0-release-notes.md). CakeSplitter is licensed
under the [MIT License](LICENSE). v0.5.0 is a local development release; public
publication is planned for v0.8.0.
