use super::{ToolError, ToolExecutor};
use agena_domain::{AdapterId, CapabilitySourceKind, CapabilityUnavailableResult};

impl ToolExecutor {
    /// Opt into the AI invocation guard. None means an unresolved model
    /// adapter and fails closed for cloud tools only. A fresh host executor
    /// has no guard, preserving explicit non-model application operations.
    pub fn with_cloud_tool_adapter(mut self, adapter: Option<AdapterId>) -> Self {
        self.cloud_tool_adapter_gate =
            Some(crate::tool::tool_registry::CloudToolAdapterGate { adapter });
        self
    }

    pub(super) fn check_cloud_tool_adapter(&self, canonical_tool: &str) -> Result<(), ToolError> {
        let Some(gate) = &self.cloud_tool_adapter_gate else {
            return Ok(());
        };
        let Some(cloud) = agena_tool::provider_tools::cloud_tool(canonical_tool) else {
            return Ok(());
        };
        // The caller's protocol and the cloud plugin's outbound protocol are
        // separate: Chat Completions can call Agena functions even though it
        // cannot accept Responses-native hosted tool definitions.
        let required: &[&str] = match cloud.provider {
            "chatgpt" => &["openai_responses", "openai_chat_completions"],
            "claude" => &["anthropic"],
            "gemini" => &["gemini"],
            _ => {
                return Err(ToolError::invalid_input(
                    "unknown cloud-tool adapter requirement",
                ));
            }
        };
        let actual = gate.adapter.as_ref().map(AsRef::<str>::as_ref);
        if actual.is_some_and(|adapter| required.contains(&adapter)) {
            return Ok(());
        }
        Err(ToolError::CapabilityUnavailable(Box::new(
            CapabilityUnavailableResult {
                capability: "provider_tool_adapter".into(),
                tool_name: Some(canonical_tool.into()),
                reason: format!(
                    "Cloud tool '{}' requires one of adapters [{}]; the active model adapter is '{}'. No provider request was sent and no adapter was switched.",
                    cloud.name,
                    required.join(", "),
                    actual.unwrap_or("unresolved")
                ),
                source: CapabilitySourceKind::RuntimeConfiguration,
                retryable: false,
            },
        )))
    }
}
