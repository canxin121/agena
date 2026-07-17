use super::{
    normalize_permission_config, permission_mode_choice_items, permission_studio_read_only_message,
    permission_studio_selected_tool_tag_key,
};

pub(in crate::app) fn apply_permission_studio_entries_mode(
    permission: &mut PermissionConfig,
    kind: PermissionStudioCatalogKind,
    entries: impl IntoIterator<Item = String>,
    mode: PermissionMode,
) {
    let tools = permission.tools.get_or_insert_with(Default::default);
    match kind {
        PermissionStudioCatalogKind::ToolTags => {
            for entry in entries {
                tools.tags.insert(entry, mode);
            }
        }
        PermissionStudioCatalogKind::ToolNames => {
            for entry in entries {
                tools.names.insert(entry, mode);
            }
        }
    }
}

impl App {
    pub(in crate::app) fn open_permission_studio_add_current(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) {
        if !dialog.editable {
            self.flash_warning(permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        let action = match &dialog.page {
            PermissionStudioPage::PathDefaults
            | PermissionStudioPage::NetworkZones
            | PermissionStudioPage::Overview => {
                self.flash_warning(ui_text::t(&self.i18n, "flash-permission-studio-no-add"));
                return;
            }
            PermissionStudioPage::PathRules => PermissionStudioEditorAction::AddPathRule,
            PermissionStudioPage::NetworkRules => PermissionStudioEditorAction::AddNetworkRule,
            PermissionStudioPage::ToolTags => {
                self.open_permission_studio_catalog_selector(
                    dialog,
                    PermissionStudioCatalogKind::ToolTags,
                );
                return;
            }
            PermissionStudioPage::ToolNames => {
                self.open_permission_studio_catalog_selector(
                    dialog,
                    PermissionStudioCatalogKind::ToolNames,
                );
                return;
            }
            PermissionStudioPage::ToolCommandRules => PermissionStudioEditorAction::AddToolRule,
        };
        self.open_permission_studio_creator(dialog, action);
    }

    pub(in crate::app) fn open_permission_studio_catalog_selector(
        &mut self,
        dialog: &PermissionStudioOverlay,
        kind: PermissionStudioCatalogKind,
    ) {
        let catalog = self.backend.permission_tool_catalog();
        let existing = match kind {
            PermissionStudioCatalogKind::ToolTags => dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.tags.keys().cloned().collect::<BTreeSet<_>>())
                .unwrap_or_default(),
            PermissionStudioCatalogKind::ToolNames => dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.names.keys().cloned().collect::<BTreeSet<_>>())
                .unwrap_or_default(),
        };
        let mut items = match kind {
            PermissionStudioCatalogKind::ToolTags => {
                let mut tools_by_tag = BTreeMap::<String, Vec<String>>::new();
                for tool in &catalog {
                    for tag in &tool.tags {
                        tools_by_tag
                            .entry(tag.clone())
                            .or_default()
                            .push(tool.name.clone());
                    }
                }
                tools_by_tag
                    .into_iter()
                    .filter(|(tag, _)| !existing.contains(tag))
                    .map(|(tag, tools)| ChoiceItem {
                        label: tag.clone(),
                        detail: self.i18n.text_args(
                            "permission-studio-catalog-tag-detail",
                            &crate::fl_args!("count" => tools.len() as i64),
                        ),
                        value: tag.clone(),
                        search_text: format!("{tag} {}", tools.join(" ")),
                        current: false,
                    })
                    .collect::<Vec<_>>()
            }
            PermissionStudioCatalogKind::ToolNames => catalog
                .into_iter()
                .filter(|tool| !existing.contains(tool.name.as_str()))
                .map(|tool| ChoiceItem {
                    label: tool.name.clone(),
                    detail: tool.summary.clone(),
                    value: tool.name.clone(),
                    search_text: format!("{} {} {}", tool.name, tool.summary, tool.tags.join(" ")),
                    current: false,
                })
                .collect::<Vec<_>>(),
        };
        items.push(ChoiceItem {
            label: ui_text::t(&self.i18n, "permission-studio-catalog-custom-label"),
            detail: ui_text::t(&self.i18n, "permission-studio-catalog-custom-detail"),
            value: PERMISSION_STUDIO_CUSTOM_ENTRY.to_string(),
            search_text: ui_text::t(&self.i18n, "permission-studio-catalog-custom-search"),
            current: false,
        });
        let mut selector = self.build_choice_overlay(
            ui_text::t(
                &self.i18n,
                match kind {
                    PermissionStudioCatalogKind::ToolTags => "permission-studio-catalog-tags-title",
                    PermissionStudioCatalogKind::ToolNames => {
                        "permission-studio-catalog-names-title"
                    }
                },
            ),
            ui_text::t(&self.i18n, "permission-studio-catalog-prompt"),
            None,
            items,
            ChoiceOverlayAction::PermissionStudioAddEntries(kind),
            false,
            ChoiceOverlayStyle::SearchableSelect,
        );
        selector.config.selection_mode = SearchPickerSelectionMode::Multiple;
        selector.footer = ui_text::t(&self.i18n, "permission-studio-catalog-footer");
        self.open_choice_overlay(selector);
    }

