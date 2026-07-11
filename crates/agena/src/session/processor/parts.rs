use super::{
    AppError, DateTime, ExecutionStatus, Message, MessagePart, MessageStatus, PartContent,
    ReasoningPart, SessionProcessor, Utc,
};

impl SessionProcessor {
    pub(crate) fn start_text_part(
        &self,
        assistant: &mut Message,
        part_id: i64,
        created_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let mut part = MessagePart::from_content(
            part_id,
            assistant.id,
            created_at,
            ExecutionStatus::Pending,
            PartContent::text(String::new()),
        );
        part.part_index = assistant.parts.len() as i32;
        assistant.parts.push(part);
        if assistant.state == MessageStatus::Pending {
            let _ = assistant.transition_state(MessageStatus::InProgress);
        }
        Ok(())
    }

    pub(crate) fn start_reasoning_part(
        &self,
        assistant: &mut Message,
        part_id: i64,
        created_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let mut part = MessagePart::from_content(
            part_id,
            assistant.id,
            created_at,
            ExecutionStatus::Pending,
            PartContent::Reasoning(ReasoningPart {
                summary: Vec::new(),
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        part.part_index = assistant.parts.len() as i32;
        assistant.parts.push(part);
        if assistant.state == MessageStatus::Pending {
            let _ = assistant.transition_state(MessageStatus::InProgress);
        }
        Ok(())
    }

    pub(crate) fn append_text_delta(
        &self,
        assistant: &mut Message,
        part_id: i64,
        delta: &str,
    ) -> Result<(), AppError> {
        let part = assistant
            .parts
            .iter_mut()
            .find(|part| part.id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "active text part missing from assistant snapshot: {part_id}"
                ))
            })?;
        if part.status == ExecutionStatus::Pending {
            part.transition_status(ExecutionStatus::InProgress)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        if !part.append_text_delta(delta) {
            return Err(AppError::Internal(format!(
                "failed to append text delta to part {part_id}: kind mismatch"
            )));
        }
        Ok(())
    }

    pub(crate) fn append_reasoning_delta(
        &self,
        assistant: &mut Message,
        part_id: i64,
        delta: &str,
    ) -> Result<(), AppError> {
        let part = assistant
            .parts
            .iter_mut()
            .find(|part| part.id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "active reasoning part missing from assistant snapshot: {part_id}"
                ))
            })?;
        if part.status == ExecutionStatus::Pending {
            part.transition_status(ExecutionStatus::InProgress)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        if !part.append_reasoning_summary_delta(delta.to_string()) {
            return Err(AppError::Internal(format!(
                "failed to append reasoning delta to part {part_id}: kind mismatch"
            )));
        }
        Ok(())
    }
}
