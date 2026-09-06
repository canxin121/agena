// The HTTP export expression must remain callable for every new instance,
// including plugins whose generated init populates a OnceLock settings store.
#[cfg(feature = "http")]
#[allow(dead_code)]
mod exported {
    use agena_plugin_sdk::{PluginSettings, agena_plugin};

    #[derive(Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
    struct Settings {
        revision: usize,
    }

    #[derive(Default)]
    struct SettingsPlugin {
        settings: PluginSettings<Settings>,
    }

    #[agena_plugin(
        namespace = "test",
        name = "http-settings-export",
        version = "1.0.0",
        summary = "Compile an HTTP settings plugin export.",
        settings = Settings,
        settings_default = default,
        settings_field = settings,
        export = http,
        bind = "127.0.0.1:0",
    )]
    impl SettingsPlugin {}
}

fn main() {}
