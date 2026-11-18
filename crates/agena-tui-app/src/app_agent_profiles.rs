//! Select named session profiles through the server-owned profile catalog.

use crate::{App, ChoiceItem, ChoiceOverlayAction, I18n, Overlay, choice_item_with_value};

fn profile_catalog(
    sources: &agena_api::resource::ConfigJsonSources,
    global: bool,
) -> &serde_json::Value {
    if global {
        &sources.file
    } else {
        &sources.effective
    }
}

fn profile_choices(i18n: &I18n, effective: &serde_json::Value) -> Vec<ChoiceItem> {
    let mut choices = vec![choice_item_with_value(
        i18n.text("settings-agent-profile-none"),
        "",
        i18n.text("settings-agent-profile-none-detail"),
    )];
    if let Some(profiles) = effective
        .get("profiles")
        .and_then(serde_json::Value::as_object)
    {
        for (name, profile) in profiles {
            let label = profile
                .get("title")
                .and_then(serde_json::Value::as_str)
                .filter(|title| !title.trim().is_empty() && *title != name.as_str())
                .map(|title| format!("{title} ({name})"))
                .unwrap_or_else(|| name.clone());
            let description = profile
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            choices.push(choice_item_with_value(label, name.clone(), description));
        }
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_preserves_profile_ids_and_exposes_descriptions() {
        let choices = profile_choices(
            &I18n::english(),
            &serde_json::json!({
                "profiles": {
                    "reviewer": {"title": "Code review", "description": "Check regressions"},
                    "writer": {"title": "  "}
                }
            }),
        );
        assert_eq!(choices[0].value, "");
        let review = choices
            .iter()
            .find(|item| item.value == "reviewer")
            .unwrap();
        assert_eq!(review.label, "Code review (reviewer)");
        assert_eq!(review.detail, "Check regressions");
        assert!(review.search_text.contains("reviewer"));
        let writer = choices.iter().find(|item| item.value == "writer").unwrap();
        assert_eq!(writer.label, "writer");
    }

    #[test]
    fn clearing_remains_available_when_the_catalog_is_empty() {
        let choices = profile_choices(&I18n::english(), &serde_json::json!({}));
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].value, "");
        assert_eq!(choices[0].label, "No profile");
    }

    #[test]
    fn global_sessions_exclude_workspace_profiles_and_overrides() {
        let sources = agena_api::resource::ConfigJsonSources {
            config_path: Default::default(),
            config_found: true,
            project_config_path: Default::default(),
            project_config_found: true,
            applied_layers: vec![],
            file: serde_json::json!({"profiles":{"reviewer":{"title":"Global review"}}}),
            project_file: serde_json::json!({"profiles":{"project-only":{}}}),
            effective: serde_json::json!({"profiles":{
                "reviewer":{"title":"Project review"},"project-only":{}
            }}),
        };
        let global = profile_choices(&I18n::english(), profile_catalog(&sources, true));
        assert_eq!(global.len(), 2);
        assert_eq!(global[1].label, "Global review (reviewer)");
        let workspace = profile_choices(&I18n::english(), profile_catalog(&sources, false));
        assert_eq!(workspace.len(), 3);
        assert!(
            workspace
                .iter()
                .any(|choice| choice.value == "project-only")
        );
        assert!(
            workspace
                .iter()
                .any(|choice| choice.label == "Project review (reviewer)")
        );
    }
}

impl App {
    pub(crate) fn open_session_profile_picker(&mut self, session_id: i64) {
        self.dispatch_backend_operation(
            move |application| async move {
                application.refresh_config_sources().await?;
                let sources = crate::app_backend::config::config_json_sources(&application)?;
                let summary = application
                    .client()
                    .query(agena_api::queries::Query::GetSession(
                        agena_api::queries::GetSessionParams { session_id },
                    ))
                    .await?;
                let agena_api::queries::QueryResult::Session(session) = summary else {
                    anyhow::bail!("server returned the wrong session result");
                };
                // A removed profile may make the existing execution state
                // invalid. Keep the picker available so it can be repaired.
                let current = application.get_session_state(session_id).await;
                Ok::<_, anyhow::Error>((sources, current, session.workspace_id.is_none()))
            },
            move |app, result| match result {
                Ok((sources, current, global)) => {
                    let mut prompt = app.i18n.text_args(
                        "settings-agent-profile-prompt",
                        &agena_tui::fl_args!("session" => session_id),
                    );
                    let selected = match current {
                        Ok(state) => state.execution.profile.unwrap_or_default(),
                        Err(error) => {
                            prompt.push_str("\n\n");
                            prompt.push_str(&app.i18n.text_args(
                                "settings-agent-profile-repair",
                                &agena_tui::fl_args!("error" => error.to_string()),
                            ));
                            String::new()
                        }
                    };
                    let choices = profile_choices(&app.i18n, profile_catalog(&sources, global));
                    app.overlay = Some(Overlay::Choice(app.build_choice_overlay(
                        app.i18n.text("settings-agent-profile-label"),
                        prompt,
                        Some(selected),
                        choices,
                        ChoiceOverlayAction::SessionProfile { session_id },
                        false,
                        agena_tui::choice::ChoicePresentationStyle::SelectOnly,
                    )));
                }
                Err(error) => app.flash_error(error),
            },
        );
    }

    pub(crate) fn set_session_profile(&mut self, session_id: i64, profile: Option<String>) {
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .client()
                    .set_session_profile(session_id, profile)
                    .await?;
                application.get_session_state(session_id).await
            },
            move |app, result| match result {
                Ok(state) => {
                    app.handle_session_execution_updated(session_id, state, true);
                    app.request_settings_async_context();
                    app.flash_success(app.i18n.text("settings-agent-profile-saved"));
                }
                Err(error) => app.flash_error(error),
            },
        );
    }
}
