use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct CommandOutputPlugin;

#[agena_plugin(
    namespace = "test",
    name = "command-output",
    version = "0.0.0",
    summary = "Command output macro test plugin."
)]
impl CommandOutputPlugin {
    #[tool(summary = "Plain string tool.", read_only)]
    fn raw_echo(&self, text: String) -> String {
        text
    }

    #[command(id = "test.inline", title = "Inline")]
    fn inline(&self, #[arg(trim, non_empty)] name: String) -> String {
        name
    }

    #[command(id = "test.chain", title = "Chain")]
    fn chain(&self) -> PluginCommandOutput {
        PluginCommandOutput::invoke_command(
            "test.inline",
            Some(agena_plugin_sdk::serde_json::json!({ "name": "Ada" })),
        )
    }

    #[command(id = "test.maybe_prompt", title = "Maybe Prompt")]
    fn maybe_prompt(&self, #[arg(default)] enabled: bool) -> Option<PluginCommandOutput> {
        if enabled {
            Some(PluginCommandOutput::submit_prompt("hello prompt"))
        } else {
            None
        }
    }

    #[command(id = "test.flag", title = "Flag")]
    fn flag(&self, enabled: bool) -> String {
        enabled.to_string()
    }
}

fn main() {}
