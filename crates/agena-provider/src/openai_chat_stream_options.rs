use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ChatStreamOptions {
    pub include_usage: bool,
}
