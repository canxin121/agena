//! Workspace file search: a lazily-built ignore-aware file index plus the
//! ranking helpers that power file-mention suggestions in the composer.

use std::path::{Path, PathBuf};

use anyhow::Result;

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

/// Ranks workspace files against `query`. The ignore-aware index is built
/// and owned by the server. Kept synchronous: this runs directly inside the
/// composer's key handler over the backend's cached snapshot.
pub(crate) fn search_workspace_files(
    application: &crate::TuiBackend,
    query: &str,
    limit: usize,
) -> Result<Vec<PathBuf>> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let trimmed = query.trim();
    let query_lower = trimmed.to_lowercase();
    let index = application.workspace_file_index();

    let mut matches = index
        .iter()
        .filter_map(|path| {
            let score = file_search_score(path, query_lower.as_str())?;
            Some((score, path.clone()))
        })
        .collect::<Vec<_>>();

    let typed = Path::new(trimmed);
    let direct = if typed.is_absolute() {
        typed
            .strip_prefix(application.workspace_root())
            .map(Path::to_path_buf)
            .ok()
    } else {
        Some(typed.to_path_buf())
    };
    if let Some(path) = direct.and_then(|direct| {
        index
            .iter()
            .find(|path| {
                path.to_string_lossy().replace('\\', "/")
                    == direct.to_string_lossy().replace('\\', "/")
            })
            .cloned()
    }) {
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
