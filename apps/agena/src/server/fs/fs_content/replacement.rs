use std::{collections::HashMap, path::Path, sync::Arc};

use axum::{
    Json,
    body::Bytes,
    extract::Query,
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use regex::Regex;
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use super::{
    ApiResult, AppError, ContentReplaceBody, ContentReplaceFileResult, ContentReplaceResponse,
    MAX_CONTENT_REPLACE_PATHS, MAX_CONTENT_SEARCH_FILE_BYTES, ProjectDirQuery, build_content_regex,
    normalize_content_scope_paths, normalize_relative_search_path, resolve_path_within_workspace,
    resolve_project_directory, to_api_path, walk_workspace_files,
};
use crate::server::fs::{MAX_UPLOAD_BYTES, file_write, normalize_for_workspace_compare};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PartialReplacement {
    completed: ContentReplaceResponse,
    failed_path: String,
    remaining: usize,
}

#[derive(Debug)]
pub struct ReplacementError {
    source: AppError,
    partial: Option<PartialReplacement>,
}

impl From<AppError> for ReplacementError {
    fn from(source: AppError) -> Self {
        Self {
            source,
            partial: None,
        }
    }
}

impl IntoResponse for ReplacementError {
    fn into_response(self) -> Response {
        match self.partial {
            Some(details) => self.source.into_response_with_details(details),
            None => self.source.into_response(),
        }
    }
}

pub(super) fn content_revision(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut revision = String::with_capacity(64);
    for &byte in &digest {
        revision.push(HEX[(byte >> 4) as usize] as char);
        revision.push(HEX[(byte & 15) as usize] as char);
    }
    revision
}

fn validate_revision(revision: &str) -> ApiResult<()> {
    if revision.len() != 64 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::bad_request(
            "Expected revision must be a SHA-256 digest",
        ));
    }
    Ok(())
}

fn check_revision(bytes: Option<&[u8]>, expected: Option<&str>) -> ApiResult<()> {
    if let Some(expected) = expected
        && !bytes.is_some_and(|bytes| content_revision(bytes).eq_ignore_ascii_case(expected))
    {
        return Err(AppError::conflict(
            "File changed since the search; run search again before replacing",
        ));
    }
    Ok(())
}

