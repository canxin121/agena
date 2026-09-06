use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Instant,
};

use axum::{Json, extract::Query, http::HeaderMap};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

mod replacement;
pub use replacement::fs_content_replace;

#[cfg(test)]
mod tests;

use super::fs_core::{ProjectDirQuery, resolve_project_directory};
use super::fs_search::{default_respect_gitignore, normalize_relative_search_path};
use super::{
    ApiResult, AppError, DEFAULT_CONTENT_SEARCH_CONTEXT_CHARS,
    DEFAULT_CONTENT_SEARCH_MAX_MATCHES_PER_FILE, DEFAULT_CONTENT_SEARCH_MAX_RESULTS,
    FILE_SEARCH_EXCLUDED_DIRS, MAX_CONTENT_REPLACE_PATHS, MAX_CONTENT_SEARCH_CONTEXT_CHARS,
    MAX_CONTENT_SEARCH_FILE_BYTES, MAX_CONTENT_SEARCH_MAX_MATCHES_PER_FILE,
    MAX_CONTENT_SEARCH_MAX_RESULTS, ensure_within_base, has_parent_dir_component,
    normalize_directory_path, to_api_path,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSearchBody {
    pub query: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default = "default_respect_gitignore")]
    pub respect_gitignore: bool,
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    pub max_results: Option<usize>,
    pub max_matches_per_file: Option<usize>,
    pub context_chars: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSearchMatch {
    pub line: usize,
    pub start_column: usize,
    pub end_column: usize,
    pub start_offset: usize,
    pub end_offset: usize,
    pub before: String,
    pub matched: String,
    pub after: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSearchFileResult {
    pub path: String,
    pub relative_path: String,
    pub revision: String,
    pub match_count: usize,
    pub matches: Vec<ContentSearchMatch>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSearchResponse {
    pub root: String,
    pub query: String,
    pub file_count: usize,
    pub match_count: usize,
    pub files: Vec<ContentSearchFileResult>,
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentReplaceMatchRef {
    pub path: Option<String>,
    pub start_offset: Option<usize>,
    pub end_offset: Option<usize>,
    pub expected: Option<String>,
    pub expected_revision: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContentReplaceBody {
    pub query: Option<String>,
    pub replace: Option<String>,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default = "default_respect_gitignore")]
    pub respect_gitignore: bool,
    #[serde(default)]
    pub is_regex: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub paths: Vec<String>,
    pub expected_revisions: Option<HashMap<String, String>>,
    pub r#match: Option<ContentReplaceMatchRef>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentReplaceFileResult {
    pub path: String,
    pub relative_path: String,
    pub replacements: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentReplaceResponse {
    pub root: String,
    pub file_count: usize,
    pub replacement_count: usize,
    pub skipped: usize,
    pub truncated: bool,
    pub files: Vec<ContentReplaceFileResult>,
}

fn resolve_path_within_workspace(base: &Path, target: &str) -> ApiResult<PathBuf> {
    let target_trimmed = target.trim();
    if target_trimmed.is_empty() {
        return Err(AppError::bad_request("Path is required"));
    }

    let normalized = normalize_directory_path(target_trimmed);
    let mut candidate = PathBuf::from(normalized);
    if !candidate.is_absolute() {
        candidate = base.join(candidate);
    }

    if has_parent_dir_component(&candidate) {
        return Err(AppError::bad_request(
            "Invalid path: path traversal not allowed",
        ));
    }
    ensure_within_base(base, &candidate)?;
    Ok(candidate)
}

fn build_content_regex(
    query: &str,
    is_regex: bool,
    case_sensitive: bool,
    whole_word: bool,
) -> ApiResult<Regex> {
    let q = query.trim();
    if q.is_empty() {
        return Err(AppError::bad_request("Search query is required"));
    }

    let mut pattern = if is_regex {
        q.to_string()
    } else {
        regex::escape(q)
    };

    if whole_word {
        pattern = format!(r"\b(?:{})\b", pattern);
    }

    let mut builder = RegexBuilder::new(&pattern);
    builder.case_insensitive(!case_sensitive);

    let regex = builder
        .build()
        .map_err(|err| AppError::bad_request(format!("Invalid search pattern: {}", err)))?;

    if regex.is_match("") {
        return Err(AppError::bad_request(
            "Search pattern matches empty string; refine the query",
        ));
    }

    Ok(regex)
}

fn walk_workspace_files(
    root: &Path,
    include_hidden: bool,
    respect_gitignore: bool,
    max_files: usize,
) -> (Vec<PathBuf>, bool) {
    let excluded: HashSet<&'static str> = FILE_SEARCH_EXCLUDED_DIRS.iter().copied().collect();
    let root_for_filter = root.to_path_buf();

    let mut builder = WalkBuilder::new(root);
    builder.hidden(!include_hidden);
    if !respect_gitignore {
        builder.git_ignore(false);
        builder.git_global(false);
        builder.git_exclude(false);
        builder.parents(false);
    }
    builder.follow_links(false);

    let mut files = Vec::new();
    let mut truncated = false;

    for result in builder
        .filter_entry(move |entry| {
            let path = entry.path();
            if path == root_for_filter {
                return true;
            }

            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                return true;
            };

            let lower = name.to_ascii_lowercase();
            if excluded.contains(lower.as_str()) {
                return false;
            }
            if !include_hidden && name.starts_with('.') {
                return false;
            }
            true
        })
        .build()
    {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                truncated = true;
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "filesystem content discovery skipped an unreadable workspace entry",
                        &error,
                    ),
                    "filesystem content discovery result is partial"
                );
                continue;
            }
        };

        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        files.push(entry.path().to_path_buf());
        if files.len() >= max_files {
            truncated = true;
            break;
        }
    }

    (files, truncated)
}

