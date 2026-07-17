# CakeSplitter v0.3.0

CakeSplitter v0.3.0 is an early technical source release for splitting one
local file into verified `.slice` files and a `.cake.json` manifest, inspecting
package health, and rebuilding the original byte-for-byte.

The Rust CLI and SplitTheCake Web App share Cake Package Manifest 1.0 and
interoperate in both directions. Every successful rebuild matches exact bytes,
size, and SHA-256.

## Local-only Web App

Files are processed in the browser page and its same-origin Web Worker. There
is no upload, account, analytics, telemetry, remote checksum, remote error
reporting, or cloud fallback. The PWA service worker caches only the trusted
static application shell. Bounded task metadata may be stored in OPFS for
interrupted-task guidance; selected file contents are not stored there.

## v0.3 highlights

- Slice-size and Slice-count planning plus drag/drop;
- bounded Worker progress, active pause/resume, and safe cancellation;
- healthy, missing, modified, duplicate, unexpected, and unsafe package
  inspection;
- OPFS task metadata, interrupted-state detection, safe restart, and Clear All;
- installable PWA with trusted offline startup; and
- hardened source stability, service-worker shell identity, persistence
  barriers, and terminal-safe Unicode.

## Validation

- Rust formatting and strict Clippy passed;
- 34 Rust tests and the optimized workspace build passed;
- lint, typecheck, 87 Node/Web tests, compatibility, and production build
  passed;
- 12 production Edge tests passed;
- npm audits reported zero vulnerabilities; and
- RustSec scanned 83 locked crates with zero advisories.

## Browser limitations

Direct Folder Mode remains disabled because current browser APIs do not provide
portable atomic no-replace finalization. Compatibility Mode is capped at 256
MiB, 1,000 Split downloads, and 10,000 selected files. Merge buffers the rebuilt
Blob, and browser/OS download and memory limits may be lower.

Pause/resume applies only while the Worker remains alive. Interrupted work is
reselected and restarted from byte zero, not resumed from partial bytes.

## Install locally

Download and extract the v0.3.0 source archive, or clone this repository from
its GitHub page. From the repository root, run:

```powershell
npm ci
npm run dev
```

CLI help:

```powershell
cargo run --locked -p cakesplitter-cli -- --help
```

CakeSplitter Desktop, plugins, marketplace, compression, encryption, cloud
integration, accounts, and persistent byte resume are not part of this release.
Cake Package is a project format, not an industry standard or authenticated
backup format.

This release should be marked as a pre-release because the project is an early
technical prototype. Publish it as source only; do not attach unverified
binaries.
