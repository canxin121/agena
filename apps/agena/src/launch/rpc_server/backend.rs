use crate::error::AgenaProcessError;
use agena_api::{
    commands::{Command, CommandResult, ResolveWorkspaceParams},
    queries::{ListSessionsParams, Query, QueryResult},
    resource::{
        ModelRef, PermissionReply, PermissionReplyKind, PermissionScope, ProviderSummaryResource,
        RunOptions, SessionState, SessionTranscriptPart,
    },
};
use agena_api_server::jsonrpc::protocol::{
    CancelRunParams, CancelRunResult, CreateSessionParams, CreateSessionResult,
    ListSessionsParams as AppListSessionsParams, ListSessionsResult as AppListSessionsResult,
    PermissionDecision as AppPermissionDecision, PermissionRememberScope, PermissionReplyParams,
    PermissionReplyResult, ReadPartsParams, ReadPartsResult, SessionListItem, SubmitRunParams,
    SubmitRunResult,
};
use agena_api_server::jsonrpc::{self, AppServerError};
use agena_cli::{RpcServerRequest, RpcServerTransport};
use agena_client::{AgenaClient, ClientError};
use async_trait::async_trait;

pub(crate) async fn run(request: RpcServerRequest) -> Result<(), AgenaProcessError> {
    if request.database_url.is_some() || request.database_path.is_some() {
        return Err(AgenaProcessError::Configuration(
            "--database-url/--database-path belong to the server and cannot be used by rpc-server"
                .to_owned(),
        ));
    }
    if !request.config_override_expressions.is_empty() {
        return Err(AgenaProcessError::Configuration(
            "--set overrides belong to the server and cannot be used by rpc-server".to_owned(),
        ));
    }

    let workspace_root = request
        .args
        .workspace
        .clone()
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)?;
    let server_url = super::super::server_client::resolve_server_url(request.args.server.clone());
    let backend = AgenaAppServerBackend::connect(
        server_url.as_str(),
        workspace_root,
        request.args.server_token.as_deref(),
        request.args.server_password.as_deref(),
    )
    .await?;
    match request.args.transport {
        RpcServerTransport::Stdio => jsonrpc::serve_stdio(backend)
            .await
            .map_err(|err| AgenaProcessError::Configuration(err.to_string())),
    }
}

#[derive(Clone)]
struct AgenaAppServerBackend {
    client: AgenaClient,
    workspace_id: i64,
    providers: Vec<ProviderSummaryResource>,
}

impl AgenaAppServerBackend {
    async fn connect(
        server_url: &str,
        workspace_root: std::path::PathBuf,
        server_token: Option<&str>,
        server_password: Option<&str>,
    ) -> Result<Self, AgenaProcessError> {
        let client = AgenaClient::connect_server(server_url, server_token, server_password)
            .await
            .map_err(|error| {
                process_client_error("server readiness/authentication handshake failed", &error)
            })?;
        let workspace = client
            .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
                path: workspace_root.to_string_lossy().into_owned(),
                create_if_missing: true,
            }))
            .await
            .map_err(|error| {
                process_client_error(
                    "failed to resolve the IDE workspace through the server",
                    &error,
                )
            })?;
        let CommandResult::Workspace(workspace) = workspace else {
            return Err(AgenaProcessError::Internal(
                "server returned the wrong result while resolving the IDE workspace".to_owned(),
            ));
        };
        let providers = client.query(Query::ListProviders).await.map_err(|error| {
            process_client_error("failed to read the server's provider catalog", &error)
        })?;
        let QueryResult::Providers(providers) = providers else {
            return Err(AgenaProcessError::Internal(
                "server returned the wrong provider-list result".to_owned(),
            ));
        };
        Ok(Self {
            client,
            workspace_id: workspace.id,
            providers,
        })
    }

    fn run_options(
        &self,
        model: Option<&str>,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
    ) -> Result<RunOptions, AppServerError> {
        Ok(RunOptions {
            model: model
                .map(|target| self.resolve_model_target(target))
                .transpose()?,
            temperature,
            max_output_tokens,
            ..RunOptions::default()
        })
    }

    fn resolve_model_target(&self, target: &str) -> Result<ModelRef, AppServerError> {
        let target = target.trim();
        if target.is_empty() {
            return Err(AppServerError::InvalidParams(
                "provider or model reference cannot be empty".to_owned(),
            ));
        }

        if let Some((provider_id, model_id)) = target.split_once('/') {
            if provider_id.trim().is_empty() || model_id.trim().is_empty() {
                return Err(AppServerError::InvalidParams(format!(
                    "invalid model reference `{target}`; expected provider/model"
                )));
            }
            let adapter_id = self
                .providers
                .iter()
                .find(|provider| provider.provider_id == provider_id.trim())
                .and_then(|provider| provider.defaults.adapter.clone());
            return Ok(ModelRef {
                provider_id: provider_id.trim().to_owned(),
                adapter_id,
                model_id: model_id.trim().to_owned(),
            });
        }

        let provider = self
            .providers
            .iter()
            .find(|provider| provider.provider_id == target)
            .ok_or_else(|| {
                AppServerError::InvalidParams(format!("provider not found: {target}"))
            })?;
        Ok(ModelRef {
            provider_id: provider.provider_id.clone(),
            adapter_id: provider.defaults.adapter.clone(),
            model_id: provider.defaults.model.clone(),
        })
    }

    async fn paginated_sessions(
        &self,
        offset: u64,
        limit: Option<u64>,
    ) -> Result<Vec<agena_api::resource::SessionResource>, AppServerError> {
        let mut cursor = None;
        let mut skipped = 0_u64;
        let mut sessions = Vec::new();

        loop {
            let response = self
                .client
                .query(Query::ListSessions(ListSessionsParams {
                    cursor,
                    limit: Some(agena_api::pagination::MAX_LIMIT),
                    workspace_id: Some(self.workspace_id),
                    parent_id: None,
                    roots: false,
                    exclude_subagents: true,
                    search: None,
                }))
                .await
                .map_err(client_backend_error)?;
            let QueryResult::Sessions(page) = response else {
                return Err(AppServerError::Backend(
                    "server returned the wrong session-list result".to_owned(),
                ));
            };
            for session in page.items {
                if skipped < offset {
                    skipped = skipped.saturating_add(1);
                    continue;
                }
                sessions.push(session);
                if limit.is_some_and(|limit| sessions.len() as u64 >= limit) {
                    return Ok(sessions);
                }
            }
            if !page.page.has_more {
                return Ok(sessions);
            }
            cursor = page.page.next_cursor;
            if cursor.is_none() {
                return Err(AppServerError::Backend(
                    "server returned a truncated session page without a cursor".to_owned(),
                ));
            }
        }
    }
}

