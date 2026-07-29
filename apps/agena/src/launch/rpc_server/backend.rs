use agena_domain::{PermissionReply, SessionListRequest, TextPart};
use std::path::PathBuf;

use crate::error::AgenaProcessError;
use agena_api_server::jsonrpc::protocol::{
    CancelRunParams, CancelRunResult, CreateSessionParams, CreateSessionResult,
    ListSessionsParams as AppListSessionsParams, ListSessionsResult as AppListSessionsResult,
    MessageItem, PermissionDecision as AppPermissionDecision, PermissionRememberScope,
    PermissionReplyParams, PermissionReplyResult, ReadMessagesParams, ReadMessagesResult,
    SessionListItem, SubmitMessageParams, SubmitMessageResult,
};
use agena_api_server::jsonrpc::{self, AppServerError};
use agena_cli::{AgenaCli, RpcServerRequest, RpcServerTransport};
use agena_domain::{PermissionReplyKind, PermissionScope};
use agena_provider::ProviderCatalog;
use agena_runtime::bootstrap_application_services;
use agena_runtime::{
    RuntimeApplicationServices, SessionCreateRequest, SessionExecutionCommandService,
    SessionExecutionControl, SessionPermissionReplyRequest, SessionQueryService, SessionRunOptions,
    SessionUserMessagePart, SessionUserMessageRequest,
};
use async_trait::async_trait;
use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub(crate) async fn run_command(cli: AgenaCli) -> Result<(), AgenaProcessError> {
    let tracing = agena_runtime::resolve_runtime_bootstrap_preflight(
        &agena_runtime::RuntimeBootstrapRequest {
            config_override_expressions: cli.overrides.clone(),
            ..Default::default()
        },
    )
    .ok()
    .map(|preflight| preflight.tracing)
    .unwrap_or_default();

    let initial_filter = agena_runtime::runtime_env_filter(&tracing).unwrap_or_else(|_| {
        agena_runtime::runtime_env_filter(&agena_runtime::RuntimeTracingConfiguration::default())
            .expect("default tracing filter should parse")
    });
    tracing_subscriber::registry()
        .with(initial_filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact()
                .with_writer(std::io::stderr),
        )
        .init();

    cli.run_command()
        .await
        .map_err(|error| AgenaProcessError::Internal(error.to_string()))
}

pub(crate) async fn run(request: RpcServerRequest) -> Result<(), AgenaProcessError> {
    let runtime = session_runtime_with_workspace(&request, request.args.workspace.as_ref()).await?;
    let backend = AgenaAppServerBackend {
        services: runtime.application_services(),
        runtime: runtime.clone(),
    };
    // Keep the result as part of the backend's server-lifetime state rather
    // than relying on the independently-held local clone below.
    let _ = &backend.runtime;
    let result = match request.args.transport {
        RpcServerTransport::Stdio => jsonrpc::serve_stdio(backend)
            .await
            .map_err(|err| AgenaProcessError::Configuration(err.to_string())),
    };
    runtime.shutdown();
    result
}

async fn session_runtime_with_workspace(
    request: &RpcServerRequest,
    workspace: Option<&PathBuf>,
) -> Result<agena_runtime::RuntimeBootstrapResult, AgenaProcessError> {
    bootstrap_application_services(agena_runtime::RuntimeBootstrapRequest {
        workspace_root: workspace.cloned(),
        config_override_expressions: request.config_override_expressions.clone(),
        database_url: request.database_url.clone(),
        database_path: request.database_path.clone(),
        initialize_schema: true,
        tracing_reload_handle: None,
    })
    .await
    .map_err(|error| AgenaProcessError::Internal(error.to_string()))
}

#[derive(Clone)]
struct AgenaAppServerBackend {
    services: RuntimeApplicationServices,
    // Retain Runtime lifecycle ownership for the complete RPC-server lifetime.
    runtime: agena_runtime::RuntimeBootstrapResult,
}

#[async_trait]
impl jsonrpc::AppServerBackend for AgenaAppServerBackend {
    async fn create_session(
        &self,
        params: CreateSessionParams,
    ) -> Result<CreateSessionResult, AppServerError> {
        let commands = app_session_commands(&self.services)?;
        let session = commands
            .create_session(SessionCreateRequest {
                title: params.title.unwrap_or_else(|| "IDE session".to_owned()),
                parent_session_id: params.parent_session_id,
            })
            .await
            .map_err(app_backend_error)?;
        let presentation = app_session_queries(&self.services)?
            .session_presentation(session.session_id)
            .await
            .map_err(app_backend_error)?;
        Ok(CreateSessionResult {
            session_id: presentation.id,
            title: presentation.title,
        })
    }

    async fn submit_message(
        &self,
        params: SubmitMessageParams,
    ) -> Result<SubmitMessageResult, AppServerError> {
        let commands = app_session_commands(&self.services)?;
        let options = resolve_run_options(
            self.services.provider_catalog.as_ref(),
            params.model.as_deref(),
            params.temperature,
            params.max_output_tokens,
        )
        .map_err(app_backend_error)?;
        let outcome = commands
            .submit_user_message(SessionUserMessageRequest::new(
                params.session_id,
                options,
                vec![SessionUserMessagePart::Text(TextPart {
                    text: params.prompt,
                    synthetic: false,
                })],
            ))
            .await
            .map_err(app_backend_error)?;
        let queries = app_session_queries(&self.services)?;
        let presentation = queries
            .session_presentation(outcome.session_id)
            .await
            .map_err(app_backend_error)?;
        let messages = queries
            .list_projected_messages(outcome.session_id, true)
            .await
            .map_err(app_backend_error)?;
        Ok(SubmitMessageResult {
            session_id: presentation.id,
            status: format!("{:?}", presentation.workflow_state).to_ascii_lowercase(),
            text: last_assistant_text_from_projection(messages),
        })
    }

