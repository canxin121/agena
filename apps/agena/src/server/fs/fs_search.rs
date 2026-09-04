use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Instant,
};

use axum::{Json, extract::Query};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};

use super::{
    ApiResult, AppError, DEFAULT_FILE_SEARCH_LIMIT, FILE_SEARCH_EXCLUDED_DIRS,
    MAX_FILE_SEARCH_LIMIT, home_dir_env, resolve_path, to_api_path,
};

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub root: Option<String>,
    pub directory: Option<String>,
    pub q: Option<String>,
    #[serde(default, rename = "includeHidden")]
    pub include_hidden: bool,
    #[serde(default = "default_respect_gitignore", rename = "respectGitignore")]
    pub respect_gitignore: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFile {
    pub name: String,
    pub path: String,
    pub relative_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResponse {
    pub root: String,
    pub count: usize,
    pub files: Vec<SearchFile>,
    pub truncated: bool,
}

pub(super) fn default_respect_gitignore() -> bool {
    true
}

pub(super) fn normalize_relative_search_path(root: &Path, target: &Path) -> String {
    let rel = target
        .strip_prefix(root)
        .ok()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| target.file_name().map(Path::new).unwrap_or(target));
    rel.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn fuzzy_match_score_normalized(query: &str, candidate: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }

    let q = query;
    let c = candidate.to_ascii_lowercase();

    if let Some(idx) = c.find(q) {
        let bonus = if idx == 0 {
            20
        } else {
            let prev = c.as_bytes()[idx.saturating_sub(1)] as char;
            if prev == '/' || prev == '_' || prev == '-' || prev == '.' || prev == ' ' {
                15
            } else {
                0
            }
        };
        let score = 100 + bonus - (idx.min(20) as i32) - ((c.len() / 5) as i32);
        return Some(score);
    }

    let mut score: i32 = 0;
    let mut last_index: i32 = -1;
    let mut consecutive: i32 = 0;

    for ch in q.chars() {
        if ch == ' ' {
            continue;
        }
        let start = (last_index + 1).max(0) as usize;
        let idx = (start + c[start..].find(ch)?) as i32;

        let gap = idx - last_index - 1;
        if gap == 0 {
            consecutive += 1;
        } else {
            consecutive = 0;
        }

        score += 10;
        score += (18 - idx).max(0);
        score -= gap.min(10);

        if idx == 0 {
            score += 12;
        } else {
            let prev = c.as_bytes()[(idx - 1) as usize] as char;
            if prev == '/' || prev == '_' || prev == '-' || prev == '.' || prev == ' ' {
                score += 10;
            }
        }

        if consecutive > 0 {
            score += 12;
        }

        last_index = idx;
    }

    score += (24 - (c.len() as i32 / 3)).max(0);
    Some(score)
}

pub async fn fs_search(Query(q): Query<SearchQuery>) -> ApiResult<Json<SearchResponse>> {
    let raw_root = q
        .root
        .or(q.directory)
        .unwrap_or_else(|| home_dir_env().unwrap_or_default());
    let raw_query = q.q.unwrap_or_default();
    let limit = q
        .limit
        .unwrap_or(DEFAULT_FILE_SEARCH_LIMIT)
        .clamp(1, MAX_FILE_SEARCH_LIMIT);

    let resolved_root = resolve_path(&raw_root);
    let abs_root = if resolved_root.is_absolute() {
        resolved_root
    } else {
        std::env::current_dir()
            .map_err(|error| {
                AppError::internal_error_with_context(
                    "resolve the current directory for filesystem search",
                    &error,
                )
            })?
            .join(resolved_root)
    };

    let stats = tokio::fs::metadata(&abs_root)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "read filesystem search root metadata",
                        &err,
                    ),
                    "filesystem search root was not found"
                );
                AppError::not_found("Directory not found")
            }
            std::io::ErrorKind::PermissionDenied => {
                AppError::forbidden_error("read filesystem search root metadata", &err)
            }
            _ => {
                AppError::internal_error_with_context("read filesystem search root metadata", &err)
            }
        })?;
    if !stats.is_dir() {
        return Err(AppError::bad_request("Specified root is not a directory"));
    }

    let query_norm = raw_query.trim().to_ascii_lowercase();
    let match_all = query_norm.is_empty();
    let collect_limit = if match_all {
        limit
    } else {
        (limit * 3).max(200)
    };

    let excluded: HashSet<&'static str> = FILE_SEARCH_EXCLUDED_DIRS.iter().copied().collect();
    let started = Instant::now();

    let abs_root_for_filter = abs_root.clone();
    let mut builder = WalkBuilder::new(&abs_root);
    builder.hidden(!q.include_hidden);
    if !q.respect_gitignore {
        builder.git_ignore(false);
        builder.git_global(false);
        builder.git_exclude(false);
        builder.parents(false);
    }
    builder.follow_links(false);

    let mut candidates: Vec<(SearchFile, i32)> = Vec::new();
    let mut truncated = false;

    for result in builder
        .filter_entry(move |entry| {
            let path = entry.path();
            if path == abs_root_for_filter {
                return true;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                return true;
            };

            let lower = name.to_ascii_lowercase();
            if excluded.contains(lower.as_str()) {
                return false;
            }
            if !q.include_hidden && name.starts_with('.') {
                return false;
            }
            true
        })
        .build()
    {
        let entry = match result {
            Ok(e) => e,
            Err(error) => {
                truncated = true;
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "filesystem search skipped an unreadable workspace entry",
                        &error,
                    ),
                    "filesystem search result is partial"
                );
                continue;
            }
        };

        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        let path = entry.path().to_path_buf();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }

        let relative_path = normalize_relative_search_path(&abs_root, &path);
        let extension = name
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_lowercase())
            .filter(|ext| !ext.is_empty());

        let score = if match_all {
            0
        } else {
            match fuzzy_match_score_normalized(&query_norm, &relative_path) {
                Some(score) => score,
                None => continue,
            }
        };

        candidates.push((
            SearchFile {
                name,
                path: to_api_path(&path),
                relative_path,
                extension,
            },
            score,
        ));

        if candidates.len() >= collect_limit {
            truncated = true;
            break;
        }
    }

    if !match_all {
        candidates.sort_by(|(a, sa), (b, sb)| {
            sb.cmp(sa)
                .then_with(|| a.relative_path.len().cmp(&b.relative_path.len()))
                .then_with(|| a.relative_path.cmp(&b.relative_path))
        });
    }

    if candidates.len() > limit {
        truncated = true;
    }
    let files = candidates
        .into_iter()
        .take(limit)
        .map(|(f, _)| f)
        .collect::<Vec<_>>();

    tracing::debug!(
        "fs_search root={} q='{}' count={} elapsed_ms={}",
        abs_root.to_string_lossy(),
        raw_query,
        files.len(),
        started.elapsed().as_millis()
    );

    Ok(Json(SearchResponse {
        root: to_api_path(&abs_root),
        count: files.len(),
        files,
        truncated,
    }))
}
