use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct LegacyExamplesPlugin;

#[agena_plugin(
    namespace = "test",
    name = "legacy-examples",
    version = "0.0.0",
    summary = "Rejects the removed per-tool example surface."
)]
impl LegacyExamplesPlugin {
    #[tool(summary = "Declares its own example text.", tags(read_only), examples(r#"{"path":"README.md"}"#))]
    fn declared_example(&self) -> String {
        String::new()
    }
}

fn main() {}
