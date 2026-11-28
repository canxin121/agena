use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use agena_domain::{ContentId, ToolApiFunction, ToolInvocation};
use agena_plugin_host::sdk::{
    PluginKey,
    host_api::{HookTranscriptFormat, ImportedHookFile},
};
use agena_storage::kernel::{
    EntryId, EntryPayload, EntryView, KernelReader, RoundId, SessionId, SessionSummaryView,
    TextCursor, ToolPhase, ToolView, WatchCursor,
};
use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufWriter};
use uuid::Uuid;

use super::private_directory;

const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
static FILES: agena_async::BlockingPool = agena_async::BlockingPool::new(4);

#[derive(Debug, thiserror::Error)]
pub enum TranscriptError {
    #[error(transparent)]
    Storage(#[from] agena_storage::kernel::KernelError),
    #[error("transcript file operation failed: {0}")]
    Io(#[from] io::Error),
    #[error("could not encode a transcript record")]
    Json(#[from] serde_json::Error),
}

#[derive(Default)]
pub(super) struct SessionTranscripts {
    files: Arc<tokio::sync::Mutex<BTreeMap<HookTranscriptFormat, Transcript>>>,
}

struct Transcript {
    file: ImportedHookFile,
    workspace: PathBuf,
    cursor: Option<WatchCursor>,
    owners: BTreeMap<PluginKey, PathBuf>,
}

impl SessionTranscripts {
    pub(super) async fn for_hook(
        &self,
        reader: &KernelReader,
        session: SessionId,
        workspace: &Path,
        format: HookTranscriptFormat,
        owner: (PluginKey, PathBuf),
    ) -> Result<ImportedHookFile, TranscriptError> {
        self.refresh(reader, session, workspace, format, Some(owner))
            .await
    }

    pub(super) async fn active_formats(
        &self,
        allowed: impl Fn(&PluginKey, &Path) -> bool,
    ) -> Vec<HookTranscriptFormat> {
        self.files
            .lock()
            .await
            .iter()
            .filter(|(_, entry)| entry.owners.iter().any(|(key, root)| allowed(key, root)))
            .map(|(format, _)| *format)
            .collect()
    }

    pub(super) async fn refresh(
        &self,
        reader: &KernelReader,
        session: SessionId,
        workspace: &Path,
        format: HookTranscriptFormat,
        owner: Option<(PluginKey, PathBuf)>,
    ) -> Result<ImportedHookFile, TranscriptError> {
        let mut files = self.files.clone().lock_owned().await;
        // Install the watch before reading the membership/contents. Events
        // that race the export will be seen by the next refresh.
        let previous = files.get(&format).and_then(|entry| entry.cursor.clone());
        let mut watch = reader.watch(session, previous).ok();
        let cursor = watch.as_ref().and_then(|watch| watch.head().cloned());
        if let Some(existing) = files.get_mut(&format)
            && existing.workspace == workspace
            && watch.as_mut().is_some_and(|watch| {
                !watch.needs_snapshot() && matches!(watch.try_recv(), Ok(None))
            })
        {
            existing.cursor = cursor;
            if let Some((key, root)) = owner {
                existing.owners.insert(key, root);
            }
            return Ok(existing.file.clone());
        }

        let summary = reader.summary(session).await?;
        let file = match files.get(&format) {
            Some(previous) => previous.file.clone(),
            None => FILES
                .run(move || new_file(format))
                .await
                .map_err(io::Error::other)??,
        };
        let location = file
            .path()
            .parent()
            .expect("transcript has a private directory")
            .to_owned();
        let (draft, writer) = FILES
            .run(move || {
                let draft = tempfile::NamedTempFile::new_in(location)?;
                let writer = draft.reopen()?;
                Ok::<_, io::Error>((draft, writer))
            })
            .await
            .map_err(io::Error::other)??;
        let mut document = Document {
            writer: BufWriter::new(tokio::fs::File::from_std(writer)),
            format,
            session_uuid: projection_id(workspace, session),
            summary: &summary,
            workspace,
            previous: None,
        };
        document.project(reader, MAX_FILE_BYTES).await?;
        document.writer.flush().await?;
        drop(document);

        let workspace = workspace.to_owned();
        // Keep the mutex through publication even if the callback is dropped
        // after the blocking job has started.
        FILES
            .run(move || {
                draft.persist(file.path()).map_err(|error| error.error)?;
                let entry = files.entry(format).or_insert_with(|| Transcript {
                    file: file.clone(),
                    workspace: workspace.clone(),
                    cursor: None,
                    owners: BTreeMap::new(),
                });
                entry.workspace = workspace;
                // Forked entries may acquire content from another session's
                // execution. Its writes are outside this session's change feed.
                entry.cursor = cursor.filter(|_| summary.parent_session_id.is_none());
                if let Some((key, root)) = owner {
                    entry.owners.insert(key, root);
                }
                Ok::<_, io::Error>(file)
            })
            .await
            .map_err(io::Error::other)?
            .map_err(Into::into)
    }
}

fn new_file(format: HookTranscriptFormat) -> io::Result<ImportedHookFile> {
    let directory = Arc::new(private_directory()?);
    let name = match format {
        HookTranscriptFormat::Claude => "conversation.jsonl",
        HookTranscriptFormat::Codex => "rollout.jsonl",
    };
    Ok(ImportedHookFile::new(
        directory.path().join(name),
        directory,
    ))
}

fn projection_id(workspace: &Path, session: SessionId) -> Uuid {
    let mut identity = blake3::Hasher::new_derive_key("agena hook transcript session");
    identity.update(workspace.as_os_str().as_encoded_bytes());
    identity.update(&session.get().to_le_bytes());
    uuid_from_hash(identity.finalize())
}

fn entry_uuid(session: Uuid, entry: EntryId, part: u8) -> Uuid {
    let mut identity = blake3::Hasher::new_derive_key("agena hook transcript entry");
    identity.update(session.as_bytes());
    identity.update(&entry.get().to_le_bytes());
    identity.update(&[part]);
    uuid_from_hash(identity.finalize())
}

fn uuid_from_hash(hash: blake3::Hash) -> Uuid {
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash.as_bytes()[..16]);
    Uuid::from_bytes(
        uuid::Builder::from_custom_bytes(bytes)
            .into_uuid()
            .into_bytes(),
    )
}

struct Document<'a> {
    writer: BufWriter<tokio::fs::File>,
    format: HookTranscriptFormat,
    session_uuid: Uuid,
    summary: &'a SessionSummaryView,
    workspace: &'a Path,
    previous: Option<Uuid>,
}

struct EntryRecords {
    first: Value,
    second: Option<Value>,
    tool_round: Option<RoundId>,
}

impl EntryRecords {
    fn new(first: Value, second: Option<Value>) -> Self {
        Self {
            first,
            second,
            tool_round: None,
        }
    }

