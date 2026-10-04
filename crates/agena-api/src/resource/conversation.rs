use serde::{Deserialize, Serialize};

use super::RunOptions;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BtwRequest {
    pub question: String,
    #[serde(default)]
    pub options: RunOptions,
}

/// Coalesced, ordered answer snapshots. A disconnect cancels this question;
/// clients must never automatically replay this POST.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtwAnswer {
    pub text: String,
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
