use super::*;
use agena::message::{MessagePart, MessageUsage};
use agena::role::Role;

impl ApiService {
    pub async fn list_messages(
        &self,
        manager: &SessionManager,
        session_id: i64,
        query: MessageListQuery,
    ) -> ApiResult<PaginatedResponse<MessageResource>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(query.pagination.limit());
        let cursor = query
            .pagination
            .cursor()
            .map(decode_cursor::<MessageCursor>)
            .transpose()?;
        let visible =
            load_visible_message_projection(manager, session_id, query.parts == PartLoadMode::Full)
                .await?;
        let part_counts = load_visible_part_counts(manager, session_id, &visible).await?;
        let (messages, has_more, next_cursor) =
            paginate_visible_messages(visible.messages.as_slice(), cursor, limit);
        let items: Vec<MessageResource> = messages
            .iter()
            .map(|message| {
                message_resource_from_message(
                    session_id,
                    message,
                    query.parts,
                    visible_part_count(&part_counts, message),
                )
            })
            .collect();

        build_page(
            items,
            has_more,
            next_cursor.map(|(created_at_ms, id)| MessageCursor { created_at_ms, id }),
            PageOrder::Asc,
            limit,
        )
    }

    pub async fn get_message(
        &self,
        manager: &SessionManager,
        message_id: i64,
        parts: PartLoadMode,
    ) -> ApiResult<Option<MessageResource>> {
        let Some(session_id) = manager
            .find_session_id_for_message(message_id)
            .await
            .map_err(api_error_from_app)?
        else {
            return Ok(None);
        };
        let visible =
            load_visible_message_projection(manager, session_id, parts == PartLoadMode::Full)
                .await?;
        let part_counts = load_visible_part_counts(manager, session_id, &visible).await?;
        Ok(visible.find_message(message_id).map(|message| {
            message_resource_from_message(
                session_id,
                message,
                parts,
                visible_part_count(&part_counts, message),
            )
        }))
    }

    pub async fn list_message_parts(
        &self,
        manager: &SessionManager,
        message_id: i64,
        mode: PartLoadMode,
    ) -> ApiResult<Vec<MessagePart>> {
        let Some(session_id) = manager
            .find_session_id_for_message(message_id)
            .await
            .map_err(api_error_from_app)?
        else {
            return Err(ApiError::not_found(format!(
                "message not found: {message_id}"
            )));
        };
        if mode == PartLoadMode::None {
            return Ok(Vec::new());
        }
        let visible =
            load_visible_message_projection(manager, session_id, mode == PartLoadMode::Full)
                .await?;
        let Some(message) = visible.find_message(message_id) else {
            return Err(ApiError::not_found(format!(
                "message not found: {message_id}"
            )));
        };
        Ok(message
            .message
            .parts
            .iter()
            .cloned()
            .map(|part| project_part(part, mode))
            .collect())
    }

    pub async fn get_message_part(
        &self,
        manager: &SessionManager,
        part_id: i64,
    ) -> ApiResult<Option<MessagePart>> {
        let Some(session_id) = manager
            .find_session_id_for_part(part_id)
            .await
            .map_err(api_error_from_app)?
        else {
            return Ok(None);
        };
        let visible = load_visible_message_projection(manager, session_id, true).await?;
        Ok(visible.find_part(part_id))
    }
}

fn message_resource_from_message(
    session_id: i64,
    message: &VisibleMessageRecord,
    parts_mode: PartLoadMode,
    part_count: u64,
) -> MessageResource {
    let parts = match parts_mode {
        PartLoadMode::None => None,
        PartLoadMode::Summary | PartLoadMode::Full => Some(
            message
                .message
                .parts
                .iter()
                .cloned()
                .map(|p| project_part(p, parts_mode))
                .collect(),
        ),
    };
    MessageResource::from_message(
        session_id,
        &message.message,
        message.updated_at,
        part_count,
        parts,
    )
}

#[derive(Debug, Clone)]
struct VisibleMessageRecord {
    message: Message,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct VisibleMessageProjection {
    messages: Vec<VisibleMessageRecord>,
    hidden_message_aliases: HashMap<i64, i64>,
}

impl VisibleMessageProjection {
    fn find_message(&self, message_id: i64) -> Option<&VisibleMessageRecord> {
        let visible_id = self
            .hidden_message_aliases
            .get(&message_id)
            .copied()
            .unwrap_or(message_id);
        self.messages
            .iter()
            .find(|message| message.message.id == visible_id)
    }

