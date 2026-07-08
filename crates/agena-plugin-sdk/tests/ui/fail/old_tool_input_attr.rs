use agena_plugin_sdk::serde::{Deserialize, Serialize};
use agena_plugin_sdk::{JsonSchema, ToolInput};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(tag = "action", rename_all = "snake_case")]
enum OldInput {
    #[tool(default_when_empty = true)]
    List {},
}

fn main() {}