    pub(in crate::app) fn open_permission_studio_add_entries_mode(
        &mut self,
        kind: PermissionStudioCatalogKind,
        entries: Vec<String>,
        add_custom_after: bool,
    ) {
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, "overlay-permission-rule-choice-mode-title"),
            ui_text::t(&self.i18n, "overlay-permission-rule-choice-mode-prompt"),
            Some("ask".to_owned()),
            permission_mode_choice_items(&self.i18n),
            ChoiceOverlayAction::PermissionStudioAddEntriesMode {
                kind,
                entries,
                add_custom_after,
            },
            false,
            ChoiceOverlayStyle::SelectOnly,
        ));
    }

    pub(in crate::app) fn open_permission_studio_delete_current(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) {
        if !dialog.editable {
            self.flash_warning(permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        let selected_action = dialog.state.selected_item().map(|item| item.action.clone());
        let (title, body, action) = match &dialog.page {
            PermissionStudioPage::PathDefaults
            | PermissionStudioPage::NetworkZones
            | PermissionStudioPage::Overview => {
                self.flash_warning(ui_text::t(&self.i18n, "flash-permission-studio-no-delete"));
                return;
            }
            PermissionStudioPage::PathRules => {
                let Some(pattern) = selected_action.and_then(|action| match action {
                    PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathRuleRead { pattern },
                    )
                    | PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathRuleWrite { pattern },
                    ) => Some(pattern),
                    _ => None,
                }) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-path-rules"),
                            "value" => pattern.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeletePathRule { pattern },
                )
            }
            PermissionStudioPage::NetworkRules => {
                let Some(target) = selected_action.and_then(|action| match action {
                    PermissionStudioAction::EditMode(PermissionStudioModeTarget::NetworkRule {
                        target,
                    }) => Some(target),
                    _ => None,
                }) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-network-rules"),
                            "value" => target.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeleteNetworkRule { target },
                )
            }
            PermissionStudioPage::ToolTags => {
                let Some(key) = permission_studio_selected_tool_tag_key(dialog) else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-permission-studio-no-delete"));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-tags"),
                            "value" => key.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeleteToolTag { key },
                )
            }
            PermissionStudioPage::ToolNames => {
                let Some(key) = selected_action.and_then(|action| match action {
                    PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolName {
                        key,
                    }) => Some(key),
                    _ => None,
                }) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-names"),
                            "value" => key.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeleteToolName { key },
                )
            }
            PermissionStudioPage::ToolCommandRules => {
                let Some((tool_name, pattern)) = selected_action.and_then(|action| match action {
                    PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::ToolCommandPattern { tool_name, pattern },
                    ) => Some((tool_name, Some(pattern))),
                    PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolRule {
                        tool_name,
                    }) => Some((tool_name, None)),
                    _ => None,
                }) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-tool-rules"),
                            "value" => pattern.clone().unwrap_or_else(|| tool_name.clone()),
                        ),
                    )],
                    match pattern {
                        Some(pattern) => ConfirmAction::PermissionStudioDeleteToolCommandPattern {
                            tool_name,
                            pattern,
                        },
                        None => ConfirmAction::PermissionStudioDeleteToolRule { tool_name },
                    },
                )
            }
        };
        self.overlay = Some(Overlay::Confirm(
            self.build_confirm_overlay(title, body, action),
        ));
    }

    pub(in crate::app) fn delete_permission_studio_config<F>(&mut self, mutator: F)
    where
        F: FnOnce(&mut PermissionConfig),
    {
        let Some((host, mut dialog)) = self.take_permission_studio_dialog() else {
            self.flash_error(ui_text::t(
                &self.i18n,
                "flash-permission-studio-context-lost",
            ));
            return;
        };

        let mut permission = dialog.permission.clone();
        mutator(&mut permission);
        normalize_permission_config(&mut permission);
        match self.persist_permission_studio(&mut dialog, permission) {
            Ok(()) => self.restore_permission_studio_dialog(host, dialog),
            Err(error) => {
                self.restore_permission_studio_dialog(host, dialog);
                self.flash_error(error);
            }
        }
    }

    pub(in crate::app) fn delete_permission_studio_path_rule(&mut self, pattern: &str) {
        let pattern = pattern.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(path) = permission.path.as_mut() {
                path.rules.shift_remove(pattern.as_str());
            }
        });
    }

    pub(in crate::app) fn delete_permission_studio_network_rule(&mut self, target: &str) {
        let target = target.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(network) = permission.network.as_mut() {
                network.rules.shift_remove(target.as_str());
            }
        });
    }

    pub(in crate::app) fn delete_permission_studio_tool_tag(&mut self, key: &str) {
        let key = key.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(tools) = permission.tools.as_mut() {
                tools.tags.remove(key.as_str());
            }
        });
    }

    pub(in crate::app) fn delete_permission_studio_tool_name(&mut self, key: &str) {
        let key = key.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(tools) = permission.tools.as_mut() {
                tools.names.remove(key.as_str());
            }
        });
    }

    pub(in crate::app) fn delete_permission_studio_tool_rule(&mut self, tool_name: &str) {
        let tool_name = tool_name.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(tools) = permission.tools.as_mut() {
                tools.rules.remove(tool_name.as_str());
            }
        });
    }

    pub(in crate::app) fn delete_permission_studio_tool_command_pattern(
        &mut self,
        tool_name: &str,
        pattern: &str,
    ) {
        let tool_name = tool_name.to_string();
        let pattern = pattern.to_string();
        self.delete_permission_studio_config(move |permission| {
            let Some(tools) = permission.tools.as_mut() else {
                return;
            };
            let Some(ToolPermissionRules::Ordered(entries)) =
                tools.rules.get_mut(tool_name.as_str())
            else {
                return;
            };
            entries.shift_remove(pattern.as_str());
        });
    }

    pub(in crate::app) fn refresh_current_transcript_execution_state(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        match self.block_on_async(self.backend.get_session_state(session_id)) {
            Ok(execution) => {
                let _ = self.apply_transcript_execution(execution);
            }
            Err(error) => self.flash_error(error.to_string()),
        }
    }

    pub(in crate::app) fn select_settings_studio_query(
        &self,
        dialog: &mut SettingsStudioOverlay,
        query: &str,
    ) {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return;
        }
        for (section_index, section) in dialog.state.sections().iter().enumerate() {
            for (item_index, item) in section.items.iter().enumerate() {
                if section.label.to_ascii_lowercase().contains(query.as_str())
                    || section
                        .summary
                        .to_ascii_lowercase()
                        .contains(query.as_str())
                    || section
                        .description
                        .to_ascii_lowercase()
                        .contains(query.as_str())
                    || item.label.to_ascii_lowercase().contains(query.as_str())
                    || item.value.to_ascii_lowercase().contains(query.as_str())
                    || item.detail.to_ascii_lowercase().contains(query.as_str())
                {
                    dialog.state.set_indices(section_index, item_index);
                    dialog.state.set_focus(SettingsStudioFocus::Items);
                    return;
                }
            }
        }
    }

    pub(in crate::app) fn activate_settings_studio_selection(
        &mut self,
        dialog: &mut SettingsStudioOverlay,
    ) -> bool {
        if dialog.state.focus() == SettingsStudioFocus::Navigation {
            dialog.state.set_focus(SettingsStudioFocus::Items);
            return false;
        }
        let Some(item) = dialog.state.selected_item().cloned() else {
            return false;
        };
        match item.action {
            SettingsPickerAction::EditField(field) => {
                self.open_settings_field_editor(field, "");
                false
            }
            SettingsPickerAction::OpenProviderDefaultModelChooser => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_model_chooser(SessionModelChooserPurpose::ProviderDefault);
                false
            }
            SettingsPickerAction::OpenAgentList => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_agent_list("");
                false
            }
            SettingsPickerAction::OpenProviderList => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_provider_list("");
                false
            }
            SettingsPickerAction::OpenModelCatalogWorkbench => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_model_catalog_studio();
                false
            }
            SettingsPickerAction::OpenGlobalPermissionWorkbench => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_global_permission_studio();
                false
            }
            SettingsPickerAction::OpenWorkspacePermissionWorkbench => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_workspace_permission_studio();
                false
            }
            SettingsPickerAction::OpenCurrentSessionPermissionWorkbench => {
                let Some(session_id) = self.current_or_selected_session_id() else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
                    return false;
                };
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_session_permission_studio(session_id);
                false
            }
            SettingsPickerAction::OpenSessionEffectivePermissionView(session_id) => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                match self.build_permission_studio_overlay(
                    PermissionStudioSource::EffectiveSession { session_id },
                    PermissionStudioPage::Overview,
                    Some(PermissionStudioSectionId::RootPath),
                    None,
                    PermissionStudioFocus::Navigation,
                ) {
                    Ok(permission) => self.current_route = Route::PermissionStudio(permission),
                    Err(error) => self.flash_error(error),
                }
                false
            }
            SettingsPickerAction::OpenPluginWorkbench => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                match self.build_plugin_workbench("") {
                    Ok(workbench) => {
                        self.current_route = Route::PluginWorkbench(Box::new(workbench));
                    }
                    Err(error) => self.flash_error(error),
                }
                false
            }
            SettingsPickerAction::RefreshProviderClientVersions => {
                match self.block_on_async(self.backend.refresh_provider_client_versions()) {
                    Ok(versions) => self.flash_success(self.i18n.text_args(
                        "flash-provider-client-versions-refreshed",
                        &crate::fl_args!(
                            "codex" => versions.codex,
                            "claude" => versions.claude,
                            "gemini" => versions.gemini,
                        ),
                    )),
                    Err(error) => self.flash_error(error),
                }
                self.refresh_settings_studio_overlay(dialog);
                false
            }
            SettingsPickerAction::OpenConfigFile => {
                self.open_runtime_config_in_editor();
                false
            }
            SettingsPickerAction::OpenTerminalDiagnostics => {
                self.open_terminal_diagnostics();
                false
            }
        }
    }

    pub(in crate::app) fn refresh_restored_route(&self, route: Route) -> Route {
        match route {
            Route::SettingsStudio(dialog) => self
                .rebuild_settings_studio_overlay(&dialog)
                .map(Route::SettingsStudio)
                .unwrap_or(Route::SettingsStudio(dialog)),
            Route::AgentStudio(dialog) => self
                .build_agent_studio_overlay(
                    dialog.agent_name.as_str(),
                    dialog
                        .workbench
                        .list
                        .selected_item()
                        .map(|item| item.label.as_str()),
                )
                .map(Route::AgentStudio)
                .unwrap_or(Route::AgentStudio(dialog)),
            Route::PermissionStudio(dialog) => self
                .build_permission_studio_overlay(
                    dialog.source.clone(),
                    dialog.page.clone(),
                    dialog.state.selected_section().map(|section| section.id),
                    dialog.state.selected_item().map(|item| item.label.as_str()),
                    dialog.state.focus(),
                )
                .map(|mut updated| {
                    updated.pane_focus = dialog.pane_focus;
                    Route::PermissionStudio(updated)
                })
                .unwrap_or(Route::PermissionStudio(dialog)),
            Route::Picker(dialog) if matches!(dialog.meta.kind, PickerKind::Agents) => {
                Route::Picker(self.build_agent_list_overlay(dialog.input.text(), false))
            }
            Route::Picker(dialog)
                if matches!(
                    dialog.meta.kind,
                    PickerKind::Providers(ProviderPickerPurpose::Configure)
                ) =>
            {
                Route::Picker(self.build_provider_list_overlay(dialog.input.text(), false))
            }
            Route::PluginWorkbench(dialog) => {
                Route::PluginWorkbench(Box::new(self.refresh_restored_plugin_workbench(*dialog)))
            }
            other => other,
        }
    }

    pub(in crate::app) fn refresh_restored_overlay(&self, overlay: Overlay) -> Overlay {
        overlay
    }

    pub(in crate::app) fn refresh_current_route_after_local_edit(&mut self) {
        self.refresh_tui_palette_from_runtime();
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        self.current_route = self.refresh_restored_route(route);
    }

    pub(in crate::app) fn take_picker_dialog(&mut self) -> Option<(DialogHost, PickerOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::Picker(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::Picker(dialog)) => Some((DialogHost::Overlay, dialog)),
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    pub(in crate::app) fn restore_picker_dialog(
        &mut self,
        host: DialogHost,
        dialog: PickerOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::Picker(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::Picker(dialog)),
        }
    }

    pub(in crate::app) fn take_session_search_dialog(
        &mut self,
    ) -> Option<(DialogHost, SessionSearchOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::SessionSearch(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::SessionSearch(dialog)) => Some((DialogHost::Overlay, dialog)),
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    pub(in crate::app) fn restore_session_search_dialog(
        &mut self,
        host: DialogHost,
        dialog: SessionSearchOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::SessionSearch(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::SessionSearch(dialog)),
        }
    }

    pub(in crate::app) fn take_provider_studio_dialog(
        &mut self,
    ) -> Option<(DialogHost, ProviderStudioOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::ProviderStudio(dialog) => Some((DialogHost::Route, *dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::ProviderStudio(dialog)) => Some((DialogHost::Overlay, *dialog)),
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    pub(in crate::app) fn take_permission_studio_dialog(
        &mut self,
    ) -> Option<(DialogHost, PermissionStudioOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::PermissionStudio(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                None
            }
        }
    }

    pub(in crate::app) fn restore_permission_studio_dialog(
        &mut self,
        host: DialogHost,
        dialog: PermissionStudioOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::PermissionStudio(dialog),
            DialogHost::Overlay => {}
        }
    }

    pub(in crate::app) fn restore_provider_studio_dialog(
        &mut self,
        host: DialogHost,
        dialog: ProviderStudioOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::ProviderStudio(Box::new(dialog)),
            DialogHost::Overlay => self.overlay = Some(Overlay::ProviderStudio(Box::new(dialog))),
        }
    }

    pub(in crate::app) fn take_model_catalog_dialog(
        &mut self,
    ) -> Option<(DialogHost, ModelCatalogStudioOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::ModelCatalogStudio(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::ModelCatalogStudio(dialog)) => {
                        Some((DialogHost::Overlay, dialog))
                    }
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    pub(in crate::app) fn restore_model_catalog_dialog(
        &mut self,
        host: DialogHost,
        dialog: ModelCatalogStudioOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::ModelCatalogStudio(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::ModelCatalogStudio(dialog)),
        }
    }

    pub(in crate::app) fn take_timeline_dialog(&mut self) -> Option<(DialogHost, TimelineOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::Timeline(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::Timeline(dialog)) => Some((DialogHost::Overlay, dialog)),
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    pub(in crate::app) fn restore_timeline_dialog(
        &mut self,
        host: DialogHost,
        dialog: TimelineOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::Timeline(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::Timeline(dialog)),
        }
    }

    pub(in crate::app) fn open_choice_overlay(&mut self, mut dialog: ChoiceOverlay) {
        Self::refresh_choice_overlay(&mut dialog);
        Self::select_current_choice_overlay_row(&mut dialog);
        self.overlay = Some(Overlay::Choice(dialog));
    }

    pub(in crate::app) fn refresh_choice_overlay(dialog: &mut ChoiceOverlay) {
        dialog.refresh_results();
    }

    pub(in crate::app) fn sync_choice_overlay_input(dialog: &mut ChoiceOverlay) {
        Self::refresh_choice_overlay(dialog);
        Self::select_choice_overlay_query_row(dialog);
    }

    pub(in crate::app) fn select_current_choice_overlay_row(dialog: &mut ChoiceOverlay) {
        if dialog.select_item_where(|item| item.current) {
            return;
        }
        if dialog.meta.current_value.is_none() && dialog.clear_action.is_some() {
            dialog.selected = 0;
        }
    }

    pub(in crate::app) fn select_choice_overlay_query_row(dialog: &mut ChoiceOverlay) {
        let trimmed = dialog.input.text().trim().to_string();
        if trimmed.is_empty() {
            Self::select_current_choice_overlay_row(dialog);
            return;
        }

        if dialog.select_item_where(|item| {
            item.value.eq_ignore_ascii_case(&trimmed) || item.label.eq_ignore_ascii_case(&trimmed)
        }) {
            return;
        }

        if dialog.select_item_where(|_| true) {
            return;
        }

        if dialog.config.input_mode.allows_custom_value()
            && ChoiceCustomValue::search_picker_from_input(dialog.input.text(), &dialog.meta)
                .is_some()
        {
            dialog.selected = usize::from(dialog.clear_action.is_some());
        } else {
            dialog.clamp_selection();
        }
    }
}
use crate::app::{
    App, BTreeMap, BTreeSet, ChoiceCustomValue, ChoiceItem, ChoiceOverlay, ChoiceOverlayAction,
    ChoiceOverlayStyle, ConfirmAction, DialogHost, ModelCatalogStudioOverlay, Overlay,
    PERMISSION_STUDIO_CUSTOM_ENTRY, PermissionConfig, PermissionMode, PermissionStudioAction,
    PermissionStudioCatalogKind, PermissionStudioEditorAction, PermissionStudioFocus,
    PermissionStudioModeTarget, PermissionStudioOverlay, PermissionStudioPage,
    PermissionStudioSectionId, PermissionStudioSource, PickerKind, PickerOverlay,
    ProviderPickerPurpose, ProviderStudioOverlay, Route, SessionModelChooserPurpose,
    SessionSearchOverlay, SettingsPickerAction, SettingsStudioFocus, SettingsStudioOverlay,
    TimelineOverlay, ToolPermissionRules, ui_text,
};
use agena_tui_components::{SearchPickerCustomValue, SearchPickerSelectionMode};

