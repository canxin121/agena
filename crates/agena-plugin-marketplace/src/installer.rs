//! Install / uninstall flows. Wraps cache layout, http fetch, sha256/signature
//! verification, and toml_edit-based config writes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use sha2::{Digest, Sha256};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

use crate::cache::{InstalledRecord, MarketplaceCache, write_secure_file};
use crate::error::MarketplaceError;
use crate::manifest::{PluginKind, PluginVersion, RegistryIndex};
use crate::{HttpFetcher, ReqwestFetcher};

/// Configured registry endpoint.
#[derive(Debug, Clone)]
pub struct RegistrySpec {
    pub id: String,
    pub url: String,
    pub require_signature: bool,
}

/// Lazily-fetched registry handle bound to a [`MarketplaceCache`].
pub struct RegistryHandle<'a, F: HttpFetcher> {
    spec: RegistrySpec,
    cache: &'a MarketplaceCache,
    fetcher: &'a F,
}

impl<'a, F: HttpFetcher> RegistryHandle<'a, F> {
    pub fn fetch_index(&self, force_refresh: bool) -> Result<RegistryIndex, MarketplaceError> {
        if !force_refresh && let Some(bytes) = self.cache.load_index_raw(&self.spec.id)? {
            return Ok(serde_json::from_slice(&bytes)?);
        }
        let bytes = self.fetcher.fetch(&self.spec.url)?;
        self.cache.save_index(&self.spec.id, &bytes)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// Top-level marketplace operations.
pub struct MarketplaceClient<F: HttpFetcher = ReqwestFetcher> {
    cache: MarketplaceCache,
    fetcher: F,
    trusted_keys: BTreeMap<String, String>,
}

impl MarketplaceClient<ReqwestFetcher> {
    pub fn with_default_fetcher(
        cache: MarketplaceCache,
        trusted_keys: BTreeMap<String, String>,
    ) -> Self {
        Self {
            cache,
            fetcher: ReqwestFetcher::new(),
            trusted_keys,
        }
    }
}

impl<F: HttpFetcher> MarketplaceClient<F> {
    pub fn new(
        cache: MarketplaceCache,
        fetcher: F,
        trusted_keys: BTreeMap<String, String>,
    ) -> Self {
        Self {
            cache,
            fetcher,
            trusted_keys,
        }
    }

    pub fn cache(&self) -> &MarketplaceCache {
        &self.cache
    }

    pub fn registry<'a>(&'a self, spec: RegistrySpec) -> RegistryHandle<'a, F> {
        RegistryHandle {
            spec,
            cache: &self.cache,
            fetcher: &self.fetcher,
        }
    }

    pub fn install(&self, req: InstallRequest) -> Result<InstallOutcome, MarketplaceError> {
        self.cache.ensure_dirs()?;
        let registry = self.registry(req.registry.clone());
        let index = registry.fetch_index(req.refresh_index)?;
        let plugin = index
            .plugins
            .into_iter()
            .find(|p| p.id == req.plugin_id)
            .ok_or_else(|| MarketplaceError::PluginNotFound(req.plugin_id.clone()))?;

        let version = select_version(
            &plugin.versions,
            req.version.as_deref(),
            current_target_triple(),
        )
        .ok_or_else(|| {
            MarketplaceError::NoMatchingVersion(
                plugin.id.clone(),
                current_target_triple().to_string(),
            )
        })?;

        // Download artifact
        let bytes = self.fetcher.fetch(&version.url)?;
        let actual_sha = sha256_hex(&bytes);
        match (&version.sha256, req.allow_unverified) {
            (Some(expected), _) if !expected.eq_ignore_ascii_case(&actual_sha) => {
                return Err(MarketplaceError::Sha256Mismatch {
                    plugin: plugin.id.clone(),
                    expected: expected.clone(),
                    got: actual_sha,
                });
            }
            (None, false) => return Err(MarketplaceError::MissingSha256(plugin.id.clone())),
            _ => {}
        }

        if let Some(signature) = version.signature.as_ref() {
            agena_plugin_host::verify_signature_bytes(&bytes, signature, &self.trusted_keys)
                .map_err(|err| MarketplaceError::SignatureFailed {
                    plugin: plugin.id.clone(),
                    message: err,
                })?;
        } else if req.registry.require_signature {
            return Err(MarketplaceError::SignatureFailed {
                plugin: plugin.id.clone(),
                message: "registry requires a signature, but the version record has none".into(),
            });
        }

        // Lay out artifact on disk
        let artifact_path = self
            .cache
            .artifact_path(&plugin.id, &version.version, version.kind);
        if let Some(parent) = artifact_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_secure_file(&artifact_path, &bytes)?;
        self.cache.save_manifest_snapshot(&plugin.id, &version)?;

        // Update config
        let config_path = req.config_path.clone();
        let mut document = read_or_create_doc(&config_path)?;
        let already_present = plugin_entry_exists(&document, &plugin.id);
        if already_present && !req.force {
            return Err(MarketplaceError::AlreadyInstalled(plugin.id.clone()));
        }
        write_plugin_entry(&mut document, &plugin.id, &version, &artifact_path)?;
        if !req.dry_run {
            write_secure_file(&config_path, document.to_string().as_bytes())?;
        }

        // Update installed.json
        let mut records = self.cache.load_installed()?;
        records.records.insert(
            plugin.id.clone(),
            InstalledRecord {
                plugin_id: plugin.id.clone(),
                version: version.version.clone(),
                kind: version.kind,
                platform: version.platform.clone(),
                binary_path: artifact_path.clone(),
                config_path: config_path.clone(),
                sha256: version.sha256.clone(),
                installed_at: Utc::now(),
            },
        );
        if !req.dry_run {
            self.cache.save_installed(&records)?;
        }

        Ok(InstallOutcome {
            plugin_id: plugin.id,
            version: version.version,
            kind: version.kind,
            artifact_path,
            config_path,
            dry_run: req.dry_run,
        })
    }

