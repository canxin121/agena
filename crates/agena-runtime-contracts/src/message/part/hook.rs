use serde::{Deserialize, Serialize};

/// One observed plugin hook run recorded as a first-class transcript part.
///
/// Hook activity (for example the workflow plan's `agent.stop` autorun hook)
/// rides the same activity pipeline as tool calls: it is persisted with the
/// message, projected to clients, and rendered by the transcript with the
/// existing activity styles.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HookPart {
    /// The hook identifier that ran, for example `agent.stop`.
    pub hook: String,
    /// The plugin that ran the hook, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Short human-facing summary of the hook outcome.
    pub summary: String,
    /// Optional human-facing detail (for example the injected continuation
    /// message) rendered when the activity is expanded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
