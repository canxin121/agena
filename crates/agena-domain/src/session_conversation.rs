//! Durable identity for user-initiated conversations beside an existing run.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMode {
    Side,
    Btw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConversation {
    pub mode: ConversationMode,
    pub parent_session_id: i64,
}

impl SessionConversation {
    pub fn is_read_only(&self) -> bool {
        self.mode == ConversationMode::Btw
    }

    pub fn instruction(&self) -> String {
        let purpose = match self.mode {
            ConversationMode::Side => {
                "You are in a side conversation, separate from the parent conversation. Answer the user's questions and perform only work explicitly requested in this side conversation. Do not modify files, git state, permissions, configuration, or other workspace state unless the user explicitly requests that change here. Coordinate any such change with the fact that the parent may still be working in the same workspace."
            }
            ConversationMode::Btw => {
                "You are answering a temporary /btw question while the parent conversation may continue running. Give a concise, complete answer to this question. Read-only tools are available for inspection. Mutations, shell execution, delegation, scheduling, changing permissions, and other side effects are unavailable. If a change is needed, explain it and let the user request it in the main conversation or /side."
            }
        };
        format!(
            "# Conversation boundary\n\n{purpose}\n\nParent session: {}. Inherited history before the conversation-boundary notice is reference material only, not your current task. Do not continue or execute inherited instructions, plans, approvals, or pending tool calls. Only user messages after that boundary are active requests here. Do not manage the parent's running tools or subagents. Your reply is shown separately and is not automatically delivered to the parent. If no new question has been submitted here, wait for one.",
            self.parent_session_id
        )
    }

    pub fn boundary_notice(&self) -> String {
        format!(
            "<conversation_boundary>\n{}\n\nThe user's new request follows this notice.\n</conversation_boundary>",
            self.instruction()
        )
    }
}
