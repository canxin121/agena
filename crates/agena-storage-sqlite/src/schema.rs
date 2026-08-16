//! Concrete SQLite table and index definitions for the v2 Agena store.
//!
//! Chat-data tables (`parts`, `session_parts`, `sessions`,
//! `execution_leases`, `sequences`, `workspaces`, `permission_rules`,
//! `usage`, `idempotency`, `background_operations`,
//! `background_deliveries`) plus the unchanged model-catalog infrastructure
//! tables. Parts remain the transcript entity; normalized background rows are
//! the runtime control plane and project their observable results into parts.
//!
//! The scheduler used to live here too; it now owns a dedicated SQLite
//! database and schema (`agena-scheduler::schema`), so this database has no
//! scheduler tables.

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

/// Creates the complete v2 SQLite schema and applies its version marker
/// atomically.
///
/// Serialized across processes by a filesystem lock so concurrent cold starts
/// of the same database file cannot race the WAL switch or the DDL transaction.
/// A version-0 database is created from scratch; a database already at
/// [`CURRENT_SCHEMA_VERSION`] is left untouched. Compatible v8, v9, and v10
/// schemas are migrated forward in one transaction; other development schemas are
/// rejected (v1 databases were discarded by decision D3).
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
        8 => migrate_v8_to_current(db).await,
        9 => migrate_v9_to_current(db).await,
        10 => migrate_v10_to_v11(db).await,
        v if v == CURRENT_SCHEMA_VERSION => Ok(()),
        v => Err(DbErr::Custom(format!(
            "database schema version {v} is incompatible with the supported version {CURRENT_SCHEMA_VERSION}; \
             Agena does not migrate incompatible databases, so create a fresh database"
        ))),
    }
}

/// Add the background-operation aggregate without rewriting any transcript
/// row. Existing v8 sessions remain readable; legacy in-progress markers are
/// adopted lazily by the runtime when first observed.
async fn migrate_v8_to_current(db: &DatabaseConnection) -> Result<(), DbErr> {
    let txn = db.begin().await?;
    for statement in BACKGROUND_TABLES.iter().chain(BACKGROUND_INDEXES) {
        txn.execute(Statement::from_string(
            txn.get_database_backend(),
            (*statement).to_owned(),
        ))
        .await?;
    }
    // Adopt every legacy marker that still has a usable external id. This is
    // intentionally data-preserving: transcript rows remain untouched while
    // runtime callbacks gain the normalized index immediately after upgrade.
    txn.execute(Statement::from_string(
        txn.get_database_backend(),
        "INSERT OR IGNORE INTO agena_background_operations \
         (operation_id, session_id, launch_run_id, launch_tool_part_id, kind, external_id, \
          phase, last_event_seq, revision, created_at_ms, updated_at_ms, finished_at_ms) \
         SELECT 'bg_' || p.origin_session_id || '_' || p.part_id, p.origin_session_id, p.run_id, \
                p.part_id, \
                json_extract(p.content, '$.operation.metadata.\"agena.background\".kind'), \
                json_extract(p.content, '$.operation.metadata.\"agena.background\".id'), \
                CASE p.state WHEN 'completed' THEN 'completed' WHEN 'failed' THEN 'failed' \
                     WHEN 'cancelled' THEN 'cancelled' WHEN 'pending' THEN 'launching' \
                     ELSE 'running' END, \
                0, 1, p.created_at_ms, p.updated_at_ms, \
                CASE WHEN p.state IN ('completed','failed','cancelled') \
                     THEN COALESCE(p.finished_at_ms, p.updated_at_ms) ELSE NULL END \
         FROM agena_parts p \
         WHERE p.kind = 'tool_call' AND p.run_id IS NOT NULL \
           AND json_extract(p.content, '$.operation.metadata.\"agena.background\".kind') \
               IN ('shell','task','monitor') \
           AND length(COALESCE(json_extract(p.content, \
               '$.operation.metadata.\"agena.background\".id'), '')) > 0"
            .to_owned(),
    ))
    .await?;
    add_session_metadata_columns(&txn).await?;
    txn.execute(Statement::from_string(
        txn.get_database_backend(),
        format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"),
    ))
    .await?;
    txn.commit().await
}

