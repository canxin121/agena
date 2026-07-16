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
    let controls = agena_tui_components::build_shortcut_line([
        agena_tui_components::ShortcutHint::new(
            "Alt+T",
            format!("transport {}", dialog.transport_filter.label()),
        ),
        agena_tui_components::ShortcutHint::new(
            "Alt+C",
            format!("config {}", dialog.config_filter.label()),
        ),
        agena_tui_components::ShortcutHint::new("Ctrl+R", "refresh"),
    ]);
    frame.render_widget(
        Paragraph::new(vec![Line::from(filter_line), controls]).wrap(Wrap { trim: false }),
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
        "Type to search · Up/Down select · Enter open · Esc close",
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
    render_plugin_footer(
        frame,
        rows[3],
        "Tab/Alt+Tab section  Up/Down scroll  Esc back",
    );
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
        Paragraph::new(compact_config_toolbar_text()).wrap(Wrap { trim: false }),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(agena_tui_components::build_horizontal_divider(inner.width))
            .wrap(Wrap { trim: false }),
        rows[3],
    );

    let navigation_width = agena_tui_components::workbench_navigation_width(rows[4].width);
    let stacked =
        agena_tui_components::should_stack_detail_layout(rows[4].width, navigation_width, 28);
    let navigation_height = rows[4]
        .height
        .saturating_div(3)
        .clamp(4, 8)
        .min(rows[4].height.saturating_sub(2));
    let body = Layout::default()
        .direction(if stacked {
            Direction::Vertical
        } else {
            Direction::Horizontal
        })
        .constraints(if stacked {
            vec![
                Constraint::Length(navigation_height),
                Constraint::Length(1),
                Constraint::Min(1),
            ]
        } else {
            vec![
                Constraint::Length(navigation_width),
                Constraint::Length(1),
                Constraint::Min(28),
            ]
        })
        .split(rows[4]);
    frame.render_widget(
        Paragraph::new(compact_config_sections_text(dialog, plugin, body[0].width))
            .wrap(Wrap { trim: false }),
        body[0],
    );
    frame.render_widget(
        Paragraph::new(if stacked {
            agena_tui_components::build_horizontal_divider(body[1].width)
        } else {
            agena_tui_components::build_vertical_divider(body[1].height)
        })
        .wrap(Wrap { trim: false }),
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
    PluginConfigSelectionOverlay, PluginDetailTab, PluginWorkbenchOverlay, PluginWorkbenchPlugin,
    Rect, Span, Style, SurfaceMode, Text, Wrap, clean, compact_config_header_line,
    compact_config_sections_text, compact_config_toolbar_text, compact_config_view_line,
    config_diff_text, config_editor_text, drilldown_footer_text, drilldown_selected_row_cell,
    fixed_columns, group_has_action_column, path_display, plugin_capabilities_text,
    plugin_commands_text, plugin_diagnostics_text, plugin_header_text, plugin_logs_text,
    plugin_tools_text, plugin_uses_compact_config_layout,
    plugin_workbench_selection_highlight_style, render_editor_dialog, render_framed_surface,
    row_visible, standard_config_row_line, standard_config_row_line_with_action,
    standard_config_row_line_with_focus, transport_display,
};