#[async_trait]
impl jsonrpc::AppServerBackend for AgenaAppServerBackend {
    async fn create_session(
        &self,
        params: CreateSessionParams,
    ) -> Result<CreateSessionResult, AppServerError> {
        let session = self
            .client
            .create_session(
                self.workspace_id,
                params.title.unwrap_or_else(|| "IDE session".to_owned()),
                params.parent_session_id,
            )
            .await
            .map_err(client_backend_error)?;
        Ok(CreateSessionResult {
            session_id: session.id,
            title: session.title,
        })
    }

    async fn submit_message(
        &self,
        params: SubmitRunParams,
    ) -> Result<SubmitRunResult, AppServerError> {
        let options = self.run_options(
            params.model.as_deref(),
            params.temperature,
            params.max_output_tokens,
        )?;
        let execution = self
            .client
            .submit_message(agena_api::commands::SubmitRunParams {
                session_id: params.session_id,
                options,
                document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: params.prompt,
                }]),
            })
            .await
            .map_err(client_backend_error)?;
        let (run_id, parts) = latest_run_parts(&execution.parts);
        Ok(SubmitRunResult {
            session_id: execution.session.id,
            run_id,
            parts,
        })
    }

    async fn reply_permission(
        &self,
        params: PermissionReplyParams,
    ) -> Result<PermissionReplyResult, AppServerError> {
        let execution = self
            .client
            .reply_permission(agena_api::commands::ReplyPermissionParams {
                session_id: params.session_id,
                options: RunOptions::default(),
                reply: PermissionReply {
                    request_id: params.request_id,
                    kind: app_permission_reply_kind(params.decision, params.remember),
                    reason: params.reason,
                    scope: params.remember.map(app_permission_scope),
                },
            })
            .await
            .map_err(client_backend_error)?;
        Ok(PermissionReplyResult {
            session_id: execution.session.id,
            status: format!("{:?}", execution.session.state.workflow_state()).to_ascii_lowercase(),
        })
    }

    async fn list_sessions(
        &self,
        params: AppListSessionsParams,
    ) -> Result<AppListSessionsResult, AppServerError> {
        let sessions = self.paginated_sessions(params.offset, params.limit).await?;
        Ok(AppListSessionsResult {
            sessions: sessions
                .into_iter()
                .map(|session| SessionListItem {
                    session_id: session.id,
                    title: session.title,
                    status: session_state_name(&session.state).to_owned(),
                    updated_at: session.updated_at,
                })
                .collect(),
        })
    }

    async fn read_messages(
        &self,
        params: ReadPartsParams,
    ) -> Result<ReadPartsResult, AppServerError> {
        let execution = self
            .client
            .get_session_state(params.session_id)
            .await
            .map_err(client_backend_error)?;
        Ok(ReadPartsResult {
            parts: execution.parts,
        })
    }

    async fn cancel_run(&self, params: CancelRunParams) -> Result<CancelRunResult, AppServerError> {
        let result = self
            .client
            .cancel_run(params.session_id, params.execution_id)
            .await
            .map_err(client_backend_error)?;
        Ok(CancelRunResult {
            session_id: params.session_id,
            result,
        })
    }
}