#[cfg(test)]
mod choice_overlay_tests {
    use super::App;
    use crate::{
        app::{
            ChoiceOverlay, ChoiceOverlayAction, ChoiceOverlayMeta, PermissionRuleStudioChoiceField,
            choice_item, mark_current_choice_item,
        },
        i18n::I18n,
    };
    use agena_tui_components::{
        Editor, SearchPickerClearAction, SearchPickerConfig, SearchPickerInputMode,
    };

    fn choice_dialog(query: &str, current_value: Option<&str>) -> ChoiceOverlay {
        let i18n = I18n::english();
        let mut items = vec![
            choice_item("build", "agent"),
            choice_item("review", "agent"),
        ];
        mark_current_choice_item(&i18n, &mut items, current_value);
        let mut config = SearchPickerConfig::searchable();
        config.input_mode = SearchPickerInputMode::SearchWithCustomValue;
        let mut dialog = ChoiceOverlay::new(
            "Agents".into(),
            String::new(),
            String::new(),
            "No agents".into(),
            Editor::from_text(query.to_string()),
            config,
            Some(SearchPickerClearAction {
                label: "Clear".into(),
                detail: String::new(),
                current: current_value.is_none(),
            }),
            ChoiceOverlayMeta {
                i18n,
                action: ChoiceOverlayAction::PermissionRuleStudio(
                    PermissionRuleStudioChoiceField::Mode,
                ),
                current_value: current_value.map(str::to_string),
            },
        );
        dialog.replace_items(items);
        dialog
    }

