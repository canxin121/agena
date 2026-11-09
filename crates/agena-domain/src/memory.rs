//! Host-owned lifecycle metadata for editable Markdown memories.
//!
//! A source identifies the host event used to produce a note. It is evidence
//! provenance, not a claim that a model-generated note has been verified true.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    #[default]
    Fact,
    Preference,
    Episode,
    Procedure,
    Task,
}

impl MemoryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Episode => "episode",
            Self::Procedure => "procedure",
            Self::Task => "task",
        }
    }
}

/// Scope remains inside its owning project or instance storage boundary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryScope {
    #[default]
    Workspace,
    Global,
    Session {
        session_id: i64,
        instance_id: String,
    },
}

impl MemoryScope {
    pub fn includes_context(&self, session_id: Option<i64>, instance_id: Option<&str>) -> bool {
        match self {
            Self::Workspace | Self::Global => true,
            Self::Session {
                session_id: owner,
                instance_id: owner_instance,
            } => session_id == Some(*owner) && instance_id == Some(owner_instance.as_str()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    #[default]
    Active,
    Archived,
    Superseded,
}

/// Who last changed these bytes; only intact Derived records may be auto-updated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOrigin {
    #[default]
    External,
    Explicit,
    Derived,
}

/// Supplied by the runtime from its authenticated callback/event context.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemorySource {
    /// Durable runtime/database identity; session and entry IDs are local to it.
    pub instance_id: String,
    pub session_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    /// Last durable source entry included in this note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryMetadata {
    pub id: String,
    /// Monotonic store revision, also written into the document frontmatter.
    pub revision: u64,
    pub kind: MemoryKind,
    pub scope: MemoryScope,
    pub status: MemoryStatus,
    pub origin: MemoryOrigin,
    /// Relative importance in the range 0..=100, not a truth confidence score.
    pub importance: u8,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<MemorySource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// True only when current bytes match a host-stamped source record.
    /// This does not certify the factual correctness of the note.
    pub provenance_verified: bool,
    /// Computed by the store when a user has forgotten this derived source.
    /// Such a record must not be returned through any model-facing reader.
    pub source_suppressed: bool,
}

impl Default for MemoryMetadata {
    fn default() -> Self {
        Self {
            id: String::new(),
            revision: 0,
            kind: MemoryKind::Fact,
            scope: MemoryScope::Workspace,
            status: MemoryStatus::Active,
            origin: MemoryOrigin::External,
            importance: 50,
            created_at_ms: 0,
            updated_at_ms: 0,
            source: None,
            supersedes: Vec::new(),
            superseded_by: None,
            provenance_verified: false,
            source_suppressed: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryRevision {
    pub name: String,
    pub sha256: String,
}
