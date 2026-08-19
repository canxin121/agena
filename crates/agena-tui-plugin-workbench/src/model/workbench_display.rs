pub(crate) fn transport_display(transport: &str) -> &str {
    match transport {
        "static" => "native",
        other => other,
    }
}

pub fn plugin_uses_compact_config_layout(plugin: &PluginWorkbenchPlugin) -> bool {
    let _ = plugin;
    true
}

pub(crate) fn compact_plugin_label(plugin: &PluginWorkbenchPlugin) -> String {
    plugin
        .plugin_id
        .rsplit('.')
        .next()
        .unwrap_or(plugin.plugin_id.as_str())
        .to_owned()
}

pub(crate) fn compact_config_header_line(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> String {
    let save_state = if plugin.dirty {
        dialog.i18n.text("plugin-workbench-config-dirty")
    } else {
        dialog.i18n.text("plugin-workbench-config-saved")
    };
    let config_label = dialog.i18n.text_args(
        "plugin-workbench-config-summary",
        &agena_tui::fl_args![
            "status" => plugin.config_status.kind.label(&dialog.i18n),
            "save_state" => save_state,
        ],
    );
    let label = compact_plugin_label(plugin);
    let version_label = format!("v{}", plugin.version);
    fixed_columns(
        &[
            (label.as_str(), 24),
            (version_label.as_str(), 14),
            (transport_display(plugin.transport.as_str()), 14),
            (config_label.as_str(), 30),
        ],
        112,
    )
}

pub(crate) fn compact_config_view_line(
    plugin: &PluginWorkbenchPlugin,
    dialog: &PluginWorkbenchOverlay,
) -> String {
    let cell_label = selected_config_row_context(dialog)
        .map(|context| {
            config_row_cell_label(&dialog.i18n, &context.row, context.layout, context.cell)
        })
        .unwrap_or_else(|| dialog.i18n.text("plugin-workbench-config-value"));
    dialog.i18n.text_args(
        "plugin-workbench-config-view-summary",
        &agena_tui::fl_args![
            "changed" => override_leaf_count(&plugin.draft_override),
            "cell" => cell_label,
        ],
    )
}

pub(crate) fn drilldown_footer_text(
    dialog: &PluginWorkbenchOverlay,
    _overlay: &PluginConfigDrilldownOverlay,
) -> String {
    dialog.i18n.text("plugin-workbench-config-drilldown-footer")
}

pub(crate) fn compact_config_toolbar_text(dialog: &PluginWorkbenchOverlay) -> Text<'static> {
    agena_tui_components::build_shortcut_bar([
        agena_tui_components::ShortcutHint::new(
            "Ctrl+K",
            dialog.i18n.text("plugin-workbench-action-validate"),
        ),
        agena_tui_components::ShortcutHint::new(
            "Ctrl+U",
            dialog.i18n.text("plugin-workbench-action-reset-all"),
        ),
        agena_tui_components::ShortcutHint::new(
            "Ctrl+P",
            dialog.i18n.text("plugin-workbench-action-diff"),
        ),
        agena_tui_components::ShortcutHint::new(
            "Ctrl+S",
            dialog.i18n.text("plugin-workbench-action-save"),
        ),
        agena_tui_components::ShortcutHint::new(
            "Ctrl+R",
            dialog.i18n.text("plugin-workbench-action-restart"),
        ),
        agena_tui_components::ShortcutHint::new(
            "Ctrl+D",
            dialog.i18n.text("plugin-workbench-action-remove-selected"),
        ),
    ])
}

pub(crate) fn compact_config_sections_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
    width: u16,
) -> Text<'static> {
    let mut lines = vec![Line::from(Span::styled(
        dialog.i18n.text("plugin-workbench-sections"),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(""));
    let content_width = width.saturating_sub(1).max(6) as usize;
    for (index, section) in plugin.sections.iter().enumerate() {
        let mut label = section.title.clone();
        if section.issue_count > 0 {
            label.push_str(format!(" !{}", section.issue_count).as_str());
        } else if section.dirty {
            label.push_str(
                format!(" {}", dialog.i18n.text("plugin-workbench-config-dirty")).as_str(),
            );
        }
        let focused =
            dialog.config_focus == PluginConfigFocus::Structure && index == dialog.selected_section;
        let prefixes = if focused { ("> ", "  ") } else { ("  ", "  ") };
        let wrapped = wrap_prefixed_text(label.as_str(), prefixes.0, prefixes.1, content_width);
        for line in wrapped {
            let padded = pad_to_width(line.as_str(), content_width);
            let style = if focused {
                plugin_workbench_selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(padded, style)));
        }
    }
    Text::from(lines)
}

use super::{
    Line, Modifier, PluginConfigDrilldownOverlay, PluginConfigFocus, PluginWorkbenchOverlay,
    PluginWorkbenchPlugin, Span, Style, Text, config_row_cell_label, fixed_columns,
    override_leaf_count, pad_to_width, plugin_workbench_selection_highlight_style,
    selected_config_row_context, wrap_prefixed_text,
};
