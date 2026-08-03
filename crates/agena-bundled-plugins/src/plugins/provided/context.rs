//! Safe model-context budget, model identity, and compaction status.

use std::sync::{Arc, OnceLock};

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::{HostClient, HostContextStatusRequest};
use agena_plugin_host::sdk::{InitContext, InitOutcome, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};

pub(crate) const CONTEXT_PLUGIN_ID: &str = "agena.context";

pub(crate) struct ContextPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "context",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Safe context-window budget, model identity, and compaction status.",
    display = detailed
)]
impl ContextPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: OnceLock::new(),
        }
    }

    #[hook(init)]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("context plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        tags(query, discovery),
        summary = "Inspect remaining context budget, model identity, and compaction health without exposing prompts.",
        read_only,
        display = detailed,

        concurrency_safe
    )]
    async fn status(&self, context: &ToolInvokeContext<'_>) -> SdkResult<ToolInvokeOutput> {
        let status = self
            .host
            .get()
            .ok_or_else(|| PluginError::internal("context plugin invoked before init"))?
            .get_context_status(HostContextStatusRequest {
                session_id: Some(context.session_id),
            })
            .await?;
        let ratio = status
            .limit_tokens
            .and_then(|limit| (limit > 0).then_some(status.current_tokens as f64 / limit as f64));
        let model_identity = match (
            status.model_provider_id.as_deref(),
            status.model_adapter_id.as_deref(),
            status.model_id.as_deref(),
        ) {
            (Some(provider), Some(adapter), Some(model)) => {
                format!("{provider}/{adapter}/{model}")
            }
            (Some(provider), None, Some(model)) => format!("{provider}/{model}"),
            _ => "unknown".to_string(),
        };
        let mut model_detail = vec![format!("Model: {model_identity}")];
        if let Some(thinking) = status.thinking_mode.as_deref() {
            model_detail.push(format!("thinking: {thinking}"));
        }
        if let Some(speed) = status.speed_mode.as_deref() {
            model_detail.push(format!("speed: {speed}"));
        }
        if let Some(verbosity) = status.verbosity.as_deref() {
            model_detail.push(format!("verbosity: {verbosity}"));
        }
        let text = format!(
            "Context: {} token(s) used; limit {}; remaining {}; generation {}; compacted={}; auto_compaction_disabled={}. {}.",
            status.current_tokens,
            status
                .limit_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status
                .remaining_tokens
                .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
            status.prompt_window_generation,
            status.compacted,
            status.auto_compaction_disabled,
            model_detail.join("; "),
        );
        let payload = serde_json::json!({
            "session_id": status.session_id,
            "model_provider_id": status.model_provider_id,
            "model_adapter_id": status.model_adapter_id,
            "model_id": status.model_id,
            "thinking_mode": status.thinking_mode,
            "speed_mode": status.speed_mode,
            "verbosity": status.verbosity,
            "current_tokens": status.current_tokens,
            "measured_prompt_tokens": status.measured_prompt_tokens,
            "projected_tokens": status.projected_tokens,
            "limit_tokens": status.limit_tokens,
            "remaining_tokens": status.remaining_tokens,
            "usage_ratio": ratio,
            "reserved_tokens": status.reserved_tokens,
            "model_context_window_tokens": status.model_context_window_tokens,
            "model_max_input_tokens": status.model_max_input_tokens,
            "model_max_output_tokens": status.model_max_output_tokens,
            "prompt_window_generation": status.prompt_window_generation,
            "compacted": status.compacted,
            "last_compaction_before_tokens": status.last_compaction_before_tokens,
            "last_compaction_after_tokens": status.last_compaction_after_tokens,
            "auto_compaction_disabled": status.auto_compaction_disabled,
            "consecutive_compaction_failures": status.consecutive_compaction_failures,
        });
        Ok(ToolInvokeOutput::from_parts(
            "context status",
            status.remaining_tokens.map_or_else(
                || format!("{} tokens used", status.current_tokens),
                |remaining| format!("{} used · {remaining} remaining", status.current_tokens),
            ),
            text,
            Some(payload),
            std::collections::BTreeMap::from([
                (
                    "current_tokens".to_string(),
                    status.current_tokens.to_string(),
                ),
                (
                    "remaining_tokens".to_string(),
                    status
                        .remaining_tokens
                        .map_or_else(|| "unknown".to_string(), |value| value.to_string()),
                ),
            ]),
            Vec::new(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::Plugin;

    use super::ContextPlugin;

    #[test]
    fn manifest_exposes_safe_context_status() {
        let manifest = ContextPlugin::new().manifest();
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "status");
    }
}
