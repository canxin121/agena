use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct OldSettingsBuilder;

#[agena_plugin(
    namespace = "test",
    name = "old-settings-builder",
    version = "0.0.0",
    summary = "compile fail fixture",
    settings_builder = SettingsContract::empty_object("Settings", "")
)]
impl OldSettingsBuilder {}

fn main() {}
