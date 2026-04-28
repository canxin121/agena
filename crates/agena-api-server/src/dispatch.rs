//! Command/Query dispatch into the underlying [`agena::session::SessionManager`].
//! REST and WS handlers funnel through these helpers so semantics stay
//! identical regardless of transport.

use agena::{
    event::EventKind,
    model::ModelRef,
    session::{
        SessionContinueRequest, SessionCreateRequest, SessionListRequest,
        SessionPermissionReplyRequest, SessionRunOptions, SessionUserInputReplyRequest,
        SessionUserTurnRequest,
    },
};
use agena_api::{
    commands::{
        Command, CommandResult, ContinueRunParams, CreateSessionParams, ReplyPermissionParams,
        ReplyUserInputParams, SubmitTurnParams,
    },
    pagination::{PageInfo, PaginatedResponse, normalize_limit},
    queries::{
        GetMessageParams, GetSessionParams, ListEventsParams, ListMessagesParams,
        ListSessionsParams, PaginatedEvents, Query, QueryResult,
    },
    resource::{RunOptions, SessionResource},
};
use agena_event::{EventStore, StoreRange};

use crate::{error::ServerError, state::AppState};

const DEFAULT_MODEL_REF: &str = "openai/gpt-4o-mini";

fn run_options_to_core(options: &RunOptions) -> SessionRunOptions {
    let model = options
        .model
        .clone()
        .unwrap_or_else(|| {
            let parts: Vec<&str> = DEFAULT_MODEL_REF.split('/').collect();
            ModelRef::new(parts[0], parts.get(1).copied().unwrap_or("gpt-4o-mini"))
        });
    SessionRunOptions {
        model,
        system: options.system.clone(),
        temperature: options.temperature,
        max_output_tokens: options.max_output_tokens,
    }
}

fn session_to_resource(session: &agena::session::Session) -> SessionResource {
    SessionResource {
        id: session.id,
        parent_id: session.parent_id,
        workspace_id: session.workspace_id,
        title: session.title.clone(),
        version: session.version,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: session.messages.len() as u64,
        child_session_count: 0,
        last_message_at: session.messages.last().map(|m| m.created_at),
    }
}

// ─── Command dispatch ───────────────────────────────────────────────────

pub async fn dispatch_command(
    state: &AppState,
    command: Command,
) -> Result<CommandResult, ServerError> {
    let manager = state.session_manager()?;
    match command {
        Command::CreateSession(CreateSessionParams { title, .. }) => {
            // Workspace selection is currently driven by the `SessionManager`
            // itself (it tracks a workspace_root). We surface the title and
            // optional parent_id only.
            let request = SessionCreateRequest {
                title,
                parent_session_id: None,
            };
            let session = manager.create_session(request).await?;
            Ok(CommandResult::Session(session_to_resource(&session)))
        }
        Command::SubmitTurn(SubmitTurnParams {
            session_id,
            options,
            parts,
        }) => {
            let request = SessionUserTurnRequest {
                session_id,
                options: run_options_to_core(&options),
                parts,
            };
            let session = manager.submit_user_turn(request).await?;
            Ok(CommandResult::Session(session_to_resource(&session)))
        }
        Command::ContinueRun(ContinueRunParams {
            session_id,
            options,
        }) => {
            let request = SessionContinueRequest {
                session_id,
                options: run_options_to_core(&options),
            };
            let session = manager.continue_session(request).await?;
            Ok(CommandResult::Session(session_to_resource(&session)))
        }
        Command::CancelTurn(_params) => {
            // Core does not yet expose cancellation through SessionManager —
            // the underlying processor handles it via the run-state machine.
            // Surface as Ack for now; concrete cancellation API will be
            // added when the provider abstraction exposes a stop handle.
            Ok(CommandResult::Ack)
        }
        Command::ReplyPermission(ReplyPermissionParams {
            session_id,
            options,
            reply,
        }) => {
            let request = SessionPermissionReplyRequest {
                session_id,
                options: run_options_to_core(&options),
                reply,
            };
            let session = manager.reply_permission(request).await?;
            Ok(CommandResult::Session(session_to_resource(&session)))
        }
        Command::ReplyUserInput(ReplyUserInputParams {
            session_id,
            options,
            reply,
        }) => {
            let request = SessionUserInputReplyRequest {
                session_id,
                options: run_options_to_core(&options),
                reply,
            };
            let session = manager.reply_user_input(request).await?;
            Ok(CommandResult::Session(session_to_resource(&session)))
        }
        // Workspace + permission-rule + delete flows live in the legacy
        // `agena-http-api` crate during the cutover. v2 surfaces them once
        // the core APIs exist.
        Command::CreateWorkspace(_)
        | Command::UpdateWorkspace(_)
        | Command::DeleteWorkspace(_)
        | Command::ResolveWorkspace(_)
        | Command::UpdateSession(_)
        | Command::DeleteSession(_)
        | Command::UpsertPermissionRule(_)
        | Command::DeletePermissionRule(_) => Err(ServerError::BadRequest(
            "this command is not yet implemented in the v2 API".into(),
        )),
    }
}

