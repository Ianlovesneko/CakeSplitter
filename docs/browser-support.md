# Browser Support

CakeSplitter v0.4.0 targets current desktop Microsoft Edge and Chromium with
Web Workers, Blob streams, downloads, OPFS, service workers, `File`, and Web
Crypto random UUID support. The release matrix was executed on Windows 11 with
Microsoft Edge 150.0.4078.83.

Other current desktop Chromium-derived browsers may work but are not certified
by this release matrix. Safari and Firefox do not receive a compatibility claim
beyond standards their current versions independently implement.

## Capability matrix

| Operation | Browser behavior | Boundary |
|---|---|---|
| Split | Worker streams the selected File, buffers one completed Slice, downloads Slices then manifest | Cake at most 256 MiB; at most 1,000 downloads |
| Inspect | Streams and hashes selected Slices; writes no output | At most 10,000 selected files |
| Merge | Verifies Slices in manifest order, buffers the rebuilt Blob, downloads after final SHA-256 | Rebuilt Cake at most 256 MiB; at most 10,000 selected files |
| Tasks | Stores bounded recovery metadata in OPFS | At most 200 records; 256 KiB per record |
| PWA | Caches canonical shell and declared same-origin static assets | No selected or task data in Cache Storage |
| Direct Folder | Disabled by the atomic no-replace security gate | No direct writes or cleanup attempted |

Cake Package Manifest 1.0 supports up to 50,000 Slices. Browser limits are
intentionally lower and do not change the portable format.

## Compatibility Download Mode

Compatibility Split buffers one completed Slice at a time for browser download.
Compatibility Merge buffers the entire rebuilt output Blob. Operations over
256 MiB are rejected before processing. Plans over 1,000 downloads and package
selections over 10,000 files are also rejected before unsafe allocation.

Memory use, browser download prompts, automatic-download policies, disk space,
Blob implementation limits, and platform behavior may impose lower practical
limits. CakeSplitter does not claim unlimited browser capacity.

## Direct Folder Mode

The browser may expose directory selection, writable streams, handle identity,
and file move. It does not expose a portable single operation that publishes a
staged file only if the final name is absent. A preflight existence check plus
an overwriting move has a race and is not accepted.

For that reason:

- the Direct Folder radio is disabled with the missing capability shown;
- no directory picker is invoked;
- the Worker rejects direct-mode requests; and
- no production adapter performs partial creation, move, cleanup, or output
  replacement.

The security contract and future requirements are documented in
[`direct-folder-security.md`](direct-folder-security.md) and
[`backlog-direct-folder-mode.md`](backlog-direct-folder-mode.md).

## Task recovery

Pause/resume works only while the active Worker remains alive. A reload marks
active metadata interrupted. Recovery requires reselection and starts a new
fully verified task from byte zero; partial byte output is not resumed. See
[`task-recovery.md`](task-recovery.md).

## PWA and offline

After one successful online load, a supporting browser can install the app and
start from its cached static shell offline. File processing remains local.
Service-worker updates require an explicit message and are not activated during
active processing. See [`pwa-offline.md`](pwa-offline.md).

## Privacy and hosting

Files are processed locally and are not uploaded. The host still serves static
application assets. The production CSP sets `connect-src 'none'`, and the
service worker does not cache selected content or task state. CakeSplitter
Desktop is a separate Windows x64 runtime; it does not make the browser's
Direct Folder capability available.
