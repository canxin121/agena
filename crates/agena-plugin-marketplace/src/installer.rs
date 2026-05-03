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

        // Lay out artifact on disk. Archive payloads are extracted under the
        // plugin/version dir; non-archive payloads go straight to a single
        // file named binary.<ext>.
        let plugin_dir = self.cache.plugin_dir(&plugin.id, &version.version);
        std::fs::create_dir_all(&plugin_dir)?;
        let (artifact_path, archive_extracted) = match version.archive.as_ref() {
            Some(crate::manifest::ArchiveSpec::TarGz { entrypoint }) => {
                extract_tar_gz(&bytes, &plugin_dir).map_err(|err| MarketplaceError::Archive {
                    plugin: plugin.id.clone(),
                    message: err,
                })?;
                let entrypoint_path = plugin_dir.join(entrypoint);
                if !entrypoint_path.exists() {
                    return Err(MarketplaceError::Archive {
                        plugin: plugin.id.clone(),
                        message: format!("entrypoint `{entrypoint}` not found in archive"),
                    });
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if matches!(version.kind, PluginKind::Stdio) {
                        let perms = std::fs::Permissions::from_mode(0o755);
                        let _ = std::fs::set_permissions(&entrypoint_path, perms);
                    }
                }
                (entrypoint_path, true)
            }
            None => {
                let path = self
                    .cache
                    .artifact_path(&plugin.id, &version.version, version.kind);
                write_secure_file(&path, &bytes)?;
                (path, false)
            }
        };
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
                registry_id: req.registry.id.clone(),
                registry_url: req.registry.url.clone(),
                archive_extracted,
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

    /// Re-resolve `plugin_id` against its recorded registry and reinstall if a
    /// strictly newer semver version is available. Returns the outcome
    /// describing whether anything changed.
    pub fn upgrade(
        &self,
        plugin_id: &str,
        registry_override: Option<RegistrySpec>,
    ) -> Result<UpgradeOutcome, MarketplaceError> {
        let installed = self.cache.load_installed()?;
        let record =
            installed.records.get(plugin_id).cloned().ok_or_else(|| {
                MarketplaceError::Config(format!("`{plugin_id}` is not installed"))
            })?;
        let registry = registry_override
            .or_else(|| {
                if record.registry_url.is_empty() {
                    None
                } else {
                    Some(RegistrySpec {
                        id: if record.registry_id.is_empty() {
                            "default".into()
                        } else {
                            record.registry_id.clone()
                        },
                        url: record.registry_url.clone(),
                        require_signature: false,
                    })
                }
            })
            .ok_or_else(|| {
                MarketplaceError::Config(format!(
                    "`{plugin_id}` has no recorded registry url; pass --registry to upgrade"
                ))
            })?;

        let handle = self.registry(registry.clone());
        let index = handle.fetch_index(true)?;
        let plugin = index
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| MarketplaceError::PluginNotFound(plugin_id.to_string()))?;
        let candidate = select_version(&plugin.versions, None, current_target_triple())
            .ok_or_else(|| {
                MarketplaceError::NoMatchingVersion(
                    plugin_id.to_string(),
                    current_target_triple().to_string(),
                )
            })?;

        if !is_newer(&candidate.version, &record.version) {
            return Ok(UpgradeOutcome {
                plugin_id: plugin_id.to_string(),
                previous_version: record.version.clone(),
                installed_version: record.version,
                upgraded: false,
                outcome: None,
            });
        }

        let outcome = self.install(InstallRequest {
            registry,
            plugin_id: plugin_id.to_string(),
            version: Some(candidate.version.clone()),
            config_path: record.config_path.clone(),
            force: true,
            dry_run: false,
            allow_unverified: record.sha256.is_none(),
            refresh_index: false,
        })?;

        Ok(UpgradeOutcome {
            plugin_id: plugin_id.to_string(),
            previous_version: record.version,
            installed_version: outcome.version.clone(),
            upgraded: true,
            outcome: Some(outcome),
        })
    }

    /// Iterate over all installed records and report those that have a newer
    /// version available on their registry. Records with no `registry_url`
    /// are skipped silently.
    pub fn list_outdated(&self) -> Result<Vec<OutdatedRecord>, MarketplaceError> {
        let installed = self.cache.load_installed()?;
        let mut out = Vec::new();
        for (_, record) in installed.records {
            if record.registry_url.is_empty() {
                continue;
            }
            let registry = RegistrySpec {
                id: if record.registry_id.is_empty() {
                    "default".into()
                } else {
                    record.registry_id.clone()
                },
                url: record.registry_url.clone(),
                require_signature: false,
            };
            let handle = self.registry(registry);
            let index = match handle.fetch_index(false) {
                Ok(idx) => idx,
                Err(err) => {
                    tracing::warn!(plugin = %record.plugin_id, error = %err, "skip outdated check");
                    continue;
                }
            };
            let Some(plugin) = index.plugins.iter().find(|p| p.id == record.plugin_id) else {
                continue;
            };
            let Some(latest) = select_version(&plugin.versions, None, current_target_triple())
            else {
                continue;
            };
            if is_newer(&latest.version, &record.version) {
                out.push(OutdatedRecord {
                    plugin_id: record.plugin_id.clone(),
                    installed_version: record.version.clone(),
                    latest_version: latest.version.clone(),
                });
            }
        }
        out.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
        Ok(out)
    }
}