/// Rebuild the two background tables so scheduled deliveries can retain a
/// paired assistant launch run/tool reference. SQLite cannot alter a CHECK
/// constraint in place. Renaming both tables first preserves every foreign
/// key while the canonical names are recreated and copied transactionally.
async fn migrate_v9_to_current(db: &DatabaseConnection) -> Result<(), DbErr> {
    let txn = db.begin().await?;
    for statement in [
        "ALTER TABLE agena_background_deliveries RENAME TO agena_background_deliveries_v9",
        "ALTER TABLE agena_background_operations RENAME TO agena_background_operations_v9",
        BACKGROUND_TABLES[0],
        BACKGROUND_TABLES[1],
        "INSERT INTO agena_background_operations SELECT * FROM agena_background_operations_v9",
        "INSERT INTO agena_background_deliveries SELECT * FROM agena_background_deliveries_v9",
        "DROP TABLE agena_background_deliveries_v9",
        "DROP TABLE agena_background_operations_v9",
    ] {
        txn.execute(Statement::from_string(
            txn.get_database_backend(),
            statement.to_owned(),
        ))
        .await?;
    }
    for statement in BACKGROUND_INDEXES {
        txn.execute(Statement::from_string(
            txn.get_database_backend(),
            (*statement).to_owned(),
        ))
        .await?;
    }
    add_session_metadata_columns(&txn).await?;
    txn.execute(Statement::from_string(
        txn.get_database_backend(),
        format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"),
    ))
    .await?;
    txn.commit().await
}

/// Add the client-shared favorite/pinned flags without rewriting session
/// identity, lineage, or transcript data.
async fn migrate_v10_to_v11(db: &DatabaseConnection) -> Result<(), DbErr> {
    let txn = db.begin().await?;
    add_session_metadata_columns(&txn).await?;
    txn.execute(Statement::from_string(
        txn.get_database_backend(),
        format!("PRAGMA user_version = {CURRENT_SCHEMA_VERSION}"),
    ))
    .await?;
    txn.commit().await
}

