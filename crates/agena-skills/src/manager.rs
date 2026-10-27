use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::import::{ignored_path, join_relative, path_string, read_bounded, safe_relative};
use crate::{
    DeclaredSkill, MAX_DOCUMENT_BYTES, MAX_PACKAGE_BYTES, MAX_PACKAGE_DEPTH, MAX_PACKAGE_FILES,
    Result, STATE_FILE, SkillCatalog, SkillDetail, SkillDiagnostic, SkillError, SkillErrorKind,
    SkillFile, SkillImport, SkillInstallPreview, SkillInstallResult, SkillMetadata, SkillResource,
    SkillRoot, SkillScope, SkillSource, SkillSummary, parse_document, validate_document,
};

type ReadFilter = Arc<dyn Fn(&Path) -> bool + Send + Sync>;
static MUTATIONS: Mutex<()> = Mutex::new(());

#[derive(Clone)]
pub struct SkillManager {
    workspace: PathBuf,
    global_root: PathBuf,
    additional_roots: Vec<PathBuf>,
    declared: Vec<DeclaredSkill>,
    disabled_names: Vec<String>,
    read_filter: Option<ReadFilter>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct PackageState {
    enabled: bool,
    source: Option<SkillSource>,
    revision: u64,
}

impl Default for PackageState {
    fn default() -> Self {
        Self {
            enabled: true,
            source: None,
            revision: 0,
        }
    }
}

/// Lowest to highest precedence. Agena's managed location wins within a scope,
/// and project packages win over user packages. Compatibility roots are read-only.
pub fn default_roots(workspace: Option<&Path>, global_root: &Path) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        for name in [".codex/skills", ".claude/skills", ".agents/skills"] {
            roots.push(SkillRoot {
                path: PathBuf::from(&home)
                    .join(name)
                    .to_string_lossy()
                    .into_owned(),
                scope: SkillScope::Global,
                editable: false,
            });
        }
    }
    roots.push(SkillRoot {
        path: global_root.to_string_lossy().into_owned(),
        scope: SkillScope::Global,
        editable: true,
    });
    if let Some(workspace) = workspace {
        for name in [
            ".codex/skills",
            ".claude/skills",
            ".agents/skills",
            ".agena/skills",
        ] {
            roots.push(SkillRoot {
                path: workspace.join(name).to_string_lossy().into_owned(),
                scope: SkillScope::Workspace,
                editable: name == ".agena/skills",
            });
        }
    }
    roots
}

/// Missing metadata preserves compatibility with existing plain skill folders.
/// Malformed metadata is disabled rather than silently enabling a broken state.
pub fn package_enabled(skill_path: &Path) -> bool {
    let Some(parent) = skill_path.parent() else {
        return false;
    };
    package_enabled_override(&parent.join("SKILL.md")).unwrap_or(true)
}

pub fn package_enabled_override(skill_path: &Path) -> Option<bool> {
    let path = skill_path.parent()?.join(STATE_FILE);
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        _ => Some(
            read_state(&path)
                .map(|state| state.enabled)
                .unwrap_or(false),
        ),
    }
}

impl SkillManager {
    pub fn new(workspace: PathBuf, global_root: PathBuf) -> Self {
        Self {
            workspace,
            global_root,
            additional_roots: Vec::new(),
            declared: Vec::new(),
            disabled_names: Vec::new(),
            read_filter: None,
        }
    }

