use std::sync::Arc;

use agena_domain::{PermissionMode, PermissionScope};
use agena_storage::{
    PermissionRuleListQuery, PermissionRuleRecord, PermissionRuleRepository,
    PermissionRuleRepositoryError, PermissionRuleTransactionWriter, PersistedPermissionRule,
};
use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, Statement, Value,
};

const TABLE: &str = "agena_permission_rules";
const COLUMNS: &str = "id, action_key, mode, scope, session_id, workspace_id, source, reason, operator, revoked_at_ms, revoked_reason, revoked_by, created_at_ms, updated_at_ms";

/// SQLite implementation of the application-facing permission-rule repository.
pub struct SeaPermissionRuleRepository {
    db: Arc<DatabaseConnection>,
}

/// Transaction-scoped SQLite permission writer. Session orchestration supplies
/// the transaction so permission and session-row updates remain atomic.
pub struct SeaPermissionRuleTransactionWriter;

impl SeaPermissionRuleTransactionWriter {
    pub async fn upsert_in_transaction(
        txn: &DatabaseTransaction,
        rule: &PersistedPermissionRule,
    ) -> Result<(PermissionRuleRecord, bool), PermissionRuleRepositoryError> {
        upsert_rule(txn, rule).await
    }
}

#[async_trait]
impl PermissionRuleTransactionWriter<DatabaseTransaction> for SeaPermissionRuleTransactionWriter {
    async fn upsert_in_transaction(
        &self,
        txn: &DatabaseTransaction,
        rule: &PersistedPermissionRule,
    ) -> Result<(PermissionRuleRecord, bool), PermissionRuleRepositoryError> {
        Self::upsert_in_transaction(txn, rule).await
    }
}

impl SeaPermissionRuleRepository {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }

    async fn record(
        &self,
        id: i64,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError> {
        self.db
            .query_one(statement(
                format!("SELECT {COLUMNS} FROM {TABLE} WHERE id = ?"),
                [id.into()],
            ))
            .await
            .map_err(map_error)?
            .map(record_from_row)
            .transpose()
    }
}

