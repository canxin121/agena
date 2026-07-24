use std::sync::Arc;

use crate::plugins::provided::workflow::{
    EnterSnapshotCommandInput, ExitSnapshotCommandInput, WorkflowPlugin, WorkflowPluginConfig,
};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};

pub(crate) const SNAPSHOT_PLUGIN_ID: &str = "agena.snapshot";

pub(crate) struct SnapshotPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
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

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        summary = "Enter a managed repository snapshot.",
        mutating,
        filesystem_write,
        snapshot,
        display = brief,
        capabilities(HostCapability::SnapshotRegistry, HostCapability::PluginStorage),
        path(requests = self.inner.permission_snapshot_enter(input).await?)
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
        path(requests = self.inner.permission_snapshot_exit(input).await?)
    )]
    async fn exit(&self, input: &ExitSnapshotCommandInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_snapshot_exit(input).await
    }
}
