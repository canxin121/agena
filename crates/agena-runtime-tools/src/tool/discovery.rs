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

pub(super) fn walk_builder(base_path: &Path, include_ignored: bool) -> WalkBuilder {
    let mut builder = WalkBuilder::new(base_path);
    builder
        .follow_links(false)
        .standard_filters(!include_ignored)
        .sort_by_file_path(|left, right| left.cmp(right));
    if !include_ignored {
        builder.filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_some_and(|kind| kind.is_dir())
                || !entry
                    .file_name()
                    .to_str()
                    .is_some_and(is_common_ignored_directory)
        });
    }
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
mod tests {
    use super::effective_include_ignored;

    #[test]
    fn explicit_hidden_or_generated_base_reenables_ignored_paths() {
        let workspace = std::path::Path::new("/workspace");

        assert!(!effective_include_ignored(false, workspace, workspace));
        assert!(!effective_include_ignored(
            false,
            &workspace.join("crates/runtime"),
            workspace,
        ));
        assert!(effective_include_ignored(
            false,
            &workspace.join(".github/workflows"),
            workspace,
        ));
        assert!(effective_include_ignored(
            false,
            &workspace.join("target/debug"),
            workspace,
        ));
        assert!(effective_include_ignored(
            true,
            &workspace.join("crates/runtime"),
            workspace,
        ));
    }
}
