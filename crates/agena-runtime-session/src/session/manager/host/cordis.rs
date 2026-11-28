use super::*;
use crate::{BuildCommandFollowup, SessionPluginCommandRequest, SessionPluginCommandService};
use agena_failure::Failure;
use agena_plugin_host::{
    agent_plugin::CordisFollowupBlock,
    sdk::{
        CommandInvokeInput, CommandResult, CommandStatus, PluginKey,
        cancellation::run_with_cancellation,
    },
};

use super::super::media::CordisAttachments;

// Bound the number of snapshots retained between admission and followup.
static CORDIS_INVOCATIONS: Semaphore = Semaphore::const_new(2);

#[async_trait::async_trait]
impl SessionPluginCommandService for SessionManager {
    async fn invoke_session_plugin_command(
        &self,
        request: SessionPluginCommandRequest,
        build_followup: BuildCommandFollowup<'_>,
    ) -> Result<CommandResult, Failure> {
        let failed = |error: AppError| error.failure();
        let session = session_id(request.session_id).map_err(failed)?;
        let state = self.state_for_session(session).await.map_err(failed)?;
        let context = self
            .execution_context_with_state(session, &state)
            .await
            .map_err(failed)?;
        let executor = state.executor.for_session_resource_context(&context);
        let host = executor.plugin_manager();
        let key: PluginKey = request
            .plugin_id
            .parse()
            .map_err(|error| AppError::Config(format!("Invalid plugin ID: {error}")).failure())?;
        if context.global
            || context.owner.is_some()
            || context.channel_authority.is_some()
            || !executor.plugin_is_enabled(&key)
            || !host.command_accepts_attachments(&key, &request.command_id)
        {
            return Err(AppError::Config(
                "This command does not accept attachments in the current session".into(),
            )
            .failure());
        }
        self.require_writable_bot_history(session.get())
            .await
            .map_err(failed)?;
        let _invocation = tokio::select! {
            _ = self.shutdown.cancelled() => return Err(AppError::Cancelled.failure()),
            permit = CORDIS_INVOCATIONS.acquire() => permit.map_err(|_| AppError::Cancelled.failure())?,
        };
        let attachments =
            CordisAttachments::admit(self, session, &state, &context, request.document)
                .await
                .map_err(failed)?;
        let mut result = run_with_cancellation(
            self.shutdown.child_token(),
            host.invoke_plugin_command_async(
                &request.plugin_id,
                CommandInvokeInput {
                    session_id: Some(request.session_id),
                    call_id: None,
                    workspace_root: Some(executor.workspace_root().to_string_lossy().into_owned()),
                    command_id: request.command_id,
                    slash: request.slash,
                    raw: request.raw,
                    prompt_context: String::new(),
                    cordis_attachments: Some(attachments.metadata()),
                    input: request.input,
                },
            ),
        )
        .await
        .map_err(|error| *error.failure)?;
        let Some(output) = result
            .output
            .take()
            .filter(|_| result.status == CommandStatus::Succeeded)
        else {
            return Ok(result);
        };
        let blocks: Vec<CordisFollowupBlock> =
            serde_json::from_value(output).map_err(|error| AppError::SerdeJson(error).failure())?;
        let mut followup = attachments.followup(session, blocks).map_err(failed)?;
        let submission = build_followup(std::mem::take(&mut followup.document)).await?;
        attachments
            .publish_files(session, &followup)
            .await
            .map_err(failed)?;
        self.submit_with_snapshots(submission, None, None, followup.images)
            .await
            .map_err(failed)?;
        Ok(result)
    }
}
