use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct OutputPlugin;

#[agena_plugin(
    namespace = "test",
    name = "command-output",
    version = "0.0.0",
    summary = "Command output compile-pass fixture."
)]
impl OutputPlugin {
    #[command(id = "test.inline", title = "Inline")]
    fn inline(&self) -> String {
        "inline".to_string()
    }

    #[command(id = "test.effect", title = "Effect")]
    fn effect(&self) -> CommandResult {
        CommandResult::succeeded("effect").with_effect(CommandHostEffect::InsertPrompt {
            prompt: "continue".to_string(),
        })
    }

    #[command(id = "test.maybe_prompt", title = "Maybe Prompt")]
    fn maybe_prompt(&self, #[arg(default)] enabled: bool) -> Option<CommandResult> {
        enabled.then(|| {
            CommandResult::succeeded("prompt").with_effect(CommandHostEffect::InsertPrompt {
                prompt: "hello prompt".to_string(),
            })
        })
    }

    #[command(id = "test.flag", title = "Flag")]
    fn flag(&self, #[arg(default)] enabled: bool) -> String {
        enabled.to_string()
    }
}

fn main() {}
