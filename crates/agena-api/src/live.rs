use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A separately loadable section of a tool-call detail view.
///
/// Transcript list responses load only `presentation` by default. The other
/// sections are fetched from the tool-section endpoint when the user opens
/// them, so large inputs and raw outputs do not cross the transport boundary
/// merely because their disclosure row is collapsed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ToolDetailSection {
    Metadata,
    Input,
    Output,
    Presentation,
}

impl ToolDetailSection {
    pub const ALL: [Self; 4] = [
        Self::Metadata,
        Self::Input,
        Self::Output,
        Self::Presentation,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Metadata => "metadata",
            Self::Input => "input",
            Self::Output => "output",
            Self::Presentation => "presentation",
        }
    }
}

impl std::str::FromStr for ToolDetailSection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "metadata" => Ok(Self::Metadata),
            "input" => Ok(Self::Input),
            "output" => Ok(Self::Output),
            "presentation" => Ok(Self::Presentation),
            other => Err(format!("unknown tool detail section: {other}")),
        }
    }
}

/// The result of fetching one tool-call section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDetailResource {
    pub part_id: i64,
    pub revision: i64,
    pub updated_at_ms: i64,
    pub section: ToolDetailSection,
    /// Output carries its lifecycle state so a cached section remains useful
    /// when only a sibling section advances the part's envelope timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_state: Option<String>,
    pub value: Value,
}

use crate::part::PartResource;

/// Return the canonical tool-call content with only the requested detail
/// sections populated. Identity, lifecycle, error, authorization, and user
/// interaction records remain available because they drive transcript state
/// and actions; potentially large tool sections are omitted until explicitly
/// requested.
pub fn project_tool_call_content(content: &Value, sections: &[ToolDetailSection]) -> Value {
    let Some(object) = content.as_object() else {
        return content.clone();
    };

    let has = |section| sections.contains(&section);
    let mut projected = object.clone();

    if has(ToolDetailSection::Input) {
        if let Some(input) = object.get("input") {
            projected.insert("input".to_owned(), input.clone());
        }
    } else {
        projected.remove("input");
    }

    if has(ToolDetailSection::Metadata) {
        if let Some(metadata) = object.get("metadata") {
            projected.insert("metadata".to_owned(), metadata.clone());
        }
    } else {
        projected.remove("metadata");
    }

    if !has(ToolDetailSection::Output) {
        projected.remove("output");
    }

    projected.into()
}

/// Factual membership and paging metadata for one run's child window.
/// Clients decide how to display, combine or fold these children.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunSummaryResource {
    pub run_id: i64,
    pub part_count: u64,
    pub page: crate::pagination::PageInfo,
}

/// Current ordered part snapshot used for initial load and reconnect catch-up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionPartsResource {
    pub session_id: i64,
    /// Monotonic `sessions.version` high-water mark.
    pub version: i64,
    pub parts: Vec<PartResource>,
    pub runs: Vec<RunSummaryResource>,
    /// Exact count of user-send messages in the session. Raw part snapshots
    /// may omit it; transcript pages populate it so clients can display the
    /// full history size without loading every page first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message_count: Option<u64>,
    /// Cursor metadata for newest-first keyset pagination. `parts` itself is
    /// returned chronologically so renderers can append it directly.
    pub page: crate::pagination::PageInfo,
}

/// A committed Part or session mutation. This observer
/// notification is never persisted, replayed, or assigned a global sequence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionChangeResource {
    PartAdded {
        session_id: i64,
        part: Box<PartResource>,
    },
    PartUpdated {
        session_id: i64,
        part: Box<PartResource>,
    },
    PartRemoved {
        session_id: i64,
        part_id: i64,
    },
    SessionMetaUpdated {
        session_id: i64,
        version: i64,
        title: String,
        favorite: bool,
        pinned: bool,
        updated_at_ms: i64,
    },
    SessionDeleted {
        session_id: i64,
        workspace_id: i64,
    },
}

impl SessionChangeResource {
    pub fn session_id(&self) -> i64 {
        match self {
            Self::PartAdded { session_id, .. }
            | Self::PartUpdated { session_id, .. }
            | Self::PartRemoved { session_id, .. }
            | Self::SessionDeleted { session_id, .. }
            | Self::SessionMetaUpdated { session_id, .. } => *session_id,
        }
    }
}

/// Ephemeral, non-session runtime signal. `kind` is open-ended; `payload`
/// contains the signal-specific current value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSignalResource {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub payload: Value,
}