    #[test]
    fn opening_choice_overlay_keeps_query_empty_and_selects_current_item() {
        let mut dialog = choice_dialog("", Some("build"));

        App::refresh_choice_overlay(&mut dialog);
        App::select_current_choice_overlay_row(&mut dialog);

        assert_eq!(dialog.input.text(), "");
        assert_eq!(dialog.result_count(), 2);
        assert_eq!(
            dialog.selected_item().map(|item| item.value.as_str()),
            Some("build")
        );
    }

    #[test]
    fn filtering_prefers_the_first_matching_item_over_clear_and_custom_rows() {
        let mut dialog = choice_dialog("rev", Some("build"));

        App::sync_choice_overlay_input(&mut dialog);

        assert_eq!(dialog.result_count(), 1);
        assert_eq!(
            dialog.selected_item().map(|item| item.value.as_str()),
            Some("review")
        );
    }

    #[test]
    fn periodic_refresh_preserves_navigation_away_from_committed_item() {
        let mut dialog = choice_dialog("", Some("build"));
        App::refresh_choice_overlay(&mut dialog);
        App::select_current_choice_overlay_row(&mut dialog);

        dialog.move_selection(1);
        App::refresh_choice_overlay(&mut dialog);

        assert_eq!(
            dialog.selected_item().map(|item| item.value.as_str()),
            Some("review")
        );
    }
}
