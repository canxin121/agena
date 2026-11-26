use super::*;
use agena_domain::ContentId;

impl SessionManager {
    pub(super) async fn assistant_text(
        &self,
        session: SessionId,
        content: ContentId,
        max: usize,
    ) -> Result<String, AppError> {
        let reader = &self.kernel().reader;
        let head = match persist(|| reader.text_range(session, content, None, 1)).await {
            Ok(range) => range.stream.clone(),
            Err(KernelError::Invalid(_)) => (*persist(|| reader.watch_content(session, content))
                .await
                .map_err(kernel_error)?
                .head()
                .map_err(kernel_error)?)
            .clone(),
            Err(error) => return Err(kernel_error(error)),
        };
        persist(|| async {
            let mut remaining = if head.kind == agena_domain::ContentKind::Structured {
                agena_domain::MAX_DOCUMENT_BYTES.saturating_mul(3)
            } else {
                max.saturating_mul(3).saturating_add(4096)
            };
            super::super::kernel::input::read_text(
                &self.kernel().reader,
                session,
                &head,
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
        if let Some(execution) = execution {
            return self
                .final_execution_text(session, execution, None, max)
                .await;
        }
        let mut before = None;
        for _ in 0..32 {
            let page = persist(|| self.kernel().reader.history(session, before, 128))
                .await
                .map_err(kernel_error)?;
            // Pages select the newest window but expose it chronologically.
            // A continued execution can have several completed model rounds;
            // its final response is the newest text, not the first round.
            for entry in page.entries.iter().rev() {
                if let EntryPayload::Text { content_id } = entry.payload
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
    pub(super) async fn final_execution_text(
        &self,
        session: SessionId,
        execution: ExecutionId,
        generation: Option<AttemptGeneration>,
        max: usize,
    ) -> Result<Option<String>, AppError> {
        let sources = persist(|| {
            self.kernel()
                .reader
                .execution_text_sources(session, execution, generation)
        })
        .await
        .map_err(kernel_error)?;
        for &content in sources.iter() {
            let text = self
                .assistant_text(session, content, max.min(1024 * 1024))
                .await?;
            if !text.trim().is_empty() {
                return Ok(Some(text));
            }
        }
        Ok(None)
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
