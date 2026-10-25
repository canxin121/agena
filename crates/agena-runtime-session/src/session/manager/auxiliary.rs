//! Short, isolated model calls shared by title generation and future helpers.
use super::*;
use agena_domain::Role;
use agena_provider::{CompletionInputPart, CompletionInputRun, CompletionResponse};

const TIMEOUT: Duration = Duration::from_secs(30);

impl SessionManager {
    /// Run a bounded, tool-free helper without adding turns to the session or
    /// changing its model. An explicit auxiliary selection is used as a whole;
    /// otherwise the session's effective model and inference modes are reused.
    pub async fn complete_auxiliary(
        &self,
        id: i64,
        system: String,
        prompt: String,
        max_output_tokens: u32,
    ) -> Result<CompletionResponse, AppError> {
        if system.len().saturating_add(prompt.len()) > 128 * 1024 {
            return Err(AppError::Config(
                "auxiliary task input exceeds 128 KiB".into(),
            ));
        }
        let _permit = self
            .auxiliary_tasks
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::Config("auxiliary task capacity exhausted".into()))?;
        let metadata = self
            .kernel()
            .reader
            .metadata(session_id(id)?)
            .await
            .map_err(kernel_error)?;
        self.complete_auxiliary_with_state(
            &self.state.load_full(),
            metadata.selection.as_ref(),
            system,
            prompt,
            max_output_tokens,
        )
        .await
    }

    pub(super) async fn complete_auxiliary_with_state(
        &self,
        state: &ExecutionState,
        session_selection: Option<&ExecutionSelection>,
        system: String,
        prompt: String,
        max_output_tokens: u32,
    ) -> Result<CompletionResponse, AppError> {
        let fallback = state
            .config
            .default_selection
            .overlay_with_cascade(session_selection.unwrap_or(&ExecutionSelection::default()));
        let selection = if state.config.auxiliary_selection.is_empty() {
            &fallback
        } else {
            &state.config.auxiliary_selection
        };
        let model = state
            .providers
            .resolve_default_model_selection(selection)?
            .ok_or_else(|| AppError::Config("select a model for auxiliary tasks".into()))?;
        let mut options = SessionRunOptions {
            model,
            thinking_mode: selection.thinking_mode.clone(),
            speed_mode: selection.speed_mode.clone(),
            verbosity: selection.verbosity.clone(),
            model_supports_parallel_tool_calls: Some(false),
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: Some(max_output_tokens.clamp(256, 8192)),
        };
        self.apply_model_modes_with_state(&mut options, state)?;
        // Mode overrides must not silently expand a helper's output budget.
        crate::prompt_budget::align_output_token_overrides(
            &mut options.request_override,
            options.max_output_tokens.unwrap_or(1024),
        );
        let mut request = execution::completion_request(
            &options,
            Some(system),
            vec![CompletionInputRun {
                role: Role::User,
                parts: vec![CompletionInputPart::Text { text: prompt }],
                provider_state: Default::default(),
            }],
            Vec::new(),
        );
        request.disable_tools = true;
        let cancel = self.shutdown.child_token();
        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(AppError::Cancelled),
            result = tokio::time::timeout(TIMEOUT, crate::provider::with_request_cancellation(
                Some(cancel.clone()), state.providers.complete(&options.model, request),
            )) => match result {
                Ok(result) => result.map_err(AppError::from),
                Err(_) => Err(AppError::Internal("auxiliary model request timed out".into())),
            },
        };
        cancel.cancel();
        response
    }
}
