use agena_plugin_sdk::prelude::*;

struct BadPlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad",
    version = "0.0.0",
    summary = "Bad plugin."
)]
impl BadPlugin {
    #[hook(chat.message, tool = "render")]
    fn chat_message(&self, _input: ChatMessageInput) -> Option<ChatMessagePatch> {
        None
    }
}

fn main() {}