    pub fn with_additional_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.additional_roots = roots;
        self
    }

    pub fn with_declared_skills(mut self, skills: Vec<DeclaredSkill>) -> Self {
        self.declared = skills;
        self
    }

    pub fn with_disabled_names(mut self, names: Vec<String>) -> Self {
        self.disabled_names = names;
        self
    }

    pub fn with_read_filter(mut self, filter: ReadFilter) -> Self {
        self.read_filter = Some(filter);
        self
    }

    pub fn managed_root(&self, scope: SkillScope) -> PathBuf {
        match scope {
            SkillScope::Global => self.global_root.clone(),
            SkillScope::Workspace => self.workspace.join(".agena/skills"),
        }
    }

    pub fn roots(&self) -> Vec<SkillRoot> {
        let mut roots = default_roots(Some(&self.workspace), &self.global_root);
        let insert_at = roots.len().saturating_sub(1);
        for (offset, root) in self.additional_roots.iter().enumerate() {
            roots.insert(
                insert_at + offset,
                SkillRoot {
                    path: self.workspace.join(root).to_string_lossy().into_owned(),
                    scope: SkillScope::Workspace,
                    editable: false,
                },
            );
        }
        let mut seen = BTreeSet::new();
        roots.reverse();
        roots.retain(|root| seen.insert(root.path.clone()));
        roots.reverse();
        roots
    }

    fn allowed(&self, path: &Path) -> bool {
        self.read_filter.as_ref().is_none_or(|filter| filter(path))
    }

    /// Resolve an opaque identity using directory metadata only. The caller can
    /// authorize the returned path before any document content is read.
    pub fn resolve_path(&self, id: &str) -> Result<PathBuf> {
        if let Some(declared) = self.declared.iter().find(|entry| declared_id(entry) == id) {
            return Ok(PathBuf::from(declared_path(declared)));
        }
        let mut lookup = self.clone();
        lookup.read_filter = None;
        let (entries, _) = lookup.scan_paths();
        entries
            .into_iter()
            .find(|(path, _)| path_id(path) == id)
            .map(|(path, _)| path)
            .ok_or_else(|| SkillError::new(SkillErrorKind::NotFound, "Skill does not exist"))
    }

    pub fn list(&self) -> Result<SkillCatalog> {
        let (paths, mut diagnostics) = self.scan_paths();
        let mut declared = self.declared.iter().collect::<Vec<_>>();
        declared.sort_by(|left, right| {
            (&left.plugin_id, &left.name).cmp(&(&right.plugin_id, &right.name))
        });
        let mut skills = declared
            .into_iter()
            .map(|entry| {
                let mut summary = declared_summary(entry);
                summary.enabled = !self.legacy_disabled(&summary.name);
                summary
            })
            .collect::<Vec<_>>();
        for (path, root) in paths {
            match self.summarize(&path, &root) {
                Ok((summary, _, _)) => skills.push(summary),
                Err(error) => diagnostics.push(SkillDiagnostic {
                    path: path.to_string_lossy().into_owned(),
                    message: error.message,
                }),
            }
        }
        let mut effective = BTreeMap::<String, usize>::new();
        for index in 0..skills.len() {
            if !skills[index].enabled || !skills[index].valid {
                continue;
            }
            let name = skills[index].name.clone();
            if let Some(previous) = effective.insert(name, index) {
                skills[previous].shadowed = true;
            }
        }
        skills.sort_by(|left, right| (&left.name, &left.path).cmp(&(&right.name, &right.path)));
        Ok(SkillCatalog {
            skills,
            diagnostics,
            roots: self.roots(),
        })
    }

    pub fn get(&self, id: &str) -> Result<SkillDetail> {
        if let Some(declared) = self.declared.iter().find(|entry| declared_id(entry) == id) {
            let mut detail = declared_detail(declared);
            detail.summary.enabled = !self.legacy_disabled(&detail.summary.name);
            if self.read_filter.is_none() {
                detail.summary.shadowed = self
                    .list()?
                    .skills
                    .iter()
                    .find(|entry| entry.id == id)
                    .is_some_and(|entry| entry.shadowed);
            }
            return Ok(detail);
        }
        let path = self.resolve_path(id)?;
        let root = self.root_for(&path)?;
        let (mut summary, document, files) = self.summarize(&path, &root)?;
        // A targeted tool read must not request unrelated catalog permissions,
        // especially after a mutation has already committed. Administrative
        // clients without a filter can include complete shadowing information.
        if self.read_filter.is_none() {
            summary.shadowed = self
                .list()?
                .skills
                .iter()
                .find(|entry| entry.id == id)
                .is_some_and(|entry| entry.shadowed);
        }
        let metadata = parse_document(&document, directory_name(&path))
            .ok()
            .map(|parsed| parsed.metadata);
        Ok(SkillDetail {
            summary,
            document,
            metadata,
            files,
        })
    }

    pub fn create(&self, scope: SkillScope, document: &str) -> Result<SkillDetail> {
        let metadata = require_valid(document, None)?;
        let _guard = mutation_guard()?;
        let root = self.ensure_root(scope)?;
        let destination = root.join(&metadata.name);
        if std::fs::symlink_metadata(&destination).is_ok() {
            return Err(SkillError::conflict(format!(
                "Skill '{}' already exists in this scope",
                metadata.name
            )));
        }
        let stage = tempfile::Builder::new()
            .prefix(".agena-skill-stage-")
            .tempdir_in(&root)?;
        let staged = stage.path().join(&metadata.name);
        std::fs::create_dir(&staged)?;
        atomic_write(&staged.join("SKILL.md"), document.as_bytes())?;
        write_state(&staged, &PackageState::default())?;
        std::fs::rename(&staged, &destination)?;
        self.get(&path_id(&destination.join("SKILL.md")))
    }

    pub fn update(
        &self,
        id: &str,
        document: &str,
        expected_revision: Option<&str>,
    ) -> Result<SkillDetail> {
        let _guard = mutation_guard()?;
        let detail = self.editable_detail(id)?;
        check_revision(&detail.summary.revision, expected_revision)?;
        let path = PathBuf::from(&detail.summary.path);
        require_valid(document, directory_name(&path))?;
        atomic_write(&path, document.as_bytes())?;
        self.get(id)
    }

    pub fn remove(&self, id: &str, expected_revision: Option<&str>) -> Result<SkillSummary> {
        let _guard = mutation_guard()?;
        let detail = self.editable_detail(id)?;
        check_revision(&detail.summary.revision, expected_revision)?;
        let path = PathBuf::from(&detail.summary.path);
        let directory = path
            .parent()
            .ok_or_else(|| SkillError::invalid("Skill has no directory"))?;
        let root = self.ensure_root(detail.summary.scope)?;
        let trash = tempfile::Builder::new()
            .prefix(".agena-skill-trash-")
            .tempdir_in(root)?;
        let removed = trash.path().join("package");
        std::fs::rename(directory, &removed)?;
        // The rename is the removal commit point. TempDir owns the entire
        // package, including supporting files, after the visible entry is gone.
        trash.close().map_err(|error| {
            SkillError::new(
                SkillErrorKind::Io,
                format!("Skill was removed, but temporary package cleanup failed: {error}"),
            )
        })?;
        Ok(detail.summary)
    }

    pub fn set_enabled(
        &self,
        id: &str,
        enabled: bool,
        expected_revision: Option<&str>,
    ) -> Result<SkillDetail> {
        let _guard = mutation_guard()?;
        let detail = self.editable_detail(id)?;
        check_revision(&detail.summary.revision, expected_revision)?;
        let path = PathBuf::from(&detail.summary.path);
        let directory = path
            .parent()
            .ok_or_else(|| SkillError::invalid("Skill has no directory"))?;
        let mut state = self.load_state(directory)?;
        state.enabled = enabled;
        state.revision = state.revision.saturating_add(1);
        write_state(directory, &state)?;
        self.get(id)
    }

    pub fn read_resource(&self, id: &str, path: &str) -> Result<SkillResource> {
        let detail = self.get(id)?;
        let relative = resource_path(path)?;
        let root = filesystem_directory(&detail.summary)?;
        let candidate = checked_resource(&root, &relative, false)?;
        if !self.allowed(&candidate) {
            return Err(SkillError::new(
                SkillErrorKind::NotFound,
                "Skill resource is not available",
            ));
        }
        let content = read_text(&candidate, MAX_DOCUMENT_BYTES)?;
        let revision = hash_bytes(content.as_bytes());
        Ok(SkillResource {
            path: relative,
            content,
            revision,
        })
    }

    /// An empty expected revision means create-only, preventing a new file
    /// dialog from overwriting a file created by another client.
    pub fn write_resource(
        &self,
        id: &str,
        path: &str,
        content: &str,
        expected_revision: Option<&str>,
    ) -> Result<SkillResource> {
        if content.len() > MAX_DOCUMENT_BYTES {
            return Err(SkillError::too_large(
                "Edited resources cannot exceed 1 MiB",
            ));
        }
        let relative = resource_path(path)?;
        let _guard = mutation_guard()?;
        let detail = self.editable_detail(id)?;
        let root = filesystem_directory(&detail.summary)?;
        let candidate = checked_resource(&root, &relative, true)?;
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                if expected_revision == Some("") {
                    return Err(SkillError::conflict(
                        "Resource already exists; open it before editing",
                    ));
                }
                if !self.allowed(&candidate) {
                    return Err(SkillError::new(
                        SkillErrorKind::ReadOnly,
                        "Resource is not readable",
                    ));
                }
                let old = read_text(&candidate, MAX_DOCUMENT_BYTES)?;
                check_revision(&hash_bytes(old.as_bytes()), expected_revision)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if expected_revision.is_some_and(|revision| !revision.is_empty()) {
                    return Err(SkillError::conflict(
                        "Resource was removed after it was opened",
                    ));
                }
            }
            Err(error) => return Err(error.into()),
        }
        if let Some(parent) = candidate.parent() {
            std::fs::create_dir_all(parent)?;
            ensure_contained_directory(&root, parent)?;
        }
        let old_size = std::fs::metadata(&candidate)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let total = detail.files.iter().map(|file| file.size).sum::<u64>();
        if total
            .saturating_sub(old_size)
            .saturating_add(content.len() as u64)
            > MAX_PACKAGE_BYTES as u64
        {
            return Err(SkillError::too_large("Skill package exceeds 128 MiB"));
        }
        self.bump_state(&root)?;
        atomic_write(&candidate, content.as_bytes())?;
        Ok(SkillResource {
            path: relative,
            content: content.into(),
            revision: hash_bytes(content.as_bytes()),
        })
    }

    pub fn remove_resource(
        &self,
        id: &str,
        path: &str,
        expected_revision: Option<&str>,
    ) -> Result<SkillDetail> {
        let relative = resource_path(path)?;
        let _guard = mutation_guard()?;
        let detail = self.editable_detail(id)?;
        let root = filesystem_directory(&detail.summary)?;
        let candidate = checked_resource(&root, &relative, false)?;
        if !self.allowed(&candidate) {
            return Err(SkillError::new(
                SkillErrorKind::ReadOnly,
                "Resource is not readable",
            ));
        }
        // A package revision covers the resource inventory and lets clients
        // delete binary or oversized resources without reading them as text.
        // Older callers can continue to use a resource's content revision.
        if expected_revision != Some(detail.summary.revision.as_str()) {
            let bytes = read_file(&candidate, MAX_PACKAGE_BYTES)?;
            check_revision(&hash_bytes(&bytes), expected_revision)?;
        }
        self.bump_state(&root)?;
        std::fs::remove_file(candidate)?;
        self.get(id)
    }

    pub fn preview_install(
        &self,
        source: &SkillImport,
        scope: SkillScope,
    ) -> Result<SkillInstallPreview> {
        let root = self.managed_root(scope);
        let mut candidates = source.candidates();
        for candidate in &mut candidates {
            candidate.conflict = std::fs::symlink_metadata(root.join(&candidate.name)).is_ok();
        }
        Ok(SkillInstallPreview {
            revision: source.revision(),
            candidates,
        })
    }

    pub fn install(
        &self,
        import: &SkillImport,
        scope: SkillScope,
        selected: &[String],
        replace: bool,
        source: Option<SkillSource>,
    ) -> Result<SkillInstallResult> {
        let preview = self.preview_install(import, scope)?;
        let candidates = if selected.is_empty() {
            preview
                .candidates
                .into_iter()
                .filter(|candidate| candidate.valid)
                .collect::<Vec<_>>()
        } else {
            let requested = selected.iter().map(String::as_str).collect::<BTreeSet<_>>();
            if requested.len() != selected.len() {
                return Err(SkillError::invalid(
                    "A skill candidate was selected more than once",
                ));
            }
            let mut candidates = Vec::new();
            for requested in selected {
                let candidate = preview
                    .candidates
                    .iter()
                    .find(|candidate| candidate.path == *requested)
                    .ok_or_else(|| {
                        SkillError::invalid(format!("Unknown skill candidate '{requested}'"))
                    })?;
                if !candidate.valid {
                    return Err(SkillError::invalid(format!(
                        "Skill '{}' is invalid: {}",
                        candidate.name,
                        candidate.diagnostics.join("; ")
                    )));
                }
                candidates.push(candidate.clone());
            }
            candidates
        };
        if candidates.is_empty() {
            return Err(SkillError::invalid(
                "Source contains no valid selected skills",
            ));
        }
        let mut names = BTreeSet::new();
        for candidate in &candidates {
            if !names.insert(candidate.name.clone()) {
                return Err(SkillError::conflict(format!(
                    "Source contains multiple skills named '{}'; select one",
                    candidate.name
                )));
            }
        }
        let _guard = mutation_guard()?;
        let root = self.ensure_root(scope)?;
        let staging = tempfile::Builder::new()
            .prefix(".agena-skill-stage-")
            .tempdir_in(&root)?;
        let backups = tempfile::Builder::new()
            .prefix(".agena-skill-backup-")
            .tempdir_in(&root)?;
        let mut plans = Vec::new();
        for candidate in &candidates {
            let destination = root.join(&candidate.name);
            let mut enabled = true;
            if std::fs::symlink_metadata(&destination).is_ok() {
                if !replace {
                    return Err(SkillError::conflict(format!(
                        "Skill '{}' already exists; explicitly choose replace",
                        candidate.name
                    )));
                }
                ensure_package_directory(&root, &destination)?;
                let document = destination.join("SKILL.md");
                if !std::fs::symlink_metadata(&document)
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
                {
                    return Err(SkillError::conflict(format!(
                        "Destination '{}' is not an existing skill package",
                        candidate.name
                    )));
                }
                enabled = self.load_state(&destination)?.enabled;
            }
            let staged = staging.path().join(&candidate.name);
            std::fs::create_dir(&staged)?;
            let prefix = if candidate.path == "." {
                String::new()
            } else {
                format!("{}/", candidate.path)
            };
            for file in import.package_files(&candidate.path) {
                let relative = file
                    .path
                    .strip_prefix(&prefix)
                    .ok_or_else(|| SkillError::invalid("Source package prefix changed"))?;
                let destination_file = join_relative(&staged, relative);
                if let Some(parent) = destination_file.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                atomic_write(&destination_file, &file.content)?;
                #[cfg(unix)]
                if file.executable {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        &destination_file,
                        std::fs::Permissions::from_mode(0o700),
                    )?;
                }
            }
            let document = read_text(&staged.join("SKILL.md"), MAX_DOCUMENT_BYTES)?;
            require_valid(&document, Some(&candidate.name))?;
            let mut provenance = source.clone();
            if let Some(provenance) = &mut provenance {
                provenance.revision = Some(import.revision());
                if candidate.path != "." {
                    provenance.subpath = Some(match provenance.subpath.take() {
                        Some(prefix) if !prefix.is_empty() => {
                            format!("{}/{}", prefix.trim_end_matches('/'), candidate.path)
                        }
                        _ => candidate.path.clone(),
                    });
                }
            }
            write_state(
                &staged,
                &PackageState {
                    enabled,
                    source: provenance,
                    revision: 0,
                },
            )?;
            plans.push(InstallPlan {
                destination,
                staged,
                backup: backups.path().join(&candidate.name),
                installed: false,
            });
        }
        if let Err(error) = commit_install(&mut plans) {
            // Never delete a backup when rollback could not restore it. The
            // retained recovery directory is excluded from discovery.
            let recovery = backups.keep();
            return Err(SkillError::new(
                error.kind,
                format!(
                    "{}; recovery directory: {}",
                    error.message,
                    recovery.display(),
                ),
            ));
        }
        let mut installed = Vec::new();
        for candidate in candidates {
            let id = path_id(&root.join(candidate.name).join("SKILL.md"));
            installed.push(self.get(&id)?.summary);
        }
        Ok(SkillInstallResult { installed })
    }

    fn scan_paths(&self) -> (Vec<(PathBuf, SkillRoot)>, Vec<SkillDiagnostic>) {
        let mut paths = Vec::new();
        let mut diagnostics = Vec::new();
        let mut seen = BTreeSet::new();
        for root in self.roots() {
            let path = PathBuf::from(&root.path);
            if std::fs::symlink_metadata(&path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                continue;
            }
            if !self.allowed(&path) {
                continue;
            }
            let mut entries = 0;
            scan_directory(
                self,
                &path,
                &root,
                0,
                &mut entries,
                &mut paths,
                &mut diagnostics,
            );
        }
        paths.reverse();
        paths.retain(|(path, _)| seen.insert(path.clone()));
        paths.reverse();
        (paths, diagnostics)
    }

    fn root_for(&self, path: &Path) -> Result<SkillRoot> {
        self.roots()
            .into_iter()
            .rev()
            .find(|root| path.starts_with(&root.path))
            .ok_or_else(|| SkillError::invalid("Skill path is outside configured roots"))
    }

    fn summarize(
        &self,
        path: &Path,
        root: &SkillRoot,
    ) -> Result<(SkillSummary, String, Vec<SkillFile>)> {
        if !self.allowed(path) {
            return Err(SkillError::new(
                SkillErrorKind::NotFound,
                "Skill is not available",
            ));
        }
        let mut diagnostics = Vec::new();
        let document = match read_text(path, MAX_DOCUMENT_BYTES) {
            Ok(document) => document,
            Err(error) => {
                diagnostics.push(error.message);
                String::new()
            }
        };
        let validation = validate_document(&document, directory_name(path));
        diagnostics.extend(validation.errors);
        let parent = path
            .parent()
            .ok_or_else(|| SkillError::invalid("Skill path has no parent"))?;
        let metadata_path = parent.join(STATE_FILE);
        let state_readable = self.allowed(&metadata_path);
        let explicit_state = state_readable && std::fs::symlink_metadata(&metadata_path).is_ok();
        let state = if state_readable {
            match read_state(&metadata_path) {
                Ok(state) => state,
                Err(error) => {
                    diagnostics.push(format!("Invalid Agena skill state: {error}"));
                    PackageState {
                        enabled: false,
                        ..Default::default()
                    }
                }
            }
        } else {
            PackageState {
                enabled: false,
                ..Default::default()
            }
        };
        let mut digest = Sha256::new();
        digest.update(document.as_bytes());
        digest.update(
            serde_json::to_vec(&state).map_err(|error| SkillError::invalid(error.to_string()))?,
        );
        let (files, file_diagnostics) = inventory(self, parent, &mut digest);
        diagnostics.extend(file_diagnostics);
        let editable = root.editable
            && parent.parent() == Some(Path::new(&root.path))
            && ensure_package_directory(Path::new(&root.path), parent).is_ok();
        let name = validation
            .metadata
            .as_ref()
            .map(|metadata| metadata.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| directory_name(path).unwrap_or("invalid-skill").into());
        let description = validation
            .metadata
            .as_ref()
            .map(|metadata| metadata.description.clone())
            .unwrap_or_default();
        let enabled = if !state_readable {
            false
        } else if explicit_state {
            state.enabled
        } else {
            !self.legacy_disabled(&name)
        };
        Ok((
            SkillSummary {
                id: path_id(path),
                name,
                description,
                scope: root.scope,
                path: path.to_string_lossy().into_owned(),
                editable,
                enabled,
                valid: validation.valid && diagnostics.is_empty(),
                shadowed: false,
                revision: hex::encode(digest.finalize()),
                source: state.source,
                diagnostics,
            },
            document,
            files,
        ))
    }

    fn editable_detail(&self, id: &str) -> Result<SkillDetail> {
        let detail = self.get(id)?;
        if !detail.summary.editable {
            return Err(SkillError::new(
                SkillErrorKind::ReadOnly,
                "This skill is read-only; install a copy into Global or Workspace to edit it",
            ));
        }
        let path = PathBuf::from(&detail.summary.path);
        let root = self.ensure_root(detail.summary.scope)?;
        let parent = path
            .parent()
            .ok_or_else(|| SkillError::invalid("Skill path has no parent"))?;
        ensure_package_directory(&root, parent)?;
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SkillError::invalid("SKILL.md must be a regular file"));
        }
        Ok(detail)
    }

    fn ensure_root(&self, scope: SkillScope) -> Result<PathBuf> {
        let path = self.managed_root(scope);
        // Validate existing path components before mkdir: creating a directory
        // through an outside symlink would already be an unintended mutation.
        reject_symlink_components(&path)?;
        std::fs::create_dir_all(&path)?;
        let canonical = path.canonicalize()?;
        if scope == SkillScope::Workspace {
            let workspace = self.workspace.canonicalize()?;
            if !canonical.starts_with(workspace) {
                return Err(SkillError::invalid(
                    "Workspace skill root escapes the workspace",
                ));
            }
        }
        Ok(path)
    }

    fn legacy_disabled(&self, name: &str) -> bool {
        self.disabled_names
            .iter()
            .any(|disabled| disabled.trim().eq_ignore_ascii_case(name))
    }

    fn load_state(&self, directory: &Path) -> Result<PackageState> {
        let path = directory.join(STATE_FILE);
        if !self.allowed(&path) {
            return Err(SkillError::new(
                SkillErrorKind::ReadOnly,
                "Skill state is not readable",
            ));
        }
        read_state(&path)
    }

    fn bump_state(&self, directory: &Path) -> Result<()> {
        let mut state = self.load_state(directory)?;
        state.revision = state.revision.saturating_add(1);
        write_state(directory, &state)
    }
}