fn searchable_text(bytes: Option<&[u8]>) -> Option<&str> {
    let bytes = bytes?;
    if bytes.contains(&0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

/// Compile the library's replacement syntax once, without expanding any file
/// captures. A capture is an index at an offset in the unexpanded literal text.
struct ReplacementPlan {
    regex: Regex,
    literal: String,
    references: Vec<(usize, usize)>,
}

impl ReplacementPlan {
    fn new(regex: Regex, replacement: String, is_regex: bool) -> Self {
        if !is_regex {
            return Self {
                regex,
                literal: replacement,
                references: Vec::new(),
            };
        }
        let names: HashMap<_, _> = regex
            .capture_names()
            .enumerate()
            .filter_map(|(index, name)| name.map(|name| (name, index)))
            .collect();
        let mut literal = String::new();
        let mut references = Vec::new();
        regex_automata::util::interpolate::string(
            &replacement,
            |index, output| references.push((output.len(), index)),
            |name| names.get(name).copied(),
            &mut literal,
        );
        Self {
            regex,
            literal,
            references,
        }
    }

    fn apply(&self, content: &str) -> ApiResult<(String, usize)> {
        let mut output = BoundedOutput::default();
        let mut cursor = 0;
        let mut replacements = 0;
        if self.references.is_empty() {
            for matched in self.regex.find_iter(content) {
                output.append(&content[cursor..matched.start()])?;
                output.append(&self.literal)?;
                cursor = matched.end();
                replacements += 1;
            }
        } else {
            for captures in self.regex.captures_iter(content) {
                let matched = captures
                    .get(0)
                    .expect("a successful capture has a full match");
                output.append(&content[cursor..matched.start()])?;
                let mut literal_start = 0;
                for &(offset, index) in &self.references {
                    output.append(&self.literal[literal_start..offset])?;
                    if let Some(capture) = captures.get(index) {
                        output.append(capture.as_str())?;
                    }
                    literal_start = offset;
                }
                output.append(&self.literal[literal_start..])?;
                cursor = matched.end();
                replacements += 1;
            }
        }
        output.append(&content[cursor..])?;
        Ok((output.0, replacements))
    }
}

#[derive(Default)]
struct BoundedOutput(String);

impl BoundedOutput {
    fn append(&mut self, text: &str) -> ApiResult<()> {
        let needed = self
            .0
            .len()
            .checked_add(text.len())
            .filter(|length| *length <= MAX_UPLOAD_BYTES)
            .ok_or_else(|| {
                AppError::payload_too_large("Replacement would exceed the 50 MiB file limit")
            })?;
        if needed > self.0.capacity() {
            let capacity = needed
                .max(self.0.capacity().saturating_mul(2))
                .min(MAX_UPLOAD_BYTES);
            self.0.reserve_exact(capacity - self.0.len());
        }
        self.0.push_str(text);
        Ok(())
    }
}

enum FileOutcome {
    Skipped,
    Unchanged,
    Replaced(usize),
}

fn completed_response(
    root: &Path,
    mut files: Vec<ContentReplaceFileResult>,
    replacement_count: usize,
    skipped: usize,
    truncated: bool,
) -> ContentReplaceResponse {
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    ContentReplaceResponse {
        root: to_api_path(root),
        file_count: files.len(),
        replacement_count,
        skipped,
        truncated,
        files,
    }
}

pub async fn fs_content_replace(
    headers: HeaderMap,
    Query(query): Query<ProjectDirQuery>,
    Json(body): Json<ContentReplaceBody>,
) -> Result<Json<ContentReplaceResponse>, ReplacementError> {
    let root = resolve_project_directory(&headers, query.directory.as_deref()).await?;
    let replacement = body
        .replace
        .ok_or_else(|| AppError::bad_request("Replace text is required"))?;
    if let Some(target) = body.r#match {
        let path = target
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or_else(|| AppError::bad_request("Match path is required"))?;
        let path = resolve_path_within_workspace(&root, path)?;
        let expected = target
            .expected
            .ok_or_else(|| AppError::bad_request("Match expected text is required"))?;
        let start = target
            .start_offset
            .ok_or_else(|| AppError::bad_request("Match startOffset is required"))?;
        let end = target
            .end_offset
            .ok_or_else(|| AppError::bad_request("Match endOffset is required"))?;
        if end <= start {
            return Err(AppError::bad_request("Invalid match range").into());
        }
        if let Some(revision) = &target.expected_revision {
            validate_revision(revision)?;
        }
        let changed = file_write::transform_file_atomically(
            path.clone(),
            MAX_CONTENT_SEARCH_FILE_BYTES,
            move |bytes| {
                check_revision(bytes, target.expected_revision.as_deref())?;
                let content = searchable_text(bytes).ok_or_else(|| {
                    AppError::bad_request("Target file is not a searchable text file")
                })?;
                let current = content.get(start..end).ok_or_else(|| {
                    AppError::conflict("Match range is no longer valid; run search again")
                })?;
                if current != expected {
                    return Err(AppError::conflict(
                        "Selected match changed; run search again before replacing",
                    ));
                }
                if current == replacement {
                    return Ok((None, false));
                }
                let mut updated = BoundedOutput::default();
                updated.append(&content[..start])?;
                updated.append(&replacement)?;
                updated.append(&content[end..])?;
                Ok((Some(Bytes::from(updated.0)), true))
            },
        )
        .await?;
        let files = if changed {
            vec![ContentReplaceFileResult {
                path: to_api_path(&path),
                relative_path: normalize_relative_search_path(&root, &path),
                replacements: 1,
            }]
        } else {
            Vec::new()
        };
        return Ok(Json(completed_response(
            &root,
            files,
            usize::from(changed),
            0,
            false,
        )));
    }

    let query = body
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| AppError::bad_request("Search query is required"))?;
    let regex = build_content_regex(query, body.is_regex, body.case_sensitive, body.whole_word)?;
    let plan = Arc::new(ReplacementPlan::new(regex, replacement, body.is_regex));
    let (candidates, truncated) = if body.paths.is_empty() {
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
    let expected_revisions = body
        .expected_revisions
        .map(|revisions| {
            let mut expected = HashMap::new();
            for (path, revision) in revisions {
                validate_revision(&revision)?;
                let path = resolve_path_within_workspace(&root, &path)?;
                let key = normalize_for_workspace_compare(&path);
                if expected
                    .insert(key, revision.to_ascii_lowercase())
                    .is_some()
                {
                    return Err(AppError::bad_request("Duplicate expected revision path"));
                }
            }
            if expected.len() != candidates.len()
                || candidates
                    .iter()
                    .any(|path| !expected.contains_key(&normalize_for_workspace_compare(path)))
            {
                return Err(AppError::conflict(
                    "Replacement scope changed since the search; run search again",
                ));
            }
            Ok(expected)
        })
        .transpose()?;

    let mut files = Vec::new();
    let mut replacements = 0;
    let mut skipped = 0;
    let total_files = candidates.len();
    for (index, path) in candidates.into_iter().enumerate() {
        let expected = expected_revisions
            .as_ref()
            .and_then(|revisions| revisions.get(&normalize_for_workspace_compare(&path)))
            .cloned();
        let plan = plan.clone();
        let outcome = file_write::transform_file_atomically(
            path.clone(),
            MAX_CONTENT_SEARCH_FILE_BYTES,
            move |bytes| {
                check_revision(bytes, expected.as_deref())?;
                let Some(content) = searchable_text(bytes) else {
                    return Ok((None, FileOutcome::Skipped));
                };
                let (updated, count) = plan.apply(content)?;
                if updated == content {
                    return Ok((None, FileOutcome::Unchanged));
                }
                Ok((Some(Bytes::from(updated)), FileOutcome::Replaced(count)))
            },
        )
        .await;
        match outcome {
            Ok(FileOutcome::Skipped) => skipped += 1,
            Ok(FileOutcome::Unchanged) => {}
            Ok(FileOutcome::Replaced(count)) => {
                replacements += count;
                files.push(ContentReplaceFileResult {
                    path: to_api_path(&path),
                    relative_path: normalize_relative_search_path(&root, &path),
                    replacements: count,
                });
            }
            Err(source) => {
                tracing::warn!(
                    completed_files = files.len(),
                    completed_replacements = replacements,
                    remaining_files = total_files - index - 1,
                    diagnostic = %source,
                    "content replacement stopped; completed files remain published"
                );
                return Err(ReplacementError {
                    source,
                    partial: Some(PartialReplacement {
                        completed: completed_response(
                            &root,
                            files,
                            replacements,
                            skipped,
                            truncated,
                        ),
                        failed_path: to_api_path(&path),
                        remaining: total_files - index - 1,
                    }),
                });
            }
        }
    }
    Ok(Json(completed_response(
        &root,
        files,
        replacements,
        skipped,
        truncated,
    )))
}

#[cfg(test)]
mod tests;
