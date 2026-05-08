//! First-party `agena.web` plugin: web_fetch / web_search.

use crate::message::{WebFetchToolInput, WebSearchToolInput};
use crate::plugin::sdk::{EntryBehavior as SdkEntryBehavior, PlanModePolicy, PluginEntryDecl};
use crate::plugins::bundled::router::FirstPartyRouterPlugin;

pub(crate) const WEB_PLUGIN_ID: &str = "agena.web";

pub(crate) fn new_plugin() -> FirstPartyRouterPlugin {
    FirstPartyRouterPlugin::new(
        "agena-web",
        "Web tools (web_fetch, web_search) routed through the shared first-party executor bridge.",
        entries(),
    )
}

fn entries() -> Vec<PluginEntryDecl> {
    vec![
        PluginEntryDecl::new(
            "web_fetch",
            crate::entry::definition::json_schema_for::<WebFetchToolInput>(),
        )
        .description(
            "Fetch a URL and return its content as Markdown. HTTP is upgraded to HTTPS; cached for 15 minutes.",
        )
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["web", "fetch", "download", "url", "http", "page"])
        .deferred_load()
        .plan_mode_policy(PlanModePolicy::Allowed),
        PluginEntryDecl::new(
            "web_search",
            crate::entry::definition::json_schema_for::<WebSearchToolInput>(),
        )
        .description(
            "Search the web. Backend selectable in config (tavily, exa, brave, or duckduckgo_html as zero-config default).",
        )
        .behavior(SdkEntryBehavior::ReadOnly)
        .search_terms(["web", "search", "google", "ddg", "find online"])
        .deferred_load()
        .plan_mode_policy(PlanModePolicy::Allowed),
    ]
}
