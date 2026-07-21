# Desktop Error Codes

Errors cross the Tauri boundary as bounded `{ code, message, retryable,
recoveryAction, conflict }` values. Messages are privacy-safe and native Rust
is authoritative.

Important codes include:

- `queue_capacity_reached`, `checkpoint_history_capacity_reached`, and
  `recovery_capacity_exceeded` for bounded admission and recovery;
- `task_conflict` and `preflight_blocked` for resource planning;
- `destination_identity_changed` and `unsafe_filesystem_path` when a selected
  destination cannot be proven stable;
- `package_identity_changed` and `resume_rejected` for recovery rebinding;
- `export_collision`, `export_size_limit`, and `invalid_export` for receipts
  and diagnostics; and
- `insufficient_space`, `retry_not_allowed`, and `invalid_task_state` for
  operational control flow.

Transient errors identify a safe recovery action. The renderer never converts
an error into success and never decides whether a filesystem operation is
safe.
