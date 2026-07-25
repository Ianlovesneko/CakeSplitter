# Contributing to CakeSplitter

CakeSplitter is local-first. Contributions must preserve these boundaries:

- no selected-content upload, account, analytics, telemetry, remote checksum,
  crash upload, cloud fallback, or project-managed updater; browser-managed PWA
  checks stay same-origin, static-shell-only, and free of selected content;
- no execution of selected files and no shell-based concatenation;
- no trusted paths from manifests or renderer-provided filesystem paths;
- bounded streaming I/O for native large-file processing;
- bounded browser Compatibility Mode;
- atomic no-replace publication and fail-closed identity checks; and
- SHA-256 verification before an output is reported complete.

## Development setup

Install Rust 1.85+, Node.js 20.19+ or 22.12+, npm, the Windows MSVC toolchain,
and Microsoft Edge WebView2. Then run:

```powershell
npm ci
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
npm run lint
npm run typecheck
npm test
npm run test:compatibility
npm run build
npm run build:desktop
npm run test:e2e
```

Build the Windows NSIS preview installer with:

```powershell
npm --workspace @cakesplitter/desktop run tauri:build -- --bundles nsis
```

On a Windows host that interferes with dependency build output in the
repository `target` directory, set `CARGO_TARGET_DIR` to a fresh, user-writable
temporary path for the Cargo command. Do not change global Rust or Node
configuration for this project.

## Change requirements

1. Update both Rust and TypeScript validators when the package contract changes.
2. Update the JSON Schema and format document when applicable.
3. Add boundary tests for parsing, limits, cancellation, collision, corruption,
   runtime messages, storage, identity, recovery, or filenames as applicable.
4. Preserve Rust-to-Web, Web-to-Rust, and Desktop Cake Package 1.0 compatibility
   with exact bytes, size, and SHA-256 comparisons.
5. Keep Web and Desktop privacy claims covered by production runtime checks.
6. Run formatting, strict Clippy, Rust tests/build, lint, typecheck, Node tests,
   compatibility, production Web/Desktop builds, browser tests, npm audit, and
   RustSec audit.
7. Do not weaken native directory authority, object-identity binding, durable
   package binding, or no-replace finalization.
8. Do not enable Web Direct Folder Mode without validated exclusive creation,
   atomic no-replace publication, handle identity, rebinding protection, and
   ownership-safe cleanup on every supported browser.
9. Keep PWA cache changes canonical-shell-only and never cache selected content,
   Slices, manifests, hashes, handles, or task records.
10. Preserve browser Clear All generation barriers and native transactional task
    admission, bounded recovery, and terminal-only history pruning.
11. Do not add a Tauri shell, arbitrary process, updater, HTTP, unrestricted
    filesystem, telemetry, or remote-content capability.

`v0.8.1` changes should remain focused on correctness, security,
compatibility, accessibility, packaging, public metadata, and documentation.
Compression,
encryption, PAR2, plugins, marketplaces, cloud features, AI features, macOS,
Linux, ARM64, and arbitrary-byte resume are outside this release line.

For suspected vulnerabilities, follow [`SECURITY.md`](SECURITY.md) instead of
opening a public issue.

## Contribution conduct and publication boundary

Contributors must follow [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md), be
technically specific, and be careful
not to disclose private files, paths, identities, scan workspaces, or raw proof
artifacts. Maintainers may decline contributions that cross the local-only
privacy or scope boundaries above.

Open a focused issue or pull request against `main`. Do not include unpatched
vulnerability details in public contributions; use the private reporting flow
in [`SECURITY.md`](SECURITY.md).
