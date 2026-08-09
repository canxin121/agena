use serde::{Deserialize, Serialize};

/// A runtime-originated, human-facing notice recorded as a first-class
/// transcript part (for example "model-turn budget exhausted"). It rides the
/// same activity pipeline as tool calls: persisted with the message,
/// projected to clients, and rendered by the transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NoticePart {
    /// Machine-readable notice category, e.g. `max_turns_exhausted`.
    pub kind: String,
    /// Short human-facing summary (the collapsed headline).
    pub summary: String,
    /// Optional human-facing detail rendered when expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Optional display headline chosen by the producer. Consumers render
    /// this verbatim when present and fall back to a kind-derived title
    /// otherwise, so the title vocabulary is not owned by any single UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
