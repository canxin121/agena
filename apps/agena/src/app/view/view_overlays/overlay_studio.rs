use super::super::{
    AgentStudioOverlay, App, BoundedListPanelHeight, DetailTextLine, DetailTextSpec, Frame, Line,
    ListWorkbenchDialogSpec, ListWorkbenchPanelState, Modifier, PermissionRuleStudioOverlay,
    PermissionStudioFocus, PermissionStudioOverlay, PermissionStudioPaneFocus,
    PermissionStudioSection, PermissionStudioSectionId, Rect, SectionedWorkbenchDialogSpec,
    SettingsStudioFocus, SettingsStudioOverlay, Span, Style, SurfaceMode, Text, UnicodeWidthStr,
    WorkbenchOverlayDialogSpec, WorkbenchTextSection, agent_profile_scope_label_localized,
    agent_profile_storage_label_localized, agent_studio_item_detail_text,
    agent_studio_overview_text, build_accented_two_line_list_item, build_detail_text,
    panel_highlight_style, permission_rule_draft_label, permission_rule_mode_label,
    permission_rule_studio_detail_text, permission_rule_subject_kind_name,
    permission_studio_table_label, render_list_workbench_dialog, render_sectioned_workbench_dialog,
    sanitize_display_text, selection_highlight_style, settings_item_detail_text,
    settings_item_detail_title, settings_table_columns, ui_text, workbench_navigation_width,
};

