use agena_application::{
    dto::SessionPermissionUpdateRequest,
    session::{
        session_execution_request, session_execution_resource, session_permission_reply_request,
        session_resource_from_summary, session_user_input_reply_request,
        session_user_message_request,
    },
};

async fn session_execution_json_from_id(
    state: &AppState,
    session_id: i64,
) -> Result<Json<agena_application::dto::SessionExecutionResource>, ServerError> {
    let services = state.application().session_execution_services()?;
    Ok(Json(
        session_execution_resource(
            state.application(),
            services.execution_control.as_ref(),
            services.queries.as_ref(),
            session_id,
        )
        .await?,
    ))
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
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .set_session_permission(session_id, permission)
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    session_execution_json_from_id(&state, outcome.session_id).await
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

pub async fn list_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionEventListCompatQuery>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(after_seq) = query.after_seq {
        let limit = query.limit.unwrap_or(100).clamp(1, 1000);
        let events = state.application().event_query_service()?;
        let items = state
            .service()
            .list_session_events_after(events.as_ref(), session_id, after_seq, Some(limit))
            .await
            .map_err(server_error_from_application)?;
        let returned = items.len() as u64;
        let next_cursor = items.last().map(|event| event.meta.seq_global.to_string());
        return Ok(Json(serde_json::json!({
            "items": items.iter().map(agena_application::event_projection::event_resource_from_runtime).collect::<Vec<_>>(),
            "page": {
                "limit": limit,
                "returned": returned,
                "has_more": returned >= limit,
                "next_cursor": next_cursor,
                "order": "asc"
            }
        })));
    }

    let events = state.application().event_query_service()?;
    let page = state
        .service()
        .list_session_events(
            events.as_ref(),
            session_id,
            agena_application::dto::CursorPaginationQuery {
                cursor: query.cursor,
                limit: query.limit,
            },
        )
        .await
        .map_err(server_error_from_application)?;
    let page = agena_application::pagination::PaginatedResponse {
        items: page
            .items
            .iter()
            .map(agena_application::event_projection::event_resource_from_runtime)
            .collect::<Vec<_>>(),
        page: page.page,
    };
    Ok(Json(serde_json::to_value(page).map_err(|error| {
        ServerError::internal(error.to_string())
    })?))
}

