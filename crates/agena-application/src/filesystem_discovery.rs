//! Bound opportunistic discovery before opening macOS privacy domains.
//!
//! A recursive home/root scan must not trigger a new TCC prompt for every app
//! container, standard user folder, or mounted disk it happens to encounter.
//! Explicitly choosing a workspace inside such a domain still allows access.
//! This is a discovery policy, not an authorization check; never probe private
//! files or cache an OS grant based on whether a previous read happened to work.

use std::path::{Component, Path, PathBuf};

/// Run a bounded single-worker walk whose filter executes BEFORE opening a
/// directory. ignore's serial iterator uses walkdir, which opens directories
/// before calling filter_entry; skipping afterwards is too late for TCC.
pub fn visit(
    builder: &mut ignore::WalkBuilder,
    visitor: impl FnMut(Result<ignore::DirEntry, ignore::Error>) -> ignore::WalkState + Send,
) {
    let visitor = std::sync::Mutex::new(visitor);
    builder
        .threads(1)
        .build_parallel()
        .run(|| Box::new(|entry| visitor.lock().expect("discovery visitor poisoned")(entry)));
}

/// Whether recursive file discovery may descend into `path` from `root`.
/// Direct directory listing and explicit file reads remain available.
pub fn may_descend(root: &Path, path: &Path) -> bool {
    if cfg!(target_os = "macos") {
        may_descend_on_macos(root, path)
    } else {
        true
    }
}

fn may_descend_on_macos(root: &Path, path: &Path) -> bool {
    if root == path {
        return true;
    }
    // Do not traverse mounted drives just because the user indexed / or
    // /Volumes. Selecting /Volumes/MyDrive or a project under it is explicit.
    for base in ["/Volumes", "/Network"] {
        let base = Path::new(base);
        if path.starts_with(base) && !root.starts_with(base) {
            return false;
        }
        if root == base && path != base {
            return false;
        }
    }
    for base in ["/Library", "/System", "/Applications", "/private/var"] {
        let base = Path::new(base);
        if path.starts_with(base) && !root.starts_with(base) {
            return false;
        }
    }

    let mut components = path.components();
    if components.next() != Some(Component::RootDir)
        || components.next() != Some(Component::Normal("Users".as_ref()))
    {
        return true;
    }
    let Some(Component::Normal(user)) = components.next() else {
        return true;
    };
    let Some(Component::Normal(folder)) = components.next() else {
        return true;
    };
    if matches!(
        folder.to_str(),
        Some(
            "Library"
                | "Desktop"
                | "Documents"
                | "Downloads"
                | "Pictures"
                | "Music"
                | "Movies"
                | ".Trash"
        )
    ) {
        let boundary = PathBuf::from("/Users").join(user).join(folder);
        return root.starts_with(boundary);
    }
    true
}
