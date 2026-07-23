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
        let scope = scope_to_string(rule.scope);
        let existing = txn.query_one(statement(
            format!("SELECT id FROM {TABLE} WHERE action_key = ? AND scope = ? AND session_id IS ? AND workspace_id IS ?"),
            [rule.action_key.clone().into(), scope.clone().into(), rule.session_id.into(), rule.workspace_id.into()],
        )).await.map_err(map_error)?;
        let now = Utc::now().timestamp_millis();
        let (id, created) = if let Some(row) = existing {
            let id: i64 = row.try_get("", "id").map_err(map_error)?;
            txn.execute(statement(
                format!("UPDATE {TABLE} SET mode = ?, source = ?, reason = ?, operator = ?, revoked_at_ms = ?, revoked_reason = ?, revoked_by = ?, updated_at_ms = ? WHERE id = ?"),
                [mode_to_string(rule.mode).into(), rule.source.clone().into(), rule.reason.clone().into(), rule.operator.clone().into(), rule.revoked_at_ms.into(), rule.revoked_reason.clone().into(), rule.revoked_by.clone().into(), now.into(), id.into()],
            )).await.map_err(map_error)?;
            (id, false)
        } else {
            let result = txn.execute(statement(
                format!("INSERT INTO {TABLE} (action_key, mode, scope, session_id, workspace_id, source, reason, operator, revoked_at_ms, revoked_reason, revoked_by, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),
                [rule.action_key.clone().into(), mode_to_string(rule.mode).into(), scope.into(), rule.session_id.into(), rule.workspace_id.into(), rule.source.clone().into(), rule.reason.clone().into(), rule.operator.clone().into(), rule.revoked_at_ms.into(), rule.revoked_reason.clone().into(), rule.revoked_by.clone().into(), now.into(), now.into()],
            )).await.map_err(map_error)?;
            (
                i64::try_from(result.last_insert_id()).map_err(|_| {
                    PermissionRuleRepositoryError::Backend(
                        "permission rule identifier exceeds i64 range".to_owned(),
                    )
                })?,
                true,
            )
        };
        let row = txn
            .query_one(statement(
                format!("SELECT {COLUMNS} FROM {TABLE} WHERE id = ?"),
                [id.into()],
            ))
            .await
            .map_err(map_error)?;
        row.map(record_from_row)
            .transpose()?
            .map(|record| (record, created))
            .ok_or_else(missing_row)
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
        let scope = scope_to_string(rule.scope);
        let existing = self.db.query_one(statement(
            format!("SELECT id FROM {TABLE} WHERE action_key = ? AND scope = ? AND session_id IS ? AND workspace_id IS ?"),
            [rule.action_key.clone().into(), scope.clone().into(), rule.session_id.into(), rule.workspace_id.into()],
        )).await.map_err(map_error)?;
        let now = Utc::now().timestamp_millis();
        if let Some(existing) = existing {
            let id: i64 = existing.try_get("", "id").map_err(map_error)?;
            self.db.execute(statement(
                format!("UPDATE {TABLE} SET mode = ?, source = ?, reason = ?, operator = ?, revoked_at_ms = ?, revoked_reason = ?, revoked_by = ?, updated_at_ms = ? WHERE id = ?"),
                [mode_to_string(rule.mode).into(), rule.source.clone().into(), rule.reason.clone().into(), rule.operator.clone().into(), rule.revoked_at_ms.into(), rule.revoked_reason.clone().into(), rule.revoked_by.clone().into(), now.into(), id.into()],
            )).await.map_err(map_error)?;
            return self
                .record(id)
                .await?
                .map(|row| (row, false))
                .ok_or_else(missing_row);
        }
        let result = self.db.execute(statement(
            format!("INSERT INTO {TABLE} (action_key, mode, scope, session_id, workspace_id, source, reason, operator, revoked_at_ms, revoked_reason, revoked_by, created_at_ms, updated_at_ms) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),
            [rule.action_key.clone().into(), mode_to_string(rule.mode).into(), scope.into(), rule.session_id.into(), rule.workspace_id.into(), rule.source.clone().into(), rule.reason.clone().into(), rule.operator.clone().into(), rule.revoked_at_ms.into(), rule.revoked_reason.clone().into(), rule.revoked_by.clone().into(), now.into(), now.into()],
        )).await.map_err(map_error)?;
        let id = i64::try_from(result.last_insert_id()).map_err(|_| {
            PermissionRuleRepositoryError::Backend(
                "permission rule identifier exceeds i64 range".to_owned(),
            )
        })?;
        self.record(id)
            .await?
            .map(|row| (row, true))
            .ok_or_else(missing_row)
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
}

fn statement(sql: String, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Sqlite, sql, values)
}
fn missing_row() -> PermissionRuleRepositoryError {
    PermissionRuleRepositoryError::Backend("permission rule row is missing after write".to_owned())
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
        PermissionMode::Ask => "ask",
        PermissionMode::Deny => "deny",
    }
    .to_owned()
}
fn mode_from_string(value: &str) -> Option<PermissionMode> {
    match value {
        "allow" => Some(PermissionMode::Allow),
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
        SeaPermissionRuleRepository::new(Arc::new(db))
    }

    fn rule(
        scope: PermissionScope,
        session_id: Option<i64>,
        workspace_id: Option<i64>,
    ) -> PersistedPermissionRule {
        PersistedPermissionRule {
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
