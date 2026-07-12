pub(in crate::app) fn plugin_policy_sections_text(
    dialog: &PluginPolicyStudioOverlay,
    width: u16,
    height: u16,
) -> Text<'static> {
    let sections = dialog.state.sections();
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "Plugins {}/{}",
            sections
                .len()
                .min(dialog.state.selected_section_index() + 1),
            sections.len()
        ),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(Span::styled(
        format!(
            "Config: {} ({})",
            clean(dialog.config_path.as_str()),
            if dialog.config_found {
                "found"
            } else {
                "missing"
            }
        ),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));
    lines.push(Line::from(""));
    if sections.is_empty() {
        lines.push(Line::from(Span::styled(
            "No plugin entries available.",
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
        return Text::from(lines);
    }

    let visible_limit = width.max(1) as usize;
    let header_rows = lines.len();
    let page_size = height
        .saturating_sub(header_rows as u16)
        .saturating_div(2)
        .max(1) as usize;
    dialog
        .visible_section_page_size
        .set(page_size.min(dialog.state.sections().len()).max(1));
    let (start, end) = plugin_policy_page_range(
        dialog.state.sections().len(),
        dialog.state.selected_section_index(),
        dialog.visible_section_page_size.get(),
    );
    for (index, section) in sections.iter().enumerate().take(end).skip(start) {
        let selected = index == dialog.state.selected_section_index();
        let style = if selected && dialog.state.focus() == SectionedListFocus::Navigation {
            plugin_workbench_selection_highlight_style()
        } else if selected {
            Style::default()
                .fg(agena_tui_components::theme::accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let marker = if selected { ">> " } else { "   " };
        lines.push(Line::from(Span::styled(
            pad_to_width(
                truncate_text(
                    format!("{marker}{}", clean(section.label.as_str())).as_str(),
                    visible_limit,
                )
                .as_str(),
                visible_limit,
            ),
            style,
        )));
        lines.push(Line::from(Span::styled(
            pad_to_width(
                truncate_text(
                    format!("   {}", clean(section.summary.as_str())).as_str(),
                    visible_limit,
                )
                .as_str(),
                visible_limit,
            ),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
    }
    Text::from(lines)
}

pub(in crate::app) fn plugin_policy_table_text(
    dialog: &PluginPolicyStudioOverlay,
    width: u16,
    height: u16,
) -> Text<'static> {
    let mut lines = Vec::new();
    let Some(section) = dialog.selected_section() else {
        lines.push(Line::from(Span::styled(
            "No plugin selected.",
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
        return Text::from(lines);
    };
    lines.push(Line::from(Span::styled(
        clean(section.description.as_str()),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));
    lines.push(Line::from(""));
    let label_width = width.saturating_sub(30).clamp(14, 44) as usize;
    let prompt_width = 12usize;
    let ui_width = 12usize;
    lines.push(Line::from(Span::styled(
        fixed_columns(
            &[
                ("Row", label_width),
                ("Prompt", prompt_width),
                ("UI", ui_width),
            ],
            width,
        ),
        Style::default()
            .fg(agena_tui_components::theme::accent_color())
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "─".repeat(width as usize),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    )));
    if section.items.is_empty() {
        lines.push(Line::from(Span::styled(
            "No policy rows available.",
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
        return Text::from(lines);
    }

    let visible_rows = height.saturating_sub(lines.len() as u16).max(1) as usize;
    let page_size = visible_rows.max(1);
    dialog.visible_item_page_size.set(page_size);
    let (start, end) = plugin_policy_page_range(
        section.items.len(),
        dialog.state.selected_item_index(),
        dialog.visible_item_page_size.get(),
    );
    for index in start..end {
        let item = &section.items[index];
        let selected = index == dialog.state.selected_item_index();
        let style = if selected && dialog.state.focus() == SectionedListFocus::Items {
            plugin_workbench_selection_highlight_style()
        } else if selected {
            Style::default()
                .fg(agena_tui_components::theme::accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let label = if selected {
            format!(">> {}", clean(item.label.as_str()))
        } else {
            format!("   {}", clean(item.label.as_str()))
        };
        let prompt = if selected && dialog.selected_column == PluginPolicyColumn::Prompt {
            format!("[{}]", prompt_mode_label(item.prompt_effective_mode))
        } else {
            prompt_mode_label(item.prompt_effective_mode).to_owned()
        };
        let ui = if selected && dialog.selected_column == PluginPolicyColumn::Ui {
            format!(
                "[{}]",
                plugin_text_display_mode_label(item.ui_effective_mode)
            )
        } else {
            plugin_text_display_mode_label(item.ui_effective_mode).to_owned()
        };
        lines.push(Line::from(Span::styled(
            fixed_columns(
                &[
                    (label.as_str(), label_width),
                    (prompt.as_str(), prompt_width),
                    (ui.as_str(), ui_width),
                ],
                width,
            ),
            style,
        )));
    }
    Text::from(lines)
}

pub(in crate::app) fn plugin_policy_detail_title(dialog: &PluginPolicyStudioOverlay) -> String {
    let column = match dialog.selected_column {
        PluginPolicyColumn::Prompt => "Prompt Policy",
        PluginPolicyColumn::Ui => "UI Policy",
    };
    dialog
        .selected_item()
        .map(|item| format!("{column}: {}", item.scope_label))
        .unwrap_or_else(|| column.to_owned())
}

pub(in crate::app) fn plugin_policy_detail_text(
    dialog: &PluginPolicyStudioOverlay,
) -> Text<'static> {
    let mut lines = Vec::new();
    let Some(section) = dialog.selected_section() else {
        lines.push(Line::from("No plugins available."));
        return Text::from(lines);
    };
    let Some(item) = dialog.selected_item() else {
        lines.push(Line::from(clean(section.description.as_str())));
        return Text::from(lines);
    };

    lines.push(Line::from(format!(
        "Plugin: {}",
        clean(section.label.as_str())
    )));
    lines.push(Line::from(format!(
        "Plugin ID: {}",
        clean(section.plugin_id.as_str())
    )));
    lines.push(Line::from(format!(
        "Scope: {}",
        clean(item.scope_label.as_str())
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(clean(item.description.as_str())));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Prompt",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(format!(
        "Stored value: {}",
        prompt_override_label(item.prompt_file_override)
    )));
    lines.push(Line::from(format!(
        "Effective value: {}",
        prompt_mode_label(item.prompt_effective_mode)
    )));
    lines.push(Line::from(format!(
        "Source: {}",
        plugin_text_display_source_label(item.prompt_source)
    )));
    if let Some(mode) = item.prompt_tool_default {
        lines.push(Line::from(format!(
            "Tool default: {}",
            prompt_mode_label(mode)
        )));
    }
    if let Some(mode) = item.prompt_plugin_declared_default {
        lines.push(Line::from(format!(
            "Plugin default: {}",
            prompt_mode_label(mode)
        )));
    }
    lines.push(Line::from(format!("Writes to: {}", item.prompt_path)));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "UI",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(format!(
        "Stored value: {}",
        ui_override_label(item.ui_file_override)
    )));
    lines.push(Line::from(format!(
        "Effective value: {}",
        plugin_text_display_mode_label(item.ui_effective_mode)
    )));
    lines.push(Line::from(format!(
        "Source: {}",
        plugin_text_display_source_label(item.ui_source)
    )));
    if let Some(mode) = item.ui_tool_default {
        lines.push(Line::from(format!(
            "Tool default: {}",
            plugin_text_display_mode_label(mode)
        )));
    }
    if let Some(mode) = item.ui_plugin_declared_default {
        lines.push(Line::from(format!(
            "Plugin default: {}",
            plugin_text_display_mode_label(mode)
        )));
    }
    lines.push(Line::from(format!("Writes to: {}", item.ui_path)));
    Text::from(lines)
}

pub(in crate::app) fn render_plugin_list_page(
    frame: &mut Frame,
    area: Rect,
    dialog: &PluginWorkbenchOverlay,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(area);
    let filter_line = format!(
        "Search plugins... {}        Transport: {}        Config: {}        {}/{} shown",
        if dialog.query.text().is_empty() {
            "all".to_owned()
        } else {
            clean(dialog.query.text())
        },
        dialog.transport_filter.label(),
        dialog.config_filter.label(),
        dialog.visible_plugins.len(),
        dialog.plugins.len()
    );
    let control_labels = [
        format!("Transport: {}", dialog.transport_filter.label()),
        format!("Config: {}", dialog.config_filter.label()),
        "Refresh".to_owned(),
    ];
    let control_spans = control_labels
        .iter()
        .enumerate()
        .flat_map(|(index, label)| {
            let focused = dialog.list_controls_focused && dialog.selected_list_control == index;
            [
                Span::styled(
                    format!("[ {label} ]"),
                    if focused {
                        plugin_workbench_selection_highlight_style()
                    } else {
                        Style::default().fg(agena_tui_components::theme::muted_color())
                    },
                ),
                Span::raw(" "),
            ]
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(vec![Line::from(filter_line), Line::from(control_spans)])
            .wrap(Wrap { trim: false }),
        rows[0],
    );

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        fixed_columns(
            &[
                ("Plugin", 22),
                ("Visible Tool", 16),
                ("Version", 12),
                ("Transport", 11),
                ("Tools", 7),
                ("Commands", 10),
                ("Config", 16),
            ],
            area.width.saturating_sub(4),
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    if dialog.visible_plugins.is_empty() {
        lines.push(Line::from("No plugins match the current filters."));
    } else {
        for (visible_row, plugin_index) in dialog.visible_plugins.iter().enumerate() {
            let Some(plugin) = dialog.plugins.get(*plugin_index) else {
                continue;
            };
            let selected = visible_row == dialog.selected_plugin;
            let marker = if selected { ">> " } else { "   " };
            let line = format!(
                "{}{}",
                marker,
                fixed_columns(
                    &[
                        (plugin.plugin_id.as_str(), 22),
                        (plugin.visible_tool.as_str(), 16),
                        (plugin.version.as_str(), 12),
                        (transport_display(plugin.transport.as_str()), 11),
                        (plugin.tools.len().to_string().as_str(), 7),
                        (plugin.commands.len().to_string().as_str(), 10),
                        (plugin.config_status.label.as_str(), 16),
                    ],
                    area.width.saturating_sub(7),
                )
            );
            let style = if selected {
                plugin_workbench_selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(clean(line), style)));
        }
    }
    render_plugin_panel(frame, rows[1], "Plugins", Text::from(lines), None);
    render_plugin_footer(
        frame,
        rows[2],
        "Type to search  Tab controls/list  Enter activate  Esc close",
    );
}

pub(in crate::app) fn render_plugin_detail_page(
    frame: &mut Frame,
    area: Rect,
    dialog: &PluginWorkbenchOverlay,
) {
    let Some(plugin) = dialog.selected_plugin() else {
        render_plugin_panel(
            frame,
            area,
            "Plugin",
            Text::from("No plugin selected."),
            None,
        );
        return;
    };
    if dialog.detail_tab == PluginDetailTab::Config {
        render_plugin_compact_config_page(frame, area, dialog, plugin);
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(area);
    render_plugin_panel(
        frame,
        rows[0],
        plugin.plugin_id.as_str(),
        plugin_header_text(plugin),
        None,
    );
    render_plugin_tabs(frame, rows[1], dialog.detail_tab);
    let body = match dialog.detail_tab {
        PluginDetailTab::Config => Text::default(),
        PluginDetailTab::Tools => plugin_tools_text(plugin),
        PluginDetailTab::Commands => plugin_commands_text(plugin),
        PluginDetailTab::Capabilities => plugin_capabilities_text(plugin),
        PluginDetailTab::Logs => plugin_logs_text(plugin),
        PluginDetailTab::Diagnostics => plugin_diagnostics_text(plugin),
    };
    render_plugin_panel(frame, rows[2], dialog.detail_tab.label(), body, None);
    render_plugin_footer(frame, rows[3], "Tab next section  Up/Down scroll  Esc back");
}

pub(in crate::app) fn render_plugin_compact_config_page(
    frame: &mut Frame,
    area: Rect,
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) {
    let block = Block::default()
        .title(format!(" {} / Config ", clean(plugin.plugin_id.as_str())))
        .borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(10),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(compact_config_header_line(plugin)).wrap(Wrap { trim: false }),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(compact_config_view_line(plugin, dialog)).wrap(Wrap { trim: false }),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(compact_config_toolbar_text(dialog)).wrap(Wrap { trim: false }),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(compact_config_divider(inner.width)).wrap(Wrap { trim: false }),
        rows[3],
    );

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(20),
            Constraint::Length(1),
            Constraint::Min(28),
        ])
        .split(rows[4]);
    frame.render_widget(
        Paragraph::new(compact_config_sections_text(dialog, plugin, body[0].width))
            .wrap(Wrap { trim: false }),
        body[0],
    );
    frame.render_widget(
        Paragraph::new(compact_vertical_divider(rows[4].height)).wrap(Wrap { trim: false }),
        body[1],
    );
    frame.render_widget(
        Paragraph::new(config_editor_text(dialog, plugin)).wrap(Wrap { trim: false }),
        body[2],
    );
}

pub(in crate::app) fn render_plugin_workbench_editor_overlay(
    frame: &mut Frame,
    area: Rect,
    _workbench_area: Rect,
    dialog: &PluginWorkbenchOverlay,
) {
    if dialog.show_diff
        && dialog
            .selected_plugin()
            .is_some_and(plugin_uses_compact_config_layout)
        && let Some(plugin) = dialog.selected_plugin()
    {
        render_plugin_config_diff_overlay(frame, area, dialog, plugin);
    }
    if let Some(overlay) = dialog.current_drilldown() {
        render_plugin_config_drilldown_overlay(frame, area, dialog, overlay);
    }
    if let Some(selection) = dialog.selection.as_ref() {
        render_plugin_config_selection_overlay(frame, area, selection);
        if let Some(actions) = dialog.actions.as_ref() {
            render_plugin_config_actions_overlay(frame, area, actions);
        }
        return;
    }
    let Some(editor) = dialog.editor.as_ref() else {
        if let Some(actions) = dialog.actions.as_ref() {
            render_plugin_config_actions_overlay(frame, area, actions);
        }
        return;
    };
    render_editor_dialog(
        frame,
        area,
        SurfaceMode::Overlay,
        &EditorDialogSpec {
            title: clean(editor.title.as_str()).into(),
            prompt: clean(editor.prompt.as_str()).into(),
            footer: clean(editor.footer.as_str()).into(),
            target_width: if editor.multiline { 96 } else { 78 },
            multiline: editor.multiline,
            prompt_height_bounds: (1, 5),
            footer_height_bounds: (1, 2),
        },
        &editor.input,
    );
    if let Some(actions) = dialog.actions.as_ref() {
        render_plugin_config_actions_overlay(frame, area, actions);
    }
}

pub(in crate::app) fn render_plugin_config_selection_overlay(
    frame: &mut Frame,
    area: Rect,
    overlay: &PluginConfigSelectionOverlay,
) {
    agena_tui_components::render_search_picker_dialog(
        frame,
        area,
        overlay,
        &agena_tui_components::SearchPickerDialogSpec::new(
            "Loading choices…".into(),
            "Choices".into(),
        ),
        |value| clean(value),
    );
}

pub(in crate::app) fn render_plugin_config_actions_overlay(
    frame: &mut Frame,
    area: Rect,
    overlay: &PluginConfigActionOverlay,
) {
    agena_tui_components::render_search_picker_dialog(
        frame,
        area,
        overlay,
        &agena_tui_components::SearchPickerDialogSpec::new(
            "Loading actions…".into(),
            "Actions".into(),
        ),
        |value| clean(value),
    );
}

pub(in crate::app) fn render_plugin_config_drilldown_overlay(
    frame: &mut Frame,
    area: Rect,
    dialog: &PluginWorkbenchOverlay,
    overlay: &PluginConfigDrilldownOverlay,
) {
    let surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Overlay,
        &FramedSurfaceSpec {
            title: clean(format!(
                "{} · {}",
                overlay.title,
                path_display(&overlay.path)
            ))
            .into(),
            target_width: 108,
            target_height: 30,
        },
    );
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(8), Constraint::Length(1)])
        .split(surface.inner);
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        overlay.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for group in &overlay.groups {
        if !group
            .rows
            .iter()
            .any(|row| row_visible(row, dialog.config_view))
        {
            continue;
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            group.title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            if group_has_action_column(group, dialog.config_view) {
                standard_config_row_line_with_action(
                    "Setting",
                    "Type",
                    "Value",
                    "Default",
                    "Action",
                    "State",
                    rows[0].width.saturating_sub(4),
                )
            } else {
                standard_config_row_line(
                    "Setting",
                    "Type",
                    "Value",
                    "Default",
                    "State",
                    rows[0].width.saturating_sub(4),
                )
            },
            Style::default().add_modifier(Modifier::BOLD),
        )));
        let mut visible_index = 0usize;
        for drill_group in &overlay.groups {
            for row in &drill_group.rows {
                if !row_visible(row, dialog.config_view) {
                    continue;
                }
                if std::ptr::eq(drill_group, group) {
                    let include_action = group_has_action_column(group, dialog.config_view);
                    let focused_cell = if visible_index == overlay.selected_row {
                        Some(drilldown_selected_row_cell(
                            overlay,
                            dialog.config_view,
                            overlay.selected_cell,
                        ))
                    } else {
                        None
                    };
                    lines.push(standard_config_row_line_with_focus(
                        row,
                        rows[0].width.saturating_sub(4),
                        focused_cell,
                        include_action,
                    ));
                }
                visible_index += 1;
            }
        }
    }
    if lines.len() == 1 {
        lines.push(Line::from(""));
        lines.push(Line::from("No editable rows."));
    }
    render_plugin_panel(
        frame,
        rows[0],
        overlay.title.as_str(),
        Text::from(lines),
        None,
    );
    let footer = drilldown_footer_text(dialog, overlay);
    render_plugin_footer(frame, rows[1], footer.as_str());
}

pub(in crate::app) fn render_plugin_config_diff_overlay(
    frame: &mut Frame,
    area: Rect,
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) {
    let surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Overlay,
        &FramedSurfaceSpec {
            title: clean("Config Diff").into(),
            target_width: 112,
            target_height: 18,
        },
    );
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(1)])
        .split(surface.inner);
    render_plugin_panel(
        frame,
        rows[0],
        "Config Diff",
        config_diff_text(dialog, plugin),
        None,
    );
    render_plugin_footer(frame, rows[1], "Esc close");
}

pub(in crate::app) fn render_plugin_panel(
    frame: &mut Frame,
    area: Rect,
    title: impl Into<String>,
    body: Text<'static>,
    scroll: Option<(u16, u16)>,
) {
    let block = Block::default()
        .title(format!(" {} ", clean(title.into())))
        .borders(Borders::ALL);
    let paragraph = Paragraph::new(body)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll(scroll.unwrap_or((0, 0)));
    frame.render_widget(paragraph, area);
}

pub(in crate::app) fn render_plugin_footer(frame: &mut Frame, area: Rect, text: &str) {
    frame.render_widget(Paragraph::new(clean(text)).wrap(Wrap { trim: false }), area);
}

pub(in crate::app) fn render_plugin_tabs(frame: &mut Frame, area: Rect, selected: PluginDetailTab) {
    let mut spans = Vec::new();
    for (index, tab) in PluginDetailTab::ALL.iter().copied().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" | "));
        }
        let style = if tab == selected {
            agena_tui_components::theme::selection_style()
        } else {
            Style::default()
        };
        spans.push(Span::styled(format!(" {} ", tab.label()), style));
    }
    render_plugin_panel(frame, area, "Tabs", Text::from(Line::from(spans)), None);
}
use super::{
    Block, Borders, Constraint, Direction, EditorDialogSpec, Frame, FramedSurfaceSpec, Layout,
    Line, Modifier, Paragraph, PluginConfigActionOverlay, PluginConfigDrilldownOverlay,
    PluginConfigSelectionOverlay, PluginDetailTab, PluginPolicyColumn, PluginPolicyStudioOverlay,
    PluginWorkbenchOverlay, PluginWorkbenchPlugin, Rect, SectionedListFocus, Span, Style,
    SurfaceMode, Text, Wrap, clean, compact_config_divider, compact_config_header_line,
    compact_config_sections_text, compact_config_toolbar_text, compact_config_view_line,
    compact_vertical_divider, config_diff_text, config_editor_text, drilldown_footer_text,
    drilldown_selected_row_cell, fixed_columns, group_has_action_column, pad_to_width,
    path_display, plugin_capabilities_text, plugin_commands_text, plugin_diagnostics_text,
    plugin_header_text, plugin_logs_text, plugin_policy_page_range, plugin_text_display_mode_label,
    plugin_text_display_source_label, plugin_tools_text, plugin_uses_compact_config_layout,
    plugin_workbench_selection_highlight_style, prompt_mode_label, prompt_override_label,
    render_editor_dialog, render_framed_surface, row_visible, standard_config_row_line,
    standard_config_row_line_with_action, standard_config_row_line_with_focus, transport_display,
    truncate_text, ui_override_label,
};
