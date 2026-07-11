use super::super::{
    AgentStudioOverlay, Alignment, App, BoundedListPanelHeight, Color, Constraint, DetailTextLine,
    DetailTextSpec, Direction, EditorDialogSpec, Frame, FramedSurfaceSpec, HeaderRowSpec, Layout,
    Line, ListPanelSpec, ListWorkbenchDialogSpec, ListWorkbenchPanelState, Modifier, Paragraph,
    PermissionRuleStudioOverlay, PermissionStudioFocus, PermissionStudioOverlay,
    PermissionStudioPaneFocus, PermissionStudioSection, PermissionStudioSectionId, Rect,
    SettingsStudioOverlay, Span, Style, SurfaceMode, Text, TextPanelSpec, UnicodeWidthStr,
    VerticalSectionSize, WorkbenchOverlayDialogSpec, WorkbenchTextSection, Wrap,
    adaptive_detail_split, agent_profile_scope_label_localized,
    agent_profile_storage_label_localized, agent_studio_item_detail_text,
    agent_studio_overview_text, build_accented_two_line_list_item, build_detail_text,
    permission_rule_draft_label, permission_rule_mode_label, permission_rule_studio_detail_text,
    permission_rule_subject_kind_name, permission_studio_table_label, render_editor_dialog,
    render_framed_surface, render_header_row, render_list_panel, render_list_workbench_dialog,
    render_text_panel, sanitize_display_text, selection_highlight_style,
    settings_compact_editor_text, settings_compact_fixed_columns,
    settings_compact_item_detail_text, settings_compact_item_detail_title,
    settings_compact_sections_text, settings_compact_vertical_divider, split_vertical_sections,
    ui_text,
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
        let section_title = current_section
            .map(|section| section.label.clone())
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-settings-default-section-title"));
        let frame_title = format!("{} / {}", dialog.title.as_str(), section_title.as_str());

        let framed = render_framed_surface(
            frame,
            area,
            surface,
            &FramedSurfaceSpec {
                title: sanitize_display_text(frame_title).into(),
                target_width: 150,
                target_height: 42,
            },
        );
        let page_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(1)])
            .split(framed.inner);
        let inner = page_rows[0];

        let nav_width = inner
            .width
            .saturating_mul(22)
            .saturating_div(100)
            .clamp(18, 28)
            .min(inner.width.saturating_sub(1));
        let wide_inspector = inner.width >= 100 && inner.height >= 12;
        let inspector_title = settings_compact_item_detail_title(&self.i18n, dialog);
        let inspector_text = settings_compact_item_detail_text(&self.i18n, dialog);

        if wide_inspector {
            let available_after_nav = inner.width.saturating_sub(nav_width).saturating_sub(2);
            let inspector_width = inner
                .width
                .saturating_mul(30)
                .saturating_div(100)
                .clamp(30, 44)
                .min(available_after_nav.saturating_sub(32))
                .max(28);
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(nav_width),
                    Constraint::Length(1),
                    Constraint::Min(32),
                    Constraint::Length(1),
                    Constraint::Length(inspector_width),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(settings_compact_sections_text(
                    &self.i18n,
                    dialog,
                    body[0].width,
                ))
                .wrap(Wrap { trim: false }),
                body[0],
            );
            frame.render_widget(
                Paragraph::new(settings_compact_vertical_divider(body[1].height))
                    .wrap(Wrap { trim: false }),
                body[1],
            );
            frame.render_widget(
                Paragraph::new(settings_compact_editor_text(
                    &self.i18n,
                    dialog,
                    current_section,
                    body[2].width,
                    body[2].height,
                ))
                .wrap(Wrap { trim: false }),
                body[2],
            );
            frame.render_widget(
                Paragraph::new(settings_compact_vertical_divider(body[3].height))
                    .wrap(Wrap { trim: false }),
                body[3],
            );
            render_text_panel(
                frame,
                body[4],
                &TextPanelSpec {
                    title: Some(inspector_title.into()),
                    body: &inspector_text,
                    wrap: true,
                    scroll: None,
                    alignment: None,
                },
            );
        } else {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(nav_width),
                    Constraint::Length(u16::from(inner.width > 0)),
                    Constraint::Min(24),
                ])
                .split(inner);
            let detail_height = body[2]
                .height
                .saturating_mul(35)
                .saturating_div(100)
                .clamp(7, 14)
                .min(body[2].height.saturating_sub(4));
            let editor_rows = if detail_height > 0 {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(4), Constraint::Length(detail_height)])
                    .split(body[2])
            } else {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(0)])
                    .split(body[2])
            };
            frame.render_widget(
                Paragraph::new(settings_compact_sections_text(
                    &self.i18n,
                    dialog,
                    body[0].width,
                ))
                .wrap(Wrap { trim: false }),
                body[0],
            );
            frame.render_widget(
                Paragraph::new(settings_compact_vertical_divider(body[1].height))
                    .wrap(Wrap { trim: false }),
                body[1],
            );
            frame.render_widget(
                Paragraph::new(settings_compact_editor_text(
                    &self.i18n,
                    dialog,
                    current_section,
                    editor_rows[0].width,
                    editor_rows[0].height,
                ))
                .wrap(Wrap { trim: false }),
                editor_rows[0],
            );
            if editor_rows[1].height > 0 {
                render_text_panel(
                    frame,
                    editor_rows[1],
                    &TextPanelSpec {
                        title: Some(inspector_title.into()),
                        body: &inspector_text,
                        wrap: true,
                        scroll: None,
                        alignment: None,
                    },
                );
            }
        }
        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str()))
                .wrap(Wrap { trim: false }),
            page_rows[1],
        );
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
        let selected_item = dialog.workbench.list.selected_item();
        let detail_body = selected_item
            .map(|item| {
                agent_studio_item_detail_text(&self.i18n, &dialog.profile, item, dialog.storage)
            })
            .unwrap_or_else(|| {
                Text::from(ui_text::t(&self.i18n, "overlay-agent-studio-empty-detail"))
            });

        let list_items = dialog
            .workbench
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
            sanitize_display_text(dialog.workbench.title.as_str()).into(),
            Some(sanitize_display_text(title_summary.as_str()).into()),
            sanitize_display_text(dialog.workbench.footer.as_str()).into(),
            138,
            36,
            None,
            None,
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 2,
                    min_body_height: 6,
                    max_body_height: 16,
                },
                Some(ui_text::t(&self.i18n, "overlay-agent-studio-fields").into()),
                list_items.as_slice(),
                (!dialog.workbench.list.items.is_empty()).then_some(dialog.workbench.list.selected),
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
            dialog
                .workbench
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
        let framed = render_framed_surface(
            frame,
            area,
            surface,
            &FramedSurfaceSpec {
                title: sanitize_display_text(dialog.title.as_str()).into(),
                target_width: 140,
                target_height: area.height,
            },
        );
        if framed.inner.width == 0 || framed.inner.height == 0 {
            return;
        }

        let header_height = if framed.inner.height >= 4 { 2 } else { 1 };
        let footer_height = if framed.inner.height >= 3 { 1 } else { 0 };
        let body_height = framed
            .inner
            .height
            .saturating_sub(header_height)
            .saturating_sub(footer_height);
        let sections = split_vertical_sections(
            framed.inner,
            &[
                VerticalSectionSize::Fixed(header_height),
                VerticalSectionSize::Flexible(body_height.max(1)),
                VerticalSectionSize::Fixed(footer_height),
            ],
        );
        let header_area = sections.first().copied().unwrap_or(framed.inner);
        let body_area = sections.get(1).copied().unwrap_or(framed.inner);
        let footer_area = sections.get(2).copied().unwrap_or(Rect {
            x: framed.inner.x,
            y: framed
                .inner
                .y
                .saturating_add(framed.inner.height.saturating_sub(1)),
            width: framed.inner.width,
            height: 1,
        });

        render_header_row(
            frame,
            header_area,
            &HeaderRowSpec {
                left: sanitize_display_text(dialog.title_context.as_str()).into(),
                right: Some(
                    sanitize_display_text(format!(
                        "{} · {} · {}",
                        dialog.source_label.as_str(),
                        dialog.scope_label.as_str(),
                        if dialog.editable {
                            ui_text::t(&self.i18n, "value-editable")
                        } else {
                            ui_text::t(&self.i18n, "value-read-only")
                        }
                    ))
                    .into(),
                ),
                left_style: Style::default()
                    .fg(self.theme_color("accent", Color::Cyan))
                    .add_modifier(Modifier::BOLD),
                right_style: Style::default().fg(Color::DarkGray),
            },
        );

        let body_constraints = adaptive_detail_split(body_area.width, 26, 40);
        let body_sections = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(body_constraints)
            .split(body_area);
        let nav_area = body_sections.first().copied().unwrap_or(body_area);
        let table_area = body_sections.get(1).copied().unwrap_or(body_area);

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
        let nav_spec = ListPanelSpec::new(
            Some(ui_text::t(&self.i18n, "overlay-permission-studio-nav").into()),
            nav_items.as_slice(),
            Some(dialog.nav.selected),
            if dialog.pane_focus == PermissionStudioPaneFocus::Navigation {
                selection_highlight_style()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            },
            ">> ".into(),
        );
        render_list_panel(frame, nav_area, &nav_spec);

        let Some(selected_section) = dialog.state.selected_section() else {
            let empty_text = Text::from(ui_text::t(&self.i18n, "overlay-settings-empty-section"));
            render_text_panel(
                frame,
                table_area,
                &TextPanelSpec {
                    title: Some(ui_text::t(&self.i18n, "overlay-workbench-overview").into()),
                    body: &empty_text,
                    wrap: true,
                    scroll: None,
                    alignment: None,
                },
            );
            if footer_area.height > 0 {
                self.render_permission_studio_footer_row(frame, footer_area, dialog);
            }
            return;
        };
        let table_text =
            self.permission_studio_table_text(dialog, selected_section, table_area.width);
        render_text_panel(
            frame,
            table_area,
            &TextPanelSpec {
                title: Some(sanitize_display_text(selected_section.label.as_str()).into()),
                body: &table_text,
                wrap: false,
                scroll: None,
                alignment: None,
            },
        );

        if footer_area.height > 0 {
            self.render_permission_studio_footer_row(frame, footer_area, dialog);
        }

        if let Some(editor) = dialog.editor.as_ref() {
            render_editor_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                &EditorDialogSpec {
                    title: editor.title.clone().into(),
                    prompt: editor.prompt.clone().into(),
                    footer: editor.footer.clone().into(),
                    target_width: if editor.multiline { 96 } else { 78 },
                    multiline: editor.multiline,
                    prompt_height_bounds: (1, 3),
                    footer_height_bounds: (1, 2),
                },
                &editor.input,
            );
        }
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
            .fg(self.theme_color("accent", Color::Cyan))
            .add_modifier(Modifier::BOLD);
        let row_style = Style::default();
        let selected_style = selection_highlight_style();
        let selected_index = dialog.state.selected_item_index();
        let selected_focus = dialog.state.focus() == PermissionStudioFocus::Items;

        lines.push(Line::from(Span::styled(
            settings_compact_fixed_columns(
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
            Style::default().fg(Color::DarkGray),
        )));

        if section.items.is_empty() {
            lines.push(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-settings-empty-items"),
                Style::default().fg(Color::DarkGray),
            )));
            return Text::from(lines);
        }

        for (index, item) in section.items.iter().enumerate() {
            let is_selected = index == selected_index;
            let style = if is_selected && selected_focus {
                selected_style
            } else if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                row_style
            };
            let marker = if is_selected { ">> " } else { "   " };
            let label_width = (left_width as usize).saturating_sub(UnicodeWidthStr::width(marker));
            let label = permission_studio_table_label(item, section.id, label_width);
            let left = format!("{marker}{label}");
            lines.push(Line::from(Span::styled(
                settings_compact_fixed_columns(
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

    pub(in crate::app) fn render_permission_studio_footer_row(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionStudioOverlay,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let Some(section) = dialog.state.selected_section() else {
            return;
        };
        let buttons = self.permission_studio_footer_buttons(dialog, section.id);
        if buttons.is_empty() {
            return;
        }

        let spans =
            buttons
                .into_iter()
                .enumerate()
                .fold(Vec::new(), |mut acc, (index, (label, style))| {
                    if index > 0 {
                        acc.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
                    }
                    acc.push(Span::styled(format!("[ {label} ]"), style));
                    acc
                });
        frame.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
            area,
        );
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

    pub(in crate::app) fn permission_studio_footer_buttons(
        &self,
        dialog: &PermissionStudioOverlay,
        section_id: PermissionStudioSectionId,
    ) -> Vec<(String, Style)> {
        if !dialog.editable {
            return Vec::new();
        }
        let accent = Style::default()
            .fg(self.theme_color("accent", Color::Cyan))
            .add_modifier(Modifier::BOLD);
        let danger = Style::default()
            .fg(self.theme_color("danger", Color::Red))
            .add_modifier(Modifier::BOLD);
        match section_id {
            PermissionStudioSectionId::PathDefaults
            | PermissionStudioSectionId::NetworkZones
            | PermissionStudioSectionId::RootPath
            | PermissionStudioSectionId::RootNetwork
            | PermissionStudioSectionId::RootTools => Vec::new(),
            PermissionStudioSectionId::PathRules
            | PermissionStudioSectionId::NetworkRules
            | PermissionStudioSectionId::ToolTags
            | PermissionStudioSectionId::ToolNames
            | PermissionStudioSectionId::ToolCommandRules => vec![
                (ui_text::t(&self.i18n, "value-add"), accent),
                (ui_text::t(&self.i18n, "value-edit"), accent),
                (ui_text::t(&self.i18n, "value-duplicate"), accent),
                (ui_text::t(&self.i18n, "value-delete"), danger),
            ],
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
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-mode"),
                    sanitize_display_text(permission_rule_mode_label(dialog.draft.mode)),
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-scope"),
                    sanitize_display_text(dialog.draft.scope.as_str()),
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-session"),
                    sanitize_display_text(if dialog.draft.session_id.trim().is_empty() {
                        ui_text::t(&self.i18n, "value-unset")
                    } else {
                        dialog.draft.session_id.clone()
                    }),
                    Style::default().fg(Color::DarkGray),
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
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
            ],
            &DetailTextSpec::label_width(14),
        );
        let detail_text = dialog
            .workbench
            .list
            .selected_item()
            .map(|item| permission_rule_studio_detail_text(&self.i18n, &dialog.draft, item))
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-permission-rule-empty-detail"));

        let list_items = dialog
            .workbench
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
            sanitize_display_text(dialog.workbench.title.as_str()).into(),
            Some(
                sanitize_display_text(permission_rule_subject_kind_name(dialog.draft.subject_kind))
                    .into(),
            ),
            sanitize_display_text(dialog.workbench.footer.as_str()).into(),
            132,
            40,
            None,
            None,
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 2,
                    min_body_height: 8,
                    max_body_height: 18,
                },
                Some(ui_text::t(&self.i18n, "overlay-permission-rule-fields").into()),
                list_items.as_slice(),
                (!dialog.workbench.list.items.is_empty()).then_some(dialog.workbench.list.selected),
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
            dialog
                .workbench
                .editor
                .as_ref()
                .map(WorkbenchOverlayDialogSpec::from_source),
        );
        render_list_workbench_dialog(frame, area, surface, &spec);
    }
}
