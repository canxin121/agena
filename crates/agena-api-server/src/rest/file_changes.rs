//! Complete durable membership, not the visible/paged transcript or Git tree.
use crate::{error::ServerError, state::AppState};
use agena_api::resource::{
    SessionFileChangeResource, SessionFileChangesResource, SessionFileEditResource,
};
use agena_domain::ToolResultState;
use agena_runtime_contracts::part_content::ToolCallContent;
use agena_storage::store::{Part, PartState};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
};
use lru::LruCache;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::ops::Deref;
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Deserialize)]
pub struct FileChangesQuery {
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
    #[serde(default)]
    pub summary: bool,
    pub path: Option<String>,
    pub max_bytes: Option<usize>,
}

pub async fn get_session_file_changes(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<FileChangesQuery>,
    headers: HeaderMap,
) -> Result<Response, ServerError> {
    let resource = format!("session:{id}:files");
    let read = crate::revisions::ConditionalRead::new(&state, &resource).await?;
    if let Some(response) = read.not_modified(&headers) {
        return Ok(response);
    }
    if let Some(facts) = state.file_changes_cache().get(id, read.token()) {
        let projection = FILE_PROJECTIONS
            .run(move || project_facts(&facts, &query))
            .await
            .map_err(|error| ServerError::internal_error(&error))?;
        return read.json(projection).await;
    }
    let session = state
        .session_store()?
        .load_owned_file_change_parts(id)
        .await
        .map_err(super::server_error_from_store)?;
    let revisions = state.revisions()?.clone();
    let token = read.token().to_owned();
    let projection = FILE_PROJECTIONS
        .run(move || {
            revisions.register_file_facts(id, &session.parts);
            let facts = Arc::new(collect_file_facts(id, &session.parts));
            // A concurrent mutation must not attach its revision to an older read.
            if revisions.token(&resource) == token {
                state.file_changes_cache().insert(id, &token, facts.clone());
            }
            project_facts(&facts, &query)
        })
        .await
        .map_err(|error| ServerError::internal_error(&error))?;
    read.json(projection).await
}

static FILE_PROJECTIONS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

const MAX_FILE_FACT_CACHE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) struct FileChangesProjectionCache {
    entries: Mutex<(
        LruCache<i64, (String, Arc<RecordedFileFacts>, usize)>,
        usize,
    )>,
    max_bytes: usize,
}

impl Default for FileChangesProjectionCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new((LruCache::new(NonZeroUsize::new(32).unwrap()), 0)),
            max_bytes: MAX_FILE_FACT_CACHE_BYTES,
        }
    }
}

impl FileChangesProjectionCache {
    fn get(&self, id: i64, token: &str) -> Option<Arc<RecordedFileFacts>> {
        let mut cache = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (cached, facts, _) = cache.0.get(&id)?;
        (cached == token).then(|| facts.clone())
    }

    fn insert(&self, id: i64, token: &str, facts: Arc<RecordedFileFacts>) {
        let bytes = facts.retained_bytes();
        let mut retired = Vec::new();
        let mut cache = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((_, facts, previous)) = cache.0.pop(&id) {
            cache.1 -= previous;
            retired.push(facts);
        }
        if bytes > self.max_bytes {
            drop(cache);
            drop(retired);
            return;
        }
        while cache.1.saturating_add(bytes) > self.max_bytes {
            let Some((_, (_, facts, previous))) = cache.0.pop_lru() else {
                break;
            };
            cache.1 -= previous;
            retired.push(facts);
        }
        if let Some((_, (_, facts, previous))) = cache.0.push(id, (token.to_owned(), facts, bytes))
        {
            cache.1 -= previous;
            retired.push(facts);
        }
        cache.1 += bytes;
        drop(cache);
        drop(retired);
    }
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn has_delta(diff: &str) -> bool {
    let mut in_hunk = false;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
        }
        if line.starts_with("@@") {
            in_hunk = true;
            continue;
        }
        if in_hunk && (line.starts_with('+') || line.starts_with('-')) {
            return true;
        }
    }
    false
}
// Match the exact header emitted by apply_patch; don't parse paths on spaces.
fn patch_file_diff<'a>(diff: &'a str, path: &str, from: Option<&str>) -> Option<&'a str> {
    let header = format!("diff --git a/{} b/{path}\n", from.unwrap_or(path));
    let start = diff
        .match_indices(&header)
        .find(|(start, _)| *start == 0 || diff.as_bytes()[start - 1] == b'\n')?
        .0;
    let rest = &diff[start..];
    let end = rest[header.len()..]
        .find("\ndiff --git ")
        .map_or(rest.len(), |i| header.len() + i + 1);
    Some(&rest[..end])
}