    fn encoded_size(&self) -> Result<usize, serde_json::Error> {
        // Allow room for the newline and the final Claude UUID link.
        std::iter::once(&self.first)
            .chain(self.second.iter())
            .try_fold(0, |total, record| {
                Ok(total + serde_json::to_vec(record)?.len() + 38)
            })
    }
}

impl Document<'_> {
    async fn project(
        &mut self,
        reader: &KernelReader,
        mut bytes_left: usize,
    ) -> Result<(), TranscriptError> {
        if self.format == HookTranscriptFormat::Codex {
            let created = timestamp(self.summary.created_at_ms);
            let root = projection_id(self.workspace, self.summary.root_session_id);
            let parent = self
                .summary
                .parent_session_id
                .map(|id| projection_id(self.workspace, id));
            let meta = json!({
                "type":"session_meta", "timestamp":created,
                "payload":{
                    "id":self.session_uuid, "session_id":root, "parent_thread_id":parent,
                    "timestamp":created, "cwd":self.workspace, "originator":"agena",
                    "cli_version":env!("CARGO_PKG_VERSION"), "source":"exec",
                    "agena_session_id":self.summary.id.get().to_string(),
                }
            });
            bytes_left = bytes_left
                .checked_sub(serde_json::to_vec(&meta)?.len() + 1)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "transcript metadata exceeds the file budget",
                    )
                })?;
            self.append(meta).await?;
        }
        let mut newest_first = Vec::new();
        let mut cursor = None;
        'history: loop {
            let page = reader.history(self.summary.id, cursor, 128).await?;
            for entry in page.entries.iter().rev() {
                let Some(records) = self.project_entry(reader, entry).await? else {
                    continue;
                };
                let bytes = records.encoded_size()?;
                if bytes > bytes_left {
                    break 'history;
                }
                bytes_left -= bytes;
                newest_first.push(records);
            }
            if !page.has_more {
                break;
            }
            cursor = page.next;
        }
        let mut round = None;
        let mut results = Vec::new();
        for records in newest_first.into_iter().rev() {
            if records.tool_round != round {
                for result in results.drain(..) {
                    self.append(result).await?;
                }
                round = records.tool_round;
            }
            self.append(records.first).await?;
            if let Some(second) = records.second {
                if round.is_some() {
                    results.push(second);
                } else {
                    self.append(second).await?;
                }
            }
        }
        for result in results {
            self.append(result).await?;
        }
        Ok(())
    }

    async fn project_entry(
        &self,
        reader: &KernelReader,
        entry: &EntryView,
    ) -> Result<Option<EntryRecords>, TranscriptError> {
        let records = match &entry.payload {
            EntryPayload::User { document, .. } if entry.execution_started => {
                let mut text = document.text();
                if document.activity_ids().next().is_some() {
                    text.push_str("\n[See the Agena session for attached content.]");
                }
                self.message(entry, "user", &bounded_text(&text))
            }
            EntryPayload::PluginPrompt(prompt) => {
                self.message(entry, "user", &bounded_text(&prompt.text))
            }
            EntryPayload::Text { content_id, .. } if entry.compaction_phase.is_none() => {
                let text = read_text(reader, self.summary.id, *content_id).await?;
                if text.is_empty() {
                    return Ok(None);
                }
                self.message(entry, "assistant", &text)
            }
            EntryPayload::Tool { tool_id } => {
                let tool = reader.tool(self.summary.id, *tool_id).await?;
                if tool.phase == ToolPhase::Preparing {
                    return Ok(None);
                }
                self.tool(entry, &tool)?
            }
            EntryPayload::Compaction { summary, .. } if entry.compaction_committed => {
                let text = read_text(reader, self.summary.id, *summary).await?;
                let record = match self.format {
                    HookTranscriptFormat::Claude => json!({
                        "type":"summary", "summary":text, "leafUuid":null,
                    }),
                    HookTranscriptFormat::Codex => {
                        self.codex(entry.created_at_ms, "compacted", json!({"message":text}))
                    }
                };
                EntryRecords::new(record, None)
            }
            _ => return Ok(None),
        };
        Ok(Some(records))
    }

    fn message(&self, entry: &EntryView, role: &str, text: &str) -> EntryRecords {
        match self.format {
            HookTranscriptFormat::Claude => EntryRecords::new(
                self.claude(
                    entry.id,
                    0,
                    entry.created_at_ms,
                    role,
                    json!({"type":"text", "text":text}),
                ),
                None,
            ),
            HookTranscriptFormat::Codex => {
                let (kind, event) = if role == "user" {
                    ("input_text", "user_message")
                } else {
                    ("output_text", "agent_message")
                };
                let message = self.codex(
                    entry.created_at_ms,
                    "response_item",
                    json!({
                        "type":"message", "role":role, "content":[{"type":kind,"text":text}],
                    }),
                );
                let event = self.codex(
                    entry.created_at_ms,
                    "event_msg",
                    json!({"type":event,"message":text}),
                );
                EntryRecords::new(message, Some(event))
            }
        }
    }

    fn tool(&self, entry: &EntryView, tool: &ToolView) -> Result<EntryRecords, TranscriptError> {
        let call = tool.id.get().to_string();
        let (name, input) = tool_call(tool);
        let (name, input) =
            agena_plugin_host::agent_plugin::project_transcript_tool(&name, input, self.format);
        let finished = tool.finished_at_ms.unwrap_or(tool.created_at_ms);
        let output = tool.result.as_ref().map(tool_result);
        let (first, second) = match self.format {
            HookTranscriptFormat::Claude => {
                let first = self.claude(
                    entry.id,
                    0,
                    tool.created_at_ms,
                    "assistant",
                    json!({
                        "type":"tool_use", "id":call, "name":name, "input":input,
                    }),
                );
                let second = output.map(|output| {
                    self.claude(
                        entry.id,
                        1,
                        finished,
                        "user",
                        json!({
                            "type":"tool_result", "tool_use_id":call, "content":output,
                            "is_error":tool.phase != ToolPhase::Succeeded,
                        }),
                    )
                });
                (first, second)
            }
            HookTranscriptFormat::Codex => {
                let first = self.codex(
                    tool.created_at_ms,
                    "response_item",
                    json!({
                        "type":"function_call", "name":name, "call_id":call,
                        "arguments":serde_json::to_string(&input)?,
                    }),
                );
                let second = output.map(|output| {
                    self.codex(
                        finished,
                        "response_item",
                        json!({
                            "type":"function_call_output", "call_id":call, "output":output,
                        }),
                    )
                });
                (first, second)
            }
        };
        Ok(EntryRecords {
            first,
            second,
            tool_round: Some(tool.round_id),
        })
    }

    fn claude(&self, entry: EntryId, part: u8, time_ms: i64, role: &str, content: Value) -> Value {
        let uuid = entry_uuid(self.session_uuid, entry, part);
        json!({
            "type":role, "sessionId":self.summary.id.get().to_string(), "parentUuid":null,
            "uuid":uuid, "cwd":self.workspace, "timestamp":timestamp(time_ms),
            "isSidechain":self.summary.relation_kind.is_subagent(), "version":env!("CARGO_PKG_VERSION"),
            "message":{"role":role,"content":[content]},
        })
    }

    fn codex(&self, time_ms: i64, kind: &str, payload: Value) -> Value {
        json!({"timestamp":timestamp(time_ms),"type":kind,"payload":payload})
    }

    async fn append(&mut self, mut record: Value) -> Result<(), TranscriptError> {
        if let Some(link) = record.get_mut("parentUuid") {
            *link = json!(self.previous);
            self.previous = record["uuid"]
                .as_str()
                .and_then(|uuid| Uuid::parse_str(uuid).ok());
        } else if let Some(link) = record.get_mut("leafUuid") {
            *link = json!(self.previous);
        }
        let line = serde_json::to_vec(&record)?;
        self.writer.write_all(&line).await?;
        self.writer.write_all(b"\n").await?;
        Ok(())
    }
}