async fn add_session_metadata_columns<C: ConnectionTrait>(connection: &C) -> Result<(), DbErr> {
    let existing = connection
        .query_all(Statement::from_string(
            connection.get_database_backend(),
            "PRAGMA table_info(agena_sessions)".to_owned(),
        ))
        .await?
        .into_iter()
        .map(|row| row.try_get::<String>("", "name"))
        .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
    for (name, statement) in [
        (
            "favorite",
            "ALTER TABLE agena_sessions ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1))",
        ),
        (
            "pinned",
            "ALTER TABLE agena_sessions ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1))",
        ),
    ] {
        if existing.contains(name) {
            continue;
        }
        connection
            .execute(Statement::from_string(
                connection.get_database_backend(),
                statement.to_owned(),
            ))
            .await?;
    }
    Ok(())
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

/// Seed rows for the database-backed part-id allocator. `next_val` is the next
/// value to hand out, so the allocator starts at 1. The `__agena_write_lock__`
/// row is the reserved write-lock sentinel used by every write transaction
/// (see `transaction.rs`).
const SEEDS: &[&str] = &[
    "INSERT OR IGNORE INTO agena_sequences (seq_name, next_val) VALUES ('part_id', 1)",
    "INSERT OR IGNORE INTO agena_sequences (seq_name, next_val) VALUES ('__agena_write_lock__', 1)",
];

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, parent_id INTEGER NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, depth INTEGER NOT NULL DEFAULT 0, root_id INTEGER NOT NULL DEFAULT 0, workspace_id INTEGER NOT NULL REFERENCES agena_workspaces(id) ON UPDATE CASCADE ON DELETE CASCADE, relation_kind TEXT NOT NULL DEFAULT 'root' CHECK (relation_kind IN ('root','child','fork','rewind','subagent')), is_subagent INTEGER NOT NULL GENERATED ALWAYS AS (relation_kind = 'subagent') STORED, cutoff_part_id INTEGER NULL, title TEXT NOT NULL, favorite INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)), pinned INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)), version INTEGER NOT NULL, lifecycle_state TEXT NOT NULL DEFAULT 'creating' CHECK (lifecycle_state IN ('creating','ready','failed')), creation_failure_json JSON NULL, task_id TEXT NULL, subtask_status TEXT NULL, subtask_started_at_ms INTEGER NULL, subtask_finished_at_ms INTEGER NULL, subtask_failure_json JSON NULL, config_json JSON NULL, provider_anchors_json JSON NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_parts (part_id INTEGER PRIMARY KEY, kind TEXT NOT NULL, role TEXT NOT NULL CHECK (role IN ('user','assistant','system','tool','runtime')), state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending','in_progress','completed','failed','cancelled')), content JSON NOT NULL, summary TEXT NULL, visibility TEXT NOT NULL DEFAULT 'both' CHECK (visibility IN ('both','user','ai')), rendered_markdown TEXT NULL, parent_part_id INTEGER NULL REFERENCES agena_parts(part_id), run_id INTEGER NULL REFERENCES agena_parts(part_id), origin_session_id INTEGER NULL, revision INTEGER NOT NULL DEFAULT 1, started_at_ms INTEGER NOT NULL, finished_at_ms INTEGER NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, provider_state JSON NULL, CHECK (finished_at_ms IS NULL OR finished_at_ms >= started_at_ms), CHECK ((state IN ('pending','in_progress') AND finished_at_ms IS NULL) OR (state IN ('completed','failed','cancelled') AND finished_at_ms IS NOT NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_session_parts (session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, part_id INTEGER NOT NULL REFERENCES agena_parts(part_id), added_at_ms INTEGER NOT NULL, PRIMARY KEY (session_id, part_id))",
    "CREATE TABLE IF NOT EXISTS agena_execution_leases (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, owner_id TEXT NOT NULL, run_id INTEGER NULL, lease_started_at_ms INTEGER NOT NULL, heartbeat_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_sequences (seq_name TEXT PRIMARY KEY, next_val INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_permission_rules (id INTEGER PRIMARY KEY AUTOINCREMENT, action_key TEXT NOT NULL, mode TEXT NOT NULL, scope TEXT NOT NULL, session_id INTEGER NULL, workspace_id INTEGER NULL, source TEXT NOT NULL, reason TEXT NULL, operator TEXT NULL, revoked_at_ms INTEGER NULL, revoked_reason TEXT NULL, revoked_by TEXT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_usage (usage_id INTEGER PRIMARY KEY AUTOINCREMENT, workspace_id INTEGER NOT NULL, session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, run_id INTEGER NULL, provider_id TEXT NOT NULL, model_id TEXT NOT NULL, created_at_ms INTEGER NOT NULL, input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL DEFAULT 0, cache_write_tokens INTEGER NOT NULL DEFAULT 0, cache_read_tokens INTEGER NOT NULL DEFAULT 0, tool_use_tokens INTEGER NOT NULL DEFAULT 0, other_tokens INTEGER NOT NULL DEFAULT 0, total_cost_micros INTEGER NOT NULL DEFAULT 0, recorded_cost_micros INTEGER NULL, cost_estimate_incomplete INTEGER NOT NULL DEFAULT 0, detail_json JSON NULL)",
    "CREATE TABLE IF NOT EXISTS agena_idempotency (session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, idempotency_key TEXT NOT NULL, run_id INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, PRIMARY KEY (session_id, idempotency_key))",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, model_id TEXT NOT NULL, definition_json JSON NOT NULL, search_text TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_state (id INTEGER PRIMARY KEY, fetched_at_unix_ms INTEGER NULL, source TEXT NULL, last_error TEXT NULL, updated_at_ms INTEGER NOT NULL)",
    BACKGROUND_TABLES[0],
    BACKGROUND_TABLES[1],
];

