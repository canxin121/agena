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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_discovery_does_not_enter_protected_domains() {
        for folder in [
            "Library",
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Music",
            "Movies",
            ".Trash",
        ] {
            let protected = PathBuf::from("/Users/alice").join(folder);
            assert!(!may_descend_on_macos(Path::new("/Users/alice"), &protected));
            assert!(!may_descend_on_macos(
                Path::new("/"),
                &protected.join("nested")
            ));
            assert!(may_descend_on_macos(&protected, &protected.join("project")));
        }
        assert!(may_descend_on_macos(
            Path::new("/Users/alice"),
            Path::new("/Users/alice/Projects/Library/src")
        ));
    }

    #[test]
    fn external_drive_requires_explicit_scope() {
        assert!(!may_descend_on_macos(Path::new("/"), Path::new("/Volumes")));
        assert!(!may_descend_on_macos(
            Path::new("/Volumes"),
            Path::new("/Volumes/External")
        ));
        assert!(may_descend_on_macos(
            Path::new("/Volumes/External"),
            Path::new("/Volumes/External/project")
        ));
        assert!(!may_descend_on_macos(
            Path::new("/"),
            Path::new("/Library/Application Support")
        ));
        assert!(may_descend_on_macos(
            Path::new("/Library/Example"),
            Path::new("/Library/Example/src")
        ));
    }

    #[test]
    fn filtered_walk_preserves_ignore_rules_and_stops_at_the_budget() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::create_dir(fixture.path().join("blocked")).unwrap();
        std::fs::write(fixture.path().join("blocked/private.txt"), "private").unwrap();
        std::fs::write(fixture.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(fixture.path().join("ignored.txt"), "ignored").unwrap();
        for i in 0..8 {
            std::fs::write(fixture.path().join(format!("public-{i}.txt")), "public").unwrap();
        }
        let mut builder = ignore::WalkBuilder::new(fixture.path());
        builder
            .require_git(false)
            .filter_entry(|entry| entry.file_name() != "blocked");
        let mut names = Vec::new();
        visit(&mut builder, |entry| {
            let entry = entry.unwrap();
            if entry.file_type().is_some_and(|kind| kind.is_file()) {
                names.push(entry.file_name().to_owned());
            }
            if names.len() == 3 {
                ignore::WalkState::Quit
            } else {
                ignore::WalkState::Continue
            }
        });
        assert_eq!(names.len(), 3);
        assert!(
            names
                .iter()
                .all(|name| name.to_string_lossy().starts_with("public-"))
        );
    }
}
