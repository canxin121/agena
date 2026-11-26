use std::collections::{BTreeMap, HashSet};

use agena_domain::{
    EXTERNAL_AGENT_URL_INPUT_KIND, EXTERNAL_AGENT_URL_OPEN_OPTION, external_agent_authorization_url,
};
use agena_plugin_host::sdk::schema_validation::CompiledSchema;
use serde_json::{Map, Number};

use super::*;

pub(in crate::session::manager) const FORM_INPUT_KIND: &str = "external_agent_form";
const FORM_TOOL: &str = "external_agent_input";
const URL_TOOL: &str = "external_agent_authorization";

impl ExternalTurn {
    pub(super) async fn interaction_agent_label(&self, params: &Value) -> String {
        if let Some(session) = params["sessionId"].as_str() {
            if let Some(child) = self.native_child_label(session).await {
                return format!("{} · {child}", self.agent.label());
            }
        }
        self.agent.label().to_owned()
    }

    pub(super) async fn request_elicitation(
        &self,
        params: &Value,
        withdrawal: &CancellationToken,
    ) -> Result<Option<Value>, AppError> {
        match params["mode"].as_str() {
            Some("form") => self.request_form(params, withdrawal).await,
            Some("url") => self.request_url(params, withdrawal).await,
            _ => Ok(None),
        }
    }

    async fn request_form(
        &self,
        params: &Value,
        withdrawal: &CancellationToken,
    ) -> Result<Option<Value>, AppError> {
        let agent_label = self.interaction_agent_label(params).await;
        let Some((input, form)) = form_input(params, &agent_label) else {
            return Ok(None);
        };
        if withdrawal.is_cancelled() {
            return Ok(Some(json!({"action": "cancel"})));
        }
        let title = input.title.clone();
        let tool = self
            .begin_tool(
                FORM_TOOL.into(),
                json!({
                    "agent": self.agent,
                    "tool_call_id": params["toolCallId"],
                    "schema": params["requestedSchema"],
                }),
            )
            .await?;
        let answer = self
            .manager
            .request_input_for_tool(
                &tool,
                &self.control,
                RequestId::new(),
                input,
                Some(withdrawal),
            )
            .await?;
        let (result, phase, summary) = if withdrawal.is_cancelled() || answer.timed_out {
            (
                json!({"action": "cancel"}),
                ToolPhase::Cancelled,
                "Question cancelled",
            )
        } else if answer.cancelled {
            (
                json!({"action": "decline"}),
                ToolPhase::Cancelled,
                "Question declined",
            )
        } else {
            (
                json!({"action": "accept", "content": form.content(&answer.answers)?}),
                ToolPhase::Succeeded,
                "Question answered",
            )
        };
        let outcome = ToolResult {
            phase,
            value: json!({"title": title, "summary": summary}),
        };
        let command = RequestId::new();
        persist(|| tool.complete(command, outcome.clone()))
            .await
            .map_err(kernel_error)?;
        Ok(Some(result))
    }

    async fn request_url(
        &self,
        params: &Value,
        withdrawal: &CancellationToken,
    ) -> Result<Option<Value>, AppError> {
        let agent_label = self.interaction_agent_label(params).await;
        let Some(input) = url_input(params, &agent_label) else {
            return Ok(None);
        };
        if withdrawal.is_cancelled() || self.control.cancel.is_cancelled() {
            return Ok(Some(json!({"action": "cancel"})));
        }

        let title = input.title.clone();
        let tool = self
            .begin_tool(URL_TOOL.into(), json!({"agent": self.agent}))
            .await?;
        let answer = self
            .manager
            .request_input_for_tool(
                &tool,
                &self.control,
                RequestId::new(),
                input,
                Some(withdrawal),
            )
            .await?;
        let action = if answer.timed_out
            || withdrawal.is_cancelled()
            || self.control.cancel.is_cancelled()
        {
            "cancel"
        } else if !answer.cancelled && answer.reply == EXTERNAL_AGENT_URL_OPEN_OPTION {
            "accept"
        } else {
            "decline"
        };
        let (phase, summary) = match action {
            "accept" => (ToolPhase::Succeeded, "Authorization page accepted"),
            "cancel" => (ToolPhase::Cancelled, "Authorization cancelled"),
            _ => (ToolPhase::Cancelled, "Authorization declined"),
        };
        let outcome = ToolResult {
            phase,
            value: json!({"title": title, "summary": summary}),
        };
        let command = RequestId::new();
        persist(|| tool.complete(command, outcome.clone()))
            .await
            .map_err(kernel_error)?;
        Ok(Some(json!({"action": action})))
    }

