use super::*;

impl ApiService {
    pub async fn list_messages(
        &self,
        manager: &SessionManager,
        session_id: i64,
        query: MessageListQuery,
    ) -> ApiResult<PaginatedResponse<MessageResource>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(query.limit);
        let cursor = query
            .cursor
            .as_deref()
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
    message: &Message,
    parts_mode: PartLoadMode,
    part_count: u64,
) -> MessageResource {
    let parts = match parts_mode {
        PartLoadMode::None => None,
        PartLoadMode::Summary | PartLoadMode::Full => Some(
            message
                .parts
                .iter()
                .cloned()
                .map(|p| project_part(p, parts_mode))
                .collect(),
        ),
    };
    MessageResource {
        id: message.id,
        session_id,
        role: visible_message_role(message.role),
        state: message.state,
        created_at: message.created_at,
        // The append-only event log carries no separate "updated_at" — every
        // message in `Session.messages` is in its terminal projected form.
        updated_at: message.created_at,
        metadata: message.metadata.clone(),
        usage: message.usage.clone(),
        finish: message.finish.clone(),
        part_count,
        parts,
    }
}

#[derive(Debug, Clone)]
struct VisibleMessageProjection {
    messages: Vec<Message>,
    hidden_message_aliases: HashMap<i64, i64>,
}

impl VisibleMessageProjection {
    fn find_message(&self, message_id: i64) -> Option<&Message> {
        let visible_id = self
            .hidden_message_aliases
            .get(&message_id)
            .copied()
            .unwrap_or(message_id);
        self.messages
            .iter()
            .find(|message| message.id == visible_id)
    }

    fn find_part(&self, part_id: i64) -> Option<MessagePart> {
        self.messages.iter().find_map(|message| {
            message
                .parts
                .iter()
                .find(|part| part.id == part_id)
                .cloned()
        })
    }
}

fn visible_message_role(role: agena::role::Role) -> agena_api::resource::MessageRole {
    match role {
        agena::role::Role::User => agena_api::resource::MessageRole::User,
        agena::role::Role::Assistant => agena_api::resource::MessageRole::Assistant,
        agena::role::Role::System => agena_api::resource::MessageRole::System,
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
        if let Some(part_count) = header_counts.get(&message.id).copied() {
            counts.insert(message.id, part_count);
        }
    }

    for (hidden_message_id, visible_message_id) in &projection.hidden_message_aliases {
        if let Some(part_count) = header_counts.get(hidden_message_id).copied() {
            *counts.entry(*visible_message_id).or_default() += part_count;
        }
    }

    for message in &projection.messages {
        let loaded_part_count = message.parts.len() as u64;
        let count = counts.entry(message.id).or_default();
        *count = (*count).max(loaded_part_count);
    }

    Ok(counts)
}

fn visible_part_count(part_counts: &HashMap<i64, u64>, message: &Message) -> u64 {
    part_counts
        .get(&message.id)
        .copied()
        .unwrap_or(message.parts.len() as u64)
}

fn project_visible_messages(messages: Vec<Message>) -> VisibleMessageProjection {
    let mut visible = Vec::with_capacity(messages.len());

    for mut message in messages {
        normalize_message_parts(&mut message);
        visible.push(message);
    }

    VisibleMessageProjection {
        messages: visible,
        hidden_message_aliases: HashMap::new(),
    }
}

fn normalize_message_parts(message: &mut Message) {
    for (index, part) in message.parts.iter_mut().enumerate() {
        part.message_id = message.id;
        part.part_index = index as i32;
    }
}

fn paginate_visible_messages(
    messages: &[Message],
    cursor: Option<MessageCursor>,
    limit: u64,
) -> (Vec<Message>, bool, Option<(i64, i64)>) {
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

fn message_cursor_key(message: &Message) -> (i64, i64) {
    (message.created_at.timestamp_millis(), message.id)
}

fn project_part(mut part: MessagePart, mode: PartLoadMode) -> MessagePart {
    if mode == PartLoadMode::Summary {
        // Drop the heavy detail payload — clients in summary mode only consume
        // the part header.
        part.content = None;
    }
    part
}
