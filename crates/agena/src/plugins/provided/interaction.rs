use std::sync::Arc;

use crate::message::{AskUserToolInput, InteractionNotifyToolInput};
use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HostCapability, InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput,
};
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig};

pub(crate) const INTERACTION_PLUGIN_ID: &str = "agena.interaction";

pub(crate) struct InteractionPlugin {
    inner: WorkflowPlugin,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "interaction",
    version = env!("CARGO_PKG_VERSION"),
    summary = "User interaction tools.",
    display = brief_detailed
)]
impl InteractionPlugin {
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
        summary = "Ask the user for short structured input.",
        interactive,
        display = brief,
        capabilities(HostCapability::AskUser)
    )]
    async fn ask(&self, input: &AskUserToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_ask_user(input).await
    }

    #[tool(
        summary = "Show a non-blocking Markdown notification to the user.",
        display = detailed,
        concurrency_safe
    )]
    fn notify(&self, input: &InteractionNotifyToolInput) -> SdkResult<ToolInvokeOutput> {
        let input = InteractionNotifyToolInput::parse_input(
            serde_json::to_value(input)
                .map_err(|err| crate::plugin::sdk::PluginError::invalid_params(err.to_string()))?,
        )?;
        let level = input.level.as_str();
        let title = if input.title.is_empty() {
            match input.level {
                crate::message::InteractionNotificationLevel::Info => "Notice",
                crate::message::InteractionNotificationLevel::Success => "Completed",
                crate::message::InteractionNotificationLevel::Warning => "Attention",
                crate::message::InteractionNotificationLevel::Error => "Error",
            }
            .to_string()
        } else {
            input.title
        };
        Ok(ToolInvokeOutput::from_parts(
            title.clone(),
            input.body_markdown.clone(),
            Some(serde_json::json!({
                "title": title,
                "body_markdown": input.body_markdown,
                "level": level,
            })),
            std::collections::BTreeMap::from([
                ("agena.effect".to_string(), "notification".to_string()),
                ("agena.notification.level".to_string(), level.to_string()),
            ]),
            Vec::new(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::message::{AskUserToolInput, InteractionNotifyToolInput};
    use crate::plugin::sdk::Plugin;

    use super::InteractionPlugin;

    #[test]
    fn manifest_contains_only_user_interaction_tools() {
        let manifest = InteractionPlugin::new().manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(manifest.namespace, "agena");
        assert_eq!(manifest.name, "interaction");
        assert_eq!(tool_names, ["ask", "notify"]);
        assert!(manifest.tools[1].runtime.concurrency_safe);
    }

    #[test]
    fn ask_supports_bounded_auto_resolution_and_markdown_previews() {
        let parsed = AskUserToolInput::parse_input(serde_json::json!({
            "auto_resolution_ms": 60_000,
            "questions": [{
                "id": "choice",
                "question": "Choose",
                "options": [{
                    "label": "A",
                    "preview_markdown": "  **Preview**  "
                }]
            }]
        }))
        .expect("valid ask input");
        assert_eq!(parsed.auto_resolution_ms, Some(60_000));
        assert_eq!(
            parsed.questions[0].options[0].preview_markdown,
            "**Preview**"
        );

        let err = AskUserToolInput::parse_input(serde_json::json!({
            "auto_resolution_ms": 59_999,
            "questions": [{
                "id": "choice",
                "question": "Choose",
                "allow_custom": true
            }]
        }))
        .expect_err("sub-minute auto resolution must be rejected");
        assert!(err.to_string().contains("auto_resolution_ms"));
    }

    #[test]
    fn notification_input_is_trimmed_and_requires_a_body() {
        let parsed = InteractionNotifyToolInput::parse_input(serde_json::json!({
            "title": "  Build complete  ",
            "body_markdown": "  **Done**  ",
            "level": "success"
        }))
        .expect("valid notification input");
        assert_eq!(parsed.title, "Build complete");
        assert_eq!(parsed.body_markdown, "**Done**");
        assert_eq!(parsed.level.as_str(), "success");

        assert!(
            InteractionNotifyToolInput::parse_input(serde_json::json!({
                "body_markdown": "   "
            }))
            .is_err()
        );
    }

    #[test]
    fn notification_output_keeps_markdown_and_tui_severity() {
        let input = InteractionNotifyToolInput::parse_input(serde_json::json!({
            "title": "Release",
            "body_markdown": "## Ready",
            "level": "warning"
        }))
        .expect("valid notification input");
        let output = InteractionPlugin::new()
            .notify(&input)
            .expect("notification output");
        assert_eq!(output.title, "Release");
        assert_eq!(output.output_text, "## Ready");
        assert_eq!(
            output.metadata.get("agena.notification.level"),
            Some(&"warning".to_string())
        );
        assert_eq!(
            output
                .payload
                .as_ref()
                .and_then(|payload| payload.get("level"))
                .and_then(serde_json::Value::as_str),
            Some("warning")
        );
    }
}
