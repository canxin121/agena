use agena_domain::mcp_elicitation::McpElicitation;
use rmcp::ErrorData;
use rmcp::model::{CustomRequest, ErrorCode, ExtensionCapabilities, JsonObject};
use serde::Deserialize;
use serde_json::{Value, json};

const LEGACY_FORM: &str = "openai/form";
const ELICITATION: &str = "openai/elicitation";
const CREATE: &str = "openai/elicitation/create";

pub(super) fn capabilities() -> ExtensionCapabilities {
    [
        (LEGACY_FORM.into(), JsonObject::new()),
        (
            ELICITATION.into(),
            JsonObject::from_iter([("form".into(), json!({}))]),
        ),
    ]
    .into()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Form {
    #[serde(default)]
    mode: Option<String>,
    message: String,
    requested_schema: Value,
}

pub(super) fn decode(request: CustomRequest) -> Result<McpElicitation, ErrorData> {
    if !matches!(request.method.as_str(), CREATE | LEGACY_FORM) {
        return Err(ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            "unsupported MCP request",
            None,
        ));
    }
    let invalid =
        || ErrorData::invalid_params("invalid or unsupported OpenAI form elicitation", None);
    let mut form = request
        .params_as::<Form>()
        .ok()
        .flatten()
        .ok_or_else(invalid)?;
    if request.method == CREATE && form.mode.as_deref() != Some("form") {
        return Err(invalid());
    }
    if form.requested_schema["type"] != "object" {
        return Err(invalid());
    }
    if let Some(properties) = form
        .requested_schema
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        for field in properties
            .values_mut()
            .filter(|value| value["type"] == "openai/imagePicker")
        {
            let items = field["items"]
                .as_array()
                .filter(|items| !items.is_empty())
                .ok_or_else(invalid)?;
            let options = items
                .iter()
                .map(|item| {
                    let id = item["id"].as_str().ok_or_else(invalid)?;
                    let title = item["title"].as_str().unwrap_or(id);
                    Ok(json!({"const":id,"title":title}))
                })
                .collect::<Result<Vec<_>, ErrorData>>()?;
            let mut schema = json!({"type":"string","oneOf":options});
            for key in ["title", "description", "default"] {
                if let Some(value) = field.get(key) {
                    schema[key] = value.clone();
                }
            }
            *field = schema;
        }
    }
    Ok(McpElicitation::Form {
        message: form.message,
        requested_schema: form.requested_schema,
    })
}