impl App {
    pub(in crate::app) fn render_settings_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SettingsStudioOverlay,
        surface: SurfaceMode,
    ) {
        let current_section = dialog.state.selected_section();
        let nav_items = dialog
            .state
            .sections()
            .iter()
            .map(|section| {
                build_accented_two_line_list_item(
                    sanitize_display_text(section.label.as_str()).into(),
                    Some(section.items.len().to_string().into()),
                    Some(
                        sanitize_display_text(
                            agena_tui::settings_studio::section_group_label(&self.i18n, section.id)
                                .as_str(),
                        )
                        .into(),
                    ),
                )
            })
            .collect::<Vec<_>>();
        let item_rows = current_section
            .map(|section| {
                section
                    .items
                    .iter()
                    .map(|item| {
                        build_accented_two_line_list_item(
                            sanitize_display_text(item.label.as_str()).into(),
                            Some(sanitize_display_text(item.value.as_str()).into()),
                            Some(sanitize_display_text(item.detail.as_str()).into()),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let section_title = current_section
            .map(|section| sanitize_display_text(section.label.as_str()))
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-settings-default-section-title"));
        let section_text = current_section
            .map(|section| Text::from(sanitize_display_text(section.description.as_str())))
            .unwrap_or_else(|| {
                Text::from(ui_text::t(&self.i18n, "overlay-settings-empty-section"))
            });
        let inspector_title = settings_item_detail_title(&self.i18n, dialog);
        let inspector_text = settings_item_detail_text(&self.i18n, dialog);
        let content_width = surface.content_width(area, 150);
        let nav_width = workbench_navigation_width(content_width);
        let spec = SectionedWorkbenchDialogSpec::new(
            sanitize_display_text(dialog.title.as_str()).into(),
            sanitize_display_text(dialog.footer.as_str()).into(),
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 2,
                    min_body_height: 8,
                    max_body_height: 30,
                },
                Some(ui_text::t(&self.i18n, "overlay-settings-sections").into()),
                nav_items.as_slice(),
                (!nav_items.is_empty()).then_some(dialog.state.selected_section_index()),
                panel_highlight_style(dialog.state.focus() == SettingsStudioFocus::Navigation),
                ">> ".into(),
            ),
            WorkbenchTextSection::new(
                ui_text::t(&self.i18n, "overlay-workbench-overview").into(),
                section_text,
                2,
                5,
            ),
            if item_rows.is_empty() {
                ListWorkbenchPanelState::empty(
                    Some(section_title.clone().into()),
                    ui_text::t(&self.i18n, "overlay-settings-empty-items").into(),
                    BoundedListPanelHeight {
                        lines_per_item: 2,
                        min_body_height: 6,
                        max_body_height: 18,
                    },
                )
            } else {
                ListWorkbenchPanelState::items(
                    BoundedListPanelHeight {
                        lines_per_item: 2,
                        min_body_height: 6,
                        max_body_height: 18,
                    },
                    Some(section_title.clone().into()),
                    item_rows.as_slice(),
                    Some(dialog.state.selected_item_index()),
                    panel_highlight_style(dialog.state.focus() == SettingsStudioFocus::Items),
                    ">> ".into(),
                )
            },
            WorkbenchTextSection::new(inspector_title.into(), inspector_text, 5, 14),
        )
        .summary(Some(section_title.clone().into()))
        .target_width(150)
        .navigation_width(nav_width);
        render_sectioned_workbench_dialog(frame, area, surface, &spec);
    }

    pub(in crate::app) fn render_agent_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &AgentStudioOverlay,
        surface: SurfaceMode,
    ) {
        let title_summary = format!(
            "{} · {} · {}",
            agent_profile_scope_label_localized(&self.i18n, &dialog.profile),
            agent_profile_storage_label_localized(&self.i18n, dialog.storage),
            if dialog.editable {
                ui_text::t(&self.i18n, "value-editable")
            } else {
                ui_text::t(&self.i18n, "value-read-only")
            }
        );
        let overview_body = agent_studio_overview_text(
            &self.i18n,
            &dialog.profile,
            dialog.default_agent_name.as_deref(),
            dialog.storage,
        );
        let selected_item = dialog.presentation.list.selected_item();
        let detail_body = selected_item
            .map(|item| {
                agent_studio_item_detail_text(&self.i18n, &dialog.profile, item, dialog.storage)
            })
            .unwrap_or_else(|| {
                Text::from(ui_text::t(&self.i18n, "overlay-agent-studio-empty-detail"))
            });

        let list_items = dialog
            .presentation
            .list
            .items
            .iter()
            .map(|item| {
                build_accented_two_line_list_item(
                    sanitize_display_text(item.label.as_str()).into(),
                    Some(sanitize_display_text(item.value.as_str()).into()),
                    Some(sanitize_display_text(item.detail.as_str()).into()),
                )
            })
            .collect::<Vec<_>>();
        let spec = ListWorkbenchDialogSpec::new(
            sanitize_display_text(dialog.presentation.title.as_str()).into(),
            sanitize_display_text(dialog.presentation.footer.as_str()).into(),
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 2,
                    min_body_height: 6,
                    max_body_height: 16,
                },
                Some(ui_text::t(&self.i18n, "overlay-agent-studio-fields").into()),
                list_items.as_slice(),
                (!dialog.presentation.list.items.is_empty())
                    .then_some(dialog.presentation.list.selected),
                selection_highlight_style(),
                ">> ".into(),
            ),
            vec![
                WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-workbench-overview").into(),
                    overview_body,
                    2,
                    5,
                ),
                WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-workbench-details").into(),
                    detail_body,
                    3,
                    10,
                ),
            ],
        )
        .summary(Some(sanitize_display_text(title_summary.as_str()).into()))
        .target_width(138)
        .left_panel_width(36)
        .overlay(
            dialog
                .editor
                .as_ref()
                .map(WorkbenchOverlayDialogSpec::from_source),
        );
        render_list_workbench_dialog(frame, area, surface, &spec);
    }

    pub(in crate::app) fn render_permission_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionStudioOverlay,
        surface: SurfaceMode,
    ) {
        let nav_items = dialog
            .nav
            .items
            .iter()
            .map(|item| {
                build_accented_two_line_list_item(
                    sanitize_display_text(format!(
                        "{}{}",
                        "  ".repeat(item.level),
                        item.label.as_str()
                    ))
                    .into(),
                    None,
                    None,
                )
            })
            .collect::<Vec<_>>();
        let content_width = surface.content_width(area, 140);
        let nav_width = workbench_navigation_width(content_width);
        let selected_section = dialog.state.selected_section();
        let table_title = selected_section
            .map(|section| sanitize_display_text(section.label.as_str()))
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-workbench-overview"));
        let table_text = selected_section
            .map(|section| {
                self.permission_studio_table_text(
                    dialog,
                    section,
                    content_width.saturating_sub(nav_width).saturating_sub(2),
                )
            })
            .unwrap_or_else(|| {
                Text::from(ui_text::t(&self.i18n, "overlay-settings-empty-section"))
            });
        let summary = format!(
            "{} · {} · {} · {}",
            dialog.title_context,
            dialog.source_label,
            dialog.scope_label,
            if dialog.editable {
                ui_text::t(&self.i18n, "value-editable")
            } else {
                ui_text::t(&self.i18n, "value-read-only")
            }
        );
        let spec = ListWorkbenchDialogSpec::new(
            sanitize_display_text(dialog.title.as_str()).into(),
            sanitize_display_text(dialog.footer.as_str()).into(),
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 1,
                    min_body_height: 8,
                    max_body_height: 30,
                },
                Some(ui_text::t(&self.i18n, "overlay-permission-studio-nav").into()),
                nav_items.as_slice(),
                (!nav_items.is_empty()).then_some(dialog.nav.selected),
                panel_highlight_style(dialog.pane_focus == PermissionStudioPaneFocus::Navigation),
                ">> ".into(),
            ),
            vec![WorkbenchTextSection::new(table_title.into(), table_text, 8, 30).wrap(false)],
        )
        .summary(Some(sanitize_display_text(summary).into()))
        .target_width(140)
        .left_panel_width(nav_width)
        .overlay(
            dialog
                .editor
                .as_ref()
                .map(WorkbenchOverlayDialogSpec::from_source),
        );
        render_list_workbench_dialog(frame, area, surface, &spec);
    }

    pub(in crate::app) fn permission_studio_table_text(
        &self,
        dialog: &PermissionStudioOverlay,
        section: &PermissionStudioSection,
        width: u16,
    ) -> Text<'static> {
        let (left_header, right_header) = Self::permission_studio_table_headers(section.id);
        let width = width.max(24);
        let left_width = width
            .saturating_mul(45)
            .saturating_div(100)
            .clamp(12, width - 12);
        let right_width = width.saturating_sub(left_width).saturating_sub(3).max(8);
        let mut lines = Vec::new();
        let header_style = Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD);
        let row_style = Style::default();
        let selected_style = selection_highlight_style();
        let selected_index = dialog.state.selected_item_index();
        let selected_focus = dialog.state.focus() == PermissionStudioFocus::Items;

        lines.push(Line::from(Span::styled(
            settings_table_columns(
                &[
                    (left_header.as_str(), left_width as usize),
                    (right_header.as_str(), right_width as usize),
                ],
                width,
            ),
            header_style,
        )));
        lines.push(Line::from(Span::styled(
            "─".repeat(width as usize),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));

        if section.items.is_empty() {
            lines.push(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-settings-empty-items"),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )));
            return Text::from(lines);
        }

        for (index, item) in section.items.iter().enumerate() {
            let is_selected = index == selected_index;
            let style = if is_selected && selected_focus {
                selected_style
            } else if is_selected {
                Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD)
            } else {
                row_style
            };
            let marker = if is_selected { ">> " } else { "   " };
            let label_width = (left_width as usize).saturating_sub(UnicodeWidthStr::width(marker));
            let label = permission_studio_table_label(item, section.id, label_width);
            let left = format!("{marker}{label}");
            lines.push(Line::from(Span::styled(
                settings_table_columns(
                    &[
                        (left.as_str(), left_width as usize),
                        (item.value.as_str(), right_width as usize),
                    ],
                    width,
                ),
                style,
            )));
        }

        Text::from(lines)
    }

    pub(in crate::app) fn permission_studio_table_headers(
        section_id: PermissionStudioSectionId,
    ) -> (String, String) {
        match section_id {
            PermissionStudioSectionId::PathDefaults => ("Setting".to_string(), "Value".to_string()),
            PermissionStudioSectionId::PathRules => ("Path".to_string(), "Access".to_string()),
            PermissionStudioSectionId::NetworkZones => ("Zone".to_string(), "Connect".to_string()),
            PermissionStudioSectionId::NetworkRules => {
                ("Domain".to_string(), "Connect".to_string())
            }
            PermissionStudioSectionId::ToolTags => ("Tag".to_string(), "Access".to_string()),
            PermissionStudioSectionId::ToolNames => ("Name".to_string(), "Access".to_string()),
            PermissionStudioSectionId::ToolCommandRules => {
                ("Tool".to_string(), "Access".to_string())
            }
            PermissionStudioSectionId::RootPath
            | PermissionStudioSectionId::RootNetwork
            | PermissionStudioSectionId::RootTools => ("Item".to_string(), "Summary".to_string()),
        }
    }

    pub(in crate::app) fn render_permission_rule_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionRuleStudioOverlay,
        surface: SurfaceMode,
    ) {
        let overview_body = build_detail_text(
            vec![
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-draft"),
                    sanitize_display_text(permission_rule_draft_label(&self.i18n, &dialog.draft)),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-mode"),
                    sanitize_display_text(permission_rule_mode_label(dialog.draft.mode)),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-scope"),
                    sanitize_display_text(dialog.draft.scope.as_str()),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-session"),
                    sanitize_display_text(if dialog.draft.session_id.trim().is_empty() {
                        ui_text::t(&self.i18n, "value-unset")
                    } else {
                        dialog.draft.session_id.clone()
                    }),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(
                        &self.i18n,
                        "overlay-permission-rule-overview-workspace-root",
                    ),
                    sanitize_display_text(if dialog.draft.workspace_root.trim().is_empty() {
                        ui_text::t(&self.i18n, "value-runtime-default")
                    } else {
                        dialog.draft.workspace_root.clone()
                    }),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
            ],
            &DetailTextSpec::label_width(14),
        );
        let detail_text = dialog
            .presentation
            .list
            .selected_item()
            .map(|item| permission_rule_studio_detail_text(&self.i18n, &dialog.draft, item))
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-permission-rule-empty-detail"));

        let list_items = dialog
            .presentation
            .list
            .items
            .iter()
            .map(|item| {
                build_accented_two_line_list_item(
                    sanitize_display_text(item.label.as_str()).into(),
                    Some(sanitize_display_text(item.value.as_str()).into()),
                    Some(sanitize_display_text(item.detail.as_str()).into()),
                )
            })
            .collect::<Vec<_>>();
        let spec = ListWorkbenchDialogSpec::new(
            sanitize_display_text(dialog.presentation.title.as_str()).into(),
            sanitize_display_text(dialog.presentation.footer.as_str()).into(),
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 2,
                    min_body_height: 8,
                    max_body_height: 18,
                },
                Some(ui_text::t(&self.i18n, "overlay-permission-rule-fields").into()),
                list_items.as_slice(),
                (!dialog.presentation.list.items.is_empty())
                    .then_some(dialog.presentation.list.selected),
                selection_highlight_style(),
                ">> ".into(),
            ),
            vec![
                WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-workbench-overview").into(),
                    overview_body,
                    3,
                    8,
                ),
                WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-workbench-details").into(),
                    Text::from(sanitize_display_text(detail_text)),
                    4,
                    14,
                ),
            ],
        )
        .summary(Some(
            sanitize_display_text(permission_rule_subject_kind_name(dialog.draft.subject_kind))
                .into(),
        ))
        .target_width(132)
        .left_panel_width(40)
        .overlay(
            dialog
                .editor
                .as_ref()
                .map(WorkbenchOverlayDialogSpec::from_source),
        );
        render_list_workbench_dialog(frame, area, surface, &spec);
    }
}
