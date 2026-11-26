//! Markdown reference generator for the bundled plugin and tool catalog.
//!
//! [`bundled_tools_markdown_reference`] renders a deterministic, human-readable
//! reference from the compiled plugin manifests. The build script uses its
//! anchors to produce the index and detail pages under [`crate::tools_docs`].

use std::fmt::Write as _;

use agena_domain::{ApplicableSessions, SessionKind};
use agena_plugin_sdk::{PluginKey, PluginManifest, ToolDefinition, ToolStreamingMode};
use serde_json::Value;

use crate::capability_manifest::bundled_plugin_manifests;

mod plugin;

/// Render the Markdown reference for the bundled plugin and tool catalog.
///
/// Plugins are sorted by ID and tools by name. The reference includes the
/// settings, commands, and services declared by plugins with zero tools.
pub fn bundled_tools_markdown_reference() -> String {
    let mut plugins: Vec<(String, PluginManifest, Option<String>)> = bundled_plugin_manifests()
        .into_iter()
        .map(|(manifest, conditional)| (plugin_id(&manifest), manifest, conditional))
        .collect();
    plugins.sort_by(|left, right| left.0.cmp(&right.0));

    let plugin_count = plugins.len();
    let tool_count: usize = plugins
        .iter()
        .map(|(_, manifest, _)| manifest.tools.len())
        .sum();

    let mut out = String::new();
    render_header(&mut out, plugin_count, tool_count);
    render_toc(&mut out, &plugins);
    for (id, manifest, conditional) in &plugins {
        render_plugin(&mut out, id, manifest, conditional.as_deref());
    }
    out
}

fn render_header(out: &mut String, plugin_count: usize, tool_count: usize) {
    writeln!(out, "# Agena Bundled Plugins and Tools").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "> Generated from the bundled plugin manifests. Regenerate with the `generate_tools_reference` Cargo example or `agena inspect --tools-reference`."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "The compiled catalog contains **{plugin_count} plugins and {tool_count} execution tools**. Open a plugin to browse its declarations and tools. Each tool lists its canonical ID, applicable session types, and declared input and output contracts."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "For a running instance, use `agena plugin status` and `agena plugin inspect <plugin-id>`. Within a session, `plugins_list` and `tools_list` discover its live catalog. Pass the tool ID to `tools_help` before using `tools_call`. Output schemas describe the business payload. Some tools also return text, metadata, or attachments through the tool protocol.").unwrap();
    writeln!(out).unwrap();
}

fn render_toc(out: &mut String, plugins: &[(String, PluginManifest, Option<String>)]) {
    writeln!(out, "## Plugin index").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "| Plugin | Name | Tools | Commands | Summary |").unwrap();
    writeln!(out, "| --- | --- | ---: | ---: | --- |").unwrap();
    for (id, manifest, _) in plugins {
        let name = cell_text(nonempty(manifest.display_name.as_deref()).unwrap_or(""));
        let summary = cell_text(nonempty(manifest.summary.as_deref()).unwrap_or(""));
        writeln!(
            out,
            "| [`{id}`](#plugin-{id}) | {name} | {} | {} | {summary} |",
            manifest.tools.len(),
            manifest.commands.len()
        )
        .unwrap();
    }
    writeln!(out).unwrap();
}

