mod plugin;

pub(crate) use plugin::{WEB_PLUGIN_ID, WebPlugin};

pub(crate) fn new_web_plugin() -> impl agena_plugin_host::sdk::Plugin {
    WebPlugin::new()
}

pub(crate) fn web_plugin_id() -> &'static str {
    WEB_PLUGIN_ID
}
