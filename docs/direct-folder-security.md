# Direct Folder Security

## Current `v0.7.0-dev` decision

Direct Folder Mode is disabled in CakeSplitter `v0.7.0-dev`. This is a fail-closed
security decision, not a missing UI toggle.

Current Chromium File System Access APIs expose directory selection, writable
streams, entry identity, and file move. They do not expose a portable atomic
operation equivalent to “publish this staged file only if the final name does
not exist.” A separate existence check followed by a move can overwrite a file
created during the race.

CakeSplitter therefore sets the production capability `atomicNoReplace` to
false, disables the radio control, does not request a directory, and rejects a
direct-mode Worker request.

## Required publication contract

The dormant `SecureOutputAdapter` interface documents the minimum future
contract:

1. identify and later revalidate the selected directory;
2. reject a destination that already exists;
3. create a task-owned partial entry exclusively;
4. stream bounded chunks while hashing;
5. close the stream successfully;
6. revalidate partial identity, size, and SHA-256;
7. atomically publish without replacing a raced-in destination; and
8. reopen and revalidate final identity, size, and SHA-256.

Cancellation and write failure leave the incomplete entry identified as
incomplete. Cleanup may remove an entry only when ownership and identity are
still proven; name-only cleanup is insufficient.

## Current validation

Ten Web file-I/O tests cover:

- capability failure when atomic no-replace is unavailable;
- bounded streaming;
- duplicate and reserved output names;
- destination collision before and during processing;
- directory and partial rebinding;
- failed close and permission loss;
- size and checksum mismatch;
- cancellation without publication;
- final identity validation; and
- rejection of an adapter without the atomic primitive.

These tests validate the security contract but do not claim that a production
browser adapter exists. Compatibility Download Mode remains the only available
browser output path. Native Desktop uses Windows filesystem primitives and does
not weaken this browser decision.

## Future enablement gate

Reconsideration requires a browser/platform primitive with documented atomic
no-replace semantics, handle identity and rebinding tests, permission-revocation
tests, collision tests, failed-stream-close tests, ownership-safe cleanup, and
end-to-end regression tests on every supported browser. See
[`backlog-direct-folder-mode.md`](backlog-direct-folder-mode.md).
