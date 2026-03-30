use std::path::{Path, PathBuf};
use std::sync::Arc;

use abi_stable::library::RootModule;

use crate::plugin::api::{
    AgenaPlugin, AgenaPluginModuleRef, PluginAfterToolRequest, PluginAfterToolResponse,
    PluginBeforeToolRequest, PluginBeforeToolResponse, PluginError, PluginMetadata,
    PluginShellEnvRequest, PluginShellEnvResponse, PluginToolCallRequest, PluginToolCallResponse,
    PluginToolDescriptor,
};

use super::manager::PluginLoadError;

pub trait PluginRuntime: Send + Sync {
    fn metadata(&self) -> PluginMetadata;
    fn tools(&self) -> Vec<PluginToolDescriptor>;
    fn invoke_tool(
        &self,
        request: PluginToolCallRequest,
    ) -> Result<PluginToolCallResponse, PluginError>;
    fn before_tool(
        &self,
        request: PluginBeforeToolRequest,
    ) -> Result<PluginBeforeToolResponse, PluginError>;
    fn after_tool(
        &self,
        request: PluginAfterToolRequest,
    ) -> Result<PluginAfterToolResponse, PluginError>;
    fn shell_env(
        &self,
        request: PluginShellEnvRequest,
    ) -> Result<PluginShellEnvResponse, PluginError>;
}

pub struct DynamicPluginRuntime {
    _path: PathBuf,
    module: AgenaPluginModuleRef,
    metadata: PluginMetadata,
    tools: Vec<PluginToolDescriptor>,
}

impl DynamicPluginRuntime {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PluginLoadError> {
        let path = path.as_ref().to_path_buf();
        let module = AgenaPluginModuleRef::load_from_file(path.as_path()).map_err(|err| {
            PluginLoadError::Load {
                path: path.clone(),
                message: err.to_string(),
            }
        })?;
        let metadata = module.metadata()().into();
        let tools = module
            .tools()()
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();

        Ok(Self {
            _path: path,
            module,
            metadata,
            tools,
        })
    }
}

impl PluginRuntime for DynamicPluginRuntime {
    fn metadata(&self) -> PluginMetadata {
        self.metadata.clone()
    }

    fn tools(&self) -> Vec<PluginToolDescriptor> {
        self.tools.clone()
    }

    fn invoke_tool(
        &self,
        request: PluginToolCallRequest,
    ) -> Result<PluginToolCallResponse, PluginError> {
        self.module
            .invoke_tool()(request.into())
            .into_result()
            .map(Into::into)
            .map_err(Into::into)
    }

    fn before_tool(
        &self,
        request: PluginBeforeToolRequest,
    ) -> Result<PluginBeforeToolResponse, PluginError> {
        self.module
            .before_tool()(request.into())
            .into_result()
            .map(Into::into)
            .map_err(Into::into)
    }

    fn after_tool(
        &self,
        request: PluginAfterToolRequest,
    ) -> Result<PluginAfterToolResponse, PluginError> {
        self.module
            .after_tool()(request.into())
            .into_result()
            .map(Into::into)
            .map_err(Into::into)
    }

    fn shell_env(
        &self,
        request: PluginShellEnvRequest,
    ) -> Result<PluginShellEnvResponse, PluginError> {
        self.module
            .shell_env()(request.into())
            .into_result()
            .map(Into::into)
            .map_err(Into::into)
    }
}

pub struct StaticPluginRuntime {
    plugin: Arc<dyn AgenaPlugin>,
}

impl StaticPluginRuntime {
    pub fn new(plugin: impl AgenaPlugin) -> Self {
        Self {
            plugin: Arc::new(plugin),
        }
    }
}

impl PluginRuntime for StaticPluginRuntime {
    fn metadata(&self) -> PluginMetadata {
        self.plugin.metadata()
    }

    fn tools(&self) -> Vec<PluginToolDescriptor> {
        self.plugin.tools()
    }

    fn invoke_tool(
        &self,
        request: PluginToolCallRequest,
    ) -> Result<PluginToolCallResponse, PluginError> {
        self.plugin.invoke_tool(request)
    }

    fn before_tool(
        &self,
        request: PluginBeforeToolRequest,
    ) -> Result<PluginBeforeToolResponse, PluginError> {
        self.plugin.before_tool(request)
    }

    fn after_tool(
        &self,
        request: PluginAfterToolRequest,
    ) -> Result<PluginAfterToolResponse, PluginError> {
        self.plugin.after_tool(request)
    }

    fn shell_env(
        &self,
        request: PluginShellEnvRequest,
    ) -> Result<PluginShellEnvResponse, PluginError> {
        self.plugin.shell_env(request)
    }
}
