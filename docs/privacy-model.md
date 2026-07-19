# Privacy Model

## Promise

CakeSplitter v0.4.0 processes selected content and Cake Package metadata
locally. Desktop and Web have no upload, account, analytics, telemetry, remote
error reporting, crash upload, remote checksum, cloud fallback, remote logging,
background service, or automatic update check.

The Web statement is:

> Processed locally in your browser. Your files never leave your device.

The Desktop header states:

> Local only · no uploads · no automatic updates

## Desktop data flow

1. A native Windows picker returns a short-lived opaque selection token.
2. The renderer submits the token and bounded command values through narrow
   Tauri IPC.
3. Rust reopens and validates the selected object, filesystem identity, task
   plan, disk space, and package evidence.
4. One native worker streams fixed-size buffers, computes SHA-256, and updates a
   checksummed SQLite task record at verified boundaries.
5. Rust revalidates source, destination, package, and staged-output identity
   before security-sensitive publication and publishes without replacement.

The renderer does not receive selected full paths. Native task records may
contain local full paths because restart recovery must reopen the exact source,
destination, manifest, Slices, or rebuilt output. These paths remain in local
app data and are not logged or transmitted. Task records contain no selected
file contents.

Desktop app data is stored at:

```text
%LOCALAPPDATA%\io.cakesplitter.desktop
```

The primary database is `tasks.sqlite3`, with ordinary SQLite WAL/SHM files
while active. A local lock prevents multiple processes from mutating the store.
Unsupported future schemas are preserved as renamed local evidence; bounded
invalid records may be quarantined with redacted reasons.

Clear All advances the store epoch, cancels or drains active work, and removes
managed task/history records and bounded quarantine diagnostics. It does not
delete user-created Slices, manifests, rebuilt outputs, browser downloads, or
unrelated files. The Windows uninstaller removes the executable, shortcut, and
installer registration but intentionally preserves app data. Users who want to
erase it must first close CakeSplitter, then remove the directory above manually.

## Browser data flow

1. The user selects a Cake, manifest, or Slices.
2. The page structured-clones selected `File` objects and bounded commands to a
   same-origin module Web Worker.
3. The Worker validates every message at runtime, reads Blob chunks, calculates
   SHA-256, and returns progress, bounded downloads, or inspection evidence.
4. The page validates every Worker response before rendering or downloading it.
5. Bounded task metadata may be written to OPFS so interrupted work is visible
   after reload. Selected file contents and file-system handles are not stored.

Web Direct Folder handles are not requested. Compatibility Split buffers one
completed Slice for download; Compatibility Merge buffers the rebuilt Cake,
subject to the 256 MiB limit. Inspect writes no output.

The browser's `cakesplitter-tasks` OPFS directory contains at most 200 metadata
records of at most 256 KiB each. Browser Clear All removes that controlled
directory after fencing active and delayed persistence. Browser download history
and files already saved by the user remain under browser and operating-system
control.

## Network and capability boundary

Desktop production capabilities include only core event delivery and the named
CakeSplitter commands. There is no Tauri HTTP, updater, shell, process, or
unrestricted filesystem capability. The bundled WebView loads local static
application assets, and its Content Security Policy permits only Tauri IPC
connect endpoints. A packaged runtime observation found no TCP connection while
Split, Inspect, Verify, Merge, task recovery, and Clear All were exercised.

Web application code has no upload or reporting endpoint and does not use
`XMLHttpRequest`, `WebSocket`, `EventSource`, or `sendBeacon`. Its service worker
uses `fetch` only for same-origin static application-shell assets. The production
Web CSP sets `connect-src 'none'`. Production Edge tests instrument network APIs
and observe no selected data transmission during processing, persistence,
errors, offline startup, and Clear All.

## Service-worker cache

Cache entries are limited to canonical application-shell and hashed static
asset paths on the current origin. Selected filenames, manifests, Slices,
hashes, task records, and downloads are not cache keys or response bodies.
Offline startup uses only the marked canonical shell.

## User responsibility

Do not paste sensitive manifests, task details, filenames, or local paths into
public issue reports. SHA-256 is integrity evidence, not package authenticity.
Obtain CakeSplitter from a trusted release source and verify published artifact
checksums before installing the unsigned preview.
