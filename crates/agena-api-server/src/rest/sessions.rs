use crate::session_support::{
    session_execution_reply_request, session_execution_request, session_execution_resource,
    session_permission_reply_request, session_user_message_request,
};

async fn session_execution_json(
    state: &AppState,
    manager: &agena::session::SessionManager,
    session: &agena::session::Session,
) -> Result<Json<crate::local_api::SessionExecutionResource>, ServerError> {
    Ok(Json(
        session_execution_resource(state, manager, session).await?,
    ))
}

async fn session_execution_json_result(
    state: &AppState,
    manager: &agena::session::SessionManager,
    future: impl Future<Output = Result<agena::session::Session, agena::AppError>>,
) -> Result<Json<crate::local_api::SessionExecutionResource>, ServerError> {
    let session = future.await.map_err(ServerError::Core)?;
    session_execution_json(state, manager, &session).await
}

async fn assert_if_match_session_version(
    state: &AppState,
    session_id: i64,
    headers: &HeaderMap,
) -> Result<(), ServerError> {
    let Some(expected_version) = if_match_version(headers)? else {
        return Ok(());
    };
    state
        .service()
        .assert_session_version(session_id, expected_version)
        .await
        .map_err(server_error_from_http)?;
    Ok(())
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SessionListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().list_sessions(query)).await
}

pub async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    json_http_found(state.service().get_session(session_id), || {
        format!("session not found: {session_id}")
    })
    .await
}

pub async fn get_session_state(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    session_execution_json_result(&state, manager.as_ref(), manager.get_session(session_id)).await
}

#[tracing::instrument(skip_all)]
pub async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<SessionCreateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    json_http(state.service().create_session(request)).await
}

pub async fn replace_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionHierarchyRequest>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    json_http(state.service().replace_session(session_id, request)).await
}

pub async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    json_http(state.service().delete_session(session_id)).await
}

pub async fn list_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionEventListCompatQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    if let Some(after_seq) = query.after_seq {
        let limit = query.limit.unwrap_or(100).clamp(1, 1000);
        let items = state
            .service()
            .list_session_events_after(manager.as_ref(), session_id, after_seq, Some(limit))
            .await
            .map_err(server_error_from_http)?;
        let returned = items.len() as u64;
        let next_cursor = items.last().map(|event| event.meta.seq_global.to_string());
        return Ok(Json(serde_json::json!({
            "items": items,
            "page": {
                "limit": limit,
                "returned": returned,
                "has_more": returned >= limit,
                "next_cursor": next_cursor,
                "order": "asc"
            }
        })));
    }

    let page = state
        .service()
        .list_session_events(
            manager.as_ref(),
            session_id,
            crate::local_api::CursorPaginationQuery {
                cursor: query.cursor,
                limit: query.limit,
            },
        )
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(serde_json::to_value(page).map_err(|error| {
        ServerError::Internal(error.to_string())
    })?))
}

