# Task Recovery and Local Storage

CakeSplitter v0.3.0 stores bounded task metadata in the browser's Origin
Private File System (OPFS). The purpose is to explain interrupted work and guide
a safe restart. It is not a queue or a partial-byte resume system.

## Stored data

The `cakesplitter-tasks` directory contains at most 200 JSON records. Each
record is limited to 256 KiB and is validated before use. A record may contain:

- task, package, and operation identifiers;
- expected portable filename, byte size, Slice count, and SHA-256 when known;
- completed Slice indexes reported by the active Worker;
- task status and timestamps;
- output mode and browser capability reason; and
- which inputs must be reselected.

Records never contain source bytes, Slice bytes, rebuilt bytes, persistent
file-system handles, passwords, tokens, or remote identifiers.

## Active pause and resume

Pause and resume operate at bounded Worker checkpoints while the current page
and Worker remain alive. Pausing does not publish incomplete output as success.
Cancellation wakes a paused task, terminates it as cancelled, and keeps its
result incomplete.

## Interrupted work

On startup, tasks recorded as running or paused become interrupted. The user
can choose “Reselect and restart safely.” This does not continue from a byte
offset. It starts a new task ID and reprocesses the selected inputs from byte
zero.

For Split, the reselected source must match the recorded portable filename and
size. A same-name, same-size replacement may be selected, but it is processed
and hashed from byte zero; no prior output is reused. For Merge or Inspect, the
manifest must match the original filename, size, and package ID when recorded,
and every selected Slice is validated again.

Because Direct Folder Mode is disabled, v0.3.0 does not persist output
directory handles or reusable direct-output partials.

## Clear All

Clear All is a barrier, not a cosmetic list reset:

1. synchronously advances the task-store generation;
2. invalidates old and in-flight saves;
3. sends cancellation to an active Worker;
4. waits for terminal Worker acknowledgement and queued persistence;
5. removes the controlled OPFS directory; and
6. rejects stale messages or writes after completion.

Concurrent Clear All calls share one operation. The UI reports completion only
after every barrier succeeds. Browser downloads and files the user already
saved are outside OPFS and are not deleted.

If cleanup fails, the app blocks new tasks until reload rather than claiming
that local data was cleared. Production Edge tests cover active Split and
Merge, delayed persistence, stale messages, concurrent store cleanup, reload,
empty OPFS, and new-generation persistence.

## Failure and quota behavior

If OPFS is unavailable, corrupted, or quota-exhausted, the error is displayed.
A task may still complete in Compatibility Mode, but recovery metadata is not
claimed as stored. Oversized, malformed, or excessive records are rejected.
No automatic loop accumulates unlimited temporary output data.