fn timestamp(time_ms: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(time_ms)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn tool_call(tool: &ToolView) -> (String, Value) {
    if let Some(invocation) = tool
        .result
        .as_ref()
        .and_then(|result| result.get("invocation"))
        .and_then(|value| serde_json::from_value::<ToolInvocation>(value.clone()).ok())
    {
        return (invocation.name, invocation.input.into());
    }
    if tool.call.name == ToolApiFunction::Call.function_name()
        && let Some(name) = tool.call.input.get("tool").and_then(Value::as_str)
        && let Some(input) = tool
            .call
            .input
            .get("input")
            .filter(|input| input.is_object())
    {
        return (name.to_owned(), input.clone());
    }
    (tool.call.name.clone(), tool.call.input.clone())
}

fn tool_result(value: &Value) -> String {
    if let Some(text) = [
        "/hook_model_output",
        "/tool_api_response/output_text",
        "/error/user/fallback",
        "/raw_output/payload/output",
        "/raw_output/payload/text",
    ]
    .into_iter()
    .find_map(|key| value.pointer(key).and_then(Value::as_str))
    {
        return bounded_text(text);
    }
    if let Some(payload) = value.pointer("/raw_output/payload") {
        return bounded_text(&payload.to_string());
    }
    bounded_text(
        value
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
}

async fn read_text(
    reader: &KernelReader,
    session: SessionId,
    content: ContentId,
) -> Result<String, TranscriptError> {
    let first = reader.text_range(session, content, None, 1).await?;
    let head = &first.stream;
    let mut cursor = TextCursor {
        content,
        epoch: head.epoch,
        offset: head.retained_start,
    };
    let limit = head
        .committed
        .saturating_sub(head.retained_start)
        .min(MAX_TEXT_BYTES as u64) as usize;
    let mut bytes = Vec::with_capacity(limit);
    while bytes.len() < limit {
        let range = reader
            .text_range_at(
                session,
                head,
                Some(cursor),
                (limit - bytes.len()).min(64 * 1024),
            )
            .await?;
        if range.next.offset <= cursor.offset {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "transcript text cursor did not advance",
            )
            .into());
        }
        bytes.extend_from_slice(&range.bytes);
        cursor = range.next;
    }
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if head.retained_start > 0 {
        text.insert_str(0, "[Earlier text is outside the retained range.]\n");
    }
    if cursor.offset < head.committed {
        text.push_str("\n[Transcript text truncated.]");
    }
    Ok(text)
}

fn bounded_text(text: &str) -> String {
    if text.len() <= MAX_TEXT_BYTES {
        text.to_owned()
    } else {
        format!(
            "{}\n[Transcript text truncated.]",
            &text[..text.floor_char_boundary(MAX_TEXT_BYTES)]
        )
    }
}

#[cfg(test)]
mod tests;