    pub fn uninstall(&self, plugin_id: &str) -> Result<UninstallOutcome, MarketplaceError> {
        let mut records = self.cache.load_installed()?;
        let record = records
            .records
            .remove(plugin_id)
            .ok_or_else(|| MarketplaceError::Config(format!("`{plugin_id}` is not installed")))?;
        let mut document = read_or_create_doc(&record.config_path)?;
        remove_plugin_entry(&mut document, plugin_id);
        write_secure_file(&record.config_path, document.to_string().as_bytes())?;
        let plugin_dir = self.cache.plugin_dir(plugin_id, &record.version);
        if plugin_dir.exists() {
            std::fs::remove_dir_all(&plugin_dir)?;
        }
        self.cache.save_installed(&records)?;
        Ok(UninstallOutcome {
            plugin_id: record.plugin_id,
            version: record.version,
            config_path: record.config_path,
        })
    }

    pub fn list_installed(&self) -> Result<Vec<InstalledRecord>, MarketplaceError> {
        Ok(self.cache.load_installed()?.records.into_values().collect())
    }
}

#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub registry: RegistrySpec,
    pub plugin_id: String,
    pub version: Option<String>,
    pub config_path: PathBuf,
    pub force: bool,
    pub dry_run: bool,
    pub allow_unverified: bool,
    pub refresh_index: bool,
}

#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub plugin_id: String,
    pub version: String,
    pub kind: PluginKind,
    pub artifact_path: PathBuf,
    pub config_path: PathBuf,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
pub struct UninstallOutcome {
    pub plugin_id: String,
    pub version: String,
    pub config_path: PathBuf,
}

fn select_version<'a>(
    versions: &'a [PluginVersion],
    requested: Option<&str>,
    target: &str,
) -> Option<PluginVersion> {
    let mut candidates: Vec<&PluginVersion> = versions
        .iter()
        .filter(|v| v.platform == target || v.platform == "any")
        .collect();
    if let Some(req) = requested {
        candidates.retain(|v| v.version == req);
    }
    candidates.sort_by(|a, b| {
        let av = semver::Version::parse(&a.version).ok();
        let bv = semver::Version::parse(&b.version).ok();
        match (av, bv) {
            (Some(a), Some(b)) => b.cmp(&a),
            _ => b.version.cmp(&a.version),
        }
    });
    candidates.first().cloned().cloned()
}

fn current_target_triple() -> &'static str {
    // Best-effort: use `cfg!` to compose a triple. Cargo sets TARGET in
    // build.rs; here we synthesize a reasonable default for common hosts.
    if cfg!(all(target_arch = "x86_64", target_os = "linux")) {
        "x86_64-unknown-linux-gnu"
    } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
        "aarch64-unknown-linux-gnu"
    } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_arch = "x86_64", target_os = "windows")) {
        "x86_64-pc-windows-msvc"
    } else {
        "any"
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn read_or_create_doc(path: &Path) -> Result<DocumentMut, MarketplaceError> {
    if path.exists() {
        let text = std::fs::read_to_string(path)?;
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|e| MarketplaceError::Config(format!("parse {}: {e}", path.display())))?;
        Ok(doc)
    } else {
        Ok(DocumentMut::new())
    }
}

