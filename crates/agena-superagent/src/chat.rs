use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result, Task, validate_key, validate_text};

/// An authenticated operator message in an existing Agena conversation.
/// Sender names and message bodies are never accepted as authorization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub session_id: i64,
    pub message_id: String,
    pub text: String,
}

impl ChatMessage {
    pub fn validate(&self) -> Result<()> {
        if self.session_id <= 0 {
            return Err(Error::Invalid("session_id must be positive"));
        }
        validate_key(&self.message_id)?;
        validate_text(&self.text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCommand {
    Conversation,
    Create(String),
    List,
    Status(Uuid),
    Cancel(Uuid),
}

impl ChatCommand {
    /// Intentionally deterministic. Do not let an LLM turn incidental chat,
    /// quoted commands, or instructions from retrieved content into authority.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let (command, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
        let rest = rest.trim();
        match command {
            "/task" => {
                validate_text(rest)?;
                Ok(Self::Create(rest.to_owned()))
            }
            "/tasks" if rest.is_empty() => Ok(Self::List),
            "/tasks" => Err(Error::Invalid("usage: /tasks")),
            "/status" | "/cancel" => {
                let id = rest.parse().map_err(|_| {
                    Error::Invalid("use /status or /cancel followed by a task UUID")
                })?;
                Ok(if command == "/status" {
                    Self::Status(id)
                } else {
                    Self::Cancel(id)
                })
            }
            _ => Ok(Self::Conversation),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatReply {
    Conversation {
        session_id: i64,
        execution_id: String,
        attempt_generation: i64,
    },
    Task {
        task: Box<Task>,
    },
    Tasks {
        items: Vec<Task>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_top_level_commands_create_tasks() {
        for text in [
            "你好",
            "明天可以看看测试",
            "Someone said /task delete",
            "> /task delete",
            "```\n/task delete\n```",
            "/taskforce",
        ] {
            assert_eq!(ChatCommand::parse(text).unwrap(), ChatCommand::Conversation);
        }
        assert_eq!(
            ChatCommand::parse("/task 修复测试").unwrap(),
            ChatCommand::Create("修复测试".into())
        );
        assert!(ChatCommand::parse("/task ").is_err());
        assert!(ChatCommand::parse("/cancel not-an-id").is_err());
        assert_eq!(ChatCommand::parse("/tasks").unwrap(), ChatCommand::List);
    }
}
