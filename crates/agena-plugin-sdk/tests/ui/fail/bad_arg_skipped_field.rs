use agena_plugin_sdk::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema, ToolInput)]
struct BadInput {
    #[serde(skip)]
    #[arg(trim)]
    path: String,
}

fn main() {}
