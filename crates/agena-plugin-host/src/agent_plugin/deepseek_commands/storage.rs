use std::{collections::BTreeMap, sync::Arc};

use agena_plugin_sdk::{
    HostClient, PluginError, Result,
    host_api::{
        HostStorageGetRequest, HostStorageScope, HostStorageSetRequest, HostStorageVisibility,
    },
};
use sea_orm::sqlx::sqlite::SqliteJournalMode;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use serde_yaml::Value as Yaml;

mod records;
pub(super) use records::Mutation as RecordMutation;
use records::{Backend, RecordStore};

pub(super) const MODULE: &str = include_str!("storage.js");
pub(in crate::agent_plugin) const HUB: &str = "@deepseek-ai/dsh-storage";
pub(in crate::agent_plugin) const JSON_BACKEND: &str = "@deepseek-ai/dsh-storage-json";
pub(in crate::agent_plugin) const SQLITE_BACKEND: &str = "@deepseek-ai/dsh-storage-sqlite";
pub(in crate::agent_plugin) const DOMAIN: &str = "@deepseek-ai/dsh-storage-domain";
pub(super) const HUB_MODULE: &str = r#"
export const UNIT_NAME_RE = /^[a-z][a-z0-9_]*$/;
export const storageBackendServiceKey = name => `storage.backend.${name}`;
export class StorageError extends Error {
    constructor(code, message, options) { super(message, options); this.name = 'StorageError'; this.code = code; }
}
"#;
pub(super) const DOMAIN_MODULE: &str = r#"
export const defineDomain = spec => spec;
export const domainTable = valueSchema => ({valueSchema});
export const descriptorOf = ({name, version, tables, global, layout, compatibleVersions}) =>
    ({name, version, tables: Object.keys(tables), hasGlobal: global !== undefined, layout, compatibleVersions});
"#;
const MAX_UNIT_BYTES: usize = 128 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 512 * 1024;
const MAX_UNITS: usize = 64;
const SNAPSHOT_KEY: &str = "units";
const COMMIT_ATTEMPTS: usize = 8;
const SCOPE: HostStorageScope = HostStorageScope::Global;
const VISIBILITY: HostStorageVisibility = HostStorageVisibility::Private;

#[derive(Clone)]
pub(super) struct Config {
    json_namespace: Option<String>,
    json_records: Option<Arc<RecordStore>>,
    sqlite: Option<Arc<RecordStore>>,
    domain: DomainConfig,
}

#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DomainConfig {
    backend: String,
    routes: BTreeMap<String, String>,
}