fn mutation_guard() -> Result<std::sync::MutexGuard<'static, ()>> {
    MUTATIONS
        .lock()
        .map_err(|_| SkillError::new(SkillErrorKind::Io, "Skill mutation lock is poisoned"))
}

fn directory_name(path: &Path) -> Option<&str> {
    path.parent()?.file_name()?.to_str()
}

fn path_id(path: &Path) -> String {
    format!("skill-{}", hash_bytes(path.to_string_lossy().as_bytes()))
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn require_valid(document: &str, name: Option<&str>) -> Result<SkillMetadata> {
    let validation = validate_document(document, name);
    if !validation.valid {
        return Err(SkillError::invalid(validation.errors.join("; ")));
    }
    validation
        .metadata
        .ok_or_else(|| SkillError::invalid("Skill metadata is missing"))
}

fn check_revision(actual: &str, expected: Option<&str>) -> Result<()> {
    if expected.is_some_and(|expected| expected != actual) {
        return Err(SkillError::conflict(
            "Skill changed after it was opened; reload before saving",
        ));
    }
    Ok(())
}

fn read_file(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SkillError::invalid(
            "Skill files must be regular files, not links",
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(SkillError::too_large(format!(
            "'{}' exceeds the {limit} byte limit",
            path.display()
        )));
    }
    let mut file = std::fs::File::open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file()
        || opened.len() != metadata.len()
        || opened.modified().ok() != metadata.modified().ok()
    {
        return Err(SkillError::conflict("Skill file changed while opening it"));
    }
    let content = read_bounded(&mut file, limit)?;
    let after = file.metadata()?;
    if content.len() as u64 != after.len()
        || opened.len() != after.len()
        || opened.modified().ok() != after.modified().ok()
    {
        return Err(SkillError::conflict("Skill file changed while reading it"));
    }
    Ok(content)
}

fn read_text(path: &Path, limit: usize) -> Result<String> {
    String::from_utf8(read_file(path, limit)?)
        .map_err(|_| SkillError::invalid("Skill resource is not UTF-8 text"))
}

fn read_state(path: &Path) -> Result<PackageState> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PackageState::default()),
        Err(error) => Err(error.into()),
        Ok(_) => serde_json::from_slice(&read_file(path, MAX_DOCUMENT_BYTES)?)
            .map_err(|error| SkillError::invalid(format!("Invalid package state: {error}"))),
    }
}

