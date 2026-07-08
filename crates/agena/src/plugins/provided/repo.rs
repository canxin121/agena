use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HostCapability, InitContext, InitOutcome, PathRequest, Result as SdkResult, ToolInvokeOutput,
};
use crate::plugins::provided::workflow::{
    EnterSnapshotCommandInput, ExitSnapshotCommandInput, WorkflowPlugin, WorkflowPluginConfig,
};

pub(crate) const SNAPSHOT_PLUGIN_ID: &str = "agena.snapshot";

pub(crate) struct SnapshotPlugin {
    inner: WorkflowPlugin,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "snapshot",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Managed snapshot tools backed by Rift or git worktree.",
    display = brief_detailed
)]
impl SnapshotPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(crate::plugin::sdk::Plugin::manifest(self)))
    }

    #[tool(
        summary = "Enter a managed repository snapshot.",
        mutating,
        filesystem_write,
        snapshot,
        display = brief,
        capabilities(HostCapability::SnapshotRegistry, HostCapability::PluginStorage),
        permission(paths = permission_enter)
    )]
    async fn enter(&self, input: &EnterSnapshotCommandInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_snapshot_enter(input).await
    }

    #[tool(
        summary = "Exit a managed repository snapshot.",
        mutating,
        filesystem_write,
        snapshot,
        display = brief,
        capabilities(HostCapability::SnapshotRegistry, HostCapability::PluginStorage),
        permission(paths = permission_exit)
    )]
    async fn exit(&self, input: &ExitSnapshotCommandInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_snapshot_exit(input).await
    }

    async fn permission_enter(
        &self,
        input: &EnterSnapshotCommandInput,
    ) -> SdkResult<Vec<PathRequest>> {
        self.inner.permission_snapshot_enter(input).await
    }

    async fn permission_exit(
        &self,
        input: &ExitSnapshotCommandInput,
    ) -> SdkResult<Vec<PathRequest>> {
        self.inner.permission_snapshot_exit(input).await
    }
}
