use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadPlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad",
    version = "0.0.0",
    summary = "Bad plugin."
)]
impl BadPlugin {
    #[hook(tool.before)]
    fn before_tool(&self, _input: ToolAfterInput) {}
}

fn main() {}
