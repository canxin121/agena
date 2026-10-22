//! Bounded message windows over immutable transcript membership.
use super::*;

/// Ascending owned-tool position in a bounded prefix. Unrelated session
/// updates and appends do not invalidate a retained membership anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OwnedToolCursor {
    pub session: SessionId,
    pub membership: MembershipGeneration,
    pub control_version: i64,
    pub after: EntryId,
    #[serde(default)]
    pub through: Option<EntryId>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnedToolPage {
    pub metadata: SessionMetadata,
    pub control_version: i64,
    pub entries: Vec<EntryView>,
    pub next: Option<OwnedToolCursor>,
    pub has_more: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct MessageHistorySummary {
    pub id: EntryId,
    pub entry_count: u64,
    pub loaded_count: u64,
    pub next: Option<HistoryCursor>,
}
#[derive(Debug, Clone, Serialize)]
pub struct MessageHistoryPage {
    pub metadata: SessionMetadata,
    pub entries: Vec<EntryView>,
    pub messages: Vec<MessageHistorySummary>,
    pub next: Option<HistoryCursor>,
    pub has_more: bool,
}
impl KernelReader {
    /// Read only origin-owned tool entries still in the live membership.
    /// Identity/size preflight excludes ordinary transcript and provider replay.
    pub async fn owned_tool_entries(
        &self,
        session: SessionId,
        after: Option<OwnedToolCursor>,
        limit: usize,
    ) -> Result<ReadView<OwnedToolPage>> {
        self.0
            .backend
            .owned_tool_entries(session, after, limit)
            .await
    }

    /// IDs name real user entries or the first entry of an assistant response.
    pub async fn history_window(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        message_ids: &[i64],
        limit: usize,
    ) -> Result<ReadView<HistoryPage>> {
        self.0
            .backend
            .history_window(session, before, message_ids, limit)
            .await
    }
    pub async fn message_history(
        &self,
        session: SessionId,
        before: Option<HistoryCursor>,
        message_limit: usize,
        entry_limit: usize,
    ) -> Result<ReadView<MessageHistoryPage>> {
        self.0
            .backend
            .message_history(session, before, message_limit, entry_limit)
            .await
    }
}
