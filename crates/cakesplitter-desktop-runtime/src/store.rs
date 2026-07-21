use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::Mutex,
};

use fs4::FileExt;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    FAIRNESS_ADMISSION_WINDOW, MAX_FAILURE_HISTORY, MAX_NONTERMINAL_TASKS,
    MAX_QUARANTINE_DATA_BYTES, MAX_QUARANTINE_REASON_BYTES, MAX_QUARANTINE_RECORDS,
    MAX_RECOVERY_RECORDS, MAX_TASK_HISTORY, MAX_TASK_METADATA_BYTES, SCHEDULER_VERSION,
    TASK_STATE_SCHEMA_VERSION,
    model::{
        DesktopPreferences, QueueDirection, RecoveryCheckpoint, StartupRecoveryReport,
        StartupRecoveryState, StorageSummary, TaskPriority, TaskRecord, TaskSpec, TaskStatus, now,
    },
};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("task store I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("another CakeSplitter Desktop process owns the task store")]
    ActiveWriter,
    #[error("task database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("task state serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("task state schema {actual} is newer than supported schema {supported}")]
    FutureSchema { actual: u32, supported: u32 },
    #[error("task was not found")]
    NotFound,
    #[error("task state was corrupt and has been quarantined")]
    CorruptState,
    #[error("task update belongs to stale epoch {actual}; current epoch is {expected}")]
    StaleEpoch { expected: u64, actual: u64 },
    #[error("task transition is invalid")]
    InvalidTransition,
    #[error("only terminal task history can be removed")]
    TaskActive,
    #[error("task queue capacity reached (maximum {maximum} nonterminal tasks)")]
    QueueCapacityReached { maximum: usize },
    #[error("checkpointed terminal history reached its maximum of {maximum} tasks")]
    CheckpointHistoryCapacityReached { maximum: usize },
    #[error("startup recovery found {actual} nonterminal tasks; maximum is {maximum}")]
    RecoveryCapacityExceeded { actual: usize, maximum: usize },
    #[error("task metadata is {actual} bytes; maximum is {maximum} bytes")]
    TaskMetadataTooLarge { actual: usize, maximum: usize },
    #[error("task plan exceeds the supported Slice bound")]
    TaskPlanTooLarge,
    #[error("queued task reordering is invalid")]
    InvalidReorder,
    #[error("task priority cannot be changed in the current state")]
    InvalidPriorityChange,
}

pub struct TaskStore {
    connection: Mutex<Connection>,
    _writer_lock: File,
    database_path: PathBuf,
    startup_recovery: Mutex<StartupRecoveryReport>,
}

impl TaskStore {
    pub fn open(app_data_directory: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(app_data_directory).map_err(|source| StoreError::Io {
            path: app_data_directory.to_path_buf(),
            source,
        })?;
        let lock_path = app_data_directory.join("task-store.lock");
        let writer_lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| StoreError::Io {
                path: lock_path.clone(),
                source,
            })?;
        FileExt::try_lock(&writer_lock).map_err(|_| StoreError::ActiveWriter)?;