fn ensure_plugins_list_table(doc: &mut DocumentMut) -> &mut Table {
    if !doc.contains_key("plugins") || !doc["plugins"].is_table() {
        doc["plugins"] = Item::Table(Table::new());
    }
    let plugins = doc["plugins"].as_table_mut().expect("plugins table");
    if !plugins.contains_key("list") || !plugins["list"].is_table() {
        plugins["list"] = Item::Table(Table::new());
    }
    plugins["list"].as_table_mut().expect("plugins.list table")
}

fn plugin_entry_exists(doc: &DocumentMut, plugin_id: &str) -> bool {
    doc.get("plugins")
        .and_then(|p| p.as_table())
        .and_then(|t| t.get("list"))
        .and_then(|l| l.as_table())
        .map(|l| l.contains_key(plugin_id))
        .unwrap_or(false)
}

fn write_plugin_entry(
    doc: &mut DocumentMut,
    plugin_id: &str,
    version: &PluginVersion,
    artifact_path: &Path,
) -> Result<(), MarketplaceError> {
    let list = ensure_plugins_list_table(doc);
    let mut table = Table::new();
    table.set_implicit(false);
    table["kind"] = Item::Value(Value::from(version.kind.as_str()));
    match version.kind {
        PluginKind::Cdylib => {
            table["path"] = Item::Value(Value::from(artifact_path.display().to_string()));
            if let Some(sha) = version.sha256.as_ref() {
                table["sha256"] = Item::Value(Value::from(sha.as_str()));
            }
            if let Some(sig) = version.signature.as_ref() {
                let mut inline = InlineTable::new();
                inline.insert("key_id", Value::from(sig.key_id.as_str()));
                inline.insert("signature", Value::from(sig.signature.as_str()));
                table["signature"] = Item::Value(Value::InlineTable(inline));
            }
        }
        PluginKind::Wasm => {
            table["path"] = Item::Value(Value::from(artifact_path.display().to_string()));
            if let Some(sha) = version.sha256.as_ref() {
                table["sha256"] = Item::Value(Value::from(sha.as_str()));
            }
        }
        PluginKind::Stdio => {
            let command = version
                .command
                .clone()
                .unwrap_or_else(|| artifact_path.display().to_string());
            table["command"] = Item::Value(Value::from(command));
            if !version.args.is_empty() {
                let mut arr = Array::new();
                for arg in &version.args {
                    arr.push(arg.as_str());
                }
                table["args"] = Item::Value(Value::Array(arr));
            }
            if let Some(sha) = version.sha256.as_ref() {
                table["sha256"] = Item::Value(Value::from(sha.as_str()));
            }
        }
        PluginKind::Http => {
            table["url"] = Item::Value(Value::from(version.url.as_str()));
        }
    }
    list[plugin_id] = Item::Table(table);
    Ok(())
}