fn edit(
    part: &Part,
    tool: &ToolCallContent,
    payload: &Value,
    change: &Value,
    diff: &Option<(Arc<str>, u64)>,
) -> RecordedFileEdit {
    RecordedFileEdit {
        diff: diff.clone(),
        resource: SessionFileEditResource {
            part_id: part.part_id,
            tool: tool.name.clone(),
            kind: string(change, "kind").unwrap_or_else(|| "updated".into()),
            from_path: string(change, "from_path"),
            before_sha256: string(payload, "before_sha256")
                .or_else(|| {
                    (tool.name == "fs.write")
                        .then(|| string(&tool.input, "expected_sha256"))
                        .flatten()
                })
                .map(|sha| sha.to_ascii_lowercase()),
            after_sha256: string(payload, "after_sha256")
                .or_else(|| string(payload, "sha256"))
                .map(|sha| sha.to_ascii_lowercase()),
            diff: None,
            diff_truncated: payload
                .get("diff_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || tool.output.as_ref().is_some_and(|o| o.truncated),
            diff_scope: if tool.name == "fs.apply_patch" {
                "operation"
            } else {
                "file"
            }
            .into(),
            diff_unavailable_reason: string(payload, "diff_unavailable_reason"),
        },
    }
}

#[derive(Clone)]
struct RecordedFileEdit {
    resource: SessionFileEditResource,
    diff: Option<(Arc<str>, u64)>,
}
impl Deref for RecordedFileEdit {
    type Target = SessionFileEditResource;
    fn deref(&self) -> &Self::Target {
        &self.resource
    }
}
impl Hash for RecordedFileEdit {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.resource.hash(state);
        self.diff.as_ref().map(|(_, hash)| hash).hash(state);
    }
}
#[derive(Default, Hash)]
struct RecordedFileFacts {
    rows: BTreeMap<String, Vec<RecordedFileEdit>>,
    incomplete: bool,
}

impl RecordedFileFacts {
    fn retained_bytes(&self) -> usize {
        let mut bytes = std::mem::size_of::<Self>();
        let mut diffs = HashSet::new();
        for (path, edits) in &self.rows {
            bytes = bytes.saturating_add(path.len() + 64);
            for edit in edits {
                bytes = bytes.saturating_add(
                    std::mem::size_of::<RecordedFileEdit>()
                        + edit.tool.len()
                        + edit.kind.len()
                        + edit.diff_scope.len(),
                );
                for value in [
                    &edit.from_path,
                    &edit.before_sha256,
                    &edit.after_sha256,
                    &edit.diff_unavailable_reason,
                ]
                .into_iter()
                .flatten()
                {
                    bytes = bytes.saturating_add(value.len());
                }
                if let Some((diff, _)) = &edit.diff
                    && diffs.insert(Arc::as_ptr(diff) as *const ())
                {
                    bytes = bytes.saturating_add(diff.len());
                }
            }
        }
        bytes
    }
}

