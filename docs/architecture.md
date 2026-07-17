# Architecture

CakeSplitter v0.3.0 is a monorepo with compatible native and browser runtimes
and no server-side processing layer.

```text
local Cake
   ├─ Rust CLI ──> format + integrity + core ──> Cake Package on disk
   └─ Browser UI ──> validated Web Worker ─────> bounded downloads
          │                    │
          │                    └─ incremental SHA-256 + Manifest 1.0
          ├─ OPFS bounded task metadata (no selected content)
          └─ service worker static application shell (no user data)
```

Cake Package Manifest 1.0 is the compatibility boundary. The application
version is independent from the format version.

## Native runtime

- `cakesplitter-format` owns manifest types, portable names, resource limits,
  and strict semantics.
- `cakesplitter-integrity` owns incremental SHA-256.
- `cakesplitter-core` owns fixed-buffer Split, Inspect, Verify, Merge,
  cancellation, stable-source enforcement, and structured errors.
- `cakesplitter-cli` maps commands and exit codes without invoking a shell and
  visibly escapes terminal-control and bidirectional-control characters.

The core uses a 1 MiB buffer. Each output begins as an exclusive `.partial`, is
flushed and synchronized, and is recorded with filesystem identity, expected
size, and SHA-256. Immediately before publication, the core reopens and
revalidates those facts. Publication uses an OS atomic no-replace primitive on
Windows, Linux/Android, and Apple platforms, followed by final identity and
content verification. Unsupported platforms fail closed.

Split keeps one source handle, records identity and metadata, compares a full
preflight SHA-256 with the streaming SHA-256, detects path rebinding, and
revalidates before and after publication. Instability returns `source_changed`
and never leaves an apparently complete package.

Split publishes Slices before the manifest; the manifest is the completion
marker. A late collision may leave verified but incomplete outputs without a
manifest. They are not removed by name because ownership cannot be proven after
a race.

## Browser runtime

React owns selection, disclosure, task state, and result presentation. Every
Worker request and response crosses runtime validators; TypeScript types are
not the trust boundary. The Worker reads Blob slices in bounded chunks and uses
incremental SHA-256.

Compatibility Split buffers one completed Slice for each download.
Compatibility Merge buffers the rebuilt Blob. The UI and Worker independently
enforce 256 MiB, 1,000-download, and 10,000-selected-file limits before unsafe
allocation or processing.

Pause and resume coordinate only an active Worker at bounded chunk boundaries.
Cancellation remains incomplete and cannot be converted into success.

## Task storage and recovery

OPFS stores at most 200 JSON metadata records, each at most 256 KiB. Records
contain identifiers, expected names/sizes/hashes, progress indexes, status, and
reselection requirements—not selected file bytes or directory handles.

Startup converts active or paused records to interrupted. Recovery means the
user reselects required inputs and starts from byte zero under a new task ID;
there is no partial-byte resume journal. Clear All advances a generation,
cancels and waits for the active Worker, drains persistence, removes controlled
OPFS state, and rejects stale messages or writes.

## PWA and offline shell

The service worker caches the canonical marked `/index.html` plus declared
same-origin static assets. Alternate navigation, redirects, unmarked HTML,
opaque responses, query variants, and user/task data cannot become the shell.
Updates activate only through an exact message and are blocked while file
processing is active.

## Direct Folder boundary

The browser-facing mode is disabled. `packages/web-file-io` defines and tests a
future `SecureOutputAdapter` contract requiring exclusive partial creation,
directory and file identity, size/hash verification, and one atomic
`publishNoReplace` operation. Current File System Access APIs do not expose the
last property, so no production adapter is registered and direct requests fail
closed.

## Compatibility contract

Both runtimes implement the same identifier/version, camelCase fields, exact
integer range, 50,000-Slice maximum, 200-byte portable filename rule, Windows
reserved-name rejection, contiguous ranges, lowercase SHA-256, and zero-padded
naming. The compatibility harness rebuilds boundary cases in both directions
and compares exact size, bytes, and SHA-256.

## Deliberately absent

There is no queue, persistent byte resume, compression, encryption, PAR2,
plugin system, marketplace, desktop shell, account, cloud service, telemetry,
digital signature, or AI feature.
