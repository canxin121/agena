use agena_plugin_sdk::prelude::*;

struct BadPlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad",
    version = "0.0.0",
    summary = "Bad plugin."
)]
impl BadPlugin {
    #[hook]
    fn init(&self, _ctx: InitContext, _host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        Ok(InitOutcome::ack(Plugin::manifest(self)))
    }
}

fn main() {}
