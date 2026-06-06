use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolCommand)]
#[tool_command(tool = "bad.echo", description = "Bad echo.")]
struct EchoInput {
    text: String,
}

#[derive(Default)]
struct BadPlugin;

#[plugin(
    id = "bad.stream_without_sink",
    version = "1.0.0",
    description = "Bad plugin."
)]
impl BadPlugin {
    #[tool]
    async fn echo(&self, input: EchoInput) -> String {
        input.text
    }

    #[tool_stream]
    async fn echo_stream(&self, input: EchoInput) -> Result<ToolStreamEnd> {
        Ok(ToolStreamEnd::text("bad", input.text))
    }
}

fn main() {}
