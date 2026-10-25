use std::path::{Component, Path};

use ignore::WalkBuilder;

/// Names that commonly contain VCS metadata, dependencies, generated output,
/// or Agena's own durable runtime data. The `ignore` crate handles matching;
/// this list is only used to preserve the explicit-target escape hatch.
const COMMON_IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".agena",
    ".cache",
    "node_modules",
    "target",
    "dist",
    "build",
];

pub(super) fn walk_builder_with_filter(
    base_path: &Path,
    include_ignored: bool,
    access: Option<agena_tool::path_filter::PathPredicate>,
) -> WalkBuilder {
    let mut builder = WalkBuilder::new(base_path);
    builder
        .follow_links(false)
        .standard_filters(!include_ignored)
        .sort_by_file_path(|left, right| left.cmp(right));
    builder.filter_entry(move |entry| {
        (include_ignored
            || entry.depth() == 0
            || !entry.file_type().is_some_and(|kind| kind.is_dir())
            || !entry
                .file_name()
                .to_str()
                .is_some_and(is_common_ignored_directory))
            && access.as_ref().is_none_or(|check| {
                !entry.file_type().is_some_and(|kind| kind.is_dir()) || check.allows(entry.path())
            })
    });
    builder
}

pub(super) fn effective_include_ignored(
    requested: bool,
    base_path: &Path,
    workspace_root: &Path,
) -> bool {
    requested
        || base_path.is_file()
        || explicitly_targets_ignored_directory(base_path, workspace_root)
}

fn explicitly_targets_ignored_directory(base_path: &Path, workspace_root: &Path) -> bool {
    base_path
        .strip_prefix(workspace_root)
        .unwrap_or(base_path)
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .any(|name| name.starts_with('.') || is_common_ignored_directory(name))
}

fn is_common_ignored_directory(name: &str) -> bool {
    COMMON_IGNORED_DIRECTORIES.contains(&name)
}

#[cfg(test)]
mod permission_tests {
    use super::*;
    #[test]
    fn denied_directories_are_pruned_before_descendant_discovery() {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("private");
        std::fs::create_dir(&private).unwrap();
        std::fs::write(private.join("secret.txt"), "private").unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let visited = seen.clone();
        let filter = agena_tool::path_filter::PathFilter::new(move |path| {
            visited.lock().unwrap().push(path.to_owned());
            if path == private {
                Err("child denied")
            } else {
                Ok(())
            }
        });
        let paths = walk_builder_with_filter(root.path(), true, Some(filter.predicate()))
            .build()
            .collect::<Vec<_>>();
        assert_eq!(filter.finish(Ok(paths)).unwrap_err(), "child denied");
        assert!(
            !seen
                .lock()
                .unwrap()
                .iter()
                .any(|path| path.file_name().is_some_and(|name| name == "secret.txt"))
        );
    }
}
