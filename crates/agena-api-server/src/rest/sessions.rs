use super::*;

pub async fn list_sessions(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SessionListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_sessions(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let session = state
        .service()
        .get_session(session_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("session not found: {session_id}")))?;
    Ok(Json(session))
}

pub async fn get_session_state(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let session = manager
        .get_session(session_id)
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn get_session_goal(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let session = manager
        .get_session(session_id)
        .await
        .map_err(ServerError::Core)?;
    let goal = match session.goal.as_ref() {
        Some(goal) => Some(
            state
                .service()
                .session_goal_resource(manager.as_ref(), &session, goal)
                .await
                .map_err(server_error_from_http)?,
        ),
        None => None,
    };
    Ok(Json(goal))
}

pub async fn set_session_goal(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    Json(request): Json<SessionGoalSetRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    if request.clear {
        let cleared = manager
            .clear_goal(session_id)
            .await
            .map_err(ServerError::Core)?;
        if !cleared {
            return Err(ServerError::NotFound(format!(
                "session {session_id} goal not found"
            )));
        }
        return Ok(Json(serde_json::Value::Null));
    }

    let goal = if manager
        .get_goal(session_id)
        .await
        .map_err(ServerError::Core)?
        .is_some()
    {
        manager
            .update_goal(agena::session::SessionGoalUpdateRequest {
                session_id,
                objective: request.objective,
                status: request.status,
                token_budget: request.token_budget,
                expected_goal_id: None,
            })
            .await
            .map_err(ServerError::Core)?
    } else {
        if !matches!(
            request.status,
            None | Some(agena::session::GoalStatus::Active)
        ) {
            return Err(ServerError::BadRequest(format!(
                "session {session_id} goal must be created with status active"
            )));
        }
        let objective = request.objective.ok_or_else(|| {
            ServerError::BadRequest(format!(
                "session {session_id} goal objective is required when creating a goal"
            ))
        })?;
        manager
            .create_goal(agena::session::SessionGoalCreateRequest {
                session_id,
                objective,
                token_budget: request.token_budget.flatten(),
            })
            .await
            .map_err(ServerError::Core)?
    };

    let session = manager
        .get_session(session_id)
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_goal_resource(manager.as_ref(), &session, &goal)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(serde_json::to_value(resource).map_err(|error| {
        ServerError::Internal(error.to_string())
    })?))
}

pub async fn complete_session_goal(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let goal = manager
        .complete_goal(session_id)
        .await
        .map_err(ServerError::Core)?;
    let session = manager
        .get_session(session_id)
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_goal_resource(manager.as_ref(), &session, &goal)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn clear_session_goal(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let cleared = manager
        .clear_goal(session_id)
        .await
        .map_err(ServerError::Core)?;
    if !cleared {
        return Err(ServerError::NotFound(format!(
            "session {session_id} goal not found"
        )));
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[tracing::instrument(skip_all)]
pub async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<SessionCreateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_session(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplaceRequest>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }
    Ok(Json(
        state
            .service()
            .replace_session(session_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }
    Ok(Json(
        state
            .service()
            .delete_session(session_id)
            .await
            .map_err(server_error_from_http)?,
    ))
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
            crate::local_api::SessionEventListQuery {
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

    let bus = manager.event_bus();
    let mut subscription = bus.subscribe(EventFilter::new(Scope::Session { session_id }));

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
                    last_seen = arc_event.meta.seq_global;
                    match Event::default()
                        .event("session_event")
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

pub async fn submit_turn(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionTurnRequest>,
) -> Result<impl IntoResponse, ServerError> {
    if request.parts.is_empty() {
        return Err(ServerError::BadRequest(
            "session turn requires at least one part".into(),
        ));
    }
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .submit_user_turn(agena::session::SessionUserTurnRequest {
            session_id,
            options,
            parts: request.parts,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn continue_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionContinueRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .continue_session(agena::session::SessionContinueRequest {
            session_id,
            options,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
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
    let session = manager
        .fork_session(agena::session::SessionForkRequest {
            session_id,
            at_message_id: request.at_message_id,
            title: request.title,
            expected_version: if_match_version(&headers)?,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_command(
        &state,
        agena_api::commands::Command::CancelTurn(agena_api::commands::CancelTurnParams {
            session_id,
        }),
    )
    .await?
    {
        agena_api::commands::CommandResult::Ack => Ok(Json(serde_json::json!({ "ok": true }))),
        _ => unreachable!("cancel turn returned unexpected result"),
    }
}

pub async fn reply_permission(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionPermissionReplyRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .reply_permission(agena::session::SessionPermissionReplyRequest {
            session_id,
            options,
            reply: request.reply,
            operator: Some("http_api".to_string()),
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn reply_user_input(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionUserInputReplyRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .reply_user_input(agena::session::SessionUserInputReplyRequest {
            session_id,
            options,
            reply: request.reply,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn rewind_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRewindRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let expected_version = if_match_version(&headers)?;
    let manager = state.session_manager()?;
    let session = manager
        .rewind_session(agena::session::SessionRewindRequest {
            session_id,
            message_id: request.message_id,
            expected_version,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn unrewind_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRewindRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    let expected_version = if_match_version(&headers)?;
    let manager = state.session_manager()?;
    let session = manager
        .unrewind_session(agena::session::SessionUnrewindRequest {
            session_id,
            message_id: request.message_id,
            expected_version,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
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

pub async fn list_rewind_checkpoints(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let checkpoints = manager
        .list_rewind_checkpoints(session_id)
        .await
        .map_err(ServerError::Core)?;
    let resources: Vec<agena_api::resource::RewindCheckpointResource> =
        checkpoints.into_iter().map(Into::into).collect();
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
    let session = manager
        .import_session_jsonl(&request.jsonl)
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}
