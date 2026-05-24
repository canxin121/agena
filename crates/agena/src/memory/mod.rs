mod index;
mod paths;
mod plugin;
pub mod project_instructions;
pub(crate) mod prompt;
pub mod store;

pub(crate) use index::{MemoryIndex, MemorySearchDocument};
pub use paths::MemoryDir;
pub use plugin::{MEMORY_PLUGIN_ID, MemoryPlugin};
pub use project_instructions::{
    ProjectInstructionLayer, discover, discover_global, render_section,
};
pub use prompt::build_memory_prompt_section;
pub use store::{MemoryEntry, MemoryError, MemoryFrontmatter, MemoryStore, MemoryType, NewMemory};

pub fn new_memory_plugin(config: crate::config::MemoryConfig) -> impl crate::plugin::sdk::Plugin {
    MemoryPlugin::new(config)
}

pub fn memory_plugin_id() -> &'static str {
    MEMORY_PLUGIN_ID
}
