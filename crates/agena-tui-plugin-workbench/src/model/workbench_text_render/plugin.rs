use super::super::{
    Line, Modifier, PluginWorkbenchOverlay, PluginWorkbenchPlugin, Span, Style, Text, clean,
    default_value_for_schema, fixed_columns, operation_argument_count, operation_schema_and_value,
    plugin_package_preview, schema_property_count,
};
use super::append_schema_editor_lines;
use super::diagnostics_text;

pub(crate) fn plugin_header_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let package = plugin
        .configured_plugin_value
        .as_ref()
        .and_then(|configured_plugin| configured_plugin.get("package"))
        .map(plugin_package_preview)
        .unwrap_or_else(|| dialog.i18n.text("plugin-workbench-unavailable"));
    Text::from(vec![
        Line::from(format!(
            "{}        {}        v{}        {}",
            clean(plugin.plugin_id.as_str()),
            clean(plugin.visible_tool.as_str()),
            clean(plugin.version.as_str()),
            clean(plugin.transport.as_str())
        )),
        Line::from(dialog.i18n.text_args(
            "plugin-workbench-header-summary",
            &agena_tui::fl_args![
                "tools" => plugin.tools.len(),
                "operations" => plugin.operations.len(),
                "config" => plugin.config_status.kind.label(&dialog.i18n),
            ],
        )),
        Line::from(dialog.i18n.text_args(
            "plugin-workbench-package-summary",
            &agena_tui::fl_args!["package" => package],
        )),
    ])
}

pub(crate) fn plugin_tools_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    if plugin.tools.is_empty() {
        return Text::from(dialog.i18n.text("plugin-workbench-no-tools"));
    }
    let mut lines = vec![Line::from(dialog.i18n.text("plugin-workbench-tools-help"))];
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        fixed_columns(
            &[
                (
                    dialog.i18n.text("plugin-workbench-column-tool").as_str(),
                    24,
                ),
                (
                    dialog
                        .i18n
                        .text("plugin-workbench-column-description")
                        .as_str(),
                    54,
                ),
                (
                    dialog.i18n.text("plugin-workbench-column-inputs").as_str(),
                    8,
                ),
            ],
            124,
        ),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (index, tool) in plugin.tools.iter().enumerate() {
        let inputs = schema_property_count(&tool.contract.input_schema);
        let description = tool
            .docs
            .help
            .as_deref()
            .or(tool.docs.summary.as_deref())
            .unwrap_or("");
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
            dialog.i18n.text_args(
                "plugin-workbench-input-preview",
                &agena_tui::fl_args!["tool" => tool.name.clone()],
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        let tags = if tool.tags.is_empty() {
            dialog.i18n.text("plugin-workbench-none-declared")
        } else {
            tool.tags
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        lines.push(Line::from(dialog.i18n.text_args(
            "plugin-workbench-tags-summary",
            &agena_tui::fl_args!["tags" => tags],
        )));
        let default =
            default_value_for_schema(&tool.contract.input_schema, &tool.contract.input_schema);
        append_schema_editor_lines(
            &mut lines,
            &dialog.i18n,
            Some(&tool.contract.input_schema),
            Some(&tool.contract.input_schema),
            &default,
            dialog
                .i18n
                .text("plugin-workbench-column-arguments")
                .as_str(),
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
                dialog.i18n.text_args(
                    if result.succeeded {
                        "plugin-workbench-last-result-success"
                    } else {
                        "plugin-workbench-last-result-failed"
                    },
                    &agena_tui::fl_args!["tool" => result.tool_name.clone()],
                ),
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

pub(crate) fn plugin_operations_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    if plugin.operations.is_empty() {
        return Text::from(dialog.i18n.text("plugin-workbench-no-operations"));
    }
    let mut lines = vec![Line::from(Span::styled(
        fixed_columns(
            &[
                (
                    dialog
                        .i18n
                        .text("plugin-workbench-column-operation")
                        .as_str(),
                    30,
                ),
                (
                    dialog
                        .i18n
                        .text("plugin-workbench-column-description")
                        .as_str(),
                    64,
                ),
                (dialog.i18n.text("plugin-workbench-column-args").as_str(), 8),
                (
                    dialog
                        .i18n
                        .text("plugin-workbench-column-category")
                        .as_str(),
                    18,
                ),
            ],
            124,
        ),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for operation in &plugin.operations {
        let args = operation_argument_count(plugin, operation);
        lines.push(Line::from(fixed_columns(
            &[
                (operation.title.as_str(), 30),
                (operation.description.as_str(), 64),
                (args.to_string().as_str(), 8),
                (
                    operation
                        .category
                        .as_deref()
                        .unwrap_or(operation.group.as_str()),
                    18,
                ),
            ],
            124,
        )));
    }
    if let Some(operation) = plugin.operations.first() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            dialog.i18n.text_args(
                "plugin-workbench-operation-arguments",
                &agena_tui::fl_args!["operation" => operation.title.clone()],
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        match operation_schema_and_value(plugin, operation) {
            Some((schema, value)) => append_schema_editor_lines(
                &mut lines,
                &dialog.i18n,
                Some(&schema),
                Some(&schema),
                &value,
                dialog
                    .i18n
                    .text("plugin-workbench-column-arguments")
                    .as_str(),
                0,
                124,
                18,
            ),
            None => lines.push(Line::from(
                dialog.i18n.text("plugin-workbench-no-structured-arguments"),
            )),
        }
    }
    Text::from(lines)
}

pub(crate) fn plugin_capabilities_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let mut lines = Vec::new();
    if let Some(authority) = plugin
        .inspect
        .as_ref()
        .and_then(|inspect| inspect.authority.as_ref())
    {
        lines.push(Line::from(dialog.i18n.text_args(
            "plugin-workbench-trust-level",
            &agena_tui::fl_args!["level" => authority.trust_level.clone()],
        )));
        if !authority.provenance.is_empty() {
            lines.push(Line::from(dialog.i18n.text_args(
                "plugin-workbench-provenance",
                &agena_tui::fl_args![
                    "provenance" => clean(authority.provenance.join(", ")),
                ],
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            dialog.i18n.text("plugin-workbench-plugin-capabilities"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if authority.plugin_capabilities.is_empty() {
            lines.push(Line::from(format!(
                "  {}",
                dialog.i18n.text("plugin-workbench-none")
            )));
        } else {
            for capability in &authority.plugin_capabilities {
                lines.push(Line::from(format!("  {}", clean(capability))));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            dialog.i18n.text("plugin-workbench-tool-capabilities"),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        if authority.tool_capabilities.is_empty() {
            lines.push(Line::from(format!(
                "  {}",
                dialog.i18n.text("plugin-workbench-none")
            )));
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
        lines.push(Line::from(
            dialog.i18n.text("plugin-workbench-authority-unavailable"),
        ));
    }
    Text::from(lines)
}

pub(crate) fn plugin_logs_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    if plugin.logs.is_empty() {
        return Text::from(dialog.i18n.text("plugin-workbench-no-logs"));
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

pub(crate) fn plugin_diagnostics_text(
    dialog: &PluginWorkbenchOverlay,
    plugin: &PluginWorkbenchPlugin,
) -> Text<'static> {
    let mut diagnostics = plugin.diagnostics.clone();
    diagnostics.extend(plugin.runtime_diagnostics.clone());
    diagnostics_text(dialog, diagnostics.as_slice(), false, 0)
}