fn process_client_error(context: &str, error: &ClientError) -> AgenaProcessError {
    let detail = error
        .diagnostic_message()
        .unwrap_or_else(|| error.to_string());
    AgenaProcessError::Configuration(format!("{context}: {detail}"))
}

fn client_backend_error(error: ClientError) -> AppServerError {
    AppServerError::Backend(error.to_string())
}

fn latest_run_parts(parts: &[SessionTranscriptPart]) -> (Option<i64>, Vec<SessionTranscriptPart>) {
    let run_id = parts
        .iter()
        .rev()
        .find(|part| part.kind == "run")
        .map(|part| part.part_id);
    let selected = run_id
        .map(|run_id| {
            parts
                .iter()
                .filter(|part| part.part_id == run_id || part.run_id == Some(run_id))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    (run_id, selected)
}

fn session_state_name(state: &SessionState) -> &'static str {
    match state {
        SessionState::Creating => "creating",
        SessionState::Ready { .. } => "ready",
        SessionState::Running { .. } => "running",
        SessionState::AwaitingInteraction { .. } => "awaiting_interaction",
        SessionState::Interrupted { .. } => "interrupted",
        SessionState::Failed { .. } => "failed",
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use agena_api::resource::{ProviderDefaultsResource, SessionTranscriptPart};

    fn backend_with_public_provider_metadata() -> AgenaAppServerBackend {
        AgenaAppServerBackend {
            client: AgenaClient::new("http://127.0.0.1:3210").expect("client"),
            workspace_id: 7,
            providers: vec![ProviderSummaryResource {
                provider_id: "example".to_owned(),
                defaults: ProviderDefaultsResource {
                    adapter: Some("openai".to_owned()),
                    model: "default-model".to_owned(),
                },
                adapters: Vec::new(),
            }],
        }
    }

    fn part(part_id: i64, kind: &str, run_id: Option<i64>) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: kind.to_owned(),
            role: "assistant".to_owned(),
            state: "completed".to_owned(),
            content: serde_json::json!({}),
            presentation: None,
            summary: None,
            created_at_ms: part_id,
            parent_part_id: None,
            run_id,
        }
    }

    #[test]
    fn rpc_backend_resolves_models_from_public_provider_metadata() {
        let backend = backend_with_public_provider_metadata();
        assert_eq!(
            backend
                .resolve_model_target("example")
                .expect("provider default"),
            ModelRef {
                provider_id: "example".to_owned(),
                adapter_id: Some("openai".to_owned()),
                model_id: "default-model".to_owned(),
            }
        );
        assert_eq!(
            backend
                .resolve_model_target("example/override-model")
                .expect("qualified model"),
            ModelRef {
                provider_id: "example".to_owned(),
                adapter_id: Some("openai".to_owned()),
                model_id: "override-model".to_owned(),
            }
        );
        assert!(matches!(
            backend.resolve_model_target("missing"),
            Err(AppServerError::InvalidParams(_))
        ));
    }

    #[test]
    fn submit_result_contains_only_the_newest_run_parts() {
        let parts = vec![
            part(1, "run", Some(1)),
            part(2, "text", Some(1)),
            part(3, "run", Some(3)),
            part(4, "reasoning", Some(3)),
            part(5, "text", Some(3)),
        ];
        let (run_id, latest) = latest_run_parts(&parts);
        assert_eq!(run_id, Some(3));
        assert_eq!(
            latest.iter().map(|part| part.part_id).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[tokio::test]
    async fn rpc_server_rejects_database_ownership_instead_of_bootstrapping_runtime() {
        let error = run(RpcServerRequest {
            config_override_expressions: Vec::new(),
            database_url: Some("sqlite::memory:".to_owned()),
            database_path: None,
            args: agena_cli::RpcServerArgs {
                server: Some("http://127.0.0.1:3210".to_owned()),
                server_token: None,
                server_password: None,
                workspace: None,
                transport: RpcServerTransport::Stdio,
            },
        })
        .await
        .expect_err("database ownership must stay at the server");
        assert!(error.to_string().contains("belong to the server"));
    }
}
