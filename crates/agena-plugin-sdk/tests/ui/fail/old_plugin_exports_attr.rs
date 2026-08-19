use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct OldExports;

#[agena_plugin(
    namespace = "test",
    name = "old-exports",
    version = "0.0.0",
    summary = "compile fail fixture",
    exports(PluginServiceExport::new("test.echo", 1))
)]
impl OldExports {}

fn main() {}
