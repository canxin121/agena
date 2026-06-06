use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadPlugin {
    config: PluginConfig<BadConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
struct BadConfig {
    enabled: bool,
}

#[plugin(
    id = "bad.config_field_without_config",
    version = "1.0.0",
    description = "Bad plugin.",
    config_field = config
)]
impl BadPlugin {}

fn main() {}
