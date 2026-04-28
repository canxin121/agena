//! Demonstration plugin for the new agena plugin SDK.
//!
//! Implements every relevant hook surface in a tiny amount of code, then
//! exports itself as a cdylib via `export_cdylib!`. The same `Plugin` impl
//! could be exported as stdio (`export_stdio!`) or HTTP (`export_http!`)
//! by enabling the corresponding feature on `agena-plugin-sdk`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use agena_plugin_sdk::prelude::*;

#[derive(Default)]
pub struct EchoPlugin {
    uppercase: AtomicBool,
}

#[async_trait]
impl Plugin for EchoPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("echo", env!("CARGO_PKG_VERSION"))
            .description("Sample plugin: echo + before/after/shell hooks.")
            .hooks(
                HookSubscription::INIT
                    | HookSubscription::TOOL_INVOKE
                    | HookSubscription::TOOL_BEFORE
                    | HookSubscription::TOOL_AFTER
                    | HookSubscription::SHELL_ENV
                    | HookSubscription::EVENT,
            )
            .tool(
                ToolDecl::new(
                    "echo",
                    json!({
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" }
                        },
                        "required": ["text"]
                    }),
                )
                .description("Echo the supplied text.")
                .behavior(ToolBehavior::ReadOnly),
            )
            .build()
    }

    async fn init(
        &self,
        ctx: InitContext,
        _host: Arc<dyn HostClient>,
    ) -> Result<InitOutcome> {
        if let Some(b) = ctx.options.get("uppercase").and_then(|v| v.as_bool()) {
            self.uppercase.store(b, Ordering::Relaxed);
        }
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        let text = input
            .input
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::invalid_params("missing `text`"))?
            .to_string();

        let rendered = if self.uppercase.load(Ordering::Relaxed) {
            text.to_uppercase()
        } else {
            text
        };

        Ok(ToolInvokeOutput {
            title: "Echo".to_string(),
            output_text: rendered.clone(),
            payload: Some(json!({ "rendered": rendered })),
            metadata: BTreeMap::from([("plugin".to_string(), "echo".to_string())]),
        })
    }

    async fn tool_execute_before(
        &self,
        input: ToolBeforeInput,
    ) -> Result<Option<ToolBeforePatch>> {
        if input.tool_name != "echo" {
            return Ok(None);
        }
        let mut new_input = input.input.clone();
        if let Some(text) = new_input.get_mut("text") {
            if let Some(s) = text.as_str() {
                *text = serde_json::Value::String(format!("[prepared] {s}"));
            }
        }
        Ok(Some(ToolBeforePatch {
            input: Some(new_input),
            title_override: Some("Echo (prepared)".into()),
            ..Default::default()
        }))
    }

    async fn tool_execute_after(
        &self,
        input: ToolAfterInput,
    ) -> Result<Option<ToolAfterPatch>> {
        if input.tool_name != "echo" {
            return Ok(None);
        }
        Ok(Some(ToolAfterPatch {
            output_text: Some(format!("{}\n[echo after-hook]", input.output_text)),
            metadata: BTreeMap::from([(
                "after_hook".to_string(),
                "applied".to_string(),
            )]),
            ..Default::default()
        }))
    }

    async fn shell_env(&self, input: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        let mut p = ShellEnvPatch::default();
        p.set
            .insert("AGENA_ECHO".into(), "1".into());
        p.set.insert(
            "AGENA_ECHO_CWD".into(),
            input.cwd.to_string_lossy().to_string(),
        );
        Ok(Some(p))
    }

    async fn event(&self, _ev: EventEnvelope) -> Result<()> {
        // We could log to host via `host.log(...)`. Skipped for brevity.
        Ok(())
    }
}

agena_plugin_sdk::export_cdylib!(EchoPlugin);