    pub(super) async fn complete_url_elicitation(&self) -> Result<(), AppError> {
        if self.control.cancel.is_cancelled() {
            return Ok(());
        }
        let tool = self
            .begin_tool(URL_TOOL.into(), json!({"agent": self.agent}))
            .await?;
        let outcome = ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({
                "title": format!("{} · Authorization", self.agent.label()),
                "summary": "Authorization completed"
            }),
        };
        let command = RequestId::new();
        persist(|| tool.complete(command, outcome.clone()))
            .await
            .map_err(kernel_error)?;
        Ok(())
    }
}

pub(super) fn elicitation_id(params: &Value) -> Option<&str> {
    params["elicitationId"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 4096)
}

pub(super) fn form_input(
    params: &Value,
    agent_label: &str,
) -> Option<(crate::part::AskUserToolInput, Form)> {
    let schema = &params["requestedSchema"];
    let message = bounded_message(params)?;
    let form = Form::parse(schema).ok()?;
    let input = crate::part::AskUserToolInput {
        title: format!(
            "{agent_label} · {}",
            label(schema, "title").unwrap_or("Question")
        ),
        body_markdown: message,
        kind: FORM_INPUT_KIND.into(),
        questions: form.questions(),
        auto_resolution_ms: None,
    };
    Some((input, form))
}

pub(super) fn url_input(
    params: &Value,
    agent_label: &str,
) -> Option<crate::part::AskUserToolInput> {
    let address = external_agent_authorization_url(params["url"].as_str()?)?;
    let message = bounded_message(params)?;
    Some(crate::part::AskUserToolInput {
        title: format!("{agent_label} · Authorization"),
        body_markdown: message,
        kind: EXTERNAL_AGENT_URL_INPUT_KIND.into(),
        questions: vec![UserInputQuestion {
            header: String::new(),
            question: address.to_string(),
            options: vec![option(EXTERNAL_AGENT_URL_OPEN_OPTION, "", false)],
            multiple: false,
            allow_custom: false,
            optional: false,
        }],
        auto_resolution_ms: None,
    })
}

fn bounded_message(params: &Value) -> Option<String> {
    let message = params["message"].as_str()?;
    Some(message[..message.floor_char_boundary(8192)].to_owned())
}

impl SessionManager {
    pub(in crate::session::manager) async fn validate_external_agent_form_reply(
        &self,
        session: SessionId,
        tool: ToolId,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Result<(), AppError> {
        let tool = self
            .kernel()
            .reader
            .tool(session, tool)
            .await
            .map_err(kernel_error)?;
        if tool.call.name == FORM_TOOL {
            Form::parse(&tool.call.input["schema"])?.content(answers)?;
        }
        Ok(())
    }
}

pub(super) struct Form {
    fields: Vec<Field>,
    validator: CompiledSchema,
}

struct Field {
    name: String,
    question: UserInputQuestion,
    input: Input,
}

enum Input {
    Text,
    Integer,
    Number,
    Boolean,
    Choice(Vec<String>),
}

impl Form {
    fn parse(schema: &Value) -> Result<Self, AppError> {
        if !schema.is_object() || !(schema["type"].is_null() || schema["type"] == "object") {
            return Err(invalid_schema());
        }
        let required: Vec<String> = if schema["required"].is_null() {
            Vec::new()
        } else {
            serde_json::from_value(schema["required"].clone()).map_err(|_| invalid_schema())?
        };
        let empty = Map::new();
        let properties = if schema["properties"].is_null() {
            &empty
        } else {
            schema["properties"]
                .as_object()
                .ok_or_else(invalid_schema)?
        };
        if required.iter().any(|key| !properties.contains_key(key)) {
            return Err(invalid_schema());
        }
        let mut fields = Vec::with_capacity(properties.len());
        let mut validation_properties = Map::new();
        for (name, property) in properties {
            let required = required.contains(name);
            let (field, rules) = Field::parse(name, property, required)?;
            validation_properties.insert(name.clone(), rules);
            fields.push(field);
        }
        let validator = CompiledSchema::new(&json!({
            "type": "object", "properties": validation_properties, "required": required,
        }))
        .map_err(AppError::Config)?;
        Ok(Self { fields, validator })
    }

    fn questions(&self) -> Vec<UserInputQuestion> {
        if self.fields.is_empty() {
            return vec![UserInputQuestion {
                header: String::new(),
                question: "Continue?".into(),
                options: vec![option("Continue", "", false)],
                multiple: false,
                allow_custom: false,
                optional: false,
            }];
        }
        self.fields
            .iter()
            .map(|field| field.question.clone())
            .collect()
    }

