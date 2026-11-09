//! Trusted integration dispatch into an existing session's native FIFO.
//! The dispatcher validates the bounded grant; external content is never a
//! direct user message and cannot supply user-quote authority to tools.
use super::*;

impl SessionManager {
    pub async fn submit_integration_instruction(
        &self,
        target: i64,
        key: &str,
        binding_id: &str,
        epoch: i64,
        text: &str,
    ) -> Result<ExecutionView, AppError> {
        if text.trim().is_empty() || text.len() > 32768 {
            return Err(AppError::Config(
                "integration instruction exceeds its bounded input contract".into(),
            ));
        }
        self.verify_channel_session(target).await?;
        if self.is_reserved_bot_session(target).await? {
            return Err(AppError::Config(
                "operator Bot cannot be an automated execution target".into(),
            ));
        }
        let session = session_id(target)?;
        let dispatch = self.session_dispatch(session);
        let _submission = dispatch.submission.lock().await;
        self.require_writable_bot_history(target).await?;
        let configuration = self
            .kernel()
            .reader
            .configuration(session)
            .await
            .map_err(kernel_error)?;
        if configuration.relation_kind.is_subagent()
            || configuration.agent.is_some()
            || configuration.conversation.is_some()
            || configuration
                .channel_authority
                .as_ref()
                .is_some_and(|a| a.actor != "@service")
        {
            return Err(AppError::Config(
                "automation requires an ordinary project or authorized channel service session"
                    .into(),
            ));
        }
        let metadata = self
            .kernel()
            .reader
            .metadata(session)
            .await
            .map_err(kernel_error)?;
        if metadata.workspace_id.is_none() {
            return Err(AppError::Config(
                "automation requires an explicit project session; select one in /integrations"
                    .into(),
            ));
        }
        self.restore_queued_executions(session).await?;
        let mut options = self
            .options_for_session(session, &metadata.selection.clone().unwrap_or_default())
            .await?;
        self.prepare_options(session, &mut options).await?;
        if metadata.selection.is_none() {
            self.update_session_selection(target, options.clone())
                .await?;
        }
        let permit = self.reserve_execution()?;
        let payload = serde_json::json!({"kind":"integration_instruction","binding_id":binding_id,"epoch":epoch,"text":text});
        let command = request_key("integration-instruction", key);
        let execution = persist(|| {
            self.kernel().executor.deliver_control_notice(
                command,
                session,
                key.into(),
                payload.clone(),
                text.into(),
            )
        })
        .await
        .map_err(kernel_error)?
        .value;
        if matches!(
            execution.phase,
            ExecutionPhase::Queued | ExecutionPhase::Running
        ) {
            self.supervise(execution.clone(), options, false, permit)
                .await?;
        }
        Ok(execution)
    }

    pub async fn integration_execution_result(
        &self,
        id: i64,
        execution: &str,
    ) -> Result<Option<String>, AppError> {
        let execution = execution
            .parse()
            .map_err(|e| AppError::Config(format!("invalid integration execution: {e}")))?;
        self.final_text_for(session_id(id)?, Some(execution), 24 * 1024)
            .await
    }

    pub async fn integration_execution_questions(
        &self,
        id: i64,
    ) -> Result<Vec<InteractionView>, AppError> {
        Ok(self
            .kernel()
            .reader
            .control_snapshot(session_id(id)?, 32)
            .await
            .map_err(kernel_error)?
            .pending_interactions
            .interactions
            .clone())
    }
}
