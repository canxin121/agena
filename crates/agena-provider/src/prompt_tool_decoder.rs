//! Incremental decoder for the Agena prompt-envelope tool-call protocol.

use crate::{CompletionToolCall, PromptToolCallsEnvelope};

const MAX_BUFFERED_ENVELOPE_BYTES: usize = 1024 * 1024;

/// One decoded fragment from an incremental prompt-envelope response.
#[derive(Debug)]
pub enum PromptToolDecodedItem {
    Text(String),
    Calls(Vec<CompletionToolCall>),
}

/// Incrementally separates ordinary provider text from complete prompt-tool
/// call envelopes without requiring transport, session, or Core error types.
#[derive(Debug)]
pub struct PromptToolTextDecoder {
    state: DecoderState,
    buffer: String,
    call_open: String,
    call_close: String,
}

#[derive(Debug, Default)]
enum DecoderState {
    #[default]
    Text,
    Envelope,
}

impl PromptToolTextDecoder {
    pub fn new(call_open: impl Into<String>, call_close: impl Into<String>) -> Self {
        Self {
            state: DecoderState::Text,
            buffer: String::new(),
            call_open: call_open.into(),
            call_close: call_close.into(),
        }
    }

    pub fn push(&mut self, delta: &str) -> Vec<PromptToolDecodedItem> {
        let mut buffer = std::mem::take(&mut self.buffer);
        buffer.push_str(delta);
        let items = self.drain(&mut buffer, false);
        self.buffer = buffer;
        items
    }

    pub fn finish(&mut self) -> Vec<PromptToolDecodedItem> {
        let mut buffer = std::mem::take(&mut self.buffer);
        self.drain(&mut buffer, true)
    }

    fn drain(&mut self, buffer: &mut String, finishing: bool) -> Vec<PromptToolDecodedItem> {
        let mut items = Vec::new();
        loop {
            match self.state {
                DecoderState::Text => {
                    if let Some(index) = buffer.find(self.call_open.as_str()) {
                        if index > 0 {
                            items.push(PromptToolDecodedItem::Text(buffer[..index].to_owned()));
                        }
                        buffer.drain(..index + self.call_open.len());
                        self.state = DecoderState::Envelope;
                        continue;
                    }

                    if finishing {
                        if !buffer.is_empty() {
                            items.push(PromptToolDecodedItem::Text(std::mem::take(buffer)));
                        }
                    } else {
                        let retained =
                            longest_marker_prefix_suffix(buffer, self.call_open.as_str());
                        let emit_len = buffer.len().saturating_sub(retained);
                        if emit_len > 0 {
                            items.push(PromptToolDecodedItem::Text(buffer[..emit_len].to_owned()));
                            buffer.drain(..emit_len);
                        }
                    }
                    break;
                }
                DecoderState::Envelope => {
                    if let Some((index, calls)) =
                        find_decodable_envelope(buffer, self.call_close.as_str())
                    {
                        buffer.drain(..index + self.call_close.len());
                        self.state = DecoderState::Text;
                        items.push(PromptToolDecodedItem::Calls(calls));
                        continue;
                    }

                    if finishing || buffer.len() > MAX_BUFFERED_ENVELOPE_BYTES {
                        items.push(PromptToolDecodedItem::Text(format!(
                            "{}{}",
                            self.call_open,
                            std::mem::take(buffer)
                        )));
                        self.state = DecoderState::Text;
                    }
                    break;
                }
            }
        }
        items
    }
}

fn find_decodable_envelope(
    buffer: &str,
    call_close: &str,
) -> Option<(usize, Vec<CompletionToolCall>)> {
    let mut offset = 0;
    while let Some(relative_index) = buffer[offset..].find(call_close) {
        let index = offset + relative_index;
        if let Some(calls) = decode_prompt_tool_calls(&buffer[..index]) {
            return Some((index, calls));
        }
        offset = index + call_close.len();
    }
    None
}

fn longest_marker_prefix_suffix(value: &str, marker: &str) -> usize {
    let max = value.len().min(marker.len().saturating_sub(1));
    (1..=max)
        .rev()
        .find(|length| value.ends_with(&marker[..*length]))
        .unwrap_or_default()
}

/// Decode one complete JSON call-envelope body.
pub fn decode_prompt_tool_calls(body: &str) -> Option<Vec<CompletionToolCall>> {
    let envelope = serde_json::from_str::<PromptToolCallsEnvelope>(body.trim()).ok()?;
    if envelope.calls.is_empty() {
        return None;
    }
    envelope
        .calls
        .into_iter()
        .map(|call| {
            let name = call.name;
            if name.is_empty() || !call.arguments.is_object() {
                return None;
            }
            Some(CompletionToolCall::Function {
                id: call
                    .id
                    .map(|id| id.trim().to_owned())
                    .filter(|id| !id.is_empty())
                    .unwrap_or_default(),
                name,
                arguments_json: serde_json::to_string(&call.arguments).ok()?,
            })
        })
        .collect()
}
