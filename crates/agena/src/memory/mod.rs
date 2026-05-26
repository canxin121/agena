mod index;
mod paths;
mod plugin;
pub mod project_instructions;
pub(crate) mod prompt;
pub mod store;

pub(crate) use index::{MemoryIndex, MemorySearchDocument};
pub use paths::MemoryDir;
pub use plugin::{
    MEMORY_PLUGIN_ID, MemoryConfig, MemoryPlugin, MemoryRetrievalConfig, ProjectInstructionsConfig,
};
pub use project_instructions::{
    ProjectInstructionLayer, discover, discover_global, render_section,
};
pub use prompt::build_memory_prompt_section;
pub use store::{MemoryError, MemoryFrontmatter, MemoryRecord, MemoryStore, MemoryType, NewMemory};

pub fn new_memory_plugin() -> impl crate::plugin::sdk::Plugin {
    MemoryPlugin::new()
}

pub fn memory_plugin_id() -> &'static str {
    MEMORY_PLUGIN_ID
}
