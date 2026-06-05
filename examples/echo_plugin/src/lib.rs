//! Demonstration plugin for the new agena plugin SDK.
//!
//! Implements every relevant hook surface in a tiny amount of code, then
//! exports itself as a cdylib via `export_cdylib!`. The same `Plugin` impl
//! could be exported as stdio (`export_stdio!`) or HTTP (`export_http!`)
//! by enabling the corresponding feature on `agena-plugin-sdk`.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct EchoPluginConfig {
    uppercase: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolCommand)]
#[tool_command(
    tool = "echo",
    description = "Echo the supplied text.",
    summary = "Echo text back to the caller.",
    trim("text"),
    non_empty("text"),
    tags(ToolTag::ReadOnly),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct EchoToolInput {
    /// Text payload to echo back.
    text: String,
}

#[derive(Default)]
pub struct EchoPlugin {
    config: OnceLock<EchoPluginConfig>,
}

#[plugin(
    id = "echo",
    version = env!("CARGO_PKG_VERSION"),
    description = "Sample plugin: echo + before/after/shell hooks.",
    config_schema_type = EchoPluginConfig,
    config_schema_default = default,
    display = compact,
    export = cdylib
)]
impl EchoPlugin {
    fn uppercase(&self) -> bool {
        self.config.get().is_some_and(|config| config.uppercase)
    }

    #[tool]
    async fn invoke_echo(&self, input: EchoToolInput) -> Result<ToolInvokeOutput> {
        let rendered = if self.uppercase() {
            input.text.to_uppercase()
        } else {
            input.text
        };

        Ok(ToolInvokeOutput {
            title: "Echo".to_string(),
            output_text: rendered.clone(),
            payload: Some(json!({ "rendered": rendered })),
            metadata: BTreeMap::from([("plugin".to_string(), "echo".to_string())]),
            attachments: Vec::new(),
        })
    }

    #[hook]
    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        let config = agena_plugin_sdk::macro_support::parse_defaulted_config(
            ctx.config,
            "invalid echo config",
        )?;
        self.config
            .set(config)
            .map_err(|_| PluginError::invalid_params("echo plugin config already initialized"))?;
        Ok(InitOutcome::ack(Plugin::manifest(self)))
    }

    #[hook]
    async fn tool_execute_before(&self, input: ToolBeforeInput) -> Result<Option<ToolBeforePatch>> {
        if input.tool_name != "echo" {
            return Ok(None);
        }
        let mut new_input = input.input.clone();
        if let Some(text) = new_input.get_mut("text")
            && let Some(s) = text.as_str()
        {
            *text = serde_json::Value::String(format!("[prepared] {s}"));
        }
        Ok(Some(ToolBeforePatch {
            input: Some(new_input),
            title_override: Some("Echo (prepared)".into()),
            ..Default::default()
        }))
    }

    #[hook]
    async fn tool_execute_after(&self, input: ToolAfterInput) -> Result<Option<ToolAfterPatch>> {
        if input.tool_name != "echo" {
            return Ok(None);
        }
        Ok(Some(ToolAfterPatch {
            output_text: Some(format!("{}\n[echo after-hook]", input.output_text)),
            metadata: BTreeMap::from([("after_hook".to_string(), "applied".to_string())]),
            ..Default::default()
        }))
    }

    #[hook]
    async fn shell_env(&self, input: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        let mut p = ShellEnvPatch::default();
        p.set.insert("AGENA_ECHO".into(), "1".into());
        p.set.insert(
            "AGENA_ECHO_CWD".into(),
            input.cwd.to_string_lossy().to_string(),
        );
        Ok(Some(p))
    }

    #[hook]
    async fn event(&self, _ev: EventEnvelope) -> Result<()> {
        Ok(())
    }

    #[hook]
    async fn pre_run(&self, _input: PreRunInput) -> Result<()> {
        Ok(())
    }

    #[hook]
    async fn post_run(&self, _input: PostRunInput) -> Result<()> {
        Ok(())
    }

    #[hook]
    async fn permission_ask(
        &self,
        _input: PermissionAskInput,
    ) -> Result<Option<PermissionAskDecision>> {
        Ok(None)
    }

    #[hook]
    async fn session_start(&self, input: SessionStartInput) -> Result<Option<SessionStartPatch>> {
        Ok(Some(SessionStartPatch {
            additional_context: Some(format!(
                "Echo plugin attached to session {}",
                input.session_id
            )),
            initial_user_message: None,
        }))
    }

    #[hook]
    async fn session_end(&self, _input: SessionEndInput) -> Result<()> {
        Ok(())
    }

    #[hook]
    async fn provider_list(&self, input: ProviderListInput) -> Result<Option<ProviderListPatch>> {
        let already_registered = input
            .current
            .iter()
            .any(|provider| provider.id == "echo-mock");
        if already_registered {
            return Ok(None);
        }

        Ok(Some(ProviderListPatch {
            add: vec![ProviderDescriptor {
                id: "echo-mock".to_string(),
                display_name: "Echo Mock".to_string(),
                models: vec!["echo-mock-model".to_string()],
                endpoint: None,
                kind: ProviderKind::Custom,
            }],
            remove: Vec::new(),
        }))
    }
}