        let database_path = app_data_directory.join("tasks.sqlite3");
        let mut connection = Connection::open(&database_path)?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let mut startup_recovery = StartupRecoveryReport::default();
        if version > TASK_STATE_SCHEMA_VERSION {
            let _ = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            drop(connection);
            quarantine_unsupported_database(&database_path, version)?;
            connection = Connection::open(&database_path)?;
            startup_recovery = StartupRecoveryReport {
                state: StartupRecoveryState::UnsupportedVersion,
                recovered_tasks: 0,
                quarantined_records: 1,
                capacity_exceeded_records: 0,
            };
        }
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        migrate(&mut connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
            _writer_lock: writer_lock,
            database_path,
            startup_recovery: Mutex::new(startup_recovery),
        })
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn epoch(&self) -> Result<u64, StoreError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        current_epoch(&connection)
    }

    pub fn insert(&self, mut record: TaskRecord) -> Result<TaskRecord, StoreError> {
        if record.plan.slice_count > cakesplitter_format::MAX_SLICE_COUNT {
            return Err(StoreError::TaskPlanTooLarge);
        }
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction()?;
        let epoch = current_epoch(&transaction)?;
        if record.epoch != epoch {
            return Err(StoreError::StaleEpoch {
                expected: epoch,
                actual: record.epoch,
            });
        }
        let nonterminal = count_nonterminal(&transaction)?;
        if !record.status.is_terminal() && nonterminal >= MAX_NONTERMINAL_TASKS {
            return Err(StoreError::QueueCapacityReached {
                maximum: MAX_NONTERMINAL_TASKS,
            });
        }
        if record.status.is_terminal()
            && record.checkpoint.is_some()
            && count_checkpointed_terminal(&transaction)? >= MAX_TASK_HISTORY
        {
            return Err(StoreError::CheckpointHistoryCapacityReached {
                maximum: crate::MAX_TASK_HISTORY,
            });
        }
        if record.status == TaskStatus::Queued && record.queue_order == 0 {
            record.queue_order = next_queue_order(&transaction)?;
        }
        record.revision = 1;
        record.updated_at = now();
        let (json, checksum) = encode(&record)?;
        validate_record_shape(&record).map_err(|_| StoreError::CorruptState)?;
        transaction.execute(
            "INSERT INTO tasks (id, epoch, revision, status, updated_at, data_json, checksum) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.id,
                record.epoch,
                record.revision,
                status_name(record.status),
                record.updated_at,
                json,
                checksum
            ],
        )?;
        prune_history(&transaction)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn ensure_admission_available(&self) -> Result<(), StoreError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if count_nonterminal(&connection)? >= MAX_NONTERMINAL_TASKS {
            return Err(StoreError::QueueCapacityReached {
                maximum: MAX_NONTERMINAL_TASKS,
            });
        }
        Ok(())
    }

    pub fn nonterminal_count(&self) -> Result<usize, StoreError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        count_nonterminal(&connection)
    }

    pub fn next_scheduled_task(&self) -> Result<Option<TaskRecord>, StoreError> {
        let mut queued = self
            .list()?
            .into_iter()
            .filter(|record| record.status == TaskStatus::Queued)
            .collect::<Vec<_>>();
        let maximum_order = queued
            .iter()
            .map(|record| record.queue_order)
            .max()
            .unwrap_or(0);
        queued.sort_by_key(|record| {
            let waited_admissions = maximum_order.saturating_sub(record.queue_order);
            let fairness_boost = waited_admissions / FAIRNESS_ADMISSION_WINDOW;
            let effective_priority = record.priority.rank().saturating_sub(fairness_boost);
            (effective_priority, record.queue_order, record.id.clone())
        });
        Ok(queued.into_iter().next())
    }

    pub fn queued_in_scheduler_order(&self) -> Result<Vec<TaskRecord>, StoreError> {
        let mut queued = self
            .list()?
            .into_iter()
            .filter(|record| record.status == TaskStatus::Queued)
            .collect::<Vec<_>>();
        let maximum_order = queued
            .iter()
            .map(|record| record.queue_order)
            .max()
            .unwrap_or(0);
        queued.sort_by_key(|record| {
            let waited_admissions = maximum_order.saturating_sub(record.queue_order);
            let fairness_boost = waited_admissions / FAIRNESS_ADMISSION_WINDOW;
            (
                record.priority.rank().saturating_sub(fairness_boost),
                record.queue_order,
                record.id.clone(),
            )
        });
        Ok(queued)
    }

    pub fn set_priority(
        &self,
        id: &str,
        epoch: u64,
        priority: TaskPriority,
    ) -> Result<TaskRecord, StoreError> {
        self.mutate(id, epoch, |record| {
            if record.status != TaskStatus::Queued {
                return Err(StoreError::InvalidPriorityChange);
            }
            record.priority = priority;
            Ok(())
        })
    }

    pub fn move_queued(
        &self,
        id: &str,
        direction: QueueDirection,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction()?;
        let row = read_row(&transaction, id)?.ok_or(StoreError::NotFound)?;
        let target = decode_row(&row).map_err(|_| StoreError::CorruptState)?;
        if target.status != TaskStatus::Queued {
            return Err(StoreError::InvalidReorder);
        }
        let rows = {
            let mut statement = transaction.prepare(
                "SELECT id, epoch, revision, status, updated_at, data_json, checksum \
                 FROM tasks WHERE status = 'queued'",
            )?;
            statement
                .query_map([], row_from_sql)?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut peers = rows
            .into_iter()
            .map(|row| decode_row(&row).map_err(|_| StoreError::CorruptState))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|record| record.priority == target.priority)
            .collect::<Vec<_>>();
        peers.sort_by_key(|record| (record.queue_order, record.id.clone()));
        let index = peers
            .iter()
            .position(|record| record.id == id)
            .ok_or(StoreError::InvalidReorder)?;
        let other_index = match direction {
            QueueDirection::Earlier => index.checked_sub(1),
            QueueDirection::Later => index.checked_add(1).filter(|next| *next < peers.len()),
        }
        .ok_or(StoreError::InvalidReorder)?;
        let first_order = peers[index].queue_order;
        let second_order = peers[other_index].queue_order;
        let mut first = peers[index].clone();
        let mut second = peers[other_index].clone();
        first.queue_order = second_order;
        second.queue_order = first_order;
        let timestamp = now();
        for record in [&mut first, &mut second] {
            record.revision = record
                .revision
                .checked_add(1)
                .ok_or(StoreError::CorruptState)?;
            record.updated_at = timestamp.clone();
            write_record(&transaction, record)?;
        }
        transaction.commit()?;
        Ok(vec![first, second])
    }

    pub fn clear_completed_history(&self) -> Result<usize, StoreError> {
        self.clear_terminal_status("completed")
    }

    pub fn clear_failed_history(&self) -> Result<usize, StoreError> {
        self.clear_terminal_status("failed")
    }

    pub fn clear_quarantine(&self) -> Result<usize, StoreError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        connection
            .execute("DELETE FROM quarantine", [])
            .map_err(StoreError::from)
    }

    pub fn storage_summary(
        &self,
        diagnostic_bundle_count: u64,
    ) -> Result<StorageSummary, StoreError> {
        let records = self.list()?;
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let quarantined_records: u64 =
            connection.query_row("SELECT COUNT(*) FROM quarantine", [], |row| row.get(0))?;
        let preferences = preferences_from_connection(&connection)?;
        let database_bytes = database_file_bytes(&self.database_path)?;
        let active_tasks = records
            .iter()
            .filter(|record| {
                matches!(
                    record.status,
                    TaskStatus::Running
                        | TaskStatus::Pausing
                        | TaskStatus::Paused
                        | TaskStatus::Resuming
                        | TaskStatus::Cancelling
                )
            })
            .count() as u64;
        let nonterminal_tasks = records
            .iter()
            .filter(|record| !record.status.is_terminal())
            .count() as u64;
        let terminal_history_tasks = records
            .iter()
            .filter(|record| record.status.is_terminal())
            .count() as u64;
        let incomplete_output_references = records
            .iter()
            .filter(|record| record.checkpoint.is_some() && record.status != TaskStatus::Completed)
            .count() as u64;
        Ok(StorageSummary {
            database_bytes,
            active_tasks,
            nonterminal_tasks,
            terminal_history_tasks,
            quarantined_records,
            incomplete_output_references,
            diagnostic_bundle_count,
            maximum_terminal_history: preferences.maximum_terminal_history,
            terminal_history_days: preferences.terminal_history_days,
        })
    }

    fn clear_terminal_status(&self, status: &str) -> Result<usize, StoreError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        connection
            .execute(
                "DELETE FROM tasks WHERE status = ?1 AND status IN ('cancelled', 'failed', 'completed')",
                params![status],
            )
            .map_err(StoreError::from)
    }

    pub fn get(&self, id: &str) -> Result<TaskRecord, StoreError> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction()?;
        let row = read_row(&transaction, id)?.ok_or(StoreError::NotFound)?;
        match decode_row(&row) {
            Ok(record) => {
                transaction.commit()?;
                Ok(record)
            }
            Err(reason) => {
                quarantine(&transaction, &row, &reason)?;
                transaction.commit()?;
                Err(StoreError::CorruptState)
            }
        }
    }

    pub fn list(&self) -> Result<Vec<TaskRecord>, StoreError> {
        self.list_validated().map(|(records, _)| records)
    }

    fn list_validated(&self) -> Result<(Vec<TaskRecord>, Vec<String>), StoreError> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction()?;
        let rows = {
            let mut statement = transaction.prepare(
                "SELECT id, epoch, revision, status, updated_at, data_json, checksum \
                 FROM tasks ORDER BY updated_at DESC, id DESC",
            )?;
            statement
                .query_map([], row_from_sql)?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut records = Vec::with_capacity(rows.len());
        let mut reasons = Vec::new();
        for row in rows {
            match decode_row(&row) {
                Ok(record) => records.push(record),
                Err(reason) => {
                    quarantine(&transaction, &row, &reason)?;
                    reasons.push(reason);
                }
            }
        }
        transaction.commit()?;
        Ok((records, reasons))
    }

    pub fn mutate<F>(&self, id: &str, epoch: u64, change: F) -> Result<TaskRecord, StoreError>
    where
        F: FnOnce(&mut TaskRecord) -> Result<(), StoreError>,
    {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction()?;
        let actual_epoch = current_epoch(&transaction)?;
        if epoch != actual_epoch {
            return Err(StoreError::StaleEpoch {
                expected: actual_epoch,
                actual: epoch,
            });
        }
        let row = read_row(&transaction, id)?.ok_or(StoreError::NotFound)?;
        let mut record = match decode_row(&row) {
            Ok(record) => record,
            Err(reason) => {
                quarantine(&transaction, &row, &reason)?;
                transaction.commit()?;
                return Err(StoreError::CorruptState);
            }
        };
        if record.epoch != epoch {
            return Err(StoreError::StaleEpoch {
                expected: epoch,
                actual: record.epoch,
            });
        }
        let was_terminal = record.status.is_terminal();
        let was_checkpointed_terminal = was_terminal && record.checkpoint.is_some();
        change(&mut record)?;
        if was_terminal
            && !record.status.is_terminal()
            && count_nonterminal(&transaction)? >= MAX_NONTERMINAL_TASKS
        {
            return Err(StoreError::QueueCapacityReached {
                maximum: MAX_NONTERMINAL_TASKS,
            });
        }
        if record.status.is_terminal()
            && record.checkpoint.is_some()
            && !was_checkpointed_terminal
            && count_checkpointed_terminal(&transaction)? >= MAX_TASK_HISTORY
        {
            return Err(StoreError::CheckpointHistoryCapacityReached {
                maximum: MAX_TASK_HISTORY,
            });
        }
        record.revision = record
            .revision
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        record.updated_at = now();
        let (json, checksum) = encode(&record)?;
        validate_record_shape(&record).map_err(|_| StoreError::CorruptState)?;
        let updated = transaction.execute(
            "UPDATE tasks SET revision = ?1, status = ?2, updated_at = ?3, data_json = ?4, \
             checksum = ?5 WHERE id = ?6 AND revision = ?7 AND epoch = ?8",
            params![
                record.revision,
                status_name(record.status),
                record.updated_at,
                json,
                checksum,
                record.id,
                row.revision,
                epoch
            ],
        )?;
        if updated != 1 {
            return Err(StoreError::StaleEpoch {
                expected: actual_epoch,
                actual: epoch,
            });
        }
        prune_history(&transaction)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn remove_terminal(&self, id: &str) -> Result<(), StoreError> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction()?;
        let row = read_row(&transaction, id)?.ok_or(StoreError::NotFound)?;
        let record = decode_row(&row).map_err(|_| StoreError::CorruptState)?;
        if !record.status.is_terminal() {
            return Err(StoreError::TaskActive);
        }
        transaction.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn remove_failed_admission(&self, id: &str) -> Result<(), StoreError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let removed = connection.execute(
            "DELETE FROM tasks WHERE id = ?1 AND status = 'queued'",
            params![id],
        )?;
        if removed == 1 {
            Ok(())
        } else {
            Err(StoreError::TaskActive)
        }
    }

    pub fn clear_all(&self) -> Result<u64, StoreError> {
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction()?;
        let next_epoch = current_epoch(&transaction)?
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        transaction.execute("DELETE FROM tasks", [])?;
        transaction.execute("DELETE FROM quarantine", [])?;
        transaction.execute(
            "UPDATE metadata SET epoch = ?1 WHERE singleton = 1",
            params![next_epoch],
        )?;
        transaction.commit()?;
        *self
            .startup_recovery
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = StartupRecoveryReport::default();
        Ok(next_epoch)
    }

    pub fn recover_after_restart(&self) -> Result<StartupRecoveryReport, StoreError> {
        let prior = self.startup_recovery_report();
        let mut capacity_exceeded_records = 0_usize;
        let mut capacity_quarantine_samples = 0_usize;
        {
            let mut connection = self
                .connection
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let transaction = connection.transaction()?;
            let actual = count_nonterminal(&transaction)?;
            if actual > MAX_RECOVERY_RECORDS {
                capacity_exceeded_records = actual - MAX_RECOVERY_RECORDS;
                let excess = {
                    let mut statement = transaction.prepare(
                        "SELECT id, epoch, revision, status, updated_at, data_json, checksum \
                         FROM tasks WHERE status NOT IN ('cancelled', 'failed', 'completed') \
                         ORDER BY updated_at ASC, id ASC LIMIT ?1 OFFSET ?2",
                    )?;
                    statement
                        .query_map(
                            params![MAX_QUARANTINE_RECORDS as u64, MAX_RECOVERY_RECORDS as u64],
                            row_from_sql,
                        )?
                        .collect::<Result<Vec<_>, _>>()?
                };
                capacity_quarantine_samples = excess.len();
                for row in excess {
                    quarantine(
                        &transaction,
                        &row,
                        "startup recovery capacity exceeded; record was not resumed",
                    )?;
                }
                transaction.execute(
                    "DELETE FROM tasks WHERE id IN ( \
                         SELECT id FROM tasks \
                         WHERE status NOT IN ('cancelled', 'failed', 'completed') \
                         ORDER BY updated_at ASC, id ASC LIMIT -1 OFFSET ?1 \
                     )",
                    params![MAX_RECOVERY_RECORDS as u64],
                )?;
            }
            prune_history(&transaction)?;
            transaction.commit()?;
        }
        let (records, quarantine_reasons) = self.list_validated()?;
        let mut recovered = 0;
        for record in records {
            if matches!(
                record.status,
                TaskStatus::Running
                    | TaskStatus::Pausing
                    | TaskStatus::Paused
                    | TaskStatus::Resuming
                    | TaskStatus::Cancelling
            ) {
                self.mutate(&record.id, record.epoch, |task| {
                    task.transition(TaskStatus::Interrupted)
                        .map_err(|_| StoreError::InvalidTransition)
                })?;
                recovered += 1;
            }
        }
        let corrupt = quarantine_reasons.iter().any(|reason| {
            reason.contains("checksum")
                || reason.contains("serialization")
                || reason.contains("JSON")
        });
        let unsupported = quarantine_reasons
            .iter()
            .any(|reason| reason.contains("schema version"));
        let state = if prior.state == StartupRecoveryState::UnsupportedVersion {
            StartupRecoveryState::UnsupportedVersion
        } else if capacity_exceeded_records > 0 {
            StartupRecoveryState::CapacityExceeded
        } else if corrupt {
            StartupRecoveryState::Corrupt
        } else if unsupported {
            StartupRecoveryState::UnsupportedVersion
        } else if !quarantine_reasons.is_empty() {
            StartupRecoveryState::Quarantined
        } else if recovered > 0 {
            StartupRecoveryState::RecoveryRequired
        } else {
            StartupRecoveryState::Ready
        };
        let report = StartupRecoveryReport {
            state,
            recovered_tasks: recovered,
            quarantined_records: (quarantine_reasons.len()
                + capacity_quarantine_samples
                + usize::from(prior.state == StartupRecoveryState::UnsupportedVersion))
            .min(MAX_QUARANTINE_RECORDS),
            capacity_exceeded_records,
        };
        *self
            .startup_recovery
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = report.clone();
        Ok(report)
    }

    pub fn startup_recovery_report(&self) -> StartupRecoveryReport {
        self.startup_recovery
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn preferences(&self) -> Result<DesktopPreferences, StoreError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        preferences_from_connection(&connection)
    }

    pub fn save_preferences(
        &self,
        preferences: &DesktopPreferences,
    ) -> Result<DesktopPreferences, StoreError> {
        if !preferences.validate() {
            return Err(StoreError::CorruptState);
        }
        let json = serde_json::to_string(preferences)?;
        let checksum = format!("{:x}", Sha256::digest(json.as_bytes()));
        let mut connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO settings (singleton, revision, data_json, checksum) VALUES (1, 1, ?1, ?2) \
             ON CONFLICT(singleton) DO UPDATE SET revision = revision + 1, data_json = ?1, checksum = ?2",
            params![json, checksum],
        )?;
        prune_history_with_preferences(&transaction, preferences)?;
        transaction.commit()?;
        Ok(preferences.clone())
    }
}

