use super::super::{
    Line, Modifier, PluginTextDisplayMode, PluginWorkbenchOverlay, PluginWorkbenchPlugin, Span,
    Style, Text, clean, command_argument_count, command_schema_and_value, default_value_for_schema,
    fixed_columns, plugin_package_preview, plugin_text_display_mode_label,
    plugin_text_display_source_label, schema_property_count,
};
use super::append_schema_editor_lines;
use super::diagnostics_text;

pub(crate) fn plugin_header_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    let summary_tools = plugin
        .tool_ui_display_modes
        .values()
        .filter(|mode| **mode == PluginTextDisplayMode::Summary)
        .count();
    let detailed_tools = plugin
        .tool_ui_display_modes
        .len()
        .saturating_sub(summary_tools);
    Text::from(vec![
        Line::from(format!(
            "{}        {}        v{}        {}",
            clean(plugin.plugin_id.as_str()),
            clean(plugin.visible_tool.as_str()),
            clean(plugin.version.as_str()),
            clean(plugin.transport.as_str())
        )),
        Line::from(format!(
            "Tools: {}        Commands: {}        Config: {}",
            plugin.tools.len(),
            plugin.commands.len(),
            clean(plugin.config_status.label.as_str())
        )),
        Line::from(format!(
            "UI: {} via {}        detailed tools: {}        summary tools: {}",
            plugin_text_display_mode_label(plugin.ui_display_mode),
            plugin_text_display_source_label(plugin.ui_display_source),
            detailed_tools,
            summary_tools,
        )),
        Line::from(format!(
            "Package: {}",
            plugin
                .configured_plugin_value
                .as_ref()
                .and_then(|configured_plugin| configured_plugin.get("package"))
                .map(plugin_package_preview)
                .unwrap_or_else(|| "unavailable".to_owned())
        )),
    ])
}

pub(crate) fn plugin_tools_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    if plugin.tools.is_empty() {
        return Text::from("No tools.");
    }
    let summary_tools = plugin
        .tool_ui_display_modes
        .values()
        .filter(|mode| **mode == PluginTextDisplayMode::Summary)
        .count();
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "Effective UI mode: {} via {}. {} of {} tools currently render in summary mode.",
            plugin_text_display_mode_label(plugin.ui_display_mode),
            plugin_text_display_source_label(plugin.ui_display_source),
            summary_tools,
            plugin.tools.len(),
        ),
        Style::default().fg(agena_tui_components::theme::muted_color()),
    ))];
    lines.push(Line::from(
        "Up/Down selects a tool. Enter opens the host-owned Schema form; Ctrl+S validates and runs it.",
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        fixed_columns(
            &[
                ("Tool", 24),
                ("UI", 10),
                ("Source", 16),
                ("Default", 12),
                ("Description", 54),
                ("Inputs", 8),
            ],
            124,
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (index, tool) in plugin.tools.iter().enumerate() {
        let inputs = schema_property_count(&tool.contract.input_schema);
        let mode = plugin
            .tool_ui_display_modes
            .get(tool.name.as_str())
            .copied()
            .unwrap_or(plugin.ui_display_mode);
        let ui_label = plugin_text_display_mode_label(mode);
        let default_label = plugin
            .tool_ui_display_defaults
            .get(&tool.name)
            .copied()
            .map(plugin_text_display_mode_label)
            .unwrap_or("global");
        let source_label = plugin
            .tool_ui_display_sources
            .get(&tool.name)
            .copied()
            .map(plugin_text_display_source_label)
            .unwrap_or("sdk-fallback");
        let description = match mode {
            PluginTextDisplayMode::Detailed => tool
                .docs
                .help
                .as_deref()
                .or(tool.docs.summary.as_deref())
                .unwrap_or(""),
            PluginTextDisplayMode::Summary => tool
                .docs
                .summary
                .as_deref()
                .or(tool.docs.help.as_deref())
                .unwrap_or(""),
        };
        let marker = if index == dialog.selected_tool {
            ">> "
        } else {
            "   "
        };
        let line = format!(
            "{marker}{}",
            fixed_columns(
                &[
                    (tool.name.as_str(), 24),
                    (ui_label, 10),
                    (source_label, 16),
                    (default_label, 12),
                    (description, 54),
                    (inputs.to_string().as_str(), 8),
                ],
                121,
            )
        );
        let style = if index == dialog.selected_tool {
            super::super::plugin_workbench_selection_highlight_style()
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(line, style)));
    }
    if let Some(tool) = plugin.tools.get(dialog.selected_tool) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Input preview: {}", tool.name),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(format!(
            "Tags: {}",
            if tool.tags.is_empty() {
                "none declared".to_owned()
            } else {
                tool.tags
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )));
        let default =
            default_value_for_schema(&tool.contract.input_schema, &tool.contract.input_schema);
        append_schema_editor_lines(
            &mut lines,
            Some(&tool.contract.input_schema),
            Some(&tool.contract.input_schema),
            &default,
            "Arguments",
            0,
            124,
            18,
        );
        if let Some(result) = dialog
            .tool_result
            .as_ref()
            .filter(|result| result.plugin_id == plugin.plugin_id && result.tool_name == tool.name)
        {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                if result.succeeded {
                    format!("Last result · {} · success", result.tool_name)
                } else {
                    format!("Last result · {} · failed", result.tool_name)
                },
                Style::default()
                    .fg(if result.succeeded {
                        agena_tui_components::theme::success_color()
                    } else {
                        agena_tui_components::theme::danger_color()
                    })
                    .add_modifier(Modifier::BOLD),
            )));
            for line in result.output.lines() {
                lines.push(Line::from(clean(line)));
            }
        }
    }
    Text::from(lines)
}