pub async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionEventStreamQuery>,
) -> Result<impl IntoResponse, ServerError> {
    use agena_domain::{EventFilter, EventScope as Scope};
    use agena_runtime::RuntimeLiveEventSubscriptionItem;

    let stream_service = state.event_stream_service()?;
    // A selected parent exposes pending interactive requests and subtask
    // lifecycle from its whole descendant tree. Subscribe before reading the
    // backfill so events published during that read remain queued.
    let mut subscription = stream_service.subscribe_events(EventFilter::new(Scope::Global));
    let service = state.service().clone();
    let backfill_after = query.after_seq.unwrap_or(0);
    let backfill_limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let events = state.application().event_query_service()?;
    let session_queries = state.application().session_query_service()?;
    let initial = service
        .list_session_events_after(
            events.as_ref(),
            session_id,
            backfill_after,
            Some(backfill_limit),
        )
        .await
        .map_err(server_error_from_application)?;

    let stream = stream! {
        for event in &initial {
            let event = agena_application::event_projection::event_resource_from_runtime(event);
            match Event::default()
                .event("session_event")
                .id(event.meta.seq_global.to_string())
                .json_data(&event)
            {
                Ok(ev) => yield Ok::<Event, Infallible>(ev),
                Err(error) => {
                    yield Ok::<Event, Infallible>(sse_error_event(error));
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
                Some(RuntimeLiveEventSubscriptionItem::Event(event)) => {
                    if event.meta.seq_global <= last_seen {
                        continue;
                    }
                    let event_name = if event.meta.session_id == Some(session_id) {
                        "session_event"
                    } else {
                        let Some(descendant_id) = event.meta.session_id else {
                            continue;
                        };
                        if !event.invalidates_ancestor_projection
                            || !session_queries
                                .is_descendant_session(descendant_id, session_id)
                                .await
                                .unwrap_or(false)
                        {
                            continue;
                        }
                        // This is deliberately a different SSE event name:
                        // clients must refresh the ancestor projection rather
                        // than merge child transcript data into the parent.
                        "descendant_session_event"
                    };
                    last_seen = event.meta.seq_global;
                    match Event::default()
                        .event(event_name)
                        .id(event.meta.seq_global.to_string())
                        .json_data(agena_application::event_projection::event_resource_from_runtime(&event))
                    {
                        Ok(ev) => yield Ok::<Event, Infallible>(ev),
                        Err(error) => {
                            yield Ok::<Event, Infallible>(sse_error_event(error));
                            return;
                        }
                    }
                }
                Some(RuntimeLiveEventSubscriptionItem::Lagged(skipped)) => {
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
    Json(request): Json<SessionMessageRequest>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let request =
        session_user_message_request(&state, session_id, request.run.options, request.document)
            .await?;
    let session_services = state.application().session_execution_services()?;
    let outcome = session_services
        .commands
        .submit_user_message(request)
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    session_execution_json_from_id(&state, outcome.session_id).await
}

pub async fn continue_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let request = session_execution_request(&state, session_id, request.options).await?;
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .continue_session(request)
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    session_execution_json_from_id(&state, outcome.session_id).await
}

pub async fn compact_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let request = session_execution_request(&state, session_id, request.options).await?;
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .compact_session(request)
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    session_execution_json_from_id(&state, outcome.session_id).await
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
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .fork_session(agena_runtime::SessionForkRequest {
            session_id,
            at_message_id: request.at_message_id,
            title: request.title,
            expected_version: if_match_version(&headers)?,
        })
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    session_execution_json_from_id(&state, outcome.session_id).await
}

pub async fn cancel_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(request): Json<CancelRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_command(
        &state,
        agena_api::commands::Command::CancelRun(agena_api::commands::CancelRunParams {
            session_id,
            execution_id: request.execution_id,
        }),
    )
    .await?
    {
        agena_api::commands::CommandResult::Cancellation(result) => Ok(Json(result)),
        _ => unreachable!("cancel run returned unexpected result"),
    }
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

    let request = session_permission_reply_request(
        &state,
        session_id,
        request.run.options,
        request.reply,
        Some("http_api".to_string()),
    )
    .await?;
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .reply_permission(request)
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    session_execution_json_from_id(&state, outcome.session_id).await
}

pub async fn reply_user_input(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplyRequestBody<UserInputReply>>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;

    let request =
        session_user_input_reply_request(&state, session_id, request.run.options, request.reply)
            .await?;
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .reply_user_input(request)
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    session_execution_json_from_id(&state, outcome.session_id).await
}

pub async fn mark_interactive_request_presented(
    State(state): State<AppState>,
    Path((session_id, request_id)): Path<(i64, String)>,
) -> Result<impl IntoResponse, ServerError> {
    let services = state.application().session_execution_services()?;
    let outcome = services
        .commands
        .mark_interactive_request_presented(session_id, request_id)
        .await
        .map_err(|error| ServerError::from_failure(error.failure))?;
    session_execution_json_from_id(&state, outcome.session_id).await
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
    session_execution_json_from_id(&state, outcome.session_id).await
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
    session_execution_json_from_id(&state, outcome.session_id).await
}

use super::{
    AppState, AxumQuery, Deserialize, Event, HeaderMap, Infallible, IntoResponse, Json, Path,
    PermissionReply, ServerError, SessionCreateRequest, SessionEventListCompatQuery,
    SessionEventStreamQuery, SessionForkRequestBody, SessionListQuery, SessionMessageRequest,
    SessionReplyRequestBody, SessionRewindRequestBody, SessionRunRequestBody, SessionUpdateRequest,
    Sse, State, UserInputReply, dispatch, if_match_version, json_http, json_http_found,
    server_error_from_application, sse_error_event, stream,
};
