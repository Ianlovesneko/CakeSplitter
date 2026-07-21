# Architecture

CakeSplitter v0.6.0-dev is a monorepo with compatible Rust CLI, native Windows
Desktop, and browser runtimes. There is no server-side processing layer.

```text
local Cake
   ├─ Rust CLI ─────> format + integrity + core ──> Cake Package on disk
   ├─ Desktop UI ───> narrow Tauri IPC ───────────> Rust task engine
   │                                                ├─ one worker + bounded queue
   │                                                ├─ SQLite recovery state
   │                                                └─ guarded native filesystem
   └─ Browser UI ──> validated Web Worker ─────────> bounded downloads
          │                    │
          │                    └─ incremental SHA-256 + Manifest 1.0
          ├─ OPFS bounded task metadata (no selected content)
          └─ service worker static application shell (no user data)
```

Cake Package Manifest 1.0 is the compatibility boundary. Application version
0.6.0-dev is independent from format version 1.0. The v0.5 native runtime adds a
serialized bounded task scheduler, checksummed recovery store, and identity-
bound receipt/diagnostic publication without changing the portable format.

## Native core

- `cakesplitter-format` owns manifest types, portable names, resource limits,
  and strict semantics.
- `cakesplitter-integrity` owns incremental SHA-256.
- `cakesplitter-core` owns fixed-buffer Split, Inspect, Verify, Merge,
  resumable checkpoints, cancellation, identity guards, and structured errors.
- `cakesplitter-cli` maps commands and exit codes without invoking a shell and
  visibly escapes terminal-control and bidirectional-control characters. The
  v0.6 checkpoint also owns versioned JSON/JSONL envelopes, read-only planning,
  strict argument parsing, Ctrl+C wiring, and explicit redacted receipts.

The core uses a 1 MiB buffer. Each output begins as an exclusive `.partial`, is
flushed and synchronized, and is recorded with filesystem identity, expected
size, and SHA-256. Immediately before publication, the core reopens and
revalidates those facts. Publication uses an OS atomic no-replace primitive on
validated platforms, followed by final identity and content verification.
Unsupported or identity-poor states fail closed.

Split retains Windows directory authority over the selected output and its
replaceable ancestors. It records source identity, compares a full preflight
SHA-256 with the streamed SHA-256, checks path rebinding, and revalidates before
and after publication. Slices publish before the manifest; the manifest is the
completion marker. A failed or cancelled task may leave owned incomplete data,
but never a final manifest or false completion result.

## Desktop runtime

The React renderer receives opaque, short-lived selection tokens. It never
submits arbitrary filesystem paths to processing commands. Rust validates every
IPC value and reopens the selected object to compare native volume and file
identity, kind, length, timestamps where applicable, directory membership, and
expected package evidence.

`cakesplitter-desktop-runtime` serializes task admission and stores checksummed
JSON records in SQLite. One worker executes at most one disk task; at most 64
nonterminal tasks exist, terminal history is pruned to 500, and startup handles
at most 64 recoverable records plus a bounded 20-record diagnostic sample.
Malformed, future-schema, or ambiguous state is rejected, isolated, or
preserved as local evidence before any file operation.

Pause and resume occur at verified Slice boundaries. An active task interrupted
by application exit is marked recoverable at the last committed boundary.
Restart reopens and revalidates the exact source, destination, package, and
checkpoint identities before continuing. Recovery never silently rebinds a
same-named replacement.

Desktop persistence lives in
`%LOCALAPPDATA%\io.cakesplitter.desktop\tasks.sqlite3`. It may contain local full
paths needed for recovery, but not selected file contents. There is no
background service, updater, HTTP capability, shell capability, unrestricted
filesystem capability, or remote application content.

## Browser runtime

React owns selection, disclosure, task state, and result presentation. Every
Worker request and response crosses runtime validators; TypeScript types are
not the trust boundary. The Worker reads Blob slices in bounded chunks and uses
incremental SHA-256.

Compatibility Split buffers one completed Slice for each download.
Compatibility Merge buffers the rebuilt Blob. The UI and Worker independently
enforce 256 MiB, 1,000-download, and 10,000-selected-file limits before unsafe
allocation or processing.

Browser pause and resume coordinate only the current Worker. After reload,
recovery requires reselection and restarts from byte zero under a new task ID.
OPFS stores at most 200 JSON metadata records of at most 256 KiB. Clear All
advances a generation, cancels and drains active work, removes controlled OPFS
state, and rejects stale messages or writes.

## PWA and Direct Folder boundaries

The service worker caches the canonical marked `/index.html` plus declared
same-origin static assets. Alternate navigation, redirects, unmarked HTML,
opaque responses, query variants, and user/task data cannot become the shell.

Web Direct Folder Mode is disabled. A dormant secure-output contract requires
exclusive partial creation, directory and file identity, full verification,
and one atomic no-replace publish operation. Current browser APIs do not expose
the final guarantee, so no production adapter is registered.

## Compatibility contract

All runtimes implement the same identifier/version, camelCase fields, exact
integer range, 50,000-Slice maximum, 200-byte portable filename rule, Windows
reserved-name rejection, contiguous ranges, lowercase SHA-256, and zero-padded
naming. The compatibility harness rebuilds boundary cases in both directions
and compares exact size, bytes, and SHA-256.

## Deliberately absent

There is no compression, encryption, PAR2, plugin system, marketplace, account,
cloud service, telemetry, digital signature, automatic updater, background
service, macOS build, Linux build, ARM64 build, or arbitrary-byte resume.