const BACKGROUND_TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_background_operations (operation_id TEXT PRIMARY KEY CHECK (length(operation_id) > 0), session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, launch_run_id INTEGER NULL REFERENCES agena_parts(part_id), launch_tool_part_id INTEGER NULL REFERENCES agena_parts(part_id), kind TEXT NOT NULL CHECK (kind IN ('shell','task','monitor','scheduled_delivery')), external_id TEXT NULL CHECK (external_id IS NULL OR length(external_id) > 0), phase TEXT NOT NULL CHECK (phase IN ('launch_requested','launching','running','completed','failed','cancelled','timed_out','interrupted')), outcome_json JSON NULL, failure_json JSON NULL, last_event_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_event_seq >= 0), owner_id TEXT NULL, lease_until_ms INTEGER NULL, revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1), created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, finished_at_ms INTEGER NULL, CHECK ((launch_run_id IS NULL AND launch_tool_part_id IS NULL) OR (launch_run_id IS NOT NULL AND launch_tool_part_id IS NOT NULL)), CHECK (kind = 'scheduled_delivery' OR (launch_run_id IS NOT NULL AND launch_tool_part_id IS NOT NULL)), CHECK ((phase IN ('launch_requested','launching','running') AND finished_at_ms IS NULL) OR (phase IN ('completed','failed','cancelled','timed_out','interrupted') AND finished_at_ms IS NOT NULL)), CHECK ((owner_id IS NULL AND lease_until_ms IS NULL) OR (owner_id IS NOT NULL AND lease_until_ms IS NOT NULL)), CHECK (outcome_json IS NULL OR json_valid(outcome_json) = 1), CHECK (failure_json IS NULL OR json_valid(failure_json) = 1))",
    "CREATE TABLE IF NOT EXISTS agena_background_deliveries (delivery_id TEXT PRIMARY KEY CHECK (length(delivery_id) > 0), operation_id TEXT NOT NULL REFERENCES agena_background_operations(operation_id) ON UPDATE CASCADE ON DELETE CASCADE, session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, event_key TEXT NOT NULL CHECK (length(event_key) > 0), payload_json JSON NOT NULL CHECK (json_valid(payload_json) = 1), phase TEXT NOT NULL CHECK (phase IN ('pending','claimed','consumed')), claim_owner TEXT NULL, claim_until_ms INTEGER NULL, attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0), notification_part_id INTEGER NULL REFERENCES agena_parts(part_id), last_error_json JSON NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, consumed_at_ms INTEGER NULL, UNIQUE(operation_id, event_key), CHECK (last_error_json IS NULL OR json_valid(last_error_json) = 1), CHECK ((phase = 'claimed' AND claim_owner IS NOT NULL AND claim_until_ms IS NOT NULL) OR (phase != 'claimed' AND claim_owner IS NULL AND claim_until_ms IS NULL)), CHECK ((phase = 'consumed' AND consumed_at_ms IS NOT NULL) OR (phase != 'consumed' AND consumed_at_ms IS NULL)))",
];