fn remove_plugin_entry(doc: &mut DocumentMut, plugin_id: &str) {
    if let Some(plugins) = doc.get_mut("plugins").and_then(|p| p.as_table_mut())
        && let Some(list) = plugins.get_mut("list").and_then(|l| l.as_table_mut())
    {
        list.remove(plugin_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{PluginRecord, RegistryIndex};
    use std::fs;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct MapFetcher {
        responses: Mutex<BTreeMap<String, Vec<u8>>>,
    }

    impl MapFetcher {
        fn insert(&self, url: &str, bytes: Vec<u8>) {
            self.responses
                .lock()
                .unwrap()
                .insert(url.to_string(), bytes);
        }
    }

    impl HttpFetcher for Arc<MapFetcher> {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, MarketplaceError> {
            self.responses
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .ok_or_else(|| MarketplaceError::Http(format!("no fixture for {url}")))
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "agena-mp-{label}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }

    #[test]
    fn install_writes_config_and_records() {
        let root = temp_root("install");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        // wasm artifact: easy "any" platform
        let wasm_bytes = b"FAKEWASM".to_vec();
        let wasm_sha = sha256_hex(&wasm_bytes);
        fetcher.insert("https://example.com/echo.wasm", wasm_bytes.clone());

        let index = RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "agena-echo".into(),
                name: "Echo".into(),
                description: "Echo plugin".into(),
                homepage: None,
                versions: vec![PluginVersion {
                    version: "0.1.0".into(),
                    kind: PluginKind::Wasm,
                    platform: "any".into(),
                    url: "https://example.com/echo.wasm".into(),
                    sha256: Some(wasm_sha.clone()),
                    signature: None,
                    command: None,
                    args: Vec::new(),
                    env: Default::default(),
                    options: serde_json::Value::Null,
                    min_agena_version: None,
                }],
            }],
        };
        let index_bytes = serde_json::to_vec(&index).unwrap();
        fetcher.insert("https://registry.test/index.json", index_bytes);

        let client = MarketplaceClient::new(cache, Arc::clone(&fetcher), BTreeMap::new());
        let config_path = root.join("config.toml");

        let req = InstallRequest {
            registry: RegistrySpec {
                id: "test".into(),
                url: "https://registry.test/index.json".into(),
                require_signature: false,
            },
            plugin_id: "agena-echo".into(),
            version: None,
            config_path: config_path.clone(),
            force: false,
            dry_run: false,
            allow_unverified: false,
            refresh_index: false,
        };
        let outcome = client.install(req).expect("install ok");
        assert_eq!(outcome.plugin_id, "agena-echo");
        assert_eq!(outcome.version, "0.1.0");
        assert!(outcome.artifact_path.exists());

        // Config should now contain plugins.list.agena-echo
        let written = fs::read_to_string(&config_path).unwrap();
        assert!(written.contains("[plugins.list.agena-echo]"));
        assert!(written.contains("kind = \"wasm\""));
        assert!(written.contains(&wasm_sha));

        // Installed records persisted
        let listed = client.list_installed().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].plugin_id, "agena-echo");

        // Uninstall removes config + cache dir
        let _ = client.uninstall("agena-echo").expect("uninstall ok");
        let after = fs::read_to_string(&config_path).unwrap();
        assert!(!after.contains("[plugins.list.agena-echo]"));
        assert!(!root.join("plugins/agena-echo/0.1.0").exists());
        assert!(client.list_installed().unwrap().is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn install_rejects_sha256_mismatch() {
        let root = temp_root("sha-mismatch");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());
        fetcher.insert("https://example.com/echo.wasm", b"FAKE".to_vec());
        let index = RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "demo".into(),
                name: String::new(),
                description: String::new(),
                homepage: None,
                versions: vec![PluginVersion {
                    version: "0.1.0".into(),
                    kind: PluginKind::Wasm,
                    platform: "any".into(),
                    url: "https://example.com/echo.wasm".into(),
                    sha256: Some("00".into()),
                    signature: None,
                    command: None,
                    args: Vec::new(),
                    env: Default::default(),
                    options: serde_json::Value::Null,
                    min_agena_version: None,
                }],
            }],
        };
        fetcher.insert(
            "https://registry.test/index.json",
            serde_json::to_vec(&index).unwrap(),
        );

        let client = MarketplaceClient::new(cache, Arc::clone(&fetcher), BTreeMap::new());
        let err = client
            .install(InstallRequest {
                registry: RegistrySpec {
                    id: "test".into(),
                    url: "https://registry.test/index.json".into(),
                    require_signature: false,
                },
                plugin_id: "demo".into(),
                version: None,
                config_path: root.join("config.toml"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .unwrap_err();
        assert!(matches!(err, MarketplaceError::Sha256Mismatch { .. }));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn install_requires_sha256_unless_allow_unverified() {
        let root = temp_root("no-sha");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());
        fetcher.insert("https://example.com/echo.wasm", b"BYTES".to_vec());
        let index = RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "demo".into(),
                name: String::new(),
                description: String::new(),
                homepage: None,
                versions: vec![PluginVersion {
                    version: "0.1.0".into(),
                    kind: PluginKind::Wasm,
                    platform: "any".into(),
                    url: "https://example.com/echo.wasm".into(),
                    sha256: None,
                    signature: None,
                    command: None,
                    args: Vec::new(),
                    env: Default::default(),
                    options: serde_json::Value::Null,
                    min_agena_version: None,
                }],
            }],
        };
        fetcher.insert(
            "https://registry.test/index.json",
            serde_json::to_vec(&index).unwrap(),
        );

        let client = MarketplaceClient::new(cache, Arc::clone(&fetcher), BTreeMap::new());
        let req = InstallRequest {
            registry: RegistrySpec {
                id: "test".into(),
                url: "https://registry.test/index.json".into(),
                require_signature: false,
            },
            plugin_id: "demo".into(),
            version: None,
            config_path: root.join("config.toml"),
            force: false,
            dry_run: false,
            allow_unverified: false,
            refresh_index: false,
        };
        let err = client.install(req.clone()).unwrap_err();
        assert!(matches!(err, MarketplaceError::MissingSha256(_)));

        let req = InstallRequest {
            allow_unverified: true,
            ..req
        };
        client.install(req).expect("allow-unverified install");
        let _ = fs::remove_dir_all(&root);
    }
}
