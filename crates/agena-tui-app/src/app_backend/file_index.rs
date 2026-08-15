//! Workspace file search: a lazily-built ignore-aware file index plus the
//! ranking helpers that power file-mention suggestions in the composer.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use anyhow::Result;
use ignore::WalkBuilder;

/// Builds a sorted index of every file under `workspace_root` (relative
/// paths), honoring ignore rules the same way the runtime's own file scanning
/// does.
pub(crate) fn build_file_index(workspace_root: &Path) -> Vec<PathBuf> {
    let mut builder = WalkBuilder::new(workspace_root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .follow_links(false)
        .parents(true)
        .require_git(false);

    let mut files = builder
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(workspace_root)
                .ok()
                .map(Path::to_path_buf)
        })
        .collect::<Vec<_>>();

    files.sort();
    files
}

pub(crate) fn file_search_score(path: &Path, query_lower: &str) -> Option<(u8, usize, usize)> {
    if query_lower.is_empty() {
        return Some((4, path.components().count(), path.as_os_str().len()));
    }

    let path_text = path.to_string_lossy();
    let path_lower = path_text.to_lowercase();
    let filename_lower = path
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if filename_lower == query_lower {
        return Some((0, filename_lower.len(), path_lower.len()));
    }
    if filename_lower.starts_with(query_lower) {
        return Some((1, filename_lower.len(), path_lower.len()));
    }
    if let Some(index) = filename_lower.find(query_lower) {
        return Some((2, index, path_lower.len()));
    }

    path_lower
        .find(query_lower)
        .map(|index| (3, index, path_lower.len()))
}

pub(crate) fn direct_path_candidate(workspace_root: &Path, query: &str) -> Option<PathBuf> {
    if query.is_empty() {
        return None;
    }

    let typed = Path::new(query);
    let resolved = if typed.is_absolute() {
        typed.to_path_buf()
    } else {
        workspace_root.join(typed)
    };
    if !resolved.is_file() {
        return None;
    }

    resolved
        .strip_prefix(workspace_root)
        .map(Path::to_path_buf)
        .ok()
        .or(Some(resolved))
}

/// Ranks workspace files against `query`. The ignore-aware index is built
/// once per process (owned by `App`) and reused across suggestions. Kept
/// synchronous: this runs directly inside the composer's key handler.
pub(crate) fn search_workspace_files(
    application: &crate::TuiBackend,
    file_index: &Arc<OnceLock<Vec<PathBuf>>>,
    query: &str,
    limit: usize,
) -> Result<Vec<PathBuf>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let trimmed = query.trim();
    let query_lower = trimmed.to_lowercase();
    let workspace_root = application.workspace_root();
    let index = file_index.get_or_init(|| build_file_index(workspace_root));

    let mut matches = index
        .iter()
        .filter_map(|path| {
            let score = file_search_score(path, query_lower.as_str())?;
            Some((score, path.clone()))
        })
        .collect::<Vec<_>>();

    if let Some(path) = direct_path_candidate(workspace_root, trimmed) {
        let already_present = matches.iter().any(|(_, existing)| existing == &path);
        if !already_present {
            matches.push(((0, 0, 0), path));
        }
    }

    matches.sort_by(|(score_a, path_a), (score_b, path_b)| {
        score_a
            .cmp(score_b)
            .then_with(|| {
                path_a
                    .components()
                    .count()
                    .cmp(&path_b.components().count())
            })
            .then_with(|| path_a.as_os_str().len().cmp(&path_b.as_os_str().len()))
            .then_with(|| path_a.cmp(path_b))
    });
    matches.truncate(limit);
    Ok(matches.into_iter().map(|(_, path)| path).collect())
}