fn write_state(directory: &Path, state: &PackageState) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(state).map_err(|error| SkillError::invalid(error.to_string()))?;
    atomic_write(&directory.join(STATE_FILE), &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| SkillError::invalid("File has no parent directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    if let Ok(metadata) = std::fs::symlink_metadata(path)
        && metadata.is_file()
        && !metadata.file_type().is_symlink()
    {
        temporary
            .as_file_mut()
            .set_permissions(metadata.permissions())?;
    }
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| SkillError::from(error.error))?;
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(SkillError::invalid("Skill paths cannot contain '..'"));
        }
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(SkillError::invalid(format!(
                    "Skill path contains a symbolic link: {}",
                    current.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(SkillError::invalid("Skill directory path contains a file"));
            }
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn ensure_contained_directory(root: &Path, directory: &Path) -> Result<()> {
    if !directory.starts_with(root) {
        return Err(SkillError::invalid("Skill directory escapes its package"));
    }
    reject_symlink_components(directory)?;
    if !directory.canonicalize()?.starts_with(root.canonicalize()?) {
        return Err(SkillError::invalid(
            "Skill directory resolves outside its package",
        ));
    }
    Ok(())
}

fn ensure_package_directory(root: &Path, directory: &Path) -> Result<()> {
    ensure_contained_directory(root, directory)?;
    if directory.parent() != Some(root) {
        return Err(SkillError::new(
            SkillErrorKind::ReadOnly,
            "Only direct children of managed skill roots can be modified",
        ));
    }
    Ok(())
}

fn filesystem_directory(summary: &SkillSummary) -> Result<PathBuf> {
    if summary.path.starts_with("plugin://") {
        return Err(SkillError::new(
            SkillErrorKind::ReadOnly,
            "Plugin-declared skills have no filesystem resources",
        ));
    }
    Path::new(&summary.path)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| SkillError::invalid("Skill has no directory"))
}

fn resource_path(path: &str) -> Result<String> {
    let path = safe_relative(path)?;
    if path == "SKILL.md" || ignored_path(&path) {
        return Err(SkillError::invalid(
            "Use the skill document editor for SKILL.md; internal package files are not editable resources",
        ));
    }
    Ok(path)
}

fn checked_resource(root: &Path, relative: &str, allow_missing: bool) -> Result<PathBuf> {
    let candidate = join_relative(root, relative);
    let parent = candidate
        .parent()
        .ok_or_else(|| SkillError::invalid("Resource has no parent"))?;
    reject_symlink_components(parent)?;
    match std::fs::symlink_metadata(&candidate) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(SkillError::invalid("Resource must be a regular file"))
        }
        Ok(_) => {
            if !candidate.canonicalize()?.starts_with(root.canonicalize()?) {
                return Err(SkillError::invalid("Resource resolves outside its skill"));
            }
            Ok(candidate)
        }
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            Ok(candidate)
        }
        Err(error) => Err(error.into()),
    }
}