const INDEXES: &[&str] = &[
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_workspace_path ON agena_workspaces(path)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_parent_id ON agena_sessions(parent_id, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_root_id ON agena_sessions(root_id, updated_at_ms, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_workspace_id_updated ON agena_sessions(workspace_id, updated_at_ms, id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_session_parent_task ON agena_sessions(parent_id, task_id) WHERE task_id IS NOT NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_parts_parent ON agena_parts(parent_part_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_parts_run ON agena_parts(run_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_parts_origin ON agena_parts(origin_session_id, created_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_parts_recover ON agena_parts(kind, state)",
    "CREATE INDEX IF NOT EXISTS idx_agena_parts_origin_state ON agena_parts(origin_session_id, state)",
    "CREATE INDEX IF NOT EXISTS idx_agena_session_parts_part ON agena_session_parts(part_id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_global_subject ON agena_permission_rules(action_key, scope) WHERE session_id IS NULL AND workspace_id IS NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_workspace_subject ON agena_permission_rules(action_key, scope, workspace_id) WHERE session_id IS NULL AND workspace_id IS NOT NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_session_subject ON agena_permission_rules(action_key, scope, session_id) WHERE session_id IS NOT NULL AND workspace_id IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_permission_rule_active_updated ON agena_permission_rules(revoked_at_ms, updated_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_usage_ws_time ON agena_usage(workspace_id, created_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_usage_session ON agena_usage(session_id, created_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_usage_provider_model ON agena_usage(provider_id, model_id, created_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_leases_heartbeat ON agena_execution_leases(heartbeat_at_ms)",
    "CREATE INDEX IF NOT EXISTS idx_agena_execution_leases_owner ON agena_execution_leases(owner_id)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_model_catalog_kind_model ON agena_model_catalog_entries(kind, model_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_catalog_model_id ON agena_model_catalog_entries(model_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_catalog_kind ON agena_model_catalog_entries(kind)",
    BACKGROUND_INDEXES[0],
    BACKGROUND_INDEXES[1],
    BACKGROUND_INDEXES[2],
];

const BACKGROUND_INDEXES: &[&str] = &[
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_background_external ON agena_background_operations(kind, external_id) WHERE external_id IS NOT NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_background_launch_part ON agena_background_operations(session_id, launch_tool_part_id) WHERE launch_tool_part_id IS NOT NULL AND kind != 'scheduled_delivery'",
    "CREATE INDEX IF NOT EXISTS idx_agena_background_delivery_pending ON agena_background_deliveries(phase, claim_until_ms, created_at_ms)",
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
             (id, parent_id, depth, root_id, workspace_id, relation_kind, title, version, \
              lifecycle_state, created_at_ms, updated_at_ms) \
             VALUES (1, NULL, 0, 0, 1, 'root', 'session', 1, 'ready', 1, 1)",
        )
        .await
        .expect("insert session");
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

    /// Insert a run marker and one content part under it, in the canonical
    /// v2 shape (run marker is the root of its batch; content part points at
    /// the marker via `run_id`).
    async fn seed_parts(db: &DatabaseConnection) {
        execute(
            db,
            "INSERT INTO agena_parts \
             (part_id, kind, role, state, content, origin_session_id, started_at_ms, \
              created_at_ms, updated_at_ms) \
             VALUES (1, 'run', 'user', 'in_progress', \
                     '{\"run_kind\":\"user_send\"}', 1, 1, 1, 1)",
        )
        .await
        .expect("insert run marker");
        execute(
            db,
            "INSERT INTO agena_parts \
             (part_id, kind, role, state, content, run_id, origin_session_id, started_at_ms, \
              finished_at_ms, created_at_ms, updated_at_ms) \
             VALUES (2, 'text', 'user', 'completed', '{\"text\":\"hello\"}', 1, 1, 1, 2, 2, 2)",
        )
        .await
        .expect("insert content part");
        execute(
            db,
            "INSERT INTO agena_session_parts (session_id, part_id, added_at_ms) \
             VALUES (1, 1, 1), (1, 2, 2)",
        )
        .await
        .expect("insert membership edges");
    }

    #[tokio::test]
    async fn v2_schema_initializes_with_the_normalized_background_tables() {
        let db = initialized_database().await;
        // Assert the complete Agena-owned table set, not only positive
        // existence of the nine chat tables. Any historical chat table or
        // accidental dead-data table makes this exact-set assertion fail.
        let expected = std::collections::BTreeSet::from(
            [
                "agena_parts",
                "agena_session_parts",
                "agena_sessions",
                "agena_execution_leases",
                "agena_sequences",
                "agena_workspaces",
                "agena_permission_rules",
                "agena_usage",
                "agena_idempotency",
                "agena_model_catalog_entries",
                "agena_model_catalog_state",
                "agena_background_operations",
                "agena_background_deliveries",
            ]
            .map(str::to_owned),
        );
        let rows = db
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'agena_%' ORDER BY name"
                    .to_owned(),
            ))
            .await
            .expect("list Agena-owned tables");
        let actual = rows
            .into_iter()
            .map(|row| row.try_get::<String>("", "name").expect("table name"))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);

        let session_columns = db
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA table_info(agena_sessions)".to_owned(),
            ))
            .await
            .expect("list session columns")
            .into_iter()
            .map(|row| row.try_get::<String>("", "name").expect("column name"))
            .collect::<std::collections::BTreeSet<_>>();
        assert!(session_columns.contains("favorite"));
        assert!(session_columns.contains("pinned"));
    }

    #[tokio::test]
    async fn v10_migration_adds_defaulted_session_metadata_flags() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory SQLite");
        execute(
            &db,
            "CREATE TABLE agena_sessions (id INTEGER PRIMARY KEY, title TEXT NOT NULL)",
        )
        .await
        .expect("create v10 session fixture");
        execute(
            &db,
            "INSERT INTO agena_sessions (id, title) VALUES (1, 'existing')",
        )
        .await
        .expect("insert existing session");
        execute(&db, "PRAGMA user_version = 10")
            .await
            .expect("set v10 marker");

        initialize_schema(&db).await.expect("migrate v10 to v11");

        let row = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT favorite, pinned FROM agena_sessions WHERE id = 1".to_owned(),
            ))
            .await
            .expect("read migrated flags")
            .expect("existing session remains");
        assert!(!row.try_get::<bool>("", "favorite").expect("favorite"));
        assert!(!row.try_get::<bool>("", "pinned").expect("pinned"));
        let version: i64 = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA user_version".to_owned(),
            ))
            .await
            .expect("read migrated version")
            .expect("version row")
            .try_get("", "user_version")
            .expect("version");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn v8_migration_adds_background_tables_and_backfills_legacy_markers() {
        // Start from the compatible v8 base shape by creating the current
        // non-destructive schema, removing only v9's additive tables, and
        // restoring the old version marker. This keeps every pre-v9 invariant
        // trigger in the fixture without duplicating the full historical DDL.
        let db = initialized_database().await;
        execute(&db, "DROP TABLE agena_background_deliveries")
            .await
            .expect("remove v9 deliveries");
        execute(&db, "DROP TABLE agena_background_operations")
            .await
            .expect("remove v9 operations");
        execute(&db, "PRAGMA user_version = 8")
            .await
            .expect("restore v8 marker");

        execute(
            &db,
            "INSERT INTO agena_parts \
             (part_id, kind, role, state, content, origin_session_id, started_at_ms, \
              finished_at_ms, created_at_ms, updated_at_ms) \
             VALUES (10, 'run', 'assistant', 'completed', \
                     '{\"run_kind\":\"continue\",\"abort_reason\":null}', 1, 10, 11, 10, 11)",
        )
        .await
        .expect("insert legacy assistant run");
        execute(
            &db,
            "INSERT INTO agena_parts \
             (part_id, kind, role, state, content, run_id, origin_session_id, started_at_ms, \
              created_at_ms, updated_at_ms) \
             VALUES (11, 'tool_call', 'assistant', 'in_progress', \
                     '{\"operation\":{\"metadata\":{\"agena.background\":{\"kind\":\"shell\",\"id\":\"proc_legacy\"}}}}', \
                     10, 1, 10, 10, 11)",
        )
        .await
        .expect("insert legacy background marker");
        execute(
            &db,
            "INSERT INTO agena_session_parts (session_id, part_id, added_at_ms) \
             VALUES (1, 10, 10), (1, 11, 11)",
        )
        .await
        .expect("insert legacy membership");

        initialize_schema(&db).await.expect("migrate v8 to v9");
        let version: i64 = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA user_version".to_owned(),
            ))
            .await
            .expect("query version")
            .expect("version row")
            .try_get("", "user_version")
            .expect("version value");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        let row = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT operation_id, session_id, launch_run_id, launch_tool_part_id, kind, \
                        external_id, phase \
                 FROM agena_background_operations WHERE external_id = 'proc_legacy'"
                    .to_owned(),
            ))
            .await
            .expect("query backfilled operation")
            .expect("legacy marker backfilled");
        assert_eq!(
            row.try_get::<String>("", "operation_id").unwrap(),
            "bg_1_11"
        );
        assert_eq!(row.try_get::<i64>("", "session_id").unwrap(), 1);
        assert_eq!(row.try_get::<i64>("", "launch_run_id").unwrap(), 10);
        assert_eq!(row.try_get::<i64>("", "launch_tool_part_id").unwrap(), 11);
        assert_eq!(row.try_get::<String>("", "kind").unwrap(), "shell");
        assert_eq!(row.try_get::<String>("", "phase").unwrap(), "running");
    }

    #[tokio::test]
    async fn v9_migration_preserves_deliveries_and_allows_scheduled_launch_provenance() {
        let db = initialized_database().await;
        execute(&db, "PRAGMA foreign_keys = ON")
            .await
            .expect("enable foreign keys");
        execute(&db, "DROP TABLE agena_background_deliveries")
            .await
            .expect("drop current deliveries");
        execute(&db, "DROP TABLE agena_background_operations")
            .await
            .expect("drop current operations");
        execute(
            &db,
            "CREATE TABLE agena_background_operations (operation_id TEXT PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), launch_run_id INTEGER NULL REFERENCES agena_parts(part_id), launch_tool_part_id INTEGER NULL REFERENCES agena_parts(part_id), kind TEXT NOT NULL, external_id TEXT NULL, phase TEXT NOT NULL, outcome_json JSON NULL, failure_json JSON NULL, last_event_seq INTEGER NOT NULL DEFAULT 0, owner_id TEXT NULL, lease_until_ms INTEGER NULL, revision INTEGER NOT NULL DEFAULT 1, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, finished_at_ms INTEGER NULL, CHECK ((kind = 'scheduled_delivery' AND launch_run_id IS NULL AND launch_tool_part_id IS NULL) OR (kind != 'scheduled_delivery' AND launch_run_id IS NOT NULL AND launch_tool_part_id IS NOT NULL)))",
        )
        .await
        .expect("create v9 operations");
        execute(
            &db,
            "CREATE TABLE agena_background_deliveries (delivery_id TEXT PRIMARY KEY, operation_id TEXT NOT NULL REFERENCES agena_background_operations(operation_id) ON DELETE CASCADE, session_id INTEGER NOT NULL REFERENCES agena_sessions(id), event_key TEXT NOT NULL, payload_json JSON NOT NULL, phase TEXT NOT NULL, claim_owner TEXT NULL, claim_until_ms INTEGER NULL, attempts INTEGER NOT NULL DEFAULT 0, notification_part_id INTEGER NULL REFERENCES agena_parts(part_id), last_error_json JSON NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, consumed_at_ms INTEGER NULL, UNIQUE(operation_id, event_key))",
        )
        .await
        .expect("create v9 deliveries");
        execute(
            &db,
            "CREATE UNIQUE INDEX uq_agena_background_external ON agena_background_operations(kind, external_id) WHERE external_id IS NOT NULL",
        )
        .await
        .expect("create v9 external index");
        execute(
            &db,
            "CREATE UNIQUE INDEX uq_agena_background_launch_part ON agena_background_operations(session_id, launch_tool_part_id) WHERE launch_tool_part_id IS NOT NULL",
        )
        .await
        .expect("create v9 launch index");
        execute(
            &db,
            "CREATE INDEX idx_agena_background_delivery_pending ON agena_background_deliveries(phase, claim_until_ms, created_at_ms)",
        )
        .await
        .expect("create v9 delivery index");
        execute(
            &db,
            "INSERT INTO agena_background_operations (operation_id, session_id, kind, external_id, phase, last_event_seq, revision, created_at_ms, updated_at_ms) VALUES ('scheduled:1:legacy-fire', 1, 'scheduled_delivery', 'legacy-fire', 'running', 0, 2, 10, 11)",
        )
        .await
        .expect("insert v9 operation");
        execute(
            &db,
            "INSERT INTO agena_background_deliveries (delivery_id, operation_id, session_id, event_key, payload_json, phase, attempts, created_at_ms, updated_at_ms) VALUES ('delivery:legacy', 'scheduled:1:legacy-fire', 1, 'fire', '{}', 'pending', 0, 11, 11)",
        )
        .await
        .expect("insert v9 delivery");
        execute(
            &db,
            "INSERT INTO agena_parts (part_id, kind, role, state, content, origin_session_id, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) VALUES (20, 'run', 'assistant', 'completed', '{\"run_kind\":\"continue\",\"abort_reason\":null}', 1, 20, 20, 20, 20)",
        )
        .await
        .expect("insert assistant run");
        execute(
            &db,
            "INSERT INTO agena_parts (part_id, kind, role, state, content, run_id, origin_session_id, started_at_ms, finished_at_ms, created_at_ms, updated_at_ms) VALUES (21, 'tool_call', 'assistant', 'completed', '{\"operation\":{\"call_id\":19}}', 20, 1, 20, 21, 20, 21)",
        )
        .await
        .expect("insert cron tool receipt");
        execute(
            &db,
            "INSERT INTO agena_session_parts (session_id, part_id, added_at_ms) VALUES (1, 20, 20), (1, 21, 21)",
        )
        .await
        .expect("insert part membership");
        execute(&db, "PRAGMA user_version = 9")
            .await
            .expect("set v9 marker");

        initialize_schema(&db).await.expect("migrate v9 to v10");
        let delivery_count: i64 = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT count(*) AS count FROM agena_background_deliveries WHERE delivery_id = 'delivery:legacy'".to_owned(),
            ))
            .await
            .expect("query delivery")
            .expect("count row")
            .try_get("", "count")
            .expect("count");
        assert_eq!(delivery_count, 1, "the v9 outbox row is preserved");
        execute(
            &db,
            "INSERT INTO agena_background_operations (operation_id, session_id, launch_run_id, launch_tool_part_id, kind, phase, last_event_seq, revision, created_at_ms, updated_at_ms) VALUES ('scheduled:1:assistant-fire', 1, 20, 21, 'scheduled_delivery', 'running', 0, 1, 30, 30)",
        )
        .await
        .expect("v10 accepts assistant-owned scheduled delivery");
        let foreign_key_errors = db
            .query_all(Statement::from_string(
                DatabaseBackend::Sqlite,
                "PRAGMA foreign_key_check".to_owned(),
            ))
            .await
            .expect("foreign key check");
        assert!(foreign_key_errors.is_empty());
    }

    #[tokio::test]
    async fn part_lifecycle_and_identity_invariants_are_database_enforced() {
        let db = initialized_database().await;
        seed_parts(&db).await;

        // Terminal run marker requires an abort reason.
        let missing_abort = execute(
            &db,
            "UPDATE agena_parts SET state = 'failed', finished_at_ms = 5 \
             WHERE part_id = 1",
        )
        .await
        .expect_err("terminal run marker needs abort_reason");
        assert!(missing_abort.to_string().contains("abort_reason"));

        // With an abort reason the transition is accepted.
        execute(
            &db,
            "UPDATE agena_parts \
             SET state = 'failed', finished_at_ms = 5, \
                 content = json_set(content, '$.abort_reason', 'lease_stolen') \
             WHERE part_id = 1",
        )
        .await
        .expect("terminal run marker with abort reason");

        // Identity columns are immutable.
        let identity = execute(
            &db,
            "UPDATE agena_parts SET role = 'assistant' WHERE part_id = 2",
        )
        .await
        .expect_err("part role is immutable");
        assert!(identity.to_string().contains("immutable"));

        // Retry: failed → in_progress clears finished_at and bumps revision.
        execute(
            &db,
            "UPDATE agena_parts SET state = 'failed', revision = 2, finished_at_ms = 5, \
             updated_at_ms = 5 WHERE part_id = 2",
        )
        .await
        .expect("fail content part");
        let retry_without_bump = execute(
            &db,
            "UPDATE agena_parts SET state = 'in_progress', finished_at_ms = NULL, \
             updated_at_ms = 6 WHERE part_id = 2",
        )
        .await
        .expect_err("retry must bump revision");
        assert!(retry_without_bump.to_string().contains("retry"));
        execute(
            &db,
            "UPDATE agena_parts SET state = 'in_progress', revision = 3, \
             finished_at_ms = NULL, updated_at_ms = 6 WHERE part_id = 2",
        )
        .await
        .expect("retry with revision bump");
    }

    #[tokio::test]
    async fn session_delete_cascades_membership_but_not_parts() {
        let db = initialized_database().await;
        seed_parts(&db).await;
        execute(&db, "DELETE FROM agena_sessions WHERE id = 1")
            .await
            .expect("delete session");
        // Membership edges are gone; parts survive (shared across forks).
        let edges: i64 = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS count FROM agena_session_parts".to_owned(),
            ))
            .await
            .expect("count edges")
            .expect("count row")
            .try_get("", "count")
            .expect("count value");
        assert_eq!(edges, 0);
        let parts: i64 = db
            .query_one(Statement::from_string(
                DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS count FROM agena_parts".to_owned(),
            ))
            .await
            .expect("count parts")
            .expect("count row")
            .try_get("", "count")
            .expect("count value");
        assert_eq!(parts, 2, "parts must survive a session delete");
    }
}
