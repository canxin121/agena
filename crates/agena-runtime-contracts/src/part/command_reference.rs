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
                serde_json::json!({
                    "name": command.name,
                    "description": command.description,
                    "content_hash": command.content_hash,
                    "source": command.source,
                    "aliases": command.aliases,
                })
            })
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "semantics": "message_scoped_user_selected_command_reference",
            "guidance": [
                "The user explicitly selected these command references for this message.",
                "Command instructions are not embedded in this message. Before applying a selected command, read its instructions through the plugin named in `source` (`list` then `read` for the `agena.commands` plugin) and use the result as task guidance.",
                "The reference `content_hash` identifies the catalog version selected by the user; compare it with the tool result when consistency matters."
            ],
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

#[cfg(test)]
mod tests {
    use super::{CommandReference, CommandReferencePart};

    #[test]
    fn model_context_is_message_scoped_lazy_reference() {
        let part = CommandReferencePart {
            commands: vec![CommandReference {
                name: "review".to_string(),
                description: "Review changes".to_string(),
                content_hash: "sha256".to_string(),
                source: "bundled".to_string(),
                aliases: vec!["code-review".to_string()],
            }],
        };

        let rendered = part.model_context_text();
        assert!(rendered.contains("message_scoped_user_selected_command_reference"));
        assert!(rendered.contains("user explicitly selected"));
        assert!(rendered.contains("read its instructions through the plugin"));
        assert!(rendered.contains("Review changes"));
        assert!(rendered.contains("sha256"));
        assert_eq!(rendered.matches("</agena_command_references>").count(), 1);
        assert_eq!(part.summary(), "Command: review");
        serde_json::from_value::<CommandReference>(serde_json::json!({
            "name": "review",
            "description": "Review changes",
            "content_hash": "sha256",
            "source": "bundled"
        }))
        .expect("lazy command refs do not require instructions");
        assert!(
            serde_json::from_value::<CommandReference>(serde_json::json!({
                "name": "obsolete-shape",
                "instructions": "Unexpected instructions.",
                "content_hash": "sha256",
                "source": "bundled",
                "allowed_tools": ["agena.fs.read"]
            }))
            .is_err()
        );
    }
}
