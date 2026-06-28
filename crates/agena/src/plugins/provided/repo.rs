use std::sync::Arc;

use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, PathRequest, Plugin, PluginManifest,
    Result as SdkResult, ToolInvokeInput, ToolInvokeOutput, async_trait,
};
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig, WorktreeToolInput};

pub(crate) const REPO_PLUGIN_ID: &str = "agena.repo";

pub(crate) struct RepoPlugin {
    inner: WorkflowPlugin,
}

impl RepoPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }
}

#[async_trait]
impl Plugin for RepoPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(REPO_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Repository workspace snapshot tools backed by Rift or git worktree.")
            .brief_detailed()
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(WorktreeToolInput::tool_decl())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "worktree" {
            return Err(PluginError::not_implemented(format!(
                "tool_invoke({})",
                input.tool_name
            )));
        }
        let parsed = WorktreeToolInput::parse_input(input.input)?;
        parsed.dispatch_tool_invoke(&self.inner).await
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        let parsed = WorktreeToolInput::parse_tool(tool, input.clone())?;
        parsed.dispatch_permission_paths(&self.inner).await
    }
}
