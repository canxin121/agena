//! Bundled memory plugin: durable memory tools backed by `agena-memory-index`.

mod plugin;

pub(crate) use plugin::{MEMORY_PLUGIN_ID, MemoryPlugin};

pub fn new_memory_plugin() -> impl agena_plugin_host::sdk::Plugin {
    MemoryPlugin::new()
}

pub fn memory_plugin_id() -> &'static str {
    MEMORY_PLUGIN_ID
}
