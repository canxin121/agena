//! Shared scaffolding for first-party plugins that simply forward their entries
//! to the existing built-in tool implementation through
//! `HostClient::execute_builtin_tool`. This keeps the plugin shells tiny while
//! the substrate carries the real work.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{BuiltinToolRequest, HostClient};
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginEntryDecl, PluginManifest,
    Result as SdkResult, ToolInvokeInput, ToolInvokeOutput,
};

/// Plugin shell whose every model-visible entry is fulfilled by routing to
/// `host.execute_builtin_tool(name, input)`.
pub struct BuiltinRouterPlugin {
    plugin_name: &'static str,
    description: &'static str,
    entries: Vec<PluginEntryDecl>,
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

impl BuiltinRouterPlugin {
    pub fn new(
        plugin_name: &'static str,
        description: &'static str,
        entries: Vec<PluginEntryDecl>,
    ) -> Self {
        Self {
            plugin_name,
            description,
            entries,
            host: RwLock::new(None),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("router plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("router plugin invoked before init"))
    }
}

#[async_trait]
impl Plugin for BuiltinRouterPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut builder = PluginManifest::builder(self.plugin_name, env!("CARGO_PKG_VERSION"))
            .description(self.description)
            .hooks(HookSubscription::TOOL_INVOKE);
        for entry in &self.entries {
            builder = builder.entry(entry.clone());
        }
        builder.build()
    }

    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("router plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        self.host()?
            .execute_builtin_tool(BuiltinToolRequest {
                tool_name: input.tool_name,
                input: input.input,
            })
            .await
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
        crate::plugins::bundled::builtin::permission_paths_for(tool, input)
    }
}