// The API projection and mutation clock use exactly the same edit evidence.
// Output previews, execution timestamps and presentation metadata cannot
// change the files representation. Non-edit tools do not decode their output.
fn recorded_file_facts(part: &Part) -> RecordedFileFacts {
    let mut rows = BTreeMap::<String, Vec<RecordedFileEdit>>::new();
    let mut incomplete = false;
    'record: {
        if part.kind != "tool_call" {
            break 'record;
        }
        let name = part
            .content
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("");
        if name.starts_with("shell.") {
            // A queued command has not executed yet. Once it can have run,
            // writes remain a declaration, even when the command fails.
            incomplete = part.state != PartState::Pending
                && part
                    .content
                    .pointer("/input/writes")
                    .and_then(Value::as_array)
                    .is_some_and(|writes| !writes.is_empty());
            break 'record;
        }
        if !matches!(
            name,
            "fs.write" | "fs.replace" | "code.rewrite_ast" | "fs.apply_patch"
        ) {
            break 'record;
        }
        let Ok(tool) = ToolCallContent::try_from(&part.content) else {
            break 'record;
        };
        if part.state != PartState::Completed
            || tool.state != ToolResultState::Completed
            || tool.error.is_some()
        {
            break 'record;
        }
        // Preview is never an edit, even if an older/malformed result lacks
        // a payload. The invocation's apply flag is also required as evidence.
        if tool.name == "code.rewrite_ast" && tool.input.get("apply") != Some(&Value::Bool(true)) {
            break 'record;
        }
        let Some(payload) = tool.output.as_ref().and_then(|o| o.payload.as_ref()) else {
            incomplete = true;
            break 'record;
        };
        if payload.get("changed") == Some(&Value::Bool(false))
            || payload.get("applied") == Some(&Value::Bool(false))
        {
            break 'record;
        }
        if tool.name == "code.rewrite_ast"
            && (tool.input.get("apply") != Some(&Value::Bool(true))
                || payload.get("applied") != Some(&Value::Bool(true)))
        {
            break 'record;
        }
        let diff = payload.get("diff").and_then(Value::as_str).map(|value| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut hasher);
            (Arc::<str>::from(value), hasher.finish())
        });
        if tool.name == "fs.apply_patch" {
            let Some(changes) = payload.get("changes").and_then(Value::as_array) else {
                incomplete = true;
                break 'record;
            };
            for change in changes {
                let Some(path) = string(change, "path") else {
                    incomplete = true;
                    continue;
                };
                let kind = string(change, "kind");
                if !matches!(
                    kind.as_deref(),
                    Some("added" | "updated" | "deleted" | "moved")
                ) {
                    incomplete = true;
                    continue;
                }
                if kind.as_deref() == Some("updated") {
                    let from = string(change, "from_path");
                    let section = payload
                        .get("diff")
                        .and_then(Value::as_str)
                        .and_then(|d| patch_file_diff(d, &path, from.as_deref()));
                    match section {
                        Some(d) if !has_delta(d) => continue,
                        Some(_) => {}
                        None => {
                            incomplete = true;
                            continue;
                        }
                    }
                }
                rows.entry(path)
                    .or_default()
                    .push(edit(part, &tool, payload, change, &diff));
            }
        } else {
            let Some(path) = string(payload, "path") else {
                incomplete = true;
                break 'record;
            };
            if tool.name == "fs.write"
                && (!matches!(
                    string(payload, "kind").as_deref(),
                    Some("created" | "updated")
                ) || string(payload, "sha256").is_none())
            {
                incomplete = true;
                break 'record;
            }
            if tool.name == "fs.replace"
                && (string(payload, "before_sha256").is_none()
                    || string(payload, "after_sha256").is_none())
            {
                incomplete = true;
                break 'record;
            }
            let record = edit(part, &tool, payload, payload, &diff);
            if record.before_sha256.is_some() && record.before_sha256 == record.after_sha256 {
                break 'record;
            }
            if record.kind != "created"
                && record
                    .diff
                    .as_ref()
                    .is_some_and(|(diff, _)| diff.is_empty())
            {
                break 'record;
            }
            rows.entry(path).or_default().push(record);
        }
    }
    RecordedFileFacts { rows, incomplete }
}

pub(crate) fn recorded_file_revision(part: &Part) -> Option<u64> {
    let facts = recorded_file_facts(part);
    if facts.rows.is_empty() && !facts.incomplete {
        return None;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    facts.hash(&mut hasher);
    Some(hasher.finish())
}

pub(crate) fn recorded_file_source(part: &Part) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if part.kind != "tool_call" {
        return 0;
    }
    let name = part
        .content
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("");
    if name.starts_with("shell.") {
        // Running output, error messages and terminal status cannot change
        // this declaration's contribution after the command started.
        (part.state != PartState::Pending
            && part
                .content
                .pointer("/input/writes")
                .and_then(Value::as_array)
                .is_some_and(|writes| !writes.is_empty()))
        .hash(&mut hasher);
    } else if matches!(
        name,
        "fs.write" | "fs.replace" | "fs.apply_patch" | "code.rewrite_ast"
    ) {
        (
            part.state,
            name,
            part.content.get("state"),
            part.content.get("error"),
            part.content.pointer("/input/apply"),
            part.content.pointer("/input/expected_sha256"),
            part.content.pointer("/output/payload"),
            part.content.pointer("/output/truncated"),
        )
            .hash(&mut hasher);
    } else {
        return 0;
    }
    hasher.finish()
}

