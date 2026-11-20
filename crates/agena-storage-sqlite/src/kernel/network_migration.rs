//! Upgrade historical session permissions before recovery reads typed settings.

use std::{collections::BTreeMap, sync::Arc};

use agena_domain::{NetworkPermissionConfig, PathPermissionMode, PermissionConfig, PermissionMode};
use sea_orm::sqlx::{self, Row, SqliteConnection};
use serde::Deserialize;
use serde_json::Value;

use super::{Result, invalid, storage};
use crate::KernelDatabase;

/// Resolve the old global/workspace network default for a historical session.
/// The migration invokes this callback on the blocking pool, once per workspace.
pub type NetworkMigrationResolver =
    dyn Fn(Option<&str>) -> std::result::Result<NetworkPermissionConfig, String> + Send + Sync;

const LEGACY_PERMISSION: &str = "(json_type(s.configuration_json,'$.permission.network.egress') IS NOT NULL OR json_type(s.configuration_json,'$.permission.network.ingress') IS NOT NULL)";
const LEGACY_CEILING: &str = "(json_type(s.configuration_json,'$.permission_ceiling.network.egress') IS NOT NULL OR json_type(s.configuration_json,'$.permission_ceiling.network.ingress') IS NOT NULL)";
const UNVERSIONED_PERMISSION: &str = "(json_type(s.configuration_json,'$.permission.network')='object' AND json_type(s.configuration_json,'$.permission.network.version') IS NULL)";
const UNVERSIONED_CEILING: &str = "(json_type(s.configuration_json,'$.permission_ceiling.network')='object' AND json_type(s.configuration_json,'$.permission_ceiling.network.version') IS NULL)";
const LEGACY_PATH_DEFAULTS: &str =
    "(json_type(s.configuration_json,'$.permission.path.workspace')='object'
    OR json_type(s.configuration_json,'$.permission.path.external')='object'
    OR json_type(s.configuration_json,'$.permission_ceiling.path.workspace')='object'
    OR json_type(s.configuration_json,'$.permission_ceiling.path.external')='object')";
const LEGACY_CEILING_LAYERS: &str = "EXISTS (SELECT 1 FROM json_each(s.configuration_json,'$.permission_ceiling_layers') AS layer
    WHERE (json_type(layer.value,'$.network')='object' AND json_type(layer.value,'$.network.version') IS NULL)
    OR json_type(layer.value,'$.path.workspace')='object'
    OR json_type(layer.value,'$.path.external')='object')";
const BACKUP_PREFIX: &str = "migration.network.v2.session.";
const LEGACY_PERMISSION_BACKUP_PREFIX: &str = "migration.permission.legacy.session.";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPathDefault {
    read: Option<PermissionMode>,
    write: Option<PermissionMode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifiedNetwork {
    #[serde(rename = "internet")]
    _internet: Option<PermissionMode>,
    #[serde(rename = "private")]
    _private: Option<PermissionMode>,
    #[serde(rename = "loopback")]
    _loopback: Option<PermissionMode>,
    #[serde(default, rename = "rules")]
    _rules: BTreeMap<String, PermissionMode>,
}

fn is_legacy(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|value| value.contains_key("egress") || value.contains_key("ingress"))
}