pub async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionEventStreamQuery>,
) -> Result<impl IntoResponse, ServerError> {
    use agena::event::{EventFilter, Scope, bus::SubscriptionItem};

    let manager = state.session_manager()?;
    // A selected parent exposes pending interactive requests and subtask
    // lifecycle from its whole descendant tree. Subscribe before reading the
    // backfill so events published during that read remain queued.
    let bus = manager.event_bus();
    let mut subscription = bus.subscribe(EventFilter::new(Scope::Global));
    let service = state.service().clone();
    let backfill_after = query.after_seq.unwrap_or(0);
    let backfill_limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let initial = service
        .list_session_events_after(
            manager.as_ref(),
            session_id,
            backfill_after,
            Some(backfill_limit),
        )
        .await
        .map_err(server_error_from_http)?;

    let stream = stream! {
        for event in &initial {
            match Event::default()
                .event("session_event")
                .id(event.meta.seq_global.to_string())
                .json_data(event)
            {
                Ok(ev) => yield Ok::<Event, Infallible>(ev),
                Err(error) => {
                    yield Ok::<Event, Infallible>(sse_error_event(format!("failed to encode session event: {error}")));
                    return;
                }
            }
        }
        let mut last_seen = initial
            .last()
            .map(|event| event.meta.seq_global)
            .unwrap_or(backfill_after);

        loop {
            match subscription.recv().await {
                Some(SubscriptionItem::Event(arc_event)) => {
                    if arc_event.meta.seq_global <= last_seen {
                        continue;
                    }
                    let event_name = if arc_event.meta.session_id == Some(session_id) {
                        "session_event"
                    } else {
                        let Some(descendant_id) = arc_event.meta.session_id else {
                            continue;
                        };
                        if !arc_event.kind.invalidates_ancestor_projection()
                            || !is_descendant_session(manager.as_ref(), descendant_id, session_id).await
                        {
                            continue;
                        }
                        // This is deliberately a different SSE event name:
                        // clients must refresh the ancestor projection rather
                        // than merge child transcript data into the parent.
                        "descendant_session_event"
                    };
                    last_seen = arc_event.meta.seq_global;
                    match Event::default()
                        .event(event_name)
                        .id(arc_event.meta.seq_global.to_string())
                        .json_data(arc_event.as_ref())
                    {
                        Ok(ev) => yield Ok::<Event, Infallible>(ev),
                        Err(error) => {
                            yield Ok::<Event, Infallible>(sse_error_event(format!("failed to encode session event: {error}")));
                            return;
                        }
                    }
                }
                Some(SubscriptionItem::Lagged(skipped)) => {
                    yield Ok::<Event, Infallible>(Event::default().event("lagged").data(skipped.to_string()));
                }
                None => return,
            }
        }
    };

    Ok(Sse::new(stream))
}

async fn is_descendant_session(
    manager: &agena::session::SessionManager,
    descendant_id: i64,
    ancestor_id: i64,
) -> bool {
    let mut cursor = Some(descendant_id);
    let mut visited = std::collections::HashSet::new();
    while let Some(session_id) = cursor {
        if !visited.insert(session_id) {
            return false;
        }
        let Ok(session) = manager.get_session(session_id).await else {
            return false;
        };
        let Some(parent_id) = session.parent_id else {
            return false;
        };
        if parent_id == ancestor_id {
            return true;
        }
        cursor = Some(parent_id);
    }
    false
}

pub async fn submit_message(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionMessageRequest>,
) -> Result<impl IntoResponse, ServerError> {
    if request.parts.is_empty() {
        return Err(ServerError::BadRequest(
            "session message requires at least one part".into(),
        ));
    }
    validate_message_attachments(request.parts.as_slice())?;
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let manager = state.session_manager()?;
    let request =
        session_user_message_request(&state, session_id, request.run.options, request.parts)
            .await?;
    session_execution_json_result(
        &state,
        manager.as_ref(),
        manager.submit_user_message(request),
    )
    .await
}

fn validate_message_attachments(parts: &[agena::message::PartContent]) -> Result<(), ServerError> {
    const MAX_ATTACHMENTS: usize = 8;
    const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;
    const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

    let mut count = 0_usize;
    let mut total_bytes = 0_usize;
    for part in parts {
        let agena::message::PartContent::Attachment(attachment) = part else {
            continue;
        };
        count = count.saturating_add(attachment.attachments.len());
        for item in &attachment.attachments {
            let encoded = match &item.source {
                agena::message::AttachmentSource::Base64 { data } => data,
                _ => continue,
            };
            let padding = encoded
                .chars()
                .rev()
                .take_while(|character| *character == '=')
                .count();
            let decoded_bytes = encoded.len().saturating_mul(3) / 4;
            let decoded_bytes = decoded_bytes.saturating_sub(padding.min(2));
            if decoded_bytes > MAX_ATTACHMENT_BYTES {
                return Err(ServerError::BadRequest(
                    "a session attachment exceeds the 50 MiB limit".into(),
                ));
            }
            total_bytes = total_bytes.saturating_add(decoded_bytes);
        }
    }
    if count > MAX_ATTACHMENTS {
        return Err(ServerError::BadRequest(format!(
            "a session message cannot contain more than {MAX_ATTACHMENTS} attachments"
        )));
    }
    if total_bytes > MAX_TOTAL_BYTES {
        return Err(ServerError::BadRequest(
            "session attachments exceed the 64 MiB total limit".into(),
        ));
    }
    Ok(())
}

