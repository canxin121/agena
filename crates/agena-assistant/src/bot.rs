//! Stable, instance-scoped Bot identities. Persona is presentation, never authority.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Hard storage/wire ceiling for one Bot's additional system instructions.
///
/// Persona prompts are written in prose, and 4 KiB is far too small for the
/// character sheets people actually use. The real limit is the selected
/// model's context window, which is enforced when the prompt is assembled:
/// [`agena_runtime_session`] reports a `PromptBudgetExceeded` telling the user
/// how many tokens the instructions need and what the model allows. This
/// constant only stops a pathological payload from becoming an unbounded
/// registry record; it is deliberately generous (about 64k characters of CJK
/// prose) and is not meant to be hit in normal use.
pub const MAX_BOT_INSTRUCTIONS_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BotPersona {
    pub instructions: String,
}
impl BotPersona {
    pub fn validate(&self, name: &str) -> Result<()> {
        if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(Error::Invalid(
                "Bot name must contain 1..128 non-control UTF-8 bytes",
            ));
        }
        if self.instructions.len() > MAX_BOT_INSTRUCTIONS_BYTES {
            // The message carries the number, so it cannot be a `&'static str`.
            return Err(Error::Policy(format!(
                "Bot additional system instructions must be at most {MAX_BOT_INSTRUCTIONS_BYTES} UTF-8 bytes"
            )));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BotProfile {
    #[schemars(with = "String")]
    pub id: Uuid,
    pub name: String,
    pub persona: BotPersona,
    pub revision: i64,
    pub archived: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BotList {
    pub items: Vec<BotProfile>,
    #[schemars(with = "Option<String>")]
    pub default_bot_id: Option<Uuid>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BotCreate {
    pub request_key: String,
    pub name: String,
    pub persona: Option<BotPersona>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BotUpdate {
    pub request_key: String,
    pub expected_revision: i64,
    pub name: String,
    pub persona: BotPersona,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BotArchive {
    pub request_key: String,
    pub expected_revision: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BotSession {
    pub session_id: i64,
    #[schemars(with = "String")]
    pub bot_id: Uuid,
    pub name: String,
}
impl BotSession {
    pub fn new(session_id: i64, profile: &BotProfile) -> Self {
        Self {
            session_id,
            bot_id: profile.id,
            name: profile.name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn create_and_update_round_trip_only_instructions_and_reject_old_fields() {
        let create =
            json!({"request_key":"create", "name":"Bot", "persona":{"instructions":"Hello"}});
        let update = json!({"request_key":"update", "expected_revision":1, "name":"Bot", "persona":{"instructions":"Hello"}});
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<BotCreate>(create.clone()).unwrap())
                .unwrap(),
            create
        );
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<BotUpdate>(update.clone()).unwrap())
                .unwrap(),
            update
        );
        for (field, value) in [
            ("address_user", json!("")),
            ("tone", json!("balanced")),
            ("language", json!("auto")),
            ("unknown", json!(true)),
        ] {
            let mut old_create = create.clone();
            old_create["persona"][field] = value.clone();
            assert!(
                serde_json::from_value::<BotCreate>(old_create).is_err(),
                "{field}"
            );
            let mut old_update = update.clone();
            old_update["persona"][field] = value;
            assert!(
                serde_json::from_value::<BotUpdate>(old_update).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn persona_accepts_large_prose_prompts_and_rejects_pathological_payloads() {
        // A realistic long character sheet is accepted; the per-model context
        // window, not this storage ceiling, is what limits normal use.
        let prose = "你是一只鲸鱼娘。".repeat(2000);
        assert!(prose.len() > 4096);
        assert!(
            BotPersona {
                instructions: prose.clone()
            }
            .validate("Bot")
            .is_ok()
        );
        assert!(
            BotPersona {
                instructions: "界".repeat(MAX_BOT_INSTRUCTIONS_BYTES / 3)
            }
            .validate("Bot")
            .is_ok()
        );
        assert!(
            BotPersona {
                instructions: "a".repeat(MAX_BOT_INSTRUCTIONS_BYTES + 1)
            }
            .validate("Bot")
            .is_err()
        );
        assert!(BotPersona::default().validate("bad\nname").is_err());
    }
}
