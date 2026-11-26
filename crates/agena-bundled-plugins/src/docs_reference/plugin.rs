use std::fmt::Write as _;

use agena_plugin_sdk::{PluginManifest, SettingsContract};
use serde_json::Value;

use super::{cell_text, code_cell, default_cell, nonempty, session_kinds, yes_or_no};

pub(super) fn render_declarations(out: &mut String, id: &str, manifest: &PluginManifest) {
    if let Some(settings) = &manifest.settings {
        render_settings(out, id, settings);
    }
    if !manifest.commands.is_empty() {
        writeln!(out, "### Commands\n").unwrap();
        writeln!(out, "| Command ID | Slash spellings | Sessions | Summary |").unwrap();
        writeln!(out, "| --- | --- | --- | --- |").unwrap();
        for command in &manifest.commands {
            let spellings = command.invocation_names().map(|name| format!("/{name}"));
            let summary = nonempty(command.docs.summary.as_deref())
                .or_else(|| (command.title != command.id).then_some(command.title.as_str()))
                .unwrap_or("");
            writeln!(
                out,
                "| {} | {} | {} | {} |",
                code_cell(&command.id),
                code_list(spellings),
                session_kinds(command.applies_to),
                cell_text(summary)
            )
            .unwrap();
        }
        writeln!(out).unwrap();
    }
    if !manifest.services.is_empty() {
        writeln!(out, "### Services\n").unwrap();
        writeln!(
            out,
            "| Direction | Service ID | API version | Methods or provider |"
        )
        .unwrap();
        writeln!(out, "| --- | --- | ---: | --- |").unwrap();
        for export in &manifest.services.exports {
            writeln!(
                out,
                "| Export | {} | {} | {} |",
                code_cell(&export.id),
                export.api_version,
                code_list(export.methods.iter().map(|method| method.id.as_str()))
            )
            .unwrap();
        }
        for import in &manifest.services.imports {
            let direction = if import.optional {
                "Optional import"
            } else {
                "Required import"
            };
            writeln!(
                out,
                "| {direction} | {} | {} | {} |",
                code_cell(&import.id),
                import.api_version,
                code_list(import.provider.as_deref())
            )
            .unwrap();
        }
        writeln!(out).unwrap();
    }
    if !manifest.hook_handlers.is_empty() {
        writeln!(out, "### Hooks\n").unwrap();
        writeln!(out, "| Event | Handler | Purpose |").unwrap();
        writeln!(out, "| --- | --- | --- |").unwrap();
        for hook in &manifest.hook_handlers {
            writeln!(
                out,
                "| {} | {} | {} |",
                code_cell(&hook.event),
                code_cell(&hook.id),
                cell_text(&hook.summary)
            )
            .unwrap();
        }
        writeln!(out).unwrap();
    }
    if !manifest.skills.is_empty() {
        writeln!(out, "### Skills\n").unwrap();
        for skill in &manifest.skills {
            writeln!(out, "- `{}` — {}", skill.name, skill.description).unwrap();
        }
        writeln!(out).unwrap();
    }
    if let Some(web) = &manifest.surface.web
        && !web.pages.is_empty()
    {
        writeln!(out, "### Web pages\n").unwrap();
        for page in &web.pages {
            writeln!(out, "- {} at `/plugins/{id}/{}`", page.title, page.id).unwrap();
        }
        writeln!(out).unwrap();
    }
}

fn render_settings(out: &mut String, id: &str, settings: &SettingsContract) {
    let contract = serde_json::to_value(settings).expect("settings contract must serialize");
    let root = &contract["root"];
    writeln!(out, "### Settings\n").unwrap();
    writeln!(out, "Configure this plugin under `plugins.list[\"{id}\"].settings`. Required fields apply within their parent object or selected variant.\n").unwrap();
    if let Some(description) = nonempty(root["description"].as_str()) {
        writeln!(out, "{description}\n").unwrap();
    }
    writeln!(out, "| Path | Kind | Required | Default | Description |").unwrap();
    writeln!(out, "| --- | --- | --- | --- | --- |").unwrap();
    if let Some(fields) = root["fields"].as_array().filter(|items| !items.is_empty()) {
        for field in fields {
            render_setting(out, field, "");
        }
    } else {
        render_setting(out, root, "");
    }
    writeln!(out).unwrap();
}

fn render_setting(out: &mut String, node: &Value, condition: &str) {
    let path = nonempty(node["path"].as_str()).unwrap_or("/");
    let kind = node["kind"].as_str().expect("settings kind is serialized");
    let sensitive = node["sensitive"] == true || node["secret"] == true;
    let default = if sensitive {
        "—".into()
    } else {
        default_cell(node.get("default"))
    };
    let mut description = cell_text(
        nonempty(node["description"].as_str())
            .or_else(|| nonempty(node["title"].as_str()))
            .unwrap_or(""),
    );
    if !condition.is_empty() {
        write!(description, "<br>When {condition}").unwrap();
    }
    if let Some(options) = node["options"].as_array().filter(|items| !items.is_empty()) {
        let values = options.iter().map(|option| option["value"].to_string());
        write!(description, "<br>Values {}", code_list(values)).unwrap();
    }
    if node["constraints"]
        .as_object()
        .is_some_and(|limits| !limits.is_empty())
    {
        write!(
            description,
            "<br>Limits {}",
            code_cell(&node["constraints"].to_string())
        )
        .unwrap();
    }
    writeln!(
        out,
        "| {} | {} | {} | {default} | {description} |",
        code_cell(path),
        code_cell(kind),
        yes_or_no(node["required"] == true)
    )
    .unwrap();

    for child in node["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(node.get("item"))
        .chain(node.get("value"))
    {
        render_setting(out, child, condition);
    }
    if let Some(variants) = node["variants"].as_array() {
        let discriminator = node["discriminator"]
            .as_str()
            .expect("variant discriminator");
        for variant in variants {
            let test = code_cell(&format!("{discriminator} = {}", variant["tag"]));
            let condition = if condition.is_empty() {
                test
            } else {
                format!("{condition} and {test}")
            };
            for field in variant["fields"].as_array().into_iter().flatten() {
                render_setting(out, field, &condition);
            }
        }
    }
}

fn code_list(values: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let entries = values
        .into_iter()
        .map(|value| code_cell(value.as_ref()))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        "—".into()
    } else {
        entries.join(", ")
    }
}
