use agena_plugin_sdk::{
    ApplicableSessions, PluginError, PluginErrorKind, SessionKind, ToolContract, ToolDefinition,
    ToolDocs, ToolInvokeInput, ToolInvokeOutput, ToolKey, ToolTag,
    host_api::{HostGetSessionRequest, HostToolRegisterRequest},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{HostClient, Mods, SessionVm, bounded, valid_command_name};
use crate::agent_plugin::ToolProjection;

pub(crate) const CLAUDE_MOD_TOOL_TAG: &str = "claude-mod";
const MAX_TOOLS_PER_SESSION: usize = 128;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ToolSpec {
    name: String,
    description: String,
    input_schema: Option<Value>,
    search_hint: Option<String>,
    when_to_use: Option<String>,
    title: Option<String>,
}

impl Mods {
    pub(super) async fn register_tool(
        &self,
        host: &dyn HostClient,
        session: &SessionVm,
        spec: ToolSpec,
    ) -> Result<Value, String> {
        if !valid_command_name(&spec.name) {
            return Err("invalid Claude mod tool name".into());
        }
        if spec.description.trim().is_empty() {
            return Err("a Claude mod tool requires a description".into());
        }
        let schema = spec
            .input_schema
            .unwrap_or_else(|| json!({"type":"object"}));
        if !schema.is_object() {
            return Err("a Claude mod tool requires a JSON Schema object".into());
        }
        if let Some(properties) = schema["properties"].as_object()
            && ["tool", "tool_use_id", "agentId", "requestMeta"]
                .iter()
                .any(|key| properties.contains_key(*key))
        {
            return Err("the tool schema uses a reserved Claude event field".into());
        }
        let name = format!("mcp__{}__{}", self.name, spec.name);
        let description = bounded(spec.description.trim(), 16 * 1024);
        let help = [
            Some(description.clone()),
            spec.search_hint,
            spec.when_to_use,
        ]
        .into_iter()
        .flatten()
        .map(|text| bounded(text.trim(), 16 * 1024))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        let tool = ToolDefinition {
            name: name.clone(),
            contract: ToolContract {
                input_schema: schema,
                ..Default::default()
            },
            docs: ToolDocs {
                summary: Some(bounded(&description, 4096)),
                help: Some(help),
                ..Default::default()
            },
            runtime: Default::default(),
            // Claude treats a registered tool as potentially mutating even
            // when its author supplies isReadOnly.
            tags: vec![ToolTag::Mutate, ToolTag::Custom(CLAUDE_MOD_TOOL_TAG.into())],
            applies_to: ApplicableSessions::only(SessionKind::Project),
        };
        let mut registrations = session.tools.lock().await;
        if registrations.len() >= MAX_TOOLS_PER_SESSION && !registrations.contains_key(&name) {
            return Err("Claude mod session tool limit reached".into());
        }
        host.register_tool(HostToolRegisterRequest { tool })
            .await
            .map_err(|error| error.failure.user.fallback)?;
        let title = spec.title.and_then(|title| {
            let title = title.trim();
            (!title.is_empty() && !title.chars().any(char::is_control)).then(|| bounded(title, 64))
        });
        registrations.insert(name.clone(), title);
        Ok(json!({"tool": name}))
    }

    pub(in crate::agent_plugin) async fn invoke_registered_tool(
        &self,
        host: &dyn HostClient,
        input: ToolInvokeInput,
    ) -> agena_plugin_sdk::Result<ToolInvokeOutput> {
        let session = self
            .sessions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .get(&input.session_id)
            .cloned()
            .ok_or_else(|| unregistered(&input.tool_name))?;
        let title = session
            .tools
            .lock()
            .await
            .get(&input.tool_name)
            .cloned()
            .ok_or_else(|| unregistered(&input.tool_name))?;
        let key = ToolKey::new(self.key.clone(), input.tool_name.clone())?;
        let subagent = host
            .get_session(HostGetSessionRequest::default())
            .await
            .ok()
            .filter(|response| response.session.relation_kind.is_subagent())
            .map(|_| input.session_id);
        let event = ToolProjection::new(&key, &input.input).mod_event(
            input.call_id,
            subagent,
            &input.input,
        );
        let unhandled =
            || json!({"deny": format!("No Claude mod hook answered {}", input.tool_name)});
        let mut answer = self
            .run(
                host,
                input.session_id,
                "tool.call",
                event.clone(),
                |event| {
                    Box::pin(
                        self.run(host, input.session_id, "tool.run", event, |_| async {
                            unhandled()
                        }),
                    )
                },
            )
            .await;
        if let Some(reason) = answer["deny"].as_str() {
            return Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                bounded(reason, 8192),
            ));
        }
        let end_turn = answer["isEnd"] == Value::Bool(true);
        let content_blocks = answer
            .as_object_mut()
            .and_then(|answer| answer.remove("content"));
        let result = answer
            .as_object_mut()
            .and_then(|answer| answer.remove("result"));
        let (result_text, attachments, payload) = if let Some(Value::Array(blocks)) = content_blocks {
            let (text, attachments) =
                crate::agent_plugin::decode_content_blocks(&blocks, "Claude mod tool")?;
            let result_text = if !text.is_empty() {
                text
            } else if let Some(result) = &result {
                match result.as_str() {
                    Some(text) => bounded(text, 128 * 1024),
                    None => bounded(&result.to_string(), 128 * 1024),
                }
            } else {
                String::new()
            };
            let payload = result.filter(Value::is_object);
            (result_text, attachments, payload)
        } else {
            let result = result
                .ok_or_else(|| PluginError::internal("Claude mod returned no tool result"))?;
            let result_text = match result.as_str() {
                Some(text) => bounded(text, 128 * 1024),
                None => bounded(&result.to_string(), 128 * 1024),
            };
            (result_text, Vec::new(), result.is_object().then_some(result))
        };
        let summary = if result_text.trim().is_empty() {
            if !attachments.is_empty() {
                format!("Returned {} media attachment(s)", attachments.len())
            } else {
                format!("{} completed", input.tool_name)
            }
        } else {
            result_text.clone()
        };
        let mut response = ToolInvokeOutput::from_parts(
            title.unwrap_or(input.tool_name),
            summary,
            result_text,
            payload,
            Default::default(),
            attachments,
        );
        if end_turn {
            response
                .metadata
                .insert(agena_plugin_sdk::TOOL_END_TURN.into(), "true".into());
        }
        if let Some(context) = answer["context"].as_array() {
            let mut remaining: usize = 16 * 1024;
            response.additional_context = context
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map_while(|value| {
                    let text = bounded(value, remaining);
                    remaining = remaining.saturating_sub(text.len() + 1);
                    (!text.is_empty()).then_some(text)
                })
                .collect();
        }
        Ok(response)
    }
}

fn unregistered(name: &str) -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::ToolUnavailable,
        format!("Claude mod tool {name} is not registered in this session"),
    )
}
