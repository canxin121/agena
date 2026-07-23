use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

/// Stable category for a system notice appended to a session transcript.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SystemNoticeKind {
    ContextInjection,
    ToolPolicyHint,
    Compaction,
    Other,
}
