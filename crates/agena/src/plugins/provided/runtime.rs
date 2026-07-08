use std::sync::Arc;

use crate::message::{AgentSwitchToolInput, AskUserToolInput};
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};
use crate::plugins::provided::workflow::{
    SessionRenameToolInput, WorkflowPlugin, WorkflowPluginConfig,
};

pub(crate) const RUNTIME_PLUGIN_ID: &str = "agena.runtime";

pub(crate) struct RuntimePlugin {
    inner: WorkflowPlugin,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "runtime",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Runtime session, agent, and user-interaction tools.",
    display = brief_detailed
)]
impl RuntimePlugin {
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
        name = "switch",
        summary = "Switch the current runtime agent profile.",
        display = brief,
        capabilities(HostCapability::AgentRegistry),
        trim("agent")
    )]
    async fn switch(&self, input: &AgentSwitchToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_agent_switch(input).await
    }

    #[tool(
        name = "restore",
        summary = "Restore the previous runtime agent profile.",
        display = brief,
        capabilities(HostCapability::AgentRegistry)
    )]
    async fn restore(&self) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_agent_restore().await
    }

    #[tool(
        name = "get",
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
        name = "rename",
        summary = "Rename the current session.",
        mutating,
        display = brief,
        capabilities(HostCapability::SessionRegistry),
        trim("title"),
        non_empty("title")
    )]
    async fn rename(&self, input: &SessionRenameToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_rename_session(input).await
    }

    #[tool(
        name = "request_input",
        summary = "Request short structured input from the user.",
        interactive,
        display = brief,
        capabilities(HostCapability::AskUser),
        trim(
            "title",
            "body_markdown",
            "kind",
            "submit_label",
            "cancel_label",
            "questions[].id",
            "questions[].header",
            "questions[].question",
            "questions[].options[].label",
            "questions[].options[].description"
        ),
        min_items("questions", 1),
        max_items("questions", 3),
        max_items("questions[].options", 8),
        max_chars("questions[].header", 12),
        required_unless_present("questions[].allow_custom", "questions[].options"),
        non_empty("questions[].id", "questions[].question"),
        non_empty_if_present("questions[].options[].label"),
        distinct_trimmed("questions[].id"),
        distinct_trimmed_within("questions[].options[].label", "questions[]")
    )]
    async fn request_input(&self, input: &AskUserToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_ask_user(input).await
    }
}