    async fn reply_permission(
        &self,
        params: PermissionReplyParams,
    ) -> Result<PermissionReplyResult, AppServerError> {
        let commands = app_session_commands(&self.services)?;
        let selected_model = app_session_control(&self.services)?
            .selected_model(params.session_id)
            .await
            .map_err(app_backend_error)?;
        let options = resolve_permission_continue_options(
            self.services.provider_catalog.as_ref(),
            selected_model,
        )
        .map_err(app_backend_error)?;
        let outcome = commands
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
        let presentation = app_session_queries(&self.services)?
            .session_presentation(outcome.session_id)
            .await
            .map_err(app_backend_error)?;
        Ok(PermissionReplyResult {
            session_id: presentation.id,
            status: format!("{:?}", presentation.workflow_state).to_ascii_lowercase(),
        })
    }

    async fn list_sessions(
        &self,
        params: AppListSessionsParams,
    ) -> Result<AppListSessionsResult, AppServerError> {
        let sessions = app_session_queries(&self.services)?
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
        let messages = app_session_queries(&self.services)?
            .list_projected_messages(params.session_id, true)
            .await
            .map_err(app_backend_error)?;
        Ok(ReadMessagesResult {
            messages: messages
                .into_iter()
                .map(|message| MessageItem {
                    message_id: message.id,
                    role: message.role.to_string(),
                    status: message.state.to_string(),
                    text: projected_message_visible_text(&message),
                    created_at: message.created_at,
                })
                .collect(),
        })
    }

    async fn cancel_run(&self, params: CancelRunParams) -> Result<CancelRunResult, AppServerError> {
        app_session_control(&self.services)?
            .cancel_active_execution(params.session_id)
            .await
            .map_err(app_backend_error)?;
        Ok(CancelRunResult {
            session_id: params.session_id,
            cancelled: true,
        })
    }
}

fn app_session_queries(
    services: &RuntimeApplicationServices,
) -> Result<&std::sync::Arc<dyn SessionQueryService>, AppServerError> {
    services
        .session_queries
        .as_ref()
        .ok_or_else(|| AppServerError::Backend(session_storage_error().to_string()))
}

fn app_session_commands(
    services: &RuntimeApplicationServices,
) -> Result<&std::sync::Arc<dyn SessionExecutionCommandService>, AppServerError> {
    services
        .execution_commands
        .as_ref()
        .ok_or_else(|| AppServerError::Backend(session_storage_error().to_string()))
}

fn app_session_control(
    services: &RuntimeApplicationServices,
) -> Result<&std::sync::Arc<dyn SessionExecutionControl>, AppServerError> {
    services
        .execution_control
        .as_ref()
        .ok_or_else(|| AppServerError::Backend(session_storage_error().to_string()))
}

fn app_backend_error(error: impl ToString) -> AppServerError {
    AppServerError::Backend(error.to_string())
}

fn resolve_permission_continue_options(
    providers: &dyn ProviderCatalog,
    selected_model: Option<agena_domain::ModelRef>,
) -> Result<SessionRunOptions, AgenaProcessError> {
    let model = if let Some(model) = selected_model {
        model
    } else {
        default_model(providers)?
    };

    Ok(SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: None,
    })
}

fn resolve_run_options(
    providers: &dyn ProviderCatalog,
    model: Option<&str>,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
) -> Result<SessionRunOptions, AgenaProcessError> {
    let model = if let Some(model) = model {
        providers
            .resolve_model_target(model, None)
            .map_err(|error| AgenaProcessError::Configuration(error.to_string()))?
    } else {
        default_model(providers)?
    };

    Ok(SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature,
        max_output_tokens,
    })
}

fn default_model(
    providers: &dyn ProviderCatalog,
) -> Result<agena_domain::ModelRef, AgenaProcessError> {
    providers
        .default_model()
        .map_err(|error| AgenaProcessError::Configuration(error.to_string()))?
        .ok_or_else(|| AgenaProcessError::Configuration("no providers configured".to_owned()))
}

fn last_assistant_text_from_projection(
    messages: Vec<agena_runtime::SessionProjectedMessage>,
) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == agena_domain::Role::Assistant)
        .map(projected_message_visible_text)
        .filter(|text| !text.trim().is_empty())
}

fn projected_message_visible_text(message: &agena_runtime::SessionProjectedMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part.detail.as_ref() {
            Some(agena_runtime::SessionProjectedPartDetail::Text { text, .. }) => {
                Some(text.clone())
            }
            _ => part.summary.clone(),
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn session_storage_error() -> AgenaProcessError {
    AgenaProcessError::Configuration(
        "session storage is unavailable; configure a database URL or path".to_owned(),
    )
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
