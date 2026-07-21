# Operation Receipts

Completed and failed Desktop tasks can export bounded Markdown or JSON
operation receipts. Receipt content is generated locally from the checksummed
task record and includes operation, bounded plan/progress, result, timing,
hash evidence, and recovery status.

Receipt files use create-new semantics and never overwrite an existing file.
The selected output parent is held and revalidated with native directory
identity through publication. If the destination changes, export stops with a
structured identity error and no success result is returned.

Paths are masked by default. Additional local path detail requires an explicit
confirmation and retains only a safe filename-oriented representation. The
renderer receives the export display name and a short-lived reveal token, not
the absolute export path. Revealing a successful local export is a separate
native action.

Receipt size is bounded at 1 MiB. Receipts contain no file bytes, Slice bytes,
network destinations, telemetry, or upload behavior.
