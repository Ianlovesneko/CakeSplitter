# Browser Support

CakeSplitter v0.2.1 targets current desktop browsers with Web Workers, Blob
streams, downloads, `File`, and Web Crypto's random UUID support. The production
smoke is run with desktop Chromium on Windows.

Direct Folder Mode is disabled in v0.2.1 as a fail-closed security decision.
Compatibility mode may buffer a completed Slice or the rebuilt output in memory,
so very large operations remain constrained by available memory, browser
download behavior, and platform limits. Files are processed locally and are not
uploaded. CakeSplitter Desktop is not yet part of this release.

## v0.2.1 capability matrix

| Operation | Browser behavior | Memory or count boundary |
|---|---|---|
| Split | Reads the selected File as a stream; buffers one completed Slice into a Blob; downloads Slices, then manifest | Cake at most 256 MiB; at most 1,000 downloads |
| Inspect | Streams and hashes selected Slices; writes no output | At most 10,000 selected files |
| Merge | Streams selected Slices into an in-memory output Blob, then downloads it | Rebuilt Cake at most 256 MiB; at most 10,000 selected files |
| Direct folder | Disabled | No direct writes or cleanup are attempted |

The Cake Package format itself supports up to 50,000 Slices. Browser selection
is intentionally lower, and browser Split is lower again because many automatic
downloads are unreliable and disruptive.

## Why direct folder mode is disabled

The browser File System Access surface reviewed for v0.2.1 does not give this
implementation a portable contract for all three required properties:

1. exclusive creation of every `.partial` entry;
2. atomic final publication that never replaces a raced-in destination; and
3. cleanup that can prove it is deleting only the entry CakeSplitter created.

Capability detection therefore returns false even in Chromium browsers that
expose `showDirectoryPicker` and file-handle `move()`. This is a deliberate
fail-closed security decision, not a browser-detection failure.

The security requirements for reconsidering this mode are tracked in the
[Direct Folder Mode restoration backlog](backlog-direct-folder-mode.md).

## Tested and expected environments

- Desktop Chromium on Windows: automated Split, failed and successful Inspect,
  corrupted Merge refusal, duplicate/unexpected detection, exact Merge,
  downloads, and privacy assertions.
- Current Firefox and Safari: expected to use the same compatibility path when
  their Worker, File, Blob-stream, and download implementations are available;
  they are not certified by the v0.2.1 automated matrix.
- Mobile layouts are responsive, but large-file processing is not certified on
  mobile devices.

Browsers may prompt for or throttle multiple downloads. Download history,
collision naming, and destination selection are controlled by the browser and
operating system. CakeSplitter does not claim unlimited browser file sizes.
