//! Complete durable membership, not the visible/paged transcript or Git tree.
use crate::{error::ServerError, state::AppState};
use agena_api::part::PartResource as Part;
use agena_api::resource::{
    SessionFileChangeResource, SessionFileChangesResource, SessionFileEditResource,
};
use agena_domain::ToolResultState;
use agena_runtime_contracts::part_content::ToolCallContent;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
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
    // Summary, page and diff requests share one bounded history build. Capture
    // its resource version after waiting, never relabel an older body current.
    let gate = state.file_changes_cache().gate(id);
    let guard = gate.lock().await;
    for attempt in 0..3 {
        let read = crate::revisions::ConditionalRead::new(&state, &resource).await?;
        let facts = if let Some(facts) = state.file_changes_cache().get(id, read.token()) {
            facts
        } else {
            let facts = match read_file_facts(&state, id).await {
                Ok(facts) => Arc::new(facts),
                Err(error) if attempt < 2 && error.status() == StatusCode::CONFLICT => {
                    continue;
                }
                Err(error) => return Err(error),
            };
            let revisions = state.revisions()?;
            revisions.refresh_durable(&state).await?;
            if revisions.token(&resource) == read.token() {
                state
                    .file_changes_cache()
                    .insert(id, read.token(), facts.clone());
            }
            facts
        };
        drop(guard);
        let projection = FILE_PROJECTIONS
            .run(move || project_facts(&facts, &query))
            .await
            .map_err(|error| ServerError::internal_error(&error))?;
        let revisions = state.revisions()?;
        revisions.refresh_durable(&state).await?;
        return file_changes_response(read, projection, &headers, revisions.token(&resource)).await;
    }
    unreachable!("the final history attempt returns its result")
}

async fn read_file_facts(state: &AppState, id: i64) -> Result<RecordedFileFacts, ServerError> {
    let mut cursor: Option<String> = None;
    let mut facts = RecordedFileFacts::default();
    loop {
        let page = state
            .application()
            .read_owned_tool_parts(id, cursor.as_deref(), 128)
            .await?;
        for part in &page.parts {
            let contribution = recorded_file_facts(part);
            facts.incomplete |= contribution.incomplete;
            for (path, edits) in contribution.rows {
                facts.rows.entry(path).or_default().extend(edits);
            }
        }
        if facts.retained_bytes() > MAX_FILE_FACT_CACHE_BYTES {
            return Err(ServerError::bad_request(
                "This file history exceeds the read budget. Export the session to inspect the complete archive.",
            ));
        }
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Err(ServerError::internal("The history page did not advance."));
        }
    }
    for edits in facts.rows.values_mut() {
        edits.sort_by_key(|edit| edit.part_id);
    }
    facts.rows.retain(|_, ops| {
        !(ops.iter().all(|o| {
            o.from_path.is_none() && o.before_sha256.is_some() && o.after_sha256.is_some()
        }) && ops
            .windows(2)
            .all(|w| w[0].after_sha256 == w[1].before_sha256)
            && ops.first().and_then(|o| o.before_sha256.as_ref())
                == ops.last().and_then(|o| o.after_sha256.as_ref()))
    });
    Ok(facts)
}

/// The resource clock wakes readers; the representation validator determines
/// whether this particular summary/page/file actually needs any body bytes.
async fn file_changes_response(
    read: crate::revisions::ConditionalRead,
    projection: SessionFileChangesResource,
    headers: &HeaderMap,
    current_revision: String,
) -> Result<Response, ServerError> {
    let authority = read
        .token()
        .rsplit_once(':')
        .map_or(read.token(), |(epoch, _)| epoch)
        .to_owned();
    let (bytes, etag) = FILE_PROJECTIONS
        .run(move || {
            let bytes = serde_json::to_vec(&projection)?;
            let etag = format!("W/\"{authority}:{}\"", blake3::hash(&bytes).to_hex());
            Ok::<_, serde_json::Error>((bytes, etag))
        })
        .await
        .map_err(|error| ServerError::internal_error(&error))?
        .map_err(|error| ServerError::internal_error(&error))?;
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|tag| tag.trim().trim_start_matches("W/") == etag.trim_start_matches("W/"))
        });
    let response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
    };
    let mut response = read.headers(response);
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("file representation ETag"),
    );
    response.headers_mut().insert(
        "x-resource-current-revision",
        HeaderValue::from_str(&current_revision).expect("current file resource revision"),
    );
    Ok(response)
}

