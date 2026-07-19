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
    MAX_NONTERMINAL_TASKS, MAX_QUARANTINE_DATA_BYTES, MAX_QUARANTINE_REASON_BYTES,
    MAX_QUARANTINE_RECORDS, MAX_RECOVERY_RECORDS, MAX_TASK_HISTORY, MAX_TASK_METADATA_BYTES,
    TASK_STATE_SCHEMA_VERSION,
    model::{DesktopPreferences, TaskRecord, TaskStatus, now},
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
    #[error("startup recovery found {actual} nonterminal tasks; maximum is {maximum}")]
    RecoveryCapacityExceeded { actual: usize, maximum: usize },
    #[error("task metadata is {actual} bytes; maximum is {maximum} bytes")]
    TaskMetadataTooLarge { actual: usize, maximum: usize },
    #[error("task plan exceeds the supported Slice bound")]
    TaskPlanTooLarge,
}

pub struct TaskStore {
    connection: Mutex<Connection>,
    _writer_lock: File,
    database_path: PathBuf,
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
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        migrate(&mut connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
            _writer_lock: writer_lock,
            database_path,
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
        record.revision = 1;
        record.updated_at = now();
        let (json, checksum) = encode(&record)?;
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
        for row in rows {
            match decode_row(&row) {
                Ok(record) => records.push(record),
                Err(reason) => quarantine(&transaction, &row, &reason)?,
            }
        }
        transaction.commit()?;
        Ok(records)
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
        change(&mut record)?;
        if was_terminal
            && !record.status.is_terminal()
            && count_nonterminal(&transaction)? >= MAX_NONTERMINAL_TASKS
        {
            return Err(StoreError::QueueCapacityReached {
                maximum: MAX_NONTERMINAL_TASKS,
            });
        }
        record.revision = record
            .revision
            .checked_add(1)
            .ok_or(StoreError::CorruptState)?;
        record.updated_at = now();
        let (json, checksum) = encode(&record)?;
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
        transaction.execute(
            "UPDATE metadata SET epoch = ?1 WHERE singleton = 1",
            params![next_epoch],
        )?;
        transaction.commit()?;
        Ok(next_epoch)
    }

    pub fn recover_after_restart(&self) -> Result<usize, StoreError> {
        {
            let mut connection = self
                .connection
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let transaction = connection.transaction()?;
            let actual = count_nonterminal(&transaction)?;
            if actual > MAX_RECOVERY_RECORDS {
                return Err(StoreError::RecoveryCapacityExceeded {
                    actual,
                    maximum: MAX_RECOVERY_RECORDS,
                });
            }
            prune_history(&transaction)?;
            transaction.commit()?;
        }
        let records = self.list()?;
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
        Ok(recovered)
    }

    pub fn preferences(&self) -> Result<DesktopPreferences, StoreError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
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
        Ok(serde_json::from_str(&json)?)
    }

    pub fn save_preferences(
        &self,
        preferences: &DesktopPreferences,
    ) -> Result<DesktopPreferences, StoreError> {
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
             INSERT INTO metadata (singleton, schema_version, epoch) VALUES (1, 1, 1);\
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

fn current_epoch(connection: &Connection) -> Result<u64, StoreError> {
    connection
        .query_row(
            "SELECT epoch FROM metadata WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(StoreError::from)
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

fn decode_row(row: &StoredRow) -> Result<TaskRecord, String> {
    if row.data_json.len() > MAX_TASK_METADATA_BYTES {
        return Err("task metadata exceeds the supported limit".to_owned());
    }
    let expected = format!("{:x}", Sha256::digest(row.data_json.as_bytes()));
    if expected != row.checksum {
        return Err("checksum mismatch".to_owned());
    }
    let record: TaskRecord =
        serde_json::from_str(&row.data_json).map_err(|error| error.to_string())?;
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
        return Err("indexed task metadata does not match the checksummed record".to_owned());
    }
    Ok(record)
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
    transaction.execute(
        "DELETE FROM tasks WHERE id IN ( \
             SELECT id FROM tasks \
             WHERE status IN ('cancelled', 'failed', 'completed') \
             ORDER BY updated_at DESC, id DESC \
             LIMIT -1 OFFSET ?1 \
         )",
        params![MAX_TASK_HISTORY as u64],
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

    use tempfile::tempdir;

    use super::*;
    use crate::model::{ProcessingPlan, TaskSpec};

    fn sample_record(store: &TaskStore) -> TaskRecord {
        TaskRecord::new(
            "0.4.0-dev",
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
        let old_epoch = inserted.epoch;
        let new_epoch = store.clear_all().unwrap();
        assert!(new_epoch > old_epoch);
        assert!(matches!(
            store.mutate(&inserted.id, old_epoch, |_| Ok(())),
            Err(StoreError::StaleEpoch { .. })
        ));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn settings_are_checksummed_and_survive_task_history_clear() {
        let root = tempdir().unwrap();
        let store = TaskStore::open(root.path()).unwrap();
        let preferences = DesktopPreferences {
            default_slice_size: 64 * 1024 * 1024,
            confirm_destructive_actions: false,
            reduce_motion: true,
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
        assert_eq!(store.recover_after_restart().unwrap(), 1);
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
        assert_eq!(store.recover_after_restart().unwrap(), 0);

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
        assert!(matches!(
            store.recover_after_restart(),
            Err(StoreError::RecoveryCapacityExceeded {
                actual,
                maximum: MAX_RECOVERY_RECORDS
            }) if actual == MAX_RECOVERY_RECORDS + 1
        ));
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
}
