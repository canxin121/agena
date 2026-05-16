//! `agena.web` plugin: web_fetch / web_search.

use crate::message::{WebFetchToolInput, WebSearchToolInput};
use crate::plugin::sdk::{InputNetworkSpec, NetworkAccessSpec, PluginToolDecl, ToolTag};
use crate::plugins::provided::router::InProcessToolPlugin;

pub(crate) const WEB_PLUGIN_ID: &str = "agena.web";

pub(crate) fn new_plugin() -> InProcessToolPlugin {
    InProcessToolPlugin::new(
        "agena-web",
        "Web tools (web_fetch, web_search) backed by the in-process executor bridge.",
        entries(),
    )
}

fn entries() -> Vec<PluginToolDecl> {
    vec![
        PluginToolDecl::new(
            "web_fetch",
            crate::entry::definition::json_schema_for::<WebFetchToolInput>(),
        )
        .description(
            "Fetch a URL and return its content as Markdown. HTTP is upgraded to HTTPS; cached for 15 minutes.",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Network, ToolTag::Internet])
        .input_network(InputNetworkSpec {
            jsonpath: "$.url".to_string(),
            optional: false,
        })
        .concurrency_safe(true)
        .deferred_load(),
        PluginToolDecl::new(
            "web_search",
            crate::entry::definition::json_schema_for::<WebSearchToolInput>(),
        )
        .description(
            "Search the web. Backend selectable in config (tavily, exa, brave, or duckduckgo_html as zero-config default).",
        )
        .tags([ToolTag::ReadOnly, ToolTag::Network, ToolTag::Internet])
        .network_access(NetworkAccessSpec {
            target: "https://html.duckduckgo.com/html/".to_string(),
        })
        .concurrency_safe(true)
        .deferred_load(),
    ]
}
