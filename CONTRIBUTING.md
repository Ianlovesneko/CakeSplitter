# Contributing to CakeSplitter

CakeSplitter is local-first. Contributions must preserve these boundaries:

- no upload, account, analytics, telemetry, remote checksum, or cloud fallback;
- no execution of selected files and no shell-based concatenation;
- no trusted paths from manifests;
- bounded streaming I/O for native large-file processing;
- no overwrite by default; and
- SHA-256 verification before an output is reported complete.

## Development setup

Install Rust 1.85+, Node.js 20.19+ or 22.12+, and npm. Then run:

```powershell
npm install
cargo test --workspace
npm run validate:web
npm run test:compatibility
npm run test:e2e
```

On a Windows host that blocks dependency build output in the repository
`target` directory, set `CARGO_TARGET_DIR` to a user-writable temporary path for
the Cargo command. Do not change global Rust or Node configuration for this.

## Change requirements

1. Update both Rust and TypeScript validators when the package contract changes.
2. Update the JSON Schema and format document when applicable.
3. Add boundary tests for parsing, limits, cancellation, collision, corruption,
   Worker messages, or filenames as applicable.
4. Preserve Rust-to-Web and Web-to-Rust compatibility and compare SHA-256.
5. Keep browser privacy claims covered by production network tests.
6. Run formatting, lint, typecheck, unit, compatibility, production build, and
   browser smoke checks before requesting review.
7. Do not weaken native no-replace finalization or re-enable direct browser
   folder output without a validated exclusive-create and ownership-safe design.

Keep v0.2.x changes focused on correctness, security, compatibility, and
documentation. Queueing, resume, compression, encryption, PAR2, plugins,
marketplaces, cloud features, AI features, and new UX concepts are out of scope.

For suspected vulnerabilities, follow [`SECURITY.md`](SECURITY.md) instead of
opening a public issue.
