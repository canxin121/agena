//! Translate broad configured grants into native mounts without exposing host
//! scratch directories or making the sandbox's own launcher replaceable.

use super::path_aliases::identities;
use super::*;
use std::collections::HashSet;

pub(super) fn prepare(
    policy: &SandboxPolicy,
    temporary_root: &Path,
    launcher: &Path,
    cwd: &Path,
    environment: &HashMap<String, String>,
) -> io::Result<SandboxPolicy> {
    if !policy.configured_filesystem {
        return Ok(policy.clone());
    }
    let mut protected = Vec::new();
    for (path, _) in protected_launcher_paths(launcher, cwd, environment)? {
        protected.extend(protected_path_identities(&path)?);
    }
    partition(policy, temporary_root, &protected)
}

fn partition(
    policy: &SandboxPolicy,
    temporary_root: &Path,
    protected_writes: &[PathBuf],
) -> io::Result<SandboxPolicy> {
    let mut normalized = policy.clone();
    normalized.readonly_paths = identities(&policy.readonly_paths)?;
    normalized.readwrite_paths = identities(&policy.readwrite_paths)?;
    normalized.denied_paths = identities(&policy.denied_paths)?;
    normalized.default_denied_paths = identities(&policy.default_denied_paths)?;
    normalized.private_paths = identities(&policy.private_paths)?;
    // Saved settings reject conflicting physical aliases. Preserve the stricter
    // mode if a path changes identity between validation and native preparation.
    normalized.readwrite_paths.retain(|write| {
        !normalized
            .readonly_paths
            .iter()
            .any(|read| identity_within(read, write) && identity_within(write, read))
    });
    let policy = &normalized;
    let grants: Vec<_> = policy
        .readonly_paths
        .iter()
        .chain(&policy.readwrite_paths)
        .cloned()
        .collect();
    check_temporary_grants(&grants, temporary_root, &policy.private_paths, true)?;
    let mut private_roots = shared_temporary_roots(temporary_root)?;
    // A host-wide bind must not replace MXC's isolated kernel views with the
    // host's /proc, devices, or service-control sockets.
    #[cfg(target_os = "linux")]
    private_roots.extend(["/proc", "/sys", "/dev", "/run"].map(PathBuf::from));
    let private_roots = identities(&private_roots)?;
    let mut excluded = private_roots.clone();
    excluded.extend(identities(&policy.default_denied_paths)?);
    let protected_writes = identities(protected_writes)?;
    let mut readonly = Vec::new();
    let mut readwrite = Vec::new();
    for root in &policy.readonly_paths {
        readonly.extend(expand(policy, root, false, &excluded, &private_roots, &[])?);
    }
    for root in &policy.readwrite_paths {
        let expanded = expand(
            policy,
            root,
            true,
            &excluded,
            &private_roots,
            &protected_writes,
        )?;
        if expanded.len() != 1 || expanded[0] != *root {
            // Retain readable directory structure, while leaving ancestors of
            // trusted executables read-only so they cannot be renamed/replaced.
            readonly.extend(expand(policy, root, false, &excluded, &private_roots, &[])?);
        }
        readwrite.extend(expanded);
    }
    readonly.sort();
    readonly.dedup();
    readwrite.sort();
    readwrite.dedup();
    readonly.retain(|read| {
        !readwrite
            .iter()
            .any(|write| identity_within(read, write) && identity_within(write, read))
    });
    if readonly.len() + readwrite.len() > 8192 {
        return Err(invalid(
            "configured sandbox roots require too many mounts; grant more specific directories",
        ));
    }
    let mut prepared = policy.clone();
    prepared.readonly_paths = readonly;
    prepared.readwrite_paths = readwrite;
    prepared.default_denied_paths.clear();
    prepared.configured_filesystem = false;
    Ok(prepared)
}

