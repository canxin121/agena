//! `agena.web` plugin: web_fetch / web_search.

use crate::message::{WebFetchToolInput, WebSearchToolInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::ToolTag;
use crate::plugins::provided::router::InProcessToolPlugin;
use agena_macros::StaticToolSurface;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value as JsonValue;

pub(crate) const WEB_PLUGIN_ID: &str = "agena.web";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new_with_resolver(
        "agena-web",
        "Web fetch/search command tool backed by the in-process executor bridge.",
        vec![WebToolInput::tool_decl()],
        WebToolInput::resolve_entry,
    )
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "web",
    description = "Web command dispatcher. Set action to fetch or search. Fetch upgrades HTTP to HTTPS and caches for 15 minutes.",
    summary = "Search the web or fetch web pages.",
    help = "Use action `search` for web search and `fetch` to retrieve a URL. Fetch upgrades HTTP URLs to HTTPS where possible and caches successful fetches for 15 minutes. Legacy `command/args` inputs are still accepted for compatibility.",
    tags(ToolTag::ReadOnly, ToolTag::Network, ToolTag::Internet),
    concurrency_safe = true,
    load = "deferred",
    fallback = parse_legacy_web_input
)]
#[serde(tag = "action", rename_all = "snake_case")]
enum WebToolInput {
    #[tool(exec = "web_fetch")]
    Fetch {
        #[serde(flatten)]
        args: WebFetchToolInput,
    },
    #[tool(exec = "web_search")]
    Search {
        #[serde(flatten)]
        args: WebSearchToolInput,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
enum LegacyWebToolInput {
    Fetch(WebFetchToolInput),
    Search(WebSearchToolInput),
}

fn parse_legacy_web_input(
    input: JsonValue,
    primary: PluginError,
) -> crate::plugin::sdk::Result<(String, JsonValue)> {
    match serde_json::from_value::<LegacyWebToolInput>(input) {
        Ok(LegacyWebToolInput::Fetch(args)) => tool_args("web_fetch", args),
        Ok(LegacyWebToolInput::Search(args)) => tool_args("web_search", args),
        Err(_) => Err(primary),
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
