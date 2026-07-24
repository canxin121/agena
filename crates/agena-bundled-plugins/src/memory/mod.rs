mod plugin;
pub mod project_instructions;

pub(crate) use plugin::{MEMORY_PLUGIN_ID, MemoryPlugin};
pub(crate) use project_instructions::{discover, discover_global, render_section};

pub fn new_memory_plugin() -> impl agena_plugin_host::sdk::Plugin {
    MemoryPlugin::new()
}

pub fn memory_plugin_id() -> &'static str {
    MEMORY_PLUGIN_ID
}
