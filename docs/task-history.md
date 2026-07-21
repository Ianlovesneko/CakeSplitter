# Task History and Recovery

Desktop task records are checksummed SQLite rows in the per-user app-data
directory. Records include the operation plan, native selection identities,
progress, bounded failure history, and recovery checkpoints; they never store
selected file contents.

Terminal history is capped at 500 records and is aged using the configured
retention period (90 days by default, at most 3,650 days). Checkpoint-bearing
failed records are retained until explicit cleanup so their owned partial can
be removed safely; checkpoint history is independently capped at 500. Failed
history cleanup validates the recorded identity and removes an owned `.partial`
before deleting the row. Active and nonterminal records are never evicted as
history.

Clear All fences delayed writes with a new store epoch, drains or cancels
active work, removes managed records and quarantine evidence, and leaves user
outputs and unrelated files untouched. Restart recovery reopens only objects
whose recorded identities still match. Replaced or ambiguous selections fail
closed and require explicit reselection.