static FILE_PROJECTIONS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

const MAX_FILE_FACT_CACHE_BYTES: usize = 64 * 1024 * 1024;

pub(crate) struct FileChangesProjectionCache {
    entries: Mutex<FileChangesCacheEntries>,
    gates: Mutex<LruCache<i64, Arc<tokio::sync::Mutex<()>>>>,
    max_bytes: usize,
}

struct FileChangesCacheEntries {
    facts: LruCache<i64, (String, Arc<RecordedFileFacts>, usize)>,
    bytes: usize,
}

impl Default for FileChangesProjectionCache {
    fn default() -> Self {
        Self {
            entries: Mutex::new(FileChangesCacheEntries {
                facts: LruCache::new(NonZeroUsize::new(32).unwrap()),
                bytes: 0,
            }),
            gates: Mutex::new(LruCache::new(NonZeroUsize::new(32).unwrap())),
            max_bytes: MAX_FILE_FACT_CACHE_BYTES,
        }
    }
}

impl FileChangesProjectionCache {
    fn gate(&self, id: i64) -> Arc<tokio::sync::Mutex<()>> {
        let mut gates = self.gates.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(gate) = gates.get(&id) {
            return gate.clone();
        }
        let gate = Arc::new(tokio::sync::Mutex::new(()));
        gates.put(id, gate.clone());
        gate
    }

    fn get(&self, id: i64, token: &str) -> Option<Arc<RecordedFileFacts>> {
        let mut cache = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (cached, facts, _) = cache.facts.get(&id)?;
        (cached == token).then(|| facts.clone())
    }

    fn insert(&self, id: i64, token: &str, facts: Arc<RecordedFileFacts>) {
        let bytes = facts.retained_bytes();
        let mut retired = Vec::new();
        let mut cache = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((_, facts, previous)) = cache.facts.pop(&id) {
            cache.bytes -= previous;
            retired.push(facts);
        }
        if bytes > self.max_bytes {
            drop(cache);
            drop(retired);
            return;
        }
        while cache.bytes.saturating_add(bytes) > self.max_bytes {
            let Some((_, (_, facts, previous))) = cache.facts.pop_lru() else {
                break;
            };
            cache.bytes -= previous;
            retired.push(facts);
        }
        if let Some((_, (_, facts, previous))) =
            cache.facts.push(id, (token.to_owned(), facts, bytes))
        {
            cache.bytes -= previous;
            retired.push(facts);
        }
        cache.bytes += bytes;
        drop(cache);
        drop(retired);
    }
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
/// Count `+`/`-` body lines of one diff. File headers and hunk headers never
/// count, but write/replace diffs carry no hunk header, so a header line cannot
/// be a precondition for counting.
fn count_changed_lines(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

/// One merged addition/removal pair per file: every recorded operation of the
/// path contributes, and a patch operation contributes only this file's part.
fn changed_lines(path: &str, operations: &[RecordedFileEdit]) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for operation in operations {
        let Some((diff, _)) = &operation.diff else {
            continue;
        };
        // A patch diff covers every file of the operation, so an unlocatable
        // section counts nothing instead of borrowing another file's lines.
        let scoped = if operation.diff_scope == "operation" {
            patch_file_diff(diff, path, operation.from_path.as_deref())
        } else {
            Some(&**diff)
        };
        let Some(scoped) = scoped else {
            continue;
        };
        let (plus, minus) = count_changed_lines(scoped);
        added += plus;
        removed += minus;
    }
    (added, removed)
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
            incomplete = part.state != "pending"
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
        if part.state != "completed"
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
                let (added, removed) = changed_lines(path, operations);
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
                    added,
                    removed,
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
