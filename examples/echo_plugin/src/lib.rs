//! Demonstration plugin for the new agena plugin SDK.
//!
//! Implements every relevant hook surface in a tiny amount of code, then
//! exports itself as a cdylib via `#[agena_plugin(..., export = cdylib)]`. The same `Plugin` impl
//! could be exported as stdio (`export_stdio!`) or HTTP (`export_http!`)
//! by enabling the corresponding feature on `agena-plugin-sdk`.

use agena_plugin_sdk::prelude::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct EchoPluginConfig {
    uppercase: bool,
}

#[derive(Default, PluginConfigStore)]
pub struct EchoPlugin {
    #[config(default)]
    config: PluginConfig<EchoPluginConfig>,
}

#[agena_plugin(
    namespace = "example",
    name = "echo",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Sample plugin: echo + before/after/shell hooks.",
    config,
    display = compact,
    export = cdylib
)]
impl EchoPlugin {
    fn uppercase(&self) -> bool {
        self.config.get().is_some_and(|config| config.uppercase)
    }

    #[tool(
        name = "echo",
        summary = "Echo text back to the caller.",
        read_only,
        concurrency_safe
    )]
    async fn echo(&self, #[arg(trim, non_empty)] text: String) -> Result<ToolInvokeOutput> {
        let rendered = if self.uppercase() {
            text.to_uppercase()
        } else {
            text
        };

        Ok(ToolInvokeOutput {
            title: "Echo".to_string(),
            output_text: rendered.clone(),
            payload: Some(json!({ "rendered": rendered })),
            metadata: BTreeMap::from([("plugin".to_string(), "echo".to_string())]),
            attachments: Vec::new(),
        })
    }

    #[hook(tool.before, tool = "echo")]
    async fn tool_execute_before(&self, input: ToolBeforeInput) -> Option<ToolBeforePatch> {
        if input.tool_name() != "echo" {
            return None;
        }
        let mut new_input = input.input.clone();
        if let Some(text) = new_input.get_mut("text")
            && let Some(s) = text.as_str()
        {
            *text = serde_json::Value::String(format!("[prepared] {s}"));
        }
        Some(ToolBeforePatch {
            input: Some(new_input),
            title_override: Some("Echo (prepared)".into()),
            ..Default::default()
        })
    }

    #[hook(tool.after, tool = "echo")]
    async fn tool_execute_after(&self, input: ToolAfterInput) -> Option<ToolAfterPatch> {
        if input.tool_name() != "echo" {
            return None;
        }
        Some(ToolAfterPatch {
            output_text: Some(format!("{}\n[echo after-hook]", input.output_text)),
            metadata: BTreeMap::from([("after_hook".to_string(), "applied".to_string())]),
            ..Default::default()
        })
    }

    #[hook(shell.env)]
    async fn shell_env(&self, input: ShellEnvInput) -> ShellEnvPatch {
        let mut p = ShellEnvPatch::default();
        p.set.insert("AGENA_ECHO".into(), "1".into());
        p.set.insert(
            "AGENA_ECHO_CWD".into(),
            input.cwd.to_string_lossy().to_string(),
        );
        p
    }

    #[hook(event)]
    async fn event(&self, _ev: EventEnvelope) {}

    #[hook(run.pre)]
    async fn pre_run(&self, _input: PreRunInput) {}

    #[hook(run.post)]
    async fn post_run(&self, _input: PostRunInput) {}

    #[hook(permission.ask)]
    async fn permission_ask(&self, _input: PermissionAskInput) -> Option<PermissionAskDecision> {
        None
    }

    #[hook(session.start)]
    async fn session_start(&self, input: SessionStartInput) -> SessionStartPatch {
        SessionStartPatch {
            additional_context: Some(format!(
                "Echo plugin attached to session {}",
                input.session_id
            )),
            initial_user_message: None,
        }
    }

    #[hook(session.end)]
    async fn session_end(&self, _input: SessionEndInput) {}

    #[hook(provider.list)]
    async fn provider_list(&self, input: ProviderListInput) -> Option<ProviderListPatch> {
        let already_registered = input
            .current
            .iter()
            .any(|provider| provider.id == "echo-mock");
        if already_registered {
            return None;
        }

        Some(ProviderListPatch {
            add: vec![ProviderDescriptor {
                id: "echo-mock".to_string(),
                display_name: "Echo Mock".to_string(),
                models: vec!["echo-mock-model".to_string()],
                endpoint: None,
                kind: ProviderKind::Custom,
            }],
            remove: Vec::new(),
        })
    }
}
