use super::{
    ApplicationError, ApplicationResult, ApplicationService, DateTime, HashMap, MessageCursor,
    MessageListQuery, MessageResource, PageOrder, PaginatedResponse, PartLoadMode, Utc, build_page,
    decode_cursor, normalize_limit,
};
use agena_api::message_part::{
    MessageAttachmentPartResource, MessageErrorPartResource, MessagePartDetailResource,
    MessagePartKindResource, MessagePartResource, MessageReasoningPartResource,
    MessageRequestPartResource, MessageTextPartResource, PartExecutionStatusResource,
};
use agena_api::resource::{MessageMetadata, MessageRole, MessageStatus, MessageUsage};
use agena_domain::{ExecutionStatus, PartKind, ToolResultState};
use agena_runtime::{
    SessionProjectedMessage, SessionProjectedMessageHeader, SessionProjectedPartDetail,
    SessionQueryService,
};

impl ApplicationService {
    pub async fn list_messages(
        &self,
        queries: &dyn SessionQueryService,
        session_id: i64,
        query: MessageListQuery,
    ) -> ApplicationResult<PaginatedResponse<MessageResource>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(query.pagination.limit());
        let cursor = query
            .pagination
            .cursor()
            .map(decode_cursor::<MessageCursor>)
            .transpose()?;
        if query.parts == PartLoadMode::None {
            return list_message_headers(queries, session_id, cursor, limit).await;
        }
        if query.parts == PartLoadMode::Summary {
            let visible =
                load_visible_message_projection_from_queries(queries, session_id, false).await?;
            return paginated_message_projection(visible, session_id, cursor, limit, query.parts)
                .await;
        }
        let visible =
            load_visible_message_projection_from_queries(queries, session_id, true).await?;
        paginated_message_projection(visible, session_id, cursor, limit, query.parts).await
    }

    pub async fn get_message(
        &self,
        queries: &dyn SessionQueryService,
        message_id: i64,
        parts: PartLoadMode,
    ) -> ApplicationResult<Option<MessageResource>> {
        if parts == PartLoadMode::None {
            let Some(session_id) = queries
                .find_session_id_for_message(message_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?
            else {
                return Ok(None);
            };
            let visible = project_visible_message_headers(
                session_id,
                queries
                    .list_projected_message_headers(session_id)
                    .await
                    .map_err(|error| ApplicationError::internal(error.to_string()))?,
            )?;
            return Ok(visible
                .iter()
                .find(|message| message.source_message_ids.contains(&message_id))
                .map(|message| message.resource.clone()));
        }
        if parts == PartLoadMode::Summary {
            let Some(session_id) = queries
                .find_session_id_for_message(message_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?
            else {
                return Ok(None);
            };
            let visible =
                load_visible_message_projection_from_queries(queries, session_id, false).await?;
            let part_counts = part_counts_from_visible_projection(&visible);
            return Ok(visible.find_message(message_id).map(|message| {
                message_resource_from_message(
                    session_id,
                    message,
                    parts,
                    visible_part_count(&part_counts, message),
                )
            }));
        }
        let Some(session_id) = queries
            .find_session_id_for_message(message_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Ok(None);
        };
        let visible =
            load_visible_message_projection_from_queries(queries, session_id, true).await?;
        let part_counts = part_counts_from_visible_projection(&visible);
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
        queries: &dyn SessionQueryService,
        message_id: i64,
        mode: PartLoadMode,
    ) -> ApplicationResult<Vec<MessagePartResource>> {
        if mode == PartLoadMode::None {
            return queries
                .find_session_id_for_message(message_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?
                .map(|_| Vec::new())
                .ok_or_else(|| {
                    ApplicationError::not_found(format!("message not found: {message_id}"))
                });
        }
        if mode == PartLoadMode::Summary {
            let Some(session_id) = queries
                .find_session_id_for_message(message_id)
                .await
                .map_err(|error| ApplicationError::internal(error.to_string()))?
            else {
                return Err(ApplicationError::not_found(format!(
                    "message not found: {message_id}"
                )));
            };
            let visible =
                load_visible_message_projection_from_queries(queries, session_id, false).await?;
            let Some(message) = visible.find_message(message_id) else {
                return Err(ApplicationError::not_found(format!(
                    "message not found: {message_id}"
                )));
            };
            return Ok(message
                .message
                .parts
                .iter()
                .map(|part| message_part_resource_from_runtime(part, mode))
                .collect());
        }
        let Some(session_id) = queries
            .find_session_id_for_message(message_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Err(ApplicationError::not_found(format!(
                "message not found: {message_id}"
            )));
        };
        let visible =
            load_visible_message_projection_from_queries(queries, session_id, true).await?;
        let Some(message) = visible.find_message(message_id) else {
            return Err(ApplicationError::not_found(format!(
                "message not found: {message_id}"
            )));
        };
        Ok(message
            .message
            .parts
            .iter()
            .map(|part| message_part_resource_from_runtime(part, mode))
            .collect())
    }

    pub async fn get_message_part(
        &self,
        queries: &dyn SessionQueryService,
        part_id: i64,
    ) -> ApplicationResult<Option<MessagePartResource>> {
        let Some(session_id) = queries
            .find_session_id_for_part(part_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?
        else {
            return Ok(None);
        };
        let visible =
            load_visible_message_projection_from_queries(queries, session_id, true).await?;
        Ok(visible
            .find_part(part_id)
            .map(|part| message_part_resource_from_runtime(part, PartLoadMode::Full)))
    }
}

async fn list_message_headers(
    queries: &dyn SessionQueryService,
    session_id: i64,
    cursor: Option<MessageCursor>,
    limit: u64,
) -> ApplicationResult<PaginatedResponse<MessageResource>> {
    let headers = queries
        .list_projected_message_headers(session_id)
        .await
        .map_err(|error| ApplicationError::internal(error.to_string()))?;
    let visible = project_visible_message_headers(session_id, headers)?;
    let (items, has_more, next_cursor) = paginate_visible_message_headers(&visible, cursor, limit);
    build_page(
        items,
        has_more,
        next_cursor.map(|(created_at_ms, id)| MessageCursor { created_at_ms, id }),
        PageOrder::Asc,
        limit,
    )
}

fn project_visible_message_headers(
    session_id: i64,
    headers: Vec<SessionProjectedMessageHeader>,
) -> ApplicationResult<Vec<VisibleMessageHeader>> {
    let mut visible = Vec::<VisibleMessageHeader>::new();
    for header in headers {
        let metadata =
            serde_json::from_value::<MessageMetadata>(header.metadata).map_err(|error| {
                ApplicationError::internal(format!(
                    "invalid session message metadata projection: {error}"
                ))
            })?;
        let usage = header
            .usage
            .map(serde_json::from_value::<MessageUsage>)
            .transpose()
            .map_err(|error| {
                ApplicationError::internal(format!(
                    "invalid session message usage projection: {error}"
                ))
            })?;
        let resource = MessageResource {
            id: header.id,
            session_id,
            role: message_role_from_domain(header.role),
            state: message_status_from_domain(header.state),
            created_at: header.created_at,
            updated_at: header.created_at,
            metadata,
            usage,
            part_count: header.part_count,
            parts: None,
        };
        if resource.role == MessageRole::Assistant
            && resource.metadata.turn_id.is_some()
            && let Some(existing) = visible.iter_mut().find(|existing| {
                existing.resource.role == MessageRole::Assistant
                    && existing.resource.metadata.turn_id == resource.metadata.turn_id
                    && existing.resource.metadata.model_provider_id
                        == resource.metadata.model_provider_id
                    && existing.resource.metadata.model_adapter_id
                        == resource.metadata.model_adapter_id
                    && existing.resource.metadata.model_id == resource.metadata.model_id
            })
        {
            existing.resource.updated_at = existing.resource.updated_at.max(resource.created_at);
            existing.resource.state = resource.state;
            merge_message_resource_usage(&mut existing.resource.usage, resource.usage);
            existing.resource.part_count = existing
                .resource
                .part_count
                .saturating_add(resource.part_count);
            existing.source_message_ids.push(resource.id);
            continue;
        }
        visible.push(VisibleMessageHeader {
            source_message_ids: vec![resource.id],
            resource,
        });
    }
    Ok(visible)
}

fn merge_message_resource_usage(existing: &mut Option<MessageUsage>, next: Option<MessageUsage>) {
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

fn paginate_visible_message_headers(
    messages: &[VisibleMessageHeader],
    cursor: Option<MessageCursor>,
    limit: u64,
) -> (Vec<MessageResource>, bool, Option<(i64, i64)>) {
    let mut filtered = messages
        .iter()
        .filter(|message| match cursor {
            Some(cursor) => {
                message_resource_cursor_key(&message.resource) < (cursor.created_at_ms, cursor.id)
            }
            None => true,
        })
        .map(|message| message.resource.clone())
        .collect::<Vec<_>>();
    filtered.sort_by_key(|message| std::cmp::Reverse(message_resource_cursor_key(message)));
    let has_more = filtered.len() > limit as usize;
    filtered.truncate(limit as usize);
    let next_cursor = has_more
        .then(|| filtered.last().map(message_resource_cursor_key))
        .flatten();
    filtered.sort_by_key(message_resource_cursor_key);
    (filtered, has_more, next_cursor)
}

fn message_resource_cursor_key(message: &MessageResource) -> (i64, i64) {
    (message.created_at.timestamp_millis(), message.id)
}

#[derive(Debug, Clone)]
struct VisibleMessageHeader {
    resource: MessageResource,
    source_message_ids: Vec<i64>,
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
                .map(|part| message_part_resource_from_runtime(part, parts_mode))
                .collect(),
        ),
    };
    MessageResource {
        id: message.message.id,
        session_id,
        role: message_role_from_domain(message.message.role),
        state: message_status_from_domain(message.message.state),
        created_at: message.message.created_at,
        updated_at: message.updated_at,
        metadata: message.message.metadata.clone(),
        usage: message.message.usage.clone(),
        part_count,
        parts,
    }
}

const fn message_role_from_domain(value: agena_domain::Role) -> MessageRole {
    match value {
        agena_domain::Role::User => MessageRole::User,
        agena_domain::Role::Assistant => MessageRole::Assistant,
        agena_domain::Role::System => MessageRole::System,
        agena_domain::Role::Tool => MessageRole::Tool,
    }
}

const fn message_status_from_domain(value: agena_domain::ExecutionStatus) -> MessageStatus {
    match value {
        agena_domain::ExecutionStatus::Pending => MessageStatus::Pending,
        agena_domain::ExecutionStatus::InProgress => MessageStatus::InProgress,
        agena_domain::ExecutionStatus::Completed => MessageStatus::Completed,
        agena_domain::ExecutionStatus::Failed => MessageStatus::Failed,
        agena_domain::ExecutionStatus::Cancelled => MessageStatus::Cancelled,
    }
}

#[derive(Debug, Clone)]
struct VisibleMessageRecord {
    message: RuntimeMessage,
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

    fn find_part(&self, part_id: i64) -> Option<&agena_runtime::SessionProjectedMessagePart> {
        self.messages
            .iter()
            .find_map(|message| message.message.parts.iter().find(|part| part.id == part_id))
    }
}

async fn load_visible_message_projection_from_queries(
    queries: &dyn SessionQueryService,
    session_id: i64,
    include_content: bool,
) -> ApplicationResult<VisibleMessageProjection> {
    let messages = queries
        .list_projected_messages(session_id, include_content)
        .await
        .map_err(|error| ApplicationError::internal(error.to_string()))?
        .into_iter()
        .map(message_from_runtime_projection)
        .collect::<ApplicationResult<Vec<_>>>()?;
    Ok(project_visible_messages(messages))
}

#[derive(Debug, Clone)]
struct RuntimeMessage {
    id: i64,
    role: agena_domain::Role,
    state: ExecutionStatus,
    created_at: DateTime<Utc>,
    metadata: MessageMetadata,
    usage: Option<MessageUsage>,
    parts: Vec<agena_runtime::SessionProjectedMessagePart>,
}

fn message_from_runtime_projection(
    value: SessionProjectedMessage,
) -> ApplicationResult<RuntimeMessage> {
    let metadata = serde_json::from_value(value.metadata).map_err(|error| {
        ApplicationError::internal(format!(
            "invalid session message metadata projection: {error}"
        ))
    })?;
    let usage = value
        .usage
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| {
            ApplicationError::internal(format!("invalid session message usage projection: {error}"))
        })?;
    Ok(RuntimeMessage {
        id: value.id,
        role: value.role,
        state: value.state,
        created_at: value.created_at,
        metadata,
        usage,
        parts: value.parts,
    })
}

async fn paginated_message_projection(
    visible: VisibleMessageProjection,
    session_id: i64,
    cursor: Option<MessageCursor>,
    limit: u64,
    parts: PartLoadMode,
) -> ApplicationResult<PaginatedResponse<MessageResource>> {
    let part_counts = part_counts_from_visible_projection(&visible);
    let (messages, has_more, next_cursor) =
        paginate_visible_messages(visible.messages.as_slice(), cursor, limit);
    let items = messages
        .iter()
        .map(|message| {
            message_resource_from_message(
                session_id,
                message,
                parts,
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

fn part_counts_from_visible_projection(projection: &VisibleMessageProjection) -> HashMap<i64, u64> {
    projection
        .messages
        .iter()
        .map(|message| (message.message.id, message.message.parts.len() as u64))
        .collect()
}

fn visible_part_count(part_counts: &HashMap<i64, u64>, message: &VisibleMessageRecord) -> u64 {
    part_counts
        .get(&message.message.id)
        .copied()
        .unwrap_or(message.message.parts.len() as u64)
}

fn project_visible_messages(messages: Vec<RuntimeMessage>) -> VisibleMessageProjection {
    let mut visible = Vec::<VisibleMessageRecord>::new();
    for mut message in messages {
        normalize_message_parts(&mut message);
        if message.role == agena_domain::Role::Assistant
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

fn same_visible_assistant_turn(existing: &RuntimeMessage, next: &RuntimeMessage) -> bool {
    existing.role == agena_domain::Role::Assistant
        && existing.metadata.turn_id == next.metadata.turn_id
        // Changing the model route in the middle of an interactive pause is
        // an explicit boundary: preserve separate assistant attribution.
        && existing.metadata.model_provider_id == next.metadata.model_provider_id
        && existing.metadata.model_adapter_id == next.metadata.model_adapter_id
        && existing.metadata.model_id == next.metadata.model_id
}

fn merge_assistant_provider_round(existing: &mut VisibleMessageRecord, mut next: RuntimeMessage) {
    existing.updated_at = existing.updated_at.max(next.created_at);
    existing.message.state = next.state;
    merge_usage(&mut existing.message.usage, next.usage.take());
    existing.message.parts.append(&mut next.parts);
    existing.source_message_ids.push(next.id);
    normalize_message_parts(&mut existing.message);
}

fn merge_usage(existing: &mut Option<MessageUsage>, next: Option<MessageUsage>) {
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

fn normalize_message_parts(message: &mut RuntimeMessage) {
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

/// Maps the stable Runtime transcript projection to the public API resource.
/// Runtime-private aggregates are intentionally not accepted here.
pub fn message_part_resource_from_runtime(
    part: &agena_runtime::SessionProjectedMessagePart,
    mode: PartLoadMode,
) -> MessagePartResource {
    let content = (mode == PartLoadMode::Full)
        .then(|| {
            part.detail.as_ref().and_then(|detail| {
                (!matches!(detail, SessionProjectedPartDetail::Opaque(_)))
                    .then(|| message_part_detail_from_runtime(detail))
            })
        })
        .flatten();
    MessagePartResource {
        id: part.id,
        message_id: part.message_id,
        part_index: part.part_index,
        status: part_execution_status_from_domain(part.status),
        kind: message_part_kind_from_domain(part.kind),
        name: part.name.clone(),
        summary: part.summary.clone(),
        has_detail: part.has_detail,
        operation_id: part.operation_id.clone(),
        created_at: part.created_at,
        content,
    }
}

const fn part_execution_status_from_domain(value: ExecutionStatus) -> PartExecutionStatusResource {
    match value {
        ExecutionStatus::Pending => PartExecutionStatusResource::Pending,
        ExecutionStatus::InProgress => PartExecutionStatusResource::InProgress,
        ExecutionStatus::Completed => PartExecutionStatusResource::Completed,
        ExecutionStatus::Failed => PartExecutionStatusResource::Failed,
        ExecutionStatus::Cancelled => PartExecutionStatusResource::Cancelled,
    }
}

const fn message_part_kind_from_domain(value: PartKind) -> MessagePartKindResource {
    match value {
        PartKind::Text => MessagePartKindResource::Text,
        PartKind::Reasoning => MessagePartKindResource::Reasoning,
        PartKind::Operation => MessagePartKindResource::Operation,
        PartKind::Activity => MessagePartKindResource::Activity,
        PartKind::Attachment => MessagePartKindResource::Attachment,
        PartKind::Request => MessagePartKindResource::Request,
        PartKind::Error => MessagePartKindResource::Error,
    }
}

fn message_part_detail_from_runtime(
    value: &SessionProjectedPartDetail,
) -> MessagePartDetailResource {
    match value {
        SessionProjectedPartDetail::Text { text, synthetic } => {
            MessagePartDetailResource::Text(MessageTextPartResource {
                text: text.clone(),
                synthetic: *synthetic,
            })
        }
        SessionProjectedPartDetail::Reasoning {
            summary,
            raw_content,
            encrypted_content,
        } => MessagePartDetailResource::Reasoning(MessageReasoningPartResource {
            summary: summary.clone(),
            raw_content: raw_content.clone(),
            encrypted_content: encrypted_content.clone(),
        }),
        SessionProjectedPartDetail::Attachment(value) => {
            MessagePartDetailResource::Attachment(MessageAttachmentPartResource {
                attachments: value
                    .attachments
                    .iter()
                    .cloned()
                    .map(message_attachment_from_domain)
                    .collect(),
            })
        }
        SessionProjectedPartDetail::Error { code, message } => {
            MessagePartDetailResource::Error(MessageErrorPartResource {
                code: code.clone(),
                message: message.clone(),
            })
        }
        SessionProjectedPartDetail::Operation(value) => {
            MessagePartDetailResource::Operation(Box::new(operation_part_from_domain(value)))
        }
        SessionProjectedPartDetail::Activity(value) => {
            MessagePartDetailResource::Activity(Box::new(activity_part_from_runtime(value)))
        }
        SessionProjectedPartDetail::PermissionRequest { request, reply } => {
            MessagePartDetailResource::Request(Box::new(message_permission_request_from_runtime(
                request,
                reply.as_ref(),
            )))
        }
        SessionProjectedPartDetail::UserInputRequest { request, reply } => {
            MessagePartDetailResource::Request(Box::new(message_user_input_request_from_runtime(
                request,
                reply.as_ref(),
            )))
        }
        SessionProjectedPartDetail::Opaque(_) => {
            unreachable!("opaque details are omitted by caller")
        }
    }
}

fn activity_part_from_runtime(
    value: &agena_runtime::SessionProjectedActivityPart,
) -> agena_api::message_part::ActivityPartResource {
    use agena_api::message_part::{
        ActivityErrorResource, ActivityKindResource, ActivityPartResource,
        ExecutionFailureKindResource, ExecutionSourceResource, PromptCompactionActivityResource,
        PromptCompactionStrategyResource, PromptCompactionTriggerResource, TimeRangeResource,
    };
    let kind = match &value.kind {
        agena_runtime::SessionProjectedActivityKind::Execution {
            execution_id,
            source,
        } => ActivityKindResource::Execution {
            execution_id: (*execution_id).into(),
            source: match source {
                agena_domain::ExecutionSource::User => ExecutionSourceResource::User,
                agena_domain::ExecutionSource::Continue => ExecutionSourceResource::Continue,
                agena_domain::ExecutionSource::Compaction => ExecutionSourceResource::Compaction,
                agena_domain::ExecutionSource::PermissionReply => {
                    ExecutionSourceResource::PermissionReply
                }
                agena_domain::ExecutionSource::UserInputReply => {
                    ExecutionSourceResource::UserInputReply
                }
            },
        },
        agena_runtime::SessionProjectedActivityKind::Compaction {
            execution_id,
            activity,
        } => ActivityKindResource::Compaction {
            execution_id: (*execution_id).into(),
            activity: PromptCompactionActivityResource {
                checkpoint_id: activity.checkpoint_id.clone(),
                generation: activity.generation,
                compacted_through_message_id: activity.compacted_through_message_id,
                trigger: match activity.trigger {
                    agena_domain::PromptCompactionTrigger::Manual => {
                        PromptCompactionTriggerResource::Manual
                    }
                    agena_domain::PromptCompactionTrigger::Auto => {
                        PromptCompactionTriggerResource::Auto
                    }
                    agena_domain::PromptCompactionTrigger::Reactive => {
                        PromptCompactionTriggerResource::Reactive
                    }
                },
                strategy: match activity.strategy {
                    agena_domain::PromptCompactionStrategy::LocalSummary => {
                        PromptCompactionStrategyResource::LocalSummary
                    }
                    agena_domain::PromptCompactionStrategy::OpenAiResponses => {
                        PromptCompactionStrategyResource::OpenAiResponses
                    }
                },
                before_tokens: activity.before_tokens,
                after_tokens: activity.after_tokens,
            },
        },
    };
    ActivityPartResource {
        activity_id: value.activity_id.clone(),
        kind,
        title: value.title.clone(),
        summary: value.summary.clone(),
        error: value.error.as_ref().map(|error| ActivityErrorResource {
            message: error.message.clone(),
            failure_kind: error.failure_kind.map(|kind| match kind {
                agena_domain::ExecutionFailureKind::Provider => {
                    ExecutionFailureKindResource::Provider
                }
                agena_domain::ExecutionFailureKind::Internal => {
                    ExecutionFailureKindResource::Internal
                }
                agena_domain::ExecutionFailureKind::ProcessRestart => {
                    ExecutionFailureKindResource::ProcessRestart
                }
            }),
        }),
        lifecycle: TimeRangeResource {
            start_ms: value.lifecycle.start_ms,
            end_ms: value.lifecycle.end_ms,
        },
    }
}

fn message_attachment_from_domain(
    value: agena_plugin_host::sdk::attachment::AttachmentItem,
) -> agena_api::resource::MessageAttachment {
    use agena_api::resource::{MessageAttachment, MessageAttachmentKind, MessageAttachmentSource};
    MessageAttachment {
        kind: match value.kind {
            agena_plugin_host::sdk::attachment::AttachmentKind::Image => {
                MessageAttachmentKind::Image
            }
            agena_plugin_host::sdk::attachment::AttachmentKind::Audio => {
                MessageAttachmentKind::Audio
            }
            agena_plugin_host::sdk::attachment::AttachmentKind::Video => {
                MessageAttachmentKind::Video
            }
            agena_plugin_host::sdk::attachment::AttachmentKind::Pdf => MessageAttachmentKind::Pdf,
            agena_plugin_host::sdk::attachment::AttachmentKind::File => MessageAttachmentKind::File,
        },
        mime: value.mime,
        source: match value.source {
            agena_plugin_host::sdk::attachment::AttachmentSource::Url { url } => {
                MessageAttachmentSource::Url { url }
            }
            agena_plugin_host::sdk::attachment::AttachmentSource::DataUrl { url } => {
                MessageAttachmentSource::DataUrl { url }
            }
            agena_plugin_host::sdk::attachment::AttachmentSource::Base64 { data } => {
                MessageAttachmentSource::Base64 { data }
            }
            agena_plugin_host::sdk::attachment::AttachmentSource::FileId { file_id } => {
                MessageAttachmentSource::FileId { file_id }
            }
            agena_plugin_host::sdk::attachment::AttachmentSource::LocalPath { path } => {
                MessageAttachmentSource::LocalPath { path }
            }
        },
        filename: value.filename,
        title: value.title,
        size_bytes: value.size_bytes,
        sha256: value.sha256,
        width: value.width,
        height: value.height,
        duration_ms: value.duration_ms,
        page_count: value.page_count,
    }
}

fn operation_part_from_domain(
    value: &agena_runtime::SessionProjectedOperationPart,
) -> agena_api::message_part::OperationPartResource {
    use agena_api::message_part as wire;
    wire::OperationPartResource {
        call_id: value.call_id,
        invocation: wire::ToolInvocationResource {
            gateway_function: value
                .invocation
                .tool_api_function
                .map(tool_gateway_function_from_domain),
            name: value.invocation.name.clone(),
            plugin_name: value.invocation.plugin_name.clone(),
            input: structured_object_from_domain(&value.invocation.input),
        },
        title: value.title.clone(),
        summary: value.summary.clone(),
        model_output: model_visible_output_from_domain(&value.model_output),
        blocks: value
            .blocks
            .iter()
            .map(operation_block_from_domain)
            .collect(),
        artifacts: value.artifacts.iter().map(artifact_from_domain).collect(),
        attachments: value
            .attachments
            .iter()
            .cloned()
            .map(message_attachment_from_domain)
            .collect(),
        details: tool_output_from_domain(&value.details),
        result: tool_result_from_domain(&value.result),
        structured: value.structured.clone(),
        metadata: value.metadata.clone(),
        error: value
            .error
            .as_ref()
            .map(|error| wire::OperationErrorResource {
                message: error.message.clone(),
                code: error.code.clone(),
            }),
        raw: value.raw.clone(),
        lifecycle: wire::TimeRangeResource {
            start_ms: value.lifecycle.start_ms,
            end_ms: value.lifecycle.end_ms,
        },
    }
}

const fn tool_gateway_function_from_domain(
    value: agena_domain::ToolApiFunction,
) -> agena_api::message_part::ToolGatewayFunctionResource {
    use agena_api::message_part::ToolGatewayFunctionResource as Wire;
    match value {
        agena_domain::ToolApiFunction::List => Wire::List,
        agena_domain::ToolApiFunction::Search => Wire::Search,
        agena_domain::ToolApiFunction::Help => Wire::Help,
        agena_domain::ToolApiFunction::Tags => Wire::Tags,
        agena_domain::ToolApiFunction::Call => Wire::Call,
    }
}

fn structured_object_from_domain(
    value: &agena_domain::StructuredObject,
) -> agena_api::message_part::StructuredObjectResource {
    agena_api::message_part::StructuredObjectResource {
        fields: value
            .fields
            .iter()
            .map(|field| agena_api::message_part::StructuredFieldResource {
                name: field.name.clone(),
                value: structured_value_from_domain(&field.value),
            })
            .collect(),
    }
}
fn structured_value_from_domain(
    value: &agena_domain::StructuredValue,
) -> agena_api::message_part::StructuredValueResource {
    use agena_api::message_part::StructuredValueResource as Wire;
    match value {
        agena_domain::StructuredValue::Null => Wire::Null,
        agena_domain::StructuredValue::Boolean { value } => Wire::Boolean { value: *value },
        agena_domain::StructuredValue::Integer { value } => Wire::Integer { value: *value },
        agena_domain::StructuredValue::Number { value } => Wire::Number {
            value: value.clone(),
        },
        agena_domain::StructuredValue::Text { value } => Wire::Text {
            value: value.clone(),
        },
        agena_domain::StructuredValue::Array { items } => Wire::Array {
            items: items.iter().map(structured_value_from_domain).collect(),
        },
        agena_domain::StructuredValue::Object { fields } => Wire::Object {
            fields: fields
                .iter()
                .map(|field| agena_api::message_part::StructuredFieldResource {
                    name: field.name.clone(),
                    value: structured_value_from_domain(&field.value),
                })
                .collect(),
        },
    }
}
fn model_visible_output_from_domain(
    value: &agena_runtime::SessionProjectedModelVisibleOutput,
) -> agena_api::message_part::ModelVisibleOutputResource {
    agena_api::message_part::ModelVisibleOutputResource {
        text: value.text.clone(),
        attachments: value
            .attachments
            .iter()
            .cloned()
            .map(message_attachment_from_domain)
            .collect(),
        truncated: value.truncated,
    }
}
fn tool_output_from_domain(
    value: &agena_domain::ToolOutput,
) -> agena_api::message_part::ToolOutputResource {
    agena_api::message_part::ToolOutputResource {
        payload: structured_object_from_domain(&value.payload),
        managed_outputs: value
            .managed_outputs
            .iter()
            .map(|item| agena_api::message_part::ToolManagedOutputResource {
                path: item.path.clone(),
                size_bytes: item.size_bytes,
                media_type: item.media_type.clone(),
            })
            .collect(),
        truncated: value.truncated,
    }
}
fn tool_result_from_domain(
    value: &agena_runtime::SessionProjectedToolResult,
) -> agena_api::message_part::ToolResultEnvelopeResource {
    use agena_api::message_part as wire;
    wire::ToolResultEnvelopeResource {
        state: match value.state {
            ToolResultState::Pending => wire::ToolResultStateResource::Pending,
            ToolResultState::Running => wire::ToolResultStateResource::Running,
            ToolResultState::Completed => wire::ToolResultStateResource::Completed,
            ToolResultState::Failed => wire::ToolResultStateResource::Failed,
            ToolResultState::Cancelled => wire::ToolResultStateResource::Cancelled,
        },
        structured: value.structured.clone(),
        content: value
            .content
            .iter()
            .map(operation_block_from_domain)
            .collect(),
        model_preview: model_visible_output_from_domain(&value.model_preview),
        managed_outputs: value
            .managed_outputs
            .iter()
            .map(|item| wire::ToolManagedOutputResource {
                path: item.path.clone(),
                size_bytes: item.size_bytes,
                media_type: item.media_type.clone(),
            })
            .collect(),
        display: wire::ToolResultDisplayResource {
            title: value.display.title.clone(),
            summary: value.display.summary.clone(),
        },
        attachments: value
            .attachments
            .iter()
            .cloned()
            .map(message_attachment_from_domain)
            .collect(),
        error: value
            .error
            .as_ref()
            .map(|error| wire::OperationErrorResource {
                message: error.message.clone(),
                code: error.code.clone(),
            }),
        metadata: value.metadata.clone(),
        raw: value.raw.clone(),
    }
}
fn artifact_from_domain(
    value: &agena_domain::ArtifactRef,
) -> agena_api::message_part::ArtifactRefResource {
    agena_api::message_part::ArtifactRefResource {
        uri: value.uri.clone(),
        mime: value.mime.clone(),
        name: value.name.clone(),
        size_bytes: value.size_bytes,
        sha256: value.sha256.clone(),
    }
}
fn operation_block_from_domain(
    value: &agena_runtime::SessionProjectedOperationBlock,
) -> agena_api::message_part::OperationBlockResource {
    use agena_api::message_part as wire;
    match value {
        agena_runtime::SessionProjectedOperationBlock::Text { text } => {
            wire::OperationBlockResource::Text { text: text.clone() }
        }
        agena_runtime::SessionProjectedOperationBlock::Markdown { text } => {
            wire::OperationBlockResource::Markdown { text: text.clone() }
        }
        agena_runtime::SessionProjectedOperationBlock::Json { value } => {
            wire::OperationBlockResource::Json {
                value: value.clone(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::Table { columns, rows } => {
            wire::OperationBlockResource::Table {
                columns: columns
                    .iter()
                    .map(|column| wire::TableColumnResource {
                        key: column.key.clone(),
                        label: column.label.clone(),
                    })
                    .collect(),
                rows: rows.clone(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::Log { stream, text } => {
            wire::OperationBlockResource::Log {
                stream: stream.clone(),
                text: text.clone(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::Command {
            command,
            cwd,
            exit_code,
            stdout,
            stderr,
        } => wire::OperationBlockResource::Command {
            command: command.clone(),
            cwd: cwd.clone(),
            exit_code: *exit_code,
            stdout: stdout.clone(),
            stderr: stderr.clone(),
        },
        agena_runtime::SessionProjectedOperationBlock::Diff { diff, language } => {
            wire::OperationBlockResource::Diff {
                diff: diff.clone(),
                language: language.clone(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::FileChanges { changes } => {
            wire::OperationBlockResource::FileChanges {
                changes: changes
                    .iter()
                    .map(|change| wire::FileChangeRecordResource {
                        path: change.path.clone(),
                        kind: match change.kind {
                            agena_domain::FileChangeKind::Added => {
                                wire::FileChangeKindResource::Added
                            }
                            agena_domain::FileChangeKind::Updated => {
                                wire::FileChangeKindResource::Updated
                            }
                            agena_domain::FileChangeKind::Deleted => {
                                wire::FileChangeKindResource::Deleted
                            }
                            agena_domain::FileChangeKind::Moved => {
                                wire::FileChangeKindResource::Moved
                            }
                        },
                        from_path: change.from_path.clone(),
                    })
                    .collect(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::SearchResults { query, results } => {
            wire::OperationBlockResource::SearchResults {
                query: query.clone(),
                results: results
                    .iter()
                    .map(|result| wire::SearchResultItemResource {
                        title: result.title.clone(),
                        uri: result.uri.clone(),
                        snippet: result.snippet.clone(),
                        score: result.score,
                    })
                    .collect(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::Citation {
            uri,
            title,
            snippet,
        } => wire::OperationBlockResource::Citation {
            uri: uri.clone(),
            title: title.clone(),
            snippet: snippet.clone(),
        },
        agena_runtime::SessionProjectedOperationBlock::Image { mime, url } => {
            wire::OperationBlockResource::Image {
                mime: mime.clone(),
                url: url.clone(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::Audio { mime, url } => {
            wire::OperationBlockResource::Audio {
                mime: mime.clone(),
                url: url.clone(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::ResourceLink {
            uri,
            title,
            mime_type,
        } => wire::OperationBlockResource::ResourceLink {
            uri: uri.clone(),
            title: title.clone(),
            mime_type: mime_type.clone(),
        },
        agena_runtime::SessionProjectedOperationBlock::EmbeddedResource {
            uri,
            mime,
            text,
            base64,
        } => wire::OperationBlockResource::EmbeddedResource {
            uri: uri.clone(),
            mime: mime.clone(),
            text: text.clone(),
            base64: base64.clone(),
        },
        agena_runtime::SessionProjectedOperationBlock::File {
            url,
            filename,
            mime,
        } => wire::OperationBlockResource::File {
            url: url.clone(),
            filename: filename.clone(),
            mime: mime.clone(),
        },
        agena_runtime::SessionProjectedOperationBlock::Media {
            mime_type,
            artifact,
        } => wire::OperationBlockResource::Media {
            mime_type: mime_type.clone(),
            artifact: artifact_from_domain(artifact),
        },
        agena_runtime::SessionProjectedOperationBlock::Checklist { items } => {
            wire::OperationBlockResource::Checklist {
                items: items
                    .iter()
                    .map(|item| wire::TodoItemResource {
                        content: item.content.clone(),
                        status: match item.status {
                            agena_domain::TodoStatus::Pending => wire::TodoStatusResource::Pending,
                            agena_domain::TodoStatus::InProgress => {
                                wire::TodoStatusResource::InProgress
                            }
                            agena_domain::TodoStatus::Completed => {
                                wire::TodoStatusResource::Completed
                            }
                            agena_domain::TodoStatus::Cancelled => {
                                wire::TodoStatusResource::Cancelled
                            }
                        },
                        priority: match item.priority {
                            agena_domain::TodoPriority::High => wire::TodoPriorityResource::High,
                            agena_domain::TodoPriority::Medium => {
                                wire::TodoPriorityResource::Medium
                            }
                            agena_domain::TodoPriority::Low => wire::TodoPriorityResource::Low,
                        },
                    })
                    .collect(),
            }
        }
        agena_runtime::SessionProjectedOperationBlock::NestedTask {
            task_id,
            title,
            status,
        } => wire::OperationBlockResource::NestedTask {
            task_id: task_id.clone(),
            title: title.clone(),
            status: part_execution_status_from_domain(*status),
        },
        agena_runtime::SessionProjectedOperationBlock::Progress { message, percent } => {
            wire::OperationBlockResource::Progress {
                message: message.clone(),
                percent: *percent,
            }
        }
        agena_runtime::SessionProjectedOperationBlock::Custom { schema, value } => {
            wire::OperationBlockResource::Custom {
                schema: schema.clone(),
                value: value.clone(),
            }
        }
    }
}

fn message_permission_request_from_runtime(
    request: &agena_domain::PermissionRequest,
    reply: Option<&agena_domain::PermissionReply>,
) -> MessageRequestPartResource {
    use agena_api::resource as wire;
    let wire::PendingInteractiveRequest::Permission { request } =
        super::execution::pending_interactive_request_from_domain(
            agena_domain::PendingInteractiveRequest::Permission {
                request: request.clone(),
            },
        )
    else {
        unreachable!("permission request maps to permission wire request")
    };
    MessageRequestPartResource::Permission {
        request,
        reply: reply.map(|reply| wire::PermissionReply {
            request_id: reply.request_id.clone(),
            kind: match reply.kind {
                agena_domain::PermissionReplyKind::AllowOnce => {
                    wire::PermissionReplyKind::AllowOnce
                }
                agena_domain::PermissionReplyKind::AllowAlways => {
                    wire::PermissionReplyKind::AllowAlways
                }
                agena_domain::PermissionReplyKind::DenyOnce => wire::PermissionReplyKind::DenyOnce,
                agena_domain::PermissionReplyKind::DenyAlways => {
                    wire::PermissionReplyKind::DenyAlways
                }
            },
            reason: reply.reason.clone(),
            scope: reply
                .scope
                .map(super::execution::permission_scope_from_domain),
        }),
    }
}

fn message_user_input_request_from_runtime(
    request: &agena_domain::UserInputRequest,
    reply: Option<&agena_domain::UserInputReply>,
) -> MessageRequestPartResource {
    use agena_api::resource as wire;
    let wire::PendingInteractiveRequest::UserInput { request } =
        super::execution::pending_interactive_request_from_domain(
            agena_domain::PendingInteractiveRequest::UserInput {
                request: request.clone(),
            },
        )
    else {
        unreachable!("user-input request maps to user-input wire request")
    };
    MessageRequestPartResource::UserInput {
        request,
        reply: reply.map(|reply| wire::UserInputReply {
            request_id: reply.request_id.clone(),
            kind: match reply.kind {
                agena_domain::UserInputReplyKind::Submit => wire::UserInputReplyKind::Submit,
                agena_domain::UserInputReplyKind::Cancel => wire::UserInputReplyKind::Cancel,
                agena_domain::UserInputReplyKind::Timeout => wire::UserInputReplyKind::Timeout,
            },
            answers: reply.answers.clone(),
            reason: reply.reason.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_domain::ExecutionStatus;

    fn assistant(
        id: i64,
        turn_id: Option<i64>,
        state: agena_domain::ExecutionStatus,
    ) -> RuntimeMessage {
        RuntimeMessage {
            id,
            role: agena_domain::Role::Assistant,
            state,
            parts: Vec::new(),
            created_at: Utc::now(),
            metadata: MessageMetadata {
                source: agena_api::resource::MessageSource::Assistant,
                idempotency_key: None,
                turn_id,
                parent_message_id: None,
                generated_by_call_id: None,
                model_provider_id: "provider".to_owned(),
                model_adapter_id: None,
                model_id: "model".to_owned(),
                model_thinking_mode: None,
                model_speed_mode: None,
            },
            usage: None,
        }
    }

    #[test]
    fn consecutive_assistant_rounds_keep_independent_identity_and_state() {
        let projection = project_visible_messages(vec![
            assistant(10, None, agena_domain::ExecutionStatus::Completed),
            assistant(11, None, agena_domain::ExecutionStatus::Cancelled),
        ]);

        assert_eq!(projection.messages.len(), 2);
        assert_eq!(projection.messages[0].message.id, 10);
        assert_eq!(
            projection.messages[0].message.state,
            agena_domain::ExecutionStatus::Completed
        );
        assert_eq!(projection.messages[1].message.id, 11);
        assert_eq!(
            projection.messages[1].message.state,
            agena_domain::ExecutionStatus::Cancelled
        );
    }

    #[test]
    fn provider_rounds_in_one_turn_project_as_one_assistant_message() {
        let mut first = assistant(10, Some(7), agena_domain::ExecutionStatus::Completed);
        first.parts = vec![runtime_text_part(10, 0, "tool round")];
        let mut second = assistant(11, Some(7), agena_domain::ExecutionStatus::Completed);
        second.parts = vec![runtime_text_part(11, 0, "final answer")];

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
        let first = assistant(10, Some(7), agena_domain::ExecutionStatus::Completed);
        let mut second = assistant(11, Some(7), agena_domain::ExecutionStatus::Completed);
        second.metadata.model_id = "other-model".to_owned();

        let projection = project_visible_messages(vec![first, second]);

        assert_eq!(projection.messages.len(), 2);
    }

    #[test]
    fn header_projection_matches_assistant_round_merging_without_core_messages() {
        let created_at = Utc::now();
        let metadata = serde_json::json!({
            "source": "assistant",
            "turn_id": 7,
            "model_provider_id": "provider",
            "model_id": "model"
        });
        let headers = vec![
            SessionProjectedMessageHeader {
                id: 10,
                role: agena_domain::Role::Assistant,
                state: ExecutionStatus::Completed,
                created_at,
                metadata: metadata.clone(),
                usage: Some(
                    serde_json::to_value(MessageUsage {
                        input_tokens: 1,
                        total_cost: 0.1,
                        ..Default::default()
                    })
                    .expect("serialize usage"),
                ),
                part_count: 2,
            },
            SessionProjectedMessageHeader {
                id: 11,
                role: agena_domain::Role::Assistant,
                state: ExecutionStatus::Completed,
                created_at: created_at + chrono::Duration::seconds(1),
                metadata,
                usage: Some(
                    serde_json::to_value(MessageUsage {
                        output_tokens: 3,
                        total_cost: 0.2,
                        ..Default::default()
                    })
                    .expect("serialize usage"),
                ),
                part_count: 1,
            },
        ];

        let visible = project_visible_message_headers(42, headers).expect("project headers");

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].resource.id, 10);
        assert_eq!(visible[0].source_message_ids, vec![10, 11]);
        assert_eq!(visible[0].resource.session_id, 42);
        assert_eq!(visible[0].resource.part_count, 3);
        assert_eq!(
            visible[0].resource.updated_at,
            created_at + chrono::Duration::seconds(1)
        );
        assert_eq!(
            visible[0]
                .resource
                .usage
                .as_ref()
                .map(|usage| usage.input_tokens),
            Some(1)
        );
        assert_eq!(
            visible[0]
                .resource
                .usage
                .as_ref()
                .map(|usage| usage.output_tokens),
            Some(3)
        );
        assert!(
            (visible[0]
                .resource
                .usage
                .as_ref()
                .map(|usage| usage.total_cost)
                .expect("merged usage")
                - 0.3)
                .abs()
                < f64::EPSILON
        );
        assert!(visible[0].resource.parts.is_none());
    }

    #[test]
    fn full_part_projection_is_typed_and_summary_projection_has_no_detail() {
        let created_at = Utc::now();
        let part = agena_runtime::SessionProjectedMessagePart {
            id: 12,
            message_id: 34,
            part_index: 2,
            status: ExecutionStatus::Completed,
            kind: agena_domain::PartKind::Text,
            name: None,
            summary: None,
            has_detail: true,
            operation_id: None,
            created_at,
            detail: Some(SessionProjectedPartDetail::Text {
                text: "typed wire projection".to_owned(),
                synthetic: true,
            }),
            content: None,
        };

        let full = message_part_resource_from_runtime(&part, PartLoadMode::Full);
        assert_eq!(full.id, 12);
        assert_eq!(full.message_id, 34);
        assert_eq!(full.part_index, 2);
        assert_eq!(full.status, PartExecutionStatusResource::Completed);
        assert!(matches!(
            full.content,
            Some(MessagePartDetailResource::Text(MessageTextPartResource { text, synthetic: true })) if text == "typed wire projection"
        ));

        let summary = message_part_resource_from_runtime(&part, PartLoadMode::Summary);
        assert!(summary.content.is_none());
        assert!(summary.has_detail);
    }

    #[test]
    fn runtime_full_projection_decodes_detail_at_application_boundary() {
        let created_at = Utc::now();
        let projected = SessionProjectedMessage {
            id: 34,
            role: agena_domain::Role::Assistant,
            state: ExecutionStatus::Completed,
            created_at,
            metadata: serde_json::json!({"source":"assistant", "model_provider_id":"provider", "model_id":"model"}),
            usage: None,
            parts: vec![agena_runtime::SessionProjectedMessagePart {
                id: 12,
                message_id: 34,
                part_index: 0,
                status: ExecutionStatus::Completed,
                kind: agena_domain::PartKind::Text,
                name: None,
                summary: Some("runtime detail".to_owned()),
                has_detail: true,
                operation_id: None,
                created_at,
                detail: Some(agena_runtime::SessionProjectedPartDetail::Text {
                    text: "runtime detail".to_owned(),
                    synthetic: false,
                }),
                content: None,
            }],
        };

        let message = message_from_runtime_projection(projected).expect("decode runtime");
        let resource = message_part_resource_from_runtime(
            message.parts.first().expect("projected part"),
            PartLoadMode::Full,
        );
        assert!(matches!(
            resource.content,
            Some(MessagePartDetailResource::Text(MessageTextPartResource { text, .. })) if text == "runtime detail"
        ));
    }

    fn runtime_text_part(
        message_id: i64,
        part_index: i32,
        text: &str,
    ) -> agena_runtime::SessionProjectedMessagePart {
        agena_runtime::SessionProjectedMessagePart {
            id: message_id * 100 + i64::from(part_index),
            message_id,
            part_index,
            status: ExecutionStatus::Completed,
            kind: agena_domain::PartKind::Text,
            name: None,
            summary: Some(text.to_owned()),
            has_detail: true,
            operation_id: None,
            created_at: Utc::now(),
            detail: Some(SessionProjectedPartDetail::Text {
                text: text.to_owned(),
                synthetic: false,
            }),
            content: None,
        }
    }
}
