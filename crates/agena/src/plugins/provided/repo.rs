use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, PathRequest, Plugin, PluginManifest,
    Result as SdkResult, ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{SnapshotToolSuite, WorkflowPlugin, WorkflowPluginConfig};

pub(crate) const SNAPSHOT_PLUGIN_ID: &str = "agena.snapshot";

pub(crate) struct SnapshotPlugin {
    inner: WorkflowPlugin,
}

impl SnapshotPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }
}

#[async_trait]
impl Plugin for SnapshotPlugin {
    fn manifest(&self) -> PluginManifest {
        let mut manifest = PluginManifest::new("agena", "snapshot", env!("CARGO_PKG_VERSION"));
        manifest.summary =
            Some("Managed snapshot tools backed by Rift or git worktree.".to_string());
        manifest.set_display(crate::plugin::sdk::ToolDisplayPreset::BriefDetailed);
        manifest.hooks |= HookSubscription::TOOL_INVOKE;
        manifest.tools.extend(SnapshotToolSuite::tool_definitions());
        manifest
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let parsed = SnapshotToolSuite::parse_tool(input.tool_name.as_str(), input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        let parsed = SnapshotToolSuite::parse_tool(tool, input.clone())?;
        parsed.dispatch_permission_paths(&self.inner).await
    }
}
