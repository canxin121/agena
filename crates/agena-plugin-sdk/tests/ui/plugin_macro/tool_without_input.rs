use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadPlugin;

#[plugin(
    id = "bad.tool_without_input",
    version = "1.0.0",
    description = "Bad plugin."
)]
impl BadPlugin {
    #[tool]
    async fn echo(&self) -> Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text("bad"))
    }
}

fn main() {}
