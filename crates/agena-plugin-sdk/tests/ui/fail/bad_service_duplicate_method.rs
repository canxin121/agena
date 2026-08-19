use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct BadServicePlugin;

#[agena_plugin(
    namespace = "test",
    name = "bad_service_duplicate_method",
    version = "0.0.0",
    summary = "compile fail fixture"
)]
impl BadServicePlugin {
    #[service("test.echo", version = 1, method = "query")]
    fn first(&self) -> String {
        "first".to_string()
    }

    #[service("test.echo", version = 1, method = "query")]
    fn second(&self) -> String {
        "second".to_string()
    }
}

fn main() {}
