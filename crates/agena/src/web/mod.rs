mod plugin;

pub use plugin::{WEB_PLUGIN_ID, WebPlugin};

pub fn new_web_plugin(config: crate::config::WebConfig) -> impl crate::plugin::sdk::Plugin {
    WebPlugin::new(config)
}

pub fn web_plugin_id() -> &'static str {
    WEB_PLUGIN_ID
}
