use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct ChatStreamChunk {
    #[serde(default)]
    pub choices: Vec<ChatStreamChoice>,
    #[serde(default)]
    pub usage: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize, Clone)]
pub struct ChatStreamChoice {
    #[serde(default)]
    pub delta: Option<ChatStreamDelta>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}
#[derive(Debug, Deserialize, Clone)]
pub struct ChatStreamDelta {
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    #[serde(default)]
    pub reasoning_content: Option<serde_json::Value>,
    #[serde(default)]
    pub reasoning_details: Option<serde_json::Value>,
    #[serde(default)]
    pub reasoning_text: Option<serde_json::Value>,
    #[serde(default)]
    pub reasoning_opaque: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<serde_json::Value>>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponsesToolEventKind {
    Added,
    Delta,
    Done,
}
#[derive(Debug, Clone)]
pub struct ResponsesToolEvent {
    pub kind: ResponsesToolEventKind,
    pub output_index: Option<usize>,
    pub item_id: Option<String>,
    pub call_id: Option<String>,
    pub namespace: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}
impl ResponsesToolEvent {
    pub fn stream_key_candidates(&self) -> Result<Vec<String>, String> {
        let mut keys = Vec::new();
        if let Some(value) = self.item_id.as_ref() {
            keys.push(format!("item:{value}"));
        }
        if let Some(value) = self.output_index {
            keys.push(format!("idx:{value}"));
        }
        if let Some(value) = self.call_id.as_ref() {
            keys.push(format!("call:{value}"));
        }
        if keys.is_empty() {
            return Err("returned tool event without item_id/output_index/call_id".into());
        }
        Ok(keys)
    }
}
