use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::plugin::api::{
    AgenaPlugin, PluginAfterToolRequest, PluginAfterToolResponse, PluginBeforeToolRequest,
    PluginBeforeToolResponse, PluginError, PluginMetadata, PluginShellEnvRequest,
    PluginShellEnvResponse, PluginToolCallRequest, PluginToolCallResponse, PluginToolDescriptor,
};

use super::runtime::{DynamicPluginRuntime, PluginRuntime, StaticPluginRuntime};

#[derive(Debug, Error)]
pub enum PluginLoadError {
    #[error("failed to load plugin from {path}: {message}")]
    Load { path: PathBuf, message: String },
    #[error("plugin tool name collision with different owners: {tool_name}")]
    ToolCollision { tool_name: String },
}

pub struct LoadedPlugin {
    pub metadata: PluginMetadata,
    pub library_path: Option<PathBuf>,
    pub tools: Vec<PluginToolDescriptor>,
    runtime: Arc<dyn PluginRuntime>,
}

impl fmt::Debug for LoadedPlugin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("metadata", &self.metadata)
            .field("library_path", &self.library_path)
            .field("tools", &self.tools)
            .finish()
    }
}

impl LoadedPlugin {
    fn new(
        runtime: Arc<dyn PluginRuntime>,
        library_path: Option<PathBuf>,
    ) -> Result<Self, PluginLoadError> {
        let metadata = runtime.metadata();
        let tools = runtime.tools();
        Ok(Self {
            metadata,
            library_path,
            tools,
            runtime,
        })
    }

    pub fn invoke_tool(
        &self,
        request: PluginToolCallRequest,
    ) -> Result<PluginToolCallResponse, PluginError> {
        self.runtime.invoke_tool(request)
    }

    pub fn before_tool(
        &self,
        request: PluginBeforeToolRequest,
    ) -> Result<PluginBeforeToolResponse, PluginError> {
        self.runtime.before_tool(request)
    }

    pub fn after_tool(
        &self,
        request: PluginAfterToolRequest,
    ) -> Result<PluginAfterToolResponse, PluginError> {
        self.runtime.after_tool(request)
    }

    pub fn shell_env(
        &self,
        request: PluginShellEnvRequest,
    ) -> Result<PluginShellEnvResponse, PluginError> {
        self.runtime.shell_env(request)
    }
}

#[derive(Debug, Default)]
pub struct PluginManager {
    plugins: Vec<LoadedPlugin>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn plugins(&self) -> &[LoadedPlugin] {
        &self.plugins
    }

    pub fn register_static(
        &mut self,
        plugin: impl AgenaPlugin,
    ) -> Result<&LoadedPlugin, PluginLoadError> {
        let runtime = Arc::new(StaticPluginRuntime::new(plugin));
        self.push_loaded(LoadedPlugin::new(runtime, None)?)
    }

    pub fn load_dynamic(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<&LoadedPlugin, PluginLoadError> {
        let path = path.as_ref().to_path_buf();
        let runtime = Arc::new(DynamicPluginRuntime::load(path.as_path())?);
        self.push_loaded(LoadedPlugin::new(runtime, Some(path))?)
    }

    pub fn discover_directory(
        &mut self,
        dir: impl AsRef<Path>,
    ) -> Result<Vec<String>, PluginLoadError> {
        let dir = dir.as_ref();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut loaded = Vec::new();
        let mut entries = fs::read_dir(dir)
            .map_err(|err| PluginLoadError::Load {
                path: dir.to_path_buf(),
                message: err.to_string(),
            })?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| is_dynamic_library(path))
            .collect::<Vec<_>>();
        entries.sort();

        for path in entries {
            let plugin = self.load_dynamic(path)?;
            loaded.push(plugin.metadata.name.clone());
        }

        Ok(loaded)
    }

    pub fn custom_tool(&self, tool_name: &str) -> Option<(&LoadedPlugin, &PluginToolDescriptor)> {
        self.plugins.iter().rev().find_map(|plugin| {
            plugin
                .tools
                .iter()
                .find(|descriptor| descriptor.name == tool_name)
                .map(|descriptor| (plugin, descriptor))
        })
    }

    pub fn apply_before_tool_hooks(
        &self,
        mut request: PluginBeforeToolRequest,
    ) -> Result<PluginBeforeToolResponse, PluginError> {
        let mut response = PluginBeforeToolResponse::passthrough(request.input_json.clone());
        for plugin in &self.plugins {
            request.input_json = response.input_json.clone();
            let next = plugin.before_tool(request.clone())?;
            response.input_json = next.input_json;
            if next.title_override.is_some() {
                response.title_override = next.title_override;
            }
            response.metadata.extend(next.metadata);
        }
        Ok(response)
    }

    pub fn apply_after_tool_hooks(
        &self,
        mut request: PluginAfterToolRequest,
    ) -> Result<PluginAfterToolResponse, PluginError> {
        let mut response = PluginAfterToolResponse::default();
        for plugin in &self.plugins {
            if let Some(title) = response.title.as_ref() {
                request.title = title.clone();
            }
            if let Some(output_text) = response.output_text.as_ref() {
                request.output_text = output_text.clone();
            }
            if let Some(payload_json) = response.payload_json.as_ref() {
                request.payload_json = Some(payload_json.clone());
            }
            request.metadata.extend(response.metadata.clone());

            let next = plugin.after_tool(request.clone())?;
            if next.title.is_some() {
                response.title = next.title;
            }
            if next.output_text.is_some() {
                response.output_text = next.output_text;
            }
            if next.payload_json.is_some() {
                response.payload_json = next.payload_json;
            }
            response.metadata.extend(next.metadata);
        }
        Ok(response)
    }

    pub fn shell_env(
        &self,
        request: PluginShellEnvRequest,
    ) -> Result<PluginShellEnvResponse, PluginError> {
        let mut response = PluginShellEnvResponse::default();
        for plugin in &self.plugins {
            response.env.extend(plugin.shell_env(request.clone())?.env);
        }
        Ok(response)
    }

    fn push_loaded(&mut self, plugin: LoadedPlugin) -> Result<&LoadedPlugin, PluginLoadError> {
        if let Some(conflict) = self
            .plugins
            .iter()
            .flat_map(|loaded| loaded.tools.iter())
            .find(|existing| {
                plugin
                    .tools
                    .iter()
                    .any(|candidate| candidate.name == existing.name)
            })
            .map(|tool| tool.name.clone())
        {
            return Err(PluginLoadError::ToolCollision {
                tool_name: conflict,
            });
        }

        self.plugins.push(plugin);
        Ok(self.plugins.last().expect("just pushed plugin"))
    }
}

fn is_dynamic_library(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("so" | "dylib" | "dll")
    )
}
