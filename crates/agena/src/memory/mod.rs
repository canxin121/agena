mod paths;
pub(crate) mod prompt;
mod plugin;

pub use paths::MemoryDir;
pub use prompt::build_memory_prompt_section;
pub use plugin::MemoryPlugin;

pub fn new_memory_plugin() -> impl crate::plugin::sdk::Plugin {
    MemoryPlugin::new()
}