    fn find_part(&self, part_id: i64) -> Option<MessagePart> {
        self.messages.iter().find_map(|message| {
            message
                .message
                .parts
                .iter()
                .find(|part| part.id == part_id)
                .cloned()
        })
    }
}

async fn load_visible_message_projection(
    manager: &SessionManager,
    session_id: i64,
    include_full_parts: bool,
) -> ApiResult<VisibleMessageProjection> {
    let messages = manager
        .list_projected_messages(session_id, include_full_parts)
        .await
        .map_err(api_error_from_app)?;
    Ok(project_visible_messages(messages))
}

async fn load_visible_part_counts(
    manager: &SessionManager,
    session_id: i64,
    projection: &VisibleMessageProjection,
) -> ApiResult<HashMap<i64, u64>> {
    let headers = manager
        .list_projected_message_headers(session_id)
        .await
        .map_err(api_error_from_app)?;
    let header_counts = headers
        .into_iter()
        .map(|header| (header.id, header.part_count))
        .collect::<HashMap<_, _>>();

    let mut counts = HashMap::<i64, u64>::new();
    for message in &projection.messages {
        if let Some(part_count) = header_counts.get(&message.message.id).copied() {
            counts.insert(message.message.id, part_count);
        }
    }

    for (hidden_message_id, visible_message_id) in &projection.hidden_message_aliases {
        if let Some(part_count) = header_counts.get(hidden_message_id).copied() {
            *counts.entry(*visible_message_id).or_default() += part_count;
        }
    }

    for message in &projection.messages {
        let loaded_part_count = message.message.parts.len() as u64;
        let count = counts.entry(message.message.id).or_default();
        *count = (*count).max(loaded_part_count);
    }

    Ok(counts)
}

fn visible_part_count(part_counts: &HashMap<i64, u64>, message: &VisibleMessageRecord) -> u64 {
    part_counts
        .get(&message.message.id)
        .copied()
        .unwrap_or(message.message.parts.len() as u64)
}

fn project_visible_messages(messages: Vec<Message>) -> VisibleMessageProjection {
    let mut visible = Vec::with_capacity(messages.len());
    let mut hidden_message_aliases = HashMap::new();
    let mut cursor = 0usize;

    while cursor < messages.len() {
        let mut message = messages[cursor].clone();
        normalize_message_parts(&mut message);

        if message.role != Role::Assistant {
            let updated_at = message.created_at;
            visible.push(VisibleMessageRecord {
                message,
                updated_at,
            });
            cursor += 1;
            continue;
        }

        let visible_message_id = message.id;
        let mut group = vec![message];
        let mut updated_at = group[0].created_at;
        cursor += 1;

        while cursor < messages.len() {
            let mut next = messages[cursor].clone();
            normalize_message_parts(&mut next);
            if next.role != Role::Assistant {
                break;
            }
            updated_at = next.created_at;
            hidden_message_aliases.insert(next.id, visible_message_id);
            group.push(next);
            cursor += 1;
        }

        visible.push(collapse_assistant_group(group, updated_at));
    }

    VisibleMessageProjection {
        messages: visible,
        hidden_message_aliases,
    }
}

fn normalize_message_parts(message: &mut Message) {
    for (index, part) in message.parts.iter_mut().enumerate() {
        part.message_id = message.id;
        part.part_index = index as i32;
    }
}

fn paginate_visible_messages(
    messages: &[VisibleMessageRecord],
    cursor: Option<MessageCursor>,
    limit: u64,
) -> (Vec<VisibleMessageRecord>, bool, Option<(i64, i64)>) {
    let mut filtered = messages
        .iter()
        .filter(|message| match cursor {
            Some(cursor) => {
                let key = message_cursor_key(message);
                key < (cursor.created_at_ms, cursor.id)
            }
            None => true,
        })
        .cloned()
        .collect::<Vec<_>>();

    filtered.sort_by_key(|message| std::cmp::Reverse(message_cursor_key(message)));
    let has_more = filtered.len() > limit as usize;
    filtered.truncate(limit as usize);
    let next_cursor = if has_more {
        filtered.last().map(message_cursor_key)
    } else {
        None
    };
    filtered.sort_by_key(message_cursor_key);

    (filtered, has_more, next_cursor)
}

fn message_cursor_key(message: &VisibleMessageRecord) -> (i64, i64) {
    (
        message.message.created_at.timestamp_millis(),
        message.message.id,
    )
}

fn collapse_assistant_group(
    mut group: Vec<Message>,
    updated_at: DateTime<Utc>,
) -> VisibleMessageRecord {
    let mut visible = group
        .first()
        .cloned()
        .expect("assistant group should contain at least one message");
    let usage = aggregate_usage(group.iter().filter_map(|message| message.usage.as_ref()));
    visible.usage = usage.clone();
    visible.metadata = collapse_assistant_metadata(group.as_slice());
    visible.state = collapse_assistant_state(group.as_slice());

    let mut parts = Vec::new();
    for message in group.drain(..) {
        for mut part in message.parts {
            part.message_id = visible.id;
            part.part_index = parts.len() as i32;
            parts.push(part);
        }
    }

    visible.parts = parts;
    normalize_message_parts(&mut visible);

    VisibleMessageRecord {
        message: visible,
        updated_at,
    }
}

fn aggregate_usage<'a>(usages: impl Iterator<Item = &'a MessageUsage>) -> Option<MessageUsage> {
    let mut total = MessageUsage::default();
    let mut seen = false;
    for usage in usages {
        seen = true;
        total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
        total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
        total.reasoning_tokens = total
            .reasoning_tokens
            .saturating_add(usage.reasoning_tokens);
        total.cache_write_tokens = total
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
        total.cache_read_tokens = total
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        total.total_cost += usage.total_cost;
    }
    seen.then_some(total)
}

