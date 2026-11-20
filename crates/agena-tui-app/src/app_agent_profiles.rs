//! Select named session profiles through the server-owned profile catalog.

use crate::{App, ChoiceItem, ChoiceOverlayAction, I18n, Overlay, choice_item_with_value};

fn profile_catalog(
    sources: &agena_api::resource::ConfigJsonSources,
    global: bool,
) -> serde_json::Value {
    let mut profiles = sources
        .effective
        .get("builtin_profiles")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(configured) = sources
        .file
        .get("profiles")
        .and_then(serde_json::Value::as_object)
    {
        for (name, overlay) in configured {
            let next = profiles
                .get(name)
                .map(|base| merge_builtin_profile(base, overlay))
                .unwrap_or_else(|| overlay.clone());
            profiles.insert(name.clone(), next);
        }
    }
    if !global {
        if let Some(effective) = sources
            .effective
            .get("profiles")
            .and_then(serde_json::Value::as_object)
        {
            profiles = effective.clone();
        }
    }
    let origins: serde_json::Map<String, serde_json::Value> = profiles
        .keys()
        .map(|name| {
            let source = if !global
                && sources
                    .project_file
                    .get("profiles")
                    .and_then(|items| items.get(name))
                    .is_some()
            {
                "workspace"
            } else if sources
                .file
                .get("profiles")
                .and_then(|items| items.get(name))
                .is_some()
            {
                "global"
            } else {
                "builtin"
            };
            (name.clone(), serde_json::Value::String(source.into()))
        })
        .collect();
    serde_json::json!({"profiles":profiles,"profile_sources":origins})
}

fn merge_builtin_profile(
    base: &serde_json::Value,
    overlay: &serde_json::Value,
) -> serde_json::Value {
    use agena_prompt_layers::{LayerMode, PromptLayer};
    let mut result = base.as_object().cloned().unwrap_or_default();
    for key in ["title", "description"] {
        if let Some(value) = overlay
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        {
            result.insert(key.into(), serde_json::Value::String(value.into()));
        }
    }
    for key in ["mode", "hidden", "extends", "model", "max_turns"] {
        if let Some(value) = overlay.get(key).filter(|value| !value.is_null()) {
            result.insert(key.into(), value.clone());
        }
    }
    if let Some(value) = overlay.get("permission") {
        if let (Ok(previous), Ok(patch)) = (
            serde_json::from_value::<agena_domain::PermissionConfig>(
                base.get("permission")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            ),
            serde_json::from_value::<agena_domain::PermissionConfig>(value.clone()),
        ) {
            if let Ok(permission) = serde_json::to_value(previous.merged_with(&patch)) {
                result.insert("permission".into(), permission);
            }
        }
    }
    if let Some(value) = overlay.get("prompts") {
        if let (Ok(mut previous), Ok(patch)) = (
            serde_json::from_value::<PromptLayer>(
                base.get("prompts")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            ),
            serde_json::from_value::<PromptLayer>(value.clone()),
        ) {
            if patch.mode == LayerMode::Replace {
                previous = patch;
            } else {
                previous.order.retain(|id| !patch.disabled.contains(id));
                previous
                    .inline
                    .retain(|fragment| !patch.disabled.contains(&fragment.id));
                for id in patch.disabled {
                    if !previous.disabled.contains(&id) {
                        previous.disabled.push(id);
                    }
                }
                for id in patch.order {
                    previous.order.retain(|old| old != &id);
                    previous.order.push(id);
                }
                for fragment in patch.inline {
                    previous.inline.retain(|old| old.id != fragment.id);
                    previous.inline.push(fragment);
                }
            }
            if let Ok(prompts) = serde_json::to_value(previous) {
                result.insert("prompts".into(), prompts);
            }
        }
    }
    serde_json::Value::Object(result)
}

