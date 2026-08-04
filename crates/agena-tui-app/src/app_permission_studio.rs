use super::{
    normalize_permission_config, permission_mode_choice_items, permission_studio_read_only_message,
};

pub(crate) fn apply_permission_studio_entries_mode(
    permission: &mut PermissionConfig,
    kind: PermissionStudioCatalogKind,
    entries: impl IntoIterator<Item = String>,
    mode: PermissionMode,
) {
    let tools = permission.tools.get_or_insert_with(Default::default);
    match kind {
        PermissionStudioCatalogKind::ToolNames => {
            for entry in entries {
                tools.names.insert(entry, mode);
            }
        }
    }
}

impl App {
    pub(crate) fn open_permission_studio_add_current(
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
            PermissionStudioPage::PathDefaults | PermissionStudioPage::NetworkZones => {
                self.flash_warning(ui_text::t(&self.i18n, "flash-permission-studio-no-add"));
                return;
            }
            PermissionStudioPage::PathRules => PermissionStudioEditorAction::AddPathRule,
            PermissionStudioPage::NetworkRules => PermissionStudioEditorAction::AddNetworkRule,
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

    pub(crate) fn open_permission_studio_catalog_selector(
        &mut self,
        dialog: &PermissionStudioOverlay,
        kind: PermissionStudioCatalogKind,
    ) {
        let catalog = self.backend.permission_tool_catalog();
        let existing = match kind {
            PermissionStudioCatalogKind::ToolNames => dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.names.keys().cloned().collect::<BTreeSet<_>>())
                .unwrap_or_default(),
        };
        let mut items = match kind {
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
            ui_text::t(&self.i18n, "permission-studio-catalog-names-title"),
            ui_text::t(&self.i18n, "permission-studio-catalog-prompt"),
            None,
            items,
            ChoiceOverlayAction::PermissionStudioAddEntries(kind),
            false,
            agena_tui::choice::ChoicePresentationStyle::SearchableSelect,
        );
        selector.presentation.config.selection_mode = SearchPickerSelectionMode::Multiple;
        selector.presentation.footer = ui_text::t(&self.i18n, "permission-studio-catalog-footer");
        self.open_choice_overlay(selector);
    }

