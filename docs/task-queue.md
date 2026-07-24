# Desktop Task Queue

CakeSplitter Desktop `v0.8.0` keeps scheduling authoritative in Rust. The
renderer may request work, but it cannot bypass admission, conflict, identity,
or resource checks.

- one native execution worker is active at a time;
- at most 64 nonterminal tasks (queued plus active/recoverable) are persisted;
- admission is serialized before task persistence or worker allocation;
- queued priorities are high, normal, and low, with FIFO ordering within a
  priority and bounded fairness promotion after eight admissions;
- reordering is limited to queued tasks in the same priority group; and
- duplicate source, package, and output conflicts are rejected before work.

Requests above capacity return a structured error and leave no task record,
worker, file handle, or progress channel behind. Startup recovery processes no
more than 64 nonterminal records. A task plan cannot exceed the portable
50,000-Slice limit.
