# CakeSplitter v0.8.0

This is CakeSplitter's first public **pre-release**. CakeSplitter splits local
files into verified Cake Package 1.0 Slices and rebuilds them byte-for-byte
through interoperable Rust CLI, Windows Desktop, and SplitTheCake Web
workflows.

## What is included

- Split, Merge, Inspect, and Verify on Windows Desktop and the Rust CLI
- local browser Split, Merge, Inspect, Tasks, and offline PWA shell
- CLI human, JSON, and JSONL contracts with stable exit codes
- bounded Batch Validate, Plan, Run, Resume, and Status workflows
- native bounded queueing, pause/resume/cancel, restart recovery, receipts,
  diagnostics, history, and storage management
- exact `.slice` and `.cake.json` interoperability across Rust, Desktop, and Web

## Local-only privacy

Files are processed on your device and are not uploaded. CakeSplitter has no
account, telemetry, analytics, Desktop updater, project-managed Web update
endpoint, remote checksum, or cloud fallback. Supporting browsers may check
the same-origin service worker and static shell during the normal PWA
lifecycle; those requests never contain selected content or task metadata.

## Security hardening

The release preserves strict Manifest and portable-filename validation,
filesystem identity/rebinding protection, bounded task and package resources,
runtime Worker and IPC validation, atomic no-replace publication, terminal
redaction, browser privacy controls, and exact cross-runtime compatibility.
The focused v0.8 publication-diff review found no unresolved Critical, High, or
actionable Medium issue.

## Validation summary

- Rust formatting and strict Clippy passed with zero warnings
- 150 Rust tests passed; the explicit 1 GiB profile remains opt-in and ignored
- 104 Node tests passed
- all 12 Microsoft Edge production/privacy/offline/accessibility tests passed
- Rust-to-Web and Web-to-Rust compatibility passed
- malicious Manifest fixtures were rejected by both runtimes
- Web and Desktop frontend production builds passed
- npm audits reported zero vulnerabilities
- RustSec reported zero vulnerability advisories; accepted transitive
  maintenance/unsoundness warnings are documented in the security report

## Download safety

Windows artifacts are x64-only and **unsigned**. Windows SmartScreen may warn.
Verify downloads with `SHA256SUMS.txt` and `artifact-manifest.json`. The signed
Git tag authenticates the source release; it does not code-sign the Windows
binaries. Installer binaries may not be bit-for-bit reproducible across
toolchain environments.

See the
[desktop installation guide](https://github.com/Ianlovesneko/CakeSplitter/blob/v0.8.0/docs/desktop-installation.md)
and
[release notes](https://github.com/Ianlovesneko/CakeSplitter/blob/v0.8.0/docs/v0.8.0-release-notes.md).

## Known limitations

- Web Direct Folder Mode is disabled.
- Web Compatibility Mode may buffer a Slice or rebuilt output in memory and is
  constrained by browser/platform limits.
- Uninstall may preserve local application data.
- Cake Package 1.0 has SHA-256 integrity but no publisher authenticity,
  digital signatures, encryption, or compression.
- macOS, Linux, ARM64, a Desktop automatic updater, persistent arbitrary-byte
  resume, cloud transfer, plugins, and a marketplace are not available.

CakeSplitter v0.8.0 remains early-stage pre-release software. Keep an
independent copy of important data.
