use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadPlugin;

#[plugin(
    id = "bad.unknown_hook",
    version = "1.0.0",
    description = "Bad plugin."
)]
impl BadPlugin {
    #[hook]
    async fn not_a_hook(&self) -> Result<()> {
        Ok(())
    }
}

fn main() {}
