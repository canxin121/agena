use agena_api::part::{PartDetailResource, PartExecutionStatusResource};
use agena_api::resource::{RunMetadata, RunRole, RunStatus, RunUsage};
use chrono::{DateTime, Utc};

/// Local display model used by the transcript renderer and its fixtures.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptPart {
    pub id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub status: PartExecutionStatusResource,
    pub kind: TranscriptPartKind,

    pub name: Option<String>,

    pub summary: Option<String>,

    pub has_detail: bool,

    pub activity_id: Option<agena_domain::ActivityId>,

    pub segment_id: Option<agena_domain::TextSegmentId>,

    pub operation_id: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Human presentation of this part's durable facts. Kind-agnostic: every
    /// part kind may carry one, so consumers never probe `content` to render a
    /// row.
    pub presentation: Option<agena_domain::PartDocument>,

    pub content: Option<PartDetailResource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]

/// Kind of a message part; pairs with [`agena_domain::PartKind`] and drives how the part is rendered.
pub enum TranscriptPartKind {
    Text,
    Activity,
}

#[derive(Debug, Clone)]
/// A message in a session transcript.
pub struct TranscriptRun {
    pub id: i64,
    pub session_id: i64,
    pub role: RunRole,
    pub state: RunStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: RunMetadata,

    pub usage: Option<RunUsage>,
    pub part_count: u64,

    pub parts: Option<Vec<TranscriptPart>>,
}
