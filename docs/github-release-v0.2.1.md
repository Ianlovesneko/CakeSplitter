# CakeSplitter v0.2.1

CakeSplitter is a local-first prototype that splits one file into verified
`.slice` files plus a `.cake.json` manifest, inspects and verifies a package, and
rebuilds the original bytes. The Rust CLI and browser implementation use the
same Cake Package Manifest 1.0 contract and interoperate in both directions.

## Security hardening

This release addresses all 12 validated review findings: 5 Medium and 7 Low,
with no unresolved Critical or High issues. Highlights include native atomic
no-replace finalization, staged-file identity and digest checks, strict manifest
and portable-filename validation, bounded resource use, runtime validation of
all Worker messages, escaped CLI diagnostics, and fail-closed browser output
behavior.

Browser processing is local-only. The production application has no account,
upload, analytics, telemetry, remote checksum, or cloud fallback path, and its
Content Security Policy sets `connect-src 'none'`.

## Validation

- Rust formatting and strict Clippy passed with zero warnings.
- 24 Rust tests and 42 Node tests passed.
- Type checking, linting, compatibility tests, and the production build passed.
- Three production Edge browser/privacy tests passed.
- Rust-to-Web and Web-to-Rust rebuilds produced identical bytes and SHA-256.
- npm audit and RustSec audit reported zero known vulnerabilities.
- The release audit found no secrets, generated-file leaks, stale markers,
  broken internal links, or trailing whitespace.

See the [test report](v0.2-test-report.md) and
[security report](v0.2-security-report.md) for the complete evidence.

## Browser limitations

Direct Folder Mode is disabled in v0.2.1 as a fail-closed security decision.
Compatibility Split buffers one completed Slice, while Compatibility Merge
buffers the rebuilt output. Each operation is limited to 256 MiB; browser Split
is limited to 1,000 downloads, and browser selection is limited to 10,000 files.
Very large operations may also be constrained by memory, browser download
behavior, and platform limits. Files remain local and are not uploaded.

CakeSplitter Desktop is not part of this release.

## Accepted risks

SHA-256 proves byte integrity but not publisher identity. Browser downloads can
be throttled or collision-renamed by the browser. Native safety ultimately
depends on the filesystem provider's atomic semantics, and verified but
incomplete Slices can remain after a late native collision. A hash-consistent
package can still contain malicious bytes and must not be treated as safe to
execute.

## Install and run locally

Requirements: Rust 1.85 or later, Node.js 20.19+ or 22.12+, and npm.

```powershell
npm install
npm run dev
```

For the native CLI:

```powershell
cargo run -p cakesplitter-cli -- split .\large.bin --slice-size 100MiB --output-dir .\package
cargo run -p cakesplitter-cli -- verify .\package\large.bin.cake.json
cargo run -p cakesplitter-cli -- merge .\package\large.bin.cake.json --output .\rebuilt.bin
```

CakeSplitter v0.2.1 is a source release and prototype. It is not a backup system
or an authenticated archival format. Keep an independent copy of important
data.
