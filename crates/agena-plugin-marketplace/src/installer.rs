//! Install / uninstall flows. Wraps cache layout, http fetch, sha256/signature
//! verification, and JSON config writes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};

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
        self.fetch_index_using_cache(force_refresh, true)
    }

    fn fetch_index_using_cache(
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
    pub fn new(cache: MarketplaceCache, trusted_keys: BTreeMap<String, String>) -> Self {
        Self {
            cache,
            fetcher: ReqwestFetcher::new(),
            trusted_keys,
        }
    }
}

impl<F: HttpFetcher> MarketplaceClient<F> {
    pub fn from_parts(
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
        let index = registry_handle.fetch_index_using_cache(req.refresh_index, !req.dry_run)?;
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

        // Snapshot installed.json + the user's agena.json before mutating
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
        let index = registry.fetch_index_using_cache(false, !req.dry_run)?;
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
        let already_present = plugin_config_exists(&document, &plugin.id);
        if already_present && !req.force {
            return Err(MarketplaceError::AlreadyInstalled(plugin.id.clone()));
        }
        write_plugin_config(&mut document, &plugin.id, &version, &artifact_path)?;
        if !req.dry_run {
            write_config_doc(&config_path, &document)?;
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
        remove_plugin_config(&mut document, plugin_id);
        write_config_doc(&record.config_path, &document)?;
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
/// Request to install a marketplace plugin.
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
/// Outcome of installing a plugin.
pub struct InstallOutcome {
    pub plugin_id: String,
    pub version: String,
    pub kind: PluginKind,
    pub artifact_path: PathBuf,
    pub config_path: PathBuf,
    pub dry_run: bool,
}

#[derive(Debug, Clone)]
/// Outcome of uninstalling a plugin.
pub struct UninstallOutcome {
    pub plugin_id: String,
    pub version: String,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone)]
/// Outcome of upgrading a plugin.
pub struct UpgradeOutcome {
    pub plugin_id: String,
    pub previous_version: String,
    pub installed_version: String,
    pub upgraded: bool,
    pub outcome: Option<InstallOutcome>,
}

#[derive(Debug, Clone)]
/// An installed plugin with a newer version available.
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

fn read_or_create_doc(path: &Path) -> Result<JsonValue, MarketplaceError> {
    if path.exists() {
        let text = std::fs::read_to_string(path)?;
        if text.trim().is_empty() {
            return Ok(JsonValue::Object(JsonMap::new()));
        }
        let doc: JsonValue = serde_json::from_str(&text).map_err(|e| {
            MarketplaceError::Config(format!("parse {} as JSON: {e}", path.display()))
        })?;
        if !doc.is_object() {
            return Err(MarketplaceError::Config(format!(
                "{} must contain a JSON object",
                path.display()
            )));
        }
        Ok(doc)
    } else {
        Ok(JsonValue::Object(JsonMap::new()))
    }
}

fn write_config_doc(path: &Path, doc: &JsonValue) -> Result<(), MarketplaceError> {
    let mut bytes = serde_json::to_vec_pretty(doc).map_err(|e| {
        MarketplaceError::Config(format!("serialize {} as JSON: {e}", path.display()))
    })?;
    bytes.push(b'\n');
    write_secure_file(path, &bytes)
}

fn ensure_object<'a>(
    value: &'a mut JsonValue,
    label: &str,
) -> Result<&'a mut JsonMap<String, JsonValue>, MarketplaceError> {
    if value.is_null() {
        *value = JsonValue::Object(JsonMap::new());
    }
    value
        .as_object_mut()
        .ok_or_else(|| MarketplaceError::Config(format!("`{label}` must be a JSON object")))
}

fn ensure_plugins_list_object(
    doc: &mut JsonValue,
) -> Result<&mut JsonMap<String, JsonValue>, MarketplaceError> {
    let root = ensure_object(doc, "config")?;
    let plugins = root
        .entry("plugins")
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let plugins = ensure_object(plugins, "plugins")?;
    let list = plugins
        .entry("list")
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    ensure_object(list, "plugins.list")
}

fn plugin_config_exists(doc: &JsonValue, plugin_id: &str) -> bool {
    doc.get("plugins")
        .and_then(JsonValue::as_object)
        .and_then(|plugins| plugins.get("list"))
        .and_then(JsonValue::as_object)
        .map(|l| l.contains_key(plugin_id))
        .unwrap_or(false)
}

fn write_plugin_config(
    doc: &mut JsonValue,
    plugin_id: &str,
    version: &PluginVersion,
    artifact_path: &Path,
) -> Result<(), MarketplaceError> {
    let list = ensure_plugins_list_object(doc)?;
    let mut package = JsonMap::new();
    package.insert(
        "kind".to_string(),
        JsonValue::from(version.kind.to_string()),
    );
    match version.kind {
        PluginKind::Cdylib => {
            package.insert(
                "path".to_string(),
                JsonValue::from(artifact_path.display().to_string()),
            );
            if let Some(sha) = version.sha256.as_ref() {
                package.insert("sha256".to_string(), JsonValue::from(sha.as_str()));
            }
            if let Some(sig) = version.signature.as_ref() {
                package.insert(
                    "signature".to_string(),
                    serde_json::json!({
                        "key_id": sig.key_id.as_str(),
                        "signature": sig.signature.as_str(),
                    }),
                );
            }
        }
        PluginKind::Wasm => {
            package.insert(
                "path".to_string(),
                JsonValue::from(artifact_path.display().to_string()),
            );
            if let Some(sha) = version.sha256.as_ref() {
                package.insert("sha256".to_string(), JsonValue::from(sha.as_str()));
            }
        }
        PluginKind::Stdio => {
            let command = version
                .command
                .clone()
                .unwrap_or_else(|| artifact_path.display().to_string());
            package.insert("command".to_string(), JsonValue::from(command));
            if !version.args.is_empty() {
                package.insert("args".to_string(), serde_json::json!(version.args));
            }
            if !version.env.is_empty() {
                package.insert("env".to_string(), serde_json::json!(version.env));
            }
            if let Some(sha) = version.sha256.as_ref() {
                package.insert("sha256".to_string(), JsonValue::from(sha.as_str()));
            }
        }
        PluginKind::Http => {
            package.insert("url".to_string(), JsonValue::from(version.url.as_str()));
        }
    }
    let mut plugin_config = JsonMap::new();
    plugin_config.insert("package".to_string(), JsonValue::Object(package));
    if !version.config.is_null() {
        plugin_config.insert("config".to_string(), version.config.clone());
    }
    list.insert(plugin_id.to_string(), JsonValue::Object(plugin_config));
    Ok(())
}

fn remove_plugin_config(doc: &mut JsonValue, plugin_id: &str) {
    if let Some(plugins) = doc.get_mut("plugins").and_then(JsonValue::as_object_mut)
        && let Some(list) = plugins.get_mut("list").and_then(JsonValue::as_object_mut)
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
