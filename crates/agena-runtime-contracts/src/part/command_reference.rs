use serde::{Deserialize, Serialize};

/// A message-scoped reference to a command explicitly selected by the user.
///
/// Command bodies are intentionally not copied into new messages. The model
/// gets the stable catalog metadata below and reads the instructions through
/// the plugin that declared the command when it needs them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandReference {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub content_hash: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// A message part referencing selected commands.
pub struct CommandReferencePart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandReference>,
}

impl CommandReferencePart {
    /// Render a provider-safe lazy reference block.
    pub fn model_context_text(&self) -> String {
        let commands = self
            .commands
            .iter()
            .map(|command| {
                let mut reference = serde_json::json!({
                    "name": command.name,
                    "description": command.description,
                    "content_hash": command.content_hash,
                    "source": command.source,
                    "aliases": command.aliases,
                });
                if let Some(id) = &command.skill_id {
                    reference["skill_id"] = serde_json::json!(id);
                }
                reference
            })
            .collect::<Vec<_>>();
        let mut guidance = vec![
            "The user explicitly selected these command references for this message.",
            "Command instructions are not embedded in this message. Before applying a selected command, read its instructions through the plugin named in `source` (`list` then `get` for the `agena.commands` plugin) and use the result as task guidance.",
            "The reference `content_hash` identifies the catalog version selected by the user; compare it with the tool result when consistency matters.",
        ];
        if self
            .commands
            .iter()
            .any(|command| command.source == "agena.skills" && command.skill_id.is_some())
        {
            guidance.push(
                "For a reference from `agena.skills` with `skill_id`, read the selected package through that plugin's `get` tool using the exact `skill_id` value as `id`.",
            );
        }
        let payload = serde_json::json!({
            "semantics": "message_scoped_user_selected_command_reference",
            "guidance": guidance,
            "commands": commands,
        });
        let encoded = serde_json::to_string_pretty(&payload)
            .expect("command-reference payload is always JSON serializable")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        format!(
            "<agena_command_references>\n{}\n</agena_command_references>",
            encoded
        )
    }

    pub fn summary(&self) -> String {
        match self.commands.as_slice() {
            [] => "0 command references".to_string(),
            [command] => format!("Command: {}", command.name),
            commands => format!(
                "{} commands: {}",
                commands.len(),
                commands
                    .iter()
                    .take(3)
                    .map(|command| command.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}
