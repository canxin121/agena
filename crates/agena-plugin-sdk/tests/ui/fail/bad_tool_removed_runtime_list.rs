use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct LegacyRuntimeListPlugin;

#[agena_plugin(
    namespace = "test",
    name = "legacy-runtime-list",
    version = "0.0.0",
    summary = "Rejects the removed concurrency declaration."
)]
impl LegacyRuntimeListPlugin {
    #[tool(summary = "List spelling.", tags(read_only), concurrency_safe = true)]
    fn list_entry(&self) -> String {
        String::new()
    }
}

fn main() {}
