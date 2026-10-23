use super::*;
use agena_domain::ContentId;
use agena_provider::CompletionUsage;

impl SessionManager {
    async fn assistant_text(
        &self,
        session: SessionId,
        content: ContentId,
        max: usize,
    ) -> Result<String, AppError> {
        let range = persist(|| self.kernel().reader.text_range(session, content, None, 1))
            .await
            .map_err(kernel_error)?;
        persist(|| async {
            let mut remaining = max.saturating_mul(3).saturating_add(4096);
            super::super::kernel::input::read_text(
                &self.kernel().reader,
                session,
                &range.stream,
                &mut remaining,
                Some(max),
            )
            .await
        })
        .await
        .map_err(kernel_error)
    }
    pub async fn final_text(&self, id: i64, max: usize) -> Result<Option<String>, AppError> {
        self.final_text_for(session_id(id)?, None, max).await
    }
    pub(super) async fn final_text_for(
        &self,
        session: SessionId,
        execution: Option<ExecutionId>,
        max: usize,
    ) -> Result<Option<String>, AppError> {
        let mut before = None;
        for _ in 0..32 {
            let page = persist(|| self.kernel().reader.history(session, before, 128))
                .await
                .map_err(kernel_error)?;
            // Pages select the newest window but expose it chronologically.
            // A continued execution can have several completed model rounds;
            // its final response is the newest text, not the first round.
            for entry in page.entries.iter().rev() {
                if execution.is_none_or(|id| entry.execution_id == id)
                    && let EntryPayload::Text { content_id } = entry.payload
                    && let text = self
                        .assistant_text(session, content_id, max.min(1024 * 1024))
                        .await?
                    && !text.trim().is_empty()
                {
                    return Ok(Some(text));
                }
            }
            if !page.has_more {
                return Ok(None);
            }
            before = page.next;
        }
        Ok(None)
    }
    pub async fn aggregate_usage(&self, id: i64) -> Result<CompletionUsage, AppError> {
        let session = session_id(id)?;
        let mut after = None;
        let mut usage = CompletionUsage::default();
        loop {
            let page = persist(|| self.kernel().reader.rounds(session, after, 200))
                .await
                .map_err(kernel_error)?;
            if page.is_empty() {
                break;
            }
            for round in page.iter() {
                if let Some(state) = &round.provider_state {
                    let state: super::super::kernel::KernelProviderState =
                        serde_json::from_value(state.clone())?;
                    if let Some(observed) = state.usage {
                        usage.add_assign(&observed);
                    }
                }
            }
            after = page.last().map(|r| r.id);
        }
        Ok(usage)
    }
}
pub(super) fn truncate(text: &mut String, max: usize) {
    if text.len() > max {
        let mut end = max;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
}