fn upgrade_permission(
    permission: &mut Value,
    parent: Option<&NetworkPermissionConfig>,
    ceiling: bool,
) -> std::result::Result<bool, String> {
    let json_error = |error: serde_json::Error| error.to_string();
    let mut migrated_legacy_permissions = false;
    if let Some(path) = permission.get_mut("path") {
        for field in ["workspace", "external"] {
            let Some(value) = path.get_mut(field).filter(|value| value.is_object()) else {
                continue;
            };
            let modes: LegacyPathDefault = serde_json::from_value(value.clone())
                .map_err(|error| format!("path.{field}: {error}"))?;
            // MXC has no per-resource approvals or write-only access. Carry
            // forward only the access this default granted unconditionally.
            let mode = match (modes.read, modes.write) {
                (Some(PermissionMode::Allow), Some(PermissionMode::Allow)) => {
                    PathPermissionMode::Readwrite
                }
                (Some(PermissionMode::Allow), _) => PathPermissionMode::Readonly,
                _ => PathPermissionMode::Deny,
            };
            *value = serde_json::to_value(mode).map_err(json_error)?;
            migrated_legacy_permissions = true;
        }
    }

    let Some(network) = permission
        .get_mut("network")
        .filter(|network| network.is_object())
    else {
        return Ok(migrated_legacy_permissions);
    };
    let complete = if is_legacy(network) {
        let overlay = NetworkPermissionConfig::from_legacy_value(network.clone())?;
        if ceiling {
            if overlay.is_empty() {
                // The old domain skipped entirely empty ceilings. Keep that
                // behavior for the nested format, where {} was an empty layer.
                let mut remainder = permission.clone();
                remainder
                    .as_object_mut()
                    .expect("network belongs to an object")
                    .remove("network");
                if serde_json::from_value::<PermissionConfig>(remainder.clone())
                    .map_err(json_error)?
                    .is_empty()
                {
                    *permission = remainder;
                    return Ok(migrated_legacy_permissions);
                }
            }
            overlay.resolved()
        } else {
            let mut effective = parent.cloned().ok_or_else(|| {
                "workspace network configuration changed during migration; retry".to_owned()
            })?;
            effective.merge_legacy_from(overlay);
            effective.resolved()
        }
    } else if network.get("version").is_some() {
        return Ok(migrated_legacy_permissions);
    } else if ["internet", "private", "loopback", "rules"]
        .into_iter()
        .any(|field| network.get(field).is_some())
    {
        // Hostname classes, ordered rules, and approval modes cannot be
        // represented by MXC. Pin the policy offline and back up the original.
        serde_json::from_value::<ClassifiedNetwork>(network.clone()).map_err(json_error)?;
        migrated_legacy_permissions = true;
        NetworkPermissionConfig::offline()
    } else {
        // Flat v1 policies were complete; decoding fills every omitted field.
        serde_json::from_value(network.clone()).map_err(json_error)?
    };
    permission["network"] = serde_json::to_value(complete).map_err(json_error)?;
    Ok(migrated_legacy_permissions)
}

fn upgrade_document(
    original: &str,
    parent: Option<&NetworkPermissionConfig>,
    session: i64,
) -> Result<(String, bool)> {
    let mut document: Value = serde_json::from_str(original).map_err(storage)?;
    let mut migrated_legacy_permissions = false;
    for (section, ceiling) in [("permission", false), ("permission_ceiling", true)] {
        if let Some(permission) = document.get_mut(section) {
            migrated_legacy_permissions |= upgrade_permission(permission, parent, ceiling)
                .map_err(|error| invalid(format!("session {session}: {section}: {error}")))?;
        }
    }
    if let Some(layers) = document
        .get_mut("permission_ceiling_layers")
        .and_then(Value::as_array_mut)
    {
        for (index, layer) in layers.iter_mut().enumerate() {
            migrated_legacy_permissions |=
                upgrade_permission(layer, None, true).map_err(|error| {
                    invalid(format!(
                        "session {session}: permission_ceiling_layers[{index}]: {error}"
                    ))
                })?;
        }
    }
    let replacement = serde_json::to_string(&document).map_err(storage)?;
    if replacement.len() > 65_536 {
        return Err(invalid(format!(
            "session {session}: permission migration exceeds the 65536-byte session settings limit"
        )));
    }
    Ok((replacement, migrated_legacy_permissions))
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
        Ok(legacy_sessions) => {
            transaction.commit().await.map_err(storage)?;
            if legacy_sessions > 0 {
                tracing::warn!(
                    sessions = legacy_sessions,
                    "Migrated pre-native session permissions and backed up the original settings"
                );
            }
            Ok(())
        }
        Err(error) => {
            transaction.rollback().await.map_err(storage)?;
            Err(error)
        }
    }
}