async fn normalize_content_scope_paths(
    root: &Path,
    paths: &[String],
    include_hidden: bool,
    respect_gitignore: bool,
) -> ApiResult<(Vec<PathBuf>, bool)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut truncated = paths.len() > MAX_CONTENT_REPLACE_PATHS;

    for raw in paths.iter().take(MAX_CONTENT_REPLACE_PATHS) {
        if out.len() >= MAX_CONTENT_REPLACE_PATHS {
            truncated = true;
            break;
        }
        let resolved = resolve_path_within_workspace(root, raw)?;

        let meta = match tokio::fs::metadata(&resolved).await {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                return Err(AppError::forbidden_error(
                    "inspect an explicitly selected content-search path",
                    &err,
                ));
            }
            Err(err) => return Err(AppError::internal_error(&err)),
        };

        if meta.is_file() {
            let key = resolved.to_string_lossy().into_owned();
            if seen.insert(key) {
                out.push(resolved);
            }
            continue;
        }

        if !meta.is_dir() {
            continue;
        }

        let (nested, nested_truncated) = walk_workspace_files(
            &resolved,
            include_hidden,
            respect_gitignore,
            MAX_CONTENT_REPLACE_PATHS.saturating_sub(out.len()),
        );
        truncated |= nested_truncated;
        for file in nested {
            if !file.starts_with(root) {
                continue;
            }
            let key = file.to_string_lossy().into_owned();
            if seen.insert(key) {
                out.push(file);
            }
            if out.len() >= MAX_CONTENT_REPLACE_PATHS {
                return Ok((out, true));
            }
        }
    }

    Ok((out, truncated))
}

async fn read_searchable_text(path: &Path) -> ApiResult<Option<String>> {
    let bytes = match super::file_write::read_file_bounded(
        path.to_path_buf(),
        MAX_CONTENT_SEARCH_FILE_BYTES,
    )
    .await
    {
        Ok(super::file_write::FileRead::Contents(bytes)) => bytes,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(AppError::forbidden_error(
                "read a file for content search",
                &error,
            ));
        }
        Err(error) => return Err(AppError::internal_error(&error)),
    };
    if bytes.contains(&0) {
        return Ok(None);
    }

    match String::from_utf8(bytes) {
        Ok(content) => Ok(Some(content)),
        Err(error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode a content-search candidate as UTF-8",
                    &error,
                ),
                "content-search candidate is not searchable text"
            );
            Ok(None)
        }
    }
}

fn line_starts(content: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (idx, b) in content.as_bytes().iter().enumerate() {
        if *b == b'\n' && idx + 1 < content.len() {
            starts.push(idx + 1);
        }
    }
    starts
}

fn line_index_for_offset(starts: &[usize], offset: usize) -> usize {
    match starts.binary_search(&offset) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

fn line_bounds(content: &str, starts: &[usize], line_index: usize) -> (usize, usize) {
    let start = starts.get(line_index).copied().unwrap_or(0);
    let mut end = starts
        .get(line_index + 1)
        .copied()
        .unwrap_or(content.len())
        .min(content.len());

    if end > start && content.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end = end.saturating_sub(1);
    }
    if end > start && content.as_bytes().get(end.saturating_sub(1)) == Some(&b'\r') {
        end = end.saturating_sub(1);
    }

    (start, end)
}

fn take_last_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let total = input.chars().count();
    if total <= max_chars {
        return input.to_string();
    }

    let skip = total.saturating_sub(max_chars);
    input.chars().skip(skip).collect()
}

fn take_first_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    input.chars().take(max_chars).collect()
}

