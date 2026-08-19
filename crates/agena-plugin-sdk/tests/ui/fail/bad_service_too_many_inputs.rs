use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadServicePlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad_service_too_many_inputs",
    version = "0.0.0",
    summary = "compile fail fixture"
)]
impl BadServicePlugin {
    #[service("test.echo", version = 1)]
    fn query(&self, left: String, right: String) -> String {
        format!("{left}{right}")
    }
}

fn main() {}
