//! Minimal MCP fixture used by the DSV4F gateway integration suite.
//!
//! It intentionally has no Agena runtime dependency: tests can launch it as a
//! child MCP server and exercise the resource, prompt, and tool bridge paths
//! independently of the child server's configured plugins.

use std::collections::BTreeMap;

use agena_mcp_client::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult, PromptArgument,
    PromptDescriptor, PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents,
    ResourceDescriptor, ToolDescriptor,
};
use agena_mcp_server::{McpServerBackend, McpServerError};
use async_trait::async_trait;

const RESOURCE_URI: &str = "fixture://hello";
const PROMPT_NAME: &str = "probe";
const TOOL_NAME: &str = "echo";

#[derive(Debug, Default)]
struct FixtureBackend;

#[async_trait]
impl McpServerBackend for FixtureBackend {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError> {
        Ok(vec![ToolDescriptor {
            name: TOOL_NAME.to_string(),
            aliases: Vec::new(),
            description: Some("Return a deterministic fixture echo response.".to_string()),
            before_help: None,
            after_help: None,
            input_schema: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "value": {
                        "type": "string",
                        "description": "Value included in the fixture response."
                    }
                },
                "required": ["value"],
                "additionalProperties": false
            })),
        }])
    }

    async fn call_tool(&self, params: CallToolParams) -> Result<CallToolResult, McpServerError> {
        if params.name != TOOL_NAME {
            return Err(McpServerError::NotFound(params.name));
        }

        let value = params
            .arguments
            .as_ref()
            .and_then(serde_json::Value::as_object)
            .and_then(|arguments| arguments.get("value"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                McpServerError::InvalidParams(
                    "fixture tool `echo` requires a string `value` argument".to_string(),
                )
            })?;

        Ok(agena_mcp_server::text_result(format!(
            "MCP_ECHO_OK: {value}"
        )))
    }

    async fn list_resources(&self) -> Result<Vec<ResourceDescriptor>, McpServerError> {
        Ok(vec![ResourceDescriptor {
            uri: RESOURCE_URI.to_string(),
            name: Some("Fixture hello".to_string()),
            description: Some("Deterministic MCP resource fixture.".to_string()),
            mime_type: Some("text/plain".to_string()),
        }])
    }

    async fn read_resource(
        &self,
        params: ReadResourceParams,
    ) -> Result<ReadResourceResult, McpServerError> {
        if params.uri != RESOURCE_URI {
            return Err(McpServerError::NotFound(params.uri));
        }

        Ok(ReadResourceResult {
            contents: vec![ResourceContents {
                uri: RESOURCE_URI.to_string(),
                mime_type: Some("text/plain".to_string()),
                text: Some("MCP_RESOURCE_OK".to_string()),
                blob: None,
            }],
        })
    }

    async fn list_prompts(&self) -> Result<Vec<PromptDescriptor>, McpServerError> {
        Ok(vec![PromptDescriptor {
            name: PROMPT_NAME.to_string(),
            description: Some("Return a deterministic fixture prompt.".to_string()),
            arguments: vec![PromptArgument {
                name: "name".to_string(),
                description: Some("Optional name included in the fixture prompt.".to_string()),
                required: false,
            }],
        }])
    }

    async fn get_prompt(&self, params: GetPromptParams) -> Result<GetPromptResult, McpServerError> {
        if params.name != PROMPT_NAME {
            return Err(McpServerError::NotFound(params.name));
        }

        let name = optional_prompt_name(params.arguments)?;
        let text = name.map_or_else(
            || "MCP_PROMPT_OK".to_string(),
            |name| format!("MCP_PROMPT_OK: {name}"),
        );
        Ok(GetPromptResult {
            description: Some("Deterministic MCP fixture prompt.".to_string()),
            messages: vec![PromptMessage {
                role: "user".to_string(),
                content: ContentBlock::Text { text },
            }],
        })
    }
}

fn optional_prompt_name(
    arguments: Option<BTreeMap<String, String>>,
) -> Result<Option<String>, McpServerError> {
    let Some(mut arguments) = arguments else {
        return Ok(None);
    };
    let name = arguments.remove("name");
    if arguments.is_empty() {
        Ok(name)
    } else {
        Err(McpServerError::InvalidParams(
            "fixture prompt `probe` accepts only the optional `name` argument".to_string(),
        ))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    agena_mcp_server::serve_stdio(FixtureBackend).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use agena_mcp_client::{CallToolParams, ContentBlock, GetPromptParams, ReadResourceParams};

    use super::{FixtureBackend, McpServerBackend, PROMPT_NAME, RESOURCE_URI, TOOL_NAME};

    #[tokio::test]
    async fn fixture_advertises_and_reads_its_resource() {
        let backend = FixtureBackend;
        let resources = backend.list_resources().await.expect("list resources");
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].uri, RESOURCE_URI);

        let result = backend
            .read_resource(ReadResourceParams {
                uri: RESOURCE_URI.to_string(),
            })
            .await
            .expect("read fixture resource");
        assert_eq!(result.contents[0].text.as_deref(), Some("MCP_RESOURCE_OK"));
    }

    #[tokio::test]
    async fn fixture_prompt_accepts_its_optional_name() {
        let backend = FixtureBackend;
        let prompts = backend.list_prompts().await.expect("list prompts");
        assert_eq!(prompts[0].name, PROMPT_NAME);
        assert_eq!(prompts[0].arguments[0].name, "name");
        assert!(!prompts[0].arguments[0].required);

        let default_prompt = backend
            .get_prompt(GetPromptParams {
                name: PROMPT_NAME.to_string(),
                arguments: None,
            })
            .await
            .expect("get fixture prompt without optional name");
        let ContentBlock::Text { text } = &default_prompt.messages[0].content else {
            panic!("fixture prompt must contain text");
        };
        assert_eq!(text, "MCP_PROMPT_OK");

        let prompt = backend
            .get_prompt(GetPromptParams {
                name: PROMPT_NAME.to_string(),
                arguments: Some(BTreeMap::from([(
                    String::from("name"),
                    String::from("Agena"),
                )])),
            })
            .await
            .expect("get fixture prompt");
        let ContentBlock::Text { text } = &prompt.messages[0].content else {
            panic!("fixture prompt must contain text");
        };
        assert_eq!(text, "MCP_PROMPT_OK: Agena");
    }

    #[tokio::test]
    async fn fixture_echo_returns_the_deterministic_marker() {
        let backend = FixtureBackend;
        let tools = backend.list_tools().await.expect("list tools");
        assert_eq!(tools[0].name, TOOL_NAME);

        let result = backend
            .call_tool(CallToolParams {
                name: TOOL_NAME.to_string(),
                arguments: Some(serde_json::json!({"value": "hello"})),
            })
            .await
            .expect("call fixture echo");
        let ContentBlock::Text { text } = &result.content[0] else {
            panic!("fixture tool must return text");
        };
        assert_eq!(text, "MCP_ECHO_OK: hello");
    }
}