fn render_plugin(out: &mut String, id: &str, manifest: &PluginManifest, conditional: Option<&str>) {
    writeln!(out, "<a id=\"plugin-{id}\"></a>").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "## {id}").unwrap();
    writeln!(out).unwrap();

    if let Some(name) = nonempty(manifest.display_name.as_deref()) {
        writeln!(out, "**{name}**\n").unwrap();
    }
    let mut meta = format!(
        "**Version** `{}` · **Tools** {}",
        manifest.version,
        manifest.tools.len()
    );
    if !manifest.authors.is_empty() {
        write!(meta, " · **Authors** {}", manifest.authors.join(", ")).unwrap();
    }
    if !manifest.tags.is_empty() {
        let tags = manifest
            .tags
            .iter()
            .map(|tag| format!("`{tag}`"))
            .collect::<Vec<_>>()
            .join(" ");
        meta.push_str(&format!(" · **Tags** {tags}"));
    }
    if let Some(condition) = conditional {
        meta.push_str(&format!(" · **Condition** `{condition}`"));
    }
    writeln!(out, "{meta}").unwrap();
    writeln!(out).unwrap();

    if let Some(summary) = nonempty(manifest.summary.as_deref()) {
        writeln!(out, "{summary}").unwrap();
        writeln!(out).unwrap();
    }
    if let Some(help) = nonempty(manifest.help.as_deref()) {
        writeln!(out, "{}", blockquote(help)).unwrap();
        writeln!(out).unwrap();
    }

    let mut tools = manifest.tools.iter().collect::<Vec<_>>();
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    if !tools.is_empty() {
        writeln!(out, "### Tool index\n").unwrap();
        writeln!(out, "| Tool | Sessions | Summary |").unwrap();
        writeln!(out, "| --- | --- | --- |").unwrap();
        for tool in &tools {
            let name = &tool.name;
            let summary = cell_text(nonempty(tool.summary_text()).unwrap_or(""));
            let sessions = session_kinds(tool.applies_to);
            writeln!(
                out,
                "| [`{name}`](#tool-{id}.{name}) | {sessions} | {summary} |"
            )
            .unwrap();
        }
        writeln!(out).unwrap();
    }
    plugin::render_declarations(out, id, manifest);
    for tool in tools {
        render_tool(out, id, tool);
    }
}

fn render_tool(out: &mut String, plugin_id: &str, tool: &ToolDefinition) {
    let canonical = format!("{plugin_id}.{}", tool.name);
    writeln!(out, "<a id=\"tool-{canonical}\"></a>").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "### {}", tool.name).unwrap();
    writeln!(out).unwrap();

    writeln!(out, "**Tool ID** `{canonical}`").unwrap();
    if let Some(summary) = nonempty(tool.summary_text()) {
        writeln!(out, "\n{summary}").unwrap();
    }

    let tags = tool.effective_tags();
    if !tags.is_empty() {
        let rendered = tags
            .iter()
            .map(|tag| format!("`{tag}`"))
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(out).unwrap();
        writeln!(out, "**Tags** {rendered}").unwrap();
    }

    writeln!(out).unwrap();
    let streaming = match tool.runtime.streaming {
        ToolStreamingMode::Buffered => "buffered",
        ToolStreamingMode::Streaming => "streaming",
    };
    let sessions = session_kinds(tool.applies_to);
    writeln!(
        out,
        "**Output delivery** `{streaming}` · **Sessions** {sessions}"
    )
    .unwrap();

    for (heading, text) in [
        ("Before calling", tool.before_help_text()),
        ("Usage", tool.help_text()),
        ("After calling", tool.after_help_text()),
    ] {
        if let Some(text) = nonempty(text) {
            writeln!(out, "\n#### {heading}\n\n{}", blockquote(text)).unwrap();
        }
    }

    let input_schema = tool.input_schema();
    render_property_table(out, "Input parameters", &input_schema);
    render_json_details(out, "Input JSON Schema", &input_schema);

    let output_schema = tool.output_schema();
    if !output_schema.is_null() {
        render_property_table(out, "Output payload fields", &output_schema);
        render_json_details(out, "Output JSON Schema", &output_schema);
    }
    writeln!(out).unwrap();
}

fn plugin_id(manifest: &PluginManifest) -> String {
    PluginKey::new(manifest.namespace.clone(), manifest.name.clone())
        .expect("bundled plugin manifests always carry valid keys")
        .to_string()
}

fn session_kinds(sessions: ApplicableSessions) -> &'static str {
    if sessions.is_all() {
        "Project, Bot"
    } else if sessions.contains(SessionKind::Project) {
        "Project"
    } else if sessions.contains(SessionKind::Bot) {
        "Bot"
    } else {
        "—"
    }
}

