use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use crate::commands::CommandSpec;

use super::{
    ComposerDraft, I18n, MathRenderContext, ModelRef, RenderedTranscript, SessionExecutionResource,
    SessionLoadScope, TranscriptDetailDefaults, TranscriptInteraction, TranscriptNodeKey,
    TranscriptTextPosition, TranscriptViewport,
};
#[cfg(test)]
use agena_api::resource::RunResource;
pub(crate) use agena_tui_session::session_hub::{
    SessionHubItem, SessionHubPresentation, SessionHubSection, SessionHubSectionKind,
};
pub(crate) use agena_tui_session::session_search::{SessionSearchItem, SessionSearchOverlay};

/// App-owned concrete effect map for the TUI-owned generic selection picker.
/// The TUI sees only opaque display keys; configuration/session effects remain
/// App-owned and are selected through this map.
#[derive(Debug, Clone)]
pub(crate) struct SelectionPickerOverlay {
    pub(crate) presentation: agena_tui::selection_picker::SelectionPickerPresentation,
    pub(crate) query: SelectionPickerQuery,
    pub(crate) actions: BTreeMap<String, SelectionPickerCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionPickerQuery {
    Providers(ProviderPickerPurpose),
}

#[derive(Debug, Clone)]
pub(crate) enum SelectionPickerCommand {
    ProviderCreate,
    Provider { provider_id: String },
}

/// App-owned concrete effect map for the TUI-owned command-palette
/// presentation. Rows and search behavior stay in `agena_tui`; only command
/// execution payloads remain here.
#[derive(Debug, Clone)]
pub(crate) struct CommandPaletteOverlay {
    pub(crate) presentation: agena_tui::command_palette::CommandPalettePresentation,
    pub(crate) actions: BTreeMap<String, CommandPaletteCommand>,
}

#[derive(Debug, Clone)]
pub(crate) enum CommandPaletteCommand {
    BuiltIn(&'static CommandSpec),
    Plugin(Box<agena_plugin_host::PluginOperationCatalogItem>),
}

/// App-owned Skill catalog page. The presentation remains a generic search
/// picker, while the App keeps the concrete lazy references returned by the
/// catalog. Selecting a Skill does not read its body.
#[derive(Debug, Clone)]
pub(crate) struct SkillPickerOverlay {
    pub(crate) presentation: agena_tui::selection_picker::SelectionPickerPresentation,
    pub(crate) session_id: i64,
    pub(crate) actions: BTreeMap<String, SkillPickerReference>,
    pub(crate) offset: usize,
    pub(crate) total: usize,
    pub(crate) limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkillPickerReference {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) content_hash: String,
    pub(crate) source: String,
}

/// Workspace Skill management surface. The catalog can display every
/// discovered Skill, but `editable` is deliberately limited to the
/// workspace-owned `.agena/skills` documents exposed by the Skills plugin.
#[derive(Debug, Clone)]
pub(crate) struct SkillStudioOverlay {
    pub(crate) presentation: agena_tui::selection_picker::SelectionPickerPresentation,
    pub(crate) actions: BTreeMap<String, SkillStudioItem>,
    pub(crate) detail: Option<SkillStudioDetail>,
    pub(crate) editor: Option<SkillStudioEditor>,
    pub(crate) offset: usize,
    pub(crate) total: usize,
    pub(crate) limit: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillStudioItem {
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) summary: String,
    pub(crate) aliases: Vec<String>,
    pub(crate) source: String,
    pub(crate) editable: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SkillStudioDetail {
    pub(crate) item: SkillStudioItem,
    pub(crate) document: String,
    pub(crate) scroll: u16,
}

pub(crate) type SkillStudioEditor =
    agena_tui_components::EditorDialogState<SkillStudioEditorAction>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SkillStudioEditorAction {
    Create,
    Update { name: String },
}

/// App-owned concrete effect map for the TUI-owned session-navigation
/// presentation. Runtime resources are projected into opaque TUI rows; the
/// map remains here because opening a session and confirming a rewind are
/// concrete application effects.
#[derive(Debug, Clone)]
pub(crate) struct SessionNavigationOverlay {
    pub(crate) presentation: agena_tui_session::session_navigation::SessionNavigationPresentation,
    pub(crate) query: SessionNavigationQuery,
    pub(crate) actions: BTreeMap<String, SessionNavigationCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionNavigationQuery {
    Lineage { session_id: i64 },
    RewindMessages { session_id: i64 },
    ChildSessions { parent_session_id: i64 },
}

#[derive(Debug, Clone)]
pub(crate) enum SessionNavigationCommand {
    OpenSession {
        session_id: i64,
    },
    Rewind {
        session_id: i64,
        turn_id: agena_domain::TurnId,
        message_text: String,
        target: String,
    },
}

/// A user-run boundary offered as a rewind target, derived from the v2 parts
/// projection (one `run` marker with role `user`, plus its text parts).
///
/// The backend rewind contract (agena-api B1) still takes
/// `agena_domain::TurnId`; the v2 part list carries only `part_id`. The TUI
/// synthesizes a deterministic `TurnId` from the run marker's `part_id` (the
/// v2 turn identity, database-design-v2.md §4.1.1) so navigation keys stay
/// stable across refreshes. Bridging that id to the runtime's rewind target is
/// the A3 migration's contract.
#[derive(Debug, Clone)]
pub(crate) struct RewindTarget {
    pub(crate) turn_id: agena_domain::TurnId,
    pub(crate) sequence: i64,
    pub(crate) message_text: String,
    pub(crate) created_at_ms: i64,
}

impl RewindTarget {
    pub(crate) fn from_run(
        marker: &agena_api::resource::SessionTranscriptPart,
        sequence: i64,
        text: &str,
    ) -> Self {
        Self {
            turn_id: turn_id_for_run(marker.part_id),
            sequence,
            message_text: text.trim().to_owned(),
            created_at_ms: marker.created_at_ms,
        }
    }
}

/// Deterministic `TurnId` for a run marker. The v2 part list carries only the
/// run marker's `part_id` (the v2 turn identity); the backend rewind contract
/// still takes a `TurnId`, so one is synthesized from the part id to keep
/// navigation keys stable across refreshes. See [`RewindTarget`].
fn turn_id_for_run(part_id: i64) -> agena_domain::TurnId {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(b"agena-rw");
    bytes[8..].copy_from_slice(&part_id.to_be_bytes());
    agena_domain::TurnId(uuid::Uuid::from_bytes(bytes))
}

pub(crate) use agena_tui::model_chooser::{
    SessionModelChoiceItem, SessionModelChooserOverlay, SessionModelChooserPurpose,
    SessionModelIdentity,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderPickerPurpose {
    Configure,
}

#[derive(Debug, Clone)]
pub(crate) struct CurrentLineageState {
    pub(crate) session_id: i64,
    pub(crate) summary: agena_tui_session::session_navigation::SessionLineageSummary,
}

pub(crate) use agena_notification::{
    NotificationScope as NoticeScope, NotificationSeverity as NoticeSeverity, NotificationSurface,
};

/// Runtime query lifecycle for the TUI-owned session-list presentation.
///
/// Session rows, hierarchy, filtering, and selection live in
/// `agena_tui_session::session_list::SessionListPresentation`; this App state only
/// tracks an in-flight concrete Runtime request.
#[derive(Default)]
pub(crate) struct SessionListLoadState {
    pub(crate) pending_scope: Option<SessionLoadScope>,
    pub(crate) loading: bool,
    /// When the current session-list request was issued. A request whose
    /// response is lost would otherwise leave `loading` set forever, and
    /// `request_sessions` coalesces every later request while it is set —
    /// freezing the session list at stale rows. The stall timer recovers it.
    pub(crate) requested_at: Option<Instant>,
    pub(crate) initialized: bool,
}

impl SessionListLoadState {
    /// Clear an in-flight session-list request that has exceeded `timeout` so
    /// the next `request_sessions` can proceed. Returns true when recovered.
    pub(crate) fn recover_stalled_request(&mut self, timeout: Duration) -> bool {
        if self.loading
            && self
                .requested_at
                .is_some_and(|requested_at| requested_at.elapsed() >= timeout)
        {
            self.loading = false;
            self.pending_scope = None;
            self.requested_at = None;
            return true;
        }
        false
    }
}

/// Composer recovery belongs to the session submit lifecycle rather than the
/// transcript presentation state.
#[derive(Default)]
pub(crate) struct SessionComposerState {
    pub(crate) pending_restore_draft: Option<ComposerDraft>,
}

pub(crate) struct TranscriptState {
    pub(crate) i18n: I18n,
    pub(crate) math_render_context: MathRenderContext,
    pub(crate) session_id: Option<i64>,
    pub(crate) session_title: String,
    #[cfg(test)]
    pub(crate) messages: Vec<RunResource>,
    /// The session's canonical part transcript (ordered parts, including
    /// `run` markers), mirroring `SessionExecutionResource.parts`.
    pub(crate) parts: Vec<agena_api::resource::SessionTranscriptPart>,
    /// Last failure observed per run marker. A continuation run clears the
    /// runtime failure projection when the reply recovers, but the chat keeps
    /// the last failure so the error Activity remains visible. Keyed by the
    /// run marker's `part_id`.
    pub(crate) reply_failures: BTreeMap<i64, agena_failure::UserProblem>,
    pub(crate) pending_user_messages: Vec<PendingUserMessage>,
    pub(crate) refreshing: bool,
    pub(crate) state_loading: bool,
    /// When the current session refresh was issued; `None` while idle. A
    /// refresh whose response never arrives leaves `refreshing` set and
    /// blocks every later refresh, freezing the transcript at a stale
    /// snapshot. `recover_stalled_requests` clears the wedge.
    pub(crate) refresh_in_flight_since: Option<Instant>,
    /// When the current session-state load was issued. Same recovery
    /// contract as `refresh_in_flight_since`.
    pub(crate) state_load_in_flight_since: Option<Instant>,
    /// Cursor for the next older transcript page. The initial session load
    /// owns only the newest bounded page; older pages are fetched on demand
    /// when the user reaches the top of the viewport.
    pub(crate) transcript_next_cursor: Option<String>,
    pub(crate) transcript_has_more: bool,
    pub(crate) transcript_older_loading: bool,
    pub(crate) transcript_older_in_flight_since: Option<Instant>,
    /// Once an older page has been prepended, subsequent recent snapshots must
    /// update only their newest window so they do not discard loaded history.
    pub(crate) transcript_older_pages_loaded: bool,
    pub(crate) transcript_folds: Vec<agena_api::live::SessionTranscriptFoldResource>,
    pub(crate) viewport: TranscriptViewport,
    pub(crate) interaction: TranscriptInteraction,
    pub(crate) search_query: String,
    pub(crate) search_match_index: Option<usize>,
    /// Vim-style jump list for `Ctrl+O`/`Ctrl+I`. Each entry records where a
    /// large navigation jump started so the user can return to it.
    pub(crate) jump_history: Vec<TranscriptTextPosition>,
    /// Index of the current position inside [`Self::jump_history`].
    pub(crate) jump_history_index: usize,
    pub(crate) execution: Option<SessionExecutionResource>,
    pub(crate) last_event_seq: Option<i64>,
    pub(crate) detail_expanded_by_default: TranscriptDetailDefaults,
    pub(crate) node_expansions: BTreeMap<TranscriptNodeKey, bool>,
    /// Number of older activities currently revealed for each folded
    /// assistant run. The renderers keep the newest few visible by default;
    /// repeated expansion reveals another bounded chunk.
    pub(crate) activity_summary_visible_counts: BTreeMap<TranscriptNodeKey, usize>,
    /// Operation Activity ids whose detail the user has expanded. Live
    /// streaming deltas are only applied to these — collapsing an Activity
    /// removes it, so computation and transfer for its detail stop.
    pub(crate) expanded_operation_activity_ids: BTreeSet<agena_domain::ActivityId>,
    /// Live activity-v2 overlays driven by `ActivityLiveEvent`s. Pure
    /// in-memory presentation state: never part of the persisted snapshot
    /// and cleared when the session resets. Each entry carries the terminal
    /// state-node fields plus merged live `ViewBlock`s for expanded
    /// rendering (07 §5.2).
    pub(crate) v2_activities: BTreeMap<agena_domain::ActivityId, V2LiveActivity>,
    /// Live selection snapshots for pending user-input interaction parts,
    /// keyed by `request_id`. The App projects these from its
    /// `UserInputPresentation` state (selected option, custom draft, answer
    /// markers) while the plan body and decision labels come from the wire
    /// `request`; the renderer draws them natively inside expanded pending
    /// interaction parts ("everything is a part").
    pub(crate) interaction_views: BTreeMap<String, agena_tui_transcript::PendingInteractionView>,
    pub(crate) rendered: Option<RenderedTranscript>,
}

/// Session-scoped in-memory transcript cache. The TUI keeps this across
/// session switches so scrolling back and forth does not refetch the same
/// cursor pages; dropping the App drops the cache with the TUI process.
#[derive(Debug, Clone)]
pub(crate) struct TranscriptCache {
    pub(crate) parts: Vec<agena_api::resource::SessionTranscriptPart>,
    pub(crate) reply_failures: BTreeMap<i64, agena_failure::UserProblem>,
    pub(crate) transcript_next_cursor: Option<String>,
    pub(crate) transcript_has_more: bool,
    pub(crate) transcript_older_pages_loaded: bool,
    pub(crate) transcript_folds: Vec<agena_api::live::SessionTranscriptFoldResource>,
    pub(crate) node_expansions: BTreeMap<TranscriptNodeKey, bool>,
    pub(crate) activity_summary_visible_counts: BTreeMap<TranscriptNodeKey, usize>,
    pub(crate) expanded_operation_activity_ids: BTreeSet<agena_domain::ActivityId>,
}

/// A live activity-v2 overlay entry. Activity-v2 events are broadcast in
/// memory only, so the live detail is owned here and merged from
/// `ActivityLiveEvent::DetailDelta` blocks.
#[derive(Debug, Clone)]
pub(crate) struct V2LiveActivity {
    pub(crate) activity_id: agena_domain::ActivityId,
    pub(crate) title: String,
    pub(crate) state: agena_domain::ActivityState,
    pub(crate) live_blocks: Vec<agena_domain::ViewBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PendingUserMessage {
    pub(crate) id: u64,
    pub(crate) document: agena_domain::ComposerDocument,
    pub(crate) confirmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionActivity {
    Idle,
    Running,
    AwaitingPermission,
    AwaitingInteraction,
    NeedsRecovery,
}

impl SessionActivity {
    pub(crate) fn is_busy(self) -> bool {
        matches!(
            self,
            Self::Running | Self::AwaitingPermission | Self::AwaitingInteraction
        )
    }

    pub(crate) fn is_running(self) -> bool {
        self == Self::Running
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RunOptionsState {
    pub(crate) model: Option<ModelRef>,
    pub(crate) thinking_mode: Option<String>,
    pub(crate) speed_mode: Option<String>,
    pub(crate) verbosity: Option<String>,
    pub(crate) parallel_tool_calls: Option<bool>,
}
