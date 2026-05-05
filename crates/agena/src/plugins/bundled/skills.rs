use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::entry::{BuiltinExecution, ToolExecutionView};
use crate::message::{BuiltinToolOutput, SkillRunToolInput};
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostSkillGetRequest};
use crate::plugin::sdk::{
    EntryBehavior as SdkEntryBehavior, HookSubscription, HostCapability, InitContext, InitOutcome,
    Plugin, PluginEntryDecl, PluginManifest, Result as SdkResult, ToolInvokeInput,
    ToolInvokeOutput,
};

pub(crate) const SKILLS_PLUGIN_ID: &str = "agena.skills";

pub(crate) struct SkillsPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
}

impl SkillsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("skills plugin host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("skills plugin invoked before init"))
    }
}

#[async_trait]
impl Plugin for SkillsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-skills", env!("CARGO_PKG_VERSION"))
            .description("Agena skills exposed as first-party plugin entries.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .entry(skill_run_decl())
            .build()
    }

    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("skills plugin host lock poisoned"))? = Some(host);
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "skill_run" {
            return Err(PluginError::invalid_params(format!(
                "unknown skills plugin entry '{}'",
                input.tool_name
            )));
        }
        let input: SkillRunToolInput = serde_json::from_value(input.input)?;
        let skill = self
            .host()?
            .skill_get(HostSkillGetRequest {
                name: input.name.clone(),
            })
            .await?;

        let mut body = skill.body.clone();
        if let Some(args) = input.args.as_deref() {
            let trimmed = args.trim();
            if !trimmed.is_empty() {
                body.push_str("\n\n# User-supplied arguments\n\n");
                body.push_str(trimmed);
                body.push('\n');
            }
        }

        let output = BuiltinToolOutput::SkillRun {
            name: skill.name.clone(),
            body_chars: body.chars().count(),
            allowed_tools: skill.allowed_tools,
            model: skill.model,
        };
        let view = ToolExecutionView::simple(format!("skill_run: {}", skill.name), body);
        Ok(crate::plugins::bundled::builtin::builtin_to_invoke_output(
            BuiltinExecution::new(output, view),
        ))
    }
}

pub(crate) fn skill_run_decl() -> PluginEntryDecl {
    PluginEntryDecl::new(
        "skill_run",
        crate::entry::definition::json_schema_for::<SkillRunToolInput>(),
    )
    .description("Run a discovered or bundled skill by name. Returns the skill's system body so the model can follow it on subsequent turns.")
    .behavior(SdkEntryBehavior::ReadOnly)
    .search_terms(["skill", "workflow", "macro", "preset"])
    .always_load()
    .host_capability(HostCapability::SkillsManager)
}

#[cfg(test)]
mod tests {
    use crate::message::BuiltinToolOutput;
    use crate::plugin::sdk::host_api::{
        EventSubscription, HostSkillGetResponse, LogLevel, ToolDescriptor,
    };
    use crate::plugin::sdk::{
        EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision, ToolInvokeOutput,
    };

    use super::*;

    struct TestHost;

    #[async_trait]
    impl HostClient for TestHost {
        async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

        async fn publish_event(&self, _env: EventEnvelope) -> SdkResult<()> {
            Ok(())
        }

        async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
            Ok(EventSubscription { id: "sub".into() })
        }

        async fn ask_permission(&self, _req: PermissionAskInput) -> SdkResult<PermissionDecision> {
            Ok(PermissionDecision::Prompt)
        }

        async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }

        async fn invoke_tool(
            &self,
            tool: String,
            _input: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            Err(PluginError::new(format!(
                "unexpected invoke_tool for {tool}"
            )))
        }

        async fn list_tools(&self) -> SdkResult<Vec<ToolDescriptor>> {
            Ok(Vec::new())
        }

        async fn skill_get(&self, req: HostSkillGetRequest) -> SdkResult<HostSkillGetResponse> {
            assert_eq!(req.name, "demo");
            Ok(HostSkillGetResponse {
                name: "demo".to_string(),
                body: "Follow these instructions.".to_string(),
                allowed_tools: vec!["read".to_string()],
                model: Some("claude-sonnet-4-6".to_string()),
            })
        }
    }

    #[tokio::test]
    async fn skill_run_appends_args_and_preserves_allowed_tools() {
        let plugin = SkillsPlugin::new();
        plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: "/tmp".into(),
                    plugin_id: SKILLS_PLUGIN_ID.to_string(),
                    host_callback_url: None,
                    host_callback_token: None,
                    options: serde_json::Value::Null,
                    protocol_version: crate::plugin::sdk::rpc::PROTOCOL_VERSION,
                },
                Arc::new(TestHost),
            )
            .await
            .expect("init");

        let output = plugin
            .tool_invoke(ToolInvokeInput {
                tool_name: "skill_run".to_string(),
                session_id: 1,
                call_id: 2,
                workspace_root: "/tmp".to_string(),
                input: serde_json::json!({ "name": "demo", "args": "extra context" }),
            })
            .await
            .expect("skill_run");

        assert_eq!(output.title, "skill_run: demo");
        assert!(output.output_text.contains("Follow these instructions."));
        assert!(output.output_text.contains("# User-supplied arguments"));
        assert!(output.output_text.contains("extra context"));

        let envelope =
            crate::plugins::bundled::builtin::payload_to_builtin_envelope(output.payload.as_ref())
                .expect("builtin envelope");
        match envelope.output {
            BuiltinToolOutput::SkillRun {
                name,
                body_chars,
                allowed_tools,
                model,
            } => {
                assert_eq!(name, "demo");
                assert_eq!(body_chars, output.output_text.chars().count());
                assert_eq!(allowed_tools, vec!["read".to_string()]);
                assert_eq!(model.as_deref(), Some("claude-sonnet-4-6"));
            }
            other => panic!("unexpected output: {other:?}"),
        }
    }
}
