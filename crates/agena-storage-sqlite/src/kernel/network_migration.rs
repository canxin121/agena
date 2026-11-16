//! Upgrade historical session network blocks before recovery reads typed settings.

use std::{collections::BTreeMap, sync::Arc};

use agena_domain::{
    NetworkEgressConfig, NetworkIngressConfig, NetworkPermissionConfig, PermissionConfig,
};
use sea_orm::sqlx::{self, Row, SqliteConnection};
use serde_json::Value;

use super::{Result, invalid, storage};
use crate::KernelDatabase;

/// Resolve the old global/workspace network default for a historical session.
/// The migration invokes this callback on the blocking pool, once per workspace.
pub type NetworkMigrationResolver =
    dyn Fn(Option<&str>) -> std::result::Result<NetworkPermissionConfig, String> + Send + Sync;

const LEGACY_PERMISSION: &str = "(json_type(s.configuration_json,'$.permission.network.egress') IS NOT NULL OR json_type(s.configuration_json,'$.permission.network.ingress') IS NOT NULL)";
const LEGACY_CEILING: &str = "(json_type(s.configuration_json,'$.permission_ceiling.network.egress') IS NOT NULL OR json_type(s.configuration_json,'$.permission_ceiling.network.ingress') IS NOT NULL)";
const BACKUP_PREFIX: &str = "migration.network.v1.session.";

fn is_legacy(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|value| value.contains_key("egress") || value.contains_key("ingress"))
}

fn legacy_empty(network: &NetworkPermissionConfig) -> bool {
    network
        .egress
        .as_ref()
        .is_none_or(NetworkEgressConfig::is_empty)
        && network
            .ingress
            .as_ref()
            .is_none_or(NetworkIngressConfig::is_empty)
}

fn upgrade_document(
    original: &str,
    parent: Option<&NetworkPermissionConfig>,
    session: i64,
) -> Result<String> {
    let mut document: Value = serde_json::from_str(original).map_err(storage)?;
    if let Some(value) = document
        .pointer("/permission/network")
        .filter(|value| is_legacy(value))
    {
        let overlay = NetworkPermissionConfig::from_legacy_value(value.clone()).map_err(invalid)?;
        let mut network = parent.cloned().ok_or_else(|| {
            invalid(format!(
                "session {session}: workspace network configuration changed during migration; retry"
            ))
        })?;
        network.merge_legacy_from(overlay);
        let network = network.resolved();
        network.validate().map_err(invalid)?;
        document["permission"]["network"] = serde_json::to_value(network).map_err(storage)?;
    }
    if let Some(value) = document
        .pointer("/permission_ceiling/network")
        .filter(|value| is_legacy(value))
    {
        let network = NetworkPermissionConfig::from_legacy_value(value.clone()).map_err(invalid)?;
        let mut remainder = document["permission_ceiling"].clone();
        remainder
            .as_object_mut()
            .expect("network belongs to an object")
            .remove("network");
        // The old domain model skipped a ceiling whose entire permission object
        // was empty. Preserve that no-op instead of turning it into an offline
        // ceiling. A new flat network:{} is never selected by this migration.
        let was_noop = legacy_empty(&network)
            && serde_json::from_value::<PermissionConfig>(remainder.clone())
                .map_err(invalid)?
                .is_empty();
        if was_noop {
            document["permission_ceiling"] = remainder;
        } else {
            let network = network.resolved();
            network.validate().map_err(invalid)?;
            document["permission_ceiling"]["network"] =
                serde_json::to_value(network).map_err(storage)?;
        }
    }
    let replacement = serde_json::to_string(&document).map_err(storage)?;
    if replacement.len() > 65_536 {
        return Err(invalid(format!(
            "session {session}: flat network migration exceeds the 65536-byte session settings limit"
        )));
    }
    Ok(replacement)
}

pub(super) async fn migrate(
    database: &KernelDatabase,
    resolver: Arc<NetworkMigrationResolver>,
) -> Result<()> {
    let pool = database.writer.get_sqlite_connection_pool();
    let workspaces = sqlx::query(&format!(
        "SELECT DISTINCT w.path AS workspace_path FROM agena_sessions s LEFT JOIN agena_workspaces w ON w.id=s.workspace_id WHERE {LEGACY_PERMISSION}"
    ))
    .fetch_all(pool).await.map_err(storage)?;
    let workspaces = workspaces
        .into_iter()
        .map(|row| {
            row.try_get::<Option<String>, _>("workspace_path")
                .map_err(storage)
        })
        .collect::<Result<Vec<_>>>()?;
    // File loading/migration may touch slow workspace storage. Keep that work
    // off the async reactor and outside the SQLite transaction.
    let parents = tokio::task::spawn_blocking(move || {
        let mut parents = BTreeMap::new();
        for workspace in workspaces {
            let network = resolver(workspace.as_deref())?;
            network.validate()?;
            parents.insert(workspace, network);
        }
        Ok::<_, String>(parents)
    })
    .await
    .map_err(storage)?
    .map_err(invalid)?;

    let mut transaction = pool.begin().await.map_err(storage)?;
    let outcome = migrate_rows(&mut transaction, &parents).await;
    match outcome {
        Ok(()) => transaction.commit().await.map_err(storage),
        Err(error) => {
            transaction.rollback().await.map_err(storage)?;
            Err(error)
        }
    }
}

