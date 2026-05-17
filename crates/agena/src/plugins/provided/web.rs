//! `agena.web` plugin: web_fetch / web_search.

use crate::message::{WebFetchToolInput, WebSearchToolInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::{PluginToolDecl, ToolTag};
use crate::plugins::provided::router::InProcessToolPlugin;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub(crate) const WEB_PLUGIN_ID: &str = "agena.web";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_resolver(
        "agena-web",
        "Web fetch/search command tool backed by the in-process executor bridge.",
        entries(),
        resolve_entry,
    )
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum WebToolInput {
    Fetch(WebFetchToolInput),
    Search(WebSearchToolInput),
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "web",
            crate::entry::definition::json_schema_for::<WebToolInput>(),
        )
        .description(
            "Web command dispatcher. Set command to fetch or search; pass that command's payload in args. Fetch upgrades HTTP to HTTPS and caches for 15 minutes.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Network, ToolTag::Internet])
        .concurrency_safe(true)
        .deferred_load(),
    ]
}

fn resolve_entry(entry: &str, input: JsonValue) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    if entry != "web" {
        return Err(PluginError::invalid_params(format!(
            "unknown web entry '{entry}'"
        )));
    }
    match serde_json::from_value::<WebToolInput>(input)? {
        WebToolInput::Fetch(args) => tool_args("web_fetch", args),
        WebToolInput::Search(args) => tool_args("web_search", args),
    }
}

fn tool_args<T: serde::Serialize>(
    tool: &str,
    args: T,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    Ok((
        tool.to_string(),
        serde_json::to_value(args).map_err(|err| PluginError::invalid_params(err.to_string()))?,
    ))
}