fn scan_directory(
    manager: &SkillManager,
    directory: &Path,
    root: &SkillRoot,
    depth: usize,
    entries: &mut usize,
    found: &mut Vec<(PathBuf, SkillRoot)>,
    diagnostics: &mut Vec<SkillDiagnostic>,
) {
    if depth > MAX_PACKAGE_DEPTH || *entries > MAX_PACKAGE_FILES || !manager.allowed(directory) {
        return;
    }
    let metadata = match std::fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            diagnostics.push(SkillDiagnostic {
                path: directory.to_string_lossy().into_owned(),
                message: error.to_string(),
            });
            return;
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return;
    }
    let children = match std::fs::read_dir(directory) {
        Ok(children) => children,
        Err(error) => {
            diagnostics.push(SkillDiagnostic {
                path: directory.to_string_lossy().into_owned(),
                message: error.to_string(),
            });
            return;
        }
    };
    let mut children = children
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry),
            Err(error) => {
                diagnostics.push(SkillDiagnostic {
                    path: directory.to_string_lossy().into_owned(),
                    message: error.to_string(),
                });
                None
            }
        })
        .collect::<Vec<_>>();
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        *entries += 1;
        if *entries > MAX_PACKAGE_FILES {
            diagnostics.push(SkillDiagnostic {
                path: root.path.clone(),
                message: "Skill discovery stopped after 10,000 entries".into(),
            });
            break;
        }
        let path = child.path();
        if !manager.allowed(&path) {
            continue;
        }
        let name = child.file_name().to_string_lossy().into_owned();
        if ignored_path(&name) || name.starts_with(".agena-skill-") {
            continue;
        }
        let kind = match child.file_type() {
            Ok(kind) => kind,
            Err(_) => continue,
        };
        if kind.is_dir() {
            scan_directory(manager, &path, root, depth + 1, entries, found, diagnostics);
        } else if kind.is_file() && name == "SKILL.md" {
            found.push((path, root.clone()));
        }
    }
}

