//! Shared wire fixtures; tests exercise each adapter's real request builder.

use agena_domain::Role;
use agena_provider::{
    CompletionInputAttachment, CompletionInputPart, CompletionInputRun, CompletionRequest,
    ModelToolFunction,
};

pub(super) fn attachment(kind: &str, mime: &str, data: &str) -> CompletionInputAttachment {
    serde_json::from_value(serde_json::json!({
        "kind":kind, "mime":mime, "filename":"fixture", "source":{"type":"base64","data":data}
    }))
    .unwrap()
}

pub(super) fn request(
    model: &str,
    attachments: Vec<CompletionInputAttachment>,
) -> CompletionRequest {
    let mut request: CompletionRequest =
        serde_json::from_value(serde_json::json!({"model":model,"messages":[]})).unwrap();
    request.turns = vec![CompletionInputRun {
        role: Role::Assistant,
        parts: vec![
            CompletionInputPart::ToolCall {
                id: "call_media".into(),
                function: ModelToolFunction::new("tools_call"),
                arguments_json: r#"{"tool":"fs.read_media","input":{"path":"fixture"}}"#.into(),
            },
            CompletionInputPart::ToolResult {
                tool_call_id: "call_media".into(),
                function: ModelToolFunction::new("tools_call"),
                arguments_json: r#"{"tool":"fs.read_media","input":{"path":"fixture"}}"#.into(),
                status: Default::default(),
                output_json: "Media file contents are attached.".into(),
                attachments,
            },
        ],
        provider_state: Default::default(),
    }];
    request
}