fn profile_mode<'a>(
    profiles: &'a serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Option<&'a str> {
    let mut current = name.to_owned();
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..64 {
        if !seen.insert(current.clone()) {
            return None;
        }
        let profile = profiles.get(&current)?;
        if let Some(mode) = profile.get("mode").filter(|value| !value.is_null()) {
            return mode
                .as_str()
                .filter(|mode| matches!(*mode, "primary" | "subagent" | "all"));
        }
        match profile.get("extends").and_then(serde_json::Value::as_str) {
            Some(parent) => current = parent.to_owned(),
            None => return Some("all"),
        }
    }
    None
}

fn profile_choices(i18n: &I18n, effective: &serde_json::Value, subagent: bool) -> Vec<ChoiceItem> {
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
            let Some(mode) = profile_mode(profiles, name) else {
                continue;
            };
            if mode != "all" && mode != if subagent { "subagent" } else { "primary" } {
                continue;
            }
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
            let mode_label = i18n.text(match mode {
                "primary" => "settings-agent-profile-mode-primary",
                "subagent" => "settings-agent-profile-mode-subagent",
                _ => "settings-agent-profile-mode-all",
            });
            let source = effective
                .get("profile_sources")
                .and_then(|origins| origins.get(name))
                .and_then(serde_json::Value::as_str);
            let source_label = source.map(|source| {
                i18n.text(match source {
                    "global" => "settings-agent-profile-source-global",
                    "workspace" => "settings-agent-profile-source-workspace",
                    _ => "settings-agent-profile-source-builtin",
                })
            });
            let detail = match source_label {
                Some(source) => format!("{source} · {mode_label} — {description}"),
                None => format!("{mode_label} — {description}"),
            };
            choices.push(choice_item_with_value(label, name.clone(), detail));
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
            false,
        );
        assert_eq!(choices[0].value, "");
        let review = choices
            .iter()
            .find(|item| item.value == "reviewer")
            .unwrap();
        assert_eq!(review.label, "Code review (reviewer)");
        assert!(review.detail.contains("Check regressions"));
        assert!(review.search_text.contains("reviewer"));
        let writer = choices.iter().find(|item| item.value == "writer").unwrap();
        assert_eq!(writer.label, "writer");
    }

    #[test]
    fn clearing_remains_available_when_the_catalog_is_empty() {
        let choices = profile_choices(&I18n::english(), &serde_json::json!({}), false);
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].value, "");
        assert_eq!(choices[0].label, "Restore default role");
    }

    #[test]
    fn global_sessions_exclude_workspace_profiles_and_overrides() {
        let sources = agena_api::resource::ConfigJsonSources {
            defaults: serde_json::Value::Null,
            global_resolved: serde_json::Value::Null,
            workspace_resolved: serde_json::Value::Null,
            global_inherited: serde_json::Value::Null,
            workspace_inherited: serde_json::Value::Null,
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
        let global = profile_choices(&I18n::english(), &profile_catalog(&sources, true), false);
        assert_eq!(global.len(), 2);
        assert_eq!(global[1].label, "Global review (reviewer)");
        let workspace = profile_choices(&I18n::english(), &profile_catalog(&sources, false), false);
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

    #[test]
    fn partial_builtin_overrides_keep_role_instructions_permissions_and_mode() {
        let base = serde_json::json!({
            "title":"Explore", "description":"Read code", "mode":"subagent",
            "permission":{"path":{"workspace":"readonly","external":"readonly"}},
            "prompts":{"order":["explore"],"inline":[{"id":"explore","body":"Inspect and report"}]}
        });
        let merged = merge_builtin_profile(&base, &serde_json::json!({"model":{"model":"fast"}}));
        assert_eq!(merged["mode"], "subagent");
        assert_eq!(merged["description"], "Read code");
        assert_eq!(merged["permission"], base["permission"]);
        assert_eq!(merged["prompts"], base["prompts"]);
        let changed = merge_builtin_profile(
            &base,
            &serde_json::json!({
                "prompts":{"disabled":["explore"],"order":["custom"],"inline":[{"id":"custom","body":"New instructions"}]}
            }),
        );
        assert_eq!(changed["prompts"]["order"], serde_json::json!(["custom"]));
    }

    #[test]
    fn modes_follow_inheritance_and_hidden_roles_remain_manually_selectable() {
        let catalog = serde_json::json!({"profiles":{
            "main":{"mode":"primary"},
            "explore":{"mode":"subagent"},
            "private-explore":{"extends":"explore","hidden":true},
            "reviewer":{"mode":"all"},
            "legacy":{}
        }});
        let roots = profile_choices(&I18n::english(), &catalog, false);
        assert!(roots.iter().any(|choice| choice.value == "main"));
        assert!(!roots.iter().any(|choice| choice.value == "explore"));
        let children = profile_choices(&I18n::english(), &catalog, true);
        assert!(!children.iter().any(|choice| choice.value == "main"));
        for name in ["explore", "private-explore", "reviewer", "legacy"] {
            assert!(children.iter().any(|choice| choice.value == name));
        }
    }

    #[test]
    fn global_catalog_keeps_builtins_and_global_overrides_without_workspace_roles() {
        let sources = agena_api::resource::ConfigJsonSources {
            defaults: serde_json::Value::Null,
            global_resolved: serde_json::Value::Null,
            workspace_resolved: serde_json::Value::Null,
            global_inherited: serde_json::Value::Null,
            workspace_inherited: serde_json::Value::Null,
            config_path: Default::default(),
            config_found: true,
            project_config_path: Default::default(),
            project_config_found: true,
            applied_layers: vec![],
            file: serde_json::json!({"profiles":{"main":{"title":"Custom main","mode":"primary"}}}),
            project_file: serde_json::json!({"profiles":{"project":{}}}),
            effective: serde_json::json!({
                "builtin_profiles":{"main":{"mode":"primary"},"explore":{"mode":"subagent"}},
                "profiles":{"main":{"title":"Custom main","mode":"primary"},"explore":{"mode":"subagent"},"project":{}}
            }),
        };
        let catalog = profile_catalog(&sources, true);
        assert!(catalog["profiles"].get("explore").is_some());
        assert!(catalog["profiles"].get("project").is_none());
        assert_eq!(catalog["profile_sources"]["main"], "global");
        assert_eq!(catalog["profile_sources"]["explore"], "builtin");
        let choices = profile_choices(&I18n::english(), &catalog, true);
        assert!(
            choices
                .iter()
                .any(|choice| choice.value == "explore" && choice.detail.contains("Built-in"))
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
                Ok::<_, anyhow::Error>((
                    sources,
                    current,
                    session.workspace_id.is_none(),
                    session.relation_kind.is_subagent(),
                ))
            },
            move |app, result| match result {
                Ok((sources, current, global, subagent)) => {
                    let mut prompt = app.i18n.text_args(
                        "settings-agent-profile-prompt",
                        &agena_tui::fl_args!("session" => session_id),
                    );
                    let selected = match current {
                        Ok(state) => {
                            if state.execution.profile_is_default {
                                if let Some(profile) = &state.execution.profile {
                                    prompt.push_str("\n\n");
                                    prompt.push_str(&app.i18n.text_args(
                                        "settings-agent-profile-effective",
                                        &agena_tui::fl_args!("profile" => profile.as_str()),
                                    ));
                                }
                                String::new()
                            } else {
                                state.execution.profile.unwrap_or_default()
                            }
                        }
                        Err(error) => {
                            prompt.push_str("\n\n");
                            prompt.push_str(&app.i18n.text_args(
                                "settings-agent-profile-repair",
                                &agena_tui::fl_args!("error" => error.to_string()),
                            ));
                            String::new()
                        }
                    };
                    let choices =
                        profile_choices(&app.i18n, &profile_catalog(&sources, global), subagent);
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
