use std::sync::Arc;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};
use crate::plugins::provided::workflow::{
    SessionRenameToolInput, WorkflowPlugin, WorkflowPluginConfig,
};

pub(crate) const SESSION_PLUGIN_ID: &str = "agena.session";

pub(crate) struct SessionPlugin {
    inner: WorkflowPlugin,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "session",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Runtime session tools.",
    display = brief_detailed
)]
impl SessionPlugin {
    pub(crate) fn new() -> Self {
        Self {
            inner: WorkflowPlugin::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.inner
            .initialize(ctx, WorkflowPluginConfig::default(), host)?;
        Ok(InitOutcome::ack(crate::plugin::sdk::Plugin::manifest(self)))
    }

    #[tool(
        summary = "Inspect the current session metadata.",
        read_only,
        display = brief,
        capabilities(HostCapability::SessionRegistry),
        concurrency_safe
    )]
    async fn get(&self) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_get_session().await
    }

    #[tool(
        summary = "Rename the current session.",
        mutating,
        display = brief,
        capabilities(HostCapability::SessionRegistry)
    )]
    async fn rename(&self, input: &SessionRenameToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_rename_session(input).await
    }
}

#[cfg(test)]
mod tests {
    use crate::plugin::sdk::Plugin;

    use super::SessionPlugin;

    #[test]
    fn manifest_contains_only_session_tools() {
        let manifest = SessionPlugin::new().manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "session");
        assert_eq!(tool_names, ["get", "rename"]);
    }
}
