# Contributing to CakeSplitter

CakeSplitter is local-first. Contributions must preserve these boundaries:

- no upload, account, analytics, telemetry, remote checksum, or cloud fallback;
- no execution of selected files and no shell-based concatenation;
- no trusted paths from manifests;
- bounded streaming I/O for native large-file processing;
- bounded browser Compatibility Mode;
- no overwrite by default; and
- SHA-256 verification before an output is reported complete.

## Development setup

Install Rust 1.85+, Node.js 20.19+ or 22.12+, and npm. Then run:

```powershell
npm ci
cargo test --workspace
npm run validate:web
npm run test:compatibility
npm run test:e2e
```

On a Windows host that interferes with dependency build output in the
repository `target` directory, set `CARGO_TARGET_DIR` to a fresh, user-writable
temporary path for the Cargo command. Do not change global Rust or Node
configuration for this project.

## Change requirements

1. Update both Rust and TypeScript validators when the package contract changes.
2. Update the JSON Schema and format document when applicable.
3. Add boundary tests for parsing, limits, cancellation, collision, corruption,
   runtime messages, storage, or filenames as applicable.
4. Preserve Rust-to-Web and Web-to-Rust compatibility with exact bytes and
   SHA-256 comparisons.
5. Keep browser privacy claims covered by production network tests.
6. Run formatting, strict Clippy, Rust tests/build, lint, typecheck, Node tests,
   compatibility, production build, browser tests, npm audit, and RustSec audit.
7. Do not weaken native no-replace finalization or enable browser Direct Folder
   Mode without a validated exclusive-create, atomic no-replace, identity, and
   ownership-safe cleanup design.
8. Keep PWA cache changes canonical-shell-only and never cache selected content,
   Slices, manifests, hashes, handles, or task records.
9. Preserve the Clear All generation and Worker acknowledgement barriers.

Version 0.3.x changes should remain focused on correctness, security,
compatibility, accessibility, and documentation. Queueing, persistent byte
resume, compression, encryption, PAR2, plugins, marketplaces, cloud features,
AI features, desktop UI, and new UX concepts are outside this release line.

For suspected vulnerabilities, follow [`SECURITY.md`](SECURITY.md) instead of
opening a public issue.
