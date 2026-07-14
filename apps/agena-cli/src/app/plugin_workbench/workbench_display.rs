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
        "Changed: {}  Cell: {}  Tab/Alt+Tab moves focus; Enter activates the selected control or cell",
        override_leaf_count(&plugin.draft_override),
        cell_label,
    )
}

pub(in crate::app) fn drilldown_footer_text(
    _dialog: &PluginWorkbenchOverlay,
    _overlay: &PluginConfigDrilldownOverlay,
) -> String {
    "Arrows navigate cells  Enter activates the selected cell  Esc returns".to_owned()
}

pub(in crate::app) fn compact_config_toolbar_text(
    dialog: &PluginWorkbenchOverlay,
) -> Text<'static> {
    let mut spans = Vec::new();
    for (index, action) in COMPACT_TOOLBAR_ACTIONS.iter().copied().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }
        let label = format!("[ {} ]", action.label());
        let style = if dialog.config_focus == PluginConfigFocus::Toolbar
            && dialog.selected_toolbar_action == index
        {
            plugin_workbench_selection_highlight_style()
        } else {
            Style::default()
        };
        spans.push(Span::styled(label, style));
    }
    Text::from(Line::from(spans))
}

pub(in crate::app) fn compact_config_divider(width: u16) -> String {
    "─".repeat(width as usize)
}

pub(in crate::app) fn compact_vertical_divider(height: u16) -> Text<'static> {
    Text::from((0..height).map(|_| Line::from("│")).collect::<Vec<_>>())
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

pub(in crate::app) fn plugin_text_display_mode_from_str(
    value: &str,
) -> Option<PluginTextDisplayMode> {
    match value {
        "summary" => Some(PluginTextDisplayMode::Summary),
        "detailed" => Some(PluginTextDisplayMode::Detailed),
        _ => None,
    }
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
        PluginTextDisplaySource::ToolOverride => "tool-override",
        PluginTextDisplaySource::PluginOverride => "plugin-override",
        PluginTextDisplaySource::ToolDefault => "tool-default",
        PluginTextDisplaySource::PluginDefault => "plugin-default",
        PluginTextDisplaySource::GlobalDefault => "global-default",
    }
}

pub(in crate::app) fn plugin_policy_page_range(
    item_count: usize,
    selected_index: usize,
    page_size: usize,
) -> (usize, usize) {
    if item_count == 0 {
        return (0, 0);
    }
    let page_size = page_size.max(1).min(item_count);
    let selected_index = selected_index.min(item_count.saturating_sub(1));
    let start = (selected_index / page_size) * page_size;
    (start, (start + page_size).min(item_count))
}

pub(in crate::app) fn prompt_mode_label(mode: agena::plugin::ToolDescriptionMode) -> &'static str {
    match mode {
        agena::plugin::ToolDescriptionMode::Detailed => "detailed",
        agena::plugin::ToolDescriptionMode::Brief => "brief",
    }
}

pub(in crate::app) fn prompt_override_label(
    mode: Option<agena::plugin::ToolDescriptionOverride>,
) -> &'static str {
    match mode.unwrap_or(agena::plugin::ToolDescriptionOverride::ToolDefault) {
        agena::plugin::ToolDescriptionOverride::ToolDefault => "default",
        agena::plugin::ToolDescriptionOverride::Detailed => "detailed",
        agena::plugin::ToolDescriptionOverride::Brief => "brief",
    }
}

pub(in crate::app) fn ui_override_label(
    mode: Option<agena::plugin::UiPresentationOverride>,
) -> &'static str {
    match mode.unwrap_or(agena::plugin::UiPresentationOverride::Default) {
        agena::plugin::UiPresentationOverride::Default => "default",
        agena::plugin::UiPresentationOverride::Detailed => "detailed",
        agena::plugin::UiPresentationOverride::Summary => "summary",
    }
}