    pub(super) fn content(
        &self,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Result<Value, AppError> {
        let mut content = Map::new();
        for (index, field) in self.fields.iter().enumerate() {
            if let Some(values) = answers.get(&index.to_string()) {
                content.insert(field.name.clone(), field.value(values)?);
            }
        }
        let content = Value::Object(content);
        self.validator
            .validate(&content)
            .map_err(AppError::Config)?;
        Ok(content)
    }
}

impl Field {
    fn parse(name: &str, property: &Value, required: bool) -> Result<(Self, Value), AppError> {
        let kind = property["type"].as_str().ok_or_else(invalid_schema)?;
        let mut rules = json!({"type": kind});
        let mut options = Vec::new();
        let input = match kind {
            "string" if !property["enum"].is_null() || !property["oneOf"].is_null() => {
                let choices = choices(property, "oneOf", &mut options)?;
                rules["enum"] = json!(choices);
                Input::Choice(choices)
            }
            "string" => Input::Text,
            "integer" => Input::Integer,
            "number" => Input::Number,
            "boolean" => {
                options = vec![option("True", "", false), option("False", "", false)];
                Input::Boolean
            }
            "array" => {
                let items = &property["items"];
                if !(items["type"].is_null() || items["type"] == "string") {
                    return Err(invalid_schema());
                }
                let choices = choices(items, "anyOf", &mut options)?;
                rules["items"] = json!({"type": "string", "enum": choices});
                rules["uniqueItems"] = json!(true);
                Input::Choice(choices)
            }
            _ => return Err(invalid_schema()),
        };
        let keywords: &[&str] = match kind {
            "string" => &["minLength", "maxLength", "pattern", "format"],
            "integer" | "number" => &["minimum", "maximum"],
            "array" => &["minItems", "maxItems"],
            _ => &[],
        };
        for &keyword in keywords {
            if !property[keyword].is_null() {
                rules[keyword] = property[keyword].clone();
            }
        }
        if let Some(text) = match input {
            Input::Text => Some("Enter text"),
            Input::Integer => Some("Enter an integer"),
            Input::Number => Some("Enter a number"),
            _ => None,
        } {
            options.push(option(text, "", true));
        }
        let title = label(property, "title")
            .or_else(|| (!name.trim().is_empty()).then_some(name.trim()))
            .unwrap_or("Field");
        let description = label(property, "description");
        let question = UserInputQuestion {
            header: description
                .filter(|text| *text != title)
                .map_or_else(String::new, |_| title.to_owned()),
            question: description.unwrap_or(title).to_owned(),
            options,
            multiple: kind == "array",
            allow_custom: false,
            optional: !required,
        };
        Ok((
            Self {
                name: name.to_owned(),
                question,
                input,
            },
            rules,
        ))
    }

    fn value(&self, values: &[String]) -> Result<Value, AppError> {
        let invalid = || AppError::Config(format!("Invalid value for {}.", self.name));
        let value = values.first().map(String::as_str).ok_or_else(invalid)?;
        match &self.input {
            Input::Text => Ok(json!(value)),
            Input::Integer => value
                .trim()
                .parse::<i64>()
                .map(|value| json!(value))
                .map_err(|_| invalid()),
            Input::Number => serde_json::from_str::<Number>(value)
                .map(Value::Number)
                .map_err(|_| invalid()),
            Input::Boolean => match value.trim() {
                "True" => Ok(json!(true)),
                "False" => Ok(json!(false)),
                _ => Err(invalid()),
            },
            Input::Choice(choices) => {
                let choice = |value: &str| {
                    self.question
                        .options
                        .iter()
                        .position(|option| option.label == value.trim())
                        .map(|index| json!(choices[index]))
                        .ok_or_else(invalid)
                };
                if self.question.multiple {
                    values.iter().map(|value| choice(value)).collect()
                } else {
                    choice(value)
                }
            }
        }
    }
}

fn choices(
    schema: &Value,
    titled_keyword: &str,
    options: &mut Vec<UserInputOption>,
) -> Result<Vec<String>, AppError> {
    let titled = !schema[titled_keyword].is_null();
    let entries = if titled {
        &schema[titled_keyword]
    } else {
        &schema["enum"]
    };
    let entries = entries
        .as_array()
        .filter(|entries| !entries.is_empty())
        .ok_or_else(invalid_schema)?;
    let mut labels = HashSet::new();
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let value = if titled {
                entry["const"].as_str()
            } else {
                entry.as_str()
            }
            .ok_or_else(invalid_schema)?;
            let mut title = label(entry, "title").unwrap_or(value.trim()).to_owned();
            if title.is_empty() {
                title = format!("{value:?}");
            }
            while !labels.insert(title.clone()) {
                title.push_str(&format!(" ({})", index + 1));
            }
            options.push(option(
                &title,
                label(entry, "description").unwrap_or_default(),
                false,
            ));
            Ok(value.to_owned())
        })
        .collect()
}

fn label<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value[key]
        .as_str()
        .map(str::trim)
        .filter(|label| !label.is_empty())
}

fn option(label: &str, description: &str, custom: bool) -> UserInputOption {
    UserInputOption {
        label: label.to_owned(),
        description: description.to_owned(),
        custom,
    }
}

fn invalid_schema() -> AppError {
    AppError::Config("Invalid ACP elicitation form schema.".into())
}
