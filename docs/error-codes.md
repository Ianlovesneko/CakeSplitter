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

## CLI exit-code contract

The v0.6 developer CLI keeps the established codes `2` (invalid input), `3`
(package integrity), `4` (collision), and `130` (cancellation), then assigns
stable codes to the additional structured categories:

| Exit | Category | Representative codes |
| ---: | --- | --- |
| `0` | success | completed command or completed plan |
| `1` | internal | `internal_failure` |
| `2` | usage / invalid Manifest | `invalid_arguments`, `invalid_json`, `invalid_manifest`, `invalid_slice_size` |
| `3` | package / integrity | `missing_slices`, `unexpected_slices`, `corrupted_slices`, `final_hash_mismatch`, `package_identity_changed` |
| `4` | conflict | `output_collision`, `receipt_collision` |
| `5` | source | `invalid_input`, `source_changed`, `non_utf8_filename` |
| `6` | destination | `destination_identity_changed`, `unsafe_filesystem_path`, `staged_identity_changed` |
| `7` | permission | permission-denied local I/O |
| `8` | storage | other bounded local I/O and receipt storage failures |
| `9` | recovery | `resume_rejected` |
| `10` | capacity | `resource_limit`, `package_enumeration_limit`, `insufficient_space` |
| `11` | batch failure | `batch_preflight_failed`, `batch_dependency_failed`, `batch_stop_policy`, or a completed batch with failed operations |
| `130` | cancellation | `cancelled` |

JSON and JSONL errors also carry the textual category, retryability,
privacy-safe technical message, suggested action, and operation ID. The exit
code is process metadata and is not duplicated into the versioned error object.
Batch JSONL uses `batch-failed` or `batch-cancelled` for preflight and recovery
errors; a loaded run uses `batch-completed`, `batch-failed`,
`batch-cancelled`, or `batch-interrupted` according to its persisted state.
