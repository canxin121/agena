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
}

#[derive(Debug, Clone)]
struct VisibleMessageProjection {
    messages: Vec<VisibleMessageRecord>,
}

impl VisibleMessageProjection {
    fn find_message(&self, message_id: i64) -> Option<&VisibleMessageRecord> {
        self.messages
            .iter()
            .find(|message| message.message.id == message_id)
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
    VisibleMessageProjection {
        messages: messages
            .into_iter()
            .map(|mut message| {
                normalize_message_parts(&mut message);
                VisibleMessageRecord {
                    updated_at: message.created_at,
                    message,
                }
            })
            .collect(),
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

    fn assistant(id: i64, state: agena::message::MessageStatus) -> Message {
        Message {
            id,
            role: agena::role::Role::Assistant,
            state,
            parts: Vec::new(),
            created_at: Utc::now(),
            metadata: Default::default(),
            provider_state: None,
            usage: None,
        }
    }

    #[test]
    fn consecutive_assistant_rounds_keep_independent_identity_and_state() {
        let projection = project_visible_messages(vec![
            assistant(10, agena::message::MessageStatus::Completed),
            assistant(11, agena::message::MessageStatus::Cancelled),
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
}
use super::{
    ApiError, ApiResult, ApiService, DateTime, HashMap, Message, MessageCursor, MessageListQuery,
    MessageResource, PageOrder, PaginatedResponse, PartLoadMode, SessionManager, Utc,
    api_error_from_app, build_page, decode_cursor, normalize_limit,
};