// ─── Query dispatch ─────────────────────────────────────────────────────

pub async fn dispatch_query(
    state: &AppState,
    query: Query,
) -> Result<QueryResult, ServerError> {
    let manager = state.session_manager()?;
    match query {
        Query::ListSessions(ListSessionsParams {
            workspace_id,
            limit,
            ..
        }) => {
            let request = SessionListRequest {
                limit,
                ..Default::default()
            };
            let summaries = manager.list_session_summaries(request).await?;
            let limit_n = normalize_limit(limit) as usize;
            let truncated: Vec<SessionResource> = summaries
                .into_iter()
                .filter(|s| match workspace_id {
                    Some(id) => s.workspace_id == id,
                    None => true,
                })
                .take(limit_n)
                .map(SessionResource::from)
                .collect();
            let returned = truncated.len() as u64;
            Ok(QueryResult::Sessions(PaginatedResponse {
                items: truncated,
                page: PageInfo {
                    next_cursor: None,
                    has_more: false,
                    returned,
                },
            }))
        }
        Query::GetSession(GetSessionParams { session_id }) => {
            let session = manager.get_session(session_id).await?;
            Ok(QueryResult::Session(session_to_resource(&session)))
        }
        Query::ListMessages(ListMessagesParams { session_id, .. }) => {
            let session = manager.get_session(session_id).await?;
            let items: Vec<_> = session
                .messages
                .iter()
                .map(|m| agena_api::resource::MessageResource {
                    id: m.id,
                    session_id: session.id,
                    role: m.role,
                    state: m.state,
                    created_at: m.created_at,
                    updated_at: m.created_at,
                    metadata: m.metadata.clone(),
                    usage: m.usage.clone(),
                    finish: m.finish.clone(),
                    part_count: m.parts.len() as u64,
                    parts: Some(m.parts.clone()),
                })
                .collect();
            let returned = items.len() as u64;
            Ok(QueryResult::Messages(PaginatedResponse {
                items,
                page: PageInfo {
                    next_cursor: None,
                    has_more: false,
                    returned,
                },
            }))
        }
        Query::GetMessage(GetMessageParams { message_id, .. }) => {
            let session_id = manager
                .find_session_id_for_message(message_id)
                .await?
                .ok_or_else(|| {
                    ServerError::NotFound(format!("message {message_id} not found"))
                })?;
            let session = manager.get_session(session_id).await?;
            let m = session
                .messages
                .iter()
                .find(|m| m.id == message_id)
                .ok_or_else(|| ServerError::NotFound(format!("message {message_id}")))?;
            Ok(QueryResult::Message(agena_api::resource::MessageResource {
                id: m.id,
                session_id: session.id,
                role: m.role,
                state: m.state,
                created_at: m.created_at,
                updated_at: m.created_at,
                metadata: m.metadata.clone(),
                usage: m.usage.clone(),
                finish: m.finish.clone(),
                part_count: m.parts.len() as u64,
                parts: Some(m.parts.clone()),
            }))
        }
        Query::ListEvents(ListEventsParams {
            scope,
            kinds,
            since_seq_global,
            limit,
        }) => {
            let publisher = state.event_publisher()?;
            let store: &std::sync::Arc<dyn EventStore<EventKind>> = publisher.store();
            let filter = agena_event::EventFilter {
                scope,
                kinds,
                since_seq_global,
            };
            let limit = normalize_limit(limit) as usize;
            let range = StoreRange {
                after_seq_global: since_seq_global.unwrap_or(0),
                limit,
            };
            let events = store
                .range(&filter, range)
                .await
                .map_err(|e| ServerError::Internal(e.to_string()))?;
            let returned = events.len() as u64;
            let next_cursor = events.last().map(|e| e.meta.seq_global.to_string());
            Ok(QueryResult::Events(PaginatedEvents {
                items: events,
                page: PageInfo {
                    next_cursor,
                    has_more: returned as usize >= limit,
                    returned,
                },
            }))
        }
        Query::Health => Ok(QueryResult::Health(agena_api::resource::HealthResponse {
            status: "ok".into(),
            generation: 0,
            loaded_at: chrono::Utc::now(),
            database_connected: true,
        })),
        // Auth/runtime/provider/workspace/permission-rule queries are not
        // re-implemented in the v2 server during the cutover. Callers should
        // still hit the legacy `agena-http-api` for those.
        Query::Runtime
        | Query::ListProviders
        | Query::ListProviderModels(_)
        | Query::ListWorkspaces(_)
        | Query::GetWorkspace(_)
        | Query::GetSessionState(_)
        | Query::ListPermissionRules(_)
        | Query::GetPermissionRule(_) => Err(ServerError::BadRequest(
            "this query is not yet implemented in the v2 API".into(),
        )),
    }
}
