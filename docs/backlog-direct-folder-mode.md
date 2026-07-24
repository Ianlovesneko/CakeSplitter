# Direct Folder Mode Security Backlog

Status: future, security-gated work. This document does not enable Direct Folder
Mode and does not assign it to a product milestone.

CakeSplitter `v0.8.0` keeps Direct Folder Mode disabled as a fail-closed
security decision. Compatibility downloads remain the only browser output path until a
portable implementation can demonstrate all requirements below.

## Required design work

1. **Handle identity validation**: establish and re-check the identity of every
   directory, staging file, and final file handle across asynchronous browser
   operations. A filename match alone is insufficient.
2. **Replacement and rebinding protection**: detect a handle or directory entry
   that has been replaced, rebound, removed, recreated, or redirected after its
   initial check. Cleanup must never delete an entry whose ownership is
   ambiguous.
3. **Output collision handling**: provide exclusive creation and atomic
   no-replace finalization for Slices, manifests, and rebuilt outputs. A
   destination created by another actor after preflight must be preserved.
4. **Browser-specific filesystem semantics**: validate the behavior of
   `FileSystemDirectoryHandle`, `FileSystemFileHandle`, writable streams,
   `move()`, permission changes, and error reporting in every supported browser
   and operating system. Unsupported or ambiguous behavior must fail closed.
5. **End-to-end security regression tests**: exercise raced-in destinations,
   partial-file replacement, handle rebinding, cancellation, permission loss,
   cleanup ownership, interrupted finalization, and browser/version variance in
   production builds.

## Acceptance gate

Direct Folder Mode may be reconsidered only when the design has a documented
threat model, runtime validation at every Worker boundary, platform support
criteria, and reproducible end-to-end tests proving collision preservation and
ownership-safe cleanup. Capability detection must default to disabled whenever
any required guarantee is unavailable or unverified.
