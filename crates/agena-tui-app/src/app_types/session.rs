use std::collections::BTreeMap;

use crate::commands::CommandSpec;

use super::{
    ComposerDraft, I18n, MathRenderContext, MessageResource, ModelRef, RenderedTranscript,
    SessionExecutionResource, SessionLoadScope, TranscriptDetailDefaults, TranscriptInteraction,
    TranscriptNodeKey, TranscriptViewport,
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
    Plugin(Box<agena_plugin_host::PluginCommandCatalogItem>),
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
        message_id: i64,
        message_text: String,
        target: String,
    },
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

pub(crate) use agena_tui::flash::{FlashLevel, FlashMessage};

/// Runtime query lifecycle for the TUI-owned session-list presentation.
///
/// Session rows, hierarchy, filtering, and selection live in
/// `agena_tui_session::session_list::SessionListPresentation`; this App state only
/// tracks an in-flight concrete Runtime request.
#[derive(Default)]
pub(crate) struct SessionListLoadState {
    pub(crate) pending_scope: Option<SessionLoadScope>,
    pub(crate) loading: bool,
    pub(crate) initialized: bool,
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
    pub(crate) messages: Vec<MessageResource>,
    pub(crate) pending_user_messages: Vec<PendingUserMessage>,
    pub(crate) older_cursor: Option<String>,
    pub(crate) has_more_older: bool,
    pub(crate) loading_initial: bool,
    pub(crate) loading_older: bool,
    pub(crate) refreshing: bool,
    pub(crate) state_loading: bool,
    pub(crate) viewport: TranscriptViewport,
    pub(crate) interaction: TranscriptInteraction,
    pub(crate) search_query: String,
    pub(crate) search_match_index: Option<usize>,
    pub(crate) execution: Option<SessionExecutionResource>,
    pub(crate) last_event_seq: Option<i64>,
    pub(crate) detail_expanded_by_default: TranscriptDetailDefaults,
    pub(crate) node_expansions: BTreeMap<TranscriptNodeKey, bool>,
    pub(crate) rendered: Option<RenderedTranscript>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingUserMessage {
    pub(crate) id: u64,
    pub(crate) text: String,
    pub(crate) confirmed: bool,
    pub(crate) persisted_message_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionActivity {
    Idle,
    Running,
    AwaitingPermission,
    AwaitingUserInput,
    Blocked,
}

impl SessionActivity {
    pub(crate) fn is_busy(self) -> bool {
        self != Self::Idle
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
