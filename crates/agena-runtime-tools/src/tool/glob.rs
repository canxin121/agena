use globset::{Glob, GlobMatcher};
use walkdir::WalkDir;

use crate::message::GlobToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    normalize_path_for_display,
};

const DEFAULT_MATCHES: usize = 200;
const MAX_MATCHES: usize = 1_000;
const DEFAULT_IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    ".cache",
];

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &GlobToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let base_path = input
        .path
        .as_deref()
        .map(|path| executor.resolve_target_path(path))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());

    if !base_path.exists() {
        return Err(ToolError::invalid_field(
            "path",
            agena_failure::FieldIssueKind::NotFound,
            format!(
                "glob base path does not exist: {}",
                executor.display_path(&base_path)
            ),
        ));
    }
    if !base_path.is_dir() {
        return Err(ToolError::invalid_field(
            "path",
            agena_failure::FieldIssueKind::Invalid,
            format!(
                "glob base path is not a directory: {}",
                executor.display_path(&base_path)
            ),
        ));
    }

    let matcher = Glob::new(&input.pattern)?.compile_matcher();
    let offset = input.offset.unwrap_or_default() as usize;
    let limit = input.limit.unwrap_or(DEFAULT_MATCHES as u32) as usize;
    if limit == 0 || limit > MAX_MATCHES {
        return Err(ToolError::invalid_field(
            "limit",
            agena_failure::FieldIssueKind::OutOfRange,
            format!("glob limit must be between 1 and {MAX_MATCHES}"),
        ));
    }
    let include_ignored = input.include_ignored
        || explicitly_targets_ignored_directory(&base_path, executor.workspace_root());
    let (matched_paths, truncated) =
        collect_matches(&base_path, &matcher, offset, limit, include_ignored)?;
    let mut matches = matched_paths
        .iter()
        .map(|path| executor.display_path(path))
        .collect::<Vec<_>>();
    matches.sort();

    let output_text = if matches.is_empty() {
        "No paths matched the glob pattern.".to_string()
    } else {
        matches.join("\n")
    };
    let output = ToolPayloadOutput::Glob {
        count: Some(matches.len() as u32),
        paths: matches.clone(),
        truncated,
    };

    let mut view = ToolExecutionView::simple(format!("Glob {}", input.pattern), output_text);
    view.metadata
        .insert("pattern".to_string(), input.pattern.clone());
    view.metadata
        .insert("base_path".to_string(), executor.display_path(&base_path));
    view.metadata
        .insert("count".to_string(), matches.len().to_string());
    view.metadata
        .insert("offset".to_string(), offset.to_string());
    view.metadata.insert("limit".to_string(), limit.to_string());
    view.metadata
        .insert("include_ignored".to_string(), include_ignored.to_string());
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());

    Ok(ToolPayloadExecution::new(output, view))
}

fn collect_matches(
    base_path: &std::path::Path,
    matcher: &GlobMatcher,
    offset: usize,
    limit: usize,
    include_ignored: bool,
) -> Result<(Vec<std::path::PathBuf>, bool), ToolError> {
    let mut matches = Vec::with_capacity(limit.min(DEFAULT_MATCHES));
    let mut skipped_matches = 0_usize;

    for entry in WalkDir::new(base_path)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry.path() == base_path
                || include_ignored
                || !entry.file_type().is_dir()
                || !entry
                    .file_name()
                    .to_str()
                    .is_some_and(is_default_ignored_directory)
        })
        .filter_map(Result::ok)
    {
        if entry.path() == base_path {
            continue;
        }

        let relative = entry.path().strip_prefix(base_path).map_err(|err| {
            ToolError::invalid_input(format!(
                "glob failed to build relative path for '{}': {err}",
                entry.path().display()
            ))
        })?;

        let relative_norm = normalize_path_for_display(relative);
        if matcher.is_match(&relative_norm) {
            if skipped_matches < offset {
                skipped_matches += 1;
                continue;
            }
            if matches.len() >= limit {
                return Ok((matches, true));
            }
            matches.push(entry.into_path());
        }
    }

    Ok((matches, false))
}

fn is_default_ignored_directory(name: &str) -> bool {
    DEFAULT_IGNORED_DIRECTORIES.contains(&name)
}

fn explicitly_targets_ignored_directory(
    base_path: &std::path::Path,
    workspace_root: &std::path::Path,
) -> bool {
    base_path
        .strip_prefix(workspace_root)
        .unwrap_or(base_path)
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .any(is_default_ignored_directory)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use globset::Glob;

    use super::{collect_matches, explicitly_targets_ignored_directory};

    #[test]
    fn glob_pages_deterministically_and_skips_heavy_directories_by_default() {
        let root =
            std::env::temp_dir().join(format!("agena-glob-test-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).expect("temporary glob workspace");
        for path in [
            "src/a.rs",
            "src/b.rs",
            "src/c.rs",
            "node_modules/dependency.rs",
            "target/generated.rs",
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
            fs::write(path, "fixture").expect("write fixture");
        }
        let matcher = Glob::new("**/*.rs").expect("glob").compile_matcher();

        let (first, first_truncated) =
            collect_matches(&root, &matcher, 0, 2, false).expect("first page");
        let (second, second_truncated) =
            collect_matches(&root, &matcher, 2, 2, false).expect("second page");
        assert_eq!(
            first
                .iter()
                .map(|path| path.strip_prefix(&root).expect("relative"))
                .collect::<Vec<_>>(),
            vec![
                std::path::Path::new("src/a.rs"),
                std::path::Path::new("src/b.rs")
            ]
        );
        assert!(first_truncated);
        assert_eq!(
            second
                .iter()
                .map(|path| path.strip_prefix(&root).expect("relative"))
                .collect::<Vec<_>>(),
            vec![std::path::Path::new("src/c.rs")]
        );
        assert!(!second_truncated);

        let (all, _) = collect_matches(&root, &matcher, 0, 10, true).expect("ignored paths");
        assert_eq!(all.len(), 5);
        fs::remove_dir_all(root).expect("remove glob fixture");
    }

    #[test]
    fn explicit_ignored_base_path_reenables_traversal() {
        let workspace = std::path::Path::new("/workspace");
        assert!(!explicitly_targets_ignored_directory(workspace, workspace));
        assert!(explicitly_targets_ignored_directory(
            &workspace.join("node_modules/package"),
            workspace
        ));
    }
}