fn expand(
    policy: &SandboxPolicy,
    root: &Path,
    write: bool,
    excluded: &[PathBuf],
    private_roots: &[PathBuf],
    protected_writes: &[PathBuf],
) -> io::Result<Vec<PathBuf>> {
    let identity = policy_path_identity(root)?;
    // Class defaults and private shared roots remove an inherited broad grant;
    // a separate concrete descendant grant can still reopen its own subtree.
    let mut cuts: Vec<_> = excluded
        .iter()
        .filter(|cut| identity_within(cut, &identity))
        .cloned()
        .collect();
    // Trusted executable paths cannot be reopened through a more specific
    // writable grant. This also protects their replaceable directory entries.
    cuts.extend(protected_writes.iter().cloned());
    let mut result = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    let mut visited = HashSet::new();
    while let Some(path) = pending.pop() {
        let derived = path != root;
        // Broad defaults describe the invoking user's accessible resources.
        // An unreadable sibling or stale symlink must not make every granted
        // resource unusable. Explicit roots still reach strict MXC validation.
        if derived && !super::host_access::can_delegate(&path, false)? {
            continue;
        }
        let canonical = policy_path_identity(&path)?;
        if !visited.insert(canonical.clone()) {
            continue;
        }
        if cuts
            .iter()
            .any(|cut| identity_within(&canonical, cut) || identity_within(&path, cut))
        {
            continue;
        }
        let access = access_at(policy, &canonical);
        if access == 0 || (write && access != 2) {
            continue;
        }
        // Enumerating a parent grant must not turn one of its symlinks into an
        // additional grant for a reserved tree or a less-authorized target.
        if canonical != path
            && private_roots
                .iter()
                .any(|cut| identity_within(&canonical, cut))
            && !identity_within(&canonical, &identity)
        {
            continue;
        }
        let mut split = cuts
            .iter()
            .any(|cut| identity_within(cut, &canonical) || identity_within(cut, &path));
        if write && derived && !super::host_access::can_delegate(&path, true)? {
            // A read-only parent such as /home can contain a user-owned writable
            // subtree. Descend to those subtrees without granting the parent.
            if !path.is_dir() {
                continue;
            }
            split = true;
        }
        // PSEC's writable Windows volume-root grant does not recursively cover
        // existing child trees. Keep an uncut root for new root entries and
        // explicitly grant each existing, authorized subtree as well.
        let volume_write = cfg!(windows) && write && path.parent().is_none();
        if volume_write && !split {
            result.push(path.clone());
        }
        if split || volume_write {
            let mut children = std::fs::read_dir(&path)
                .map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!(
                            "cannot partition configured sandbox directory {}: {error}",
                            path.display()
                        ),
                    )
                })?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<io::Result<Vec<_>>>()?;
            children.sort();
            pending.extend(children.into_iter().rev());
        } else {
            result.push(path);
        }
        if result.len() > 8192 {
            return Err(invalid(
                "configured sandbox roots require too many mounts; grant more specific directories",
            ));
        }
    }
    Ok(result)
}

