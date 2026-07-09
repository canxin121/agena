use agena_plugin_sdk::prelude::*;

struct BadPlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad",
    version = "0.0.0",
    summary = "Bad plugin."
)]
impl BadPlugin {
    #[hook(tool.after, tags(filesystem_write))]
    fn after_tool(&self, _input: ToolAfterInput) -> Option<ToolAfterPatch> {
        None
    }
}

fn main() {}