fn inventory(
    manager: &SkillManager,
    root: &Path,
    digest: &mut Sha256,
) -> (Vec<SkillFile>, Vec<String>) {
    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    let mut entries = 0_usize;
    while let Some((directory, depth)) = stack.pop() {
        if depth > MAX_PACKAGE_DEPTH {
            diagnostics.push("Skill package exceeds the directory depth limit".into());
            continue;
        }
        let children = match std::fs::read_dir(&directory) {
            Ok(children) => children,
            Err(error) => {
                diagnostics.push(error.to_string());
                continue;
            }
        };
        for child in children {
            entries += 1;
            if entries > MAX_PACKAGE_FILES {
                diagnostics.push("Skill package exceeds 10,000 entries".into());
                files.sort_by(|left: &SkillFile, right| left.path.cmp(&right.path));
                return (files, diagnostics);
            }
            let child = match child {
                Ok(child) => child,
                Err(error) => {
                    diagnostics.push(error.to_string());
                    continue;
                }
            };
            let path = child.path();
            if !manager.allowed(&path) {
                continue;
            }
            let relative = match path
                .strip_prefix(root)
                .ok()
                .and_then(|path| path_string(path).ok())
            {
                Some(relative) => relative,
                None => {
                    diagnostics.push("Skill contains an invalid file path".into());
                    continue;
                }
            };
            if ignored_path(&relative)
                || child
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".agena-skill-")
            {
                continue;
            }
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    diagnostics.push(error.to_string());
                    continue;
                }
            };
            if metadata.file_type().is_symlink() {
                diagnostics.push(format!("Resource '{relative}' is a symbolic link"));
            } else if metadata.is_dir() {
                stack.push((path, depth + 1));
            } else if metadata.is_file() {
                files.push(SkillFile {
                    path: relative,
                    size: metadata.len(),
                });
            } else {
                diagnostics.push(format!("Resource '{relative}' is not a regular file"));
            }
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    for file in &files {
        digest.update(file.path.as_bytes());
        digest.update(file.size.to_le_bytes());
        if let Ok(metadata) = std::fs::metadata(join_relative(root, &file.path))
            && let Ok(modified) = metadata.modified()
            && let Ok(elapsed) = modified.duration_since(UNIX_EPOCH)
        {
            digest.update(elapsed.as_nanos().to_le_bytes());
        }
    }
    (files, diagnostics)
}

