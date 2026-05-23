//! `agena.web` plugin: web_fetch / web_search.

use crate::message::{WebFetchToolInput, WebSearchToolInput};
use crate::plugin::sdk::ToolTag;
use crate::plugins::provided::router::InProcessToolPlugin;
use agena_macros::StaticToolSurface;
use schemars::JsonSchema;
use serde::Deserialize;

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
    help = "Use action `search` for web search and `fetch` to retrieve a URL. Fetch upgrades HTTP URLs to HTTPS where possible and caches successful fetches for 15 minutes.",
    tags(ToolTag::ReadOnly, ToolTag::Network, ToolTag::Internet),
    concurrency_safe = true
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
