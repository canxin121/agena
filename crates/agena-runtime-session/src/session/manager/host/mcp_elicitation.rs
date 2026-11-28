use agena_domain::mcp_elicitation::{McpElicitation, McpElicitationAction, McpElicitationReply};
use agena_domain::{UserInputOption, UserInputQuestion};
use agena_plugin_host::sdk::{
    ImportedMcpElicitationInput, host_api::AskUserResponse, schema_validation::CompiledSchema,
};
use serde_json::{Value, json};

use super::{AppError, CancellationToken, SessionManager, ToolExecutor};
use crate::part::AskUserToolInput;

const DECLINE: &str = "Decline";
const OPENED: &str = "Continue";
const ACCEPT: &str = "Accept";

impl SessionManager {
    pub async fn resolve_mcp_elicitation(
        &self,
        call: i64,
        mut input: ImportedMcpElicitationInput,
        cancellation: CancellationToken,
    ) -> Result<McpElicitationReply, AppError> {
        let session = input.session_id;
        let (_, control) = self.live_tool(session, call)?;
        let context = self
            .execution_context_with_state(control.session, &control.state)
            .await?;
        let executor = control
            .state
            .executor
            .for_session_resource_context(&context);
        let plugins = executor.plugin_manager();
        let notification =
            |kind: &'static str, title: &'static str, action: Option<McpElicitationAction>| {
                self.observe_claude_elicitation(&executor, session, kind, title, action);
            };
        let request_user = |dialog, kind: &'static str| {
            let cancellation = &cancellation;
            let notification = &notification;
            let should_notify = !executor.is_global() && plugins.has_imported_notification_hooks();
            async move {
                let wait =
                    self.request_cancellable_host_user_input(session, call, dialog, cancellation);
                if !should_notify {
                    return wait.await;
                }
                tokio::pin!(wait);
                tokio::select! {
                    biased;
                    response = &mut wait => response,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(6)) => {
                        notification(kind, "MCP server needs your input", None);
                        wait.await
                    }
                }
            }
        };
        let run_hooks = async |input| {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => None,
                _ = control.cancel.cancelled() => None,
                reply = plugins.dispatch_imported_mcp_elicitation(
                    input, |key| !executor.is_global() && executor.plugin_is_enabled(key),
                ) => reply,
            }
        };
        if let Some(reply) = run_hooks(input.clone()).await {
            if !cancellation.is_cancelled() && !control.cancel.is_cancelled() {
                notification(
                    "elicitation_response",
                    "MCP elicitation response",
                    Some(reply.action),
                );
            }
            return Ok(reply);
        }
        let reply = prompt_mcp_input(&input.server, &input.request, request_user).await?;
        if cancellation.is_cancelled() || control.cancel.is_cancelled() {
            return Ok(McpElicitationReply::new(McpElicitationAction::Cancel));
        }
        input.reply = Some(reply.clone());
        let reply = run_hooks(input).await.unwrap_or(reply);
        if !cancellation.is_cancelled() && !control.cancel.is_cancelled() {
            self.observe_claude_elicitation(
                &executor,
                session,
                "elicitation_response",
                "MCP elicitation response",
                Some(reply.action),
            );
        }
        Ok(reply)
    }

    fn observe_claude_elicitation(
        &self,
        executor: &ToolExecutor,
        session_id: i64,
        kind: &'static str,
        title: &'static str,
        action: Option<McpElicitationAction>,
    ) {
        if executor.is_global() || !executor.plugin_manager().has_imported_notification_hooks() {
            return;
        }
        let executor = executor.clone();
        let input = agena_plugin_host::NotificationInput {
            kind: kind.into(),
            session_id: Some(session_id),
            title: title.into(),
            message: match action {
                Some(McpElicitationAction::Accept) => "MCP elicitation accepted",
                Some(McpElicitationAction::Decline) => "MCP elicitation declined",
                Some(McpElicitationAction::Cancel) => "MCP elicitation cancelled",
                None => "An MCP server is waiting for input",
            }
            .into(),
            payload: Value::Null,
        };
        self.control_tasks.spawn(async move {
            executor
                .plugin_manager()
                .broadcast_imported_notification(input, |key| executor.plugin_is_enabled(key))
                .await;
        });
    }
}

