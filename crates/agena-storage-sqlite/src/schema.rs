//! Concrete SQLite table and index definitions for the shared Agena store.

use std::path::{Path, PathBuf};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement, TransactionTrait};

use crate::{CURRENT_SCHEMA_VERSION, install_invariant_triggers};

/// How long `initialize_schema` waits for a concurrent process to finish
/// building the schema before giving up.
const SCHEMA_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Serializes schema creation across processes.
///
/// SQLite's `PRAGMA journal_mode = WAL` needs an exclusive lock that the busy
/// timeout does not wait on, so two processes cold-starting the same database
/// file would otherwise race and one would fail with `SQLITE_BUSY`. A
/// filesystem lock on a sibling `.schema-lock` file serializes the whole
/// create path. In-memory databases have no backing file and skip the lock.
///
/// The lock is held for the lifetime of this guard: dropping it releases the
/// advisory file lock.
struct SchemaLock {
    // Held only so the file (and its lock) outlives the guard.
    _file: std::fs::File,
}

impl SchemaLock {
    async fn acquire(db: &DatabaseConnection) -> Result<Option<SchemaLock>, DbErr> {
        let Some(lock_path) = schema_lock_path(db).await? else {
            return Ok(None);
        };
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                DbErr::Custom(format!(
                    "open schema lock file {}: {error}",
                    lock_path.display()
                ))
            })?;
        let started = std::time::Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(SchemaLock { _file: file })),
                Err(_) if started.elapsed() < SCHEMA_LOCK_TIMEOUT => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(error) => {
                    return Err(DbErr::Custom(format!(
                        "timed out acquiring schema lock {}: {error}",
                        lock_path.display()
                    )));
                }
            }
        }
    }
}

/// Resolve the `<db>.schema-lock` path for a SQLite connection, or `None` for
/// in-memory databases. Uses `PRAGMA database_list` which reports the absolute
/// backing-file path of the main database.
async fn schema_lock_path(db: &DatabaseConnection) -> Result<Option<PathBuf>, DbErr> {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA database_list".to_owned(),
        ))
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let file: String = row.try_get("", "file")?;
    if file.is_empty() || file == ":memory:" {
        return Ok(None);
    }
    let path = Path::new(&file);
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".schema-lock");
    Ok(Some(PathBuf::from(lock_path)))
}

/// Creates the complete SQLite schema and applies its version marker atomically.
///
/// Serialized across processes by a filesystem lock so concurrent cold starts
/// of the same database file cannot race the WAL switch or the DDL transaction.
/// A version-0 database is created from scratch; a database already at
/// [`CURRENT_SCHEMA_VERSION`] is left untouched; anything else is rejected
/// (Agena no longer migrates older development schemas).
pub async fn initialize_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let _lock = SchemaLock::acquire(db).await?;
    // Connection hardening: WAL journal (no-op for in-memory databases),
    // bounded busy timeout, and NORMAL durability so the WAL checkpoint
    // window is small while every commit stays crash-safe.
    for pragma in [
        "PRAGMA journal_mode = WAL",
        "PRAGMA busy_timeout = 15000",
        "PRAGMA synchronous = NORMAL",
    ] {
        db.execute(Statement::from_string(
            db.get_database_backend(),
            pragma.to_owned(),
        ))
        .await?;
    }
    let current_version = read_schema_version(db).await?;
    match current_version {
        0 => {
            let txn = db.begin().await?;
            for statement in TABLES.iter().chain(INDEXES).chain(SEEDS) {
                txn.execute(Statement::from_string(
                    txn.get_database_backend(),
                    (*statement).to_owned(),
                ))
                .await?;
            }
            install_invariant_triggers(&txn).await?;
            txn.execute(Statement::from_string(
                txn.get_database_backend(),
                format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"),
            ))
            .await?;
            txn.commit().await
        }
        v if v == CURRENT_SCHEMA_VERSION => Ok(()),
        v => Err(DbErr::Custom(format!(
            "database schema version {v} is incompatible with the supported version {CURRENT_SCHEMA_VERSION}; \
             Agena does not migrate incompatible databases, so create a fresh database"
        ))),
    }
}

