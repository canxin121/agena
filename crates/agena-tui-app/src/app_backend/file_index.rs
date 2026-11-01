//! Workspace file search: a lazily-built ignore-aware file index plus the
//! ranking helpers that power file-mention suggestions in the composer.

use std::{
    collections::BinaryHeap,
    path::{Path, PathBuf},
};

use anyhow::Result;

/// Precompute Unicode case folding and path metadata once when the server
/// refreshes its workspace inventory. Completion keystrokes must not allocate
/// two lowercase strings for every file in a 50k-entry workspace.
pub(crate) struct IndexedWorkspaceFile {
    path: PathBuf,
    path_lower: String,
    filename_lower: String,
    depth: usize,
    length: usize,
}

impl IndexedWorkspaceFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path_lower: path.to_string_lossy().to_lowercase(),
            filename_lower: path
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
            depth: path.components().count(),
            length: path.as_os_str().len(),
            path,
        }
    }

    fn score(&self, query_lower: &str) -> Option<(u8, usize, usize)> {
        if query_lower.is_empty() {
            return Some((4, self.depth, self.length));
        }

        if self.filename_lower == query_lower {
            return Some((0, self.filename_lower.len(), self.path_lower.len()));
        }
        if self.filename_lower.starts_with(query_lower) {
            return Some((1, self.filename_lower.len(), self.path_lower.len()));
        }
        if let Some(index) = self.filename_lower.find(query_lower) {
            return Some((2, index, self.path_lower.len()));
        }

        self.path_lower
            .find(query_lower)
            .map(|index| (3, index, self.path_lower.len()))
    }
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

    let typed = Path::new(trimmed);
    let direct = if typed.is_absolute() {
        typed
            .strip_prefix(application.workspace_root())
            .map(Path::to_path_buf)
            .ok()
    } else {
        Some(typed.to_path_buf())
    };
    // The backend publishes the index in Path order. Look up an absolute or
    // backslash-normalized exact path without another linear tree walk.
    let direct = direct
        .map(|path| PathBuf::from(path.to_string_lossy().replace('\\', "/")))
        .and_then(|path| {
            index
                .binary_search_by(|item| item.path.cmp(&path))
                .ok()
                .map(|i| &index[i])
        });
    Ok(rank_workspace_files(&index, &query_lower, direct, limit))
}

/// Keep only the best `limit` matches. Sorting every match and cloning every
/// path was O(N log N) work and O(N) allocations on each composer keystroke.
/// The max-heap retains the original tie-break ordering in O(N log limit).
fn rank_workspace_files(
    index: &[IndexedWorkspaceFile],
    query_lower: &str,
    direct: Option<&IndexedWorkspaceFile>,
    limit: usize,
) -> Vec<PathBuf> {
    type Rank = ((u8, usize, usize), usize, usize, PathBuf);
    let mut best = BinaryHeap::<Rank>::new();
    let mut consider = |item: &IndexedWorkspaceFile, score| {
        let path = &item.path;
        let depth = item.depth;
        let length = item.length;
        if best.len() >= limit
            && let Some(worst) = best.peek()
            && (score, depth, length, path) >= (worst.0, worst.1, worst.2, &worst.3)
        {
            return;
        }
        best.push((score, depth, length, path.clone()));
        if best.len() > limit {
            best.pop();
        }
    };
    for item in index {
        if let Some(score) = item.score(query_lower) {
            consider(item, score);
        }
    }
    // Preserve exact-path selection when the typed absolute/Windows path is
    // not a substring of the relative index entry. Avoid inserting a second
    // copy of a normally scored entry.
    if let Some(item) = direct
        && item.score(query_lower).is_none()
    {
        consider(item, (0, 0, 0));
    }
    best.into_sorted_vec()
        .into_iter()
        .map(|(_, _, _, path)| path)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent reference for the old per-keystroke scoring policy. Cached
    // preprocessing must change cost, never Unicode matching or ranking.
    fn reference_score(path: &Path, query: &str) -> Option<(u8, usize, usize)> {
        if query.is_empty() {
            return Some((4, path.components().count(), path.as_os_str().len()));
        }
        let full = path.to_string_lossy().to_lowercase();
        let filename = path
            .file_name()
            .map(|value| value.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if filename == query {
            Some((0, filename.len(), full.len()))
        } else if filename.starts_with(query) {
            Some((1, filename.len(), full.len()))
        } else if let Some(position) = filename.find(query) {
            Some((2, position, full.len()))
        } else {
            full.find(query).map(|position| (3, position, full.len()))
        }
    }

    #[test]
    fn bounded_ranking_preserves_the_full_sort_order() {
        let index = (0..500)
            .rev()
            .map(|i| PathBuf::from(format!("dir{}/alpha{i:03}.rs", i % 19)))
            .chain([
                PathBuf::from("alpha.rs"),
                PathBuf::from("src/alpha.rs"),
                PathBuf::from("alpha/hidden.rs"),
                PathBuf::from("文档/测试.rs"),
                PathBuf::from("UPPER/Alpha.rs"),
            ])
            .map(IndexedWorkspaceFile::new)
            .collect::<Vec<_>>();
        for query in ["", "alpha", "hidden", "dir", "测试", "rs", "not-found"] {
            let mut expected = index
                .iter()
                .filter_map(|item| {
                    reference_score(&item.path, query)
                        .map(|score| (score, item.depth, item.length, item.path.clone()))
                })
                .collect::<Vec<_>>();
            expected.sort();
            for limit in [1, 2, 10, 100, 1000] {
                assert_eq!(
                    rank_workspace_files(&index, query, None, limit),
                    expected
                        .iter()
                        .take(limit)
                        .map(|item| item.3.clone())
                        .collect::<Vec<_>>(),
                    "query={query}, limit={limit}"
                );
            }
        }
    }

    #[test]
    fn direct_match_can_displace_a_lower_ranked_result() {
        let index = vec![
            IndexedWorkspaceFile::new(PathBuf::from("a/foo")),
            IndexedWorkspaceFile::new(PathBuf::from("b/foo")),
        ];
        assert_eq!(
            rank_workspace_files(&index, "/workspace/b/foo", Some(&index[1]), 1),
            vec![PathBuf::from("b/foo")]
        );
        assert!(rank_workspace_files(&index, "foo", None, 0).is_empty());
    }
}
