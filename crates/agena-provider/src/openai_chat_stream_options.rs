use serde::Serialize;

#[derive(Debug, Serialize)]
/// Wire shape of chat stream options.
pub struct ChatStreamOptions {
    pub include_usage: bool,
}
