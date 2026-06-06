use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
struct BadConfig {
    enabled: bool,
}

#[derive(Default, PluginConfigStore)]
struct BadPlugin {
    #[config]
    config: PluginConfig<BadConfig>,
    #[plugin_config]
    fallback_config: PluginConfig<BadConfig>,
}

fn main() {}
