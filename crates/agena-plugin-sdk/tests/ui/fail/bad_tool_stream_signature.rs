use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadStreamPlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad_stream",
    version = "0.0.0",
    summary = "Bad stream signature test plugin."
)]
impl BadStreamPlugin {
    #[tool(
        summary = "Echo text.",
        read_only,
        stream = echo_stream
    )]
    fn echo(&self, text: String) -> String {
        text
    }

    fn echo_stream(&self, _sink: ToolStreamSink) -> String {
        String::new()
    }
}

fn main() {}
