# CakeSplitter

CakeSplitter is a local-first tool for splitting one file into verified Slices
and rebuilding the original byte-for-byte. The repository contains a streaming
Rust CLI and the browser-based **SplitTheCake** workbench.

CakeSplitter v0.2.1 is a hardened source release, not a backup system or an
authenticated archival format. Keep an independent copy of important data.

## What is included

- Cake Package Manifest 1.0 with strict Rust and TypeScript validators;
- streaming native Split, Inspect, Verify, and Merge operations;
- a React/Web Worker browser implementation;
- SHA-256 for every Slice and rebuilt Cake;
- bidirectional Rust-to-Web and Web-to-Rust compatibility tests;
- explicit resource limits and portable filename rules; and
- local-only browser processing with no upload or telemetry path.

Browser direct-folder output is disabled in v0.2.1 because the available File
System Access operations do not provide the exclusive-create, atomic no-replace,
and ownership-safe cleanup guarantees CakeSplitter requires. The browser uses
bounded compatibility downloads instead. See
[`docs/browser-support.md`](docs/browser-support.md). CakeSplitter Desktop is
not yet part of this release.

## Privacy

The production browser build has no account, analytics, telemetry, remote error
reporting, upload endpoint, remote checksum service, or cloud fallback. Its
Content Security Policy sets `connect-src 'none'`. Selected content, filenames,
manifests, hashes, and task metadata remain in the page and its local Worker.

The visible statement “Processed locally in your browser. Your files never
leave your device.” is covered by the production browser privacy smoke. See
[`docs/privacy-model.md`](docs/privacy-model.md) and
[`docs/v0.2-test-report.md`](docs/v0.2-test-report.md).

## Repository layout

```text
apps/web/                         SplitTheCake React/Vite app and Web Worker
crates/cakesplitter-format/       Manifest types and strict validation
crates/cakesplitter-integrity/    Incremental SHA-256
crates/cakesplitter-core/         Streaming native operations
crates/cakesplitter-cli/          Command-line interface
packages/shared-types/            Browser validation, planning, and SHA-256
packages/web-file-io/             Browser stream helpers and capability policy
packages/ui/                      Shared UI primitives
specs/                            Cake Package format and JSON Schema
tests/                            Fixtures, compatibility, and browser smoke
docs/                             Architecture, privacy, support, and reports
```

## CLI

Rust 1.85 or later is required.

```powershell
cargo run -p cakesplitter-cli -- split .\large.bin --slice-size 100MiB --output-dir .\package
cargo run -p cakesplitter-cli -- inspect .\package\large.bin.cake.json
cargo run -p cakesplitter-cli -- verify .\package\large.bin.cake.json
cargo run -p cakesplitter-cli -- merge .\package\large.bin.cake.json --output .\rebuilt.bin
```

Size units accept bytes, decimal `KB`, `MB`, `GB`, and binary `KiB`, `MiB`,
`GiB`. Native finalization uses an operating-system no-replace operation and
revalidates the staged file's identity, size, and SHA-256. Existing outputs are
never replaced.

Exit codes:

- `0`: success;
- `1`: I/O or general processing failure;
- `2`: invalid JSON, manifest, or size input;
- `3`: incomplete, unexpected, corrupted, or hash-mismatched package;
- `4`: output collision; and
- `130`: cancellation.

## Browser app

Node.js 20.19+ or 22.12+ and npm are required by Vite 7.

```powershell
npm install
npm run dev
```

Open the displayed localhost URL and choose Split, Merge, or Inspect. Browser
Split and Merge are limited to 256 MiB per operation. Split produces at most
1,000 downloads, and a package selection contains at most 10,000 files. Merge
buffers the rebuilt Cake in memory; Split buffers one completed Slice for its
download. These are compatibility-mode limits, not claims of unlimited browser
support. Direct Folder Mode remains disabled as a fail-closed security decision;
its restoration requirements are tracked in
[`docs/backlog-direct-folder-mode.md`](docs/backlog-direct-folder-mode.md).

## Format and limits

The format is documented in
[`specs/cake-package-format.md`](specs/cake-package-format.md) and described by
[`specs/cake-manifest.schema.json`](specs/cake-manifest.schema.json). Every
manifest and Slice is untrusted input. Key limits are:

- 16 MiB UTF-8 manifest;
- JSON nesting depth 16;
- 50,000 Slices;
- 200 UTF-8 bytes per portable filename; and
- exact integers no larger than `9,007,199,254,740,991`.

## Validation

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
npm run lint
npm run typecheck
npm test
npm run build
npm run test:compatibility
npm run test:e2e
```

See the executed matrix in [`docs/v0.2-test-report.md`](docs/v0.2-test-report.md)
and the validated security review in
[`docs/v0.2-security-report.md`](docs/v0.2-security-report.md).

## Project policy

See [`CONTRIBUTING.md`](CONTRIBUTING.md), [`SECURITY.md`](SECURITY.md), and the
[`v0.2.1 release notes`](docs/v0.2.1-release-notes.md). CakeSplitter is licensed
under the [MIT License](LICENSE).