pub(in crate::app) fn plugins_config_from_root(root: &JsonValue) -> agena::plugin::PluginsConfig {
    plugin_get_json_path(root, Some("plugins"))
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub(in crate::app) fn configured_plugin_ids(
    root: &JsonValue,
) -> BTreeSet<agena::plugin::sdk::PluginKey> {
    plugin_get_json_path(root, Some("plugins.list"))
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map(|entries| {
            entries
                .keys()
                .filter_map(|key| key.parse::<agena::plugin::sdk::PluginKey>().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub(in crate::app) fn configured_plugin_kind(
    root: &JsonValue,
    plugin_id: &agena::plugin::sdk::PluginKey,
) -> Option<String> {
    plugin_get_json_path(
        root,
        Some(
            format!(
                "plugins.list.{}",
                quote_settings_segment(&plugin_id.to_string())
            )
            .as_str(),
        ),
    )
    .ok()
    .and_then(|value| value.get("package").cloned())
    .and_then(|value| value.get("kind").cloned())
    .and_then(|value| value.as_str().map(ToOwned::to_owned))
}

pub(in crate::app) fn resolve_prompt_override_mode(
    mode: agena::plugin::ToolDescriptionOverride,
    declared_default: Option<agena::plugin::ToolDescriptionMode>,
    fallback: agena::plugin::ToolDescriptionMode,
) -> agena::plugin::ToolDescriptionMode {
    match mode {
        agena::plugin::ToolDescriptionOverride::ToolDefault => declared_default.unwrap_or(fallback),
        agena::plugin::ToolDescriptionOverride::Detailed => {
            agena::plugin::ToolDescriptionMode::Detailed
        }
        agena::plugin::ToolDescriptionOverride::Brief => agena::plugin::ToolDescriptionMode::Brief,
    }
}

pub(in crate::app) fn resolve_plugin_prompt_mode(
    policy: &agena::plugin::ToolPresentationConfig,
    plugin_id: &agena::plugin::sdk::PluginKey,
    plugin_default: Option<agena::plugin::ToolDescriptionMode>,
) -> agena::plugin::ToolDescriptionMode {
    policy
        .plugins
        .get(plugin_id)
        .copied()
        .map(|mode| resolve_prompt_override_mode(mode, plugin_default, policy.default_mode))
        .unwrap_or_else(|| plugin_default.unwrap_or(policy.default_mode))
}

pub(in crate::app) fn prompt_plugin_source(
    plugin_override: Option<agena::plugin::ToolDescriptionOverride>,
    plugin_default: Option<agena::plugin::ToolDescriptionMode>,
) -> PluginTextDisplaySource {
    match plugin_override {
        Some(agena::plugin::ToolDescriptionOverride::Detailed)
        | Some(agena::plugin::ToolDescriptionOverride::Brief) => {
            PluginTextDisplaySource::PluginOverride
        }
        _ if plugin_default.is_some() => PluginTextDisplaySource::PluginDefault,
        _ => PluginTextDisplaySource::GlobalDefault,
    }
}

pub(in crate::app) fn tool_prompt_override_from_policy(
    policy: &agena::plugin::ToolPresentationConfig,
    plugin_id: &agena::plugin::sdk::PluginKey,
    tool_name: &str,
) -> Option<agena::plugin::ToolDescriptionOverride> {
    if let Ok(key) = agena::plugin::sdk::ToolKey::new(plugin_id.clone(), tool_name.to_owned())
        && let Some(mode) = policy.tools.get(&key).copied()
    {
        return Some(mode);
    }
    None
}

pub(in crate::app) fn prompt_tool_source(
    tool_override: Option<agena::plugin::ToolDescriptionOverride>,
    plugin_override: Option<agena::plugin::ToolDescriptionOverride>,
    tool_default: Option<agena::plugin::ToolDescriptionMode>,
    plugin_default: Option<agena::plugin::ToolDescriptionMode>,
) -> PluginTextDisplaySource {
    match tool_override {
        Some(agena::plugin::ToolDescriptionOverride::Detailed)
        | Some(agena::plugin::ToolDescriptionOverride::Brief) => {
            return PluginTextDisplaySource::ToolOverride;
        }
        _ => {}
    }
    match plugin_override {
        Some(agena::plugin::ToolDescriptionOverride::Detailed)
        | Some(agena::plugin::ToolDescriptionOverride::Brief) => {
            return PluginTextDisplaySource::PluginOverride;
        }
        _ => {}
    }
    if tool_default.is_some() {
        PluginTextDisplaySource::ToolDefault
    } else if plugin_default.is_some() {
        PluginTextDisplaySource::PluginDefault
    } else {
        PluginTextDisplaySource::GlobalDefault
    }
}

pub(in crate::app) fn resolve_ui_override_mode(
    mode: agena::plugin::UiPresentationOverride,
    declared_default: Option<PluginTextDisplayMode>,
    fallback: PluginTextDisplayMode,
) -> PluginTextDisplayMode {
    match mode {
        agena::plugin::UiPresentationOverride::Default => declared_default.unwrap_or(fallback),
        agena::plugin::UiPresentationOverride::Detailed => PluginTextDisplayMode::Detailed,
        agena::plugin::UiPresentationOverride::Summary => PluginTextDisplayMode::Summary,
    }
}

pub(in crate::app) fn resolve_plugin_ui_mode(
    policy: &agena::plugin::UiPresentationConfig,
    plugin_id: &agena::plugin::sdk::PluginKey,
    plugin_default: Option<PluginTextDisplayMode>,
) -> PluginTextDisplayMode {
    policy
        .plugins
        .get(plugin_id)
        .copied()
        .map(|mode| {
            resolve_ui_override_mode(
                mode,
                plugin_default,
                plugin_text_display_mode_from_declared(Some(policy.default_mode))
                    .unwrap_or(PluginTextDisplayMode::Detailed),
            )
        })
        .unwrap_or_else(|| {
            plugin_default.unwrap_or_else(|| {
                plugin_text_display_mode_from_declared(Some(policy.default_mode))
                    .unwrap_or(PluginTextDisplayMode::Detailed)
            })
        })
}

pub(in crate::app) fn ui_plugin_source(
    plugin_override: Option<agena::plugin::UiPresentationOverride>,
    plugin_default: Option<PluginTextDisplayMode>,
) -> PluginTextDisplaySource {
    match plugin_override {
        Some(agena::plugin::UiPresentationOverride::Detailed)
        | Some(agena::plugin::UiPresentationOverride::Summary) => {
            PluginTextDisplaySource::PluginOverride
        }
        _ if plugin_default.is_some() => PluginTextDisplaySource::PluginDefault,
        _ => PluginTextDisplaySource::GlobalDefault,
    }
}

pub(in crate::app) fn tool_ui_override_from_policy(
    policy: &agena::plugin::UiPresentationConfig,
    plugin_id: &agena::plugin::sdk::PluginKey,
    tool_name: &str,
) -> Option<agena::plugin::UiPresentationOverride> {
    if let Ok(key) = agena::plugin::sdk::ToolKey::new(plugin_id.clone(), tool_name.to_owned())
        && let Some(mode) = policy.tools.get(&key).copied()
    {
        return Some(mode);
    }
    None
}

pub(in crate::app) fn ui_tool_source(
    tool_override: Option<agena::plugin::UiPresentationOverride>,
    plugin_override: Option<agena::plugin::UiPresentationOverride>,
    tool_default: Option<PluginTextDisplayMode>,
    plugin_default: Option<PluginTextDisplayMode>,
) -> PluginTextDisplaySource {
    match tool_override {
        Some(agena::plugin::UiPresentationOverride::Detailed)
        | Some(agena::plugin::UiPresentationOverride::Summary) => {
            return PluginTextDisplaySource::ToolOverride;
        }
        _ => {}
    }
    match plugin_override {
        Some(agena::plugin::UiPresentationOverride::Detailed)
        | Some(agena::plugin::UiPresentationOverride::Summary) => {
            return PluginTextDisplaySource::PluginOverride;
        }
        _ => {}
    }
    if tool_default.is_some() {
        PluginTextDisplaySource::ToolDefault
    } else if plugin_default.is_some() {
        PluginTextDisplaySource::PluginDefault
    } else {
        PluginTextDisplaySource::GlobalDefault
    }
}

pub(in crate::app) fn plugin_policy_prompt_path(
    plugin_id: &str,
    tool_name: Option<&str>,
) -> String {
    match tool_name {
        Some(tool_name) => format!(
            "{PLUGIN_TOOL_PRESENTATION_PATH}.tools.{}",
            quote_settings_segment(format!("{plugin_id}/{tool_name}").as_str())
        ),
        None => format!(
            "{PLUGIN_TOOL_PRESENTATION_PATH}.plugins.{}",
            quote_settings_segment(plugin_id)
        ),
    }
}

pub(in crate::app) fn plugin_policy_ui_path(plugin_id: &str, tool_name: Option<&str>) -> String {
    match tool_name {
        Some(tool_name) => format!(
            "{PLUGIN_UI_PRESENTATION_PATH}.tools.{}",
            quote_settings_segment(format!("{plugin_id}/{tool_name}").as_str())
        ),
        None => format!(
            "{PLUGIN_UI_PRESENTATION_PATH}.plugins.{}",
            quote_settings_segment(plugin_id)
        ),
    }
}

pub(in crate::app) fn plugin_policy_item_has_override(item: &PluginPolicyItem) -> bool {
    item.prompt_file_override.is_some() || item.ui_file_override.is_some()
}
use super::{
    BTreeSet, COMPACT_TOOLBAR_ACTIONS, JsonValue, Line, Modifier, PLUGIN_TOOL_PRESENTATION_PATH,
    PLUGIN_UI_PRESENTATION_PATH, PluginConfigDrilldownOverlay, PluginConfigFocus, PluginPolicyItem,
    PluginTextDisplayMode, PluginTextDisplaySource, PluginWorkbenchOverlay, PluginWorkbenchPlugin,
    Span, Style, Text, config_row_cell_label, fixed_columns, override_leaf_count, pad_to_width,
    plugin_get_json_path, plugin_workbench_selection_highlight_style, quote_settings_segment,
    selected_config_row_context, wrap_prefixed_text,
};
