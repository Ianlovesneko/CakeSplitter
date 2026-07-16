# Browser Privacy Model

## Promise

The v0.2.1 production browser build processes selected content and package
metadata locally. It has no upload, account, analytics, telemetry, remote error
reporting, remote checksum, or cloud fallback.

The visible statement is:

> Processed locally in your browser. Your files never leave your device.

## Data flow

1. The user selects a Cake, manifest, or Slices.
2. The page structured-clones the selected `File` objects and bounded command
   data to a same-origin module Web Worker.
3. The Worker validates the message at runtime, reads Blob streams, calculates
   SHA-256, and returns progress, bounded Blob downloads, or inspection facts.
4. The page validates every Worker response before rendering or downloading it.

Direct-folder handles are not requested or used in v0.2.1. Split buffers one
completed Slice for download. Merge buffers the rebuilt Cake, subject to the
256 MiB compatibility limit. Inspect does not produce output.

No application code calls `fetch`, `XMLHttpRequest`, `WebSocket`,
`navigator.sendBeacon`, or a remote SDK. The production Content Security Policy
sets `connect-src 'none'` so an accidental future connection attempt is blocked
unless the policy is deliberately changed.

## Static hosting boundary

The browser fetches the app's HTML, JavaScript, Worker, CSS, and other static
assets from its host. Those ordinary GET requests do not include selected file
content, filenames, manifests, hashes, or task metadata. The shipped `_headers`
policy also sets `no-referrer`, blocks framing, prevents MIME sniffing, and
disables unneeded browser features where supported by the host.

## Local persistence

The app has no server database, task queue, IndexedDB persistence, OPFS recovery
journal, service worker, or account storage. Browser download history and saved
files are controlled by the browser and operating system. Errors and selected
filenames are rendered only in the local page.

Users should not paste sensitive manifests or filenames into public issue
reports. The production privacy smoke and request assertions are recorded in
[`v0.2-test-report.md`](v0.2-test-report.md).
