use super::super::{
    ConfigDiagnostic, ConfigGroupLayout, ConfigGroupView, ConfigOverviewCard, ConfigRowCell,
    ConfigRowView, ConfigSectionBody, ConfigSectionView, Line, Modifier, PathSegment,
    PluginConfigFocus, PluginWorkbenchOverlay, PluginWorkbenchPlugin, Span, Style, Text, clean,
    diagnostic_severity_label, fixed_columns, group_has_action_column, pad_to_width, path_display,
    plugin_uses_compact_config_layout, plugin_workbench_selection_highlight_style, row_visible,
    section_selected_row_cell,
};
pub(in crate::app) fn config_editor_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let mut lines = Vec::new();
    if let Some(section) = dialog.selected_section() {
        let highlight_selection = !plugin_uses_compact_config_layout(plugin)
            || dialog.config_focus == PluginConfigFocus::Editor;
        append_section_lines(&mut lines, dialog, plugin, section, 98, highlight_selection);
    } else {
        lines.push(Line::from("No config section."));
    }
    Text::from(lines)
}

pub(in crate::app) fn append_section_lines(
    lines: &mut Vec<Line<'static>>,
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
    section: &ConfigSectionView,
    width: u16,
    highlight_selection: bool,
) {
    match &section.body {
        ConfigSectionBody::Overview {
            cards,
            lines: summary,
        } => {
            append_overview_section_lines(lines, cards.as_slice(), summary.as_slice(), width);
        }
        ConfigSectionBody::Form { notice, groups } => {
            lines.push(Line::from(Span::styled(
                section.title.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            if let Some(notice) = notice.as_deref() {
                lines.push(Line::from(""));
                lines.push(Line::from(clean(notice)));
            }
            for group in groups {
                if !group
                    .rows
                    .iter()
                    .any(|row| row_visible(row, dialog.config_view))
                {
                    continue;
                }
                lines.push(Line::from(""));
                append_group_lines(
                    lines,
                    dialog,
                    plugin,
                    section,
                    group,
                    width,
                    highlight_selection,
                );
            }
        }
    }
}

pub(in crate::app) fn append_overview_section_lines(
    lines: &mut Vec<Line<'static>>,
    cards: &[ConfigOverviewCard],
    summary: &[String],
    width: u16,
) {
    lines.push(Line::from(Span::styled(
        "Overview".to_owned(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        fixed_columns(&[("Section", 12), ("Summary", 68), ("State", 12)], width),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for card in cards {
        lines.push(Line::from(fixed_columns(
            &[
                (card.title.as_str(), 12),
                (card.summary.as_str(), 68),
                (card.issue_label.as_deref().unwrap_or(""), 12),
            ],
            width,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Config".to_owned(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for line in summary {
        lines.push(Line::from(clean(line)));
    }
}

pub(in crate::app) fn standard_config_row_line(
    setting: &str,
    type_display: &str,
    value: &str,
    default: &str,
    state: &str,
    width: u16,
) -> String {
    fixed_columns(
        &[
            (setting, 22),
            (type_display, 16),
            (value, 22),
            (default, 18),
            (state, 10),
        ],
        width,
    )
}

pub(in crate::app) fn standard_config_row_line_with_action(
    setting: &str,
    type_display: &str,
    value: &str,
    default: &str,
    action: &str,
    state: &str,
    width: u16,
) -> String {
    fixed_columns(
        &[
            (setting, 18),
            (type_display, 14),
            (value, 18),
            (default, 14),
            (action, 14),
            (state, 8),
        ],
        width,
    )
}

pub(in crate::app) fn pair_config_row_line_with_action(
    setting: &str,
    value: &str,
    secondary_value: &str,
    action: &str,
    state: &str,
    width: u16,
) -> String {
    fixed_columns(
        &[
            (setting, 20),
            (value, 18),
            (secondary_value, 18),
            (action, 14),
            (state, 8),
        ],
        width,
    )
}

pub(in crate::app) fn styled_fixed_columns(
    columns: &[(String, usize, Style)],
    width: u16,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (index, (text, size, style)) in columns.iter().enumerate() {
        if index > 0 {
            if used >= width as usize {
                break;
            }
            spans.push(Span::raw("  "));
            used += 2;
        }
        let remaining = width.saturating_sub(used as u16) as usize;
        if remaining == 0 {
            break;
        }
        let size = (*size).min(remaining);
        let cell = pad_to_width(clean(text).as_str(), size);
        spans.push(Span::styled(cell, *style));
        used += size;
    }
    Line::from(spans)
}

pub(in crate::app) fn config_row_title_style(selected_cell: Option<ConfigRowCell>) -> Style {
    if selected_cell.is_some() {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

pub(in crate::app) fn config_row_cell_style(
    selected_cell: Option<ConfigRowCell>,
    cell: ConfigRowCell,
) -> Style {
    if selected_cell == Some(cell) {
        plugin_workbench_selection_highlight_style()
    } else {
        Style::default()
    }
}

pub(in crate::app) fn standard_config_row_line_with_focus(
    row: &ConfigRowView,
    width: u16,
    selected_cell: Option<ConfigRowCell>,
    include_action: bool,
) -> Line<'static> {
    let mut columns = if include_action {
        vec![
            (row.title.clone(), 18, config_row_title_style(selected_cell)),
            (
                row.type_display.clone(),
                14,
                config_row_cell_style(selected_cell, ConfigRowCell::Type),
            ),
            (
                row.value_display.clone(),
                18,
                config_row_cell_style(selected_cell, ConfigRowCell::Value),
            ),
            (
                row.default_display.clone(),
                14,
                config_row_cell_style(selected_cell, ConfigRowCell::Default),
            ),
            (
                row.action_display.clone().unwrap_or_default(),
                14,
                config_row_cell_style(selected_cell, ConfigRowCell::Action),
            ),
            (
                row.state.label().to_owned(),
                8,
                config_row_cell_style(selected_cell, ConfigRowCell::State),
            ),
        ]
    } else {
        vec![
            (row.title.clone(), 22, config_row_title_style(selected_cell)),
            (
                row.type_display.clone(),
                16,
                config_row_cell_style(selected_cell, ConfigRowCell::Type),
            ),
            (
                row.value_display.clone(),
                22,
                config_row_cell_style(selected_cell, ConfigRowCell::Value),
            ),
            (
                row.default_display.clone(),
                18,
                config_row_cell_style(selected_cell, ConfigRowCell::Default),
            ),
            (
                row.state.label().to_owned(),
                10,
                config_row_cell_style(selected_cell, ConfigRowCell::State),
            ),
        ]
    };
    styled_fixed_columns(columns.as_mut_slice(), width)
}

pub(in crate::app) fn pair_config_row_line_with_focus(
    row: &ConfigRowView,
    width: u16,
    selected_cell: Option<ConfigRowCell>,
    include_action: bool,
) -> Line<'static> {
    let mut columns = if include_action {
        vec![
            (row.title.clone(), 20, config_row_title_style(selected_cell)),
            (
                row.value_display.clone(),
                18,
                config_row_cell_style(selected_cell, ConfigRowCell::Value),
            ),
            (
                row.secondary_value_display.clone().unwrap_or_default(),
                18,
                config_row_cell_style(selected_cell, ConfigRowCell::SecondaryValue),
            ),
            (
                row.action_display.clone().unwrap_or_default(),
                14,
                config_row_cell_style(selected_cell, ConfigRowCell::Action),
            ),
            (
                row.state.label().to_owned(),
                8,
                config_row_cell_style(selected_cell, ConfigRowCell::State),
            ),
        ]
    } else {
        vec![
            (row.title.clone(), 24, config_row_title_style(selected_cell)),
            (
                row.value_display.clone(),
                20,
                config_row_cell_style(selected_cell, ConfigRowCell::Value),
            ),
            (
                row.secondary_value_display.clone().unwrap_or_default(),
                20,
                config_row_cell_style(selected_cell, ConfigRowCell::SecondaryValue),
            ),
            (
                row.state.label().to_owned(),
                10,
                config_row_cell_style(selected_cell, ConfigRowCell::State),
            ),
        ]
    };
    styled_fixed_columns(columns.as_mut_slice(), width)
}

pub(in crate::app) fn append_group_lines(
    lines: &mut Vec<Line<'static>>,
    dialog: &PluginWorkbenchOverlay,
    _plugin: &PluginWorkbenchPlugin,
    section: &ConfigSectionView,
    group: &ConfigGroupView,
    width: u16,
    highlight_selection: bool,
) {
    lines.push(Line::from(Span::styled(
        group.title.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    let include_action = group_has_action_column(group, dialog.config_view);
    match group.layout {
        ConfigGroupLayout::Standard => {
            lines.push(Line::from(Span::styled(
                if include_action {
                    standard_config_row_line_with_action(
                        "Setting", "Type", "Value", "Default", "Action", "State", width,
                    )
                } else {
                    standard_config_row_line("Setting", "Type", "Value", "Default", "State", width)
                },
                Style::default().add_modifier(Modifier::BOLD),
            )));
        }
        ConfigGroupLayout::Pair {
            left_label,
            right_label,
        } => {
            lines.push(Line::from(Span::styled(
                if include_action {
                    pair_config_row_line_with_action(
                        "Setting",
                        left_label,
                        right_label,
                        "Action",
                        "State",
                        width,
                    )
                } else {
                    fixed_columns(
                        &[
                            ("Setting", 24),
                            (left_label, 20),
                            (right_label, 20),
                            ("State", 10),
                        ],
                        width,
                    )
                },
                Style::default().add_modifier(Modifier::BOLD),
            )));
        }
    }
    let mut visible_row_index = 0usize;
    for group_cursor in section_form_groups(section) {
        for row in &group_cursor.rows {
            if !row_visible(row, dialog.config_view) {
                continue;
            }
            let is_selected = dialog.selected_section == section_index_for_row(dialog, section)
                && dialog.selected_node == visible_row_index;
            if std::ptr::eq(group_cursor, group) {
                let focused_cell = if is_selected && highlight_selection {
                    Some(section_selected_row_cell(
                        section,
                        dialog.config_view,
                        visible_row_index,
                        dialog.selected_cell,
                    ))
                } else {
                    None
                };
                let line = match group.layout {
                    ConfigGroupLayout::Standard => standard_config_row_line_with_focus(
                        row,
                        width,
                        focused_cell,
                        include_action,
                    ),
                    ConfigGroupLayout::Pair { .. } => {
                        pair_config_row_line_with_focus(row, width, focused_cell, include_action)
                    }
                };
                lines.push(line);
            }
            visible_row_index += 1;
        }
    }
}

pub(in crate::app) fn section_form_groups(section: &ConfigSectionView) -> &[ConfigGroupView] {
    match &section.body {
        ConfigSectionBody::Form { groups, .. } => groups.as_slice(),
        ConfigSectionBody::Overview { .. } => &[],
    }
}

pub(in crate::app) fn section_index_for_row(
    dialog: &PluginWorkbenchOverlay,
    section: &ConfigSectionView,
) -> usize {
    dialog
        .selected_plugin()
        .and_then(|plugin| {
            plugin
                .sections
                .iter()
                .position(|candidate| candidate.key == section.key)
        })
        .unwrap_or_default()
}

pub(in crate::app) fn pair_editor_labels(
    left_path: &[PathSegment],
    right_path: &[PathSegment],
) -> (&'static str, &'static str) {
    let left_last = left_path.last().and_then(path_segment_key_name);
    let right_last = right_path.last().and_then(path_segment_key_name);
    let left_has_defaults = left_path
        .iter()
        .filter_map(path_segment_key_name)
        .any(|segment| segment == "defaults");
    let right_has_limits = right_path
        .iter()
        .filter_map(path_segment_key_name)
        .any(|segment| segment == "limits");
    if left_has_defaults && right_has_limits {
        return ("Value", "Limit");
    }
    if left_last.is_some_and(|name| name.starts_with("default"))
        && right_last.is_some_and(|name| name.starts_with("max"))
    {
        return ("Value", "Max");
    }
    ("Value 1", "Value 2")
}

pub(in crate::app) fn path_segment_key_name(segment: &PathSegment) -> Option<&str> {
    match segment {
        PathSegment::Key(key) => Some(key.as_str()),
        PathSegment::Index(_) => None,
    }
}

pub(in crate::app) fn plugin_all_diagnostics(
    plugin: &PluginWorkbenchPlugin,
) -> Vec<ConfigDiagnostic> {
    let mut diagnostics = plugin.diagnostics.clone();
    diagnostics.extend(plugin.runtime_diagnostics.clone());
    diagnostics
}

pub(in crate::app) fn diagnostics_text(
    diagnostics: &[ConfigDiagnostic],
    highlight_selection: bool,
    selected_row: usize,
) -> Text<'static> {
    let table_width = 112;
    let mut lines = Vec::new();
    if diagnostics.is_empty() {
        lines.push(Line::from("No issues"));
    } else {
        lines.push(Line::from(Span::styled(
            fixed_columns(
                &[
                    ("Severity", 10),
                    ("Source", 10),
                    ("Field", 22),
                    ("Message", 80),
                ],
                table_width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (index, diagnostic) in diagnostics.iter().enumerate() {
            let line = fixed_columns(
                &[
                    (diagnostic_severity_label(diagnostic.severity), 10),
                    (diagnostic.source.as_str(), 10),
                    (diagnostic.field.as_str(), 22),
                    (diagnostic.message.as_str(), 80),
                ],
                table_width,
            );
            let style = if highlight_selection && index == selected_row {
                plugin_workbench_selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(clean(line), style)));
        }
    }
    Text::from(lines)
}

pub(in crate::app) fn config_diff_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let table_width = 116;
    let mut lines = Vec::new();
    if plugin.diff.is_empty() {
        lines.push(Line::from("No changes"));
    } else {
        lines.push(Line::from(Span::styled(
            fixed_columns(
                &[("Field", 28), ("Before", 28), ("After", 28), ("Change", 28)],
                table_width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for (index, row) in plugin.diff.iter().enumerate() {
            let line = fixed_columns(
                &[
                    (path_display(&row.path).as_str(), 28),
                    (row.before.as_str(), 28),
                    (row.after.as_str(), 28),
                    (row.summary.as_str(), 28),
                ],
                table_width,
            );
            let style = if dialog.config_focus == PluginConfigFocus::Diagnostics
                && dialog.show_diff
                && index == dialog.selected_diff_row
            {
                plugin_workbench_selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(clean(line), style)));
        }
    }
    Text::from(lines)
}