fn collect_file_facts(id: i64, parts: &[Part]) -> RecordedFileFacts {
    let mut rows: BTreeMap<String, Vec<RecordedFileEdit>> = BTreeMap::new();
    let mut incomplete = false;
    let mut ordered: Vec<_> = parts.iter().filter(|p| p.origin_session_id == id).collect();
    ordered.sort_by_key(|p| (p.created_at_ms, p.part_id));
    for part in ordered {
        let facts = recorded_file_facts(part);
        incomplete |= facts.incomplete;
        for (path, edits) in facts.rows {
            rows.entry(path).or_default().extend(edits);
        }
    }
    // Only eliminate a verified, contiguous SHA chain that returns to its
    // initial revision. Missing baselines and renames cannot prove net zero.
    rows.retain(|_, ops| {
        !(ops.iter().all(|o| {
            o.from_path.is_none() && o.before_sha256.is_some() && o.after_sha256.is_some()
        }) && ops
            .windows(2)
            .all(|w| w[0].after_sha256 == w[1].before_sha256)
            && ops.first().and_then(|o| o.before_sha256.as_ref())
                == ops.last().and_then(|o| o.after_sha256.as_ref()))
    });
    RecordedFileFacts { rows, incomplete }
}

#[cfg(test)]
fn project(id: i64, parts: &[Part], query: &FileChangesQuery) -> SessionFileChangesResource {
    project_facts(&collect_file_facts(id, parts), query)
}

