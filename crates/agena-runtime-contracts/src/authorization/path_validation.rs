//! Host-aware validation performed before persisting resource settings.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use agena_domain::{PathPermissionMode, PermissionConfig, canonical_permission_path};

use super::validate_permission_config;
use crate::permission::PermissionConfigError;

/// Validate one effective policy in its owning workspace. Defaults are not
/// explicit path denials. This function does not create files or directories.
pub fn validate_permission_config_for_workspace(
    config: &PermissionConfig,
    workspace: &Path,
) -> Result<(), PermissionConfigError> {
    validate_permission_config(config)?;
    let Some(path_config) = &config.path else {
        return Ok(());
    };
    let workspace = if workspace.as_os_str().is_empty() {
        None
    } else {
        Some(canonical_resource_path(workspace).map_err(|reason| invalid("<workspace>", reason))?)
    };
    let executable = std::env::current_exe()
        .map_err(|error| invalid("runtime executable", error.to_string()))
        .and_then(|path| {
            canonical_resource_path(&path).map_err(|reason| invalid("runtime executable", reason))
        })?;
    let temporary = canonical_resource_path(&std::env::temp_dir())
        .map_err(|reason| invalid("<tmp>", reason))?;
    let mut shared_temporary_roots = crate::permission::temp_roots();
    #[cfg(unix)]
    shared_temporary_roots.push(PathBuf::from("/var/tmp"));
    let shared_temporary_roots = shared_temporary_roots
        .iter()
        .map(|root| canonical_resource_path(root).map_err(|reason| invalid("<tmp>", reason)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut rules = Vec::new();
    for (pattern, modes) in &path_config.rules {
        let canonical =
            canonical_permission_path(pattern).map_err(|reason| invalid(pattern, reason))?;
        let mode = modes
            .sandbox_mode()
            .map_err(|reason| invalid(pattern, reason))?;
        for expanded in expand_path(&canonical, workspace.as_deref())? {
            let identity =
                canonical_resource_path(&expanded).map_err(|reason| invalid(pattern, reason))?;
            if mode == PathPermissionMode::Deny
                && (path_within(&executable, &identity) || path_within(&temporary, &identity))
            {
                return Err(invalid(
                    pattern,
                    "this denial contains Agena's executable or private temporary root, so no sandbox could start",
                ));
            }
            if mode != PathPermissionMode::Deny {
                for root in &shared_temporary_roots {
                    if path_within(root, &identity) {
                        return Err(invalid(
                            pattern,
                            "a grant cannot contain a shared OS temporary root; grant a specific task subdirectory instead (sandbox-private temporary directories are provided automatically)",
                        ));
                    }
                    if path_within(&identity, root) {
                        let reserved = identity
                            .components()
                            .nth(root.components().count())
                            .and_then(|component| component.as_os_str().to_str())
                            .is_some_and(|name| {
                                let name = name.to_ascii_lowercase();
                                [
                                    "agena-sandbox-",
                                    "agena-tool-worker-",
                                    "agena-media-worker-",
                                ]
                                .iter()
                                .any(|prefix| name.starts_with(prefix))
                            });
                        if reserved {
                            return Err(invalid(
                                pattern,
                                "sandbox-private temporary directories are owned by individual tasks and cannot be granted in saved settings; use an ordinary task directory",
                            ));
                        }
                    }
                }
            }
            rules.push(ResolvedRule {
                source: pattern.clone(),
                mode,
                identity,
                expanded,
            });
        }
    }
    for (index, left) in rules.iter().enumerate() {
        for right in rules.iter().skip(index + 1) {
            if left.mode == right.mode {
                continue;
            }
            let same = same_path_spelling(&left.identity, &right.identity)
                || same_existing_file(&left.identity, &right.identity)
                    .map_err(|reason| invalid(&left.source, reason))?;
            let (parent, child) = if path_within(&right.identity, &left.identity) {
                (left, right)
            } else if path_within(&left.identity, &right.identity) {
                (right, left)
            } else if same {
                (left, right)
            } else {
                continue;
            };
            if same
                || parent.mode == PathPermissionMode::Deny
                || child.mode != PathPermissionMode::Deny
            {
                return Err(invalid(
                    &child.source,
                    format!(
                        "resolved path {} conflicts with {} ({:?}); use one mode per real resource and do not reopen a denied ancestor",
                        child.identity.display(),
                        parent.source,
                        parent.mode,
                    ),
                ));
            }
        }
    }
    for rule in rules {
        if !same_path_spelling(&rule.expanded, &rule.identity) {
            return Err(invalid(
                &rule.source,
                format!(
                    "this path resolves through a symbolic link; use its real path {} so authorization and sandbox boundaries agree",
                    rule.identity.display(),
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "agena-path-validation-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&path).unwrap();
            Self(std::fs::canonicalize(path).unwrap())
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn config(rules: serde_json::Value) -> PermissionConfig {
        serde_json::from_value(serde_json::json!({
            "path":{"workspace":"deny","external":"deny","rules":rules}
        }))
        .unwrap()
    }

    #[test]
    fn default_deny_with_one_existing_or_future_file_is_valid() {
        let dir = TestDir::new();
        std::fs::write(dir.0.join("result.txt"), b"test").unwrap();
        for path in ["<workspace>/result.txt", "<workspace>/output/new.txt"] {
            validate_permission_config_for_workspace(
                &config(serde_json::json!({path:"readwrite"})),
                &dir.0,
            )
            .unwrap();
        }
    }

    #[test]
    fn equivalent_alias_and_absolute_rules_cannot_conflict() {
        let dir = TestDir::new();
        let target = dir.0.join("secret");
        std::fs::create_dir(&target).unwrap();
        let config = config(serde_json::json!({
            "<workspace>/secret":"deny",
            target.to_string_lossy().as_ref():"readwrite"
        }));
        let error = validate_permission_config_for_workspace(&config, &dir.0)
            .unwrap_err()
            .to_string();
        assert!(error.contains("conflicts"), "{error}");
    }

    #[test]
    fn runtime_denials_and_missing_context_fail_before_save() {
        let dir = TestDir::new();
        let error = validate_permission_config_for_workspace(
            &config(serde_json::json!({"<tmp>":"deny"})),
            &dir.0,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("no sandbox could start"), "{error}");
        let executable = std::env::current_exe().unwrap();
        let error = validate_permission_config_for_workspace(
            &config(serde_json::json!({executable.to_string_lossy().as_ref():"deny"})),
            &dir.0,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("no sandbox could start"), "{error}");
        assert!(
            validate_permission_config_for_workspace(
                &config(serde_json::json!({"<workspace>/input":"readonly"})),
                Path::new(""),
            )
            .is_err()
        );
    }

    #[test]
    fn required_default_aliases_and_missing_targets_remain_valid() {
        let dir = TestDir::new();
        validate_permission_config_for_workspace(&PermissionConfig::global_default(), &dir.0)
            .unwrap();
        let file = dir.0.join("file");
        std::fs::write(&file, b"test").unwrap();
        assert!(canonical_resource_path(&file.join("child")).is_err());
        assert_eq!(
            canonical_resource_path(&dir.0.join("not-created/child")).unwrap(),
            dir.0.join("not-created/child"),
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_hard_links_cannot_hide_conflicting_permissions() {
        let dir = TestDir::new();
        let original = dir.0.join("original.txt");
        let alias = dir.0.join("alias.txt");
        std::fs::write(&original, b"data").unwrap();
        std::fs::hard_link(&original, &alias).unwrap();
        assert!(!same_path_spelling(&original, &alias));
        assert!(same_existing_file(&original, &alias).unwrap());

        // This exercises identity validation even on Windows hosts without
        // native denied-path support.
        let error = validate_permission_config_for_workspace(
            &config(serde_json::json!({
                "<workspace>/original.txt":"readonly",
                "<workspace>/alias.txt":"readwrite"
            })),
            &dir.0,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("conflicts"), "{error}");
        assert!(error.contains("original.txt"), "{error}");
        assert!(error.contains("alias.txt"), "{error}");

        if mxc_sdk::process_container_common::base_container_runner::supports_native_denied_paths()
        {
            let error = validate_permission_config_for_workspace(
                &config(serde_json::json!({
                    "<workspace>/original.txt":"deny",
                    "<workspace>/alias.txt":"readwrite"
                })),
                &dir.0,
            )
            .unwrap_err()
            .to_string();
            assert!(error.contains("conflicts"), "{error}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_distinct_files_and_future_outputs_remain_valid_without_creation() {
        let dir = TestDir::new();
        let input = dir.0.join("input.txt");
        let distinct = dir.0.join("distinct.txt");
        let future = dir.0.join("output/new.txt");
        std::fs::write(&input, b"same contents").unwrap();
        std::fs::write(&distinct, b"same contents").unwrap();
        assert!(!same_existing_file(&input, &distinct).unwrap());
        assert!(!same_existing_file(&input, &future).unwrap());
        assert!(!same_existing_file(&future, &input).unwrap());
        assert!(!same_existing_file(&future, &dir.0.join("other/new.txt")).unwrap());

        validate_permission_config_for_workspace(
            &config(serde_json::json!({
                "<workspace>/input.txt":"readonly",
                "<workspace>/distinct.txt":"readwrite",
                "<workspace>/output/new.txt":"readwrite"
            })),
            &dir.0,
        )
        .unwrap();
        assert!(!dir.0.join("output").exists());
        assert!(!dir.0.join("other").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_hardlinks_cannot_hide_conflicting_permissions() {
        use std::os::unix::fs::symlink;
        let dir = TestDir::new();
        std::fs::create_dir(dir.0.join("secret")).unwrap();
        symlink(dir.0.join("secret"), dir.0.join("alias")).unwrap();
        let first = config(serde_json::json!({
            "<workspace>/secret":"deny",
            "<workspace>/alias/file":"readwrite"
        }));
        let error = validate_permission_config_for_workspace(&first, &dir.0)
            .unwrap_err()
            .to_string();
        assert!(error.contains("conflicts"), "{error}");

        std::fs::write(dir.0.join("one"), b"test").unwrap();
        std::fs::hard_link(dir.0.join("one"), dir.0.join("two")).unwrap();
        let config = config(serde_json::json!({
            "<workspace>/one":"deny",
            "<workspace>/two":"readwrite"
        }));
        assert!(validate_permission_config_for_workspace(&config, &dir.0).is_err());
        symlink("loop", dir.0.join("loop")).unwrap();
        assert!(canonical_resource_path(&dir.0.join("loop/file")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_rule_requires_the_real_path_even_without_a_conflict() {
        let dir = TestDir::new();
        std::fs::create_dir(dir.0.join("real")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("real"), dir.0.join("alias")).unwrap();
        let error = validate_permission_config_for_workspace(
            &config(serde_json::json!({"<workspace>/alias":"readonly"})),
            &dir.0,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("use its real path"), "{error}");
    }
}

struct ResolvedRule {
    source: String,
    mode: PathPermissionMode,
    identity: PathBuf,
    expanded: PathBuf,
}

fn invalid(pattern: &str, reason: impl Into<String>) -> PermissionConfigError {
    PermissionConfigError::InvalidPathRule {
        pattern: pattern.into(),
        reason: reason.into(),
    }
}

fn expand_path(
    pattern: &str,
    workspace: Option<&Path>,
) -> Result<Vec<PathBuf>, PermissionConfigError> {
    for alias in ["workspace", "state", "home", "tmp"] {
        let prefix = format!("<{alias}>");
        let Some(suffix) = pattern.strip_prefix(&prefix) else {
            continue;
        };
        let suffix = suffix.trim_start_matches('/');
        let roots = match alias {
            "workspace" => vec![
                workspace
                    .ok_or_else(|| {
                        invalid(
                            pattern,
                            "a concrete owning workspace is required to resolve <workspace>",
                        )
                    })?
                    .to_path_buf(),
            ],
            "state" => vec![
                agena_storage_layout::StorageLayout::current().workspace_dir(
                    workspace.ok_or_else(|| {
                        invalid(
                            pattern,
                            "a concrete owning workspace is required to resolve <state>",
                        )
                    })?,
                ),
            ],
            "home" => vec![crate::permission::home_dir().ok_or_else(|| {
                invalid(
                    pattern,
                    "the current user's home directory cannot be resolved",
                )
            })?],
            "tmp" => crate::permission::temp_roots(),
            _ => unreachable!(),
        };
        let mut paths = Vec::new();
        for root in roots {
            // Alias roots have one captured identity. This also handles the
            // platform's /tmp -> /private/tmp and relocated home directories.
            let root = canonical_resource_path(&root).map_err(|reason| invalid(pattern, reason))?;
            let path = if suffix.is_empty() {
                root
            } else {
                root.join(suffix)
            };
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        return Ok(paths);
    }
    let path = PathBuf::from(pattern);
    if !path.is_absolute() {
        return Err(invalid(
            pattern,
            "the path is not absolute on this host; use a supported path alias or a local absolute path",
        ));
    }
    Ok(vec![path])
}

/// Resolve every existing ancestor and retain a missing tail. Errors other
/// than NotFound are not treated as missing permissions or ignored resources.
pub(crate) fn canonical_resource_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{} must be an absolute path", path.display()));
    }
    let mut probe = path.to_path_buf();
    let mut pending: Vec<OsString> = Vec::new();
    loop {
        match std::fs::symlink_metadata(&probe) {
            Ok(_) => {
                let mut resolved = std::fs::canonicalize(&probe)
                    .map_err(|error| format!("cannot resolve {}: {error}", probe.display()))?;
                if !pending.is_empty()
                    && !std::fs::metadata(&resolved)
                        .map_err(|error| format!("cannot inspect {}: {error}", resolved.display()))?
                        .is_dir()
                {
                    return Err(format!(
                        "{} is a file, not a parent directory",
                        resolved.display()
                    ));
                }
                for component in pending.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let leaf = probe
                    .file_name()
                    .ok_or_else(|| {
                        format!("no existing filesystem ancestor for {}", path.display())
                    })?
                    .to_owned();
                if !probe.pop() {
                    return Err(format!(
                        "no existing filesystem ancestor for {}",
                        path.display()
                    ));
                }
                pending.push(leaf);
            }
            Err(error) => return Err(format!("cannot inspect {}: {error}", probe.display())),
        }
    }
}

fn same_path_spelling(left: &Path, right: &Path) -> bool {
    crate::permission::path_identity::equal(left, right)
}

fn path_within(path: &Path, root: &Path) -> bool {
    crate::permission::path_identity::within(path, root)
}

fn same_existing_file(left: &Path, right: &Path) -> Result<bool, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let inspect = |path: &Path| match std::fs::metadata(path) {
            Ok(metadata) => Ok(Some((metadata.dev(), metadata.ino()))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("cannot inspect {}: {error}", path.display())),
        };
        let left = inspect(left)?;
        let right = inspect(right)?;
        Ok(left.is_some() && left == right)
    }
    #[cfg(windows)]
    {
        use mxc_sdk::mxc_common::filesystem_object::{
            ExistingObjectComparison, compare_existing_filesystem_objects,
        };

        // Match MXC's volume + 128-bit FileIdInfo identity, including hard links
        // and ReFS. A missing output is Different; an uninspectable object must
        // not be assumed distinct from a resource with conflicting permissions.
        match compare_existing_filesystem_objects(left, right) {
            ExistingObjectComparison::Same => Ok(true),
            ExistingObjectComparison::Different => Ok(false),
            ExistingObjectComparison::Unknown => Err(format!(
                "cannot determine filesystem identity for {} and {}; check access to these resources and try again",
                left.display(),
                right.display(),
            )),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(same_path_spelling(left, right))
    }
}
