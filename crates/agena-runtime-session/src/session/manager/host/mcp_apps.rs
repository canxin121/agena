use agena_domain::{
    mcp_app_tools::{McpAppPrompt, McpAppToolResult, McpAppViewScope},
    mcp_elicitation::{McpElicitation, McpElicitationAction, McpElicitationReply},
    mcp_sampling::{McpSamplingReply, McpSamplingRequest},
};
use agena_mcp_client::{McpElicitationHandler, McpError, McpResult};
use serde_json::{Map, Value};
use uuid::Uuid;

use super::{
    AppError, AskUserResponse, CancellationToken, SessionManager,
    mcp_elicitation::prompt_mcp_input,
};
use crate::McpAppViewError;
use crate::part::AskUserToolInput;

impl SessionManager {
    pub(super) async fn sample_from_mcp_app(
        &self,
        scope: McpAppViewScope,
        params: Value,
        cancellation: CancellationToken,
    ) -> McpResult<Value> {
        let name = self.mcp_app_views.view_name(&scope)
            .map_err(|_| McpError::SamplingDeclined)?;
        let interactions = ViewUserInteractions { manager: self, scope: &scope };
        agena_mcp_client::manager::respond_to_app_sampling(
            &format!("App {name}"), params, &interactions, cancellation,
        ).await
    }

    pub async fn resolve_mcp_app_elicitation(
        &self,
        scope: &McpAppViewScope,
        source: &str,
        request: McpElicitation,
        cancellation: CancellationToken,
    ) -> Result<McpElicitationReply, AppError> {
        prompt_mcp_input(source, &request, |dialog, _| {
            self.request_mcp_app_input(scope, dialog, &cancellation)
        }).await
    }

    pub(super) async fn request_mcp_app_input(
        &self,
        scope: &McpAppViewScope,
        input: AskUserToolInput,
        cancellation: &CancellationToken,
    ) -> Result<AskUserResponse, AppError> {
        let [question] = input.questions.as_slice() else {
            return Err(AppError::Config("An MCP App prompt requires one question.".into()));
        };
        let prompt = McpAppPrompt { title: input.title, question: question.clone() };
        let answer = agena_async::permission_wait::waiting(
            self.mcp_app_views.prompt(scope, prompt, cancellation),
        ).await.map_err(app_view_error)?;
        Ok(match answer {
            Some(reply) => AskUserResponse { reply, ..Default::default() },
            None => AskUserResponse { cancelled: true, ..Default::default() },
        })
    }

    pub async fn call_mcp_app_view_tool(
        &self,
        session: i64,
        call: i64,
        target: Uuid,
        arguments: Map<String, Value>,
        cancellation: CancellationToken,
    ) -> Result<McpAppToolResult, AppError> {
        let (_, control) = self.live_tool(session, call)?;
        let interactions = AppViewInteractions {
            manager: self,
            session,
            call,
        };
        let wait = async {
            tokio::select! {
                biased;
                _ = control.cancel.cancelled() => Err(AppError::Cancelled),
                result = self.mcp_app_views.call(session, call, target, arguments, &cancellation, &interactions) => {
                    result.map_err(app_view_error)
                }
            }
        };
        // An App may call its server while preparing the reply to this tool.
        self.with_agent_wait_permit(session, Some(call), Some(&control), wait)
            .await
    }
}

pub(super) fn app_view_error(error: McpAppViewError) -> AppError {
    match error {
        McpAppViewError::Cancelled => AppError::Cancelled,
        error => AppError::Config(error.to_string()),
    }
}

struct ViewUserInteractions<'a> {
    manager: &'a SessionManager,
    scope: &'a McpAppViewScope,
}

#[async_trait::async_trait]
impl McpElicitationHandler for ViewUserInteractions<'_> {
    async fn elicit(
        &self,
        app: &str,
        request: McpElicitation,
        cancellation: CancellationToken,
    ) -> McpElicitationReply {
        self.manager.resolve_mcp_app_elicitation(self.scope, app, request, cancellation)
            .await
            .unwrap_or_else(|_| McpElicitationReply::new(McpElicitationAction::Cancel))
    }

    async fn sample(
        &self,
        app: &str,
        request: McpSamplingRequest,
        cancellation: CancellationToken,
    ) -> McpResult<McpSamplingReply> {
        self.manager.resolve_mcp_app_sampling(
            self.scope, app, "Return model output to the MCP App", request, cancellation,
        )
            .await.ok().flatten().ok_or(McpError::SamplingDeclined)
    }
}

struct AppViewInteractions<'a> {
    manager: &'a SessionManager,
    session: i64,
    call: i64,
}

#[async_trait::async_trait]
impl McpElicitationHandler for AppViewInteractions<'_> {
    async fn elicit(
        &self,
        app: &str,
        request: McpElicitation,
        cancellation: CancellationToken,
    ) -> McpElicitationReply {
        let source = format!("App {app}");
        // Use the live tool's user interaction. The view does not have the
        // originating MCP server's identity or its imported hook authority.
        prompt_mcp_input(&source, &request, |dialog, _: &'static str| {
            self.manager.request_cancellable_host_user_input(
                self.session,
                self.call,
                dialog,
                &cancellation,
            )
        })
        .await
        .unwrap_or_else(|_| {
            McpElicitationReply::new(if cancellation.is_cancelled() {
                McpElicitationAction::Cancel
            } else {
                McpElicitationAction::Decline
            })
        })
    }

    async fn sample(
        &self,
        app: &str,
        request: McpSamplingRequest,
        cancellation: CancellationToken,
    ) -> McpResult<McpSamplingReply> {
        let source = format!("App {app}");
        self.manager
            .resolve_mcp_sampling(self.session, self.call, &source, request, cancellation)
            .await
            .ok()
            .flatten()
            .ok_or(McpError::SamplingDeclined)
    }
}
