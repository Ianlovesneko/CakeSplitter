# Architecture

CakeSplitter v0.2.1 is a monorepo with compatible native and browser runtimes
and no server-side processing layer.

```text
local Cake
   ├─ Rust CLI ──> format + integrity + core ──> Cake Package on disk
   └─ Browser UI ──> validated Web Worker ─────> bounded downloads
                              │
                       shared TS contract

Cake Manifest 1.0 is the compatibility boundary.
```

## Native runtime

- `cakesplitter-format` owns manifest types, portable names, resource limits,
  and strict semantics.
- `cakesplitter-integrity` owns incremental SHA-256.
- `cakesplitter-core` owns fixed-buffer Split, Inspect, Verify, Merge,
  cancellation, and structured errors.
- `cakesplitter-cli` maps commands and exit codes without invoking a shell and
  escapes untrusted control characters in diagnostics.

The core uses a 1 MiB buffer. Each output is created exclusively as `.partial`,
flushed and synchronized, and recorded with its filesystem identity, expected
size, and SHA-256. Immediately before publication, the core reopens and
revalidates those facts. Publication uses an OS atomic no-replace primitive on
Windows, Linux/Android, and Apple platforms, then reopens the final name and
checks identity and content again. Unsupported platforms fail closed.

Split publishes Slices before publishing the manifest; the manifest is the
package completion marker. A late collision can therefore leave verified but
incomplete outputs without a manifest. They are not automatically removed
because name-based cleanup cannot prove ownership after a race.

## Browser runtime

React owns selection, disclosure, progress, and result presentation. All Worker
requests and responses cross runtime validators; TypeScript types are not the
trust boundary. The Worker streams Blob reads and uses incremental SHA-256.

Direct-folder publication is disabled in v0.2.1 after security review. Split
buffers one completed Slice for each download; Merge buffers the complete output
Blob. Explicit size, download, and selected-file limits bound those paths.

## Compatibility contract

Both runtimes implement the same identifier/version, camelCase fields, exact
integer range, 50,000-Slice format maximum, 200-byte portable filename rule,
Windows reserved-name rejection, range coverage, lowercase SHA-256, and
zero-padded naming. The compatibility harness rebuilds in both directions and
compares final SHA-256 values.

## Deliberately absent

There is no queue, resume journal, compression, encryption, PAR2, plugin system,
marketplace, desktop shell, account, cloud service, telemetry, or AI feature.
