use std::collections::BTreeMap;

use agena::plugin::{
    AgenaPlugin, PluginAfterToolRequest, PluginAfterToolResponse, PluginBeforeToolRequest,
    PluginBeforeToolResponse, PluginError, PluginMetadata, PluginShellEnvRequest,
    PluginShellEnvResponse, PluginToolCallRequest, PluginToolCallResponse, PluginToolDescriptor,
};
use agena::tool::ToolBehavior;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize, Serialize)]
struct EchoPlusInput {
    message: String,
    #[serde(default)]
    uppercase: bool,
    #[serde(default)]
    tags: Vec<String>,
}

struct EchoPlusPlugin;

impl AgenaPlugin for EchoPlusPlugin {
    fn metadata(&self) -> PluginMetadata {
        PluginMetadata {
            name: "echo_plus".to_string(),
            version: "0.1.0".to_string(),
            description: "Sample Agena plugin with a custom tool and lifecycle hooks.".to_string(),
        }
    }

    fn tools(&self) -> Vec<PluginToolDescriptor> {
        vec![PluginToolDescriptor {
            name: "echo_plus".to_string(),
            description: "Echo input text with optional uppercase and tags.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" },
                    "uppercase": { "type": "boolean", "default": false },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "default": []
                    }
                },
                "required": ["message"]
            }),
            behavior: ToolBehavior::ReadOnly,
        }]
    }

    fn invoke_tool(
        &self,
        request: PluginToolCallRequest,
    ) -> Result<PluginToolCallResponse, PluginError> {
        let input: EchoPlusInput = serde_json::from_str(request.input_json.as_str())
            .map_err(|err| PluginError::new(format!("invalid echo_plus input: {err}")))?;

        let mut rendered = input.message;
        if input.uppercase {
            rendered = rendered.to_uppercase();
        }
        if !input.tags.is_empty() {
            rendered.push_str(" [tags:");
            rendered.push_str(input.tags.join(",").as_str());
            rendered.push(']');
        }

        Ok(PluginToolCallResponse {
            title: "Echo Plus".to_string(),
            output_text: rendered.clone(),
            payload_json: Some(
                json!({
                    "rendered": rendered,
                    "workspace_root": request.workspace_root,
                    "session_id": request.session_id,
                    "call_id": request.call_id
                })
                .to_string(),
            ),
            metadata: BTreeMap::from([
                ("plugin".to_string(), "echo_plus".to_string()),
                ("tool".to_string(), request.tool_name),
            ]),
        })
    }

    fn before_tool(
        &self,
        request: PluginBeforeToolRequest,
    ) -> Result<PluginBeforeToolResponse, PluginError> {
        if request.tool_name != "echo_plus" {
            return Ok(PluginBeforeToolResponse::passthrough(request.input_json));
        }

        let mut input: EchoPlusInput = serde_json::from_str(request.input_json.as_str())
            .map_err(|err| PluginError::new(format!("invalid before_tool input: {err}")))?;
        if !input.message.starts_with("[prepared] ") {
            input.message = format!("[prepared] {}", input.message);
        }

        Ok(PluginBeforeToolResponse {
            input_json: serde_json::to_string(&input)
                .map_err(|err| PluginError::new(format!("failed to serialize input: {err}")))?,
            title_override: Some("Echo Plus (prepared)".to_string()),
            metadata: BTreeMap::from([("before_hook".to_string(), "applied".to_string())]),
        })
    }

    fn after_tool(
        &self,
        request: PluginAfterToolRequest,
    ) -> Result<PluginAfterToolResponse, PluginError> {
        if request.tool_name != "echo_plus" {
            return Ok(PluginAfterToolResponse::default());
        }

        let mut payload = request
            .payload_json
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .map_err(|err| PluginError::new(format!("invalid after_tool payload: {err}")))?
            .unwrap_or_else(|| json!({}));
        payload["after_hook"] = json!(true);

        Ok(PluginAfterToolResponse {
            title: Some(format!("{} (postprocessed)", request.title)),
            output_text: Some(format!(
                "{}\n\n[echo_plus after hook applied]",
                request.output_text
            )),
            payload_json: Some(payload.to_string()),
            metadata: BTreeMap::from([("after_hook".to_string(), "applied".to_string())]),
        })
    }

    fn shell_env(
        &self,
        request: PluginShellEnvRequest,
    ) -> Result<PluginShellEnvResponse, PluginError> {
        Ok(PluginShellEnvResponse {
            env: BTreeMap::from([
                ("AGENA_SAMPLE_PLUGIN".to_string(), "echo_plus".to_string()),
                ("AGENA_SAMPLE_PLUGIN_CWD".to_string(), request.cwd),
            ]),
        })
    }
}

agena::export_agena_plugin!(EchoPlusPlugin);
