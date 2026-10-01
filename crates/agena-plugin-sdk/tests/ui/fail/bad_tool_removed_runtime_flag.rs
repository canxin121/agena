use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct LegacyRuntimeFlagPlugin;

#[agena_plugin(
    namespace = "test",
    name = "legacy-runtime-flag",
    version = "0.0.0",
    summary = "Rejects the removed concurrency declaration."
)]
impl LegacyRuntimeFlagPlugin {
    #[tool(summary = "Bare flag spelling.", tags(read_only), concurrency_safe)]
    fn bare_flag(&self) -> String {
        String::new()
    }
}

fn main() {}