impl Default for DomainConfig {
    fn default() -> Self {
        Self {
            backend: "json".into(),
            routes: BTreeMap::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SqliteConfig {
    path: String,
    journal_mode: Option<String>,
}

impl Config {
    pub(super) fn discover(rows: &[(String, Yaml)], diagnostics: &mut Vec<String>) -> Self {
        let mut json_root = None;
        let mut json_available = true;
        let mut sqlite = None;
        let mut domain = DomainConfig::default();
        for (entry, row) in rows {
            let config = row.get("config").unwrap_or(&Yaml::Null);
            let result = match super::deepseek::module_name(row) {
                Some(JSON_BACKEND) => config
                    .get("root")
                    .and_then(Yaml::as_str)
                    .and_then(storage_path)
                    .ok_or_else(|| "storage-json requires a root string".to_owned())
                    .and_then(|root| {
                        if json_root.is_some_and(|previous| previous != root) {
                            return Err(
                                "one imported bundle can declare one JSON storage root".into()
                            );
                        }
                        json_root = Some(root);
                        Ok(())
                    }),
                Some(SQLITE_BACKEND) if sqlite.is_some() => {
                    Err("one imported bundle can declare one SQLite storage database".into())
                }
                Some(SQLITE_BACKEND) => sqlite_store(config).map(|store| {
                    sqlite = Some(Arc::new(store));
                }),
                Some(DOMAIN) => serde_yaml::from_value::<DomainConfig>(config.clone())
                    .map(|value| domain = value)
                    .map_err(|error| format!("invalid storage domain configuration: {error}")),
                _ => continue,
            };
            if let Err(error) = result {
                if super::deepseek::module_name(row) == Some(JSON_BACKEND) {
                    json_available = false;
                }
                diagnostics.push(format!("DeepSeek Harness storage entry {entry:?}: {error}"));
            }
        }
        let json_namespace = json_available.then(|| {
            json_root.map_or_else(
                || "deepseek.storage".to_owned(),
                |root| format!("deepseek.storage.{}", namespace(root)),
            )
        });
        let json_records = json_namespace.as_ref().map(|namespace_key| {
            Arc::new(RecordStore::new(
                Backend::JsonRecords,
                namespace(namespace_key),
                false,
                SqliteJournalMode::Wal,
            ))
        });
        for route in std::iter::once(&domain.backend).chain(domain.routes.values()) {
            if !(route == "json" && json_available || route == "sqlite" && sqlite.is_some()) {
                diagnostics.push(format!(
                    "DeepSeek Harness storage backend {route:?} is unavailable"
                ));
            }
        }
        Self {
            json_namespace,
            json_records,
            sqlite,
            domain,
        }
    }

    pub(super) fn binding(&self) -> Value {
        let backends: Vec<_> = [
            self.json_namespace.is_some().then_some("json"),
            self.sqlite.is_some().then_some("sqlite"),
        ]
        .into_iter()
        .flatten()
        .collect();
        json!({
            "backends": backends,
            "defaultBackend": self.domain.backend,
            "routes": self.domain.routes,
        })
    }

    pub(super) async fn snapshot(&self, host: &dyn HostClient) -> Value {
        let mut snapshot = Map::new();
        if let Some(namespace) = &self.json_namespace {
            let single = read_snapshot(host, namespace).await.map(|(_, units)| units);
            snapshot.insert("single".into(), snapshot_entry(single));
        }
        for (name, store) in [
            ("json_records", &self.json_records),
            ("sqlite", &self.sqlite),
        ] {
            if let Some(store) = store {
                snapshot.insert(name.into(), snapshot_entry(store.snapshot(host).await));
            }
        }
        Value::Object(snapshot)
    }

    pub(super) async fn commit(&self, host: &dyn HostClient, mutation: Mutation) -> Result<Value> {
        let namespace = self
            .json_namespace
            .as_deref()
            .ok_or_else(|| PluginError::invalid_params("Cordis JSON storage is unavailable"))?;
        mutation.commit(host, namespace).await
    }

    pub(super) async fn commit_records(
        &self,
        host: &dyn HostClient,
        mutation: RecordMutation,
    ) -> Result<Value> {
        let store = match mutation.backend {
            Backend::JsonRecords => self.json_records.as_ref(),
            Backend::Sqlite => self.sqlite.as_ref(),
        }
        .ok_or_else(|| {
            PluginError::invalid_params("Cordis record storage backend is unavailable")
        })?;
        serde_json::to_value(store.commit(host, mutation).await?)
            .map_err(|error| PluginError::internal_error(&error))
    }
}

fn snapshot_entry(units: Result<impl Serialize>) -> Value {
    match units {
        Ok(units) => json!({"units": units}),
        Err(error) => json!({"failure": error.failure.user.fallback}),
    }
}

fn sqlite_store(value: &Yaml) -> std::result::Result<RecordStore, String> {
    let config: SqliteConfig = serde_yaml::from_value(value.clone())
        .map_err(|error| format!("invalid storage-sqlite configuration: {error}"))?;
    let path = storage_path(&config.path).ok_or("storage-sqlite requires a path string")?;
    let journal = match config.journal_mode.as_deref().unwrap_or("wal") {
        "wal" => SqliteJournalMode::Wal,
        "delete" => SqliteJournalMode::Delete,
        "truncate" => SqliteJournalMode::Truncate,
        "persist" => SqliteJournalMode::Persist,
        _ => return Err("invalid Cordis SQLite journalMode".into()),
    };
    Ok(RecordStore::new(
        Backend::Sqlite,
        namespace(path),
        path == ":memory:",
        journal,
    ))
}

fn storage_path(path: &str) -> Option<&str> {
    (!path.trim().is_empty() && path.len() <= 4096 && !path.contains('\0')).then_some(path)
}

fn namespace(path: &str) -> String {
    blake3::hash(path.as_bytes()).to_hex()[..32].to_owned()
}

fn valid_unit(name: &str) -> bool {
    let mut bytes = name.bytes();
    name.len() <= 64
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

async fn read_snapshot(
    host: &dyn HostClient,
    namespace: &str,
) -> Result<(Option<String>, Map<String, Value>)> {
    let raw = host
        .storage_get(HostStorageGetRequest {
            scope: SCOPE,
            visibility: VISIBILITY,
            namespace: namespace.to_owned(),
            key: SNAPSHOT_KEY.to_owned(),
        })
        .await?
        .value;
    let Some(saved) = raw.as_deref() else {
        return Ok((None, Map::new()));
    };
    if saved.len() > MAX_SNAPSHOT_BYTES {
        return Err(PluginError::invalid_params(
            "Cordis storage snapshot exceeds its byte budget",
        ));
    }
    let units: Map<String, Value> = serde_json::from_str(saved)
        .map_err(|_| PluginError::internal("invalid Cordis storage snapshot"))?;
    if units.len() > MAX_UNITS
        || units.iter().any(|(name, value)| {
            !valid_unit(name) || value.as_str().is_none_or(|raw| raw.len() > MAX_UNIT_BYTES)
        })
    {
        return Err(PluginError::internal("invalid Cordis storage unit"));
    }
    Ok((raw, units))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mutation {
    unit: String,
    previous: Option<String>,
    value: String,
}

impl Mutation {
    pub(super) async fn commit(self, host: &dyn HostClient, namespace: &str) -> Result<Value> {
        if !valid_unit(&self.unit)
            || self.value.len() > MAX_UNIT_BYTES
            || self
                .previous
                .as_ref()
                .is_some_and(|value| value.len() > MAX_UNIT_BYTES)
        {
            return Err(PluginError::invalid_params(
                "invalid Cordis storage mutation",
            ));
        }
        let record: Value = serde_json::from_str(&self.value)
            .map_err(|_| PluginError::invalid_params("Cordis storage requires a JSON record"))?;
        if record["version"].as_u64().is_none()
            || record["revision"].as_u64().is_none()
            || !record["tables"].is_object()
            || record.get("global").is_none()
        {
            return Err(PluginError::invalid_params("invalid Cordis storage record"));
        }
        for _ in 0..COMMIT_ATTEMPTS {
            let (current, mut units) = read_snapshot(host, namespace).await?;
            if units.get(&self.unit).and_then(Value::as_str) != self.previous.as_deref() {
                return Ok(Value::Bool(false));
            }
            units.insert(self.unit.clone(), Value::String(self.value.clone()));
            let value = serde_json::to_string(&units)
                .map_err(|error| PluginError::internal_error(&error))?;
            if units.len() > MAX_UNITS || value.len() > MAX_SNAPSHOT_BYTES {
                return Err(PluginError::invalid_params(
                    "Cordis storage snapshot exceeds its capacity",
                ));
            }
            if host
                .storage_compare_exchange(
                    HostStorageSetRequest {
                        scope: SCOPE,
                        visibility: VISIBILITY,
                        namespace: namespace.to_owned(),
                        key: SNAPSHOT_KEY.to_owned(),
                        value,
                    },
                    current,
                )
                .await?
            {
                return Ok(Value::Bool(true));
            }
        }
        Ok(Value::Bool(false))
    }
}