#[async_trait]
impl PermissionRuleRepository for SeaPermissionRuleRepository {
    async fn list(
        &self,
        query: PermissionRuleListQuery,
    ) -> Result<Vec<PermissionRuleRecord>, PermissionRuleRepositoryError> {
        let mut clauses = Vec::new();
        let mut values = Vec::<Value>::new();
        if let Some(search) = query.search.filter(|value| !value.is_empty()) {
            clauses.push("action_key LIKE ?");
            values.push(format!("%{search}%").into());
        }
        if let (Some(updated), Some(id)) = (query.before_updated_at_ms, query.before_id) {
            clauses.push("(updated_at_ms < ? OR (updated_at_ms = ? AND id < ?))");
            values.extend([updated.into(), updated.into(), id.into()]);
        }
        values.push(query.limit.into());
        let where_clause = if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        };
        self.db.query_all(statement(format!("SELECT {COLUMNS} FROM {TABLE}{where_clause} ORDER BY updated_at_ms DESC, id DESC LIMIT ?"), values)).await
            .map_err(map_error)?.into_iter().map(record_from_row).collect()
    }

    async fn get(
        &self,
        rule_id: i64,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError> {
        self.record(rule_id).await
    }

    async fn upsert(
        &self,
        rule: &PersistedPermissionRule,
    ) -> Result<(PermissionRuleRecord, bool), PermissionRuleRepositoryError> {
        upsert_rule(self.db.as_ref(), rule).await
    }

    async fn replace(
        &self,
        rule_id: i64,
        rule: &PersistedPermissionRule,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError> {
        if self.record(rule_id).await?.is_none() {
            return Ok(None);
        }
        self.db.execute(statement(
            format!("UPDATE {TABLE} SET action_key = ?, mode = ?, scope = ?, session_id = ?, workspace_id = ?, source = ?, reason = ?, operator = ?, revoked_at_ms = ?, revoked_reason = ?, revoked_by = ?, updated_at_ms = ? WHERE id = ?"),
            [rule.action_key.clone().into(), mode_to_string(rule.mode).into(), scope_to_string(rule.scope).into(), rule.session_id.into(), rule.workspace_id.into(), rule.source.clone().into(), rule.reason.clone().into(), rule.operator.clone().into(), rule.revoked_at_ms.into(), rule.revoked_reason.clone().into(), rule.revoked_by.clone().into(), Utc::now().timestamp_millis().into(), rule_id.into()],
        )).await.map_err(map_error)?;
        self.record(rule_id).await
    }

    async fn revoke(
        &self,
        rule_id: i64,
        revoked_reason: Option<String>,
        revoked_by: Option<String>,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError> {
        if self.record(rule_id).await?.is_none() {
            return Ok(None);
        }
        self.db.execute(statement(format!("UPDATE {TABLE} SET revoked_at_ms = ?, revoked_reason = ?, revoked_by = ?, updated_at_ms = ? WHERE id = ?"), [Utc::now().timestamp_millis().into(), revoked_reason.into(), revoked_by.into(), Utc::now().timestamp_millis().into(), rule_id.into()])).await.map_err(map_error)?;
        self.record(rule_id).await
    }

    async fn delete(
        &self,
        rule_id: i64,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError> {
        let existing = self.record(rule_id).await?;
        if existing.is_some() {
            self.db
                .execute(statement(
                    format!("DELETE FROM {TABLE} WHERE id = ?"),
                    [rule_id.into()],
                ))
                .await
                .map_err(map_error)?;
        }
        Ok(existing)
    }

    async fn resolve(
        &self,
        action_key: &str,
        session_id: Option<i64>,
        workspace_id: Option<i64>,
    ) -> Result<Vec<PersistedPermissionRule>, PermissionRuleRepositoryError> {
        let mut result = Vec::new();
        for (scope, session_id, workspace_id) in [
            ("global", None, None),
            ("workspace", None, workspace_id),
            ("session", session_id, None),
        ] {
            if scope != "global" && session_id.is_none() && workspace_id.is_none() {
                continue;
            }
            let row = self.db.query_one(statement(
                format!("SELECT {COLUMNS} FROM {TABLE} WHERE action_key = ? AND scope = ? AND revoked_at_ms IS NULL AND session_id IS ? AND workspace_id IS ? ORDER BY updated_at_ms DESC, id DESC LIMIT 1"),
                [action_key.to_owned().into(), scope.to_owned().into(), session_id.into(), workspace_id.into()],
            )).await.map_err(map_error)?;
            if let Some(row) = row {
                result.push(persisted_rule_from_row(row)?);
            }
        }
        Ok(result)
    }

    async fn resolve_snapshot(
        &self,
        session_id: Option<i64>,
        workspace_id: Option<i64>,
    ) -> Result<Vec<PersistedPermissionRule>, PermissionRuleRepositoryError> {
        let rows = self
            .db
            .query_all(statement(
                format!(
                    "SELECT {COLUMNS} FROM {TABLE} WHERE revoked_at_ms IS NULL AND (scope = 'global' OR (scope = 'workspace' AND workspace_id IS ?) OR (scope = 'session' AND session_id IS ?)) ORDER BY action_key, scope, updated_at_ms DESC, id DESC"
                ),
                [workspace_id.into(), session_id.into()],
            ))
            .await
            .map_err(map_error)?;
        let mut rules = Vec::with_capacity(rows.len());
        for row in rows {
            rules.push(persisted_rule_from_row(row)?);
        }
        Ok(rules)
    }
}

fn statement(sql: String, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
fn missing_row() -> PermissionRuleRepositoryError {
    PermissionRuleRepositoryError::Backend("permission rule row is missing after write".to_owned())
}

/// Atomic upsert of a permission rule against its partial unique index.
///
/// The schema enforces subject uniqueness with three partial unique indexes
/// (`schema.rs`), so both the conflict target and the follow-up update's
/// `WHERE` must match the index's partial clause verbatim.
///
/// `created` is deterministic, not timestamp-derived: `INSERT ... ON CONFLICT
/// ... DO NOTHING RETURNING id` returns a row only when the insert actually
/// happened, and no row when the subject already existed (in which case the
/// existing row is updated). Under SQLite's single-writer model concurrent
/// processes serialize at the write lock, so this never races into a
/// unique-constraint error.
async fn upsert_rule<C: ConnectionTrait>(
    db: &C,
    rule: &PersistedPermissionRule,
) -> Result<(PermissionRuleRecord, bool), PermissionRuleRepositoryError> {
    let scope = scope_to_string(rule.scope);
    let now = Utc::now().timestamp_millis();
    let (insert_sql, update_sql, values) = match (rule.session_id, rule.workspace_id) {
        (None, None) => (
            format!(
                "INSERT INTO {TABLE} (action_key, mode, scope, session_id, workspace_id, source, reason, operator, revoked_at_ms, revoked_reason, revoked_by, created_at_ms, updated_at_ms) \
                 VALUES (?, ?, ?, NULL, NULL, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(action_key, scope) WHERE session_id IS NULL AND workspace_id IS NULL \
                 DO NOTHING RETURNING id"
            ),
            format!(
                "UPDATE {TABLE} SET mode = ?, source = ?, reason = ?, operator = ?, revoked_at_ms = ?, \
                 revoked_reason = ?, revoked_by = ?, updated_at_ms = ? \
                 WHERE action_key = ? AND scope = ? AND session_id IS NULL AND workspace_id IS NULL RETURNING id"
            ),
            rule_values(rule, &scope, now),
        ),
        (None, Some(workspace_id)) => (
            format!(
                "INSERT INTO {TABLE} (action_key, mode, scope, session_id, workspace_id, source, reason, operator, revoked_at_ms, revoked_reason, revoked_by, created_at_ms, updated_at_ms) \
                 VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(action_key, scope, workspace_id) WHERE session_id IS NULL AND workspace_id IS NOT NULL \
                 DO NOTHING RETURNING id"
            ),
            format!(
                "UPDATE {TABLE} SET mode = ?, source = ?, reason = ?, operator = ?, revoked_at_ms = ?, \
                 revoked_reason = ?, revoked_by = ?, updated_at_ms = ? \
                 WHERE action_key = ? AND scope = ? AND session_id IS NULL AND workspace_id = ? RETURNING id"
            ),
            rule_values_with_workspace(rule, &scope, workspace_id, now),
        ),
        (Some(session_id), None) => (
            format!(
                "INSERT INTO {TABLE} (action_key, mode, scope, session_id, workspace_id, source, reason, operator, revoked_at_ms, revoked_reason, revoked_by, created_at_ms, updated_at_ms) \
                 VALUES (?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(action_key, scope, session_id) WHERE session_id IS NOT NULL AND workspace_id IS NULL \
                 DO NOTHING RETURNING id"
            ),
            format!(
                "UPDATE {TABLE} SET mode = ?, source = ?, reason = ?, operator = ?, revoked_at_ms = ?, \
                 revoked_reason = ?, revoked_by = ?, updated_at_ms = ? \
                 WHERE action_key = ? AND scope = ? AND session_id = ? AND workspace_id IS NULL RETURNING id"
            ),
            rule_values_with_session(rule, &scope, session_id, now),
        ),
        (Some(_), Some(_)) => {
            return Err(PermissionRuleRepositoryError::Backend(
                "permission rule cannot target both a session and a workspace".to_owned(),
            ));
        }
    };

    // 1) Try the insert. Returns a row only when this call created the row.
    let inserted = db
        .query_one(statement(insert_sql, values))
        .await
        .map_err(map_error)?;
    let (id, created) = if let Some(row) = inserted {
        (row.try_get::<i64>("", "id").map_err(map_error)?, true)
    } else {
        // 2) Subject already exists: update it in place (created = false).
        //    `UPDATE ... RETURNING id` yields the row's id in one statement.
        let update_values = update_rule_values(rule, &scope, now)?;
        let id = db
            .query_one(statement(update_sql, update_values))
            .await
            .map_err(map_error)?
            .and_then(|row| row.try_get("", "id").ok())
            .ok_or_else(missing_row)?;
        (id, false)
    };

    let record = db
        .query_one(statement(
            format!("SELECT {COLUMNS} FROM {TABLE} WHERE id = ?"),
            [id.into()],
        ))
        .await
        .map_err(map_error)?
        .map(record_from_row)
        .transpose()?
        .ok_or_else(missing_row)?;
    Ok((record, created))
}

/// UPDATE bindings in the statement's placeholder order: `mode, source,
/// reason, operator, revoked_at_ms, revoked_reason, revoked_by, updated_at_ms,
/// action_key, scope, [subject]`. The two subject arms each append one value;
/// the global arm has none.
fn update_rule_values(
    rule: &PersistedPermissionRule,
    scope: &str,
    now: i64,
) -> Result<Vec<Value>, PermissionRuleRepositoryError> {
    let mut values = vec![
        mode_to_string(rule.mode).into(),
        rule.source.clone().into(),
        rule.reason.clone().into(),
        rule.operator.clone().into(),
        rule.revoked_at_ms.into(),
        rule.revoked_reason.clone().into(),
        rule.revoked_by.clone().into(),
        now.into(),
        rule.action_key.clone().into(),
        scope.to_owned().into(),
    ];
    match (rule.session_id, rule.workspace_id) {
        (None, None) => {}
        (None, Some(workspace_id)) => values.push(workspace_id.into()),
        (Some(session_id), None) => values.push(session_id.into()),
        (Some(_), Some(_)) => {
            return Err(PermissionRuleRepositoryError::Backend(
                "permission rule cannot target both a session and a workspace".to_owned(),
            ));
        }
    }
    Ok(values)
}

/// Bind values for a global-scope rule (`session_id` and `workspace_id` are
/// both `NULL` in the SQL).
fn rule_values(rule: &PersistedPermissionRule, scope: &str, now: i64) -> Vec<Value> {
    common_rule_values(rule, scope, now)
}

fn rule_values_with_workspace(
    rule: &PersistedPermissionRule,
    scope: &str,
    workspace_id: i64,
    now: i64,
) -> Vec<Value> {
    let mut values = common_rule_values(rule, scope, now);
    values.insert(3, workspace_id.into());
    values
}

fn rule_values_with_session(
    rule: &PersistedPermissionRule,
    scope: &str,
    session_id: i64,
    now: i64,
) -> Vec<Value> {
    let mut values = common_rule_values(rule, scope, now);
    values.insert(3, session_id.into());
    values
}

/// `action_key, mode, scope, source, reason, operator, revoked_at_ms,
/// revoked_reason, revoked_by, created_at_ms, updated_at_ms` — the columns
/// that precede the optional subject placeholders.
fn common_rule_values(rule: &PersistedPermissionRule, scope: &str, now: i64) -> Vec<Value> {
    vec![
        rule.action_key.clone().into(),
        mode_to_string(rule.mode).into(),
        scope.to_owned().into(),
        rule.source.clone().into(),
        rule.reason.clone().into(),
        rule.operator.clone().into(),
        rule.revoked_at_ms.into(),
        rule.revoked_reason.clone().into(),
        rule.revoked_by.clone().into(),
        now.into(),
        now.into(),
    ]
}
fn record_from_row(
    row: sea_orm::QueryResult,
) -> Result<PermissionRuleRecord, PermissionRuleRepositoryError> {
    Ok(PermissionRuleRecord {
        id: row.try_get("", "id").map_err(map_error)?,
        action_key: row.try_get("", "action_key").map_err(map_error)?,
        mode: row.try_get("", "mode").map_err(map_error)?,
        scope: row.try_get("", "scope").map_err(map_error)?,
        session_id: row.try_get("", "session_id").map_err(map_error)?,
        workspace_id: row.try_get("", "workspace_id").map_err(map_error)?,
        source: row.try_get("", "source").map_err(map_error)?,
        reason: row.try_get("", "reason").map_err(map_error)?,
        operator: row.try_get("", "operator").map_err(map_error)?,
        revoked_at_ms: row.try_get("", "revoked_at_ms").map_err(map_error)?,
        revoked_reason: row.try_get("", "revoked_reason").map_err(map_error)?,
        revoked_by: row.try_get("", "revoked_by").map_err(map_error)?,
        created_at_ms: row.try_get("", "created_at_ms").map_err(map_error)?,
        updated_at_ms: row.try_get("", "updated_at_ms").map_err(map_error)?,
    })
}
fn persisted_rule_from_row(
    row: sea_orm::QueryResult,
) -> Result<PersistedPermissionRule, PermissionRuleRepositoryError> {
    let record = record_from_row(row)?;
    Ok(PersistedPermissionRule {
        id: Some(record.id),
        created_at_ms: Some(record.created_at_ms),
        updated_at_ms: Some(record.updated_at_ms),
        action_key: record.action_key,
        mode: mode_from_string(&record.mode).ok_or_else(|| {
            PermissionRuleRepositoryError::Backend(format!(
                "invalid permission mode in persisted rule {}",
                record.id
            ))
        })?,
        scope: scope_from_string(&record.scope).ok_or_else(|| {
            PermissionRuleRepositoryError::Backend(format!(
                "invalid permission scope in persisted rule {}",
                record.id
            ))
        })?,
        session_id: record.session_id,
        workspace_id: record.workspace_id,
        source: record.source,
        reason: record.reason,
        operator: record.operator,
        revoked_at_ms: record.revoked_at_ms,
        revoked_reason: record.revoked_reason,
        revoked_by: record.revoked_by,
    })
}
fn mode_to_string(mode: PermissionMode) -> String {
    match mode {
        PermissionMode::Allow => "allow",
        PermissionMode::Auto => "auto",
        PermissionMode::Ask => "ask",
        PermissionMode::Deny => "deny",
    }
    .to_owned()
}
fn mode_from_string(value: &str) -> Option<PermissionMode> {
    match value {
        "allow" => Some(PermissionMode::Allow),
        "auto" => Some(PermissionMode::Auto),
        "ask" => Some(PermissionMode::Ask),
        "deny" => Some(PermissionMode::Deny),
        _ => None,
    }
}
fn scope_to_string(scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => "session",
        PermissionScope::Workspace => "workspace",
        PermissionScope::Global => "global",
    }
    .to_owned()
}
fn scope_from_string(value: &str) -> Option<PermissionScope> {
    match value {
        "session" => Some(PermissionScope::Session),
        "workspace" => Some(PermissionScope::Workspace),
        "global" => Some(PermissionScope::Global),
        _ => None,
    }
}
fn map_error(error: impl std::fmt::Display) -> PermissionRuleRepositoryError {
    PermissionRuleRepositoryError::Backend(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    async fn repository() -> SeaPermissionRuleRepository {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        db.execute(Statement::from_string(
            DatabaseBackend::Sqlite,
            format!(
                "CREATE TABLE {TABLE} (id INTEGER PRIMARY KEY AUTOINCREMENT, action_key TEXT NOT NULL, mode TEXT NOT NULL, scope TEXT NOT NULL, session_id INTEGER NULL, workspace_id INTEGER NULL, source TEXT NOT NULL, reason TEXT NULL, operator TEXT NULL, revoked_at_ms INTEGER NULL, revoked_reason TEXT NULL, revoked_by TEXT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL)"
            ),
        ))
        .await
        .expect("create permission rule fixture");
        // Mirror the three partial unique indexes from the real schema so the
        // ON CONFLICT upsert clauses resolve to a matching constraint.
        for sql in [
            format!("CREATE UNIQUE INDEX uq_rule_global ON {TABLE}(action_key, scope) WHERE session_id IS NULL AND workspace_id IS NULL"),
            format!("CREATE UNIQUE INDEX uq_rule_workspace ON {TABLE}(action_key, scope, workspace_id) WHERE session_id IS NULL AND workspace_id IS NOT NULL"),
            format!("CREATE UNIQUE INDEX uq_rule_session ON {TABLE}(action_key, scope, session_id) WHERE session_id IS NOT NULL AND workspace_id IS NULL"),
        ] {
            db.execute(Statement::from_string(DatabaseBackend::Sqlite, sql))
                .await
                .expect("create permission rule fixture index");
        }
        SeaPermissionRuleRepository::new(Arc::new(db))
    }

    fn rule(
        scope: PermissionScope,
        session_id: Option<i64>,
        workspace_id: Option<i64>,
    ) -> PersistedPermissionRule {
        PersistedPermissionRule {
            id: None,
            created_at_ms: None,
            updated_at_ms: None,
            action_key: "shell.execute".to_owned(),
            mode: PermissionMode::Ask,
            scope,
            session_id,
            workspace_id,
            source: "test".to_owned(),
            reason: Some("fixture".to_owned()),
            operator: None,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        }
    }

    #[tokio::test]
    async fn upserts_and_resolves_scope_precedence() {
        let repository = repository().await;
        let global = repository
            .upsert(&rule(PermissionScope::Global, None, None))
            .await
            .expect("global rule");
        assert!(global.1);
        let mut replacement = rule(PermissionScope::Global, None, None);
        replacement.mode = PermissionMode::Deny;
        let replaced = repository
            .upsert(&replacement)
            .await
            .expect("replace global");
        assert_eq!(replaced.0.id, global.0.id);
        assert!(!replaced.1);
        repository
            .upsert(&rule(PermissionScope::Workspace, None, Some(9)))
            .await
            .expect("workspace rule");
        repository
            .upsert(&rule(PermissionScope::Session, Some(7), None))
            .await
            .expect("session rule");
        let resolved = repository
            .resolve("shell.execute", Some(7), Some(9))
            .await
            .expect("resolve");
        assert_eq!(resolved.len(), 3);
        assert_eq!(resolved[0].mode, PermissionMode::Deny);
        assert_eq!(resolved[1].scope, PermissionScope::Workspace);
        assert_eq!(resolved[2].scope, PermissionScope::Session);
    }

    #[tokio::test]
    async fn supports_revoke_list_and_delete() {
        let repository = repository().await;
        let created = repository
            .upsert(&rule(PermissionScope::Global, None, None))
            .await
            .expect("create")
            .0;
        let revoked = repository
            .revoke(created.id, Some("obsolete".to_owned()), None)
            .await
            .expect("revoke")
            .expect("existing");
        assert!(revoked.revoked_at_ms.is_some());
        assert!(
            repository
                .resolve("shell.execute", None, None)
                .await
                .expect("resolve")
                .is_empty()
        );
        assert_eq!(
            repository
                .list(PermissionRuleListQuery {
                    search: Some("shell".to_owned()),
                    before_updated_at_ms: None,
                    before_id: None,
                    limit: 10
                })
                .await
                .expect("list")
                .len(),
            1
        );
        assert_eq!(
            repository
                .delete(created.id)
                .await
                .expect("delete")
                .map(|row| row.id),
            Some(created.id)
        );
    }
}
