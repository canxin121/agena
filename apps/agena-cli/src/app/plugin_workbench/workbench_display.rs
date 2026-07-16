pub(in crate::app) fn transport_display(transport: &str) -> &str {
    match transport {
        "static" => "native",
        other => other,
    }
}

pub(in crate::app) fn plugin_uses_compact_config_layout(plugin: &PluginWorkbenchPlugin) -> bool {
    let _ = plugin;
    true
}

pub(in crate::app) fn compact_plugin_label(plugin: &PluginWorkbenchPlugin) -> String {
    plugin
        .plugin_id
        .rsplit('.')
        .next()
        .unwrap_or(plugin.plugin_id.as_str())
        .to_owned()
}

pub(in crate::app) fn compact_config_header_line(plugin: &PluginWorkbenchPlugin) -> String {
    let save_state = if plugin.dirty { "Dirty" } else { "Saved" };
    let config_label = format!("Config: {} / {}", plugin.config_status.label, save_state);
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

pub(in crate::app) fn compact_config_view_line(
    plugin: &PluginWorkbenchPlugin,
    dialog: &PluginWorkbenchOverlay,
) -> String {
    let cell_label = selected_config_row_context(dialog)
        .map(|context| config_row_cell_label(&context.row, context.layout, context.cell).to_owned())
        .unwrap_or_else(|| "Value".to_owned());
    format!(
        "Changed: {}  Cell: {}  Tab/Shift+Tab moves focus; Enter activates the selected cell",
        override_leaf_count(&plugin.draft_override),
        cell_label,
    )
}

pub(in crate::app) fn drilldown_footer_text(
    _dialog: &PluginWorkbenchOverlay,
    _overlay: &PluginConfigDrilldownOverlay,
) -> String {
    "Arrows navigate cells  Enter activates the selected cell  Ctrl+D removes selected  Esc returns"
        .to_owned()
}

pub(in crate::app) fn compact_config_toolbar_text() -> Text<'static> {
    agena_tui_components::build_shortcut_bar([
        agena_tui_components::ShortcutHint::new("Ctrl+K", "validate"),
        agena_tui_components::ShortcutHint::new("Ctrl+U", "reset all"),
        agena_tui_components::ShortcutHint::new("Ctrl+P", "diff"),
        agena_tui_components::ShortcutHint::new("Ctrl+S", "save"),
        agena_tui_components::ShortcutHint::new("Ctrl+R", "restart"),
        agena_tui_components::ShortcutHint::new("Ctrl+D", "remove selected"),
    ])
}

pub(in crate::app) fn compact_config_sections_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
    width: u16,
) -> Text<'static> {
    let mut lines = vec![Line::from(Span::styled(
        "Sections",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(""));
    let content_width = width.saturating_sub(1).max(6) as usize;
    for (index, section) in plugin.sections.iter().enumerate() {
        let mut label = section.title.clone();
        if section.issue_count > 0 {
            label.push_str(format!(" !{}", section.issue_count).as_str());
        } else if section.dirty {
            label.push_str(" dirty");
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

pub(in crate::app) fn plugin_text_display_mode_from_declared(
    value: Option<agena::plugin::UiTextDisplayMode>,
) -> Option<PluginTextDisplayMode> {
    match value {
        Some(agena::plugin::UiTextDisplayMode::Summary) => Some(PluginTextDisplayMode::Summary),
        Some(agena::plugin::UiTextDisplayMode::Detailed) => Some(PluginTextDisplayMode::Detailed),
        None => None,
    }
}

pub(in crate::app) fn plugin_text_display_mode_label(mode: PluginTextDisplayMode) -> &'static str {
    match mode {
        PluginTextDisplayMode::Detailed => "detailed",
        PluginTextDisplayMode::Summary => "summary",
    }
}

pub(in crate::app) fn plugin_text_display_source_label(
    source: PluginTextDisplaySource,
) -> &'static str {
    match source {
        PluginTextDisplaySource::ToolPolicy => "tool-policy",
        PluginTextDisplaySource::PluginPolicy => "plugin-policy",
        PluginTextDisplaySource::ToolManifest => "tool-manifest",
        PluginTextDisplaySource::PluginManifest => "plugin-manifest",
        PluginTextDisplaySource::GlobalPolicy => "global-policy",
    }
}

use super::{
    Line, Modifier, PluginConfigDrilldownOverlay, PluginConfigFocus, PluginTextDisplayMode,
    PluginTextDisplaySource, PluginWorkbenchOverlay, PluginWorkbenchPlugin, Span, Style, Text,
    config_row_cell_label, fixed_columns, override_leaf_count, pad_to_width,
    plugin_workbench_selection_highlight_style, selected_config_row_context, wrap_prefixed_text,
};
