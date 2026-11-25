use super::*;
use crate::{ChoiceOverlayAction, Overlay, choice_item_with_value};

const CREATE_PROFILE: &str = ":create";

impl App {
    pub(super) fn show_bot_agent_profiles(&mut self, catalog: &serde_json::Value) {
        let Some(form) = self.bot.form.as_ref() else {
            return;
        };
        let mut choices = crate::app_agent_profiles::profile_choices(&self.i18n, catalog, false);
        choices[0] = choice_item_with_value(
            self.i18n.text("bot-agent-profile-default"),
            "",
            String::new(),
        );
        choices.push(choice_item_with_value(
            self.i18n.text("bot-agent-profile-create"),
            CREATE_PROFILE,
            self.i18n.text("bot-agent-profile-create-detail"),
        ));
        self.overlay = Some(Overlay::Choice(self.build_choice_overlay(
            self.i18n.text("settings-agent-profile-label"),
            self.i18n.text("bot-agent-profile-picker"),
            Some(form.agent_profile.clone().unwrap_or_default()),
            choices,
            ChoiceOverlayAction::BotAgentProfile,
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        )));
    }

    pub(crate) fn choose_bot_agent_profile(&mut self, name: String) {
        let Some(form) = self.bot.form.as_mut() else {
            return;
        };
        if name == CREATE_PROFILE {
            let suffix = Uuid::new_v4().simple().to_string();
            let id = format!("bot-{}", &suffix[..8]);
            let fragment = format!("profile-{id}");
            let draft = serde_json::json!({
                "id": id,
                "title": form.name.trim(),
                "mode": "primary",
                "prompts": {
                    "order": [fragment],
                    "inline": [{"id": fragment, "body": ""}]
                }
            });
            form.edit = Some(Editor::from_text(
                serde_json::to_string_pretty(&draft).expect("JSON draft"),
            ));
        } else {
            form.agent_profile = (!name.is_empty()).then_some(name);
            form.edit = None;
            form.request_key = Uuid::new_v4().to_string();
        }
    }
}

pub(super) fn parse_profile_draft(input: &str) -> anyhow::Result<(String, serde_json::Value)> {
    let mut profile: serde_json::Map<String, serde_json::Value> = serde_json::from_str(input)?;
    let id = profile
        .remove("id")
        .and_then(|id| id.as_str().map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("Enter a profile ID in `id`."))?;
    agena_prompt_layers::FragmentId::parse(&id)?;
    let mode = profile
        .entry("mode")
        .or_insert(serde_json::json!("primary"));
    anyhow::ensure!(
        matches!(mode.as_str(), Some("primary" | "all")),
        "Select the primary or all mode for this profile."
    );
    Ok((id, serde_json::Value::Object(profile)))
}
