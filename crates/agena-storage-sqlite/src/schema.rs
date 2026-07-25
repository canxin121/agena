//! Concrete SQLite table and index definitions for the shared Agena store.

use sea_orm::{ConnectionTrait, DatabaseConnection, DbErr, Statement};

use crate::{
    begin_schema_initialization, complete_schema_initialization, install_invariant_triggers,
};

/// Creates the complete SQLite schema and applies its version marker atomically.
pub async fn initialize_schema(db: &DatabaseConnection) -> Result<(), DbErr> {
    let (txn, current_version) = begin_schema_initialization(db).await?;
    for statement in TABLES.iter().chain(INDEXES) {
        txn.execute(Statement::from_string(
            txn.get_database_backend(),
            (*statement).to_owned(),
        ))
        .await?;
    }
    install_invariant_triggers(&txn).await?;
    complete_schema_initialization(txn, current_version).await
}

const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_sessions (id INTEGER PRIMARY KEY AUTOINCREMENT, parent_id INTEGER NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, depth INTEGER NOT NULL, root_id INTEGER NOT NULL, workspace_id INTEGER NOT NULL REFERENCES agena_workspaces(id) ON UPDATE CASCADE ON DELETE CASCADE, title TEXT NOT NULL, version INTEGER NOT NULL, lifecycle_state TEXT NOT NULL, creation_error TEXT NULL, runtime_state_json JSON NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_session_lineage (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, relation_kind TEXT NOT NULL, source_cutoff_seq_global INTEGER NULL, source_message_id INTEGER NULL, task_id TEXT NULL, subtask_status TEXT NULL, subtask_started_at_ms INTEGER NULL, subtask_finished_at_ms INTEGER NULL, subtask_error TEXT NULL, created_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_permission_rules (id INTEGER PRIMARY KEY AUTOINCREMENT, action_key TEXT NOT NULL, mode TEXT NOT NULL, scope TEXT NOT NULL, session_id INTEGER NULL, workspace_id INTEGER NULL, source TEXT NOT NULL, reason TEXT NULL, operator TEXT NULL, revoked_at_ms INTEGER NULL, revoked_reason TEXT NULL, revoked_by TEXT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_events (id INTEGER PRIMARY KEY AUTOINCREMENT, event_uuid TEXT NOT NULL UNIQUE, seq_global INTEGER NOT NULL UNIQUE, seq_session INTEGER NULL, session_id INTEGER NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, workspace_id INTEGER NULL REFERENCES agena_workspaces(id) ON UPDATE CASCADE ON DELETE CASCADE, kind_tag TEXT NOT NULL, envelope_schema INTEGER NOT NULL, payload_json JSON NOT NULL, causation_uuid TEXT NULL, correlation_uuid TEXT NULL, created_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_activity_messages (message_id INTEGER PRIMARY KEY, session_id INTEGER NOT NULL REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, turn_id INTEGER NULL, execution_id TEXT NULL, run_id TEXT NULL, role INTEGER NOT NULL, state INTEGER NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, metadata JSON NOT NULL, provider_state JSON NULL, usage JSON NULL, part_count INTEGER NOT NULL, is_hidden BOOLEAN NOT NULL DEFAULT 0)",
    "CREATE TABLE IF NOT EXISTS agena_activity_parts (part_id INTEGER PRIMARY KEY, message_id INTEGER NOT NULL REFERENCES agena_activity_messages(message_id) ON UPDATE CASCADE ON DELETE CASCADE, part_index INTEGER NOT NULL, status INTEGER NOT NULL, kind INTEGER NOT NULL, name TEXT NULL, summary TEXT NULL, has_detail BOOLEAN NOT NULL DEFAULT 0, operation_id TEXT NULL, created_at_ms INTEGER NOT NULL, content JSON NULL)",
    "CREATE TABLE IF NOT EXISTS agena_activity_projection_states (session_id INTEGER PRIMARY KEY REFERENCES agena_sessions(id) ON UPDATE CASCADE ON DELETE CASCADE, last_seq_global INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, model_id TEXT NOT NULL, definition_json JSON NOT NULL, search_text TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_state (id INTEGER PRIMARY KEY, fetched_at_unix_ms INTEGER NULL, source TEXT NULL, last_error TEXT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_scheduler_jobs (id TEXT PRIMARY KEY, job_json JSON NOT NULL, next_fire_at_ms INTEGER NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_scheduler_history (id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL, run_json JSON NOT NULL, finished_at_ms INTEGER NOT NULL)",
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
    "CREATE INDEX IF NOT EXISTS idx_agena_activity_messages_session_created ON agena_activity_messages(session_id, created_at_ms, message_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_activity_messages_session_turn ON agena_activity_messages(session_id, turn_id, message_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_activity_messages_session_hidden ON agena_activity_messages(session_id, is_hidden, created_at_ms, message_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_activity_parts_message_index ON agena_activity_parts(message_id, part_index)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_activity_parts_operation_identity ON agena_activity_parts(message_id, kind, operation_id) WHERE operation_id IS NOT NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_model_catalog_kind_model ON agena_model_catalog_entries(kind, model_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_catalog_model_id ON agena_model_catalog_entries(model_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_catalog_kind ON agena_model_catalog_entries(kind)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_next_fire ON agena_scheduler_jobs(next_fire_at_ms, id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_finished ON agena_scheduler_history(finished_at_ms DESC, id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_scheduler_history_job_finished ON agena_scheduler_history(job_id, finished_at_ms DESC, id DESC)",
];
