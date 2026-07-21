# Diagnostic Bundles

Desktop diagnostic bundles are local support evidence, not crash reports.
They contain bounded runtime capabilities, storage counters, task summaries,
redacted recent errors, and privacy guidance. They do not contain source
bytes, Slice bytes, manifests, full identity records, hashes, or unrestricted
paths.

At most 100 task summaries are included. Each diagnostic file is capped at
2 MiB. Drive-letter paths, UNC paths, profile paths, emails, credential URLs,
secret-like values, control characters, and environment assignments are
redacted before writing.

The chosen output directory is bound to its native identity and revalidated
before creation and before each file publication. A replacement, junction,
reparse point, or inaccessible destination fails closed; a final bundle is not
announced as complete.
