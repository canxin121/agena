use agena_plugin_sdk::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema, ToolInput)]
struct BadInput {
    #[arg(path.read, jsonpath = "paths[*]")]
    path: String,
}

fn main() {}
