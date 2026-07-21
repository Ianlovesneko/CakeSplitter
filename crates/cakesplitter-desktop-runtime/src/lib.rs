//! Native task execution and durable recovery for CakeSplitter Desktop.

mod engine;
mod exports;
mod model;
mod redaction;
mod store;

pub use engine::*;
pub use exports::*;
pub use model::*;
pub use redaction::*;
pub use store::*;

pub const TASK_STATE_SCHEMA_VERSION: u32 = 3;
pub const SCHEDULER_VERSION: u32 = 1;
/// One serialized worker is authoritative for native filesystem execution.
pub const MAX_ACTIVE_TASKS: usize = 1;
/// Queued work is bounded together with active/recoverable nonterminal work.
pub const MAX_QUEUED_TASKS: usize = 64;
pub const MAX_NONTERMINAL_TASKS: usize = 64;
pub const MAX_CONCURRENT_ADMISSIONS: usize = 1;
pub const MAX_RECOVERY_RECORDS: usize = MAX_NONTERMINAL_TASKS;
pub const MAX_TASK_METADATA_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_TASK_HISTORY: usize = 500;
pub const MAX_TASK_LIST_SNAPSHOTS: usize = MAX_TASK_HISTORY + MAX_NONTERMINAL_TASKS;
pub const MAX_QUARANTINE_RECORDS: usize = 20;
pub const MAX_QUARANTINE_DATA_BYTES: usize = 64 * 1024;
pub const MAX_QUARANTINE_REASON_BYTES: usize = 1_000;
pub const MAX_FAILURE_HISTORY: usize = 10;
pub const MAX_PREFLIGHT_WARNINGS: usize = 20;
pub const MAX_RECEIPT_BYTES: usize = 1024 * 1024;
pub const MAX_DIAGNOSTIC_TASKS: usize = 100;
pub const MAX_DIAGNOSTIC_FILE_BYTES: usize = 2 * 1024 * 1024;
pub const DEFAULT_HISTORY_RETENTION_DAYS: u32 = 90;
pub const MAX_HISTORY_RETENTION_DAYS: u32 = 3_650;
pub const FAIRNESS_ADMISSION_WINDOW: u64 = 8;
