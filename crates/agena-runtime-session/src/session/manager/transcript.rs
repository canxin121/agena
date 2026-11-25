use super::discovery_cursor::{continuation, resume};
use super::*;
use agena_domain::ContentId;
use agena_plugin_host::sdk::host_api::{
    HostSessionReadDirection, HostSessionReadRequest, HostSessionSearchRequest,
};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SEARCH_SCAN_BYTES: usize = 512 * 1024;
const SEARCH_CHUNK_BYTES: usize = 32 * 1024;
const SEARCH_MAX_SESSIONS: usize = 8;
const READ_ENTRY_BYTES: usize = 2048;
const MAX_QUERY_BYTES: usize = 256;

#[derive(Clone, Copy, Serialize, Deserialize)]
enum SearchPosition {
    Sessions(Option<SessionSummaryCursor>),
    Entries {
        session: SessionId,
        summary: Option<SessionSummaryCursor>,
        entry: Option<EntryId>,
        text: Option<TextCursor>,
    },
}

enum Match {
    Hit {
        snippet: String,
        start: u64,
        offset: u64,
    },
    Continue(TextCursor),
    Miss,
}

fn project_for_history(ancestry: &[ManagementAncestor]) -> Option<i64> {
    let project = ancestry.first()?.workspace_id?;
    ancestry
        .iter()
        .all(|row| row.workspace_id == Some(project) && row.owner.is_none() && !row.excluded_scope)
        .then_some(project)
}

fn unavailable_target() -> AppError {
    AppError::Config("session is not available in this transcript scope".into())
}

fn transcript_error(error: KernelError) -> KernelError {
    match error {
        KernelError::Ownership | KernelError::NotFound(_) => KernelError::Conflict(
            "session or entry is no longer retained in this transcript".into(),
        ),
        KernelError::ContentReset => KernelError::Conflict(
            "byte offset exceeds the retained text or its cursor has changed".into(),
        ),
        other => other,
    }
}

fn floor_boundary(text: &str, mut position: usize) -> usize {
    while !text.is_char_boundary(position) {
        position -= 1;
    }
    position
}

fn utf8_prefix(bytes: &[u8]) -> Result<&str, AppError> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(error) if error.error_len().is_none() => {
            std::str::from_utf8(&bytes[..error.valid_up_to()])
                .map_err(|error| AppError::Internal(error.to_string()))
        }
        Err(_) => Err(AppError::Config(
            "byte_offset must be on a UTF-8 character boundary".into(),
        )),
    }
}

fn match_snippet(pattern: &Regex, text: &str, base: u64) -> Option<Match> {
    let found = pattern.find(text)?;
    let start = floor_boundary(text, found.start().saturating_sub(120));
    let end = floor_boundary(text, (found.end() + 160).min(text.len()));
    Some(Match::Hit {
        snippet: text[start..end].to_owned(),
        start: base + start as u64,
        offset: base + found.start() as u64,
    })
}

fn identify(entry: &EntryView) -> Value {
    let user = matches!(entry.payload, EntryPayload::User { .. });
    json!({
        "entry_id": entry.id.get(),
        "message_id": entry.message_id.get(),
        "run_id": if user { entry.id } else { entry.response_entry_id.unwrap_or(entry.id) }.get(),
        "role": if user { "user" } else { "assistant" },
        "created_at_ms": entry.created_at_ms,
    })
}

impl SessionManager {
    async fn transcript_project(&self, session: SessionId) -> Result<Option<i64>, AppError> {
        let reader = &self.kernel().reader;
        let ancestors = read_with_busy_retry(session, "transcript scope", || async {
            reader
                .management_ancestry(session)
                .await
                .map_err(transcript_error)
        })
        .await?;
        Ok(project_for_history(&ancestors))
    }