pub async fn continue_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let manager = state.session_manager()?;
    let request = session_execution_request(&state, session_id, request.options).await?;
    session_execution_json_result(&state, manager.as_ref(), manager.continue_session(request)).await
}

pub async fn compact_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let manager = state.session_manager()?;
    let request = session_execution_request(&state, session_id, request.options).await?;
    session_execution_json_result(&state, manager.as_ref(), manager.compact_session(request)).await
}

pub async fn fork_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionForkRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if request.at_event_seq.is_some() && request.at_message_id.is_none() {
        return Err(ServerError::BadRequest(
            "fork expects at_message_id; at_event_seq is no longer supported".into(),
        ));
    }
    let manager = state.session_manager()?;
    session_execution_json_result(
        &state,
        manager.as_ref(),
        manager.fork_session(agena::session::SessionForkRequest {
            session_id,
            at_message_id: request.at_message_id,
            title: request.title,
            expected_version: if_match_version(&headers)?,
        }),
    )
    .await
}

pub async fn cancel_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_command(
        &state,
        agena_api::commands::Command::CancelRun(agena_api::commands::CancelRunParams {
            session_id,
        }),
    )
    .await?
    {
        agena_api::commands::CommandResult::Ack => Ok(Json(serde_json::json!({ "ok": true }))),
        _ => unreachable!("cancel run returned unexpected result"),
    }
}

pub async fn reply_permission(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplyRequestBody<PermissionReply>>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let manager = state.session_manager()?;
    let request = session_permission_reply_request(
        &state,
        session_id,
        request.run.options,
        request.reply,
        Some("http_api".to_string()),
    )
    .await?;
    session_execution_json_result(&state, manager.as_ref(), manager.reply_permission(request)).await
}

pub async fn reply_user_input(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplyRequestBody<UserInputReply>>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let manager = state.session_manager()?;
    let request =
        session_execution_reply_request(&state, session_id, request.run.options, request.reply)
            .await?;
    session_execution_json_result(&state, manager.as_ref(), manager.reply_user_input(request)).await
}

pub async fn rewind_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRewindRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let expected_version = if_match_version(&headers)?;
    let manager = state.session_manager()?;
    session_execution_json_result(
        &state,
        manager.as_ref(),
        manager.rewind_session(agena::session::SessionRewindRequest {
            session_id,
            message_id: request.message_id,
            expected_version,
        }),
    )
    .await
}

pub async fn list_session_tree(
    State(state): State<AppState>,
    Path(root_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let summaries = manager
        .list_session_tree(root_id)
        .await
        .map_err(ServerError::Core)?;
    let resources: Vec<crate::local_api::dto::SessionResource> =
        summaries.into_iter().map(Into::into).collect();
    Ok(Json(resources))
}

pub async fn export_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let jsonl = manager
        .export_session_jsonl(session_id)
        .await
        .map_err(ServerError::Core)?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
        jsonl,
    ))
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionImportRequestBody {
    pub jsonl: String,
}

pub async fn import_session(
    State(state): State<AppState>,
    Json(request): Json<SessionImportRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    session_execution_json_result(
        &state,
        manager.as_ref(),
        manager.import_session_jsonl(&request.jsonl),
    )
    .await
}
use super::{
    AppState, AxumQuery, Deserialize, Event, Future, HeaderMap, Infallible, IntoResponse, Json,
    Path, PermissionReply, ServerError, SessionCreateRequest, SessionEventListCompatQuery,
    SessionEventStreamQuery, SessionForkRequestBody, SessionHierarchyRequest, SessionListQuery,
    SessionMessageRequest, SessionReplyRequestBody, SessionRewindRequestBody,
    SessionRunRequestBody, Sse, State, UserInputReply, dispatch, if_match_version, json_http,
    json_http_found, server_error_from_http, sse_error_event, stream,
};
