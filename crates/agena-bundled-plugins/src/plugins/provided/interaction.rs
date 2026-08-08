use std::sync::Arc;

use crate::message::{AskUserToolInput, InteractionNotifyToolInput};
use crate::plugins::provided::workflow::{WorkflowPlugin, WorkflowPluginConfig};
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput};

pub(crate) const INTERACTION_PLUGIN_ID: &str = "agena.interaction";

pub(crate) struct InteractionPlugin {
    inner: WorkflowPlugin,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "interaction",
    version = env!("CARGO_PKG_VERSION"),
    summary = "User interaction tools.",
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
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(interactive),
        summary = "Ask the user for short structured input.",
        help = "Use only when you are blocked on a decision that belongs to the user: a preference, a direction choice, or a choice with no reasonable default. If a sensible default exists or you can verify the answer yourself, proceed instead of asking. Ask all necessary clarifying questions at once. Never use this tool to ask whether you should proceed or to seek plan approval.",
        interactive,

    )]
    async fn ask(&self, input: &AskUserToolInput) -> SdkResult<ToolInvokeOutput> {
        self.inner.invoke_ask_user(input).await
    }

    #[tool(
        tags(interactive),
        summary = "Show a non-blocking Markdown notification to the user.",
        concurrency_safe
    )]
    fn notify(&self, input: &InteractionNotifyToolInput) -> SdkResult<ToolInvokeOutput> {
        let input = InteractionNotifyToolInput::parse_input(serde_json::to_value(input).map_err(
            |err| agena_plugin_host::sdk::PluginError::invalid_params(err.to_string()),
        )?)?;
        let level = input.level.as_str();
        let title = if input.title.is_empty() {
            match input.level {
                agena_domain::InteractionNotificationLevel::Info => "Notice",
                agena_domain::InteractionNotificationLevel::Success => "Completed",
                agena_domain::InteractionNotificationLevel::Warning => "Attention",
                agena_domain::InteractionNotificationLevel::Error => "Error",
            }
            .to_string()
        } else {
            input.title
        };
        Ok(ToolInvokeOutput::from_parts(
            title.clone(),
            format!("{level} notification"),
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
    use agena_plugin_host::sdk::Plugin;

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
    fn ask_supports_bounded_auto_resolution_without_ids_or_previews() {
        let parsed = AskUserToolInput::parse_input(serde_json::json!({
            "auto_resolution_ms": 60_000,
            "questions": [{
                "question": "Choose",
                "options": [{
                    "label": "A"
                }]
            }]
        }))
        .expect("valid ask input");
        assert_eq!(parsed.auto_resolution_ms, Some(60_000));
        assert_eq!(parsed.questions[0].options[0].label, "A");

        let err = AskUserToolInput::parse_input(serde_json::json!({
            "auto_resolution_ms": 59_999,
            "questions": [{
                "question": "Choose",
                "allow_custom": true
            }]
        }))
        .expect_err("sub-minute auto resolution must be rejected");
        assert!(err.diagnostic_message().contains("auto_resolution_ms"));
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
