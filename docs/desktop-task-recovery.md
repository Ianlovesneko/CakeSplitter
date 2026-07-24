# Desktop Task Recovery

CakeSplitter Desktop `v0.8.0` stores checksummed task records in local SQLite so
interrupted native work can continue safely at verified Slice boundaries.
Recovery is not arbitrary-byte resume.

## Storage and bounds

The database is:

```text
%LOCALAPPDATA%\io.cakesplitter.desktop\tasks.sqlite3
```

It stores at most 64 nonterminal tasks, 500 retained terminal-history tasks,
and 32 MiB per task record. Startup processes at most 64 recovery records and a
bounded 20-record diagnostic overflow sample. One native worker is active at a
time; queue admission and persistence are serialized in Rust.

A record may include local full paths, native identities, expected size/hash,
Slice plans, package membership, progress, timestamps, and the last committed
checkpoint. It never contains source, Slice, manifest, or rebuilt file bytes.

## Pause and resume

Pause requests are acknowledged at a verified Slice boundary. A task displays
`Paused` only after the worker has committed that boundary. Resume reopens and
revalidates the source, destination, package, and partial output before work
continues. The stored checkpoint is not silently rewritten when validation
fails.

## Application exit and restart

Closing the window during active work presents three explicit choices:

- keep the app open;
- cancel active tasks safely; or
- interrupt at the next safe checkpoint and exit.

After an intentional interruption or unexpected exit, startup marks the task
`Interrupted` and `Eligible` only when a valid committed checkpoint exists.
Resume reacquires Windows directory authority and compares durable native
identities and package bindings. A same-name or same-size replacement, rebound
directory, reparse point, unavailable identity, corrupt record, or changed Slice
set fails closed and requires explicit user action.

The final manifest or rebuilt output is not published until every byte and
SHA-256 check succeeds. Partial data remains incomplete and may require manual
cleanup when ownership-safe deletion cannot be proven.

## Clear All

Clear All advances the database epoch, stops or drains active work, deletes
managed task/history rows and bounded quarantine diagnostics, and prevents stale
events from recreating them. It does not delete user output files. If the task
snapshot cannot be validated, event listeners and Clear All remain available so
local state can still be recovered or removed.
