//! Markdown reference generator for the bundled tool catalog.
//!
//! [`bundled_tools_markdown_reference`] renders a deterministic, human-readable
//! reference from the real plugin manifests. The output is committed at
//! `docs/generated/tools-reference.md` and embedded into rustdoc through
//! `include_str!`, so `cargo doc` shows every tool definition together with its
//! detailed help text, examples, tags, runtime flags, and JSON Schema
//! contracts.
//!
//! Regenerate with `agena inspect --tools-reference` (see the generated file's
//! header for the exact command); a CI drift test compares the committed file
//! against this generator.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use agena_plugin_host::sdk::{PluginKey, PluginManifest, ToolDefinition};
use serde_json::Value;

use crate::capability_manifest::bundled_plugin_manifests;

/// Render the complete Markdown reference for every bundled tool.
///
/// Plugins are sorted by id, tools by name, so the output is stable across
/// runs and only changes when an actual tool definition changes.
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
    writeln!(out, "# Agena Bundled Tools Reference").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "> Generated file — do not edit by hand. Regenerate with:\n>\n> ```bash\n> agena inspect --tools-reference > docs/generated/tools-reference.md\n> ```"
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "This document is deterministically generated from the real `agena-bundled-plugins` plugin manifests, covering **{plugin_count} plugins and {tool_count} tool definitions**."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "- Each tool entry includes: name, summary, detailed help (`before_help` / `help` / `after_help`), tags, concurrency / streaming / strict runtime flags, examples, an input parameter table, and the full input / output JSON Schema.").unwrap();
    writeln!(out, "- The `list` / `search` / `help` / `tags` / `call` tools of `agena.tools` are the stable Tool API gateway handlers; all other tools are ordinary execution tools.").unwrap();
    writeln!(out, "- Tool names (`plugin.tool`, full key `agena.<plugin>.<tool>`) appear only in `tools_help.tool` / `tools_call.tool`; they never become Provider function names.").unwrap();
    writeln!(out).unwrap();
}

fn render_toc(out: &mut String, plugins: &[(String, PluginManifest, Option<String>)]) {
    writeln!(out, "## Table of Contents").unwrap();
    writeln!(out).unwrap();
    for (id, manifest, _) in plugins {
        let summary = nonempty(manifest.summary.as_deref()).unwrap_or("");
        writeln!(
            out,
            "- [`{id}`](#{}) — {summary} ({} tools)",
            heading_anchor(id),
            manifest.tools.len()
        )
        .unwrap();
    }
    writeln!(out).unwrap();
}

fn render_plugin(out: &mut String, id: &str, manifest: &PluginManifest, conditional: Option<&str>) {
    writeln!(out, "## {id}").unwrap();
    writeln!(out).unwrap();

    let mut meta = format!(
        "**Version** `{}` · **Tools** {}",
        manifest.version,
        manifest.tools.len()
    );
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
    for tool in tools {
        render_tool(out, id, tool);
    }
}

