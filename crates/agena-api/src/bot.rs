//! Shared persistent named Bot contracts.
pub use agena_assistant::bot::*;
use serde::{Deserialize, Serialize};

/// Start a new conversation for a Bot.
///
/// Only the title is user-supplied: the conversation's identity, owner and
/// entrance shape are derived by the server from the Bot being opened.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BotConversationCreate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
