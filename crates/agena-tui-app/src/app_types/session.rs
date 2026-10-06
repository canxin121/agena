use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

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
    /// A built-in this client runs locally. Boxed because the declaration
    /// (docs, input contract) dwarfs the plugin variant.
    BuiltIn(Box<crate::commands::ClientCommand>),
    Plugin(Box<agena_plugin_host::CommandCatalogItem>),
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
        at_message_id: i64,
        message_document: agena_domain::ComposerDocument,
        target: String,
    },
}

/// A completed user message offered as a rewind target. The marker part id
/// is the same message identity accepted by the server.
#[derive(Debug, Clone)]
pub(crate) struct RewindTarget {
    pub(crate) at_message_id: i64,
    pub(crate) sequence: i64,
    pub(crate) message_document: agena_domain::ComposerDocument,
    pub(crate) created_at_ms: i64,
}

impl RewindTarget {
    pub(crate) fn from_run(
        marker: &agena_api::resource::SessionTranscriptPart,
        sequence: i64,
        document: agena_domain::ComposerDocument,
    ) -> Self {
        Self {
            at_message_id: marker.part_id,
            sequence,
            message_document: document,
            created_at_ms: marker.created_at_ms,
        }
    }
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
    pub(crate) refresh_queued: bool,
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
            self.refresh_queued = true;
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
    pub(crate) pending_submit: Option<PendingComposerSubmit>,
}

/// A composer send that is still waiting for its attachment parts. One
/// send is one request, so the deferred action carries no queueing mode.
#[derive(Clone, Copy)]
pub(crate) enum PendingComposerSubmit {
    Send,
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
    pub(crate) reconcile_loaded_parts: bool,
    pub(crate) removed_part_ids: BTreeSet<i64>,
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
    pub(crate) transcript_older_error: Option<String>,
    pub(crate) transcript_older_in_flight_since: Option<Instant>,
    /// Once an older page has been prepended, subsequent recent snapshots must
    /// update only their newest window so they do not discard loaded history.
    pub(crate) transcript_older_pages_loaded: bool,
    pub(crate) transcript_folds: Vec<agena_api::live::SessionTranscriptFoldResource>,
    /// Parts explicitly revealed from a folded reply, including its already
    /// visible tail and run markers. Bounded recent snapshots must retain them.
    pub(crate) transcript_revealed_part_ids: BTreeSet<i64>,
    pub(crate) transcript_fold_loads: BTreeMap<(i64, i64), Instant>,
    pub(crate) transcript_fold_errors: BTreeMap<i64, String>,
    /// One request per detail, tied to both the request and tool state.
    pub(crate) tool_detail_loads:
        BTreeMap<(i64, agena_api::live::ToolDetailSection), (Instant, (String, i64, i64))>,
    pub(crate) tool_detail_tasks:
        BTreeMap<(i64, agena_api::live::ToolDetailSection), tokio::task::AbortHandle>,
    /// Independent activity output, never written into a launch receipt.
    pub(crate) background_output: BTreeMap<i64, String>,
    /// Freshness is separate from the displayed value: live patches retain
    /// rendered details while a replacement is being fetched.
    pub(crate) tool_detail_versions:
        BTreeMap<(i64, agena_api::live::ToolDetailSection), (String, i64, i64)>,
    pub(crate) tool_detail_pending: BTreeSet<(i64, agena_api::live::ToolDetailSection)>,
    pub(crate) tool_detail_allowed_at: BTreeMap<(i64, agena_api::live::ToolDetailSection), Instant>,
    pub(crate) tool_detail_failures: BTreeMap<(i64, agena_api::live::ToolDetailSection), u32>,
    pub(crate) transcript_fold_seen_cursors: BTreeMap<i64, BTreeSet<String>>,
    pub(crate) refresh_failures: u32,
    pub(crate) last_history_load_at: Option<Instant>,
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
    pub(crate) transcript_revealed_part_ids: BTreeSet<i64>,
    pub(crate) node_expansions: BTreeMap<TranscriptNodeKey, bool>,
    pub(crate) activity_summary_visible_counts: BTreeMap<TranscriptNodeKey, usize>,
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
