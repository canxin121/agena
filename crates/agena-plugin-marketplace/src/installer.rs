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
        self.fetch_index_with_cache(force_refresh, true)
    }

    fn fetch_index_with_cache(
        &self,
        force_refresh: bool,
        persist_cache: bool,
    ) -> Result<RegistryIndex, MarketplaceError> {
        if !force_refresh && let Some(bytes) = self.cache.load_index_raw(&self.spec.id)? {
            return Ok(serde_json::from_slice(&bytes)?);
        }
        let bytes = self.fetcher.fetch(&self.spec.url)?;
        if persist_cache {
            self.cache.save_index(&self.spec.id, &bytes)?;
        }
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
        // Build the dependency plan first: any DependencySpec in the
        // resolved version is followed transitively through the same
        // registry, with cycle detection. Each dependency is installed
        // before its dependents.
        if !req.dry_run {
            self.cache.ensure_dirs()?;
        }
        let registry_handle = self.registry(req.registry.clone());
        let index = registry_handle.fetch_index_with_cache(req.refresh_index, !req.dry_run)?;
        let installed = self.cache.load_installed()?;

        let mut plan: Vec<(String, Option<String>)> = Vec::new();
        let mut visiting: std::collections::BTreeSet<String> = Default::default();
        let mut visited: std::collections::BTreeSet<String> = Default::default();
        self.collect_install_plan(
            &req.plugin_id,
            req.version.clone(),
            &index,
            &installed,
            &mut visiting,
            &mut visited,
            &mut plan,
        )?;

        // Snapshot installed.json + the user's config.json before mutating
        // anything so we can roll back if any step in the plan fails.
        let txn = InstallTransaction::begin(&self.cache, &req.config_path)?;

        let mut last: Option<InstallOutcome> = None;
        for (plugin_id, version_req) in plan {
            let is_root = plugin_id == req.plugin_id;
            let mut sub = req.clone();
            sub.plugin_id = plugin_id.clone();
            sub.version = version_req.clone();
            if !is_root {
                sub.force = false;
            }
            match self.install_one(sub) {
                Ok(outcome) => last = Some(outcome),
                Err(err) => {
                    if let Err(rollback_err) = txn.rollback(&self.cache) {
                        tracing::warn!(
                            error = %rollback_err,
                            "rollback failed after install error: {err}"
                        );
                    }
                    return Err(err);
                }
            }
        }
        txn.commit();
        last.ok_or_else(|| MarketplaceError::Config("install plan was empty".into()))
    }

    fn collect_install_plan(
        &self,
        plugin_id: &str,
        version_req: Option<String>,
        index: &RegistryIndex,
        installed: &crate::cache::InstalledRecords,
        visiting: &mut std::collections::BTreeSet<String>,
        visited: &mut std::collections::BTreeSet<String>,
        out: &mut Vec<(String, Option<String>)>,
    ) -> Result<(), MarketplaceError> {
        if visited.contains(plugin_id) {
            return Ok(());
        }
        if !visiting.insert(plugin_id.to_string()) {
            return Err(MarketplaceError::CircularDependency(plugin_id.to_string()));
        }
        let plugin = index
            .plugins
            .iter()
            .find(|p| p.id == plugin_id)
            .ok_or_else(|| MarketplaceError::PluginNotFound(plugin_id.to_string()))?;
        let version = select_version(
            &plugin.versions,
            version_req.as_deref(),
            current_target_triple(),
        )
        .ok_or_else(|| {
            MarketplaceError::NoMatchingVersion(
                plugin_id.to_string(),
                current_target_triple().to_string(),
            )
        })?;
        for dep in &version.dependencies {
            // Skip already-installed deps that satisfy the requirement.
            if let Some(record) = installed.records.get(&dep.plugin_id)
                && dep_satisfied(&record.version, &dep.version_req)
            {
                visited.insert(dep.plugin_id.clone());
                continue;
            }
            // Otherwise recurse, picking the highest matching version.
            let dep_version = pick_version_for_req(index, &dep.plugin_id, &dep.version_req)
                .ok_or_else(|| {
                    MarketplaceError::MissingDependency(
                        dep.plugin_id.clone(),
                        plugin_id.to_string(),
                    )
                })?;
            self.collect_install_plan(
                &dep.plugin_id,
                Some(dep_version),
                index,
                installed,
                visiting,
                visited,
                out,
            )?;
        }
        visiting.remove(plugin_id);
        visited.insert(plugin_id.to_string());
        out.push((plugin_id.to_string(), version_req));
        Ok(())
    }

    fn install_one(&self, req: InstallRequest) -> Result<InstallOutcome, MarketplaceError> {
        let registry = self.registry(req.registry.clone());
        let index = registry.fetch_index_with_cache(false, !req.dry_run)?;
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

        if !req.dry_run {
            self.cache.ensure_dirs()?;
        }

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
        let (artifact_path, archive_extracted) = if req.dry_run {
            preview_artifact_path(&self.cache, &plugin.id, &version)?
        } else {
            std::fs::create_dir_all(&plugin_dir)?;
            match version.archive.as_ref() {
                Some(crate::manifest::ArchiveSpec::TarGz { entrypoint }) => {
                    extract_tar_gz(&bytes, &plugin_dir).map_err(|err| {
                        MarketplaceError::Archive {
                            plugin: plugin.id.clone(),
                            message: err,
                        }
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
            }
        };
        if !req.dry_run {
            self.cache.save_manifest_snapshot(&plugin.id, &version)?;
        }

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
        self.uninstall_with(plugin_id, false)
            .map(|outs| outs.into_iter().next().expect("at least one"))
    }

    /// Uninstall `plugin_id`. With `cascade=false`, refuses if any other
    /// installed plugin depends on it (the list is computed from manifest
    /// snapshots saved at install time). With `cascade=true`, uninstalls
    /// downstream dependents first, then the requested plugin.
    pub fn uninstall_with(
        &self,
        plugin_id: &str,
        cascade: bool,
    ) -> Result<Vec<UninstallOutcome>, MarketplaceError> {
        let dependents = self.find_dependents(plugin_id)?;
        if !dependents.is_empty() && !cascade {
            return Err(MarketplaceError::RequiredByOthers {
                plugin: plugin_id.to_string(),
                dependents,
            });
        }
        let mut out = Vec::new();
        for dep in dependents {
            out.push(self.uninstall_one(&dep)?);
        }
        out.push(self.uninstall_one(plugin_id)?);
        Ok(out)
    }

    fn uninstall_one(&self, plugin_id: &str) -> Result<UninstallOutcome, MarketplaceError> {
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

    /// Look at every installed plugin's manifest snapshot and report any
    /// that depend on `plugin_id`.
    fn find_dependents(&self, plugin_id: &str) -> Result<Vec<String>, MarketplaceError> {
        let installed = self.cache.load_installed()?;
        let mut deps = Vec::new();
        for record in installed.records.values() {
            if record.plugin_id == plugin_id {
                continue;
            }
            let snapshot_path = self
                .cache
                .manifest_snapshot_path(&record.plugin_id, &record.version);
            if !snapshot_path.exists() {
                continue;
            }
            let bytes = std::fs::read(&snapshot_path)?;
            let version: PluginVersion = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if version
                .dependencies
                .iter()
                .any(|d| d.plugin_id == plugin_id)
            {
                deps.push(record.plugin_id.clone());
            }
        }
        deps.sort();
        Ok(deps)
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
            let index = match handle.fetch_index(true) {
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

fn dep_satisfied(installed_version: &str, req: &str) -> bool {
    match (
        semver::Version::parse(installed_version),
        semver::VersionReq::parse(req),
    ) {
        (Ok(v), Ok(r)) => r.matches(&v),
        // If either side fails to parse, fall back to literal equality.
        _ => installed_version == req,
    }
}

/// Pick the highest version of `plugin_id` matching `req` in `index`,
/// platform-filtered. Returns the version string or None.
fn pick_version_for_req(index: &RegistryIndex, plugin_id: &str, req: &str) -> Option<String> {
    let plugin = index.plugins.iter().find(|p| p.id == plugin_id)?;
    let parsed_req = semver::VersionReq::parse(req).ok();
    let target = current_target_triple();
    let mut candidates: Vec<&PluginVersion> = plugin
        .versions
        .iter()
        .filter(|v| v.platform == target || v.platform == "any")
        .filter(
            |v| match (&parsed_req, semver::Version::parse(&v.version).ok()) {
                (Some(r), Some(parsed)) => r.matches(&parsed),
                _ => v.version == req,
            },
        )
        .collect();
    candidates.sort_by(|a, b| {
        let av = semver::Version::parse(&a.version).ok();
        let bv = semver::Version::parse(&b.version).ok();
        match (av, bv) {
            (Some(a), Some(b)) => b.cmp(&a),
            _ => b.version.cmp(&a.version),
        }
    });
    candidates.first().map(|v| v.version.clone())
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

fn preview_artifact_path(
    cache: &MarketplaceCache,
    plugin_id: &str,
    version: &PluginVersion,
) -> Result<(PathBuf, bool), MarketplaceError> {
    match version.archive.as_ref() {
        Some(crate::manifest::ArchiveSpec::TarGz { entrypoint }) => {
            validate_archive_entrypoint(entrypoint, plugin_id)?;
            Ok((
                cache
                    .plugin_dir(plugin_id, &version.version)
                    .join(entrypoint),
                true,
            ))
        }
        None => Ok((
            cache.artifact_path(plugin_id, &version.version, version.kind),
            false,
        )),
    }
}

fn validate_archive_entrypoint(entrypoint: &str, plugin_id: &str) -> Result<(), MarketplaceError> {
    let path = Path::new(entrypoint);
    if entrypoint.trim().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(MarketplaceError::Archive {
            plugin: plugin_id.to_string(),
            message: format!("entrypoint `{entrypoint}` is not a safe relative path"),
        });
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

fn select_version(
    versions: &[PluginVersion],
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

/// Transactional snapshot of `installed.json` + the target config file.
/// Created at the start of `install`, committed on success, rolled back
/// on any error mid-plan so a partial dependency install can't leave
/// the system in a half-applied state.
struct InstallTransaction {
    installed_snapshot: Option<Vec<u8>>,
    config_path: PathBuf,
    config_snapshot: Option<Vec<u8>>,
    armed: std::cell::Cell<bool>,
}

impl InstallTransaction {
    fn begin(cache: &MarketplaceCache, config_path: &Path) -> Result<Self, MarketplaceError> {
        let installed_path = cache.installed_path();
        let installed_snapshot = if installed_path.exists() {
            Some(std::fs::read(&installed_path)?)
        } else {
            None
        };
        let config_snapshot = if config_path.exists() {
            Some(std::fs::read(config_path)?)
        } else {
            None
        };
        Ok(Self {
            installed_snapshot,
            config_path: config_path.to_path_buf(),
            config_snapshot,
            armed: std::cell::Cell::new(true),
        })
    }

    fn rollback(&self, cache: &MarketplaceCache) -> Result<(), MarketplaceError> {
        let installed_path = cache.installed_path();
        match &self.installed_snapshot {
            Some(bytes) => write_secure_file(&installed_path, bytes)?,
            None => {
                if installed_path.exists() {
                    std::fs::remove_file(&installed_path)?;
                }
            }
        }
        match &self.config_snapshot {
            Some(bytes) => write_secure_file(&self.config_path, bytes)?,
            None => {
                if self.config_path.exists() {
                    std::fs::remove_file(&self.config_path)?;
                }
            }
        }
        self.armed.set(false);
        Ok(())
    }

    fn commit(self) {
        self.armed.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{DependencySpec, PluginRecord, RegistryIndex};
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

    fn wasm_version(
        version: &str,
        url: &str,
        sha256: &str,
        dependencies: Vec<DependencySpec>,
    ) -> PluginVersion {
        PluginVersion {
            version: version.into(),
            kind: PluginKind::Wasm,
            platform: "any".into(),
            url: url.into(),
            sha256: Some(sha256.into()),
            signature: None,
            command: None,
            args: Vec::new(),
            env: Default::default(),
            options: serde_json::Value::Null,
            min_agena_version: None,
            archive: None,
            dependencies,
        }
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
        let config_path = root.join("config.json");

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
                config_path: root.join("config.json"),
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
            config_path: root.join("config.json"),
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
                config_path: root.join("config.json"),
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
                config_path: root.join("config.json"),
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

    #[test]
    fn install_pulls_in_dependencies_and_uninstall_blocks_when_required() {
        use crate::manifest::DependencySpec;

        let root = temp_root("deps");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let lib_bytes = b"LIB".to_vec();
        let app_bytes = b"APP".to_vec();
        let lib_sha = sha256_hex(&lib_bytes);
        let app_sha = sha256_hex(&app_bytes);
        fetcher.insert("https://example.com/lib.wasm", lib_bytes);
        fetcher.insert("https://example.com/app.wasm", app_bytes);

        let lib = PluginVersion {
            version: "0.1.0".into(),
            kind: PluginKind::Wasm,
            platform: "any".into(),
            url: "https://example.com/lib.wasm".into(),
            sha256: Some(lib_sha),
            signature: None,
            command: None,
            args: Vec::new(),
            env: Default::default(),
            options: serde_json::Value::Null,
            min_agena_version: None,
            archive: None,
            dependencies: Vec::new(),
        };
        let app = PluginVersion {
            version: "0.2.0".into(),
            url: "https://example.com/app.wasm".into(),
            sha256: Some(app_sha),
            dependencies: vec![DependencySpec {
                plugin_id: "lib".into(),
                version_req: "^0.1".into(),
            }],
            ..lib.clone()
        };
        let index = RegistryIndex {
            version: 1,
            plugins: vec![
                PluginRecord {
                    id: "lib".into(),
                    name: String::new(),
                    description: String::new(),
                    homepage: None,
                    versions: vec![lib],
                },
                PluginRecord {
                    id: "app".into(),
                    name: String::new(),
                    description: String::new(),
                    homepage: None,
                    versions: vec![app],
                },
            ],
        };
        fetcher.insert(
            "https://registry.test/index.json",
            serde_json::to_vec(&index).unwrap(),
        );

        let client = MarketplaceClient::new(cache, Arc::clone(&fetcher), BTreeMap::new());
        let registry = RegistrySpec {
            id: "test".into(),
            url: "https://registry.test/index.json".into(),
            require_signature: false,
        };

        // Installing `app` must transitively install `lib` first.
        client
            .install(InstallRequest {
                registry: registry.clone(),
                plugin_id: "app".into(),
                version: None,
                config_path: root.join("config.json"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect("install app + lib");
        let installed: Vec<_> = client
            .list_installed()
            .expect("list")
            .into_iter()
            .map(|r| r.plugin_id)
            .collect();
        assert!(installed.contains(&"lib".to_string()));
        assert!(installed.contains(&"app".to_string()));

        // Plain uninstall of `lib` is rejected because `app` depends on it.
        let err = client.uninstall("lib").expect_err("dependent blocks");
        assert!(matches!(err, MarketplaceError::RequiredByOthers { .. }));

        // Cascade removes both.
        let outs = client
            .uninstall_with("lib", true)
            .expect("cascade uninstall");
        let removed_ids: Vec<_> = outs.into_iter().map(|o| o.plugin_id).collect();
        assert!(removed_ids.contains(&"lib".to_string()));
        assert!(removed_ids.contains(&"app".to_string()));
        assert!(client.list_installed().unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn install_reports_missing_dependency_before_writing_state() {
        let root = temp_root("missing-dep");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());
        let index = RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "app".into(),
                name: String::new(),
                description: String::new(),
                homepage: None,
                versions: vec![wasm_version(
                    "1.0.0",
                    "https://example.com/app.wasm",
                    "00",
                    vec![DependencySpec {
                        plugin_id: "missing-lib".into(),
                        version_req: "^1".into(),
                    }],
                )],
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
                plugin_id: "app".into(),
                version: None,
                config_path: root.join("config.json"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect_err("missing dependency should fail");

        assert!(matches!(
            err,
            MarketplaceError::MissingDependency(dep, requested_by)
                if dep == "missing-lib" && requested_by == "app"
        ));
        assert!(client.list_installed().unwrap().is_empty());
        assert!(!root.join("config.json").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn install_reports_circular_dependency_before_writing_state() {
        let root = temp_root("circular-dep");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());
        let index = RegistryIndex {
            version: 1,
            plugins: vec![
                PluginRecord {
                    id: "app".into(),
                    name: String::new(),
                    description: String::new(),
                    homepage: None,
                    versions: vec![wasm_version(
                        "1.0.0",
                        "https://example.com/app.wasm",
                        "00",
                        vec![DependencySpec {
                            plugin_id: "lib".into(),
                            version_req: "^1".into(),
                        }],
                    )],
                },
                PluginRecord {
                    id: "lib".into(),
                    name: String::new(),
                    description: String::new(),
                    homepage: None,
                    versions: vec![wasm_version(
                        "1.0.0",
                        "https://example.com/lib.wasm",
                        "00",
                        vec![DependencySpec {
                            plugin_id: "app".into(),
                            version_req: "^1".into(),
                        }],
                    )],
                },
            ],
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
                plugin_id: "app".into(),
                version: None,
                config_path: root.join("config.json"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect_err("cycle should fail");

        assert!(matches!(err, MarketplaceError::CircularDependency(plugin) if plugin == "app"));
        assert!(client.list_installed().unwrap().is_empty());
        assert!(!root.join("config.json").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn install_rolls_back_when_a_step_fails() {
        // Plan: app depends on lib. lib's artifact URL has no fixture so
        // the lib install fails partway through; the user-facing config.json
        // and installed.json must remain untouched.
        use crate::manifest::DependencySpec;
        let root = temp_root("rollback");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let app_bytes = b"APP".to_vec();
        let app_sha = sha256_hex(&app_bytes);
        fetcher.insert("https://example.com/app.wasm", app_bytes);
        // Deliberately do NOT register lib.wasm — fetch will fail.

        let lib = PluginVersion {
            version: "0.1.0".into(),
            kind: PluginKind::Wasm,
            platform: "any".into(),
            url: "https://example.com/lib.wasm".into(),
            sha256: Some("00".into()),
            signature: None,
            command: None,
            args: Vec::new(),
            env: Default::default(),
            options: serde_json::Value::Null,
            min_agena_version: None,
            archive: None,
            dependencies: Vec::new(),
        };
        let app = PluginVersion {
            url: "https://example.com/app.wasm".into(),
            sha256: Some(app_sha),
            dependencies: vec![DependencySpec {
                plugin_id: "lib".into(),
                version_req: "^0.1".into(),
            }],
            ..lib.clone()
        };
        let index = RegistryIndex {
            version: 1,
            plugins: vec![
                PluginRecord {
                    id: "lib".into(),
                    name: String::new(),
                    description: String::new(),
                    homepage: None,
                    versions: vec![lib],
                },
                PluginRecord {
                    id: "app".into(),
                    name: String::new(),
                    description: String::new(),
                    homepage: None,
                    versions: vec![app],
                },
            ],
        };
        fetcher.insert(
            "https://registry.test/index.json",
            serde_json::to_vec(&index).unwrap(),
        );

        let client = MarketplaceClient::new(cache, Arc::clone(&fetcher), BTreeMap::new());
        let config_path = root.join("config.json");
        // Pre-existing config we expect the rollback to restore byte-for-byte.
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&config_path, b"# preexisting\n").unwrap();

        let err = client
            .install(InstallRequest {
                registry: RegistrySpec {
                    id: "test".into(),
                    url: "https://registry.test/index.json".into(),
                    require_signature: false,
                },
                plugin_id: "app".into(),
                version: None,
                config_path: config_path.clone(),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect_err("lib download should fail");
        assert!(matches!(err, MarketplaceError::Http(_)));

        // Config restored to its pre-install bytes.
        let after = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(after, "# preexisting\n");
        // installed.json must not contain either plugin.
        let listed = client.list_installed().unwrap();
        assert!(listed.is_empty(), "installed records leaked: {listed:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dry_run_reports_paths_without_writing_any_state() {
        let root = temp_root("dry-run");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let wasm_bytes = b"DRYRUN".to_vec();
        let wasm_sha = sha256_hex(&wasm_bytes);
        fetcher.insert("https://example.com/dry-run.wasm", wasm_bytes);
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
                    url: "https://example.com/dry-run.wasm".into(),
                    sha256: Some(wasm_sha),
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

        let client = MarketplaceClient::new(cache.clone(), Arc::clone(&fetcher), BTreeMap::new());
        let config_path = root.join("config.json");
        let outcome = client
            .install(InstallRequest {
                registry: RegistrySpec {
                    id: "test".into(),
                    url: "https://registry.test/index.json".into(),
                    require_signature: false,
                },
                plugin_id: "demo".into(),
                version: None,
                config_path: config_path.clone(),
                force: false,
                dry_run: true,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect("dry run succeeds");

        assert!(outcome.dry_run);
        assert_eq!(
            outcome.artifact_path,
            cache.artifact_path("demo", "0.1.0", PluginKind::Wasm)
        );
        assert!(
            !root.exists(),
            "dry run should not create cache directories"
        );
        assert!(!config_path.exists(), "dry run should not write config");
        assert!(client.list_installed().unwrap().is_empty());
    }

    #[test]
    fn dry_run_rejects_unsafe_archive_entrypoint_without_writing() {
        let root = temp_root("dry-run-archive");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let archive = build_tar_gz(&[("bin/agent-tool", b"#!/bin/sh\necho hi\n")]);
        let archive_sha = sha256_hex(&archive);
        fetcher.insert("https://example.com/bundle.tar.gz", archive);
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
                        entrypoint: "../escape".into(),
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
        let err = client
            .install(InstallRequest {
                registry: RegistrySpec {
                    id: "test".into(),
                    url: "https://registry.test/index.json".into(),
                    require_signature: false,
                },
                plugin_id: "bundle".into(),
                version: None,
                config_path: root.join("config.json"),
                force: false,
                dry_run: true,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect_err("unsafe archive path should fail");

        assert!(matches!(err, MarketplaceError::Archive { .. }));
        assert!(!root.exists(), "dry run failure should not create files");
    }

    #[test]
    fn install_rejects_missing_signature_when_registry_requires_it() {
        let root = temp_root("require-signature");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let wasm_bytes = b"SIGLESS".to_vec();
        let wasm_sha = sha256_hex(&wasm_bytes);
        fetcher.insert("https://example.com/sigless.wasm", wasm_bytes);
        let index = RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "sigless".into(),
                name: String::new(),
                description: String::new(),
                homepage: None,
                versions: vec![PluginVersion {
                    version: "1.0.0".into(),
                    kind: PluginKind::Wasm,
                    platform: "any".into(),
                    url: "https://example.com/sigless.wasm".into(),
                    sha256: Some(wasm_sha),
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
                    require_signature: true,
                },
                plugin_id: "sigless".into(),
                version: None,
                config_path: root.join("config.json"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect_err("missing signature should fail");

        assert!(matches!(err, MarketplaceError::SignatureFailed { .. }));
    }

    #[test]
    fn install_rejects_already_installed_without_force() {
        let root = temp_root("already-installed");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let wasm_bytes = b"REINSTALL".to_vec();
        let wasm_sha = sha256_hex(&wasm_bytes);
        fetcher.insert("https://example.com/reinstall.wasm", wasm_bytes);
        let index = RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "demo".into(),
                name: String::new(),
                description: String::new(),
                homepage: None,
                versions: vec![PluginVersion {
                    version: "1.0.0".into(),
                    kind: PluginKind::Wasm,
                    platform: "any".into(),
                    url: "https://example.com/reinstall.wasm".into(),
                    sha256: Some(wasm_sha),
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
        let config_path = root.join("config.json");
        let base_request = InstallRequest {
            registry: RegistrySpec {
                id: "test".into(),
                url: "https://registry.test/index.json".into(),
                require_signature: false,
            },
            plugin_id: "demo".into(),
            version: None,
            config_path: config_path.clone(),
            force: false,
            dry_run: false,
            allow_unverified: false,
            refresh_index: false,
        };

        client
            .install(base_request.clone())
            .expect("initial install succeeds");
        let err = client
            .install(base_request)
            .expect_err("repeat install should fail");
        assert!(matches!(err, MarketplaceError::AlreadyInstalled(plugin) if plugin == "demo"));
    }

    #[test]
    fn list_outdated_skips_unreadable_registry_and_reports_newer_versions() {
        let root = temp_root("outdated");
        let cache = MarketplaceCache::new(&root);
        let fetcher = Arc::new(MapFetcher::default());

        let old_bytes = b"OLD".to_vec();
        let new_bytes = b"NEW".to_vec();
        let broken_bytes = b"BROKEN".to_vec();
        let old_sha = sha256_hex(&old_bytes);
        let new_sha = sha256_hex(&new_bytes);
        let broken_sha = sha256_hex(&broken_bytes);
        fetcher.insert("https://example.com/demo-1.0.0.wasm", old_bytes);
        fetcher.insert("https://example.com/demo-2.0.0.wasm", new_bytes);
        fetcher.insert("https://example.com/broken-1.0.0.wasm", broken_bytes);

        let install_index = RegistryIndex {
            version: 1,
            plugins: vec![
                PluginRecord {
                    id: "demo".into(),
                    name: String::new(),
                    description: String::new(),
                    homepage: None,
                    versions: vec![PluginVersion {
                        version: "1.0.0".into(),
                        kind: PluginKind::Wasm,
                        platform: "any".into(),
                        url: "https://example.com/demo-1.0.0.wasm".into(),
                        sha256: Some(old_sha),
                        signature: None,
                        command: None,
                        args: Vec::new(),
                        env: Default::default(),
                        options: serde_json::Value::Null,
                        min_agena_version: None,
                        archive: None,
                        dependencies: Vec::new(),
                    }],
                },
                PluginRecord {
                    id: "broken".into(),
                    name: String::new(),
                    description: String::new(),
                    homepage: None,
                    versions: vec![PluginVersion {
                        version: "1.0.0".into(),
                        kind: PluginKind::Wasm,
                        platform: "any".into(),
                        url: "https://example.com/broken-1.0.0.wasm".into(),
                        sha256: Some(broken_sha),
                        signature: None,
                        command: None,
                        args: Vec::new(),
                        env: Default::default(),
                        options: serde_json::Value::Null,
                        min_agena_version: None,
                        archive: None,
                        dependencies: Vec::new(),
                    }],
                },
            ],
        };
        fetcher.insert(
            "https://registry.demo/index.json",
            serde_json::to_vec(&install_index).unwrap(),
        );
        fetcher.insert(
            "https://registry.broken/index.json",
            serde_json::to_vec(&install_index).unwrap(),
        );

        let client = MarketplaceClient::new(cache, Arc::clone(&fetcher), BTreeMap::new());
        client
            .install(InstallRequest {
                registry: RegistrySpec {
                    id: "demo-registry".into(),
                    url: "https://registry.demo/index.json".into(),
                    require_signature: false,
                },
                plugin_id: "demo".into(),
                version: None,
                config_path: root.join("config.json"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect("demo install");
        client
            .install(InstallRequest {
                registry: RegistrySpec {
                    id: "broken-registry".into(),
                    url: "https://registry.broken/index.json".into(),
                    require_signature: false,
                },
                plugin_id: "broken".into(),
                version: None,
                config_path: root.join("config.json"),
                force: false,
                dry_run: false,
                allow_unverified: false,
                refresh_index: false,
            })
            .expect("broken install");

        let upgraded_index = RegistryIndex {
            version: 1,
            plugins: vec![PluginRecord {
                id: "demo".into(),
                name: String::new(),
                description: String::new(),
                homepage: None,
                versions: vec![PluginVersion {
                    version: "2.0.0".into(),
                    kind: PluginKind::Wasm,
                    platform: "any".into(),
                    url: "https://example.com/demo-2.0.0.wasm".into(),
                    sha256: Some(new_sha),
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
            "https://registry.demo/index.json",
            serde_json::to_vec(&upgraded_index).unwrap(),
        );
        // Deliberately poison the second registry so list_outdated must skip it.
        fetcher.insert("https://registry.broken/index.json", b"{".to_vec());

        let outdated = client.list_outdated().expect("outdated list");
        assert_eq!(outdated.len(), 1);
        assert_eq!(outdated[0].plugin_id, "demo");
        assert_eq!(outdated[0].installed_version, "1.0.0");
        assert_eq!(outdated[0].latest_version, "2.0.0");
    }
}
