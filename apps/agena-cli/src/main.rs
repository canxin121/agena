use std::{path::PathBuf, sync::Arc};

use agena::{
    AppError,
    cli::{AgenaCli, AgenaCommand, AppServerArgs, AppServerTransport, TuiArgs},
    config::{ConfigLoader, TracingConfig},
    message::PartContent,
    model::ModelRef,
    permission::{PermissionReply, PermissionReplyKind, PermissionScope},
    role::Role,
    runtime::AgenaRuntime,
    session::{
        Session, SessionCreateRequest, SessionListRequest, SessionManager,
        SessionPermissionReplyRequest, SessionRunOptions, SessionUserMessageRequest,
    },
    storage::StorageConfig,
};
use agena_api_server::jsonrpc::protocol::{
    CancelRunParams, CancelRunResult, CreateSessionParams, CreateSessionResult,
    ListSessionsParams as AppListSessionsParams, ListSessionsResult as AppListSessionsResult,
    MessageItem, PermissionDecision as AppPermissionDecision, PermissionRememberScope,
    PermissionReplyParams, PermissionReplyResult, ReadMessagesParams, ReadMessagesResult,
    SessionListItem, SubmitMessageParams, SubmitMessageResult,
};
use agena_api_server::jsonrpc::{self, AppServerError};
use async_trait::async_trait;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() -> Result<(), agena::AppError> {
    agena::runtime::build_app_runtime()?.block_on(async_main())
}

async fn async_main() -> Result<(), agena::AppError> {
    let cli = AgenaCli::parse();
    if let Some(args) = tui_launch_args(&cli) {
        return agena_tui::run_with_load_request(
            cli.load_request(),
            agena_tui::TuiLaunchArgs {
                database_url: args.database_url.or_else(|| cli.database_url.clone()),
                database_path: args.database_path.or_else(|| cli.database_path.clone()),
                workspace_root: args.workspace,
                session: args.session,
                search: args.search,
                locale: args.locale,
                log_file: args.log_file,
                log_stderr: args.log_stderr,
                tui_config: args.tui_config,
            },
        )
        .await;
    }

    let resolution = ConfigLoader::default().load(&cli.load_request()).ok();
    let tracing = resolution
        .as_ref()
        .map(|resolution| resolution.config.tracing.clone())
        .unwrap_or_else(TracingConfig::default);

    let initial_filter = agena::tracing::env_filter(&tracing).unwrap_or_else(|_| {
        agena::tracing::env_filter(&TracingConfig::default())
            .expect("default tracing filter should parse")
    });
    let (filter_layer, filter_handle) = tracing_subscriber::reload::Layer::new(initial_filter);
    tracing_subscriber::registry()
        .with(filter_layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact(),
        )
        .init();

    match cli.command.clone() {
        Some(AgenaCommand::AppServer(args)) => run_app_server(cli, args).await,
        _ => cli.run(Some(filter_handle)).await,
    }
}

fn tui_launch_args(cli: &AgenaCli) -> Option<TuiArgs> {
    match cli.command.clone() {
        None => Some(TuiArgs::default()),
        Some(AgenaCommand::Tui(args)) => Some(args),
        _ => None,
    }
}

async fn run_app_server(cli: AgenaCli, args: AppServerArgs) -> Result<(), AppError> {
    let backend = AgenaAppServerBackend {
        runtime: session_runtime_with_workspace(&cli, args.workspace.as_ref()).await?,
    };
    match args.transport {
        AppServerTransport::Stdio => jsonrpc::serve_stdio(backend)
            .await
            .map_err(|err| AppError::Config(err.to_string())),
    }
}

async fn session_runtime_with_workspace(
    cli: &AgenaCli,
    workspace: Option<&PathBuf>,
) -> Result<AgenaRuntime, AppError> {
    let storage = StorageConfig {
        database_url: cli.database_url.clone(),
        database_path: cli.database_path.clone(),
    };
    let database_url = storage.resolve_url()?;
    StorageConfig::ensure_parent(database_url.as_str())?;
    let mut builder = AgenaRuntime::builder()
        .with_load_request(cli.load_request())
        .with_database_url(database_url);
    if let Some(workspace) = workspace {
        builder = builder.with_workspace_root(workspace.clone());
    }
    builder.build().await
}

#[derive(Clone)]
struct AgenaAppServerBackend {
    runtime: AgenaRuntime,
}

#[async_trait]
impl jsonrpc::AppServerBackend for AgenaAppServerBackend {
    async fn create_session(
        &self,
        params: CreateSessionParams,
    ) -> Result<CreateSessionResult, AppServerError> {
        let manager = app_session_manager(&self.runtime)?;
        let session = manager
            .create_session(SessionCreateRequest {
                title: params.title.unwrap_or_else(|| "IDE session".to_owned()),
                parent_session_id: params.parent_session_id,
            })
            .await
            .map_err(app_backend_error)?;
        Ok(CreateSessionResult {
            session_id: session.id,
            title: session.title,
        })
    }

    async fn submit_message(
        &self,
        params: SubmitMessageParams,
    ) -> Result<SubmitMessageResult, AppServerError> {
        let manager = app_session_manager(&self.runtime)?;
        let options = resolve_run_options(
            &self.runtime,
            params.model.as_deref(),
            params.temperature,
            params.max_output_tokens,
        )
        .map_err(app_backend_error)?;
        let session = manager
            .submit_user_message(SessionUserMessageRequest::new(
                params.session_id,
                options,
                vec![PartContent::text(params.prompt)],
            ))
            .await
            .map_err(app_backend_error)?;
        Ok(SubmitMessageResult {
            session_id: session.id,
            status: format!("{:?}", session.runtime().run.status).to_ascii_lowercase(),
            text: last_assistant_text(&session),
        })
    }

