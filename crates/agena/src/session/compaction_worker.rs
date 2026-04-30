use crate::AppError;
use crate::message::Message;

use super::prompt_window::{self, PromptCompactionPlan};

#[derive(Debug, Clone, Default)]
pub(crate) struct CompactionWorker;

impl CompactionWorker {
    pub(crate) async fn plan_compaction(
        &self,
        messages: Vec<Message>,
        keep_tail_messages: usize,
        max_prompt_chars: usize,
    ) -> Result<Option<PromptCompactionPlan>, AppError> {
        tokio::task::spawn_blocking(move || {
            prompt_window::plan_compaction(&messages, keep_tail_messages, max_prompt_chars)
        })
        .await
        .map_err(|err| AppError::Internal(format!("compaction worker failed: {err}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::role::Role;

    #[tokio::test]
    async fn worker_builds_compaction_plan_off_turn_path() {
        let mut first = Message::prompt_text(Role::User, "one");
        first.id = 1;
        let mut second = Message::prompt_text(Role::Assistant, "two");
        second.id = 2;
        let mut third = Message::prompt_text(Role::User, "three");
        third.id = 3;

        let plan = CompactionWorker
            .plan_compaction(vec![first, second, third], 1, 32_000)
            .await
            .expect("worker should complete")
            .expect("plan should exist");

        assert_eq!(plan.compacted_message_ids, vec![1, 2]);
        assert!(plan.summary_text.contains("## Goal"));
        assert!(plan.summary_text.contains("## Accomplished"));
    }
}
