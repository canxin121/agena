use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct OutputPlugin;

#[agena_plugin(
    namespace = "test",
    name = "operation-output",
    version = "0.0.0",
    summary = "Operation output compile-pass fixture."
)]
impl OutputPlugin {
    #[operation(id = "test.inline", title = "Inline")]
    fn inline(&self) -> String {
        "inline".to_string()
    }

    #[operation(id = "test.effect", title = "Effect")]
    fn effect(&self) -> PluginOperationResult {
        PluginOperationResult::succeeded("effect").with_effect(PluginHostEffect::InsertPrompt {
            prompt: "continue".to_string(),
        })
    }

    #[operation(id = "test.maybe_prompt", title = "Maybe Prompt")]
    fn maybe_prompt(&self, #[arg(default)] enabled: bool) -> Option<PluginOperationResult> {
        enabled.then(|| {
            PluginOperationResult::succeeded("prompt").with_effect(PluginHostEffect::InsertPrompt {
                prompt: "hello prompt".to_string(),
            })
        })
    }

    #[operation(id = "test.flag", title = "Flag")]
    fn flag(&self, #[arg(default)] enabled: bool) -> String {
        enabled.to_string()
    }
}

fn main() {}
