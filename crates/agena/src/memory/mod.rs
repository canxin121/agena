mod paths;
mod plugin;
pub(crate) mod prompt;
pub mod project_instructions;
pub mod store;

pub use paths::MemoryDir;
pub use plugin::MemoryPlugin;
pub use project_instructions::{ProjectInstructionLayer, discover, render_section};
pub use prompt::build_memory_prompt_section;
pub use store::{
    MemoryEntry, MemoryError, MemoryFrontmatter, MemoryStore, MemoryType, NewMemory,
};

pub fn new_memory_plugin() -> impl crate::plugin::sdk::Plugin {
    MemoryPlugin::new()
}