    async fn reply_permission(
        &self,
        params: PermissionReplyParams,
    ) -> Result<PermissionReplyResult, AppServerError> {
        let manager = app_session_manager(&self.runtime)?;
        let session = manager
            .get_session(params.session_id)
            .await
            .map_err(app_backend_error)?;
        let options = resolve_permission_continue_options(&self.runtime, &session)
            .map_err(app_backend_error)?;
        let session = manager
            .reply_permission(SessionPermissionReplyRequest::new(
                params.session_id,
                options,
                PermissionReply {
                    request_id: params.request_id,
                    kind: app_permission_reply_kind(params.decision, params.remember),
                    reason: params.reason,
                    scope: params.remember.map(app_permission_scope),
                },
                Some("app_server".to_string()),
            ))
            .await
            .map_err(app_backend_error)?;
        Ok(PermissionReplyResult {
            session_id: session.id,
            status: format!("{:?}", session.runtime().run.status).to_ascii_lowercase(),
        })
    }

    async fn list_sessions(
        &self,
        params: AppListSessionsParams,
    ) -> Result<AppListSessionsResult, AppServerError> {
        let manager = app_session_manager(&self.runtime)?;
        let sessions = manager
            .list_session_summaries(SessionListRequest {
                offset: params.offset,
                limit: params.limit,
                include_subagents: false,
            })
            .await
            .map_err(app_backend_error)?;
        Ok(AppListSessionsResult {
            sessions: sessions
                .into_iter()
                .map(|session| SessionListItem {
                    session_id: session.id,
                    title: session.title,
                    status: "idle".to_owned(),
                    updated_at: session.updated_at,
                })
                .collect(),
        })
    }

    async fn read_messages(
        &self,
        params: ReadMessagesParams,
    ) -> Result<ReadMessagesResult, AppServerError> {
        let manager = app_session_manager(&self.runtime)?;
        let session = manager
            .get_session(params.session_id)
            .await
            .map_err(app_backend_error)?;
        Ok(ReadMessagesResult {
            messages: session
                .messages
                .into_iter()
                .map(|message| MessageItem {
                    message_id: message.id,
                    role: message.role.to_string(),
                    status: message.state.to_string(),
                    text: message.as_text_lossy(),
                    created_at: message.created_at,
                })
                .collect(),
        })
    }

    async fn cancel_run(&self, params: CancelRunParams) -> Result<CancelRunResult, AppServerError> {
        let manager = app_session_manager(&self.runtime)?;
        manager
            .cancel_active_run(params.session_id)
            .await
            .map_err(app_backend_error)?;
        Ok(CancelRunResult {
            session_id: params.session_id,
            cancelled: true,
        })
    }
}

fn app_session_manager(runtime: &AgenaRuntime) -> Result<Arc<SessionManager>, AppServerError> {
    runtime
        .session_manager()
        .ok_or_else(|| AppServerError::Backend(session_storage_error().to_string()))
}

fn app_backend_error(error: impl ToString) -> AppServerError {
    AppServerError::Backend(error.to_string())
}

fn resolve_permission_continue_options(
    runtime: &AgenaRuntime,
    session: &Session,
) -> Result<SessionRunOptions, AppError> {
    let model = if let Some(model) = session
        .runtime()
        .effective_model_ref()
        .map_err(|err| AppError::Config(format!("invalid persisted model reference: {err}")))?
    {
        model
    } else {
        default_model(runtime)?
    };

    Ok(SessionRunOptions::new(model))
}

fn resolve_run_options(
    runtime: &AgenaRuntime,
    model: Option<&str>,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
) -> Result<SessionRunOptions, AppError> {
    let model = if let Some(model) = model {
        runtime
            .current_snapshot()
            .resolve_model_target(model, None)?
    } else {
        default_model(runtime)?
    };

    let mut options = SessionRunOptions::new(model);
    options.temperature = temperature;
    options.max_output_tokens = max_output_tokens;
    Ok(options)
}

fn default_model(runtime: &AgenaRuntime) -> Result<ModelRef, AppError> {
    runtime
        .current_snapshot()
        .resolve_default_model()?
        .ok_or_else(|| AppError::Config("no providers configured".to_owned()))
}

fn last_assistant_text(session: &Session) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| message.as_text_lossy())
}

fn session_storage_error() -> AppError {
    AppError::Config("session storage is unavailable; configure a database URL or path".to_owned())
}

fn app_permission_reply_kind(
    decision: AppPermissionDecision,
    remember: Option<PermissionRememberScope>,
) -> PermissionReplyKind {
    match (decision, remember) {
        (AppPermissionDecision::Allow, Some(_)) => PermissionReplyKind::AllowAlways,
        (AppPermissionDecision::Allow, None) => PermissionReplyKind::AllowOnce,
        (AppPermissionDecision::Deny, Some(_)) => PermissionReplyKind::DenyAlways,
        (AppPermissionDecision::Deny, None) => PermissionReplyKind::DenyOnce,
    }
}

fn app_permission_scope(scope: PermissionRememberScope) -> PermissionScope {
    match scope {
        PermissionRememberScope::Session => PermissionScope::Session,
        PermissionRememberScope::Workspace => PermissionScope::Workspace,
        PermissionRememberScope::Global => PermissionScope::Global,
    }
}
