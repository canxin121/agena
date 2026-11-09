//! Cross-project operations are reserved for the persisted operator Bot.
use agena_domain::{ModelRef, SessionStateKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BotAction {
    Projects,
    Project,
    RegisterProject,
    Sessions,
    Create,
    Read,
    ReadText,
    Status,
    Rename,
    Organize,
    Fork,
    Delete,
    Models,
    Configure,
    Queue,
    QueuePause,
    QueueEdit,
    QueueRemove,
    Export,
    Send,
    Stop,
}

#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema, agena_macros::ToolInput,
)]
#[serde(deny_unknown_fields)]
pub struct BotRequest {
    pub action: BotAction,
    /// Existing registered project ID returned by projects; required for project/create. Sessions may search all registered eligible projects.
    #[serde(default)]
    pub workspace_id: Option<i64>,
    /// Absolute existing directory to register. Does not grant filesystem access.
    #[serde(default)]
    pub path: Option<String>,
    /// Exact eligible target ID returned by sessions; required for conversation actions.
    #[serde(default)]
    pub session_id: Option<i64>,
    /// Human-readable title (at most 512 bytes): required for create/rename, optional for fork.
    #[serde(default)]
    pub title: Option<String>,
    /// Self-contained send instruction, or replacement text for queue_edit; at most 16 KiB.
    #[serde(default)]
    pub instruction: Option<String>,
    /// Exact quote from the current real user message, required for every write.
    #[serde(default)]
    pub user_quote: Option<String>,
    /// Stable operation identity within this user turn. Reuse unchanged on retry.
    #[serde(default)]
    pub request_key: Option<String>,
    /// Metadata version returned by status; required for rename/organize/configure/delete.
    #[serde(default)]
    pub expected_version: Option<i64>,
    /// Exact execution observed by status/queue; required for stop, queue_edit and queue_remove.
    #[serde(default)]
    pub execution_id: Option<String>,
    /// Native execution generation returned by status/queue, paired with execution_id.
    #[serde(default)]
    pub expected_generation: Option<i64>,
    /// Opaque next_cursor from the same action/target/filter; preserve it unchanged when paging.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Initial page offset where supported; cannot be combined with cursor.
    #[serde(default)]
    pub offset: u64,
    /// Page size, clamped to 1..50. Transcript previews are also byte limited.
    #[serde(default)]
    pub limit: Option<u64>,

    /// Project name/path or session title search. Resolve names here; never ask humans for IDs.
    #[serde(default)]
    pub query: Option<String>,

    /// Filter sessions, or explicitly set favorite with organize (never toggles).
    #[serde(default)]
    pub favorite: Option<bool>,

    /// Filter sessions, or explicitly set pinned with organize (never toggles).
    #[serde(default)]
    pub pinned: Option<bool>,

    /// Native session states to include in sessions.
    #[serde(default)]
    pub states: Vec<SessionStateKind>,

    /// Filter sessions by their exact ordinary parent.
    #[serde(default)]
    pub parent_session_id: Option<i64>,

    /// Visible text entry returned by read; required for read_text.
    #[serde(default)]
    pub entry_id: Option<i64>,

    /// Next byte offset returned by read_text.
    #[serde(default)]
    pub byte_offset: Option<u64>,

    /// Bounded read_text byte budget (at most 16000).
    #[serde(default)]
    pub max_bytes: Option<u64>,

    /// Fork through this visible entry; absent freezes the latest boundary on first attempt.
    #[serde(default)]
    pub at_message_id: Option<i64>,

    /// Observed native queue/control revision; required for delete and queue mutations.
    #[serde(default)]
    pub expected_control_version: Option<i64>,

    /// Observed transcript entry count, required for deletion confirmation.
    #[serde(default)]
    pub expected_entry_count: Option<u64>,

    /// Configured native provider/model reference from models. Unset preserves selection.
    #[serde(default, deserialize_with = "deserialize_bot_model")]
    #[schemars(with = "Option<BotModelSchema>")]
    pub model: Option<ModelRef>,

    /// Supported thinking option from models; unset preserves previous value.
    #[serde(default)]
    pub thinking_mode: Option<String>,

    /// Supported speed option from models; unset preserves previous value.
    #[serde(default)]
    pub speed_mode: Option<String>,

    /// Supported verbosity from models; unset preserves previous value.
    #[serde(default)]
    pub verbosity: Option<String>,

    /// Explicit parallel tool calling preference; unset preserves previous value.
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,

    /// Reset model to current project defaults; incompatible with model.
    #[serde(default)]
    pub reset_model: bool,

    /// Explicit queue pause state for queue_pause; never toggles.
    #[serde(default)]
    pub paused: Option<bool>,
}

// Native identifier newtypes serialize as strings. Keep the SDK field native
// while describing that exact wire shape without changing domain derives.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct BotModelSchema {
    provider_id: String,
    #[serde(default)]
    adapter_id: Option<String>,
    model_id: String,
}

fn deserialize_bot_model<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ModelRef>, D::Error> {
    Option::<BotModelSchema>::deserialize(deserializer)?
        .map(|wire| {
            match wire.adapter_id {
                Some(adapter) => {
                    ModelRef::try_new_with_adapter(wire.provider_id, adapter, wire.model_id)
                }
                None => ModelRef::try_new(wire.provider_id, wire.model_id),
            }
            .map_err(serde::de::Error::custom)
        })
        .transpose()
}
