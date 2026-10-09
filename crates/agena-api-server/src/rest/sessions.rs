use agena_application::dto::SessionPermissionUpdateRequest;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionOverviewQuery {
    #[serde(default)]
    pub workspace_id: Option<i64>,
    #[serde(default = "default_recent_limit")]
    pub recent_limit: u64,
}

const fn default_recent_limit() -> u64 {
    50
}

pub async fn session_overview(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SessionOverviewQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .application()
            .session_overview(query.workspace_id, query.recent_limit)
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
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    let bucket = query.bucket.map(|bucket| {
        serde_json::to_value(bucket)
            .expect("bucket")
            .as_str()
            .expect("bucket string")
            .to_owned()
    });
    let key = query.workspace_id.map_or_else(
        || match query.bucket {
            Some(_) => format!(
                "sessions:bucket:{}{}",
                bucket.as_ref().unwrap(),
                if query.count_only { ":count" } else { "" }
            ),
            None => "sessions".into(),
        },
        |id| {
            let base = format!("workspace:{id}:sessions");
            // Bucket keys cover every hierarchy subset of that bucket. Other
            // lists have clocks for roots or one parent's immediate children.
            if let Some(bucket) = &bucket {
                format!("{base}:bucket:{bucket}")
            } else if let Some(parent) = query.parent_id {
                format!("{base}:parent:{parent}")
            } else if query.roots {
                format!("{base}:roots")
            } else {
                base
            }
        },
    );
    let read = crate::revisions::ConditionalRead::new(&state, &key).await?;
    if let Some(response) = read.not_modified(&headers) {
        return Ok(response);
    }
    let revisions = state.revisions()?;
    let observed = revisions.token("sessions");
    let page = state
        .service()
        .list_sessions(query)
        .await
        .map_err(server_error_from_application)?;
    revisions.seed_session_list(&page.items, &observed);
    read.json(page).await
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
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    let read =
        crate::revisions::ConditionalRead::new(&state, &format!("session:{session_id}:state"))
            .await?;
    if let Some(response) = read.not_modified(&headers) {
        return Ok(response);
    }
    read.json(
        state
            .application()
            .session_execution_shell(session_id)
            .await?,
    )
    .await
}

pub async fn get_session_cost(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    if state.service().get_session(session_id).await?.is_none() {
        return Err(ServerError::not_found("The session was not found."));
    }
    let queries = state.application().session_query_service()?;
    Ok(Json(
        queries
            .session_cost_summary(session_id)
            .await
            .map_err(|error| ServerError::internal_error(&error))?,
    ))
}

pub async fn replace_session_selection(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRunRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    assert_if_match_session_version(&state, session_id, &headers).await?;
    Ok(Json(
        state
            .update_session_selection(session_id, request.options)
            .await?,
    ))
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
    fn ids(value: Option<&str>) -> Result<Vec<i64>, ServerError> {
        value
            .map(|value| {
                value
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<Vec<i64>, _>>()
            })
            .transpose()
            .map(|value| value.unwrap_or_default())
            .map_err(|_| ServerError::bad_request("Invalid Part selection."))
    }
    let mut resource = state
        .application()
        .read_parts(agena_api::queries::ReadPartsParams {
            session_id,
            ids: ids(query.ids.as_deref())?,
            run_ids: ids(query.run_ids.as_deref())?,
            cursor: query.cursor,
            limit: query.limit,
            sections: query
                .sections
                .as_deref()
                .map(|v| v.split(',').map(str::parse).collect::<Result<Vec<_>, _>>())
                .transpose()
                .map_err(|_| ServerError::bad_request("Unknown Part section."))?
                .unwrap_or_default(),
        })
        .await?;
    agena_application::part_locale::localize_part_headlines(
        &mut resource.parts,
        query.locale.as_deref(),
    );
    Ok(Json(resource))
}

/// Load one tool-call detail section on demand. The normal transcript
/// projection contains only the human-facing presentation; this endpoint is
/// the explicit disclosure boundary for the other four sections.
pub async fn get_session_tool_detail(
    State(state): State<AppState>,
    Path((session_id, part_id, section_name)): Path<(i64, i64, String)>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    let section = section_name
        .parse::<agena_api::live::ToolDetailSection>()
        .map_err(|error| {
            ServerError::bad_request_with_diagnostic("Unknown tool detail section.", error)
        })?;
    let key = if section == agena_api::live::ToolDetailSection::Presentation {
        format!("part:{part_id}")
    } else {
        format!("part:{part_id}:{}", section.as_str())
    };
    let read = crate::revisions::ConditionalRead::new(&state, &key).await?;
    if let Some(response) = read.not_modified(&headers) {
        return Ok(response);
    }
    let store = state.session_store()?;
    let view = store
        .load_part_ids(session_id, &[part_id])
        .await
        .map_err(crate::rest::server_error_from_store)?;
    let part = view
        .parts
        .into_iter()
        .find(|part| part.part_id == part_id && part.visibility.visible_to_user())
        .ok_or_else(|| ServerError::not_found("The tool part was not found."))?;
    state.revisions()?.register_part(&part);
    let detail = crate::live::project_tool_detail(&state, &part, section)
        .await
        .ok_or_else(|| ServerError::not_found("The tool part was not found."))?;
    read.json(detail).await
}

pub async fn get_session_plan(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    let read =
        crate::revisions::ConditionalRead::new(&state, &format!("session:{session_id}:plan"))
            .await?;
    if let Some(response) = read.not_modified(&headers) {
        return Ok(response);
    }
    // Validate the scope even if this session has not created a plan.
    if state.service().get_session(session_id).await?.is_none() {
        return Err(ServerError::not_found("The session was not found."));
    }
    let result = state
        .application()
        .invoke_plugin_tool(
            "agena.plan",
            "get",
            serde_json::json!({"view": "full"}),
            Some(session_id),
        )
        .await?;
    read.json(result).await
}

pub async fn stream_session_changes(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionChangeStreamQuery>,
) -> Result<impl IntoResponse, ServerError> {
    // Subscribe before reading the snapshot so mutations committed during the
    // read remain queued. The snapshot is current state, not replay.
    #[cfg(test)]
    let mut subscription = crate::live::subscribe_with_capacity(
        &state,
        agena_api::Scope::Session { session_id },
        query.test_queue_capacity.unwrap_or(256),
    )?;
    #[cfg(not(test))]
    let mut subscription =
        crate::live::subscribe(&state, agena_api::Scope::Session { session_id })?;
    #[cfg(test)]
    if let Some(probe) = query.test_subscription_probe.clone() {
        super::mark_test_session_stream_subscription(probe);
    }
    let session_queries = state.application().session_query_service()?;
    #[cfg(test)]
    if let Some(delay_ms) = query.test_snapshot_delay_ms.filter(|delay| *delay > 0) {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }
    let initial = crate::live::session_parts(&state, session_id).await?;

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
    Ok(Json(
        state
            .fork_session(
                session_id,
                request.at_message_id,
                request.title,
                request.conversation_mode,
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
    #[serde(default)]
    pub execution_id: Option<agena_domain::ExecutionId>,
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
            at_message_id: request.at_message_id,
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
    let resources = state
        .application()
        .session_resources_from_summaries(summaries)
        .await?;
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
