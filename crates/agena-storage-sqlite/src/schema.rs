//! Infrastructure declarations shared with the single session-kernel schema.
use sea_orm::{DatabaseConnection, DbErr};
pub(crate) mod validation;

/// Initialize a new-model database or validate its exact durable declarations.
/// Existing incompatible databases are left untouched.
pub async fn initialize_schema(database: &DatabaseConnection) -> Result<(), DbErr> {
    crate::kernel::schema::initialize(database).await
}

pub(crate) const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_instance (id INTEGER PRIMARY KEY CHECK(id=1), instance_id TEXT NOT NULL UNIQUE CHECK(length(instance_id)=36), created_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_server_state (key TEXT PRIMARY KEY CHECK(length(key) BETWEEN 1 AND 200 AND key=trim(key)), value_json TEXT NOT NULL CHECK(json_valid(value_json)), updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_permission_rules (id INTEGER PRIMARY KEY AUTOINCREMENT, action_key TEXT NOT NULL, mode TEXT NOT NULL, scope TEXT NOT NULL, session_id INTEGER NULL, workspace_id INTEGER NULL, source TEXT NOT NULL, reason TEXT NULL, operator TEXT NULL, revoked_at_ms INTEGER NULL, revoked_reason TEXT NULL, revoked_by TEXT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_entries (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, model_id TEXT NOT NULL, definition_json JSON NOT NULL, search_text TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_state (id INTEGER PRIMARY KEY, fetched_at_unix_ms INTEGER NULL, source TEXT NULL, last_error TEXT NULL, updated_at_ms INTEGER NOT NULL)",
];

pub(crate) const INDEXES: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_agena_server_state_updated ON agena_server_state(updated_at_ms DESC)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_workspace_path ON agena_workspaces(path)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_global_subject ON agena_permission_rules(action_key, scope) WHERE session_id IS NULL AND workspace_id IS NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_workspace_subject ON agena_permission_rules(action_key, scope, workspace_id) WHERE session_id IS NULL AND workspace_id IS NOT NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_session_subject ON agena_permission_rules(action_key, scope, session_id) WHERE session_id IS NOT NULL AND workspace_id IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_permission_rule_active_updated ON agena_permission_rules(revoked_at_ms, updated_at_ms)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_model_catalog_kind_model ON agena_model_catalog_entries(kind, model_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_catalog_model_id ON agena_model_catalog_entries(model_id)",
    "CREATE INDEX IF NOT EXISTS idx_agena_model_catalog_kind ON agena_model_catalog_entries(kind)",
];
