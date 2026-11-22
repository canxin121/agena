//! Infrastructure declarations shared with the single session-kernel schema.
pub(crate) mod validation;

/// Identity of this complete persistence and payload contract. This is not an
/// upgrade counter: databases from another baseline must use another data root.
pub(crate) const SCHEMA_BASELINE: &str = "agena-2026-11-22.10";

pub(crate) fn instance_table() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS agena_instance (
            id INTEGER PRIMARY KEY CHECK(id=1),
            schema_baseline TEXT NOT NULL CHECK(schema_baseline='{SCHEMA_BASELINE}'),
            instance_id TEXT NOT NULL CHECK(length(instance_id)=36),
            created_at_ms INTEGER NOT NULL,
            catalog_fetched_at_ms INTEGER,
            catalog_source TEXT,
            catalog_digest BLOB,
            CHECK(
                (catalog_fetched_at_ms IS NULL AND catalog_source IS NULL AND catalog_digest IS NULL)
                OR (catalog_fetched_at_ms IS NOT NULL AND catalog_source IS NOT NULL
                    AND catalog_digest IS NOT NULL AND length(catalog_digest)=32)
            )
        ) STRICT"
    )
}

pub(crate) const TABLES: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS agena_server_state (key TEXT PRIMARY KEY CHECK(length(key) BETWEEN 1 AND 200 AND key=trim(key)), value_json TEXT NOT NULL CHECK(json_valid(value_json)))",
    "CREATE TABLE IF NOT EXISTS agena_workspaces (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS agena_permission_rules (id INTEGER PRIMARY KEY AUTOINCREMENT, action_key TEXT NOT NULL, mode TEXT NOT NULL, scope TEXT NOT NULL, session_id INTEGER NULL, workspace_id INTEGER NULL, source TEXT NOT NULL, reason TEXT NULL, operator TEXT NULL, revoked_at_ms INTEGER NULL, revoked_reason TEXT NULL, revoked_by TEXT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL, CHECK((scope='global' AND session_id IS NULL AND workspace_id IS NULL) OR (scope='workspace' AND session_id IS NULL AND workspace_id IS NOT NULL) OR (scope='session' AND session_id IS NOT NULL AND workspace_id IS NULL)))",
    "CREATE TABLE IF NOT EXISTS agena_model_catalog_entries (model_id TEXT PRIMARY KEY, definition_json TEXT NOT NULL CHECK(json_valid(definition_json))) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_bot_local_sessions (bot_id TEXT PRIMARY KEY REFERENCES agena_bots(id), session_id INTEGER UNIQUE REFERENCES agena_sessions(id) ON DELETE SET NULL) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_bot_avatars (bot_id TEXT PRIMARY KEY REFERENCES agena_bots(id) ON DELETE CASCADE, avatar TEXT NOT NULL CHECK(length(CAST(avatar AS BLOB)) BETWEEN 23 AND 262144 AND substr(avatar,1,22)='data:image/png;base64,')) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_bot_prompts (bot_id TEXT PRIMARY KEY REFERENCES agena_bots(id) ON DELETE CASCADE, ids_json TEXT NOT NULL CHECK(json_valid(ids_json) AND json_type(ids_json)='array'), publish_to_channels INTEGER NOT NULL CHECK(publish_to_channels IN (0,1))) WITHOUT ROWID",
    "CREATE TABLE IF NOT EXISTS agena_assistant_global_profile (singleton INTEGER NOT NULL PRIMARY KEY CHECK(singleton=1), revision INTEGER NOT NULL CHECK(revision>0), profile_json TEXT NOT NULL CHECK(json_valid(profile_json)))",
];

pub(crate) const INDEXES: &[&str] = &[
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_workspace_path ON agena_workspaces(path)",
    "CREATE INDEX IF NOT EXISTS idx_agena_workspace_recent ON agena_workspaces(updated_at_ms DESC,id DESC)",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_global_subject ON agena_permission_rules(scope,action_key) WHERE session_id IS NULL AND workspace_id IS NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_workspace_subject ON agena_permission_rules(workspace_id,scope,action_key) WHERE session_id IS NULL AND workspace_id IS NOT NULL",
    "CREATE UNIQUE INDEX IF NOT EXISTS uq_agena_permission_rule_session_subject ON agena_permission_rules(session_id,scope,action_key) WHERE session_id IS NOT NULL AND workspace_id IS NULL",
    "CREATE INDEX IF NOT EXISTS idx_agena_permission_rule_updated ON agena_permission_rules(updated_at_ms DESC,id DESC)",
    "CREATE INDEX IF NOT EXISTS idx_agena_permission_rule_workspace_updated ON agena_permission_rules(workspace_id,updated_at_ms DESC,id DESC) WHERE scope='workspace'",
];