fn render_property_table(out: &mut String, heading: &str, schema: &Value) {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    if properties.is_empty() {
        return;
    }

    let mut fields = properties.iter().collect::<Vec<_>>();
    fields.sort_by_key(|(name, schema)| {
        let order = schema.get("x-agena-order").and_then(Value::as_str);
        (order.is_none(), order, name.as_str())
    });
    let required = schema["required"].as_array();
    writeln!(out, "\n#### {heading}\n").unwrap();
    writeln!(out, "| Field | Type | Required | Default | Description |").unwrap();
    writeln!(out, "| --- | --- | --- | --- | --- |").unwrap();
    for (name, property) in fields {
        let reference = property["$ref"]
            .as_str()
            .and_then(|reference| reference.strip_prefix('#'))
            .and_then(|pointer| schema.pointer(pointer));
        let description = nonempty(property["description"].as_str())
            .or_else(|| {
                reference.and_then(|definition| nonempty(definition["description"].as_str()))
            })
            .unwrap_or("");
        let required = required.is_some_and(|fields| {
            fields
                .iter()
                .any(|field| field.as_str() == Some(name.as_str()))
        });
        writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            code_cell(name),
            code_cell(&schema_type(property)),
            yes_or_no(required),
            default_cell(property.get("default")),
            cell_text(description)
        )
        .unwrap();
    }
}

fn schema_type(schema: &Value) -> String {
    if let Some(items) = schema.get("items") {
        return format!("array<{}>", schema_type(items));
    }
    match schema.get("type") {
        Some(Value::String(kind)) => kind.clone(),
        Some(Value::Array(kinds)) => kinds
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" / "),
        _ => {
            if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
                reference
                    .rsplit('/')
                    .next()
                    .unwrap_or(reference)
                    .to_string()
            } else if let Some(variants) = schema["anyOf"].as_array().or(schema["oneOf"].as_array())
            {
                variants
                    .iter()
                    .map(schema_type)
                    .collect::<Vec<_>>()
                    .join(" / ")
            } else if schema.get("enum").is_some() {
                "enum".to_string()
            } else if schema.get("const").is_some() {
                "const".to_string()
            } else {
                "any".to_string()
            }
        }
    }
}

fn default_cell(value: Option<&Value>) -> String {
    value.map_or_else(|| "—".into(), |value| code_cell(&value.to_string()))
}

fn render_json_details(out: &mut String, label: &str, value: &Value) {
    let json = serde_json::to_string_pretty(value).expect("JSON value must serialize");
    writeln!(
        out,
        "\n<details>\n<summary>{label}</summary>\n\n```json\n{json}\n```\n\n</details>"
    )
    .unwrap();
}

fn code_cell(text: &str) -> String {
    let backticks = text.split(|ch| ch != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(backticks + 1);
    cell_text(&format!("{fence}{text}{fence}"))
}

fn yes_or_no(value: bool) -> &'static str {
    if value { "Yes" } else { "No" }
}

fn cell_text(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', "<br>")
}

fn blockquote(text: &str) -> String {
    text.lines()
        .map(|line| {
            let line = line.trim_end();
            if line.is_empty() {
                ">".into()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundled_capability_identity_snapshot_json;

    #[test]
    fn generated_plugin_references_match_the_compiled_manifests() {
        assert!(
            bundled_tools_markdown_reference()
                .lines()
                .eq(include_str!("../generated/tools-reference.md").lines()),
            "tools-reference.md is out of date. Run the generate_tools_reference example."
        );
        assert!(
            bundled_capability_identity_snapshot_json()
                .lines()
                .eq(include_str!("../generated/bundled-capability-identities.json").lines()),
            "bundled-capability-identities.json is out of date. Run the generate_capability_identity_snapshot example."
        );
    }
}
