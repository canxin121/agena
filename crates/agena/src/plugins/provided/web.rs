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
    description = "Lightweight web command dispatcher. Use it for single-page fetches or Brave-backed web search. Prefer the `crawl` tool for multi-page crawling, local indexing, and repeated retrieval.",
    summary = "Search the web or fetch one page.",
    help = "Use action `search` for web search and `fetch` to retrieve a single URL. Fetch upgrades HTTP URLs to HTTPS where possible and caches successful fetches for 15 minutes. For multi-page crawling or local search over fetched pages, prefer the `crawl` tool.",
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