struct InstallPlan {
    destination: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    installed: bool,
}

fn commit_install(plans: &mut [InstallPlan]) -> Result<()> {
    for index in 0..plans.len() {
        let result = (|| -> std::io::Result<()> {
            let plan = &mut plans[index];
            if plan.destination.exists() {
                std::fs::rename(&plan.destination, &plan.backup)?;
            }
            std::fs::rename(&plan.staged, &plan.destination)?;
            plan.installed = true;
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for plan in plans[..=index].iter_mut().rev() {
                if plan.installed
                    && let Err(error) = std::fs::remove_dir_all(&plan.destination)
                {
                    rollback_errors.push(error.to_string());
                    continue;
                }
                if plan.backup.exists()
                    && let Err(error) = std::fs::rename(&plan.backup, &plan.destination)
                {
                    rollback_errors.push(error.to_string());
                }
            }
            return Err(SkillError::new(
                SkillErrorKind::Io,
                if rollback_errors.is_empty() {
                    format!("Skill installation was rolled back: {error}")
                } else {
                    format!(
                        "Skill installation failed: {error}; rollback needs attention: {}",
                        rollback_errors.join("; ")
                    )
                },
            ));
        }
    }
    Ok(())
}

fn declared_path(skill: &DeclaredSkill) -> String {
    format!("plugin://{}/{}/SKILL.md", skill.plugin_id, skill.name)
}

fn declared_id(skill: &DeclaredSkill) -> String {
    format!(
        "declared-{}",
        hash_bytes(format!("{}\0{}", skill.plugin_id, skill.name).as_bytes())
    )
}

fn declared_document(skill: &DeclaredSkill) -> String {
    // JSON strings and arrays are valid YAML scalars and sequences.
    let name = serde_json::to_string(&skill.name).unwrap_or_default();
    let description = serde_json::to_string(&skill.description).unwrap_or_default();
    let aliases = serde_json::to_string(&skill.aliases).unwrap_or_default();
    format!(
        "---\nname: {name}\ndescription: {description}\naliases: {aliases}\n---\n{}",
        skill.instructions
    )
}

fn declared_summary(skill: &DeclaredSkill) -> SkillSummary {
    let document = declared_document(skill);
    SkillSummary {
        id: declared_id(skill),
        name: skill.name.clone(),
        description: skill.description.clone(),
        scope: SkillScope::Global,
        path: declared_path(skill),
        editable: false,
        enabled: true,
        // These declarations are validated by the plugin contract; filesystem
        // naming restrictions do not retroactively invalidate legacy aliases.
        valid: true,
        shadowed: false,
        revision: hash_bytes(document.as_bytes()),
        source: Some(SkillSource {
            kind: "plugin".into(),
            location: skill.plugin_id.clone(),
            reference: None,
            subpath: None,
            revision: Some(skill.version.clone()),
        }),
        diagnostics: Vec::new(),
    }
}

fn declared_detail(skill: &DeclaredSkill) -> SkillDetail {
    SkillDetail {
        summary: declared_summary(skill),
        document: declared_document(skill),
        metadata: Some(SkillMetadata {
            name: skill.name.clone(),
            description: skill.description.clone(),
            aliases: skill.aliases.clone(),
            ..Default::default()
        }),
        files: Vec::new(),
    }
}