pub(super) async fn prompt_mcp_input<F>(
    source: &str,
    request: &McpElicitation,
    mut request_user: impl FnMut(AskUserToolInput, &'static str) -> F,
) -> Result<McpElicitationReply, AppError>
where
    F: std::future::Future<Output = Result<AskUserResponse, AppError>> + Send,
{
    match request {
        McpElicitation::Form {
            message,
            requested_schema,
        } => {
            let validator = CompiledSchema::new(requested_schema).map_err(AppError::Config)?;
            let mut invalid = None;
            let form = FormInput::new(requested_schema);
            loop {
                let dialog = form.dialog(source, message, requested_schema, invalid.as_deref());
                let response = request_user(dialog, "elicitation_dialog").await?;
                if response.cancelled || response.timed_out {
                    return Ok(McpElicitationReply::new(McpElicitationAction::Cancel));
                }
                if response.reply == DECLINE {
                    return Ok(McpElicitationReply::new(McpElicitationAction::Decline));
                }
                let content = match form.decode(&response.reply) {
                    Ok(content) => content,
                    Err(_) => {
                        invalid = Some("Enter a value in the requested JSON format.".into());
                        continue;
                    }
                };
                match validator.validate(&content) {
                    Ok(()) => {
                        return Ok(McpElicitationReply {
                            action: McpElicitationAction::Accept,
                            content: Some(content),
                        });
                    }
                    Err(error) => invalid = Some(error),
                }
            }
        }
        McpElicitation::Url { message, url, .. } => {
            let dialog = dialog(
                source,
                format!(
                    "{message}\n\n{url}\n\nOpen this address in your browser, then choose Continue."
                ),
                vec![choice(OPENED), choice(DECLINE)],
            );
            let response = request_user(dialog, "elicitation_url_dialog").await?;
            let action = if response.cancelled || response.timed_out {
                McpElicitationAction::Cancel
            } else if response.reply == OPENED {
                McpElicitationAction::Accept
            } else {
                McpElicitationAction::Decline
            };
            Ok(McpElicitationReply::new(action))
        }
    }
}

enum FormInput<'a> {
    Empty,
    Choice {
        name: &'a str,
        choices: Vec<(&'a str, &'a str)>,
    },
    Field {
        name: &'a str,
        schema: &'a Value,
    },
    Object,
}

