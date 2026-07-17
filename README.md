# CakeSplitter

CakeSplitter is a local-first tool for splitting one file into verified Slices
and rebuilding the original byte-for-byte. Version 0.3.0 contains a streaming
Rust CLI and the browser-based **SplitTheCake** workbench.

This is an early technical source release, not a backup system or an
authenticated archival format. Keep an independent copy of important data.

## What v0.3.0 includes

- Cake Package Manifest 1.0 with strict Rust and TypeScript validators;
- native Split, Inspect, Verify, and Merge with atomic no-replace publication;
- browser Split, Merge, and Inspect in a runtime-validated Web Worker;
- Slice-size and Slice-count planning, drag and drop, bounded progress,
  pause/resume while active, and cancellation;
- browser-local OPFS task metadata, interrupted-task detection, safe restart,
  and a persistence-fenced Clear All operation;
- an installable PWA with a trusted offline application shell;
- SHA-256 for every Slice and every successful rebuild;
- bidirectional Rust/Web compatibility for Manifest 1.0; and
- explicit resource limits and portable filename rules.

Direct Folder Mode is visible but disabled. Current browser File System Access
APIs do not expose the atomic no-replace primitive CakeSplitter requires for
safe final publication. Compatibility Download Mode is therefore the only Web
output mode in v0.3.0. See
[`docs/direct-folder-security.md`](docs/direct-folder-security.md).

CakeSplitter Desktop does not exist in this release.

## Privacy

The production Web App has no account, analytics, telemetry, upload, remote
checksum, remote error reporting, or cloud fallback. Its Content Security
Policy sets `connect-src 'none'`. Selected content, filenames, manifests,
hashes, task metadata, and file-system handles are not transmitted.

The service worker fetches and caches only same-origin application-shell
assets. OPFS stores bounded task metadata for recovery; it does not silently
store selected file contents or become a permanent output destination.

The visible statement “Processed locally in your browser. Your files never
leave your device.” is covered by production Edge network and API
instrumentation. See [`docs/privacy-model.md`](docs/privacy-model.md).

## Repository layout

```text
apps/web/                         SplitTheCake React/Vite app and Web Worker
crates/cakesplitter-format/       Manifest types and strict validation
crates/cakesplitter-integrity/    Incremental SHA-256
crates/cakesplitter-core/         Streaming native operations
crates/cakesplitter-cli/          Command-line interface
packages/shared-types/            Browser validation, planning, and SHA-256
packages/web-file-io/             Browser streaming and output security policy
packages/ui/                      Shared UI primitives
specs/                            Cake Package format and JSON Schema
tests/                            Fixtures, compatibility, and browser tests
docs/                             Architecture, security, support, and reports
```

## CLI

Rust 1.85 or later is required.

```powershell
cargo run --locked -p cakesplitter-cli -- split .\large.bin --slice-size 100MiB --output-dir .\package
cargo run --locked -p cakesplitter-cli -- inspect .\package\large.bin.cake.json
cargo run --locked -p cakesplitter-cli -- verify .\package\large.bin.cake.json
cargo run --locked -p cakesplitter-cli -- merge .\package\large.bin.cake.json --output .\rebuilt.bin
```

Size units accept bytes, decimal `KB`, `MB`, `GB`, and binary `KiB`, `MiB`,
`GiB`. Native finalization uses an operating-system no-replace operation and
revalidates source and staged-output identity, size, and SHA-256. Existing
outputs are never replaced.

Exit codes:

- `0`: success;
- `1`: I/O or general processing failure;
- `2`: invalid JSON, manifest, or size input;
- `3`: incomplete, unexpected, corrupted, or hash-mismatched package;
- `4`: output collision; and
- `130`: cancellation.

## Web App

Node.js 20.19+ or 22.12+ and npm are required by Vite 7.

```powershell
npm ci
npm run dev
```

Open the displayed localhost URL and choose Split, Merge, Inspect, Tasks, or
About. Compatibility Mode limits Split and Merge to 256 MiB per operation,
Split to 1,000 downloads, and a selected package to 10,000 files. Merge buffers
the rebuilt Cake; Split buffers one completed Slice at a time for download.
These are explicit limits, not claims of unlimited browser capacity.

Pause and resume apply only to an active Worker task. After a reload or browser
shutdown, CakeSplitter marks active metadata interrupted and guides a full,
revalidated restart from byte zero; it does not resume partial byte output. See
[`docs/task-recovery.md`](docs/task-recovery.md).

## Format and limits

Application version `0.3.0` and Cake Package format version `1.0` are separate.
The format did not change for this release. See
[`specs/cake-package-format.md`](specs/cake-package-format.md).

Key format limits are:

- 16 MiB UTF-8 manifest;
- JSON nesting depth 16;
- 50,000 Slices;
- 200 UTF-8 bytes per portable filename; and
- exact integers no larger than `9,007,199,254,740,991`.

## Validation

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --workspace --release
npm run lint
npm run typecheck
npm test
npm run test:compatibility
npm run build
npm run test:e2e
npm audit
npm audit --omit=dev
cargo audit
```

Executed results are in [`docs/v0.3-test-report.md`](docs/v0.3-test-report.md)
and [`docs/v0.3-security-report.md`](docs/v0.3-security-report.md).

## Project policy

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md), and the
[`v0.3.0 release notes`](docs/v0.3.0-release-notes.md). CakeSplitter is licensed
under the [MIT License](LICENSE).