#[derive(Debug)]
struct StoredRow {
    id: String,
    epoch: u64,
    revision: u64,
    status: String,
    updated_at: String,
    data_json: String,
    checksum: String,
}

fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > TASK_STATE_SCHEMA_VERSION {
        return Err(StoreError::FutureSchema {
            actual: version,
            supported: TASK_STATE_SCHEMA_VERSION,
        });
    }
    if version == 0 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "CREATE TABLE metadata (\
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
                 schema_version INTEGER NOT NULL,\
                 epoch INTEGER NOT NULL\
             );\
             INSERT INTO metadata (singleton, schema_version, epoch) VALUES (1, 3, 1);\
             CREATE TABLE tasks (\
                 id TEXT PRIMARY KEY NOT NULL,\
                 epoch INTEGER NOT NULL,\
                 revision INTEGER NOT NULL,\
                 status TEXT NOT NULL,\
                 updated_at TEXT NOT NULL,\
                 data_json TEXT NOT NULL,\
                 checksum TEXT NOT NULL\
             );\
             CREATE INDEX tasks_history ON tasks(status, updated_at DESC);\
             CREATE TABLE quarantine (\
                 id INTEGER PRIMARY KEY AUTOINCREMENT,\
                 task_id TEXT NOT NULL,\
                 quarantined_at TEXT NOT NULL,\
                 reason TEXT NOT NULL,\
                 data_json TEXT NOT NULL,\
                 checksum TEXT NOT NULL\
             );\
             CREATE TABLE settings (\
                 singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
                 revision INTEGER NOT NULL,\
                 data_json TEXT NOT NULL,\
                 checksum TEXT NOT NULL\
             );",
        )?;
        transaction.pragma_update(None, "user_version", TASK_STATE_SCHEMA_VERSION)?;
        transaction.commit()?;
    } else if version < TASK_STATE_SCHEMA_VERSION {
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE metadata SET schema_version = ?1 WHERE singleton = 1",
            params![TASK_STATE_SCHEMA_VERSION],
        )?;
        transaction.pragma_update(None, "user_version", TASK_STATE_SCHEMA_VERSION)?;
        transaction.commit()?;
    }
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings (\
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),\
             revision INTEGER NOT NULL,\
             data_json TEXT NOT NULL,\
             checksum TEXT NOT NULL\
         );",
    )?;
    Ok(())
}

