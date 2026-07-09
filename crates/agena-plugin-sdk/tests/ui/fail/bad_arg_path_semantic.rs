use agena_plugin_sdk::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema, ToolInput)]
struct BadInput {
    #[arg(path.read, path.write)]
    path: String,
}

fn main() {}
