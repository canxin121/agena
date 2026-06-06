use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolCommand)]
#[tool_command(tool = "bad.echo", description = "Bad echo.")]
struct EchoInput {
    text: String,
}

#[derive(Default)]
struct BadPlugin;

#[plugin(
    id = "bad.stream_missing_target",
    version = "1.0.0",
    description = "Bad plugin."
)]
impl BadPlugin {
    #[stream(for = echo)]
    async fn echo_stream(&self, input: EchoInput, sink: ToolStreamSink) -> Result<ToolStreamEnd> {
        Ok(ToolStreamEnd::text(sink.stream_id(), input.text))
    }
}

fn main() {}
