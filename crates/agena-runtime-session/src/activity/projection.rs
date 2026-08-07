//! View projection for activity v2 (design 07 §6).
//!
//! Pure functions over the single [`RawOutput`] fact source:
//! - [`fallback_human_view`]: the human view when a tool has no `render_human`
//!   — renders the raw output directly (the same content the model sees);
//! - [`for_model`]: the model-side projection (structured JSON preferred).

use agena_domain::{CommandOutputStream, RawOutput, ViewBlock};

/// Human fallback view: render the raw output directly.
///
/// `payload` becomes a `Json` block; `text` becomes a `Log` block. This is the
/// guaranteed-readable floor for any tool that does not provide a renderer.
pub fn fallback_human_view(raw: &RawOutput) -> Vec<ViewBlock> {
    let mut blocks = Vec::new();
    if let Some(payload) = raw.payload.as_ref() {
        blocks.push(ViewBlock::Json {
            id: Some("payload".into()),
            value: payload.clone(),
        });
    }
    if !raw.text.is_empty() {
        blocks.push(ViewBlock::Log {
            id: Some("text".into()),
            stream: CommandOutputStream::Stdout,
            text: raw.text.clone(),
        });
    }
    blocks
}

/// Model-side projection: structured JSON when present, otherwise text.
/// Mirrors the legacy `project_operation_output` semantics (07 §6).
pub fn for_model(raw: &RawOutput) -> String {
    match raw.payload.as_ref() {
        Some(payload) => serde_json::to_string(payload).unwrap_or_else(|_| raw.text.clone()),
        None => raw.text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fallback_renders_payload_and_text() {
        let raw = RawOutput {
            payload: Some(json!({ "exit_code": 0 })),
            text: "done\n".into(),
            ..RawOutput::default()
        };
        let blocks = fallback_human_view(&raw);
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], ViewBlock::Json { .. }));
        assert!(matches!(blocks[1], ViewBlock::Log { .. }));
    }

    #[test]
    fn fallback_with_only_text_is_readable() {
        let blocks = fallback_human_view(&RawOutput::text("hello\n"));
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ViewBlock::Log { text, .. } => assert_eq!(text, "hello\n"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn for_model_prefers_structured_payload() {
        let raw = RawOutput {
            payload: Some(json!({ "a": 1 })),
            text: "ignored".into(),
            ..RawOutput::default()
        };
        assert_eq!(for_model(&raw), r#"{"a":1}"#);
    }

    #[test]
    fn for_model_falls_back_to_text() {
        assert_eq!(for_model(&RawOutput::text("plain")), "plain");
    }
}
