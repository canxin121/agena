//! Generic resource reads; transport adapters use this same request contract.

use agena_domain::{ContentCursor, ContentId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadContentParams {
    pub session_id: i64,
    pub resource_id: ContentId,
    pub after: Option<ContentCursor>,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadContentTextParams {
    pub session_id: i64,
    pub resource_id: ContentId,
    pub position: Option<agena_domain::ContentTextPosition>,
    pub max_bytes: usize,
}