fn render_tool(out: &mut String, plugin_id: &str, tool: &ToolDefinition) {
    let canonical = format!("{plugin_id}.{}", tool.name);
    writeln!(out, "### {}", tool.name).unwrap();
    writeln!(out).unwrap();

    let mut intro = vec![format!("`{canonical}`")];
    if is_gateway_tool(plugin_id, &tool.name) {
        intro.push("**Tool API gateway handler**".to_string());
    }
    if let Some(summary) = nonempty(tool.summary_text()) {
        intro.push(format!("**Summary**: {summary}"));
    }
    writeln!(out, "{}", intro.join(" · ")).unwrap();

    let tags = tool.effective_tags();
    if !tags.is_empty() {
        let rendered = tags
            .iter()
            .map(|tag| format!("`{tag}`"))
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(out).unwrap();
        writeln!(out, "**Tags**: {rendered}").unwrap();
    }

    writeln!(out).unwrap();
    let concurrency = if tool.runtime.concurrency_safe {
        "✓ concurrency-safe"
    } else {
        "✗ not concurrency-safe"
    };
    let streaming = serde_json::to_string(&tool.runtime.streaming)
        .unwrap_or_else(|_| "\"buffered\"".to_string())
        .trim_matches('"')
        .to_string();
    let strict = if tool.contract.strict {
        "strict"
    } else {
        "non-strict"
    };
    writeln!(
        out,
        "**Runtime**: {concurrency} · streaming `{streaming}` · {strict}"
    )
    .unwrap();

    if let Some(before) = nonempty(tool.before_help_text()) {
        writeln!(out).unwrap();
        writeln!(out, "**Before help**:").unwrap();
        writeln!(out, "{}", blockquote(before)).unwrap();
    }
    if let Some(help) = nonempty(tool.help_text()) {
        writeln!(out).unwrap();
        writeln!(out, "**Help**:").unwrap();
        writeln!(out, "{}", blockquote(help)).unwrap();
    }
    if let Some(after) = nonempty(tool.after_help_text()) {
        writeln!(out).unwrap();
        writeln!(out, "**After help**:").unwrap();
        writeln!(out, "{}", blockquote(after)).unwrap();
    }

    let examples = tool.example_texts();
    if !examples.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "**Examples**:").unwrap();
        for example in examples {
            match serde_json::from_str::<Value>(example) {
                Ok(value) => {
                    writeln!(out, "```json").unwrap();
                    writeln!(out, "{}", pretty_json(&value)).unwrap();
                    writeln!(out, "```").unwrap();
                }
                Err(_) => {
                    writeln!(out, "```text").unwrap();
                    writeln!(out, "{example}").unwrap();
                    writeln!(out, "```").unwrap();
                }
            }
        }
    }

    let input_schema = tool.input_schema();
    let rows = schema_parameter_rows(&input_schema);
    if !rows.is_empty() {
        writeln!(out).unwrap();
        writeln!(out, "**Input parameters**:").unwrap();
        writeln!(
            out,
            "| Parameter | Type | Required | Default | Description |"
        )
        .unwrap();
        writeln!(out, "| --- | --- | --- | --- | --- |").unwrap();
        for (name, ty, required, default, desc) in rows {
            writeln!(
                out,
                "| `{name}` | `{ty}` | {} | {} | {} |",
                if required { "✓" } else { "—" },
                default,
                cell_text(&desc)
            )
            .unwrap();
        }
    }

    writeln!(out).unwrap();
    writeln!(out, "**Input schema**:").unwrap();
    writeln!(out, "```json").unwrap();
    writeln!(out, "{}", pretty_json(&input_schema)).unwrap();
    writeln!(out, "```").unwrap();

    let output_schema = tool.output_schema();
    if !output_schema.is_null() {
        writeln!(out).unwrap();
        writeln!(out, "**Output schema**:").unwrap();
        writeln!(out, "```json").unwrap();
        writeln!(out, "{}", pretty_json(&output_schema)).unwrap();
        writeln!(out, "```").unwrap();
    }
    writeln!(out).unwrap();
}

fn plugin_id(manifest: &PluginManifest) -> String {
    PluginKey::new(manifest.namespace.clone(), manifest.name.clone())
        .expect("bundled plugin manifests always carry valid keys")
        .to_string()
}

/// Match rustdoc's markdown heading id generation for the plugin headings
/// (`## agena.chatgpt` renders as `id="agenachatgpt"`): keep ASCII
/// alphanumerics plus `-` / `_`, drop everything else, lowercase.
fn heading_anchor(id: &str) -> String {
    id.chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn is_gateway_tool(plugin_id: &str, tool_name: &str) -> bool {
    plugin_id == "agena.tools" && matches!(tool_name, "list" | "search" | "help" | "tags" | "call")
}

fn schema_parameter_rows(schema: &Value) -> Vec<(String, String, bool, String, String)> {
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let mut rows = Vec::new();
    for (name, property) in properties {
        let ty = schema_type(property);
        let is_required = required.contains(name.as_str());
        let default = property
            .get("default")
            .map(|value| format!("`{}`", compact_json(value)))
            .unwrap_or_else(|| "—".to_string());
        let desc = property
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        rows.push((name.clone(), ty, is_required, default, desc));
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    rows
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
            } else if let Some(any_of) = schema.get("anyOf").and_then(Value::as_array) {
                any_of
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

fn compact_json(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn cell_text(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', "<br>")
}

fn blockquote(text: &str) -> String {
    text.lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}
