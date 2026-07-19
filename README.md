# CakeSplitter

CakeSplitter is a local-first tool for splitting one file into verified Slices
and rebuilding the original byte-for-byte. Version 0.4.0 adds an early native
Windows desktop application while preserving the browser-based SplitTheCake
workbench and Cake Package Manifest 1.0 compatibility.

This is an early technical release, not a backup system or an authenticated
archival format. Keep an independent copy of important data.

## What v0.4.0 includes

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
- a current-user NSIS installer. The v0.4.0 installer is an unsigned preview.

### SplitTheCake Web

- local Split, Merge, Inspect, Tasks, Compatibility Mode, and PWA workflows;
- a canonical offline application shell with no selected-content caching;
- runtime-validated Worker messages and bounded browser operations; and
- bidirectional Cake Package 1.0 interoperability with Rust and Desktop.

Web Direct Folder Mode remains disabled as a fail-closed security decision.
Current browser APIs do not expose the atomic no-replace primitive CakeSplitter
requires for safe publication. Compatibility Download Mode is the only Web
output mode in v0.4.0. See
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
See [`docs/privacy-model.md`](docs/privacy-model.md).

The Web app's production Content Security Policy sets `connect-src 'none'`.
Compatibility Split buffers one completed Slice at a time; Compatibility Merge
buffers the rebuilt output. Both remain subject to documented limits.

## Install CakeSplitter Desktop

The validated v0.4.0 artifact is an unsigned Windows x64 NSIS installer. Windows
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

```powershell
cargo run --locked -p cakesplitter-cli -- split .\large.bin --slice-size 100MiB --output-dir .\package
cargo run --locked -p cakesplitter-cli -- inspect .\package\large.bin.cake.json
cargo run --locked -p cakesplitter-cli -- verify .\package\large.bin.cake.json
cargo run --locked -p cakesplitter-cli -- merge .\package\large.bin.cake.json --output .\rebuilt.bin
```

Size units accept bytes, decimal `KB`, `MB`, `GB`, and binary `KiB`, `MiB`,
`GiB`. Native finalization revalidates source and staged-output identity, size,
and SHA-256, then uses an operating-system no-replace operation. Existing
outputs are never replaced.

Exit codes:

- `0`: success;
- `1`: I/O or general processing failure;
- `2`: invalid JSON, manifest, or size input;
- `3`: incomplete, unexpected, corrupted, or hash-mismatched package;
- `4`: output collision; and
- `130`: cancellation.

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

Application version `0.4.0` and Cake Package format version `1.0` are separate.
The format did not change in this release. See
[`specs/cake-package-format.md`](specs/cake-package-format.md).

Key portable-format limits are:

- 16 MiB UTF-8 manifest;
- JSON nesting depth 16;
- 50,000 Slices;
- 200 UTF-8 bytes per portable filename; and
- exact integers no larger than `9,007,199,254,740,991`.

Native Desktop additionally limits nonterminal tasks to 64, retained terminal
history to 500, and task metadata to 32 MiB per checksummed record. See
[`docs/v0.4-native-security-limits.md`](docs/v0.4-native-security-limits.md).

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

The full release matrix covers Rust formatting, strict Clippy, 99 Rust tests,
96 Node tests, 12 production Microsoft Edge tests, compatibility in both
directions, real packaged Desktop workflows, installer lifecycle, a physical
1 GiB streamed profile, dependency audits, and fresh-clone reproduction.

Executed results are in [`docs/v0.4-test-report.md`](docs/v0.4-test-report.md)
and [`docs/v0.4-security-report.md`](docs/v0.4-security-report.md).

## Project policy

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md), and the
[`v0.4.0 release notes`](docs/v0.4.0-release-notes.md). CakeSplitter is licensed
under the [MIT License](LICENSE).