fn collapse_assistant_metadata(group: &[Message]) -> agena::message::MessageMetadata {
    let mut metadata = group
        .first()
        .map(|message| message.metadata.clone())
        .unwrap_or_default();
    for message in group.iter().skip(1) {
        metadata.model_provider_id = message.metadata.model_provider_id.clone();
        metadata.model_adapter_id = message.metadata.model_adapter_id.clone();
        metadata.model_id = message.metadata.model_id.clone();
        metadata.model_thinking_mode = message.metadata.model_thinking_mode.clone();
        metadata.model_speed_mode = message.metadata.model_speed_mode.clone();
    }
    metadata
}

fn collapse_assistant_state(group: &[Message]) -> agena::message::MessageStatus {
    group
        .last()
        .map(|message| message.state)
        .unwrap_or(agena::message::MessageStatus::Completed)
}

fn project_part(mut part: MessagePart, mode: PartLoadMode) -> MessagePart {
    if mode == PartLoadMode::Summary {
        // Drop the heavy detail payload — clients in summary mode only consume
        // the part header.
        part.content = None;
    }
    part
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena::message::{ExecutionStatus, Message, MessageMetadata, MessageStatus, PartContent};
    use chrono::{TimeZone, Utc};

    #[test]
    fn project_visible_messages_collapses_consecutive_assistant_runs() {
        let user = message_with_parts(
            10,
            Role::User,
            1_000,
            MessageStatus::Completed,
            vec![part_with_text(100, 10, 1_000, "continue")],
            None,
        );
        let first_assistant = message_with_parts(
            11,
            Role::Assistant,
            2_000,
            MessageStatus::Completed,
            vec![part_with_text(110, 11, 2_000, "step one")],
            Some(MessageUsage {
                input_tokens: 11,
                output_tokens: 7,
                reasoning_tokens: 3,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 0.12,
            }),
        );
        let second_assistant = message_with_parts(
            12,
            Role::Assistant,
            3_000,
            MessageStatus::Completed,
            vec![part_with_text(120, 12, 3_000, "step two")],
            Some(MessageUsage {
                input_tokens: 5,
                output_tokens: 13,
                reasoning_tokens: 2,
                cache_write_tokens: 1,
                cache_read_tokens: 4,
                total_cost: 0.08,
            }),
        );

        let projection = project_visible_messages(vec![user, first_assistant, second_assistant]);

        assert_eq!(projection.messages.len(), 2);
        assert_eq!(projection.hidden_message_aliases.get(&12), Some(&11));

        let visible = &projection.messages[1];
        assert_eq!(visible.message.id, 11);
        assert_eq!(visible.message.role, Role::Assistant);
        assert_eq!(visible.updated_at.timestamp_millis(), 3_000);
        assert_eq!(
            visible.message.usage,
            Some(MessageUsage {
                input_tokens: 16,
                output_tokens: 20,
                reasoning_tokens: 5,
                cache_write_tokens: 1,
                cache_read_tokens: 4,
                total_cost: 0.20,
            })
        );
        assert_eq!(visible.message.parts.len(), 2);
        assert_eq!(visible.message.parts[0].id, 110);
        assert_eq!(visible.message.parts[0].message_id, 11);
        assert_eq!(visible.message.parts[1].id, 120);
        assert_eq!(visible.message.parts[1].message_id, 11);
    }

    #[test]
    fn project_part_drops_content_in_summary_mode() {
        let text = part_with_text(121, 11, 4_000, "final");
        let projected_text = project_part(text, PartLoadMode::Summary);
        assert!(projected_text.content.is_none());
    }

    fn message_with_parts(
        id: i64,
        role: Role,
        created_at_ms: i64,
        state: MessageStatus,
        parts: Vec<MessagePart>,
        usage: Option<MessageUsage>,
    ) -> Message {
        Message {
            id,
            role,
            state,
            parts,
            created_at: ts(created_at_ms),
            metadata: MessageMetadata::default(),
            provider_state: None,
            usage,
        }
    }

    fn part_with_text(id: i64, message_id: i64, created_at_ms: i64, text: &str) -> MessagePart {
        MessagePart::with_content(
            id,
            message_id,
            ts(created_at_ms),
            ExecutionStatus::Completed,
            PartContent::text(text),
        )
    }

    fn ts(timestamp_millis: i64) -> chrono::DateTime<Utc> {
        Utc.timestamp_millis_opt(timestamp_millis)
            .single()
            .expect("valid timestamp")
    }
}
