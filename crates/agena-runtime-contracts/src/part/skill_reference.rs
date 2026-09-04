use serde::{Deserialize, Serialize};

/// A message-scoped reference to a Skill explicitly selected by the user.
///
/// Skill bodies are intentionally not copied into new messages. The model gets
/// the stable catalog metadata below and can call `agena.skills.get` when it
/// needs the current Skill body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillReference {
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
/// A message part referencing skills.
pub struct SkillReferencePart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillReference>,
}

impl SkillReferencePart {
    /// Render a provider-safe lazy Skill reference block.
    pub fn model_context_text(&self) -> String {
        let skills = self
            .skills
            .iter()
            .map(|skill| {
                serde_json::json!({
                    "name": skill.name,
                    "description": skill.description,
                    "content_hash": skill.content_hash,
                    "source": skill.source,
                    "aliases": skill.aliases,
                })
            })
            .collect::<Vec<_>>();
        let payload = serde_json::json!({
            "semantics": "message_scoped_user_selected_skill_reference",
            "guidance": [
                "The user explicitly selected these Skill references for this message.",
                "Skill bodies are not embedded in this message. Before applying a selected Skill, call `agena.skills.get` with its `name` and use the returned body as task guidance.",
                "The reference `content_hash` identifies the catalog version selected by the user; compare it with the tool result when consistency matters."
            ],
            "skills": skills,
        });
        let encoded = serde_json::to_string_pretty(&payload)
            .expect("skill-reference payload is always JSON serializable")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        format!(
            "<agena_skill_references>\n{}\n</agena_skill_references>",
            encoded
        )
    }

    pub fn summary(&self) -> String {
        match self.skills.as_slice() {
            [] => "0 Skill references".to_string(),
            [skill] => format!("Skill: {}", skill.name),
            skills => format!(
                "{} Skills: {}",
                skills.len(),
                skills
                    .iter()
                    .take(3)
                    .map(|skill| skill.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SkillReference, SkillReferencePart};

    #[test]
    fn model_context_is_message_scoped_lazy_reference() {
        let part = SkillReferencePart {
            skills: vec![SkillReference {
                name: "review".to_string(),
                description: "Review changes".to_string(),
                content_hash: "sha256".to_string(),
                source: "bundled".to_string(),
                aliases: vec!["code-review".to_string()],
            }],
        };

        let rendered = part.model_context_text();
        assert!(rendered.contains("message_scoped_user_selected_skill_reference"));
        assert!(rendered.contains("user explicitly selected"));
        assert!(rendered.contains("agena.skills.get"));
        assert!(rendered.contains("Review changes"));
        assert!(rendered.contains("sha256"));
        assert_eq!(rendered.matches("</agena_skill_references>").count(), 1);
        assert_eq!(part.summary(), "Skill: review");
        let reference_without_body = serde_json::from_value::<SkillReference>(serde_json::json!({
            "name": "review",
            "description": "Review changes",
            "content_hash": "sha256",
            "source": "bundled"
        }))
        .expect("lazy Skill refs do not require instructions");
        assert!(
            serde_json::from_value::<SkillReference>(serde_json::json!({
                "name": "legacy",
                "instructions": "Legacy instructions.",
                "content_hash": "sha256",
                "source": "bundled",
                "allowed_tools": ["agena.fs.read"]
            }))
            .is_err()
        );
    }
}