async fn migrate_rows(
    connection: &mut SqliteConnection,
    parents: &BTreeMap<Option<String>, NetworkPermissionConfig>,
) -> Result<usize> {
    let select = format!(
        "SELECT s.id,s.configuration_json,s.metadata_version,s.control_version,w.path AS workspace_path FROM agena_sessions s LEFT JOIN agena_workspaces w ON w.id=s.workspace_id WHERE s.id>? AND ({LEGACY_PERMISSION} OR {LEGACY_CEILING} OR {UNVERSIONED_PERMISSION} OR {UNVERSIONED_CEILING} OR {LEGACY_PATH_DEFAULTS} OR {LEGACY_CEILING_LAYERS}) ORDER BY s.id LIMIT 32"
    );
    let mut cursor = i64::MIN;
    let mut legacy_sessions = 0;
    loop {
        // Settings rows are at most 64 KiB. Bound each page while retaining one
        // transaction for backups and all rewritten sessions.
        let rows = sqlx::query(&select)
            .bind(cursor)
            .fetch_all(&mut *connection)
            .await
            .map_err(storage)?;
        if rows.is_empty() {
            return Ok(legacy_sessions);
        }
        for row in rows {
            let session: i64 = row.try_get("id").map_err(storage)?;
            let original: String = row.try_get("configuration_json").map_err(storage)?;
            let workspace: Option<String> = row.try_get("workspace_path").map_err(storage)?;
            let metadata_version: i64 = row.try_get("metadata_version").map_err(storage)?;
            let control_version: i64 = row.try_get("control_version").map_err(storage)?;
            let (replacement, migrated_legacy_permissions) =
                upgrade_document(&original, parents.get(&workspace), session)?;
            let prefix = if migrated_legacy_permissions {
                LEGACY_PERMISSION_BACKUP_PREFIX
            } else {
                BACKUP_PREFIX
            };
            let key = format!("{prefix}{session}");
            let existing = sqlx::query_scalar::<_, String>(
                "SELECT value_json FROM agena_server_state WHERE key=?",
            )
            .bind(&key)
            .fetch_optional(&mut *connection)
            .await
            .map_err(storage)?;
            if existing.as_ref().is_some_and(|value| value != &original) {
                return Err(invalid(format!(
                    "session {session}: a different original permission migration backup already exists"
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
                    "session {session}: settings changed or a version limit was reached during permission migration"
                )));
            }
            legacy_sessions += usize::from(migrated_legacy_permissions);
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
                    "version":2,
                    "default": "allow", "inbound": "allow", "hostLoopback": "allow",
                    "allow": [{"to": "192.0.2.1", "protocol": "tcp", "ports": [443]}],
                    "deny": [{"to": "10.0.0.0/8", "protocol": "tcp", "ports": [22]}]
                })
            );
            assert_eq!(
                document["permission_ceiling"]["network"],
                json!({
                    "version":2,"default": "deny", "allow": [{"to": "192.0.2.0/24"}],"deny":[],"inbound":"deny","hostLoopback":"deny"
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
            json!({"version":2,"default":"deny","allow":[],"deny":[],"inbound":"deny","hostLoopback":"deny"})
        );
        assert_eq!(
            active_document["permission_ceiling"]["path"],
            json!({"workspace": "readonly"})
        );
        let flat_saved = settings(&database, flat).await;
        let flat_document: Value = serde_json::from_str(&flat_saved.0).unwrap();
        for section in ["permission", "permission_ceiling"] {
            assert_eq!(
                flat_document[section]["network"],
                json!({"version":2,"default":"deny","allow":[],"deny":[],"inbound":"deny","hostLoopback":"deny"})
            );
        }
        assert_eq!((flat_saved.1, flat_saved.2), (2, 2));
        assert_eq!(backup(&database, flat).await.as_deref(), Some(new_offline));
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

    #[tokio::test]
    async fn flat_v1_migration_preserves_full_policy_and_leaves_sparse_v2_untouched() {
        let database = KernelDatabase::open("sqlite::memory:").await.unwrap();
        let old = r#"{"permission":{"network":{"inbound":"allow"}}}"#;
        let modern = r#"{"permission":{"network":{"version":2,"inbound":"allow"}}}"#;
        let legacy = add_session(&database, None, old).await;
        let current = add_session(&database, None, modern).await;
        let chain_text =
            r#"{"permission_ceiling_layers":[{"network":{}},{"network":{"version":2,"deny":[]}}]}"#;
        let chain = add_session(&database, None, chain_text).await;
        let resolver: Arc<NetworkMigrationResolver> =
            Arc::new(|_| panic!("flat v1 defaults are already self-contained"));
        migrate(&database, resolver.clone()).await.unwrap();
        let saved = settings(&database, legacy).await;
        let document: Value = serde_json::from_str(&saved.0).unwrap();
        assert_eq!(
            document["permission"]["network"],
            json!({"version":2,"default":"deny","inbound":"allow","hostLoopback":"deny","allow":[],"deny":[]})
        );
        assert_eq!(backup(&database, legacy).await.as_deref(), Some(old));
        assert_eq!(
            settings(&database, current).await,
            (modern.to_owned(), 1, 1, 1)
        );
        let chain_saved = settings(&database, chain).await;
        let chain_document: Value = serde_json::from_str(&chain_saved.0).unwrap();
        assert_eq!(
            chain_document["permission_ceiling_layers"][0]["network"],
            json!({"version":2,"default":"deny","inbound":"deny","hostLoopback":"deny","allow":[],"deny":[]})
        );
        assert_eq!(
            chain_document["permission_ceiling_layers"][1]["network"],
            json!({"version":2,"deny":[]})
        );
        assert_eq!((chain_saved.1, chain_saved.2), (2, 2));
        assert_eq!(backup(&database, chain).await.as_deref(), Some(chain_text));
        migrate(&database, resolver).await.unwrap();
        assert_eq!(settings(&database, legacy).await, saved);
        assert_eq!(settings(&database, chain).await, chain_saved);
        assert_eq!(backup(&database, current).await, None);
    }

    #[tokio::test]
    async fn migrates_classified_network_and_approval_based_path_defaults_before_recovery() {
        let database = KernelDatabase::open("sqlite::memory:").await.unwrap();
        let original = r#"{
            "permission": {
                "network": {"internet":"auto","private":"auto","loopback":"auto"},
                "path": {"workspace":{"read":"allow","write":"auto"},
                         "external":{"read":"auto","write":"auto"}},
                "tools": {"default":"ask"}
            },
            "permission_ceiling": {
                "network": {"rules":{"example.com:443":"allow"}},
                "path": {"workspace":{"read":"allow","write":"ask"}}
            },
            "permission_ceiling_layers": [{
                "network": {"private":"ask"},
                "path": {"external":{"read":"deny","write":"allow"},
                         "rules":{"/srv/output":{"read":"allow","write":"allow"},
                                  "/srv/output/**":{"read":"allow","write":"allow"}}}
            }]
        }"#;
        let session = add_session(&database, None, original).await;
        let path_only = r#"{"permission":{"network":{"version":2,"default":"deny"},"path":{"workspace":{"read":"allow","write":"allow"}}}}"#;
        let path_session = add_session(&database, None, path_only).await;
        let previous_backup = r#"{"permission":{"network":{"default":"deny"}}}"#;
        sqlx::query("INSERT INTO agena_server_state(key,value_json,updated_at_ms) VALUES(?,?,1)")
            .bind(format!("{BACKUP_PREFIX}{path_session}"))
            .bind(previous_backup)
            .execute(database.writer.get_sqlite_connection_pool())
            .await
            .unwrap();

        let resolver: Arc<NetworkMigrationResolver> =
            Arc::new(|_| panic!("classified policies must not inherit grants"));
        migrate(&database, resolver.clone()).await.unwrap();
        let saved = settings(&database, session).await;
        let configuration: agena_storage::kernel::SessionConfiguration =
            serde_json::from_str(&saved.0).unwrap();
        let offline = NetworkPermissionConfig::offline();
        assert_eq!(configuration.permission.network, Some(offline.clone()));
        assert_eq!(
            configuration.permission_ceiling.network,
            Some(offline.clone())
        );
        assert_eq!(
            configuration.permission_ceiling_layers[0].network,
            Some(offline)
        );

        let document: Value = serde_json::from_str(&saved.0).unwrap();
        assert_eq!(
            document["permission"]["path"],
            json!({"workspace":"readonly","external":"deny"})
        );
        assert_eq!(document["permission"]["tools"], json!({"default":"ask"}));
        assert_eq!(
            document["permission_ceiling"]["path"]["workspace"],
            "readonly"
        );
        assert_eq!(
            document["permission_ceiling_layers"][0]["path"]["external"],
            "deny"
        );
        assert_eq!(
            document["permission_ceiling_layers"][0]["path"]["rules"],
            json!({"/srv/output":{"read":"allow","write":"allow"},
                   "/srv/output/**":{"read":"allow","write":"allow"}})
        );
        let path_saved = settings(&database, path_session).await;
        assert_eq!(
            serde_json::from_str::<Value>(&path_saved.0).unwrap()["permission"],
            json!({"network":{"version":2,"default":"deny"},"path":{"workspace":"readwrite"}})
        );
        assert_eq!(
            backup(&database, path_session).await.as_deref(),
            Some(previous_backup)
        );
        for (id, expected) in [(session, original), (path_session, path_only)] {
            let backup: String =
                sqlx::query_scalar("SELECT value_json FROM agena_server_state WHERE key=?")
                    .bind(format!("migration.permission.legacy.session.{id}"))
                    .fetch_one(database.writer.get_sqlite_connection_pool())
                    .await
                    .unwrap();
            assert_eq!(backup, expected);
        }
        assert_eq!((saved.1, saved.2), (2, 2));

        migrate(&database, resolver).await.unwrap();
        assert_eq!(settings(&database, session).await, saved);
        assert_eq!(settings(&database, path_session).await, path_saved);
    }
}
