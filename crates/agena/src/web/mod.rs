mod plugin;

pub use plugin::{WEB_PLUGIN_ID, WebConfig, WebPlugin};

pub fn new_web_plugin() -> impl crate::plugin::sdk::Plugin {
    WebPlugin::new()
}

pub fn web_plugin_id() -> &'static str {
    WEB_PLUGIN_ID
}