async fn migrate_rows(
    connection: &mut SqliteConnection,
    parents: &BTreeMap<Option<String>, NetworkPermissionConfig>,
) -> Result<()> {
    let select = format!(
        "SELECT s.id,s.configuration_json,s.metadata_version,s.control_version,w.path AS workspace_path FROM agena_sessions s LEFT JOIN agena_workspaces w ON w.id=s.workspace_id WHERE s.id>? AND ({LEGACY_PERMISSION} OR {LEGACY_CEILING}) ORDER BY s.id LIMIT 32"
    );
    let mut cursor = i64::MIN;
    loop {
        // Settings rows are at most 64 KiB. Bound each page while retaining one
        // transaction for backups and all rewritten sessions.
        let rows = sqlx::query(&select)
            .bind(cursor)
            .fetch_all(&mut *connection)
            .await
            .map_err(storage)?;
        if rows.is_empty() {
            return Ok(());
        }
        for row in rows {
            let session: i64 = row.try_get("id").map_err(storage)?;
            let original: String = row.try_get("configuration_json").map_err(storage)?;
            let workspace: Option<String> = row.try_get("workspace_path").map_err(storage)?;
            let metadata_version: i64 = row.try_get("metadata_version").map_err(storage)?;
            let control_version: i64 = row.try_get("control_version").map_err(storage)?;
            let replacement = upgrade_document(&original, parents.get(&workspace), session)?;
            let key = format!("{BACKUP_PREFIX}{session}");
            let existing = sqlx::query_scalar::<_, String>(
                "SELECT value_json FROM agena_server_state WHERE key=?",
            )
            .bind(&key)
            .fetch_optional(&mut *connection)
            .await
            .map_err(storage)?;
            if existing.as_ref().is_some_and(|value| value != &original) {
                return Err(invalid(format!(
                    "session {session}: a different original network migration backup already exists"
                )));
            }
            let now = chrono::Utc::now().timestamp_millis();
            sqlx::query("INSERT INTO agena_server_state(key,value_json,updated_at_ms) VALUES(?,?,?) ON CONFLICT(key) DO NOTHING")
                .bind(key).bind(&original).bind(now)
                .execute(&mut *connection).await.map_err(storage)?;
            let updated = sqlx::query(
                "UPDATE agena_sessions SET configuration_json=?,metadata_version=metadata_version+1,control_version=control_version+1,updated_at_ms=MAX(?,updated_at_ms+1) WHERE id=? AND configuration_json=? AND metadata_version=? AND control_version=? AND metadata_version<9223372036854775807 AND control_version<9223372036854775807 AND updated_at_ms<9223372036854775807"
            ).bind(replacement).bind(now).bind(session).bind(&original)
                .bind(metadata_version).bind(control_version)
                .execute(&mut *connection).await.map_err(storage)?;
            if updated.rows_affected() != 1 {
                return Err(invalid(format!(
                    "session {session}: settings changed or a version limit was reached during network migration"
                )));
            }
            cursor = session;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    async fn add_workspace(database: &KernelDatabase) -> i64 {
        sqlx::query("INSERT INTO agena_workspaces(path,created_at_ms,updated_at_ms) VALUES('/test/project',1,1)")
            .execute(database.writer.get_sqlite_connection_pool()).await.unwrap().last_insert_rowid()
    }

    async fn add_session(database: &KernelDatabase, workspace: Option<i64>, original: &str) -> i64 {
        sqlx::query("INSERT INTO agena_sessions(workspace_id,title,configuration_json,created_at_ms,updated_at_ms) VALUES(?,'migration fixture',?,1,1)")
            .bind(workspace).bind(original)
            .execute(database.writer.get_sqlite_connection_pool()).await.unwrap().last_insert_rowid()
    }

    async fn settings(database: &KernelDatabase, session: i64) -> (String, i64, i64, i64) {
        sqlx::query_as("SELECT configuration_json,metadata_version,control_version,updated_at_ms FROM agena_sessions WHERE id=?")
            .bind(session).fetch_one(database.writer.get_sqlite_connection_pool()).await.unwrap()
    }

    async fn backup(database: &KernelDatabase, session: i64) -> Option<String> {
        sqlx::query_scalar("SELECT value_json FROM agena_server_state WHERE key=?")
            .bind(format!("{BACKUP_PREFIX}{session}"))
            .fetch_optional(database.writer.get_sqlite_connection_pool())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn preserves_inherited_denies_and_independent_ceiling_then_is_idempotent() {
        let database = KernelDatabase::open("sqlite::memory:").await.unwrap();
        let workspace = add_workspace(&database).await;
        let original = serde_json::to_string_pretty(&json!({
            "permission": {"network": {"egress": {"allow": [{
                "to": [{"cidr": "192.0.2.1/32"}],
                "ports": [{"protocol": "tcp", "port": 443}]
            }]}}},
            "permission_ceiling": {"network": {"egress": {"allow": [{
                "to": [{"cidr": "192.0.2.0/24"}]
            }]}}},
            "custom": {"preserve": [1, "unchanged"]}
        }))
        .unwrap();
        let first = add_session(&database, Some(workspace), &original).await;
        let second = add_session(&database, Some(workspace), &original).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let resolver: Arc<NetworkMigrationResolver> = Arc::new({
            let calls = calls.clone();
            move |workspace| {
                assert_eq!(workspace, Some("/test/project"));
                calls.fetch_add(1, Ordering::SeqCst);
                serde_json::from_value(json!({
                    "default": "allow", "inbound": "allow", "hostLoopback": "allow",
                    "deny": [{"to": "10.0.0.0/8", "protocol": "tcp", "ports": [22]}]
                }))
                .map_err(|error| error.to_string())
            }
        });
        migrate(&database, resolver.clone()).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "resolve once per workspace"
        );
        let mut migrated = Vec::new();
        for session in [first, second] {
            let saved = settings(&database, session).await;
            let document: Value = serde_json::from_str(&saved.0).unwrap();
            assert_eq!(
                document["permission"]["network"],
                json!({
                    "default": "allow", "inbound": "allow", "hostLoopback": "allow",
                    "allow": [{"to": "192.0.2.1", "protocol": "tcp", "ports": [443]}],
                    "deny": [{"to": "10.0.0.0/8", "protocol": "tcp", "ports": [22]}]
                })
            );
            assert_eq!(
                document["permission_ceiling"]["network"],
                json!({
                    "default": "deny", "allow": [{"to": "192.0.2.0/24"}]
                })
            );
            assert_eq!(document["custom"], json!({"preserve": [1, "unchanged"]}));
            assert_eq!((saved.1, saved.2), (2, 2));
            assert!(saved.3 > 1);
            assert_eq!(
                backup(&database, session).await.as_deref(),
                Some(original.as_str())
            );
            migrated.push((session, saved));
        }
        migrate(&database, resolver).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "flat rows need no parent resolution"
        );
        for (session, saved) in migrated {
            assert_eq!(settings(&database, session).await, saved);
            assert_eq!(
                backup(&database, session).await.as_deref(),
                Some(original.as_str())
            );
        }
    }

    #[tokio::test]
    async fn preserves_legacy_empty_ceiling_noop_and_new_explicit_offline_block() {
        let database = KernelDatabase::open("sqlite::memory:").await.unwrap();
        let old_noop = r#"{"permission_ceiling":{"network":{"egress":{},"ingress":{}}}}"#;
        let old_active =
            r#"{"permission_ceiling":{"path":{"workspace":"readonly"},"network":{"egress":{}}}}"#;
        let new_offline = r#"{ "permission":{"network":{}}, "permission_ceiling":{"network":{}} }"#;
        let noop = add_session(&database, None, old_noop).await;
        let active = add_session(&database, None, old_active).await;
        let flat = add_session(&database, None, new_offline).await;
        let resolver: Arc<NetworkMigrationResolver> = Arc::new(|_| {
            panic!("standalone ceilings and flat policies do not inherit workspace settings")
        });
        migrate(&database, resolver).await.unwrap();
        let noop_document: Value =
            serde_json::from_str(&settings(&database, noop).await.0).unwrap();
        assert_eq!(noop_document, json!({"permission_ceiling": {}}));
        let active_document: Value =
            serde_json::from_str(&settings(&database, active).await.0).unwrap();
        assert_eq!(
            active_document["permission_ceiling"]["network"],
            json!({"default": "deny"})
        );
        assert_eq!(
            active_document["permission_ceiling"]["path"],
            json!({"workspace": "readonly"})
        );
        assert_eq!(
            settings(&database, flat).await,
            (new_offline.to_string(), 1, 1, 1)
        );
        assert_eq!(backup(&database, flat).await, None);
        assert_eq!(backup(&database, noop).await.as_deref(), Some(old_noop));
    }

    #[tokio::test]
    async fn failed_session_update_rolls_back_all_rewrites_versions_and_backups() {
        let database = KernelDatabase::open("sqlite::memory:").await.unwrap();
        let original = r#"{"permission":{"network":{"egress":{"default":"deny"}}}}"#;
        let first = add_session(&database, None, original).await;
        let second = add_session(&database, None, original).await;
        sqlx::query(&format!(
            "CREATE TRIGGER fail_network_migration BEFORE UPDATE OF configuration_json ON agena_sessions WHEN OLD.id={second} BEGIN SELECT RAISE(ABORT,'injected migration write failure'); END"
        )).execute(database.writer.get_sqlite_connection_pool()).await.unwrap();
        let error = migrate(
            &database,
            Arc::new(|_| Ok(NetworkPermissionConfig::allow_all())),
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("injected migration write failure"),
            "{error}"
        );
        for session in [first, second] {
            assert_eq!(
                settings(&database, session).await,
                (original.to_string(), 1, 1, 1)
            );
            assert_eq!(backup(&database, session).await, None);
        }
    }
}