fn is_newer(candidate: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(installed),
    ) {
        (Ok(c), Ok(i)) => c > i,
        _ => candidate != installed,
    }
}

fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    archive.set_preserve_permissions(true);
    archive.set_overwrite(true);
    for entry in archive
        .entries()
        .map_err(|e| format!("read tar entries: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let entry_path = entry
            .path()
            .map_err(|e| format!("entry path: {e}"))?
            .into_owned();
        for component in entry_path.components() {
            use std::path::Component;
            match component {
                Component::Normal(_) | Component::CurDir => {}
                _ => {
                    return Err(format!(
                        "archive contains unsafe path: {}",
                        entry_path.display()
                    ));
                }
            }
        }
        entry
            .unpack_in(dest)
            .map_err(|e| format!("unpack {}: {e}", entry_path.display()))?;
    }
    Ok(())
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

#[derive(Debug, Clone)]
pub struct UpgradeOutcome {
    pub plugin_id: String,
    pub previous_version: String,
    pub installed_version: String,
    pub upgraded: bool,
    pub outcome: Option<InstallOutcome>,
}

#[derive(Debug, Clone)]
pub struct OutdatedRecord {
    pub plugin_id: String,
    pub installed_version: String,
    pub latest_version: String,
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
                    archive: None,
                    dependencies: Vec::new(),
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
                    archive: None,
                    dependencies: Vec::new(),
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
                    archive: None,
                    dependencies: Vec::new(),
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

    fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let buf: Vec<u8> = Vec::new();
        let encoder = flate2::write::GzEncoder::new(buf, flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, name, *contents)
                .expect("append");
        }
        let encoder = builder.into_inner().expect("finish builder");
        let mut out = encoder.finish().expect("finish gz");
        out.flush().ok();
        out
    }

    #[test]
    fn install_extracts_tar_gz_archive() {
        let root = temp_root("archive");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let archive = build_tar_gz(&[
            ("bin/agent-tool", b"#!/bin/sh\necho hi\n"),
            ("README", b"hello\n"),
        ]);
        let archive_sha = sha256_hex(&archive);
        fetcher.insert("https://example.com/bundle.tar.gz", archive.clone());

        let index = RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "bundle".into(),
                name: String::new(),
                description: String::new(),
                homepage: None,
                versions: vec![PluginVersion {
                    version: "0.1.0".into(),
                    kind: PluginKind::Stdio,
                    platform: "any".into(),
                    url: "https://example.com/bundle.tar.gz".into(),
                    sha256: Some(archive_sha),
                    signature: None,
                    command: None,
                    args: Vec::new(),
                    env: Default::default(),
                    options: serde_json::Value::Null,
                    min_agena_version: None,
                    archive: Some(crate::manifest::ArchiveSpec::TarGz {
                        entrypoint: "bin/agent-tool".into(),
                    }),
                    dependencies: Vec::new(),
                }],
            }],
        };
        fetcher.insert(
            "https://registry.test/index.json",
            serde_json::to_vec(&index).unwrap(),
        );
        let client = MarketplaceClient::new(cache, Arc::clone(&fetcher), BTreeMap::new());
        let outcome = client
            .install(InstallRequest {
                registry: RegistrySpec {
                    id: "test".into(),
                    url: "https://registry.test/index.json".into(),
                    require_signature: false,
                },
                plugin_id: "bundle".into(),
                version: None,
                config_path: root.join("config.toml"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect("install");
        assert!(outcome.artifact_path.ends_with("bin/agent-tool"));
        assert!(outcome.artifact_path.exists());
        assert!(root.join("plugins/bundle/0.1.0/README").exists());

        let listed = client.list_installed().unwrap();
        assert!(listed[0].archive_extracted);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn upgrade_replaces_when_newer_version_available() {
        let root = temp_root("upgrade");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let v1 = b"V1".to_vec();
        let v2 = b"V2".to_vec();
        let v1_sha = sha256_hex(&v1);
        let v2_sha = sha256_hex(&v2);
        fetcher.insert("https://example.com/p-0.1.0.wasm", v1);
        fetcher.insert("https://example.com/p-0.2.0.wasm", v2);

        let mk_index = |versions: Vec<PluginVersion>| RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "p".into(),
                name: String::new(),
                description: String::new(),
                homepage: None,
                versions,
            }],
        };
        let v1_record = PluginVersion {
            version: "0.1.0".into(),
            kind: PluginKind::Wasm,
            platform: "any".into(),
            url: "https://example.com/p-0.1.0.wasm".into(),
            sha256: Some(v1_sha.clone()),
            signature: None,
            command: None,
            args: Vec::new(),
            env: Default::default(),
            options: serde_json::Value::Null,
            min_agena_version: None,
            archive: None,
            dependencies: Vec::new(),
        };
        let v2_record = PluginVersion {
            version: "0.2.0".into(),
            url: "https://example.com/p-0.2.0.wasm".into(),
            sha256: Some(v2_sha.clone()),
            ..v1_record.clone()
        };
        // Initial index: only v1
        fetcher.insert(
            "https://registry.test/index.json",
            serde_json::to_vec(&mk_index(vec![v1_record.clone()])).unwrap(),
        );

        let client = MarketplaceClient::new(cache, Arc::clone(&fetcher), BTreeMap::new());
        let registry = RegistrySpec {
            id: "test".into(),
            url: "https://registry.test/index.json".into(),
            require_signature: false,
        };
        client
            .install(InstallRequest {
                registry: registry.clone(),
                plugin_id: "p".into(),
                version: None,
                config_path: root.join("config.toml"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect("initial install");

        // Upgrade with same index: nothing to do.
        let no_op = client.upgrade("p", None).expect("upgrade no-op");
        assert!(!no_op.upgraded);

        // Refresh index with v2 added.
        fetcher.insert(
            "https://registry.test/index.json",
            serde_json::to_vec(&mk_index(vec![v1_record, v2_record])).unwrap(),
        );
        let result = client.upgrade("p", None).expect("upgrade run");
        assert!(result.upgraded);
        assert_eq!(result.previous_version, "0.1.0");
        assert_eq!(result.installed_version, "0.2.0");

        let outdated = client.list_outdated().expect("outdated empty");
        assert!(outdated.is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
