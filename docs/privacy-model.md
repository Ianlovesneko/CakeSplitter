# Browser Privacy Model

## Promise

The v0.3.0 production Web App processes selected content and Cake Package
metadata locally. It has no upload, account, analytics, telemetry, remote error
reporting, remote checksum, or cloud fallback.

The visible statement is:

> Processed locally in your browser. Your files never leave your device.

## Data flow

1. The user selects a Cake, manifest, or Slices.
2. The page structured-clones selected `File` objects and bounded commands to a
   same-origin module Web Worker.
3. The Worker validates every message at runtime, reads Blob chunks, calculates
   SHA-256, and returns progress, bounded downloads, or inspection evidence.
4. The page validates every Worker response before rendering or downloading it.
5. Bounded task metadata may be written to OPFS so interrupted work is visible
   after reload. Selected file contents and file-system handles are not stored.

Direct Folder handles are not requested while the security gate is disabled.
Compatibility Split buffers one completed Slice for download. Compatibility
Merge buffers the rebuilt Cake, subject to the 256 MiB limit. Inspect writes no
output.

## Network boundary

Application code has no upload or reporting endpoint and does not use
`XMLHttpRequest`, `WebSocket`, `EventSource`, or `sendBeacon`. The service worker
uses `fetch` only for same-origin HTML, scripts, styles, its Worker, manifest,
icon, and other declared static shell assets.

The production Content Security Policy sets `connect-src 'none'`, preventing
application connections even if a future accidental call is added without a
deliberate policy change. Production Edge tests instrument fetch,
XMLHttpRequest, WebSocket, EventSource, and sendBeacon and observe no app data
transmission during processing, persistence, errors, and Clear All.

## Service-worker cache

Cache entries are limited to canonical application-shell and hashed static
asset paths on the current origin. Selected filenames, manifests, Slices,
hashes, task records, and downloads are not cache keys or response bodies.
Offline startup uses only the marked canonical shell.

## Local persistence

The `cakesplitter-tasks` OPFS directory contains at most 200 metadata files of
at most 256 KiB each. Records contain bounded recovery facts and statuses. They
do not contain source bytes, Slice bytes, rebuilt bytes, or persistent
file-system handles.

Clear All removes that controlled directory after fencing active and delayed
persistence. Browser download history and files already saved by the user are
controlled by the browser and operating system and are not deleted.

If OPFS is unavailable or quota-exhausted, the UI reports that recovery
metadata was not persisted. If Clear All cannot prove completion, new tasks are
blocked until reload rather than displaying a false successful cleanup.

## Static hosting boundary

The browser fetches the application from its configured static host. Ordinary
asset requests do not include selected file content, filenames, manifests,
hashes, package IDs, task metadata, handles, or private error details. The
shipped `_headers` policy also sets `no-referrer`, blocks framing and MIME
sniffing, and disables unneeded browser features where the host supports those
headers.

Same-origin hosting compromise is outside controls the already-delivered page
can enforce. Users should obtain CakeSplitter from a trusted host or run the
source locally.

## User responsibility

Do not paste sensitive manifests, task details, or filenames into public issue
reports. SHA-256 is integrity evidence, not package-author authentication.