async fn read_schema_version(db: &DatabaseConnection) -> Result<i64, DbErr> {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "PRAGMA user_version".to_owned(),
        ))
        .await?
        .ok_or_else(|| DbErr::Custom("SQLite did not return user_version".to_owned()))?;
    row.try_get("", "user_version")
}

/// Seed rows for the database-backed sequence allocators. `next_val` is the
/// next value to hand out, so every allocator starts at 1.
const SEEDS: &[&str] = &[
    "INSERT OR IGNORE INTO agena_sequences (seq_name, next_val) VALUES ('seq_global', 1), ('message_id', 1), ('part_id', 1)",
];

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, parent_id INTEGER NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, depth INTEGER NOT NULL, root_id INTEGER NOT NULL, workspace_id INTEGER NOT NULL REFERENCES agena_workspaces(id) ON UPDATE CASCADE ON DELETE CASCADE, title TEXT NOT NULL, version INTEGER NOT NULL, lifecycle_state TEXT NOT NULL, creation_failure_json JSON NULL, runtime_state_json JSON NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_session_lineage (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, relation_kind TEXT NOT NULL, source_cutoff_seq_global INTEGER NULL, source_message_id INTEGER NULL, view_materialized_seq_global INTEGER NULL, task_id TEXT NULL, subtask_status TEXT NULL, subtask_started_at_ms INTEGER NULL, subtask_finished_at_ms INTEGER NULL, subtask_failure_json TEXT NULL, created_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_permission_rules (id INTEGER PRIMARY KEY AUTOINCREMENT, action_key TEXT NOT NULL, mode TEXT NOT NULL, scope TEXT NOT NULL, session_id INTEGER NULL, workspace_id INTEGER NULL, source TEXT NOT NULL, reason TEXT NULL, operator TEXT NULL, revoked_at_ms INTEGER NULL, revoked_reason TEXT NULL, revoked_by TEXT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_events (id INTEGER PRIMARY KEY AUTOINCREMENT, event_uuid TEXT NOT NULL UNIQUE, seq_global INTEGER NOT NULL UNIQUE, seq_session INTEGER NULL, session_id INTEGER NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, workspace_id INTEGER NULL REFERENCES agena_workspaces(id) ON UPDATE CASCADE ON DELETE CASCADE, kind_tag TEXT NOT NULL, envelope_schema INTEGER NOT NULL, payload_json JSON NOT NULL, causation_uuid TEXT NULL, correlation_uuid TEXT NULL, created_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_turns (turn_id TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, turn_seq INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, UNIQUE(session_id, turn_seq))",
    "CREATE TABLE IF NOT EXISTS agena_assistant_replies (reply_id TEXT PRIMARY KEY, turn_id TEXT NOT NULL UNIQUE REFERENCES agena_turns(turn_id) ON UPDATE CASCADE ON DELETE CASCADE, status TEXT NOT NULL, revision_seq INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, finished_at_ms INTEGER NULL, failure_json JSON NULL)",
    "CREATE TABLE IF NOT EXISTS agena_reply_executions (execution_id TEXT PRIMARY KEY, reply_id TEXT NOT NULL REFERENCES agena_assistant_replies(reply_id) ON UPDATE CASCADE ON DELETE CASCADE, source TEXT NOT NULL, status TEXT NOT NULL, revision_seq INTEGER NOT NULL, started_at_ms INTEGER NOT NULL, finished_at_ms INTEGER NULL)",
    "CREATE TABLE IF NOT EXISTS agena_content_nodes (node_id TEXT PRIMARY KEY, owner_kind TEXT NOT NULL, owner_id TEXT NOT NULL, node_type TEXT NOT NULL CHECK (node_type IN ('text','activity')), actor TEXT, title TEXT NULL, payload_json JSON, text TEXT, state TEXT NOT NULL, position INTEGER NOT NULL, revision_seq INTEGER NOT NULL, started_at_ms INTEGER NOT NULL, finished_at_ms INTEGER, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, UNIQUE (owner_kind, owner_id, position), CHECK ((node_type = 'text' AND actor IS NULL AND payload_json IS NULL AND text IS NOT NULL) OR (node_type = 'activity' AND actor IS NOT NULL AND text IS NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_model_messages (message_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, model_turn_id INTEGER NULL, execution_id TEXT NULL, run_id TEXT NULL, role INTEGER NOT NULL, state INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, metadata JSON NOT NULL, provider_state JSON NULL, usage JSON NULL, part_count INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_model_message_parts (part_id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES agena_model_messages(message_id) ON UPDATE CASCADE ON DELETE CASCADE, part_index INTEGER NOT NULL, status INTEGER NOT NULL, kind INTEGER NOT NULL, name TEXT NULL, summary TEXT NULL, has_detail BOOLEAN NOT NULL DEFAULT 0, awaits_user_reply BOOLEAN NOT NULL DEFAULT 0, activity_id TEXT NULL UNIQUE, segment_id TEXT NULL UNIQUE, operation_id TEXT NULL, created_at_ms INTEGER NOT NULL, content JSON NULL, CHECK ((activity_id IS NULL) OR (segment_id IS NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_session_messages (session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, message_id INTEGER NOT NULL REFERENCES agena_model_messages(message_id) ON UPDATE CASCADE ON DELETE CASCADE, PRIMARY KEY (session_id, message_id))",
    "CREATE TABLE IF NOT EXISTS agena_model_projection_states (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, last_seq_global INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, model_id TEXT NOT NULL, definition_json JSON NOT NULL, search_text TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_state (id INTEGER PRIMARY KEY, fetched_at_unix_ms INTEGER NULL, source TEXT NULL, last_error TEXT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json JSON NOT NULL, next_fire_at_ms INTEGER NULL, delivery_key TEXT NULL, claimed_at_ms INTEGER NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_scheduler_history (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, run_json JSON NOT NULL, finished_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_sequences (seq_name TEXT PRIMARY KEY, next_val INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_session_sequences (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, next_val INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_execution_leases (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, owner_id TEXT NOT NULL, run_id TEXT NULL, lease_started_at_ms INTEGER NOT NULL, heartbeat_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_user_message_idempotency (session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, idempotency_key TEXT NOT NULL, message_id INTEGER NOT NULL, PRIMARY KEY (session_id, idempotency_key))",
];

const INDEXES: &[&str] = &[
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_workspace_path ON agena_workspaces(path)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_parent_id ON agena_sessions(parent_id, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_root_id ON agena_sessions(root_id, depth, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_lineage_kind ON agena_session_lineage(relation_kind, session_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_lineage_task ON agena_session_lineage(task_id, session_id) WHERE task_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_workspace_id_updated ON agena_sessions(workspace_id, updated_at_ms, id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_global_subject ON agena_permission_rules(action_key, scope) WHERE session_id IS NULL AND workspace_id IS NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_workspace_subject ON agena_permission_rules(action_key, scope, workspace_id) WHERE session_id IS NULL AND workspace_id IS NOT NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_session_subject ON agena_permission_rules(action_key, scope, session_id) WHERE session_id IS NOT NULL AND workspace_id IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_permission_rule_active_updated ON agena_permission_rules(revoked_at_ms, updated_at_ms)",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_events_seq_global ON agena_events(seq_global)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_events_session_seq ON agena_events(session_id, seq_session)",
    "CREATE INDEX IF NOT EXISTS idx_agena_events_workspace_seq ON agena_events(workspace_id, seq_global)",
    "CREATE INDEX IF NOT EXISTS idx_agena_events_kind_seq ON agena_events(kind_tag, seq_global)",
    "CREATE INDEX IF NOT EXISTS idx_agena_turns_session_seq ON agena_turns(session_id, turn_seq)",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_agena_assistant_replies_turn ON agena_assistant_replies(turn_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_reply_executions_reply ON agena_reply_executions(reply_id, started_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_content_nodes_owner ON agena_content_nodes(owner_kind, owner_id, position)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_messages_session_created ON agena_model_messages(session_id, created_at_ms, message_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_messages_session_turn ON agena_model_messages(session_id, model_turn_id, message_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_messages_session_execution ON agena_model_messages(session_id, execution_id) WHERE execution_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_message_parts_message_index ON agena_model_message_parts(message_id, part_index)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_messages_message ON agena_session_messages(message_id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_model_catalog_kind_model ON agena_model_catalog_entries(kind, model_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_catalog_model_id ON agena_model_catalog_entries(model_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_catalog_kind ON agena_model_catalog_entries(kind)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_next_fire ON agena_scheduler_jobs(next_fire_at_ms, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_finished ON agena_scheduler_history(finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_job_finished ON agena_scheduler_history(job_id, finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_leases_heartbeat ON agena_execution_leases(heartbeat_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_leases_owner ON agena_execution_leases(owner_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_jobs_delivery ON agena_scheduler_jobs(delivery_key) WHERE delivery_key IS NOT NULL",
];

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement};

    use super::*;

    async fn initialized_database() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory SQLite");
        initialize_schema(&db).await.expect("initialize schema");
        execute(
            &db,
            "INSERT INTO agena_workspaces (id, path, created_at_ms, updated_at_ms) \
             VALUES (1, '/workspace', 1, 1)",
        )
        .await
        .expect("insert workspace");
        execute(
            &db,
            "INSERT INTO agena_sessions \
             (id, parent_id, depth, root_id, workspace_id, title, version, lifecycle_state, \
              creation_failure_json, runtime_state_json, created_at_ms, updated_at_ms) \
             VALUES (1, NULL, 0, 0, 1, 'session', 1, 'creating', NULL, NULL, 1, 1)",
        )
        .await
        .expect("insert session");
        execute(
            &db,
            "INSERT INTO agena_turns (turn_id, session_id, turn_seq, created_at_ms) \
             VALUES ('turn-1', 1, 1, 1)",
        )
        .await
        .expect("insert turn");
        execute(
            &db,
            "INSERT INTO agena_assistant_replies \
             (reply_id, turn_id, status, revision_seq, created_at_ms, finished_at_ms) \
             VALUES ('reply-1', 'turn-1', 'in_progress', 1, 1, NULL)",
        )
        .await
        .expect("insert assistant reply");
        execute(
            &db,
            "INSERT INTO agena_reply_executions \
             (execution_id, reply_id, source, status, revision_seq, started_at_ms, finished_at_ms) \
             VALUES ('execution-1', 'reply-1', 'user', 'in_progress', 1, 1, NULL)",
        )
        .await
        .expect("insert reply execution");
        db
    }

    async fn execute(db: &DatabaseConnection, sql: &str) -> Result<(), sea_orm::DbErr> {
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            sql.to_owned(),
        ))
        .await
        .map(|_| ())
    }

    async fn count(db: &DatabaseConnection, table: &str, predicate: &str) -> i64 {
        let row = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                format!("SELECT COUNT(*) AS count FROM {table} WHERE {predicate}"),
            ))
            .await
            .expect("count canonical rows")
            .expect("count row");
        row.try_get("", "count").expect("integer count")
    }

    #[tokio::test]
    async fn canonical_reply_and_execution_lifecycles_are_database_invariants() {
        let db = initialized_database().await;

        let duplicate_user = execute(
            &db,
            "INSERT INTO agena_reply_executions \
             (execution_id, reply_id, source, status, revision_seq, started_at_ms, finished_at_ms) \
             VALUES ('execution-duplicate-user', 'reply-1', 'user', 'in_progress', 2, 2, NULL)",
        )
        .await
        .expect_err("one reply cannot have two originating user executions");
        assert!(
            duplicate_user
                .to_string()
                .contains("invalid assistant reply execution")
        );

        execute(
            &db,
            "INSERT INTO agena_reply_executions \
             (execution_id, reply_id, source, status, revision_seq, started_at_ms, finished_at_ms) \
             VALUES ('execution-continue', 'reply-1', 'permission_reply', 'in_progress', 2, 2, NULL)",
        )
        .await
        .expect("continuation may attach after the originating user execution");
        execute(
            &db,
            "UPDATE agena_reply_executions \
             SET status = 'completed', revision_seq = 3, finished_at_ms = 3 \
             WHERE execution_id = 'execution-continue'",
        )
        .await
        .expect("terminalize continuation once");

        let reopen_execution = execute(
            &db,
            "UPDATE agena_reply_executions \
             SET status = 'in_progress', revision_seq = 4, finished_at_ms = NULL \
             WHERE execution_id = 'execution-continue'",
        )
        .await
        .expect_err("one execution cannot reopen after becoming terminal");
        assert!(
            reopen_execution
                .to_string()
                .contains("invalid assistant reply execution lifecycle")
        );

        execute(
            &db,
            "UPDATE agena_assistant_replies \
             SET status = 'completed', revision_seq = 3, finished_at_ms = 3 \
             WHERE reply_id = 'reply-1'",
        )
        .await
        .expect("terminalize assistant reply");
        execute(
            &db,
            "UPDATE agena_assistant_replies \
             SET status = 'in_progress', revision_seq = 4, finished_at_ms = NULL \
             WHERE reply_id = 'reply-1'",
        )
        .await
        .expect("a new continuation may reopen the shared assistant reply");

        let revision_regression = execute(
            &db,
            "UPDATE agena_assistant_replies SET revision_seq = 2 WHERE reply_id = 'reply-1'",
        )
        .await
        .expect_err("assistant reply revision cannot decrease");
        assert!(
            revision_regression
                .to_string()
                .contains("invalid assistant reply lifecycle")
        );

        execute(
            &db,
            "INSERT INTO agena_turns (turn_id, session_id, turn_seq, created_at_ms) \
             VALUES ('turn-2', 1, 2, 2)",
        )
        .await
        .expect("insert second canonical turn");
        execute(
            &db,
            "INSERT INTO agena_assistant_replies \
             (reply_id, turn_id, status, revision_seq, created_at_ms, finished_at_ms) \
             VALUES ('reply-without-origin', 'turn-2', 'in_progress', 1, 2, NULL)",
        )
        .await
        .expect("insert reply before its originating execution");
        let continuation_without_origin = execute(
            &db,
            "INSERT INTO agena_reply_executions \
             (execution_id, reply_id, source, status, revision_seq, started_at_ms, finished_at_ms) \
             VALUES ('execution-orphan-continuation', 'reply-without-origin', 'continue', 'in_progress', 1, 2, NULL)",
        )
        .await
        .expect_err("continuation requires an originating user execution");
        assert!(
            continuation_without_origin
                .to_string()
                .contains("invalid assistant reply execution")
        );
    }

    #[tokio::test]
    async fn content_nodes_enforce_owner_lifecycle_and_cascade_deletes() {
        let db = initialized_database().await;

        // Activity node under a missing owner is rejected.
        let error = execute(
            &db,
            "INSERT INTO agena_content_nodes \
             (node_id, owner_kind, owner_id, node_type, actor, payload_json, text, state, \
              position, revision_seq, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) \
             VALUES ('node-invalid', 'assistant_reply', 'missing-reply', 'activity', 'assistant', '{}', NULL, \
                     'completed', 0, 1, 1, 1, 1, 1)",
        )
        .await
        .expect_err("content node owner must exist");
        assert!(
            error
                .to_string()
                .contains("invalid content node owner or content position")
        );

        // Text node requires text and no actor; lifecycle rejects completed without finish.
        let error = execute(
            &db,
            "INSERT INTO agena_content_nodes \
             (node_id, owner_kind, owner_id, node_type, actor, payload_json, text, state, \
              position, revision_seq, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) \
             VALUES ('node-bad-lifecycle', 'assistant_reply', 'reply-1', 'activity', 'assistant', '{}', NULL, \
                     'completed', 0, 1, 1, NULL, 1, 1)",
        )
        .await
        .expect_err("completed activity node requires finished_at_ms");
        assert!(error.to_string().contains("invalid content node lifecycle"));

        // Text node may be completed without finished_at_ms.
        execute(
            &db,
            "INSERT INTO agena_content_nodes \
             (node_id, owner_kind, owner_id, node_type, actor, payload_json, text, state, \
              position, revision_seq, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) \
             VALUES ('node-text', 'assistant_reply', 'reply-1', 'text', NULL, NULL, 'hello', \
                     'completed', 0, 1, 1, NULL, 1, 1)",
        )
        .await
        .expect("text node lifecycle allows completed without finish");

        // Revision cannot decrease.
        let error = execute(
            &db,
            "UPDATE agena_content_nodes SET revision_seq = 0 WHERE node_id = 'node-text'",
        )
        .await
        .expect_err("revision cannot decrease");
        assert!(error.to_string().contains("revision cannot decrease"));

        // Deleting the reply cascades its content nodes.
        execute(
            &db,
            "DELETE FROM agena_assistant_replies WHERE reply_id = 'reply-1'",
        )
        .await
        .expect("delete reply");
        assert_eq!(
            count(&db, "agena_content_nodes", "owner_id = 'reply-1'").await,
            0
        );
    }
}
