use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadPlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad",
    version = "0.0.0",
    summary = "Bad plugin.",
    commands = Vec::<PluginOperationDefinition>::new()
)]
impl BadPlugin {}

fn main() {}
