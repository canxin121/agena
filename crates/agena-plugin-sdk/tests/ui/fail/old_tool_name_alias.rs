use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct OldToolAliasPlugin;

#[agena_plugin(
    namespace = "test",
    name = "old_tool_alias",
    version = "0.0.0",
    summary = "Old tool alias test plugin."
)]
impl OldToolAliasPlugin {
    #[tool(tool = "echo", summary = "Echo text.", read_only)]
    fn echo(&self, text: String) -> String {
        text
    }
}

fn main() {}