    pub(crate) fn open_permission_studio_add_entries_mode(
        &mut self,
        kind: PermissionStudioCatalogKind,
        entries: Vec<String>,
        add_custom_after: bool,
    ) {
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, "overlay-permission-rule-choice-mode-title"),
            ui_text::t(&self.i18n, "overlay-permission-rule-choice-mode-prompt"),
            Some("auto".to_owned()),
            permission_mode_choice_items(&self.i18n),
            ChoiceOverlayAction::PermissionStudioAddEntriesMode {
                kind,
                entries,
                add_custom_after,
            },
            false,
            agena_tui::choice::ChoicePresentationStyle::SelectOnly,
        ));
    }

    pub(crate) fn open_permission_studio_delete_current(
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
            PermissionStudioPage::PathDefaults | PermissionStudioPage::NetworkZones => {
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
                        &agena_tui::fl_args!(
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
                        &agena_tui::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-network-rules"),
                            "value" => target.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeleteNetworkRule { target },
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
                        &agena_tui::fl_args!(
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
                        &agena_tui::fl_args!(
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

    pub(crate) fn delete_permission_studio_config<F>(&mut self, mutator: F)
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

    pub(crate) fn delete_permission_studio_path_rule(&mut self, pattern: &str) {
        let pattern = pattern.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(path) = permission.path.as_mut() {
                path.rules.shift_remove(pattern.as_str());
            }
        });
    }

    pub(crate) fn delete_permission_studio_network_rule(&mut self, target: &str) {
        let target = target.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(network) = permission.network.as_mut() {
                network.rules.shift_remove(target.as_str());
            }
        });
    }

    pub(crate) fn delete_permission_studio_tool_name(&mut self, key: &str) {
        let key = key.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(tools) = permission.tools.as_mut() {
                tools.names.remove(key.as_str());
            }
        });
    }

    pub(crate) fn delete_permission_studio_tool_rule(&mut self, tool_name: &str) {
        let tool_name = tool_name.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(tools) = permission.tools.as_mut() {
                tools.rules.remove(tool_name.as_str());
            }
        });
    }

    pub(crate) fn delete_permission_studio_tool_command_pattern(
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

    pub(crate) fn refresh_current_transcript_execution_state(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        match self.block_on_async(self.backend.get_session_state(session_id)) {
            Ok(execution) => {
                let _ = self.apply_transcript_execution(execution);
            }
            Err(error) => self.flash_error(crate::UiFailure::internal(error)),
        }
    }

    pub(crate) fn activate_settings_studio_selection(
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
                self.open_provider_default_model_chooser();
                false
            }
            SettingsPickerAction::OpenPermissionApprovalModelChooser => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_permission_approval_model_chooser();
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
            SettingsPickerAction::OpenProviderClientVersions => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_client_versions_studio();
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
                    PermissionStudioPage::PathDefaults,
                    Some(PermissionStudioSectionId::PathDefaults),
                    None,
                    PermissionStudioFocus::Items,
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

    pub(crate) fn refresh_restored_route(&self, route: Route) -> Route {
        match route {
            Route::SettingsStudio(dialog) => self
                .rebuild_settings_studio_overlay(&dialog)
                .map(Route::SettingsStudio)
                .unwrap_or(Route::SettingsStudio(dialog)),
            Route::ClientVersionsStudio(dialog) => self
                .rebuild_client_versions_studio_overlay(&dialog)
                .map(Route::ClientVersionsStudio)
                .unwrap_or(Route::ClientVersionsStudio(dialog)),
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
            Route::SelectionPicker(dialog)
                if dialog.query
                    == SelectionPickerQuery::Providers(ProviderPickerPurpose::Configure) =>
            {
                Route::SelectionPicker(
                    self.build_provider_list_overlay(dialog.presentation.input.text(), false),
                )
            }
            Route::PluginWorkbench(dialog) => {
                Route::PluginWorkbench(Box::new(self.refresh_restored_plugin_workbench(*dialog)))
            }
            other => other,
        }
    }

    pub(crate) fn refresh_restored_overlay(&self, overlay: Overlay) -> Overlay {
        overlay
    }

    pub(crate) fn refresh_current_route_after_local_edit(&mut self) {
        self.refresh_tui_palette_from_runtime();
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        self.current_route = self.refresh_restored_route(route);
    }

    pub(crate) fn take_selection_picker_route(&mut self) -> Option<SelectionPickerOverlay> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::SelectionPicker(dialog) => Some(dialog),
            route => {
                self.current_route = route;
                None
            }
        }
    }

    pub(crate) fn restore_selection_picker_route(&mut self, dialog: SelectionPickerOverlay) {
        self.current_route = Route::SelectionPicker(dialog);
    }

    pub(crate) fn take_session_navigation_route(&mut self) -> Option<SessionNavigationOverlay> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::SessionNavigation(dialog) => Some(dialog),
            route => {
                self.current_route = route;
                None
            }
        }
    }

    pub(crate) fn restore_session_navigation_route(&mut self, dialog: SessionNavigationOverlay) {
        self.current_route = Route::SessionNavigation(dialog);
    }

    pub(crate) fn take_session_search_dialog(
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

    pub(crate) fn restore_session_search_dialog(
        &mut self,
        host: DialogHost,
        dialog: SessionSearchOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::SessionSearch(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::SessionSearch(dialog)),
        }
    }

    pub(crate) fn take_provider_studio_dialog(
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

    pub(crate) fn take_permission_studio_dialog(
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

    pub(crate) fn restore_permission_studio_dialog(
        &mut self,
        host: DialogHost,
        dialog: PermissionStudioOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::PermissionStudio(dialog),
            DialogHost::Overlay => {}
        }
    }

    pub(crate) fn restore_provider_studio_dialog(
        &mut self,
        host: DialogHost,
        dialog: ProviderStudioOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::ProviderStudio(Box::new(dialog)),
            DialogHost::Overlay => self.overlay = Some(Overlay::ProviderStudio(Box::new(dialog))),
        }
    }

    pub(crate) fn take_model_catalog_dialog(
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

    pub(crate) fn restore_model_catalog_dialog(
        &mut self,
        host: DialogHost,
        dialog: ModelCatalogStudioOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::ModelCatalogStudio(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::ModelCatalogStudio(dialog)),
        }
    }

    pub(crate) fn take_timeline_dialog(&mut self) -> Option<(DialogHost, TimelineOverlay)> {
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

    pub(crate) fn restore_timeline_dialog(&mut self, host: DialogHost, dialog: TimelineOverlay) {
        match host {
            DialogHost::Route => self.current_route = Route::Timeline(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::Timeline(dialog)),
        }
    }

    pub(crate) fn open_choice_overlay(&mut self, dialog: ChoiceOverlay) {
        self.overlay = Some(Overlay::Choice(dialog));
    }
}
use crate::{
    App, BTreeSet, ChoiceItem, ChoiceOverlay, ChoiceOverlayAction, ConfirmAction, DialogHost,
    ModelCatalogStudioOverlay, Overlay, PERMISSION_STUDIO_CUSTOM_ENTRY, PermissionConfig,
    PermissionMode, PermissionStudioAction, PermissionStudioCatalogKind,
    PermissionStudioEditorAction, PermissionStudioFocus, PermissionStudioModeTarget,
    PermissionStudioOverlay, PermissionStudioPage, PermissionStudioSectionId,
    PermissionStudioSource, ProviderPickerPurpose, ProviderStudioOverlay, Route,
    SelectionPickerOverlay, SelectionPickerQuery, SessionNavigationOverlay, SessionSearchOverlay,
    SettingsPickerAction, SettingsStudioFocus, SettingsStudioOverlay, TimelineOverlay,
    ToolPermissionRules, ui_text,
};
use agena_tui_components::SearchPickerSelectionMode;
