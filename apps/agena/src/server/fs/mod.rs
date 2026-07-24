use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

use crate::{ApiResult, AppError};
use serde::Serialize;

use crate::server::path_utils::{home_dir_env, normalize_directory_path};

mod fs_content;
mod fs_core;
mod fs_search;

pub use fs_content::*;
pub use fs_core::*;
pub use fs_search::*;

const DEFAULT_FILE_SEARCH_LIMIT: usize = 60;
const MAX_FILE_SEARCH_LIMIT: usize = 400;
const MAX_FS_LIST_LIMIT: usize = 2000;

const DEFAULT_CONTENT_SEARCH_MAX_RESULTS: usize = 1200;
const MAX_CONTENT_SEARCH_MAX_RESULTS: usize = 5000;
const DEFAULT_CONTENT_SEARCH_MAX_MATCHES_PER_FILE: usize = 80;
const MAX_CONTENT_SEARCH_MAX_MATCHES_PER_FILE: usize = 300;
const DEFAULT_CONTENT_SEARCH_CONTEXT_CHARS: usize = 48;
const MAX_CONTENT_SEARCH_CONTEXT_CHARS: usize = 160;
const MAX_CONTENT_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CONTENT_REPLACE_PATHS: usize = 4000;
const MAX_FS_CHANGE_EVENT_PATHS: usize = 160;

const FILE_SEARCH_EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "coverage",
    "tmp",
    "logs",
];

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FsChangedEventProperties {
    directory: String,
    change_type: String,
    paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    new_path: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct FsChangedEvent {
    #[serde(rename = "type")]
    event_type: &'static str,
    properties: FsChangedEventProperties,
}

fn encode_fs_changed_event<I>(
    root: &Path,
    change_type: &str,
    changed_paths: I,
    old_path: Option<&Path>,
    new_path: Option<&Path>,
) -> Option<String>
where
    I: IntoIterator,
    I::Item: AsRef<Path>,
{
    let mut seen = HashSet::<String>::new();
    let mut paths = Vec::<String>::new();
    let mut truncated = false;

    for path in changed_paths {
        let normalized = to_api_path(path.as_ref());
        if normalized.trim().is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        if paths.len() >= MAX_FS_CHANGE_EVENT_PATHS {
            truncated = true;
            continue;
        }
        paths.push(normalized);
    }

    if paths.is_empty() {
        return None;
    }

    serde_json::to_string(&FsChangedEvent {
        event_type: "agena:fs-changed",
        properties: FsChangedEventProperties {
            directory: to_api_path(root),
            change_type: change_type.to_string(),
            paths,
            old_path: old_path.map(to_api_path),
            new_path: new_path.map(to_api_path),
            truncated,
        },
    })
    .ok()
}

pub(crate) fn publish_fs_changed_event<I>(
    root: &Path,
    change_type: &str,
    changed_paths: I,
    old_path: Option<&Path>,
    new_path: Option<&Path>,
) where
    I: IntoIterator,
    I::Item: AsRef<Path>,
{
    let _ = encode_fs_changed_event(root, change_type, changed_paths, old_path, new_path);
}

fn has_parent_dir_component(p: &Path) -> bool {
    p.components().any(|c| matches!(c, Component::ParentDir))
}

fn resolve_path(input: &str) -> PathBuf {
    let normalized = normalize_directory_path(input);
    PathBuf::from(normalized)
}

fn is_windows_style_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn normalize_for_workspace_compare(path: &Path) -> String {
    let mut normalized = path
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .replace('\\', "/");

    if normalized.len() > 1 {
        normalized = normalized.trim_end_matches('/').to_string();
    }

    if cfg!(windows) || is_windows_style_path(&normalized) {
        normalized = normalized.to_ascii_lowercase();
    }

    normalized
}

fn is_path_within_base(base: &str, target: &str) -> bool {
    if target == base {
        return true;
    }
    if base == "/" {
        return target.starts_with('/');
    }

    target
        .strip_prefix(base)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

fn to_api_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn ensure_within_base(base: &Path, target: &Path) -> ApiResult<()> {
    let base = normalize_for_workspace_compare(base);
    let target = normalize_for_workspace_compare(target);

    if !is_path_within_base(&base, &target) {
        return Err(AppError::bad_request("Path is outside of active workspace"));
    }
    Ok(())
}
