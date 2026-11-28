use super::*;
use agena_plugin_host::sdk::{HookSubscription, ImportedTurnCompletedInput};

const MAX_TEXT_BYTES: usize = 32 * 1024;

impl SessionManager {
    pub(super) fn observe_imported_turn(
        &self,
        execution: ExecutionView,
        state: Arc<ExecutionState>,
    ) {
        if !state
            .executor
            .plugin_manager()
            .has_subscription(HookSubscription::IMPORTED_TURN_COMPLETED)
        {
            return;
        }
        let manager = self.clone();
        self.control_tasks.spawn(async move {
            if let Err(error) = manager.dispatch_committed_turn(execution, state).await {
                tracing::warn!(%error, "could not dispatch imported turn completion");
            }
        });
    }

    async fn dispatch_committed_turn(
        &self,
        execution: ExecutionView,
        state: Arc<ExecutionState>,
    ) -> Result<(), AppError> {
        let session = execution.session_id;
        let context = self.execution_context_with_state(session, &state).await?;
        if context.global
            || context.owner.is_some()
            || context.relation_kind.is_subagent()
            || context
                .conversation
                .as_ref()
                .is_some_and(|conversation| conversation.is_read_only())
        {
            return Ok(());
        }
        let entry = read_with_busy_retry(session, "imported_completed_user", || {
            self.kernel().reader.entry(session, execution.message_id)
        })
        .await?;
        let EntryPayload::User {
            document, author, ..
        } = &entry.payload
        else {
            return Ok(());
        };
        if author
            .as_ref()
            .is_some_and(|author| author.kind.is_system())
        {
            return Ok(());
        }
        let mut user_text = document.text();
        user_text.truncate(user_text.floor_char_boundary(user_text.len().min(MAX_TEXT_BYTES)));
        let assistant_text = self
            .final_execution_text(
                session,
                execution.id,
                Some(execution.generation),
                MAX_TEXT_BYTES,
            )
            .await?
            .unwrap_or_default();
        if user_text.trim().is_empty() && assistant_text.trim().is_empty() {
            return Ok(());
        }
        let executor = state.executor.for_session_resource_context(&context);
        executor
            .plugin_manager()
            .dispatch_imported_turn_completed(
                ImportedTurnCompletedInput {
                    session_id: session.get(),
                    user_text,
                    assistant_text,
                },
                |key| executor.plugin_is_enabled(key),
            )
            .await;
        Ok(())
    }
}
