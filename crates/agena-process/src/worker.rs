//! Trusted application workers share private working directories and lifecycle
//! handling. The caller supplies a policy compiled from live host authority.
use crate::sandbox::{self, SandboxPolicy};
use std::{
    io,
    path::{Path, PathBuf},
};

/// Reduce a compiled filesystem policy to one existing read target. A trusted
/// worker request cannot manufacture a new grant outside the original policy.
pub fn restrict_read(policy: &SandboxPolicy, path: &Path) -> io::Result<SandboxPolicy> {
    let target = std::fs::canonicalize(path)?;
    let matching_depth = |root: &Path| -> io::Result<Option<usize>> {
        let root = sandbox::policy_path_identity(root)?;
        Ok(sandbox::identity_within(&target, &root).then(|| root.components().count()))
    };
    let grant_depth = policy
        .readonly_paths
        .iter()
        .chain(&policy.readwrite_paths)
        .map(|root| matching_depth(root))
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .max();
    let default_deny_depth = policy
        .default_denied_paths
        .iter()
        .map(|root| matching_depth(root))
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .max();
    let explicitly_denied = policy
        .denied_paths
        .iter()
        .map(|root| matching_depth(root))
        .collect::<io::Result<Vec<_>>>()?
        .into_iter()
        .any(|depth| depth.is_some());
    // Match configured native mounts: a class default cuts an inherited broad
    // grant, and only a strictly more-specific grant reopens that subtree.
    // An explicit deny always wins, including over more-specific grants.
    let allowed =
        grant_depth.is_some_and(|grant| default_deny_depth.is_none_or(|denied| grant > denied));
    if !allowed || explicitly_denied {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "worker read is outside the compiled filesystem policy",
        ));
    }
    let mut narrowed = policy.clone();
    narrowed.readonly_paths = vec![target];
    narrowed.readwrite_paths.clear();
    narrowed.private_paths.clear();
    narrowed.configured_filesystem = true;
    Ok(narrowed)
}

pub fn command(
    argument: &str,
    policy: &SandboxPolicy,
) -> io::Result<(tempfile::TempDir, tokio::process::Command)> {
    if !policy.disabled && !sandbox::enabled() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "MXC is required by this operation but the native sandbox launcher is disabled",
        ));
    }
    let scratch = tempfile::Builder::new()
        .prefix("agena-native-worker-")
        .tempdir()?;
    let cwd = std::fs::canonicalize(scratch.path())?;
    let mut policy = policy.clone();
    policy.private_paths.push(cwd.clone());
    policy.readwrite_paths.push(cwd);
    let executable: PathBuf = sandbox::launcher_executable()?;
    let environment = sandbox::sanitize_environment(&std::env::vars().collect());
    let mut command = sandbox::command(
        executable
            .to_str()
            .ok_or_else(|| io::Error::other("worker executable is not UTF-8"))?,
        &[argument.to_owned()],
        scratch.path(),
        &environment,
        &policy,
    )?;
    if policy.disabled {
        command.env_clear().envs(environment);
    }
    Ok((scratch, command))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let workspace = root.join("workspace");
        let input = workspace.join("input");
        std::fs::create_dir_all(&input).unwrap();
        let target = input.join("document.txt");
        std::fs::write(&target, "input").unwrap();
        (directory, root, workspace, target)
    }

    fn default_denied_workspace(root: &Path, workspace: &Path) -> SandboxPolicy {
        SandboxPolicy {
            configured_filesystem: true,
            readonly_paths: vec![root.ancestors().last().unwrap().to_owned()],
            default_denied_paths: vec![workspace.to_owned()],
            ..Default::default()
        }
    }

    #[test]
    fn broad_external_read_cannot_reopen_a_default_denied_workspace() {
        let (_directory, root, workspace, target) = fixture();
        let policy = default_denied_workspace(&root, &workspace);
        let error = restrict_read(&policy, &target).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        let outside = root.join("reference.txt");
        std::fs::write(&outside, "reference").unwrap();
        let narrowed = restrict_read(&policy, &outside).unwrap();
        assert_eq!(narrowed.readonly_paths, [outside]);
        assert_eq!(narrowed.default_denied_paths, [workspace]);
        assert!(narrowed.readwrite_paths.is_empty());
    }

    #[test]
    fn concrete_descendant_grants_can_reopen_only_their_default_denied_subtree() {
        let (_directory, root, workspace, target) = fixture();
        let mut policy = default_denied_workspace(&root, &workspace);
        policy
            .readonly_paths
            .push(target.parent().unwrap().to_owned());
        let narrowed = restrict_read(&policy, &target).unwrap();
        assert_eq!(narrowed.readonly_paths, [target.clone()]);
        assert_eq!(narrowed.default_denied_paths, [workspace.clone()]);

        let sibling = workspace.join("secret.txt");
        std::fs::write(&sibling, "secret").unwrap();
        assert_eq!(
            restrict_read(&policy, &sibling).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );

        // An exact read-write grant also includes read access, but narrowing
        // must remove the write grant and every unrelated private directory.
        policy.readonly_paths.pop();
        policy.readwrite_paths.push(target.clone());
        policy.private_paths.push(root.join("other-worker"));
        let narrowed = restrict_read(&policy, &target).unwrap();
        assert_eq!(narrowed.readonly_paths, [target]);
        assert!(narrowed.readwrite_paths.is_empty());
        assert!(narrowed.private_paths.is_empty());
        assert!(narrowed.configured_filesystem);
    }

    #[test]
    fn deny_at_the_same_path_and_explicit_denied_ancestors_win() {
        let (_directory, root, workspace, target) = fixture();
        let mut policy = default_denied_workspace(&root, &workspace);
        policy.readwrite_paths.push(workspace.clone());
        assert_eq!(
            restrict_read(&policy, &target).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );

        policy.readwrite_paths.push(target.clone());
        policy
            .denied_paths
            .push(target.parent().unwrap().to_owned());
        assert_eq!(
            restrict_read(&policy, &target).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_aliases_cannot_hide_a_default_deny_or_an_explicit_deny() {
        use std::os::unix::fs::symlink;

        let (_directory, root, workspace, target) = fixture();
        let alias = root.join("workspace-alias");
        symlink(&workspace, &alias).unwrap();
        let alias_target = alias.join("input/document.txt");
        let mut policy = default_denied_workspace(&root, &alias);
        assert_eq!(
            restrict_read(&policy, &target).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );

        policy.readonly_paths.push(alias.join("input"));
        assert_eq!(
            restrict_read(&policy, &alias_target)
                .unwrap()
                .readonly_paths,
            [target.clone()]
        );
        policy.denied_paths.push(workspace.join("input"));
        assert_eq!(
            restrict_read(&policy, &alias_target).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