impl<'a> FormInput<'a> {
    fn new(schema: &'a Value) -> Self {
        // OpenAI forms can use object-wide conditions or require keys that
        // aren't listed in properties. Let the user enter the whole object.
        if !schema.as_object().is_some_and(|object| {
            object.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "$schema"
                        | "type"
                        | "title"
                        | "description"
                        | "properties"
                        | "required"
                        | "additionalProperties"
                )
            })
        }) {
            return Self::Object;
        }
        if let Some(properties) = schema["properties"].as_object() {
            if properties.is_empty()
                && schema
                    .get("required")
                    .is_none_or(|required| required.as_array().is_some_and(Vec::is_empty))
            {
                return Self::Empty;
            }
            if properties.len() == 1
                && let Some((name, field)) = properties.iter().next()
                && schema["required"] == json!([name])
            {
                if let Some(choices) = string_choices(field) {
                    return Self::Choice { name, choices };
                }
                return Self::Field {
                    name,
                    schema: field,
                };
            }
        }
        Self::Object
    }

    fn dialog(
        &self,
        server: &str,
        message: &str,
        schema: &Value,
        error: Option<&str>,
    ) -> AskUserToolInput {
        let (instruction, options) = match self {
            Self::Empty => (
                "Submit an empty form.".into(),
                vec![choice(ACCEPT), choice(DECLINE)],
            ),
            Self::Choice { name, choices } => (
                format!("Choose a value for {name}."),
                choices
                    .iter()
                    .map(|(label, _)| choice(label))
                    .chain([choice(DECLINE)])
                    .collect(),
            ),
            Self::Field { name, schema } if schema["type"] == "boolean" => (
                format!("Choose a value for {name}."),
                vec![choice("true"), choice("false"), choice(DECLINE)],
            ),
            Self::Field { name, schema } if schema["type"] == "string" => (
                format!(
                    "Enter {name}. You can quote the answer as a JSON string to preserve its exact text."
                ),
                vec![custom("Enter text"), choice(DECLINE)],
            ),
            Self::Field { name, .. } => (
                format!("Enter a JSON value for {name}."),
                vec![custom("Enter value"), choice(DECLINE)],
            ),
            Self::Object => (
                "Enter a JSON object.".into(),
                vec![custom("Enter JSON"), choice(DECLINE)],
            ),
        };
        let mut prompt = format!("{message}\n\n{instruction}");
        if matches!(self, Self::Field { .. } | Self::Object) {
            let preview = schema.to_string();
            prompt.push('\n');
            prompt.push_str(&preview[..preview.floor_char_boundary(preview.len().min(8192))]);
            if preview.len() > 8192 {
                prompt.push('…');
            }
        }
        if let Some(error) = error {
            prompt.push_str("\n\n");
            prompt.push_str(&error[..error.floor_char_boundary(error.len().min(500))]);
        }
        dialog(server, prompt, options)
    }

    fn decode(&self, reply: &str) -> serde_json::Result<Value> {
        match self {
            Self::Empty => Ok(json!({})),
            Self::Choice { name, choices } => {
                let value = choices
                    .iter()
                    .find_map(|(label, value)| (*label == reply.trim()).then_some(value));
                Ok(json!({(*name): value}))
            }
            Self::Field { name, schema } => {
                let value = if schema["type"] == "string" {
                    Value::String(serde_json::from_str(reply).unwrap_or_else(|_| reply.to_owned()))
                } else {
                    serde_json::from_str(reply)?
                };
                Ok(json!({(*name): value}))
            }
            Self::Object => serde_json::from_str(reply),
        }
    }
}

fn string_choices(schema: &Value) -> Option<Vec<(&str, &str)>> {
    if schema["type"] != "string" {
        return None;
    }
    let variants = schema["enum"]
        .as_array()
        .or_else(|| schema["oneOf"].as_array())?;
    if variants.is_empty() || variants.len() > 7 {
        return None;
    }
    let mut choices = Vec::new();
    for option in variants {
        let value = option.as_str().or_else(|| option["const"].as_str())?;
        let label = option["title"].as_str().unwrap_or(value);
        if label.trim().is_empty()
            || label.trim() != label
            || label == DECLINE
            || choices.iter().any(|(candidate, _)| *candidate == label)
        {
            return None;
        }
        choices.push((label, value));
    }
    Some(choices)
}

fn dialog(server: &str, question: String, options: Vec<UserInputOption>) -> AskUserToolInput {
    AskUserToolInput {
        title: format!("MCP input from {server}"),
        kind: "ask_user".into(),
        body_markdown: String::new(),
        auto_resolution_ms: None,
        questions: vec![UserInputQuestion {
            header: "MCP".into(),
            question,
            options,
            multiple: false,
            allow_custom: false,
        }],
    }
}

fn choice(label: &str) -> UserInputOption {
    UserInputOption {
        label: label.into(),
        description: String::new(),
        custom: false,
    }
}

fn custom(label: &str) -> UserInputOption {
    UserInputOption {
        custom: true,
        ..choice(label)
    }
}
