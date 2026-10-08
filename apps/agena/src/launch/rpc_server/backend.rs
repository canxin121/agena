use crate::error::AgenaProcessError;
use agena_api::{
    commands::{Command, CommandResult, ResolveWorkspaceParams},
    live::SessionPartsResource,
    queries::{ListSessionsParams, Query, QueryResult, ReadPartsParams},
    resource::{
        ModelRef, PermissionReply, PermissionReplyKind, PermissionScope, ProviderSummaryResource,
        RunOptions, SessionState,
    },
};
use agena_api_server::jsonrpc::protocol::{
    CancelRunParams, CancelRunResult, CreateSessionParams, CreateSessionResult,
    ListSessionsParams as AppListSessionsParams, ListSessionsResult as AppListSessionsResult,
    PermissionDecision as AppPermissionDecision, PermissionRememberScope, PermissionReplyParams,
    PermissionReplyResult, SessionListItem, SubmitRunParams, SubmitRunResult,
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
            .map_err(|err| AgenaProcessError::configuration_error(&err)),
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
            return Ok(ModelRef {
                provider_id: provider_id.trim().to_owned(),
                adapter_id: None,
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
        let _ = provider;
        Err(AppServerError::InvalidParams(format!(
            "model is required for provider `{target}`; pass an explicit provider/model target"
        )))
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
                    ..Default::default()
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
        Ok(SubmitRunResult {
            session_id: execution.session.id,
            receipt: execution.receipt,
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
                    status: session.state.as_str().to_owned(),
                    updated_at: session.updated_at,
                })
                .collect(),
        })
    }

    async fn read_parts(
        &self,
        params: ReadPartsParams,
    ) -> Result<SessionPartsResource, AppServerError> {
        let response = self
            .client
            .query(Query::ReadParts(params))
            .await
            .map_err(client_backend_error)?;
        let QueryResult::Parts(page) = response else {
            return Err(AppServerError::Backend(
                "server returned the wrong Part window result".to_owned(),
            ));
        };
        Ok(page)
    }

    async fn read_content(
        &self,
        params: agena_api::content::ReadContentParams,
    ) -> Result<agena_domain::ContentPage, AppServerError> {
        let response = self
            .client
            .query(Query::ReadContent(params))
            .await
            .map_err(client_backend_error)?;
        let QueryResult::Content(page) = response else {
            return Err(AppServerError::Backend(
                "server returned the wrong content result".into(),
            ));
        };
        Ok(page)
    }

    async fn read_content_text(
        &self,
        params: agena_api::content::ReadContentTextParams,
    ) -> Result<agena_domain::ContentTextPage, AppServerError> {
        self.client
            .read_content_text(params)
            .await
            .map_err(client_backend_error)
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
    let detail = error.operator_diagnostic();
    AgenaProcessError::Configuration(format!("{context}: {detail}"))
}

fn client_backend_error(error: ClientError) -> AppServerError {
    AppServerError::Backend(error.operator_diagnostic())
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

    fn backend_with_public_provider_metadata() -> AgenaAppServerBackend {
        AgenaAppServerBackend {
            client: AgenaClient::new("http://127.0.0.1:3210").expect("client"),
            workspace_id: 7,
            providers: vec![ProviderSummaryResource {
                provider_id: "example".to_owned(),
                adapters: Vec::new(),
            }],
        }
    }

    #[test]
    fn rpc_backend_requires_explicit_model_targets() {
        let backend = backend_with_public_provider_metadata();
        assert_eq!(
            backend
                .resolve_model_target("example/override-model")
                .expect("qualified model"),
            ModelRef {
                provider_id: "example".to_owned(),
                adapter_id: None,
                model_id: "override-model".to_owned(),
            }
        );
        assert!(matches!(
            backend.resolve_model_target("example"),
            Err(AppServerError::InvalidParams(_))
        ));
        assert!(matches!(
            backend.resolve_model_target("missing"),
            Err(AppServerError::InvalidParams(_))
        ));
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
