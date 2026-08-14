use agena_application::{
    dto::SessionPermissionUpdateRequest, session::session_resource_from_summary,
};

/// Thin adapter: project a session's execution resource through the shared
/// Application read-back and wrap it in JSON.
async fn session_execution_json_from_id(
    state: &AppState,
    session_id: i64,
) -> Result<Json<agena_application::dto::SessionExecutionResource>, ServerError> {
    Ok(Json(state.session_execution_resource(session_id).await?))
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
        .map_err(server_error_from_application)?;
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
    session_execution_json_from_id(&state, session_id).await
}

pub async fn get_operation_detail(
    State(state): State<AppState>,
    Path((session_id, activity_id)): Path<(i64, String)>,
) -> Result<impl IntoResponse, ServerError> {
    let activity_id: agena_domain::ActivityId = serde_json::from_str(&format!("\"{activity_id}\""))
        .map_err(|error| ServerError::internal(format!("invalid activity id: {error}")))?;
    let services = state.application().session_execution_services()?;
    let detail = services
        .queries
        .as_ref()
        .operation_detail(session_id, activity_id)
        .await
        .map_err(|error| ServerError::internal(error.to_string()))?;
    let resource = detail
        .map(|detail| agena_api::resource::OperationDetailResource {
            activity_id: detail.activity_id,
            markdown: detail.markdown,
            streaming: detail.streaming,
        })
        .unwrap_or(agena_api::resource::OperationDetailResource {
            activity_id,
            markdown: String::new(),
            streaming: false,
        });
    Ok(Json(resource))
}

pub async fn replace_session_permission(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionPermissionUpdateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    let permission = agena_application::permission_config_domain_from_resource(request.permission)?;
    Ok(Json(
        state.set_session_permission(session_id, permission).await?,
    ))
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
    Json(request): Json<SessionUpdateRequest>,
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

pub async fn list_session_parts(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionPartListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let store = state.session_store()?;
    let mut snapshot = crate::live::session_parts(store.as_ref(), session_id).await?;
    if let Some(limit) = query.limit {
        let limit = usize::try_from(limit.clamp(1, 1000)).unwrap_or(1000);
        if snapshot.parts.len() > limit {
            snapshot.parts.drain(..snapshot.parts.len() - limit);
        }
    }
    Ok(Json(snapshot))
}

pub async fn stream_session_changes(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionChangeStreamQuery>,
) -> Result<impl IntoResponse, ServerError> {
    // Subscribe before reading the snapshot so mutations committed during the
    // read remain queued. The snapshot is current state, not replay.
    let mut subscription = crate::live::subscribe(&state)?;
    let store = state.session_store()?;
    let session_queries = state.application().session_query_service()?;
    let initial = crate::live::session_parts(store.as_ref(), session_id).await?;

    let stream = stream! {
        if query.since_version != Some(initial.version) {
            match Event::default().event("session_snapshot").json_data(&initial) {
                Ok(frame) => yield Ok::<Event, Infallible>(frame),
                Err(error) => {
                    yield Ok::<Event, Infallible>(sse_error_event(error));
                    return;
                }
            }
        }

        loop {
            match subscription.recv().await {
                Some(crate::live::LiveItem::SessionChanged(change)) => {
                    let changed_session_id = change.session_id();
                    let frame_name = if changed_session_id == session_id {
                        "session_change"
                    } else {
                        if !session_queries
                                .is_descendant_session(changed_session_id, session_id)
                                .await
                                .unwrap_or(false)
                        {
                            continue;
                        }
                        "descendant_session_change"
                    };
                    match Event::default().event(frame_name).json_data(&change) {
                        Ok(frame) => yield Ok::<Event, Infallible>(frame),
                        Err(error) => {
                            yield Ok::<Event, Infallible>(sse_error_event(error));
                            return;
                        }
                    }
                }
                Some(crate::live::LiveItem::RuntimeSignal(signal)) => {
                    let Some(signal_session_id) = signal.session_id else {
                        continue;
                    };
                    let frame_name = if signal_session_id == session_id {
                        "runtime_signal"
                    } else if session_queries
                        .is_descendant_session(signal_session_id, session_id)
                        .await
                        .unwrap_or(false)
                    {
                        "descendant_runtime_signal"
                    } else {
                        continue;
                    };
                    match Event::default().event(frame_name).json_data(&signal) {
                        Ok(frame) => yield Ok::<Event, Infallible>(frame),
                        Err(error) => {
                            yield Ok::<Event, Infallible>(sse_error_event(error));
                            return;
                        }
                    }
                }
                Some(crate::live::LiveItem::Lagged(skipped)) => {
                    yield Ok::<Event, Infallible>(Event::default().event("lagged").data(skipped.to_string()));
                }
                None => return,
            }
        }
    };

    Ok(Sse::new(stream))
}

pub async fn submit_message(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRunRequest>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    Ok(Json(
        state
            .submit_user_run(session_id, request.document, request.run.options)
            .await?,
    ))
}

pub async fn continue_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    Ok(Json(
        state.continue_session(session_id, request.options).await?,
    ))
}