fn quarantine_unsupported_database(path: &Path, version: u32) -> Result<(), StoreError> {
    let suffix = format!("unsupported-v{version}-{}", uuid::Uuid::new_v4());
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tasks.sqlite3");
    let destination = parent.join(format!("{name}.{suffix}"));
    fs::rename(path, &destination).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    for sidecar in ["-wal", "-shm"] {
        let source = PathBuf::from(format!("{}{sidecar}", path.display()));
        if source.exists() {
            let target = PathBuf::from(format!("{}{sidecar}", destination.display()));
            fs::rename(&source, &target).map_err(|error| StoreError::Io {
                path: source,
                source: error,
            })?;
        }
    }
    Ok(())
}

fn current_epoch(connection: &Connection) -> Result<u64, StoreError> {
    connection
        .query_row(
            "SELECT epoch FROM metadata WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
}

fn next_queue_order(connection: &Connection) -> Result<u64, StoreError> {
    let maximum: Option<u64> = connection.query_row(
        "SELECT MAX(CAST(json_extract(data_json, '$.queueOrder') AS INTEGER)) FROM tasks",
        [],
        |row| row.get(0),
    )?;
    maximum
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(StoreError::CorruptState)
}

fn preferences_from_connection(connection: &Connection) -> Result<DesktopPreferences, StoreError> {
    let stored: Option<(String, String)> = connection
        .query_row(
            "SELECT data_json, checksum FROM settings WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((json, checksum)) = stored else {
        return Ok(DesktopPreferences::default());
    };
    if format!("{:x}", Sha256::digest(json.as_bytes())) != checksum {
        return Err(StoreError::CorruptState);
    }
    let preferences: DesktopPreferences = serde_json::from_str(&json)?;
    if !preferences.validate() {
        return Err(StoreError::CorruptState);
    }
    Ok(preferences)
}

fn database_file_bytes(path: &Path) -> Result<u64, StoreError> {
    let mut total = fs::metadata(path)
        .map_err(|source| StoreError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", path.display()));
        match fs::metadata(&sidecar) {
            Ok(metadata) => {
                total = total
                    .checked_add(metadata.len())
                    .ok_or(StoreError::CorruptState)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(StoreError::Io {
                    path: sidecar,
                    source,
                });
            }
        }
    }
    Ok(total)
}

fn read_row(connection: &Connection, id: &str) -> Result<Option<StoredRow>, StoreError> {
    connection
        .query_row(
            "SELECT id, epoch, revision, status, updated_at, data_json, checksum \
             FROM tasks WHERE id = ?1",
            params![id],
            row_from_sql,
        )
        .optional()
        .map_err(StoreError::from)
}

fn row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredRow> {
    Ok(StoredRow {
        id: row.get(0)?,
        epoch: row.get(1)?,
        revision: row.get(2)?,
        status: row.get(3)?,
        updated_at: row.get(4)?,
        data_json: row.get(5)?,
        checksum: row.get(6)?,
    })
}

fn encode(record: &TaskRecord) -> Result<(String, String), StoreError> {
    let json = serde_json::to_string(record)?;
    if json.len() > MAX_TASK_METADATA_BYTES {
        return Err(StoreError::TaskMetadataTooLarge {
            actual: json.len(),
            maximum: MAX_TASK_METADATA_BYTES,
        });
    }
    let checksum = format!("{:x}", Sha256::digest(json.as_bytes()));
    Ok((json, checksum))
}

fn write_record(transaction: &Transaction<'_>, record: &TaskRecord) -> Result<(), StoreError> {
    validate_record_shape(record).map_err(|_| StoreError::CorruptState)?;
    let (json, checksum) = encode(record)?;
    let updated = transaction.execute(
        "UPDATE tasks SET revision = ?1, status = ?2, updated_at = ?3, data_json = ?4, \
         checksum = ?5 WHERE id = ?6 AND epoch = ?7",
        params![
            record.revision,
            status_name(record.status),
            record.updated_at,
            json,
            checksum,
            record.id,
            record.epoch,
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

fn count_nonterminal(connection: &Connection) -> Result<usize, StoreError> {
    let count: u64 = connection.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status NOT IN ('cancelled', 'failed', 'completed')",
        [],
        |row| row.get(0),
    )?;
    usize::try_from(count).map_err(|_| StoreError::RecoveryCapacityExceeded {
        actual: usize::MAX,
        maximum: MAX_RECOVERY_RECORDS,
    })
}

fn count_checkpointed_terminal(connection: &Connection) -> Result<usize, StoreError> {
    let count: u64 = connection.query_row(
        "SELECT COUNT(*) FROM tasks WHERE status IN ('cancelled', 'failed', 'completed') AND json_extract(data_json, '$.checkpoint') IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    usize::try_from(count).map_err(|_| StoreError::RecoveryCapacityExceeded {
        actual: usize::MAX,
        maximum: MAX_TASK_HISTORY,
    })
}

fn decode_row(row: &StoredRow) -> Result<TaskRecord, String> {
    if row.data_json.len() > MAX_TASK_METADATA_BYTES {
        return Err("task metadata exceeds the supported limit".to_owned());
    }
    let expected = format!("{:x}", Sha256::digest(row.data_json.as_bytes()));
    if expected != row.checksum {
        return Err("checksum mismatch".to_owned());
    }
    let mut value: serde_json::Value = serde_json::from_str(&row.data_json)
        .map_err(|error| format!("task JSON serialization is invalid: {error}"))?;
    let schema_version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "task schema version is missing".to_owned())?;
    if schema_version == 2 {
        let object = value
            .as_object_mut()
            .ok_or_else(|| "task JSON must be an object".to_owned())?;
        object.insert(
            "schemaVersion".to_owned(),
            serde_json::Value::from(TASK_STATE_SCHEMA_VERSION),
        );
        object.insert(
            "schedulerVersion".to_owned(),
            serde_json::Value::from(SCHEDULER_VERSION),
        );
        object.insert("queueOrder".to_owned(), serde_json::Value::from(1_u64));
    } else if schema_version != u64::from(TASK_STATE_SCHEMA_VERSION) {
        return Err(format!(
            "task schema version {schema_version} is unsupported; expected 2 or {}",
            TASK_STATE_SCHEMA_VERSION
        ));
    }
    let record: TaskRecord = serde_json::from_value(value)
        .map_err(|error| format!("task JSON serialization is invalid: {error}"))?;
    if record.plan.slice_count > cakesplitter_format::MAX_SLICE_COUNT {
        return Err("task plan exceeds the supported Slice limit".to_owned());
    }
    if record.id != row.id
        || record.epoch != row.epoch
        || record.revision != row.revision
        || status_name(record.status) != row.status
        || record.updated_at != row.updated_at
        || record.schema_version != TASK_STATE_SCHEMA_VERSION
    {
        if record.schema_version != TASK_STATE_SCHEMA_VERSION {
            return Err(format!(
                "task schema version {} is unsupported; expected {}",
                record.schema_version, TASK_STATE_SCHEMA_VERSION
            ));
        }
        return Err("indexed task metadata does not match the checksummed record".to_owned());
    }
    validate_record_shape(&record)?;
    Ok(record)
}

fn validate_record_shape(record: &TaskRecord) -> Result<(), String> {
    if uuid::Uuid::parse_str(&record.id).is_err()
        || record.epoch == 0
        || record.revision == 0
        || record.scheduler_version != SCHEDULER_VERSION
        || record.operation != record.spec.operation()
        || record.format_version != cakesplitter_format::FORMAT_VERSION
        || record.application_version.is_empty()
        || record.application_version.len() > 64
        || record.display_name.len() > 500
        || record
            .destination_name
            .as_ref()
            .is_some_and(|value| value.len() > 500)
        || (record.status == TaskStatus::Queued && record.queue_order == 0)
        || record.failure_history.len() > MAX_FAILURE_HISTORY
        || record
            .preflight
            .as_ref()
            .is_some_and(|preflight| preflight.warnings.len() > crate::MAX_PREFLIGHT_WARNINGS)
        || record.progress.bytes_processed > record.progress.total_bytes
        || record.progress.current_slice > record.progress.slice_count
        || (record.progress.slice_count != 0
            && record.progress.slice_count != record.plan.slice_count)
        || (record.progress.total_bytes != 0
            && record.progress.total_bytes != record.plan.total_bytes)
        || chrono::DateTime::parse_from_rfc3339(&record.created_at).is_err()
        || chrono::DateTime::parse_from_rfc3339(&record.updated_at).is_err()
        || record
            .started_at
            .as_ref()
            .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err())
        || record
            .finished_at
            .as_ref()
            .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err())
    {
        return Err("task metadata invariants are invalid".to_owned());
    }
    let path_is_safe = |path: &Path| {
        path.is_absolute()
            && !path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )
            })
    };
    for failure in record.failure.iter().chain(record.failure_history.iter()) {
        if failure.code.is_empty()
            || failure.code.len() > 80
            || failure.message.len() > 2_000
            || failure.technical_message.len() > 2_000
            || (!failure.occurred_at.is_empty()
                && chrono::DateTime::parse_from_rfc3339(&failure.occurred_at).is_err())
        {
            return Err("task failure metadata is invalid".to_owned());
        }
    }
    if record.preflight.as_ref().is_some_and(|preflight| {
        preflight.checked_at.is_empty()
            || chrono::DateTime::parse_from_rfc3339(&preflight.checked_at).is_err()
            || preflight.expected_output_count > cakesplitter_format::MAX_SLICE_COUNT + 1
    }) {
        return Err("task preflight metadata is invalid".to_owned());
    }
    match &record.spec {
        TaskSpec::Split {
            source_path,
            output_directory,
            slice_size,
            package_id,
            created_at,
        } => {
            if !path_is_safe(source_path)
                || !path_is_safe(output_directory)
                || *slice_size == 0
                || *slice_size != record.plan.slice_size
                || uuid::Uuid::parse_str(package_id).is_err()
                || chrono::DateTime::parse_from_rfc3339(created_at).is_err()
                || matches!(record.checkpoint, Some(RecoveryCheckpoint::Merge(_)))
            {
                return Err("Split recovery metadata is invalid".to_owned());
            }
        }
        TaskSpec::Merge {
            manifest_path,
            output_path,
            package_binding,
        } => {
            if !path_is_safe(manifest_path)
                || !path_is_safe(output_path)
                || cakesplitter_core::validate_package_binding_shape(package_binding).is_err()
                || matches!(record.checkpoint, Some(RecoveryCheckpoint::Split(_)))
            {
                return Err("Merge recovery binding is invalid".to_owned());
            }
        }
        TaskSpec::Inspect {
            manifest_path,
            package_binding,
            ..
        }
        | TaskSpec::Verify {
            manifest_path,
            package_binding,
        } => {
            if !path_is_safe(manifest_path)
                || cakesplitter_core::validate_package_binding_shape(package_binding).is_err()
                || record.checkpoint.is_some()
            {
                return Err("inspection recovery binding is invalid".to_owned());
            }
        }
    }
    if record.status == TaskStatus::Completed && record.result.is_none() {
        return Err("completed task is missing its result".to_owned());
    }
    if record.status == TaskStatus::Failed && record.failure.is_none() {
        return Err("failed task is missing its structured failure".to_owned());
    }
    Ok(())
}