    async fn transcript_visible(
        &self,
        source: SessionId,
        project: Option<i64>,
        target: SessionId,
    ) -> Result<bool, AppError> {
        if source == target {
            return Ok(true);
        }
        let Some(project) = project else {
            return Ok(false);
        };
        match self.transcript_project(target).await {
            Ok(target_project) => Ok(target_project == Some(project)),
            Err(AppError::Config(_)) => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn transcript_page(
        &self,
        session: SessionId,
        position: TranscriptPosition,
        limit: usize,
    ) -> Result<ReadView<TranscriptPage>, AppError> {
        let reader = &self.kernel().reader;
        read_with_busy_retry(session, "plain-text transcript", || async {
            reader
                .transcript_page(session, position, limit)
                .await
                .map_err(transcript_error)
        })
        .await
    }

    async fn transcript_range(
        &self,
        session: SessionId,
        content: ContentId,
        cursor: Option<TextCursor>,
        limit: usize,
    ) -> Result<ReadView<TextRange>, AppError> {
        let reader = &self.kernel().reader;
        read_with_busy_retry(session, "transcript text", || async {
            match reader.text_range(session, content, cursor, limit).await {
                Err(KernelError::ContentGap { retained_start, .. }) => {
                    let epoch = match cursor {
                        Some(cursor) => cursor.epoch,
                        None => reader.watch_content(session, content).await?.head()?.epoch,
                    };
                    let retained = TextCursor {
                        content,
                        epoch,
                        offset: retained_start,
                    };
                    reader
                        .text_range(session, content, Some(retained), limit)
                        .await
                        .map_err(transcript_error)
                }
                result => result.map_err(transcript_error),
            }
        })
        .await
    }

    async fn search_transcript_entry(
        &self,
        session: SessionId,
        entry: &EntryView,
        pattern: &Regex,
        mut cursor: Option<TextCursor>,
        remaining: &mut usize,
    ) -> Result<Match, AppError> {
        match &entry.payload {
            EntryPayload::User { document, .. } => {
                let text = document.text();
                *remaining = remaining.saturating_sub(text.len().max(4096));
                Ok(match_snippet(pattern, &text, 0).unwrap_or(Match::Miss))
            }
            EntryPayload::Text { content_id } => {
                let mut buffer = String::new();
                loop {
                    let range = self
                        .transcript_range(session, *content_id, cursor, SEARCH_CHUNK_BYTES)
                        .await?;
                    if cursor.is_some_and(|cursor| cursor.offset != range.start) {
                        buffer.clear();
                    }
                    let start = range.start - buffer.len() as u64;
                    let text = utf8_prefix(&range.bytes)?;
                    *remaining = remaining.saturating_sub(text.len().max(4096));
                    buffer.push_str(text);
                    if let Some(found) = match_snippet(pattern, &buffer, start) {
                        return Ok(found);
                    }
                    if !range.has_more {
                        return Ok(Match::Miss);
                    }
                    if text.is_empty() {
                        return Err(AppError::Internal(
                            "transcript text cursor cannot advance".into(),
                        ));
                    }
                    // A folded Unicode scalar may use up to four bytes. Keep
                    // enough context for matches spanning any chunk boundary.
                    let keep = MAX_QUERY_BYTES * 4;
                    let tail = floor_boundary(&buffer, buffer.len().saturating_sub(keep));
                    if *remaining == 0 {
                        return Ok(Match::Continue(TextCursor {
                            offset: start + tail as u64,
                            ..range.next
                        }));
                    }
                    buffer.drain(..tail);
                    cursor = Some(TextCursor {
                        offset: range.start + text.len() as u64,
                        ..range.next
                    });
                }
            }
            _ => Err(AppError::Internal(
                "unexpected entry in the plain-text transcript".into(),
            )),
        }
    }

    pub async fn search_session_transcript(
        &self,
        source: i64,
        request: HostSessionSearchRequest,
    ) -> Result<Value, AppError> {
        let source = session_id(source)?;
        let query = request.query.trim();
        if query.is_empty() || query.len() > MAX_QUERY_BYTES {
            return Err(AppError::Config(
                "query must contain 1–256 UTF-8 bytes".into(),
            ));
        }
        let limit = request.limit.unwrap_or(10).clamp(1, 20);
        let pattern = RegexBuilder::new(&regex::escape(query))
            .case_insensitive(true)
            .build()
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let project = self.transcript_project(source).await?;
        let explicit = request.session_id.map(session_id).transpose()?;
        if let Some(target) = explicit
            && !self.transcript_visible(source, project, target).await?
        {
            return Err(unavailable_target());
        }
        let target = explicit.or(project.is_none().then_some(source));
        let reader = &self.kernel().reader;
        let scope = json!({
            "operation": "session.search", "kernel": reader.epoch(), "source": source,
            "project": project, "target": target, "query": query,
        });
        let initial = match target {
            Some(session) => SearchPosition::Entries {
                session,
                summary: None,
                entry: None,
                text: None,
            },
            None => SearchPosition::Sessions(None),
        };
        let mut position = Some(resume(&request.cursor, &scope)?.unwrap_or(initial));
        let mut remaining = SEARCH_SCAN_BYTES;
        let mut sessions = 0;
        let mut checked_target = explicit;
        let mut hits = Vec::new();
        while let Some(current) = position {
            if remaining == 0 || hits.len() == limit {
                break;
            }
            match current {
                SearchPosition::Sessions(before) => {
                    if sessions == SEARCH_MAX_SESSIONS {
                        break;
                    }
                    let page = read_with_busy_retry(source, "transcript sessions", || {
                        reader.query_summaries(SummaryQuery {
                            workspace_id: project,
                            exclude_bot_owned: true,
                            exclude_subagents: true,
                            before,
                            limit: 1,
                            ..Default::default()
                        })
                    })
                    .await?;
                    let Some(next) = page.sessions.first() else {
                        position = None;
                        break;
                    };
                    sessions += 1;
                    position = if self.transcript_visible(source, project, next.id).await? {
                        checked_target = Some(next.id);
                        Some(SearchPosition::Entries {
                            session: next.id,
                            summary: page.next,
                            entry: None,
                            text: None,
                        })
                    } else {
                        page.next.map(|next| SearchPosition::Sessions(Some(next)))
                    };
                }
                SearchPosition::Entries {
                    session,
                    summary,
                    entry,
                    text,
                } => {
                    if checked_target != Some(session) {
                        if !self.transcript_visible(source, project, session).await? {
                            return Err(unavailable_target());
                        }
                        checked_target = Some(session);
                    }
                    let window = match (entry, text) {
                        (Some(entry), Some(_)) => TranscriptPosition::AtOrBefore(entry),
                        (Some(entry), None) => TranscriptPosition::Before(entry),
                        (None, _) => TranscriptPosition::Latest,
                    };
                    let page = self.transcript_page(session, window, 12).await?;
                    let mut entries = page.entries.iter().rev().peekable();
                    position = summary.map(|next| SearchPosition::Sessions(Some(next)));
                    if text.is_some() && entries.peek().map(|row| row.id) != entry {
                        return Err(AppError::Config(
                            "search cursor's text entry is no longer available".into(),
                        ));
                    }
                    while let Some(row) = entries.next() {
                        let cursor = if Some(row.id) == entry { text } else { None };
                        match self
                            .search_transcript_entry(session, row, &pattern, cursor, &mut remaining)
                            .await?
                        {
                            Match::Hit {
                                snippet,
                                start,
                                offset,
                            } => {
                                let mut hit = identify(row);
                                hit["session_id"] = json!(session.get());
                                hit["snippet"] = json!(snippet);
                                hit["snippet_byte_offset"] = json!(start);
                                hit["match_byte_offset"] = json!(offset);
                                hits.push(hit);
                            }
                            Match::Continue(cursor) => {
                                position = Some(SearchPosition::Entries {
                                    session,
                                    summary,
                                    entry: Some(row.id),
                                    text: Some(cursor),
                                });
                                break;
                            }
                            Match::Miss => {}
                        }
                        let more = entries.peek().is_some() || page.has_more;
                        if more {
                            position = Some(SearchPosition::Entries {
                                session,
                                summary,
                                entry: Some(row.id),
                                text: None,
                            });
                        } else {
                            position = summary.map(|next| SearchPosition::Sessions(Some(next)));
                        }
                        if remaining == 0 || hits.len() == limit {
                            break;
                        }
                    }
                }
            }
        }
        Ok(json!({
            "hits": hits,
            "search_scope": if target.is_some() { "session" } else { "project" },
            "session_id": target.map(SessionId::get),
            "has_more": position.is_some(),
            "next_cursor": position.map(|p| continuation(&scope, &p)).transpose()?,
            "untrusted_data": true,
        }))
    }

    async fn read_transcript_entry(
        &self,
        session: SessionId,
        entry: &EntryView,
        offset: Option<u64>,
    ) -> Result<Value, AppError> {
        let mut output = identify(entry);
        let (text, start, has_more) = match &entry.payload {
            EntryPayload::User { document, .. } => {
                let text = document.text();
                let offset = usize::try_from(offset.unwrap_or_default())
                    .ok()
                    .filter(|offset| *offset <= text.len() && text.is_char_boundary(*offset))
                    .ok_or_else(|| {
                        AppError::Config(
                            "byte_offset must be a UTF-8 boundary within the text".into(),
                        )
                    })?;
                let end = floor_boundary(&text, (offset + READ_ENTRY_BYTES).min(text.len()));
                (
                    text[offset..end].to_owned(),
                    offset as u64,
                    end < text.len(),
                )
            }
            EntryPayload::Text { content_id } => {
                let cursor = if let Some(offset) = offset.filter(|offset| *offset > 0) {
                    let head = self.transcript_range(session, *content_id, None, 1).await?;
                    Some(TextCursor {
                        offset,
                        ..head.next
                    })
                } else {
                    None
                };
                let range = self
                    .transcript_range(session, *content_id, cursor, READ_ENTRY_BYTES)
                    .await?;
                if offset.is_some_and(|offset| range.start != offset) {
                    return Err(AppError::Config(format!(
                        "text is retained from byte offset {}",
                        range.start
                    )));
                }
                let text = utf8_prefix(&range.bytes)?;
                (
                    text.to_owned(),
                    range.start,
                    range.has_more || text.len() < range.bytes.len(),
                )
            }
            _ => {
                return Err(AppError::Internal(
                    "unexpected entry in the plain-text transcript".into(),
                ));
            }
        };
        output["byte_offset"] = json!(start);
        output["next_byte_offset"] = json!(has_more.then_some(start + text.len() as u64));
        output["text"] = json!(text);
        Ok(output)
    }

    pub async fn read_session_transcript(
        &self,
        source: i64,
        request: HostSessionReadRequest,
    ) -> Result<Value, AppError> {
        use HostSessionReadDirection::{After, Around, Before};

        let source = session_id(source)?;
        let session = request
            .session_id
            .map(session_id)
            .transpose()?
            .unwrap_or(source);
        if source != session {
            let project = self.transcript_project(source).await?;
            if !self.transcript_visible(source, project, session).await? {
                return Err(unavailable_target());
            }
        }
        let entry = request.entry_id.map(entry_id).transpose()?;
        let limit = request.limit.unwrap_or(7).clamp(1, 12);
        let mut entries = Vec::new();
        let mut before = None;
        let mut after = None;
        if let Some(offset) = request.byte_offset {
            let anchor = entry
                .filter(|_| request.direction == Around)
                .ok_or_else(|| {
                    AppError::Config("byte_offset requires entry_id and direction around".into())
                })?;
            let page = self
                .transcript_page(session, TranscriptPosition::AtOrBefore(anchor), 1)
                .await?;
            let row = page
                .entries
                .first()
                .filter(|row| row.id == anchor)
                .ok_or_else(|| AppError::Config("entry has no readable transcript text".into()))?;
            entries.push(
                self.read_transcript_entry(session, row, Some(offset))
                    .await?,
            );
        } else {
            let (position, count) = match (request.direction, entry) {
                (Around, None) => (TranscriptPosition::Latest, limit),
                (Around, Some(anchor)) => (TranscriptPosition::AtOrBefore(anchor), limit / 2 + 1),
                (Before, Some(anchor)) => (TranscriptPosition::Before(anchor), limit),
                (After, Some(anchor)) => (TranscriptPosition::After(anchor), limit),
                _ => {
                    return Err(AppError::Config(
                        "direction before or after requires entry_id".into(),
                    ));
                }
            };
            let page = self.transcript_page(session, position, count).await?;
            if page.has_more {
                if request.direction == After {
                    after = page.entries.last().map(|row| row.id);
                } else {
                    before = page.entries.first().map(|row| row.id);
                }
            }
            for row in &page.entries {
                entries.push(self.read_transcript_entry(session, row, None).await?);
            }
            if let Some(anchor) = entry.filter(|_| request.direction == Around) {
                let count = limit - entries.len();
                let newer = self
                    .transcript_page(session, TranscriptPosition::After(anchor), count.max(1))
                    .await?;
                if count == 0 && !newer.entries.is_empty() {
                    after = Some(anchor);
                } else if newer.has_more {
                    after = newer.entries.last().map(|row| row.id);
                }
                for row in newer.entries.iter().take(count) {
                    entries.push(self.read_transcript_entry(session, row, None).await?);
                }
            }
        }
        Ok(json!({
            "session_id": session.get(),
            "anchor_entry_id": entry.map(EntryId::get),
            "entries": entries,
            "before_entry_id": before.map(EntryId::get),
            "after_entry_id": after.map(EntryId::get),
            "untrusted_data": true,
        }))
    }
}
