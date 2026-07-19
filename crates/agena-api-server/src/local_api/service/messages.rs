use super::{
    ApiError, ApiResult, ApiService, DateTime, HashMap, Message, MessageCursor, MessageListQuery,
    MessageResource, PageOrder, PaginatedResponse, PartLoadMode, SessionManager, Utc,
    api_error_from_app, build_page, decode_cursor, normalize_limit,
};
use agena::message::MessagePart;

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
    /// Provider rounds represented by this visible conversation message.
    source_message_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct VisibleMessageProjection {
    messages: Vec<VisibleMessageRecord>,
}

impl VisibleMessageProjection {
    fn find_message(&self, message_id: i64) -> Option<&VisibleMessageRecord> {
        self.messages
            .iter()
            .find(|message| message.source_message_ids.contains(&message_id))
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
        let part_count = message
            .source_message_ids
            .iter()
            .filter_map(|message_id| header_counts.get(message_id))
            .copied()
            .fold(0_u64, u64::saturating_add);
        counts.insert(message.message.id, part_count);
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
    let mut visible = Vec::<VisibleMessageRecord>::new();
    for mut message in messages {
        normalize_message_parts(&mut message);
        if message.role == agena::role::Role::Assistant
            && message.metadata.turn_id.is_some()
            && let Some(existing) = visible
                .iter_mut()
                .find(|existing| same_visible_assistant_turn(&existing.message, &message))
        {
            merge_assistant_provider_round(existing, message);
            continue;
        }
        let message_id = message.id;
        visible.push(VisibleMessageRecord {
            updated_at: message.created_at,
            message,
            source_message_ids: vec![message_id],
        });
    }
    VisibleMessageProjection { messages: visible }
}

fn same_visible_assistant_turn(existing: &Message, next: &Message) -> bool {
    existing.role == agena::role::Role::Assistant
        && existing.metadata.turn_id == next.metadata.turn_id
        // Changing the model route in the middle of an interactive pause is
        // an explicit boundary: preserve separate assistant attribution.
        && existing.metadata.model_provider_id == next.metadata.model_provider_id
        && existing.metadata.model_adapter_id == next.metadata.model_adapter_id
        && existing.metadata.model_id == next.metadata.model_id
}

fn merge_assistant_provider_round(existing: &mut VisibleMessageRecord, mut next: Message) {
    existing.updated_at = existing.updated_at.max(next.created_at);
    existing.message.state = next.state;
    existing.message.provider_state = next.provider_state.take();
    merge_usage(&mut existing.message.usage, next.usage.take());
    existing.message.parts.append(&mut next.parts);
    existing.source_message_ids.push(next.id);
    normalize_message_parts(&mut existing.message);
}

fn merge_usage(
    existing: &mut Option<agena::message::MessageUsage>,
    next: Option<agena::message::MessageUsage>,
) {
    let Some(next) = next else {
        return;
    };
    let total = existing.get_or_insert_with(Default::default);
    total.input_tokens = total.input_tokens.saturating_add(next.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(next.output_tokens);
    total.reasoning_tokens = total.reasoning_tokens.saturating_add(next.reasoning_tokens);
    total.cache_write_tokens = total
        .cache_write_tokens
        .saturating_add(next.cache_write_tokens);
    total.cache_read_tokens = total
        .cache_read_tokens
        .saturating_add(next.cache_read_tokens);
    total.total_cost += next.total_cost;
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

    fn assistant(id: i64, turn_id: Option<i64>, state: agena::message::MessageStatus) -> Message {
        let mut message = Message {
            id,
            role: agena::role::Role::Assistant,
            state,
            parts: Vec::new(),
            created_at: Utc::now(),
            metadata: Default::default(),
            provider_state: None,
            usage: None,
        };
        message.metadata.turn_id = turn_id;
        message.metadata.model_provider_id = "provider".to_owned();
        message.metadata.model_id = "model".to_owned();
        message
    }

    #[test]
    fn consecutive_assistant_rounds_keep_independent_identity_and_state() {
        let projection = project_visible_messages(vec![
            assistant(10, None, agena::message::MessageStatus::Completed),
            assistant(11, None, agena::message::MessageStatus::Cancelled),
        ]);

        assert_eq!(projection.messages.len(), 2);
        assert_eq!(projection.messages[0].message.id, 10);
        assert_eq!(
            projection.messages[0].message.state,
            agena::message::MessageStatus::Completed
        );
        assert_eq!(projection.messages[1].message.id, 11);
        assert_eq!(
            projection.messages[1].message.state,
            agena::message::MessageStatus::Cancelled
        );
    }

    #[test]
    fn provider_rounds_in_one_turn_project_as_one_assistant_message() {
        let mut first = assistant(10, Some(7), agena::message::MessageStatus::Completed);
        first.parts = Message::prompt_text(agena::role::Role::Assistant, "tool round").parts;
        let mut second = assistant(11, Some(7), agena::message::MessageStatus::Completed);
        second.parts = Message::prompt_text(agena::role::Role::Assistant, "final answer").parts;

        let projection = project_visible_messages(vec![first, second]);

        assert_eq!(projection.messages.len(), 1);
        let visible = &projection.messages[0];
        assert_eq!(visible.message.id, 10);
        assert_eq!(visible.source_message_ids, vec![10, 11]);
        assert_eq!(
            projection.find_message(11).map(|record| record.message.id),
            Some(10)
        );
        assert_eq!(visible.message.parts.len(), 2);
        assert!(
            visible
                .message
                .parts
                .iter()
                .all(|part| part.message_id == 10)
        );
    }

    #[test]
    fn changing_model_route_starts_a_separate_visible_assistant_message() {
        let first = assistant(10, Some(7), agena::message::MessageStatus::Completed);
        let mut second = assistant(11, Some(7), agena::message::MessageStatus::Completed);
        second.metadata.model_id = "other-model".to_owned();

        let projection = project_visible_messages(vec![first, second]);

        assert_eq!(projection.messages.len(), 2);
    }
}