fn collect_content_matches(
    content: &str,
    regex: &Regex,
    max_matches: usize,
    context_chars: usize,
) -> (Vec<ContentSearchMatch>, bool) {
    let mut matches = Vec::new();
    let starts = line_starts(content);
    let mut truncated = false;

    for found in regex.find_iter(content) {
        if matches.len() >= max_matches {
            truncated = true;
            break;
        }

        let start_offset = found.start();
        let end_offset = found.end();
        let line_index = line_index_for_offset(&starts, start_offset);
        let (line_start, line_end) = line_bounds(content, &starts, line_index);
        let line_text = &content[line_start..line_end];

        let start_in_line = start_offset.saturating_sub(line_start).min(line_text.len());
        let end_in_line = end_offset.saturating_sub(line_start).min(line_text.len());

        let before_full = &line_text[..start_in_line];
        let matched_full = if end_in_line > start_in_line {
            line_text[start_in_line..end_in_line].to_string()
        } else {
            found
                .as_str()
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        };
        let after_full = if end_in_line < line_text.len() {
            &line_text[end_in_line..]
        } else {
            ""
        };

        let before = take_last_chars(before_full, context_chars);
        let after = take_first_chars(after_full, context_chars);

        let start_column = before_full.chars().count() + 1;
        let end_column = start_column + matched_full.chars().count();

        matches.push(ContentSearchMatch {
            line: line_index + 1,
            start_column,
            end_column,
            start_offset,
            end_offset,
            before,
            matched: matched_full,
            after,
        });
    }

    (matches, truncated)
}

pub async fn fs_content_search(
    headers: HeaderMap,
    Query(q): Query<ProjectDirQuery>,
    Json(body): Json<ContentSearchBody>,
) -> ApiResult<Json<ContentSearchResponse>> {
    let root = resolve_project_directory(&headers, q.directory.as_deref()).await?;

    let query = body
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .ok_or_else(|| AppError::bad_request("Search query is required"))?
        .to_string();

    let max_results = body
        .max_results
        .unwrap_or(DEFAULT_CONTENT_SEARCH_MAX_RESULTS)
        .clamp(1, MAX_CONTENT_SEARCH_MAX_RESULTS);
    let max_matches_per_file = body
        .max_matches_per_file
        .unwrap_or(DEFAULT_CONTENT_SEARCH_MAX_MATCHES_PER_FILE)
        .clamp(1, MAX_CONTENT_SEARCH_MAX_MATCHES_PER_FILE);
    let context_chars = body
        .context_chars
        .unwrap_or(DEFAULT_CONTENT_SEARCH_CONTEXT_CHARS)
        .clamp(0, MAX_CONTENT_SEARCH_CONTEXT_CHARS);

    let regex = build_content_regex(&query, body.is_regex, body.case_sensitive, body.whole_word)?;
    let started = Instant::now();

    let (candidates, discovery_truncated) = if body.paths.is_empty() {
        walk_workspace_files(
            &root,
            body.include_hidden,
            body.respect_gitignore,
            MAX_CONTENT_REPLACE_PATHS,
        )
    } else {
        normalize_content_scope_paths(
            &root,
            &body.paths,
            body.include_hidden,
            body.respect_gitignore,
        )
        .await?
    };

    let mut files = Vec::new();
    let mut total_matches = 0usize;
    let mut truncated = discovery_truncated;

    for path in candidates {
        if total_matches >= max_results {
            truncated = true;
            break;
        }

        let Some(content) = read_searchable_text(&path).await? else {
            continue;
        };

        let remaining = max_results.saturating_sub(total_matches);
        let max_for_file = max_matches_per_file.min(remaining);
        if max_for_file == 0 {
            truncated = true;
            break;
        }

        let (matches, file_truncated) =
            collect_content_matches(&content, &regex, max_for_file, context_chars);
        if matches.is_empty() {
            continue;
        }

        total_matches += matches.len();
        truncated |= file_truncated;

        let relative_path = normalize_relative_search_path(&root, &path);
        files.push(ContentSearchFileResult {
            path: to_api_path(&path),
            relative_path,
            revision: replacement::content_revision(content.as_bytes()),
            match_count: matches.len(),
            matches,
        });

        if total_matches >= max_results {
            truncated = true;
            break;
        }
    }

    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));

    tracing::debug!(
        "fs_content_search root={} q='{}' files={} matches={} truncated={} elapsed_ms={}",
        root.to_string_lossy(),
        query,
        files.len(),
        total_matches,
        truncated,
        started.elapsed().as_millis()
    );

    Ok(Json(ContentSearchResponse {
        root: to_api_path(&root),
        query,
        file_count: files.len(),
        match_count: total_matches,
        files,
        truncated,
    }))
}