/// Permission of a concrete target, including more-specific read-only roots.
/// Explicit denies are absolute; default denies can be reopened by descendants.
fn access_at(policy: &SandboxPolicy, path: &Path) -> u8 {
    if policy
        .denied_paths
        .iter()
        .any(|root| identity_within(path, root))
    {
        return 0;
    }
    policy
        .readonly_paths
        .iter()
        .map(|root| (root, 1))
        .chain(policy.readwrite_paths.iter().map(|root| (root, 2)))
        .chain(policy.default_denied_paths.iter().map(|root| (root, 0)))
        .filter(|(root, _)| identity_within(path, root))
        .max_by_key(|(root, access)| (root.components().count(), 2 - *access))
        .map_or(0, |(_, access)| access)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contains_grant(paths: &[PathBuf], target: &Path) -> bool {
        paths.iter().any(|root| identity_within(target, root))
    }

    #[cfg(unix)]
    #[test]
    fn broad_grants_skip_inaccessible_and_missing_derived_paths() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let readable = root.join("readable");
        let blocked = root.join("blocked");
        let missing = root.join("missing-link");
        std::fs::create_dir(&readable).unwrap();
        std::fs::create_dir(&blocked).unwrap();
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0)).unwrap();
        symlink(root.join("missing-target"), &missing).unwrap();
        let blocked_readable = super::super::host_access::can_delegate(&blocked, false).unwrap();
        let policy = SandboxPolicy {
            configured_filesystem: true,
            readonly_paths: vec![root.clone()],
            ..Default::default()
        };
        let prepared = partition(&policy, &root.join("tmp"), &[]).unwrap();
        assert!(contains_grant(&prepared.readonly_paths, &readable));
        assert_eq!(
            contains_grant(&prepared.readonly_paths, &blocked),
            blocked_readable
        );
        assert!(!contains_grant(&prepared.readonly_paths, &missing));
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn broad_writable_grant_reaches_writable_children_of_host_readonly_directories() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let parent = root.join("readonly-parent");
        let writable = parent.join("writable-child");
        std::fs::create_dir_all(&writable).unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o555)).unwrap();
        let parent_writable = super::super::host_access::can_delegate(&parent, true).unwrap();
        let policy = SandboxPolicy {
            configured_filesystem: true,
            readwrite_paths: vec![root.clone()],
            ..Default::default()
        };
        let prepared = partition(&policy, &root.join("tmp"), &[]).unwrap();
        assert!(contains_grant(&prepared.readwrite_paths, &writable));
        assert_eq!(
            contains_grant(&prepared.readwrite_paths, &parent),
            parent_writable
        );
        assert!(contains_grant(&prepared.readonly_paths, &parent) || parent_writable);
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn broad_readonly_grant_excludes_shared_scratch_and_reopens_concrete_workspace() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let temporary = root.join("tmp");
        let workspace = temporary.join("project");
        let outside = root.join("reference");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(temporary.join("agena-sandbox-other-task")).unwrap();
        let policy = SandboxPolicy {
            configured_filesystem: true,
            readonly_paths: vec![root.clone()],
            readwrite_paths: vec![workspace.clone()],
            ..Default::default()
        };
        let prepared = partition(&policy, &temporary, &[]).unwrap();
        assert!(contains_grant(&prepared.readonly_paths, &outside));
        assert!(!contains_grant(&prepared.readonly_paths, &temporary));
        assert_eq!(prepared.readwrite_paths, [workspace]);
        assert!(!prepared.configured_filesystem);
    }

    #[test]
    fn default_denied_workspace_can_reopen_a_single_concrete_file() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let workspace = root.join("project");
        let outside = root.join("reference");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let allowed = workspace.join("allowed.txt");
        std::fs::write(&allowed, "allowed").unwrap();
        let policy = SandboxPolicy {
            configured_filesystem: true,
            readonly_paths: vec![root.clone()],
            readwrite_paths: vec![allowed.clone()],
            default_denied_paths: vec![workspace.clone()],
            ..Default::default()
        };
        let prepared = partition(&policy, &root.join("temp"), &[]).unwrap();
        assert!(contains_grant(&prepared.readonly_paths, &outside));
        assert!(!contains_grant(&prepared.readonly_paths, &workspace));
        assert_eq!(prepared.readwrite_paths, [allowed]);
        assert!(prepared.default_denied_paths.is_empty());
        assert!(prepared.denied_paths.is_empty());
    }

    #[test]
    fn broad_write_grant_keeps_trusted_ancestors_readonly_and_siblings_writable() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let trusted = root.join("install/bin");
        let output = root.join("output");
        std::fs::create_dir_all(&trusted).unwrap();
        std::fs::create_dir(&output).unwrap();
        let executable = trusted.join("agena");
        std::fs::write(&executable, "fixture").unwrap();
        let policy = SandboxPolicy {
            configured_filesystem: true,
            readwrite_paths: vec![root.clone()],
            ..Default::default()
        };
        let prepared = partition(&policy, &root.join("temp"), &[executable.clone()]).unwrap();
        assert!(contains_grant(&prepared.readwrite_paths, &output));
        assert!(!contains_grant(&prepared.readwrite_paths, &executable));
        assert!(!contains_grant(&prepared.readwrite_paths, &trusted));
        assert!(contains_grant(&prepared.readonly_paths, &executable));
    }

    #[cfg(unix)]
    #[test]
    fn splitting_writable_parent_does_not_grant_a_readonly_symlink_target() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let workspace = root.join("project");
        let outside = root.join("reference");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let protected = workspace.join("launcher");
        std::fs::write(&protected, "fixture").unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("alias")).unwrap();
        let policy = SandboxPolicy {
            configured_filesystem: true,
            readonly_paths: vec![outside.clone()],
            readwrite_paths: vec![workspace.clone()],
            ..Default::default()
        };
        let prepared = partition(&policy, &root.join("temp"), &[protected]).unwrap();
        assert!(!contains_grant(&prepared.readwrite_paths, &outside));
        assert!(!prepared.readwrite_paths.contains(&workspace.join("alias")));
        assert!(contains_grant(&prepared.readonly_paths, &workspace));
    }

    #[test]
    fn explicit_other_task_grant_is_still_rejected() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let reserved = root.join("agena-sandbox-other-task");
        std::fs::create_dir(&reserved).unwrap();
        let policy = SandboxPolicy {
            configured_filesystem: true,
            readonly_paths: vec![reserved],
            ..Default::default()
        };
        assert!(
            partition(&policy, &root, &[])
                .unwrap_err()
                .to_string()
                .contains("another task")
        );
    }

    #[test]
    fn equal_identity_readonly_boundary_cannot_be_reopened_by_a_write_alias() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let data = root.join("data.txt");
        std::fs::write(&data, "fixture").unwrap();
        let policy = SandboxPolicy {
            configured_filesystem: true,
            readonly_paths: vec![root.clone()],
            readwrite_paths: vec![root.clone()],
            ..Default::default()
        };
        let prepared = partition(&policy, &root.join("temp"), &[]).unwrap();
        assert!(prepared.readwrite_paths.is_empty());
        assert!(contains_grant(&prepared.readonly_paths, &data));
    }
}
