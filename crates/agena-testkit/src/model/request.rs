use anyhow::{Context, Result, ensure};
use serde_json::Value;

use super::{ModelProtocol, ModelRequest, bounded_text, collect_text};

impl ModelRequest {
    pub fn protocol(&self) -> Option<ModelProtocol> {
        self.route_protocol
            .or_else(|| ModelProtocol::from_path(&self.path))
    }

    /// Exact URI as seen on the wire, including query parameters. This may
    /// contain credentials; avoid logging it outside a deliberate assertion.
    pub fn raw_uri(&self) -> &str {
        &self.raw_uri
    }

    /// Ordered URL-decoded query parameters. Duplicate names stay distinct.
    /// The request's Debug output does not print these sensitive values.
    pub fn query_pairs(&self) -> &[(String, String)] {
        &self.query_pairs
    }

    pub fn query_values(&self, name: &str) -> Vec<&str> {
        self.query_pairs
            .iter()
            .filter_map(|(key, value)| (key == name).then_some(value.as_str()))
            .collect()
    }

    pub fn streaming(&self) -> bool {
        match self.protocol() {
            Some(ModelProtocol::Gemini) => self.path.ends_with(":streamGenerateContent"),
            Some(ModelProtocol::BedrockAnthropic) => {
                self.path.ends_with("/invoke-with-response-stream")
            }
            Some(ModelProtocol::BedrockConverse) => self.path.ends_with("/converse-stream"),
            _ => self
                .body
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// The wire model ID, including a URI-encoded segment for path-based APIs.
    /// Use decoded_model for a provider-neutral ID or RequestMatch::model.
    pub fn model(&self) -> Option<&str> {
        if let Some(model) = self.body.get("model").and_then(Value::as_str) {
            return Some(model);
        }
        match self.protocol()? {
            ModelProtocol::Gemini => self
                .path
                .rsplit_once("/models/")?
                .1
                .rsplit_once(':')
                .map(|(model, _)| model),
            ModelProtocol::BedrockAnthropic | ModelProtocol::BedrockConverse => self
                .path
                .rsplit_once("/model/")?
                .1
                .rsplit_once('/')
                .map(|(model, _)| model),
            _ => None,
        }
    }

    pub fn decoded_model(&self) -> Option<String> {
        self.model().map(|model| {
            if self.body.get("model").and_then(Value::as_str).is_some() {
                model.to_owned()
            } else {
                super::protocol::decode_model(model)
            }
        })
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// User-authored text only. System messages, assistant text, tool results,
    /// image data, and function arguments are deliberately excluded.
    pub fn user_text(&self) -> String {
        let mut texts = Vec::new();
        if let Some(input) = self.body.get("input") {
            if let Some(text) = input.as_str() {
                texts.push(text.to_owned());
            } else {
                user_messages(input, "content", &mut texts);
            }
        } else if let Some(messages) = self.body.get("messages") {
            user_messages(messages, "content", &mut texts);
        } else if let Some(contents) = self.body.get("contents") {
            user_messages(contents, "parts", &mut texts);
        }
        texts.join("\n")
    }

    pub fn declares_function(&self, name: &str) -> bool {
        self.body
            .get("tools")
            .and_then(Value::as_array)
            .is_some_and(|tools| {
                tools.iter().any(|tool| {
                    tool.get("name").and_then(Value::as_str) == Some(name)
                        || tool.pointer("/function/name").and_then(Value::as_str) == Some(name)
                        || tool
                            .get("functionDeclarations")
                            .or_else(|| tool.get("function_declarations"))
                            .and_then(Value::as_array)
                            .is_some_and(|declarations| {
                                declarations.iter().any(|item| {
                                    item.get("name").and_then(Value::as_str) == Some(name)
                                })
                            })
                })
            })
            || self
                .body
                .pointer("/toolConfig/tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| {
                    tools.iter().any(|tool| {
                        tool.pointer("/toolSpec/name").and_then(Value::as_str) == Some(name)
                    })
                })
    }

    /// Find the first output correlated by a protocol call ID. Ollama has no
    /// wire call ID: it accepts tool_name or Agena's anchored tool_call_id prefix.
    pub fn tool_output(&self, call_id: &str) -> Option<&Value> {
        self.tool_output_items(call_id).into_iter().flatten().next()
    }

    pub fn tool_output_text(&self, call_id: &str) -> Option<String> {
        self.tool_output(call_id).map(output_text)
    }

    /// Require one matching item, counting malformed items without an output so
    /// that a missing field cannot conceal a duplicate correlation ID.
    pub fn require_tool_output(&self, call_id: &str) -> Result<&Value> {
        let outputs = self.tool_output_items(call_id);
        ensure!(
            !outputs.is_empty(),
            "request {} has no function_call_output or native tool output for {call_id:?}",
            self.sequence
        );
        ensure!(
            outputs.len() == 1,
            "request {} has {} function_call_output items (or native tool outputs) for {call_id:?}; expected exactly one",
            self.sequence,
            outputs.len()
        );
        outputs[0].with_context(|| {
            format!(
                "function_call_output or native tool output for {call_id:?} has no output field"
            )
        })
    }

    pub fn assert_tool_output_contains(&self, call_id: &str, expected: &str) -> Result<()> {
        let actual = output_text(self.require_tool_output(call_id)?);
        ensure!(
            actual.contains(expected),
            "tool output {call_id:?} does not contain {:?}; actual output: {}",
            bounded_text(expected, 2048),
            bounded_text(&actual, 4096)
        );
        Ok(())
    }

    fn tool_output_items(&self, call_id: &str) -> Vec<Option<&Value>> {
        let mut outputs = Vec::new();
        if let Some(items) = self.body.get("input").and_then(Value::as_array) {
            for item in items {
                if item.get("type").and_then(Value::as_str) == Some("function_call_output")
                    && item.get("call_id").and_then(Value::as_str) == Some(call_id)
                {
                    outputs.push(item.get("output"));
                }
            }
        }
        if let Some(messages) = self.body.get("messages").and_then(Value::as_array) {
            for message in messages {
                let role = message.get("role").and_then(Value::as_str);
                if role == Some("tool")
                    && (message
                        .get("tool_call_id")
                        .or_else(|| message.get("tool_name"))
                        .and_then(Value::as_str)
                        == Some(call_id)
                        || (self.protocol() == Some(ModelProtocol::Ollama)
                            && message
                                .get("content")
                                .and_then(Value::as_str)
                                .and_then(|text| text.strip_prefix("tool_call_id="))
                                .and_then(|text| text.split_once('\n'))
                                .is_some_and(|(id, _)| id == call_id)))
                {
                    outputs.push(message.get("content"));
                }
                if role == Some("user")
                    && let Some(blocks) = message.get("content").and_then(Value::as_array)
                {
                    for block in blocks {
                        if block.get("type").and_then(Value::as_str) == Some("tool_result")
                            && block.get("tool_use_id").and_then(Value::as_str) == Some(call_id)
                        {
                            outputs.push(block.get("content"));
                        }
                        if let Some(result) = block.get("toolResult")
                            && result.get("toolUseId").and_then(Value::as_str) == Some(call_id)
                        {
                            outputs.push(result.get("content"));
                        }
                    }
                }
            }
        }
        if let Some(contents) = self.body.get("contents").and_then(Value::as_array) {
            for content in contents {
                if content.get("role").and_then(Value::as_str) != Some("user") {
                    continue;
                }
                for part in content
                    .get("parts")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(response) = part.get("functionResponse")
                        && response.get("id").and_then(Value::as_str) == Some(call_id)
                    {
                        outputs.push(response.get("response"));
                    }
                }
            }
        }
        outputs
    }
}

fn user_messages(messages: &Value, field: &str, texts: &mut Vec<String>) {
    for message in messages.as_array().into_iter().flatten() {
        if message.get("role").and_then(Value::as_str) == Some("user")
            && let Some(content) = message.get(field)
        {
            collect_text(content, texts);
        }
    }
}

fn output_text(output: &Value) -> String {
    let mut texts = Vec::new();
    collect_text(output, &mut texts);
    if texts.is_empty() && !output.is_null() {
        output.to_string()
    } else {
        texts.join("\n")
    }
}
