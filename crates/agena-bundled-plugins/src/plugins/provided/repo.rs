use std::sync::Arc;

use crate::plugins::provided::workflow::{
    EnterSnapshotCommandInput, ExitSnapshotCommandInput, WorkflowPlugin, WorkflowPluginConfig,
};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, PluginCommandOutput, PluginUiAction, Result as SdkResult,
    ToolInvokeOutput,
};

pub(crate) const SNAPSHOT_PLUGIN_ID: &str = "agena.snapshot";

fn snapshot_config_schema() -> serde_json::Value {
    serde_json::json!({
        "title": "Snapshot Plugin Config",
        "description": "Snapshot behavior is selected per invocation; this plugin has no persistent settings.",
        "type": "object",
        "properties": {},
        "additionalProperties": false,
        "default": {}
    })
}

pub(crate) struct SnapshotPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "snapshot",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Managed snapshot tools backed by Rift or git worktree.",
    config_schema = snapshot_config_schema(),
    display = brief_detailed
)]
impl SnapshotPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[command(
        id = "snapshot.open",
        title = "Open Snapshot",
        description = "Open Snapshot tools in Plugin Workbench.",
        slash = "/snapshot",
        usage = "/snapshot",
        action = PluginUiAction::OpenPluginWorkbench { tab: Some("tools".to_string()) }
    )]
    async fn command_open(&self) -> PluginCommandOutput {
        PluginCommandOutput::open_plugin_workbench(Some("tools"))
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
        tags(query, snapshot),
        summary = "List active managed repository snapshots.",
        read_only,
        snapshot,
        display = brief,

    )]
    async fn status(&self) -> SdkResult<ToolInvokeOutput> {
        let response = self.inner.host()?.snapshot_list().await?;
        let payload = serde_json::to_value(&response)
            .map_err(|error| agena_plugin_host::PluginError::internal(error.to_string()))?;
        let output = if response.snapshots.is_empty() {
            "No active snapshots.".to_owned()
        } else {
            response
                .snapshots
                .iter()
                .map(|snapshot| {
                    format!(
                        "session #{} · {} · branch={} · created_here={}",
                        snapshot.session_id, snapshot.path, snapshot.branch, snapshot.created_here
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(ToolInvokeOutput::from_parts(
            "snapshot status",
            format!("{} active snapshots", response.snapshots.len()),
            output,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    #[tool(
        tags(mutate, snapshot),
        summary = "Enter a managed repository snapshot.",
        mutating,

        snapshot,
        display = brief,

        path(requests = self.inner.permission_snapshot_enter(input).await?)
    )]
    async fn enter(&self, input: &EnterSnapshotCommandInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_snapshot_enter(input).await
    }

    #[tool(
        tags(mutate, snapshot),
        summary = "Exit a managed repository snapshot.",
        mutating,

        snapshot,
        display = brief,

        path(requests = self.inner.permission_snapshot_exit(input).await?)
    )]
    async fn exit(&self, input: &ExitSnapshotCommandInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_snapshot_exit(input).await
    }
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::{Plugin, PluginUiAction, ToolTag};

    use super::SnapshotPlugin;

    #[test]
    fn manifest_exposes_snapshot_slash_command() {
        let manifest = SnapshotPlugin::new().manifest();
        let command = manifest
            .commands
            .iter()
            .find(|command| command.id == "snapshot.open")
            .expect("snapshot command");

        assert_eq!(command.slash.as_deref(), Some("/snapshot"));
        assert!(matches!(
            &command.action,
            PluginUiAction::OpenPluginWorkbench { tab } if tab.as_deref() == Some("tools")
        ));
        let schema = manifest
            .config_schema
            .expect("explicit empty config schema");
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["properties"], serde_json::json!({}));
        assert_eq!(manifest.tools[0].name, "status");
        assert!(
            manifest.tools[0].tags.contains(&ToolTag::Snapshot),
            "snapshot remains a discovery/UI metadata tag"
        );
        assert!(
            manifest.tools[0].permissions.read_only,
            "read_only must remain an authority-bearing contract flag"
        );
    }
}
