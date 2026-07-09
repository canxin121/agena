use agena_plugin_sdk::prelude::*;

#[derive(Serialize, Deserialize, JsonSchema, ToolInput)]
struct BadInput {
    foo: String,
    #[arg(alias = "foo")]
    bar: String,
}

fn main() {}
