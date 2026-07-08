use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct OldStreamPlugin;

#[agena_plugin(
    namespace = "test",
    name = "old_stream",
    version = "0.0.0",
    summary = "Old stream syntax test plugin."
)]
impl OldStreamPlugin {
    #[tool(summary = "Echo text.", read_only, streaming)]
    fn echo(&self, #[arg(trim)] text: String) -> String {
        text
    }

    #[stream(echo)]
    fn echo_stream(&self, text: String, _sink: ToolStreamSink) -> String {
        text
    }
}

fn main() {}
