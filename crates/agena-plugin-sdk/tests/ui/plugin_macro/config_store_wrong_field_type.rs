use agena_plugin_sdk::prelude::*;

#[derive(Default, PluginConfigStore)]
struct BadPlugin {
    #[config]
    config: String,
}

fn main() {}
