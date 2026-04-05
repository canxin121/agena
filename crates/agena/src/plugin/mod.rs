pub mod api;
mod manager;
mod runtime;

pub use api::{
    AgenaPlugin, PluginAfterToolRequest, PluginAfterToolResponse, PluginBeforeToolRequest,
    PluginBeforeToolResponse, PluginError, PluginMetadata, PluginShellEnvRequest,
    PluginShellEnvResponse, PluginToolCallRequest, PluginToolCallResponse, PluginToolDescriptor,
};
pub use manager::{LoadedPlugin, PluginLoadError, PluginManager};
