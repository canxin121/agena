use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadServicePlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad_service_missing_version",
    version = "0.0.0",
    summary = "compile fail fixture"
)]
impl BadServicePlugin {
    #[service("test.echo")]
    fn query(&self) -> String {
        "ok".to_string()
    }
}

fn main() {}
