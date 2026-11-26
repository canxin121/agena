impl App {
    pub(crate) fn commit_choice_overlay(
        &mut self,
        dialog: &mut ChoiceOverlay,
        selection: agena_tui::choice::ChoiceSelection,
    ) -> bool {
        match dialog.action.clone() {
            ChoiceOverlayAction::NativeAgent(menu) => {
                self.commit_native_agent_choice(*menu, &choice_selection_value(&selection), dialog)
            }
            ChoiceOverlayAction::ExternalAgentOptions {
                session_id,
                agent,
                changes,
            } => {
                if let Some(change) = choice_selection_value(&selection)
                    .parse::<usize>()
                    .ok()
                    .and_then(|index| changes.into_iter().nth(index))
                {
                    self.request_external_agent_options(session_id, agent, Some(change));
                }
                true
            }
            ChoiceOverlayAction::ExternalAgentWork { session_id } => {
                if let Ok(cancel_id) = choice_selection_value(&selection).parse() {
                    self.request_external_agent_work(session_id, Some(cancel_id));
                }
                true
            }
            ChoiceOverlayAction::SessionProfile { session_id } => {
                let value = choice_selection_value(&selection);
                self.set_session_profile(session_id, (!value.is_empty()).then_some(value));
                true
            }
            ChoiceOverlayAction::BotAgentProfile => {
                self.choose_bot_agent_profile(choice_selection_value(&selection));
                true
            }
            ChoiceOverlayAction::Exit => {
                let close = self.commit_exit_menu(&selection);
                if !close {
                    self.refresh_exit_menu_dialog(dialog);
                }
                close
            }
            ChoiceOverlayAction::MessageQueue { session_id } => {
                let value = choice_selection_value(&selection);
                match value.as_str() {
                    "pause" | "resume" | "clear" => self.request_queue_control(session_id, &value),
                    _ => {
                        if let Ok(execution_id) = value.parse() {
                            self.show_queued_message_actions(session_id, execution_id);
                        }
                    }
                }
                true
            }
            ChoiceOverlayAction::RewindMode {
                session_id,
                at_message_id,
                message_document,
                target,
            } => {
                let mode = match choice_selection_value(&selection).as_str() {
                    "in_place" => agena_domain::RewindMode::InPlace,
                    _ => agena_domain::RewindMode::Fork,
                };
                self.open_rewind_confirm_overlay(
                    session_id,
                    at_message_id,
                    message_document,
                    target,
                    mode,
                );
                true
            }
            ChoiceOverlayAction::ClearMode { session_id } => {
                let mode = match choice_selection_value(&selection).as_str() {
                    "context" => agena_api::commands::SessionClearMode::ContextOnly,
                    _ => agena_api::commands::SessionClearMode::History,
                };
                self.open_clear_confirm_overlay(session_id, mode);
                true
            }
            ChoiceOverlayAction::QueuedMessage {
                session_id,
                execution_id,
            } => {
                match choice_selection_value(&selection).as_str() {
                    "edit" => self.request_edit_queued_message(session_id, execution_id),
                    "remove" => self.request_delete_queued_message(session_id, execution_id),
                    _ => self.request_message_queue(session_id, false),
                }
                true
            }
            ChoiceOverlayAction::StoppedMessageQueue { session_id } => {
                if choice_selection_value(&selection) == "resume" {
                    self.request_queue_control(session_id, "resume");
                }
                true
            }
            ChoiceOverlayAction::InsertContent {
                workspace_id,
                draft_slot,
                skills,
                ..
            } => {
                if self.application.workspace_id() != workspace_id
                    || self.current_draft_slot() != draft_slot
                {
                    self.flash_warning(
                        "The active workspace or draft changed. Reopen the insertion picker.",
                    );
                    return true;
                }
                let value = choice_selection_value(&selection);
                if value == "file" {
                    self.request_file_attachment(false);
                    return true;
                }
                if let Some(name) = value.strip_prefix("native-skill:") {
                    let Some(skill) =
                        self.external_agent_commands()
                            .and_then(|(_, agent, commands)| {
                                commands
                                    .iter()
                                    .find(|command| command.skill_inline && command.name == name)
                                    .map(|command| {
                                        agena_domain::SkillReferenceActivity::external(
                                            agent,
                                            command.name.clone(),
                                            command.description.clone(),
                                        )
                                    })
                            })
                    else {
                        self.flash_warning(
                            "This Skill is no longer advertised by the active ACP agent.",
                        );
                        return true;
                    };
                    if let Err(error) = self.stage_skill_activity(skill) {
                        self.flash_error(error);
                    }
                    return true;
                }
                let Some(id) = value.strip_prefix("skill:") else {
                    return false;
                };
                let Some(skill) = skills.iter().find(|skill| skill.id == id) else {
                    return false;
                };
                if let Err(error) = self.stage_skill_reference(skill) {
                    self.flash_error(error);
                }
                true
            }
            ChoiceOverlayAction::SettingsField(field) => {
                if !self.require_settings_field_editable(&field) {
                    return false;
                }
                let input = choice_selection_value(&selection);
                match parse_settings_field_input(&self.i18n, &field, input.as_str()) {
                    Ok(Some(value)) => {
                        let path = field.path.clone();
                        let workspace = field.workspace;
                        self.dispatch_backend_operation(
                            move |application| async move {
                                if workspace {
                                    application
                                        .set_workspace_config_setting(path.as_str(), value)
                                        .await
                                } else {
                                    application.set_config_setting(path.as_str(), value).await
                                }
                            },
                            move |app, result| match result {
                                Ok(_) => {
                                    app.flash_success(settings_path_updated_message(
                                        &app.i18n,
                                        field.path.as_str(),
                                    ));
                                    app.refresh_current_route_after_local_edit();
                                }
                                Err(error) => app.flash_error(error),
                            },
                        );
                        true
                    }
                    Ok(None) => {
                        let path = field.path.clone();
                        let workspace = field.workspace;
                        self.dispatch_backend_operation(
                            move |application| async move {
                                if workspace {
                                    application
                                        .delete_workspace_config_setting(path.as_str())
                                        .await
                                } else {
                                    application.delete_config_setting(path.as_str()).await
                                }
                            },
                            move |app, result| match result {
                                Ok(_) => {
                                    app.flash_success(settings_path_cleared_message(
                                        &app.i18n,
                                        field.path.as_str(),
                                    ));
                                    app.refresh_current_route_after_local_edit();
                                }
                                Err(error) => app.flash_error(error),
                            },
                        );
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::SettingsFieldSource {
                mut field,
                local_value,
                model_editor,
            } => {
                match choice_selection_value(&selection).as_str() {
                    "inherit" => self.persist_settings_field_value(field, None),
                    "local" if field.workspace => {
                        if let Some(value) = local_value {
                            self.persist_settings_field_value(field, Some(value));
                        } else if let Some(purpose) = model_editor {
                            self.open_settings_source_model_editor(purpose);
                        } else {
                            field.explicit_local = true;
                            self.open_settings_field_editor(field, "");
                        }
                    }
                    _ => return false,
                }
                true
            }
            ChoiceOverlayAction::SessionModelSource {
                session_id,
                model,
                selected,
                local_value,
            } => self.commit_session_model_source(
                session_id,
                model,
                selected,
                local_value,
                choice_selection_value(&selection).as_str(),
            ),
            ChoiceOverlayAction::SessionModelModeSource {
                step,
                session_id,
                model,
                selected,
            } => self.commit_session_model_mode_source(
                step,
                session_id,
                model,
                selected,
                choice_selection_value(&selection).as_str(),
            ),
            ChoiceOverlayAction::SessionModelMode {
                step,
                session_id,
                model,
                explicit_local,
            } => {
                if session_id.is_some()
                    && !explicit_local
                    && !super::session_sources::session_model_mode_is_local(
                        &self.current_raw_model_selection(),
                        step,
                    )
                {
                    self.flash_warning(crate::settings_inherited_locked_message(&self.i18n, true));
                    return false;
                }
                if !super::session_sources::model_source_context_matches(
                    session_id,
                    self.transcript.session_id,
                    &model,
                    &self.current_session_model_ref(),
                ) {
                    self.flash_warning(self.i18n.text("settings-model-source-changed"));
                    return true;
                }
                if session_id.is_some_and(|id| self.session_is_busy(id)) {
                    self.flash_warning(self.i18n.text("flash-session-busy"));
                    return false;
                }
                let input = choice_selection_value(&selection);
                let previous = self.run_options.clone();
                self.run_options.raw_selection = Some(self.current_raw_model_selection());
                self.run_options
                    .apply_model_mode_input(step, input.as_str());
                if !self.persist_current_session_model_stack(previous.clone()) {
                    self.run_options = previous;
                    return false;
                }
                self.advance_session_model_mode_step(step);
                true
            }
            ChoiceOverlayAction::ModelSelectionMode {
                purpose,
                explicit_local,
                model,
                step,
                mut thinking_mode,
                mut speed_mode,
                mut verbosity,
            } => {
                let value = choice_selection_value(&selection);
                let value = (!value.trim().is_empty()).then(|| value.trim().to_owned());
                match step {
                    SessionModelModeStep::ThinkingMode => thinking_mode = value,
                    SessionModelModeStep::SpeedMode => speed_mode = value,
                    SessionModelModeStep::Verbosity => {
                        verbosity = value.map(|value| value.to_ascii_lowercase())
                    }
                }
                match step {
                    SessionModelModeStep::ThinkingMode => self
                        .open_model_selection_mode_step_or_finish(
                            purpose,
                            explicit_local,
                            model,
                            thinking_mode,
                            speed_mode,
                            verbosity,
                            SessionModelModeStep::SpeedMode,
                        ),
                    SessionModelModeStep::SpeedMode => self
                        .open_model_selection_mode_step_or_finish(
                            purpose,
                            explicit_local,
                            model,
                            thinking_mode,
                            speed_mode,
                            verbosity,
                            SessionModelModeStep::Verbosity,
                        ),
                    SessionModelModeStep::Verbosity => {
                        self.finish_model_selection(
                            purpose,
                            explicit_local,
                            model,
                            thinking_mode,
                            speed_mode,
                            verbosity,
                        );
                    }
                }
                true
            }
            ChoiceOverlayAction::ProviderStudioField(field) => {
                let value = choice_selection_value(&selection);
                let Some((host, mut parent)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return true;
                };
                match self.commit_provider_studio_field(&mut parent, field, value) {
                    Ok(()) => {
                        self.restore_provider_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_provider_studio_dialog(host, parent);
                        self.flash_error(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::ProviderStudioModelField(field) => {
                let value = choice_selection_value(&selection);
                let Some((host, mut parent)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return true;
                };
                match self.commit_provider_studio_model_field(&mut parent, field, value) {
                    Ok(()) => {
                        self.restore_provider_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_provider_studio_dialog(host, parent);
                        self.flash_error(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::RememberedPermissionRules {
                source,
                workspace_id,
                rules,
            } => {
                self.open_remembered_permission_rule_selection(
                    source,
                    workspace_id,
                    rules,
                    &choice_selection_value(&selection),
                );
                true
            }
            ChoiceOverlayAction::PermissionRuleStudio(field) => {
                let value = choice_selection_value(&selection);
                let current_session_id = self.current_or_selected_session_id();
                match &mut self.current_route {
                    Route::PermissionRuleStudio(parent) => {
                        if !parent.editable
                            || parent.save_request.is_some()
                            || (parent.scope_locked
                                && field == PermissionRuleStudioChoiceField::Scope)
                        {
                            return true;
                        }
                        match field {
                            PermissionRuleStudioChoiceField::SubjectKind => {
                                parent.draft.subject_kind = match value.as_str() {
                                    "path_access" => PermissionRuleSubjectKind::PathAccess,
                                    "network_access" => PermissionRuleSubjectKind::NetworkAccess,
                                    _ => PermissionRuleSubjectKind::Tool,
                                };
                                if parent.draft.subject_kind
                                    == PermissionRuleSubjectKind::NetworkAccess
                                    && !matches!(
                                        parent.draft.mode,
                                        PermissionMode::Allow | PermissionMode::Deny
                                    )
                                {
                                    parent.draft.mode = PermissionMode::Deny;
                                }
                            }
                            PermissionRuleStudioChoiceField::PathAccessKind => {
                                if !value.trim().is_empty() {
                                    parent.draft.path_access_kind = value;
                                }
                            }
                            PermissionRuleStudioChoiceField::Scope => {
                                parent.draft.scope = if value.trim().is_empty() {
                                    "workspace".to_string()
                                } else {
                                    value
                                };
                                if parent.draft.scope != "session" {
                                    parent.draft.session_id.clear();
                                } else if parent.draft.session_id.trim().is_empty()
                                    && let Some(session_id) = current_session_id
                                {
                                    parent.draft.session_id = session_id.to_string();
                                }
                            }
                            PermissionRuleStudioChoiceField::Mode => {
                                parent.draft.mode = match value.as_str() {
                                    "allow" => PermissionMode::Allow,
                                    "auto" => PermissionMode::Auto,
                                    "ask" => PermissionMode::Ask,
                                    "deny" => PermissionMode::Deny,
                                    _ => PermissionMode::Auto,
                                };
                            }
                        }
                        refresh_permission_rule_studio_dialog(&self.i18n, parent);
                        true
                    }
                    _ => {
                        self.flash_error(ui_text::t(
                            &self.i18n,
                            "flash-permission-rule-context-lost",
                        ));
                        true
                    }
                }
            }
            ChoiceOverlayAction::PermissionStudioMode(target) => {
                let value = choice_selection_value(&selection);
                let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
                    self.flash_error(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-context-lost",
                    ));
                    return true;
                };
                if !crate::permission_studio_target_editable(&parent, &target) {
                    self.flash_warning(crate::permission_studio_inherited_message(
                        &self.i18n, &parent,
                    ));
                    self.restore_permission_studio_dialog(host, parent);
                    return false;
                }
                let projection = crate::PermissionStudioProjection::new(&parent);
                let new_rule = match &target {
                    crate::PermissionStudioModeTarget::PathRule { pattern }
                        if !projection
                            .effective
                            .path
                            .as_ref()
                            .is_some_and(|path| path.rules.contains_key(pattern)) =>
                    {
                        Some((PermissionStudioEditorAction::AddPathRule, pattern.clone()))
                    }
                    _ => None,
                };
                let creating_rule = new_rule.is_some();
                if let Some((action, input)) = new_rule {
                    self.open_permission_studio_creator(&mut parent, action);
                    if let Some(editor) = parent.editor.as_mut() {
                        editor.input = Editor::from_text(input);
                    }
                }
                let reset = value.trim().is_empty()
                    || value.eq_ignore_ascii_case("clear")
                    || value.eq_ignore_ascii_case("unset");
                let mut permission =
                    crate::permission_studio_mode_edit_config(&parent, &target, reset);
                let result = apply_permission_studio_mode_input(
                    &self.i18n,
                    &mut permission,
                    &target,
                    value.as_str(),
                )
                .and_then(|_| self.persist_permission_studio(&mut parent, permission));
                match result {
                    Ok(()) => {
                        if creating_rule {
                            // The async save keeps this draft for an error;
                            // show the list while the request is in flight.
                            parent.editor = None;
                        }
                        self.restore_permission_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_permission_studio_dialog(host, parent);
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::PermissionStudioSource(target) => self
                .commit_permission_studio_source(
                    target,
                    choice_selection_value(&selection).as_str(),
                ),
            ChoiceOverlayAction::PermissionStudioNetworkSource => self
                .commit_permission_studio_network_source(
                    choice_selection_value(&selection).as_str(),
                ),
            ChoiceOverlayAction::PermissionStudioNetworkListSource { deny } => self
                .commit_permission_studio_network_list_source(
                    deny,
                    choice_selection_value(&selection).as_str(),
                ),
            ChoiceOverlayAction::PermissionStudioNetworkProtocol => {
                let value = choice_selection_value(&selection);
                let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
                    self.flash_error(self.i18n.text("flash-permission-studio-context-lost"));
                    return true;
                };
                if matches!(value.as_str(), "tcp" | "udp" | "icmp" | "any")
                    && let Some(draft) = parent.network_rule.as_mut()
                {
                    draft.protocol = value;
                    crate::refresh_permission_studio_dialog(
                        &self.i18n,
                        &mut parent,
                        None,
                        None,
                        Some(crate::PermissionStudioFocus::Items),
                    );
                }
                self.restore_permission_studio_dialog(host, parent);
                true
            }
            ChoiceOverlayAction::PermissionStudioAddEntries(kind) => {
                let mut entries = dialog.presentation.checked_keys.clone();
                if entries.is_empty()
                    && let agena_tui::choice::ChoiceSelection::Item { value } = &selection
                {
                    entries.push(value.clone());
                }
                let wants_custom = entries
                    .iter()
                    .any(|entry| entry == PERMISSION_STUDIO_CUSTOM_ENTRY);
                entries.retain(|entry| entry != PERMISSION_STUDIO_CUSTOM_ENTRY);
                entries.sort();
                entries.dedup();
                if entries.is_empty() && !wants_custom {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-catalog-empty",
                    ));
                    return false;
                }

                let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
                    self.flash_error(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-context-lost",
                    ));
                    return true;
                };
                if !entries.is_empty() {
                    self.restore_permission_studio_dialog(host, parent);
                    self.open_permission_studio_add_entries_mode(kind, entries, wants_custom);
                    return true;
                }
                if wants_custom {
                    self.open_permission_studio_creator(
                        &mut parent,
                        PermissionStudioEditorAction::AddToolName,
                    );
                }
                self.restore_permission_studio_dialog(host, parent);
                true
            }
            ChoiceOverlayAction::PermissionStudioAddEntriesMode {
                kind,
                entries,
                add_custom_after,
            } => {
                let value = choice_selection_value(&selection);
                let mode = match value.as_str() {
                    "allow" => PermissionMode::Allow,
                    "auto" => PermissionMode::Auto,
                    "deny" => PermissionMode::Deny,
                    _ => PermissionMode::Auto,
                };
                let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
                    self.flash_error(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-context-lost",
                    ));
                    return true;
                };
                let mut permission = crate::permission_studio_local_config(&parent);
                apply_permission_studio_entries_mode(&mut permission, kind, entries, mode);
                if let Err(error) = self.persist_permission_studio(&mut parent, permission) {
                    self.restore_permission_studio_dialog(host, parent);
                    self.flash_warning(error);
                    return false;
                }
                if add_custom_after {
                    self.open_permission_studio_creator(
                        &mut parent,
                        PermissionStudioEditorAction::AddToolName,
                    );
                }
                self.restore_permission_studio_dialog(host, parent);
                true
            }
        }
    }

    pub(crate) fn open_settings_field_editor(
        &mut self,
        field: SettingsFieldSpec,
        _return_query: &str,
    ) {
        let sources = match crate::app_backend::config::config_json_sources(&self.application) {
            Ok(sources) => sources,
            Err(error) => {
                self.flash_error(crate::UiFailure::internal(error));
                return;
            }
        };
        self.open_settings_field_editor_with_sources(field, &sources);
    }

    pub(crate) fn open_settings_field_editor_with_sources(
        &mut self,
        field: SettingsFieldSpec,
        sources: &crate::ConfigJsonSources,
    ) {
        let projection = crate::SettingsFieldProjection::new(sources, &field);
        if !crate::settings_field_value_editable(sources, &field) {
            self.flash_warning(crate::settings_inherited_locked_message(&self.i18n, false));
            return;
        }
        if projection.requires_group_reset {
            self.open_settings_layer_source(field, None, sources);
            return;
        }
        let mut prompt = settings_value_edit_prompt(
            &self.i18n,
            &field,
            projection.local.as_ref(),
            projection.effective.as_ref(),
        );
        if field.workspace && field.path != crate::MCP_SERVER_URL_SETTINGS_PATH {
            prompt.push_str(&format!(
                "\n{}: {} · {}: {}",
                self.i18n.text("settings-layer-source-title"),
                projection.source_label(&self.i18n),
                projection.fallback_label(&self.i18n),
                crate::settings_layer_value_label(&self.i18n, projection.fallback_value())
            ));
        }
        let prefill = if field.path == crate::MCP_SERVER_URL_SETTINGS_PATH {
            self.application
                .cached_mcp_server_control()
                .and_then(|control| {
                    control
                        .get("serverUrl")
                        .and_then(JsonValue::as_str)
                        .map(|value| JsonValue::String(value.to_owned()))
                })
                .unwrap_or(JsonValue::Null)
        } else {
            projection.value.clone().unwrap_or(JsonValue::Null)
        };
        if let Some(all_items) = self.settings_field_choice_items(&field) {
            let current_value = if field.workspace {
                projection.local.as_ref()
            } else {
                projection.value.as_ref()
            }
            .map(setting_value_input_text);
            let overlay_style = Self::settings_field_choice_overlay_style(&field);
            let mut picker = self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    settings_field_edit_title(&self.i18n, &field).as_str(),
                ),
                prompt,
                current_value,
                all_items,
                ChoiceOverlayAction::SettingsField(field),
                projection.workspace,
                overlay_style,
            );
            if let Some(clear) = picker.presentation.clear_action.as_mut() {
                clear.label = format!(
                    "{} · {}",
                    crate::settings_layer_reset_label(&self.i18n, projection.workspace),
                    crate::settings_layer_value_label(&self.i18n, projection.fallback_value())
                );
            }
            self.open_choice_overlay(picker);
            return;
        }
        self.overlay = Some(Overlay::SettingsValueEdit(SettingsValueEditOverlay::new(
            settings_edit_title(
                &self.i18n,
                settings_field_edit_title(&self.i18n, &field).as_str(),
            ),
            prompt,
            Editor::from_text(setting_value_input_text(&prefill)),
            field,
        )));
    }
}

fn choice_selection_value(selection: &agena_tui::choice::ChoiceSelection) -> String {
    match selection {
        agena_tui::choice::ChoiceSelection::Clear => String::new(),
        agena_tui::choice::ChoiceSelection::Custom { raw }
        | agena_tui::choice::ChoiceSelection::Item { value: raw } => raw.clone(),
    }
}
use crate::{
    App, ChoiceOverlay, ChoiceOverlayAction, Editor, JsonValue, Overlay,
    PERMISSION_STUDIO_CUSTOM_ENTRY, PermissionMode, PermissionRuleStudioChoiceField,
    PermissionRuleSubjectKind, PermissionStudioEditorAction, Route, SessionModelModeStep,
    SettingsFieldSpec, SettingsValueEditOverlay, apply_permission_studio_entries_mode,
    apply_permission_studio_mode_input, parse_settings_field_input,
    refresh_permission_rule_studio_dialog, setting_value_input_text, settings_edit_title,
    settings_field_edit_title, settings_path_cleared_message, settings_path_updated_message,
    settings_value_edit_prompt, ui_text,
};
