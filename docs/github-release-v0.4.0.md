# CakeSplitter v0.4.0

CakeSplitter v0.4.0 is an early preview of local-only file splitting,
verification, and exact rebuilding. This release adds the first native Windows
x64 CakeSplitter Desktop while preserving the SplitTheCake Web app, Rust CLI,
and Cake Package format 1.0 interoperability.

## Highlights

- Split local files into SHA-256-verified `.slice` files plus a `.cake.json`
  manifest.
- Inspect and verify a Cake Package before rebuilding the original bytes.
- Exchange Cake Package 1.0 data between Rust, Desktop, and Web.
- Run native large-file workflows with bounded queue/history, verified
  Slice-boundary pause, and restart recovery.
- Keep processing local. CakeSplitter has no file upload, account, telemetry,
  analytics, remote checksum, update check, or cloud fallback.

## Security

The v0.4 review validated three Medium and four Low findings. All seven are
remediated with production-path tests covering output-directory identity,
source/package/Slice identity, bounded admission and recovery, streaming
integrity, package enumeration, bootstrap behavior, and stale-writer fencing.

No unresolved Critical, High, or actionable Medium issue remains. A final
bounded security diff of the release-finalization patch found no new plausible
candidate. npm audits reported zero vulnerabilities. RustSec reported zero
vulnerability advisories across 463 locked crates and 17 accepted informational
dependency-maintenance warnings.

## Validation

- Rust format and strict Clippy: passed
- Rust tests: 99 passed; explicit 1 GiB streamed profile passed
- Node/Web/desktop tests: 96 passed
- Production Edge browser/privacy tests: 12 passed
- Rust→Web and Web→Rust compatibility: exact bytes and SHA-256 matched
- Web and desktop frontend production builds: passed
- Windows NSIS install, reinstall, uninstall, packaged workflows, recovery,
  resource-limit, privacy, and accessibility checks: passed

## Install the desktop preview

This is an unsigned Windows 10/11 x64 preview. Download the NSIS installer and
`SHA256SUMS.txt`, verify the installer hash, then run it for the current user.
Windows SmartScreen may show an unknown-publisher warning. Do not bypass a hash
mismatch.

Microsoft Edge WebView2 must already be available. No unverified binary is
included. See `docs/desktop-installation.md` in the source archive for complete
installation, uninstall, preserved-data, and source-build guidance.

## Run from source

```powershell
npm ci
npm run dev
cargo run --locked -p cakesplitter-cli -- --help
```

Build the desktop installer with:

```powershell
npm --workspace @cakesplitter/desktop run tauri:build -- --bundles nsis
```

## Browser and platform limitations

- Browser Direct Folder Mode remains disabled as a fail-closed decision.
- Browser Compatibility Mode buffers one completed Split Slice or the rebuilt
  Merge output and is capped at 256 MiB per operation. Browser memory,
  download, quota, and platform limits may be lower.
- Native support is Windows x64 on local filesystems with stable identity.
  Network, removable, synchronized, virtualized, or identity-poor filesystems
  may fail closed.
- Restart recovery resumes at committed Slice boundaries, not arbitrary bytes.
- The installer is unsigned and no macOS, Linux, or ARM64 package is provided.
- Cake Package 1.0 provides SHA-256 integrity but no signature, encryption, or
  publisher authenticity.

## Prototype scope

This release does not include automatic updates, background services, plugins,
a marketplace, compression, encryption, PAR2, cloud integration, accounts, or
AI features. Cake Package is a project format, not an industry standard.

See `docs/v0.4.0-release-notes.md`, `docs/v0.4-security-report.md`, and
`docs/v0.4-test-report.md` in the source archive for complete evidence and
accepted risks.
