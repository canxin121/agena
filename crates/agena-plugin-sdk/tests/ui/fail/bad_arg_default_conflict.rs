use agena_plugin_sdk::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema, ToolInput)]
struct BadInput {
    #[serde(default)]
    #[arg(default = 3)]
    count: usize,
}

fn main() {}
