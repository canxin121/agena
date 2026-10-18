//! One provider request's frozen input identities, control outcomes and content
//! boundaries. Resource bodies are read separately at the captured committed
//! heads, so a later shared-stream append cannot enter an earlier request.

use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct PromptTool {
    pub id: ToolId,
    pub execution_kind: ToolExecutionKind,
    pub call: ToolStart,
    pub phase: ToolPhase,
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptEntry {
    pub entry: EntryView,
    pub round_id: Option<RoundId>,
    pub tool: Option<PromptTool>,
    pub interaction: Option<InteractionView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptReplay {
    pub round_id: RoundId,
    pub phase: ProviderRoundPhase,
    pub provider_state: Option<serde_json::Value>,
}

/// Bounded facts for a recorded provider input. This is an input manifest,
/// including opaque round replay; provider-specific projection and content
/// hydration remain orchestration responsibilities. A read never adds entries
/// committed after the round's input boundary.
#[derive(Debug, Clone, Serialize)]
pub struct PromptView {
    pub round_id: RoundId,
    pub execution_id: ExecutionId,
    pub generation: AttemptGeneration,
    pub metadata: SessionMetadata,
    pub entries: Vec<PromptEntry>,
    pub replays: Vec<PromptReplay>,
    pub sources: Vec<ContentStreamView>,
}

impl RoundHandle {
    pub async fn prompt(&self) -> Result<ReadView<PromptView>> {
        self.attempt
            .inner
            .backend
            .prompt(self.attempt.scope.session, self.id)
            .await
    }
}

impl KernelReader {
    /// Read a byte range at an earlier committed head. Uncommitted live tails
    /// and later checkpoints are excluded. Retention/restart still return an
    /// explicit gap/reset; callers never silently mix a newer source epoch.
    pub async fn text_range_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
        after: Option<TextCursor>,
        limit: usize,
    ) -> Result<ReadView<TextRange>> {
        self.0
            .backend
            .text_range_at(session, boundary, after, limit)
            .await
    }

    pub async fn event_page_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
        after: Option<EventCursor>,
        limit: usize,
    ) -> Result<ReadView<EventPage>> {
        self.0
            .backend
            .event_page_at(session, boundary, after, limit)
            .await
    }

    pub async fn event_snapshot_at(
        &self,
        session: SessionId,
        boundary: &ContentStreamView,
    ) -> Result<ReadView<EventSnapshot>> {
        self.0.backend.event_snapshot_at(session, boundary).await
    }
}
