use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct EchoOutput {
    text: String,
}

#[derive(Default)]
struct OldOutputPlugin;

#[agena_plugin(
    namespace = "test",
    name = "old_output",
    version = "0.0.0",
    summary = "Old output syntax test plugin."
)]
impl OldOutputPlugin {
    #[tool(summary = "Echo text.", output = EchoOutput, read_only)]
    fn echo(&self, #[arg(trim, non_empty)] text: String) -> EchoOutput {
        EchoOutput { text }
    }
}

fn main() {}
