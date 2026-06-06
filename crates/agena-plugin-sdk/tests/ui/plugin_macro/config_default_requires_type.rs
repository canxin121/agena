use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
struct BadConfig {
    enabled: bool,
}

#[derive(Default, PluginConfigStore)]
struct BadPlugin {
    #[config]
    config: PluginConfig<BadConfig>,
}

#[plugin(
    id = "bad.config_default_requires_type",
    version = "1.0.0",
    description = "Bad plugin.",
    config,
    config_default = default
)]
impl BadPlugin {}

fn main() {}