pub(crate) fn plugin_commands_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    if plugin.commands.is_empty() {
        return Text::from("No commands.");
    }
    let mut lines = vec![Line::from(Span::styled(
        fixed_columns(
            &[
                ("Command", 30),
                ("Description", 64),
                ("Args", 8),
                ("Category", 18),
            ],
            124,
        ),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for command in &plugin.commands {
        let args = command_argument_count(plugin, command);
        lines.push(Line::from(fixed_columns(
            &[
                (command.title.as_str(), 30),
                (command.description.as_str(), 64),
                (args.to_string().as_str(), 8),
                (command.category.as_str(), 18),
            ],
            124,
        )));
    }
    if let Some(command) = plugin.commands.first() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Arguments: {}", command.title),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        match command_schema_and_value(plugin, command) {
            Some((schema, value)) => append_schema_editor_lines(
                &mut lines,
                Some(&schema),
                Some(&schema),
                &value,
                "Arguments",
                0,
                124,
                18,
            ),
            None => lines.push(Line::from("No structured arguments.")),
        }
    }
    Text::from(lines)
}

pub(crate) fn plugin_capabilities_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    let mut lines = Vec::new();
    if let Some(authority) = plugin
        .inspect
        .as_ref()
        .and_then(|inspect| inspect.authority.as_ref())
    {
        lines.push(Line::from(format!(
            "Trust level: {}",
            authority.trust_level
        )));
        if !authority.provenance.is_empty() {
            lines.push(Line::from(format!(
                "Provenance: {}",
                clean(authority.provenance.join(", "))
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Plugin capabilities",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if authority.plugin_capabilities.is_empty() {
            lines.push(Line::from("  none"));
        } else {
            for capability in &authority.plugin_capabilities {
                lines.push(Line::from(format!("  {}", clean(capability))));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Tool capabilities",
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if authority.tool_capabilities.is_empty() {
            lines.push(Line::from("  none"));
        } else {
            for (tool_name, capabilities) in &authority.tool_capabilities {
                lines.push(Line::from(format!(
                    "  {}: {}",
                    clean(tool_name),
                    clean(capabilities.join(", "))
                )));
            }
        }
    } else {
        lines.push(Line::from("Authority data unavailable."));
    }
    Text::from(lines)
}

pub(crate) fn plugin_logs_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    if plugin.logs.is_empty() {
        return Text::from("No logs.");
    }
    Text::from(
        plugin
            .logs
            .iter()
            .map(|log_record| {
                Line::from(format!(
                    "#{} {} {}",
                    log_record.seq,
                    log_record.level,
                    clean(log_record.message.as_str())
                ))
            })
            .collect::<Vec<_>>(),
    )
}

pub(crate) fn plugin_diagnostics_text(plugin: &PluginWorkbenchPlugin) -> Text<'static> {
    let mut diagnostics = plugin.diagnostics.clone();
    diagnostics.extend(plugin.runtime_diagnostics.clone());
    diagnostics_text(diagnostics.as_slice(), false, 0)
}