fn quarantine(
    transaction: &Transaction<'_>,
    row: &StoredRow,
    reason: &str,
) -> Result<(), StoreError> {
    let bounded_reason = truncate_utf8_bytes(reason, MAX_QUARANTINE_REASON_BYTES);
    let bounded_data = truncate_utf8_bytes(&row.data_json, MAX_QUARANTINE_DATA_BYTES);
    transaction.execute(
        "INSERT INTO quarantine (task_id, quarantined_at, reason, data_json, checksum) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![row.id, now(), bounded_reason, bounded_data, row.checksum],
    )?;
    transaction.execute("DELETE FROM tasks WHERE id = ?1", params![row.id])?;
    transaction.execute(
        "DELETE FROM quarantine WHERE id NOT IN ( \
             SELECT id FROM quarantine ORDER BY id DESC LIMIT ?1 \
         )",
        params![MAX_QUARANTINE_RECORDS as u64],
    )?;
    Ok(())
}

fn truncate_utf8_bytes(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        return value;
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

fn prune_history(transaction: &Transaction<'_>) -> Result<(), StoreError> {
    let preferences = preferences_from_connection(transaction)?;
    prune_history_with_preferences(transaction, &preferences)
}

fn prune_history_with_preferences(
    transaction: &Transaction<'_>,
    preferences: &DesktopPreferences,
) -> Result<(), StoreError> {
    let cutoff = (chrono::Utc::now()
        - chrono::Duration::days(i64::from(preferences.terminal_history_days)))
    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    transaction.execute(
        "DELETE FROM tasks WHERE status IN ('cancelled', 'failed', 'completed') \
         AND json_extract(data_json, '$.checkpoint') IS NULL \
         AND updated_at < ?1",
        params![cutoff],
    )?;
    transaction.execute(
        "DELETE FROM tasks WHERE id IN ( \
             SELECT id FROM tasks \
             WHERE status IN ('cancelled', 'failed', 'completed') \
               AND json_extract(data_json, '$.checkpoint') IS NULL \
             ORDER BY updated_at DESC, id DESC \
             LIMIT -1 OFFSET ?1 \
         )",
        params![u64::from(preferences.maximum_terminal_history)],
    )?;
    Ok(())
}

fn status_name(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Planned => "planned",
        TaskStatus::Queued => "queued",
        TaskStatus::Running => "running",
        TaskStatus::Pausing => "pausing",
        TaskStatus::Paused => "paused",
        TaskStatus::Resuming => "resuming",
        TaskStatus::Cancelling => "cancelling",
        TaskStatus::Cancelled => "cancelled",
        TaskStatus::Interrupted => "interrupted",
        TaskStatus::PermissionRequired => "permission-required",
        TaskStatus::Failed => "failed",
        TaskStatus::Completed => "completed",
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use cakesplitter_core::{
        DirectoryFingerprint, NativeFileIdentity, SourceFingerprint, SplitResumeData,
    };
    use tempfile::tempdir;

    use super::*;
    use crate::model::{ProcessingPlan, TaskSpec};

    fn sample_record(store: &TaskStore) -> TaskRecord {
        TaskRecord::new(
            "0.4.0",
            store.epoch().unwrap(),
            "sample.bin".to_owned(),
            Some("output".to_owned()),
            TaskSpec::Split {
                source_path: PathBuf::from(r"C:\fixtures\sample.bin"),
                output_directory: PathBuf::from(r"C:\fixtures\output"),
                slice_size: 1024,
                package_id: "9cd16d17-3b92-4884-8f65-e0d64d11c93e".to_owned(),
                created_at: "2026-07-18T00:00:00Z".to_owned(),
            },
            ProcessingPlan {
                total_bytes: 2048,
                slice_size: 1024,
                slice_count: 2,
                required_free_bytes: 2048,
                ..ProcessingPlan::default()
            },
        )
    }

    #[test]
    fn round_trips_checksummed_versioned_task_state() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut record = sample_record(&store);
        record.transition(TaskStatus::Queued).unwrap();
        let inserted = store.insert(record).unwrap();
        assert_eq!(inserted.revision, 1);
        let loaded = store.get(&inserted.id).unwrap();
        assert_eq!(loaded.id, inserted.id);
        assert_eq!(loaded.status, TaskStatus::Queued);
        assert_eq!(loaded.schema_version, TASK_STATE_SCHEMA_VERSION);
    }

    #[test]
    fn quarantines_a_task_with_a_modified_payload() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut record = sample_record(&store);
        record.transition(TaskStatus::Queued).unwrap();
        let inserted = store.insert(record).unwrap();
        {
            let connection = store
                .connection
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            connection
                .execute(
                    "UPDATE tasks SET data_json = data_json || ' ' WHERE id = ?1",
                    params![inserted.id],
                )
                .unwrap();
        }
        assert!(matches!(
            store.get(&inserted.id),
            Err(StoreError::CorruptState)
        ));
        assert!(matches!(store.get(&inserted.id), Err(StoreError::NotFound)));
        let connection = store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let count: u64 = connection
            .query_row("SELECT COUNT(*) FROM quarantine", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn clear_all_fences_delayed_updates_from_the_previous_epoch() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut record = sample_record(&store);
        record.transition(TaskStatus::Queued).unwrap();
        let inserted = store.insert(record).unwrap();
        let mut corrupt = sample_record(&store);
        corrupt.transition(TaskStatus::Queued).unwrap();
        let corrupt = store.insert(corrupt).unwrap();
        {
            let connection = store
                .connection
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            connection
                .execute(
                    "UPDATE tasks SET data_json = data_json || ' ' WHERE id = ?1",
                    params![corrupt.id],
                )
                .unwrap();
        }
        assert!(matches!(
            store.get(&corrupt.id),
            Err(StoreError::CorruptState)
        ));
        let old_epoch = inserted.epoch;
        let new_epoch = store.clear_all().unwrap();
        assert!(new_epoch > old_epoch);
        assert!(matches!(
            store.mutate(&inserted.id, old_epoch, |_| Ok(())),
            Err(StoreError::StaleEpoch { .. })
        ));
        assert!(store.list().unwrap().is_empty());
        let connection = store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let quarantine_count: u64 = connection
            .query_row("SELECT COUNT(*) FROM quarantine", [], |row| row.get(0))
            .unwrap();
        assert_eq!(quarantine_count, 0);
    }

    #[test]
    fn settings_are_checksummed_and_survive_task_history_clear() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let preferences = DesktopPreferences {
            default_slice_size: 64 * 1024 * 1024,
            confirm_destructive_actions: false,
            reduce_motion: true,
            ..DesktopPreferences::default()
        };
        store.save_preferences(&preferences).unwrap();
        store.clear_all().unwrap();
        assert_eq!(store.preferences().unwrap(), preferences);
    }

    #[test]
    fn startup_recovery_marks_in_process_states_interrupted() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut record = sample_record(&store);
        record.transition(TaskStatus::Queued).unwrap();
        let inserted = store.insert(record).unwrap();
        store
            .mutate(&inserted.id, inserted.epoch, |task| {
                task.transition(TaskStatus::Running)
                    .map_err(|_| StoreError::InvalidTransition)
            })
            .unwrap();
        let report = store.recover_after_restart().unwrap();
        assert_eq!(report.recovered_tasks, 1);
        assert_eq!(report.state, StartupRecoveryState::RecoveryRequired);
        assert_eq!(
            store.get(&inserted.id).unwrap().status,
            TaskStatus::Interrupted
        );
    }

    #[test]
    fn active_writer_fencing_rejects_a_second_store_process() {
        let root = tempdir().unwrap();
        let _store = TaskStore::open(root.path()).unwrap();
        assert!(matches!(
            TaskStore::open(root.path()),
            Err(StoreError::ActiveWriter)
        ));
    }

    #[test]
    fn quarantine_payload_and_history_are_bounded() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut connection = store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction().unwrap();
        for index in 0..MAX_QUARANTINE_RECORDS + 5 {
            let row = StoredRow {
                id: format!("corrupt-{index}"),
                epoch: 1,
                revision: 1,
                status: "failed".to_owned(),
                updated_at: now(),
                data_json: "資料".repeat(MAX_QUARANTINE_DATA_BYTES),
                checksum: "0".repeat(64),
            };
            quarantine(
                &transaction,
                &row,
                &"reason".repeat(MAX_QUARANTINE_REASON_BYTES),
            )
            .unwrap();
        }
        transaction.commit().unwrap();

        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM quarantine", [], |row| row.get(0))
            .unwrap();
        let maximum_data: i64 = connection
            .query_row(
                "SELECT MAX(LENGTH(CAST(data_json AS BLOB))) FROM quarantine",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let maximum_reason: i64 = connection
            .query_row(
                "SELECT MAX(LENGTH(CAST(reason AS BLOB))) FROM quarantine",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, MAX_QUARANTINE_RECORDS as i64);
        assert!(maximum_data <= MAX_QUARANTINE_DATA_BYTES as i64);
        assert!(maximum_reason <= MAX_QUARANTINE_REASON_BYTES as i64);
    }

    #[test]
    fn admission_accepts_exact_limit_rejects_excess_and_leaves_no_residue() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        for _ in 0..MAX_NONTERMINAL_TASKS {
            let mut record = sample_record(&store);
            record.transition(TaskStatus::Queued).unwrap();
            store.insert(record).unwrap();
        }
        assert_eq!(store.nonterminal_count().unwrap(), MAX_NONTERMINAL_TASKS);

        for _ in 0..1_000 {
            let mut rejected = sample_record(&store);
            rejected.transition(TaskStatus::Queued).unwrap();
            assert!(matches!(
                store.insert(rejected),
                Err(StoreError::QueueCapacityReached { .. })
            ));
        }
        assert_eq!(store.nonterminal_count().unwrap(), MAX_NONTERMINAL_TASKS);
        assert_eq!(store.list().unwrap().len(), MAX_NONTERMINAL_TASKS);
    }

    #[test]
    fn concurrent_duplicate_admissions_are_serialized_at_the_limit() {
        let root = tempdir().unwrap();
        let store = Arc::new(TaskStore::open(root.path()).unwrap());
        let mut attempts = Vec::new();
        for _ in 0..MAX_NONTERMINAL_TASKS * 2 {
            let store = Arc::clone(&store);
            attempts.push(thread::spawn(move || {
                let mut record = sample_record(&store);
                record.transition(TaskStatus::Queued).unwrap();
                store.insert(record).is_ok()
            }));
        }
        let admitted = attempts
            .into_iter()
            .map(|attempt| attempt.join().unwrap())
            .filter(|admitted| *admitted)
            .count();
        assert_eq!(admitted, MAX_NONTERMINAL_TASKS);
        assert_eq!(store.nonterminal_count().unwrap(), MAX_NONTERMINAL_TASKS);
    }

    #[test]
    fn startup_recovery_accepts_limit_and_fails_closed_above_limit() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        for _ in 0..MAX_RECOVERY_RECORDS {
            let mut record = sample_record(&store);
            record.transition(TaskStatus::Queued).unwrap();
            store.insert(record).unwrap();
        }
        assert_eq!(
            store.recover_after_restart().unwrap().state,
            StartupRecoveryState::Ready
        );

        let mut extra = sample_record(&store);
        extra.transition(TaskStatus::Queued).unwrap();
        extra.revision = 1;
        extra.updated_at = now();
        let (json, checksum) = encode(&extra).unwrap();
        store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .execute(
                "INSERT INTO tasks (id, epoch, revision, status, updated_at, data_json, checksum) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    extra.id,
                    extra.epoch,
                    extra.revision,
                    status_name(extra.status),
                    extra.updated_at,
                    json,
                    checksum
                ],
            )
            .unwrap();
        let report = store.recover_after_restart().unwrap();
        assert_eq!(report.state, StartupRecoveryState::CapacityExceeded);
        assert_eq!(report.capacity_exceeded_records, 1);
        assert_eq!(report.quarantined_records, 1);
        assert_eq!(store.nonterminal_count().unwrap(), MAX_RECOVERY_RECORDS);
    }

    #[test]
    fn bootstrap_bounds_thousands_of_persisted_rows_without_loading_the_overflow() {
        const NONTERMINAL_ROWS: usize = 2_112;
        const TERMINAL_ROWS: usize = 1_500;

        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let template = sample_record(&store);
        let mut connection = store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let transaction = connection.transaction().unwrap();

        for queue_order in 0..NONTERMINAL_ROWS {
            let mut record = template.clone();
            record.id = uuid::Uuid::new_v4().to_string();
            record.queue_order = queue_order as u64 + 1;
            record.transition(TaskStatus::Queued).unwrap();
            record.revision = 1;
            let (json, checksum) = encode(&record).unwrap();
            transaction
                .execute(
                    "INSERT INTO tasks (id, epoch, revision, status, updated_at, data_json, checksum) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        record.id,
                        record.epoch,
                        record.revision,
                        status_name(record.status),
                        record.updated_at,
                        json,
                        checksum
                    ],
                )
                .unwrap();
        }

        for _ in 0..TERMINAL_ROWS {
            let mut record = template.clone();
            record.id = uuid::Uuid::new_v4().to_string();
            record.status = TaskStatus::Completed;
            record.result = Some(crate::model::TaskResult::Split {
                manifest_filename: "sample.bin.cake.json".to_owned(),
                source_sha256: "a".repeat(64),
            });
            record.revision = 1;
            let (json, checksum) = encode(&record).unwrap();
            transaction
                .execute(
                    "INSERT INTO tasks (id, epoch, revision, status, updated_at, data_json, checksum) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        record.id,
                        record.epoch,
                        record.revision,
                        status_name(record.status),
                        record.updated_at,
                        json,
                        checksum
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        drop(connection);

        let report = store.recover_after_restart().unwrap();
        assert_eq!(report.state, StartupRecoveryState::CapacityExceeded);
        assert_eq!(
            report.capacity_exceeded_records,
            NONTERMINAL_ROWS - MAX_RECOVERY_RECORDS
        );
        assert_eq!(report.quarantined_records, MAX_QUARANTINE_RECORDS);
        let connection = store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let task_count: usize = connection
            .query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0))
            .unwrap();
        let quarantine_count: usize = connection
            .query_row("SELECT COUNT(*) FROM quarantine", [], |row| row.get(0))
            .unwrap();
        assert_eq!(task_count, MAX_RECOVERY_RECORDS + MAX_TASK_HISTORY);
        assert_eq!(quarantine_count, MAX_QUARANTINE_RECORDS);
    }

    #[test]
    fn terminal_history_is_pruned_without_evicting_nonterminal_tasks() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        for _ in 0..MAX_NONTERMINAL_TASKS {
            let mut record = sample_record(&store);
            record.transition(TaskStatus::Queued).unwrap();
            store.insert(record).unwrap();
        }
        for _ in 0..MAX_TASK_HISTORY + 5 {
            let mut record = sample_record(&store);
            record.status = TaskStatus::Completed;
            record.result = Some(crate::model::TaskResult::Split {
                manifest_filename: "sample.bin.cake.json".to_owned(),
                source_sha256: "a".repeat(64),
            });
            store.insert(record).unwrap();
        }
        let records = store.list().unwrap();
        assert_eq!(
            records
                .iter()
                .filter(|record| !record.status.is_terminal())
                .count(),
            MAX_NONTERMINAL_TASKS
        );
        assert_eq!(
            records
                .iter()
                .filter(|record| record.status.is_terminal())
                .count(),
            MAX_TASK_HISTORY
        );
    }

    #[test]
    fn checkpointed_failed_history_is_not_pruned_or_admitted_unboundedly() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut record = sample_record(&store);
        record.status = TaskStatus::Failed;
        record.failure = Some(crate::model::TaskFailure::bounded(
            "failed",
            "recoverable failure",
        ));
        let identity = NativeFileIdentity { volume: 1, file: 2 };
        record.checkpoint = Some(RecoveryCheckpoint::Split(SplitResumeData {
            source: SourceFingerprint {
                identity,
                len: 2_048,
                modified_unix_nanos: 1,
            },
            output_directory: DirectoryFingerprint { identity },
            baseline_sha256: "a".repeat(64),
            completed: Vec::new(),
            active_partial: None,
            published_manifest_sha256: None,
        }));
        let inserted = store.insert(record).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(matches!(store.ensure_admission_available(), Ok(())));

        let connection = store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let checkpointed: usize = connection
            .query_row(
                "SELECT COUNT(*) FROM tasks WHERE json_extract(data_json, '$.checkpoint') IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(checkpointed, 1);
        drop(connection);

        let mut queued = sample_record(&store);
        queued.transition(TaskStatus::Queued).unwrap();
        assert!(store.insert(queued).is_ok());
        assert_eq!(store.get(&inserted.id).unwrap().status, TaskStatus::Failed);
    }

    #[test]
    fn terminal_completion_releases_capacity_for_a_new_task() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut first_id = None;
        for index in 0..MAX_NONTERMINAL_TASKS {
            let mut record = sample_record(&store);
            record.transition(TaskStatus::Queued).unwrap();
            let record = store.insert(record).unwrap();
            if index == 0 {
                first_id = Some((record.id, record.epoch));
            }
        }
        let (id, epoch) = first_id.unwrap();
        let running = store
            .mutate(&id, epoch, |task| {
                task.transition(TaskStatus::Running)
                    .map_err(|_| StoreError::InvalidTransition)
            })
            .unwrap();
        store
            .mutate(&id, running.epoch, |task| {
                task.result = Some(crate::model::TaskResult::Split {
                    manifest_filename: "sample.bin.cake.json".to_owned(),
                    source_sha256: "a".repeat(64),
                });
                task.transition(TaskStatus::Completed)
                    .map_err(|_| StoreError::InvalidTransition)
            })
            .unwrap();
        let mut replacement = sample_record(&store);
        replacement.transition(TaskStatus::Queued).unwrap();
        store.insert(replacement).unwrap();
        assert_eq!(store.nonterminal_count().unwrap(), MAX_NONTERMINAL_TASKS);
    }

    #[test]
    fn scheduler_is_empty_then_returns_one_queued_task() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        assert!(store.next_scheduled_task().unwrap().is_none());
        let mut record = sample_record(&store);
        record.transition(TaskStatus::Queued).unwrap();
        let inserted = store.insert(record).unwrap();
        let next = store.next_scheduled_task().unwrap().unwrap();
        assert_eq!(next.id, inserted.id);
        assert_eq!(next.queue_order, 1);
    }

    #[test]
    fn scheduler_orders_priority_then_fifo_for_equal_priority() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut ids = Vec::new();
        for priority in [
            TaskPriority::Low,
            TaskPriority::High,
            TaskPriority::Normal,
            TaskPriority::High,
        ] {
            let mut record = sample_record(&store);
            record.priority = priority;
            record.transition(TaskStatus::Queued).unwrap();
            ids.push(store.insert(record).unwrap().id);
        }
        let scheduled = store
            .queued_in_scheduler_order()
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        assert_eq!(
            scheduled,
            vec![
                ids[1].clone(),
                ids[3].clone(),
                ids[2].clone(),
                ids[0].clone()
            ]
        );
    }

    #[test]
    fn scheduler_fairness_promotes_old_low_priority_work() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut low = sample_record(&store);
        low.priority = TaskPriority::Low;
        low.transition(TaskStatus::Queued).unwrap();
        let low = store.insert(low).unwrap();
        for _ in 0..(FAIRNESS_ADMISSION_WINDOW * 2) {
            let mut high = sample_record(&store);
            high.priority = TaskPriority::High;
            high.transition(TaskStatus::Queued).unwrap();
            store.insert(high).unwrap();
        }
        assert_eq!(store.next_scheduled_task().unwrap().unwrap().id, low.id);
    }

    #[test]
    fn reorder_is_atomic_bounded_and_persists_across_restart() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut ids = Vec::new();
        for _ in 0..3 {
            let mut record = sample_record(&store);
            record.transition(TaskStatus::Queued).unwrap();
            ids.push(store.insert(record).unwrap().id);
        }
        assert!(matches!(
            store.move_queued(&ids[0], QueueDirection::Earlier),
            Err(StoreError::InvalidReorder)
        ));
        store.move_queued(&ids[2], QueueDirection::Earlier).unwrap();
        let ordered = store
            .queued_in_scheduler_order()
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ordered,
            vec![ids[0].clone(), ids[2].clone(), ids[1].clone()]
        );
        drop(store);
        let reopened = TaskStore::open(root.path()).unwrap();
        let persisted = reopened
            .queued_in_scheduler_order()
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        assert_eq!(persisted, ordered);
    }

    #[test]
    fn priority_changes_are_queued_only_and_do_not_cross_reorder_groups() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut normal = sample_record(&store);
        normal.transition(TaskStatus::Queued).unwrap();
        let normal = store.insert(normal).unwrap();
        let changed = store
            .set_priority(&normal.id, normal.epoch, TaskPriority::High)
            .unwrap();
        assert_eq!(changed.priority, TaskPriority::High);

        let mut low = sample_record(&store);
        low.priority = TaskPriority::Low;
        low.transition(TaskStatus::Queued).unwrap();
        let low = store.insert(low).unwrap();
        assert!(matches!(
            store.move_queued(&low.id, QueueDirection::Earlier),
            Err(StoreError::InvalidReorder)
        ));

        let completed = store
            .mutate(&changed.id, changed.epoch, |record| {
                record
                    .transition(TaskStatus::Running)
                    .map_err(|_| StoreError::InvalidTransition)?;
                record.result = Some(crate::model::TaskResult::Split {
                    manifest_filename: "sample.bin.cake.json".to_owned(),
                    source_sha256: "a".repeat(64),
                });
                record
                    .transition(TaskStatus::Completed)
                    .map_err(|_| StoreError::InvalidTransition)
            })
            .unwrap();
        assert!(matches!(
            store.set_priority(&completed.id, completed.epoch, TaskPriority::Normal),
            Err(StoreError::InvalidPriorityChange)
        ));
    }

    #[test]
    fn retention_prunes_by_age_and_count_without_removing_nonterminal_work() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut queued = sample_record(&store);
        queued.transition(TaskStatus::Queued).unwrap();
        let queued = store.insert(queued).unwrap();
        for index in 0..4 {
            let mut terminal = sample_record(&store);
            terminal.status = TaskStatus::Completed;
            terminal.result = Some(crate::model::TaskResult::Split {
                manifest_filename: format!("sample-{index}.cake.json"),
                source_sha256: "a".repeat(64),
            });
            terminal.revision = 1;
            terminal.updated_at = if index == 0 {
                "2010-01-01T00:00:00.000Z".to_owned()
            } else {
                format!("2026-07-20T00:00:0{index}.000Z")
            };
            insert_raw_record(&store, &terminal);
        }
        store
            .save_preferences(&DesktopPreferences {
                maximum_terminal_history: 2,
                terminal_history_days: 3650,
                ..DesktopPreferences::default()
            })
            .unwrap();
        let records = store.list().unwrap();
        assert!(records.iter().any(|record| record.id == queued.id));
        assert_eq!(
            records
                .iter()
                .filter(|record| record.status.is_terminal())
                .count(),
            2
        );
    }

    #[test]
    fn schema_two_task_records_upgrade_to_the_current_scheduler_contract() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut legacy = sample_record(&store);
        legacy.schema_version = 2;
        legacy.scheduler_version = 0;
        legacy.queue_order = 0;
        legacy.transition(TaskStatus::Queued).unwrap();
        legacy.revision = 1;
        insert_raw_record(&store, &legacy);
        let upgraded = store.get(&legacy.id).unwrap();
        assert_eq!(upgraded.schema_version, TASK_STATE_SCHEMA_VERSION);
        assert_eq!(upgraded.scheduler_version, SCHEDULER_VERSION);
        assert_eq!(upgraded.queue_order, 1);
    }

    #[test]
    fn malformed_plan_and_oversized_metadata_fail_before_persistence() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut malformed = sample_record(&store);
        malformed.plan.slice_count = cakesplitter_format::MAX_SLICE_COUNT + 1;
        malformed.transition(TaskStatus::Queued).unwrap();
        assert!(matches!(
            store.insert(malformed),
            Err(StoreError::TaskPlanTooLarge)
        ));

        let mut oversized = sample_record(&store);
        oversized.display_name = "x".repeat(MAX_TASK_METADATA_BYTES);
        oversized.transition(TaskStatus::Queued).unwrap();
        assert!(matches!(
            store.insert(oversized),
            Err(StoreError::TaskMetadataTooLarge { .. })
        ));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn bootstrap_quarantines_one_corrupt_row_without_blocking_valid_work() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut valid = sample_record(&store);
        valid.transition(TaskStatus::Queued).unwrap();
        let valid = store.insert(valid).unwrap();
        let mut corrupt = sample_record(&store);
        corrupt.transition(TaskStatus::Queued).unwrap();
        let corrupt = store.insert(corrupt).unwrap();
        store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .execute(
                "UPDATE tasks SET checksum = 'invalid' WHERE id = ?1",
                params![corrupt.id],
            )
            .unwrap();

        let report = store.recover_after_restart().unwrap();
        assert_eq!(report.state, StartupRecoveryState::Corrupt);
        assert_eq!(report.quarantined_records, 1);
        let records = store.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, valid.id);
    }

    #[test]
    fn bootstrap_quarantines_unsupported_schema_and_invalid_transition_metadata() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut legacy = sample_record(&store);
        legacy.schema_version = TASK_STATE_SCHEMA_VERSION - 2;
        legacy.revision = 1;
        legacy.updated_at = now();
        insert_raw_record(&store, &legacy);
        let report = store.recover_after_restart().unwrap();
        assert_eq!(report.state, StartupRecoveryState::UnsupportedVersion);
        assert_eq!(report.quarantined_records, 1);

        store.clear_all().unwrap();
        let mut invalid = sample_record(&store);
        invalid.id = uuid::Uuid::new_v4().to_string();
        invalid.status = TaskStatus::Failed;
        invalid.failure = None;
        invalid.revision = 1;
        invalid.updated_at = now();
        insert_raw_record(&store, &invalid);
        let report = store.recover_after_restart().unwrap();
        assert_eq!(report.state, StartupRecoveryState::Quarantined);
        assert_eq!(report.quarantined_records, 1);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn task_primary_key_rejects_duplicate_ids_before_bootstrap() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let mut record = sample_record(&store);
        record.transition(TaskStatus::Queued).unwrap();
        let inserted = store.insert(record).unwrap();
        let duplicate = store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .execute(
                "INSERT INTO tasks (id, epoch, revision, status, updated_at, data_json, checksum) \
                 SELECT id, epoch, revision, status, updated_at, data_json, checksum FROM tasks \
                 WHERE id = ?1",
                params![inserted.id],
            );
        assert!(duplicate.is_err());
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn future_database_schema_is_preserved_and_recovery_ui_can_start_cleanly() {
        let root = tempdir().unwrap();
        let database = root.path().join("tasks.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection.pragma_update(None, "user_version", 999).unwrap();
        connection
            .execute_batch("CREATE TABLE future_marker (value TEXT); INSERT INTO future_marker VALUES ('preserve');")
            .unwrap();
        drop(connection);

        let store = TaskStore::open(root.path()).unwrap();
        assert_eq!(
            store.startup_recovery_report().state,
            StartupRecoveryState::UnsupportedVersion
        );
        let report = store.recover_after_restart().unwrap();
        assert_eq!(report.state, StartupRecoveryState::UnsupportedVersion);
        assert!(store.list().unwrap().is_empty());
        assert!(fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("tasks.sqlite3.unsupported-v999-")
        }));
    }

    fn insert_raw_record(store: &TaskStore, record: &TaskRecord) {
        let (json, checksum) = encode(record).unwrap();
        store
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .execute(
                "INSERT INTO tasks (id, epoch, revision, status, updated_at, data_json, checksum) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    record.id,
                    record.epoch,
                    record.revision,
                    status_name(record.status),
                    record.updated_at,
                    json,
                    checksum
                ],
            )
            .unwrap();
    }
}