fn project_facts(
    facts: &RecordedFileFacts,
    query: &FileChangesQuery,
) -> SessionFileChangesResource {
    let total_files = facts.rows.len();
    let offset = query.offset;
    let limit = query.limit.unwrap_or(40).clamp(1, 100);
    let mut budget = query
        .max_bytes
        .unwrap_or(256 * 1024)
        .clamp(1, 2 * 1024 * 1024);
    let files = if query.summary {
        Vec::new()
    } else {
        facts
            .rows
            .iter()
            .filter(|(path, _)| {
                query
                    .path
                    .as_ref()
                    .is_none_or(|wanted| wanted.as_str() == path.as_str())
            })
            .skip(if query.path.is_some() { 0 } else { offset })
            .take(limit)
            .map(|(path, operations)| {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                operations.hash(&mut hasher);
                let revision = format!("{:016x}", hasher.finish());
                let operation_count = operations.len();
                let operations = if query.path.is_none() {
                    Vec::new()
                } else {
                    operations
                        .iter()
                        .map(|edit| {
                            let mut resource = edit.resource.clone();
                            if let Some((diff, _)) = &edit.diff {
                                let mut end = budget.min(diff.len());
                                while !diff.is_char_boundary(end) {
                                    end -= 1;
                                }
                                resource.diff_truncated |= end < diff.len();
                                resource.diff = Some(diff[..end].to_owned());
                                budget = budget.saturating_sub(end);
                            }
                            resource
                        })
                        .collect()
                };
                SessionFileChangeResource {
                    path: path.clone(),
                    revision,
                    operation_count,
                    operation_history: operation_count > 1,
                    operations,
                }
            })
            .collect::<Vec<_>>()
    };
    let has_more =
        !query.summary && query.path.is_none() && offset.saturating_add(files.len()) < total_files;
    SessionFileChangesResource {
        files,
        total_files,
        offset,
        has_more,
        recording_incomplete: facts.incomplete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::RawOutput;
    use agena_storage::store::{PartRole, PartVisibility};
    use serde_json::json;

    fn part(id: i64, name: &str, payload: Value) -> Part {
        let tool = ToolCallContent {
            name: name.into(),
            state: ToolResultState::Completed,
            input: json!({"apply":true}),
            output: Some(RawOutput {
                payload: Some(payload),
                ..Default::default()
            }),
            ..Default::default()
        };
        Part {
            part_id: id,
            kind: "tool_call".into(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: tool.as_value(),
            summary: None,
            visibility: PartVisibility::Both,
            parent_part_id: None,
            run_id: Some(10),
            origin_session_id: 1,
            revision: 1,
            started_at_ms: id,
            finished_at_ms: Some(id),
            created_at_ms: id,
            updated_at_ms: id,
            provider_state: None,
        }
    }
    fn replace(id: i64, before: &str, after: &str) -> Part {
        part(
            id,
            "fs.replace",
            json!({"path":"a.txt", "before_sha256":before,"after_sha256":after,"diff":"-old\n+new\n"}),
        )
    }
    fn detail() -> FileChangesQuery {
        FileChangesQuery {
            path: Some("a.txt".into()),
            ..Default::default()
        }
    }

    #[test]
    fn file_fact_cache_reuses_only_the_observed_revision_and_bounds_sessions_and_bytes() {
        let facts = Arc::new(collect_file_facts(1, &[replace(1, "a", "b")]));
        let bytes = facts.retained_bytes();
        let cache = FileChangesProjectionCache {
            max_bytes: bytes * 2,
            ..Default::default()
        };
        cache.insert(1, "first", facts.clone());
        assert!(Arc::ptr_eq(&facts, &cache.get(1, "first").unwrap()));
        assert!(cache.get(1, "different").is_none());
        cache.insert(1, "second", facts.clone());
        assert!(cache.get(1, "first").is_none());
        assert!(cache.get(1, "second").is_some());
        cache.insert(2, "second", facts.clone());
        cache.insert(3, "second", facts.clone());
        assert!(cache.get(1, "second").is_none());
        assert!(cache.entries.lock().unwrap().1 <= bytes * 2);
        let oversized = FileChangesProjectionCache {
            max_bytes: bytes - 1,
            ..Default::default()
        };
        oversized.insert(1, "large", facts.clone());
        assert!(oversized.get(1, "large").is_none());
        let count = FileChangesProjectionCache::default();
        for id in 0..40 {
            count.insert(id, "same", facts.clone());
        }
        assert_eq!(count.entries.lock().unwrap().0.len(), 32);
        assert!(count.get(0, "same").is_none());
    }

    #[test]
    fn cached_file_projection_preserves_full_facts_across_summary_and_truncated_detail_reads() {
        let facts = collect_file_facts(1, &[replace(1, "a", "b"), replace(2, "b", "c")]);
        let summary = project_facts(
            &facts,
            &FileChangesQuery {
                summary: true,
                ..Default::default()
            },
        );
        assert_eq!(summary.total_files, 1);
        assert!(summary.files.is_empty());
        let short = project_facts(
            &facts,
            &FileChangesQuery {
                max_bytes: Some(3),
                ..detail()
            },
        );
        assert!(short.files[0].operations[0].diff_truncated);
        let complete = project_facts(&facts, &detail());
        assert_eq!(complete.files[0].operations.len(), 2);
        assert_eq!(
            complete.files[0].operations[0].diff.as_deref(),
            Some("-old\n+new\n")
        );
        assert!(!complete.files[0].operations[0].diff_truncated);
        assert_eq!(complete.files[0].revision, short.files[0].revision);
    }

    #[test]
    fn only_own_completed_applied_edits_and_raw_output_are_evidence() {
        let good = replace(1, "a", "b");
        let mut inherited = replace(2, "b", "c");
        inherited.origin_session_id = 2;
        let mut failed = replace(3, "b", "c");
        failed.state = PartState::Failed;
        let mut pending = replace(4, "b", "c");
        pending.content["state"] = json!("running");
        let noop = replace(5, "a", "a");
        let mut preview = part(
            6,
            "code.rewrite_ast",
            json!({"path":"a.txt","applied":false,"changed":true,"diff":"+preview"}),
        );
        preview.content["input"]["apply"] = json!(false);
        let mut presentation_only = replace(7, "b", "c");
        presentation_only.content["output"] = json!({"payload":{"blocks":[{"diff":"+fake"}]}});
        let result = project(
            1,
            &[
                good,
                inherited,
                failed,
                pending,
                noop,
                preview,
                presentation_only,
            ],
            &detail(),
        );
        assert_eq!(result.total_files, 1);
        assert_eq!(result.files[0].operation_count, 1);
        assert_eq!(
            result.files[0].operations[0].diff.as_deref(),
            Some("-old\n+new\n")
        );
        assert!(result.recording_incomplete);
    }

    #[test]
    fn only_verified_contiguous_sha_round_trip_disappears() {
        assert_eq!(
            project(1, &[replace(1, "a", "b"), replace(2, "b", "a")], &detail()).total_files,
            0
        );
        let disconnected = project(1, &[replace(1, "a", "b"), replace(2, "c", "a")], &detail());
        assert!(disconnected.files[0].operation_history);
        let write = part(
            3,
            "fs.write",
            json!({"path":"a.txt","kind":"updated","sha256":"a","diff":"+x"}),
        );
        let unknown = project(1, &[replace(1, "a", "b"), write], &detail());
        assert_eq!(unknown.total_files, 1);
        assert_eq!(unknown.files[0].operations.len(), 2);
        assert!(unknown.files[0].operation_history);
    }

    #[test]
    fn per_file_fingerprint_ignores_changes_to_other_files_and_diff_truncation() {
        let a = replace(1, "a", "b");
        let b = part(
            2,
            "fs.write",
            json!({"path":"b.txt","kind":"created","sha256":"c","diff":"+B"}),
        );
        let first = project(1, &[a.clone()], &detail());
        let second = project(
            1,
            &[a.clone(), b],
            &FileChangesQuery {
                max_bytes: Some(1),
                ..detail()
            },
        );
        assert_eq!(first.files[0].revision, second.files[0].revision);
        let changed = project(1, &[a, replace(3, "b", "c")], &detail());
        assert_ne!(first.files[0].revision, changed.files[0].revision);
    }

    #[test]
    fn patch_rename_and_noop_are_explicit_and_diff_budget_is_utf8_safe() {
        let diff = "diff --git a/noop b/noop\ndiff --git a/old name b/a.txt\nrename from old name\nrename to a.txt\n-旧\n+新\n";
        let patch = part(
            1,
            "fs.apply_patch",
            json!({"changes":[{"path":"noop","kind":"updated"},{"path":"a.txt","kind":"moved","from_path":"old name"}],"diff":diff}),
        );
        let result = project(
            1,
            &[patch],
            &FileChangesQuery {
                max_bytes: Some(diff.len() - 2),
                ..detail()
            },
        );
        assert_eq!(result.total_files, 1);
        let op = &result.files[0].operations[0];
        assert_eq!(op.from_path.as_deref(), Some("old name"));
        assert_eq!(op.diff_scope, "operation");
        assert!(op.diff_truncated);
        assert!(op.diff.as_ref().unwrap().len() <= diff.len() - 2);
    }

    #[test]
    fn ast_apply_and_write_revisions_distinguish_writes_from_preview_and_noop() {
        let applied = part(
            1,
            "code.rewrite_ast",
            json!({"path":"a.txt", "applied":true,"changed":true,"before_sha256":"a","after_sha256":"b","diff":"+new", "diff_truncated":true}),
        );
        let mut wrong_input = applied.clone();
        wrong_input.part_id = 2;
        wrong_input.content["input"]["apply"] = json!(false);
        let mut write = part(
            3,
            "fs.write",
            json!({"path":"a.txt","kind":"updated","sha256":"b","diff":"+new"}),
        );
        write.content["input"]["expected_sha256"] = json!("b");
        let created_empty = part(
            4,
            "fs.write",
            json!({"path":"empty.txt","kind":"created","sha256":"empty","diff":""}),
        );
        let result = project(1, &[applied, wrong_input, write, created_empty], &detail());
        assert_eq!(result.total_files, 2);
        assert_eq!(result.files[0].operation_count, 1);
        assert!(result.files[0].operations[0].diff_truncated);
        assert_eq!(result.files[0].operations[0].tool, "code.rewrite_ast");
    }

    #[test]
    fn shell_declarations_never_become_edits_and_summary_pages_are_lazy() {
        let mut shell = part(100, "shell.run", json!({"exit_code":0}));
        shell.content["input"] = json!({"writes":["declared.txt"]});
        let mut parts = (0..83)
            .map(|n| {
                part(
                    n,
                    "fs.write",
                    json!({"path":format!("{n:03}.txt"),"kind":"created","sha256":"x","diff":"+x"}),
                )
            })
            .collect::<Vec<_>>();
        parts.push(shell);
        let summary = project(
            1,
            &parts,
            &FileChangesQuery {
                summary: true,
                ..Default::default()
            },
        );
        assert_eq!(summary.total_files, 83);
        assert!(summary.files.is_empty());
        assert!(summary.recording_incomplete);
        let page = project(
            1,
            &parts,
            &FileChangesQuery {
                offset: 40,
                ..Default::default()
            },
        );
        assert_eq!(page.files.len(), 40);
        assert!(page.has_more);
        assert_eq!(page.files[0].path, "040.txt");
        assert!(page.files[0].operations.is_empty());
        let last = project(
            1,
            &parts,
            &FileChangesQuery {
                offset: 80,
                ..Default::default()
            },
        );
        assert_eq!(last.files.len(), 3);
        assert!(!last.has_more);
    }
}