pub async fn compact_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    Ok(Json(
        state.compact_session(session_id, request.options).await?,
    ))
}

pub async fn fork_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionForkRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if request.at_event_seq.is_some() && request.at_message_id.is_none() {
        return Err(ServerError::bad_request(
            "fork expects at_message_id; at_event_seq is no longer supported",
        ));
    }
    Ok(Json(
        state
            .fork_session(
                session_id,
                request.at_message_id,
                request.title,
                if_match_version(&headers)?,
            )
            .await?,
    ))
}

pub async fn cancel_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(request): Json<CancelRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state.cancel_run(session_id, request.execution_id).await?,
    ))
}

#[derive(Debug, Clone, Deserialize)]
/// Body of a cancel-run request.
pub struct CancelRunRequestBody {
    pub execution_id: agena_domain::ExecutionId,
}

pub async fn reply_permission(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplyRequestBody<PermissionReply>>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    Ok(Json(
        state
            .reply_permission(
                session_id,
                request.run.options,
                request.reply,
                Some("http_api".to_string()),
            )
            .await?,
    ))
}

pub async fn reply_user_input(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplyRequestBody<UserInputReply>>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    Ok(Json(
        state
            .reply_user_input(session_id, request.run.options, request.reply)
            .await?,
    ))
}

pub async fn mark_interactive_request_presented(
    State(state): State<AppState>,
    Path((session_id, request_id)): Path<(i64, String)>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .mark_interactive_request_presented(session_id, request_id)
            .await?,
    ))
}

pub async fn rewind_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRewindRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let expected_version = if_match_version(&headers)?;
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .rewind_session(agena_runtime::SessionRewindRequest {
            session_id,
            turn_id: request.turn_id,
            expected_version,
        })
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    Ok(Json(
        state.session_execution_resource(outcome.session_id).await?,
    ))
}

pub async fn list_session_tree(
    State(state): State<AppState>,
    Path(root_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let services = state.application().session_execution_services()?;
    let summaries = services
        .queries
        .list_session_tree(root_id)
        .await
        .map_err(|error| ServerError::from_failure(*error.failure))?;
    let resources: Vec<agena_application::dto::SessionResource> = summaries
        .into_iter()
        .map(session_resource_from_summary)
        .collect();
    Ok(Json(resources))
}

pub async fn export_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let services = state.application().session_execution_services()?;
    let jsonl = services
        .queries
        .export_session_jsonl(session_id)
        .await
        .map_err(|error| ServerError::from_failure(*error.failure))?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
        jsonl,
    ))
}

#[derive(Debug, Clone, Deserialize)]
/// Body of a session import request.
pub struct SessionImportRequestBody {
    pub jsonl: String,
}

pub async fn import_session(
    State(state): State<AppState>,
    Json(request): Json<SessionImportRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .import_session_jsonl(&request.jsonl)
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    Ok(Json(
        state.session_execution_resource(outcome.session_id).await?,
    ))
}

use super::{
    AppState, AxumQuery, Deserialize, Event, HeaderMap, Infallible, IntoResponse, Json, Path,
    PermissionReply, ServerError, SessionChangeStreamQuery, SessionCreateRequest,
    SessionForkRequestBody, SessionListQuery, SessionPartListQuery, SessionReplyRequestBody,
    SessionRewindRequestBody, SessionRunRequest, SessionRunRequestBody, SessionUpdateRequest, Sse,
    State, UserInputReply, if_match_version, json_http, json_http_found,
    server_error_from_application, sse_error_event, stream,
};
