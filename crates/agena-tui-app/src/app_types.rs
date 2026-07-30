use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use agena_api::{
    pagination::PaginatedResponse,
    resource::{
        MessageResource, ProviderAdapterModelsResponse, ProviderSummaryResource,
        SessionExecutionResource, SessionResource,
    },
};
use agena_domain::ModelRef;
use agena_domain::PermissionMode;
use agena_domain::UsagePeriod;
use agena_domain::UsageStats;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::composer_queue::ComposerQueue;
use agena_application::dto::ModelCatalogListResponse;
use agena_tui::composer::ComposerItemSelection;
use agena_tui::i18n::I18n;
use agena_tui::input::ComposerKeyBindings;
use agena_tui::main_focus::Focus;
use agena_tui::presentation_config::TuiConfig;
use agena_tui::status_line::StatusLinePresentation;
use agena_tui::usage::{UsageDashboardData, UsageDashboardPresentation};
use agena_tui_backend::{Backend, LiveEvent, SessionRefresh};
use agena_tui_components::{Editor, InputDialogState};
use agena_tui_media::{MathGraphicsConfig, MathGraphicsRenderer, MathRenderContext};
use agena_tui_platform::terminal::TerminalContext;
use agena_tui_session::session_list::SessionListPresentation;
use agena_tui_session::session_view::SessionViewMode;
use agena_tui_settings::SettingsStudioSectionId;

use super::{PluginWorkbenchOverlay, cleanup_temporary_composer_items};

mod composer;
pub(crate) use self::composer::*;
mod overlays;
pub(crate) use self::overlays::*;
mod session;
pub(crate) use self::session::*;
pub(crate) use crate::transcript_state::TranscriptViewportRow;
pub(crate) use agena_tui_transcript::{
    LayoutCache, RenderedLine, RenderedTranscript, RenderedTranscriptNode, TranscriptAction,
    TranscriptBlockCursor, TranscriptBlockSelectionMode, TranscriptClick, TranscriptCursor,
    TranscriptCursorAnchor, TranscriptDetailDefaults, TranscriptInteraction,
    TranscriptMoveDirection, TranscriptNodeKey, TranscriptNodeKind, TranscriptPointerGesture,
    TranscriptScrollbarDrag, TranscriptTextPosition, TranscriptTextSelection, TranscriptViewport,
    TranscriptVisualSelectionMode, TranscriptVisualSelectionSnapshot,
};

#[cfg(test)]
pub(crate) use agena_tui_transcript::TranscriptVerticalNavigationStep;

pub(super) const TIMELINE_EVENT_LIMIT: u64 = 200;
// A full ratatui frame can include Markdown layout, syntax highlighting and
// rich tool cards. Ten frames per second keeps spinners responsive without
// continuously re-rendering a large transcript at ~31 FPS while a tool waits.
pub(super) const UI_TICK_MS: u64 = 100;
pub(super) const REFRESH_INTERVAL_MS: u64 = 250;
pub(super) const DRAFT_PERSIST_INTERVAL_MS: u64 = 250;
pub(super) const MAX_FILE_MENTION_SUGGESTIONS: usize = 100;
pub(super) const MAX_PROMPT_HISTORY_ENTRIES: usize = 200;
pub(super) const AWS_REGION_CHOICES: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "ca-central-1",
    "sa-east-1",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-central-1",
    "eu-central-2",
    "eu-north-1",
    "eu-south-1",
    "eu-south-2",
    "ap-south-1",
    "ap-south-2",
    "ap-east-1",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-southeast-3",
    "ap-southeast-4",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "me-south-1",
    "me-central-1",
    "af-south-1",
];

pub(super) const SETTINGS_FIELDS: [SettingsFieldSpec; 15] = [
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ModelsProviders,
        path: "providers.default",
        label_key: "settings-field-default-provider-label",
        description_key: "settings-field-default-provider-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::Interface,
        path: "ui.locale",
        label_key: "settings-field-ui-locale-label",
        description_key: "settings-field-ui-locale-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::Interface,
        path: "ui.tui.color_scheme",
        label_key: "settings-field-tui-color-scheme-label",
        description_key: "settings-field-tui-color-scheme-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::Interface,
        path: "ui.tui.graphics",
        label_key: "settings-field-tui-graphics-label",
        description_key: "settings-field-tui-graphics-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::Interface,
        path: "ui.tui.theme",
        label_key: "settings-field-tui-theme-label",
        description_key: "settings-field-tui-theme-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::Diagnostics,
        path: "tracing.filter",
        label_key: "settings-field-tracing-filter-label",
        description_key: "settings-field-tracing-filter-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::Diagnostics,
        path: "tracing.database",
        label_key: "settings-field-tracing-database-label",
        description_key: "settings-field-tracing-database-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::Diagnostics,
        path: "tracing.adapter",
        label_key: "settings-field-tracing-adapter-label",
        description_key: "settings-field-tracing-adapter-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::RuntimeSession,
        path: "runtime.providers.client_versions.codex",
        label_key: "settings-field-runtime-codex-version-label",
        description_key: "settings-field-runtime-codex-version-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::RuntimeSession,
        path: "runtime.providers.client_versions.claude",
        label_key: "settings-field-runtime-claude-version-label",
        description_key: "settings-field-runtime-claude-version-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::RuntimeSession,
        path: "runtime.providers.client_versions.gemini",
        label_key: "settings-field-runtime-gemini-version-label",
        description_key: "settings-field-runtime-gemini-version-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::RuntimeSession,
        path: "session.compaction.auto",
        label_key: "settings-field-session-compaction-auto-label",
        description_key: "settings-field-session-compaction-auto-description",
        kind: SettingsFieldKind::Bool,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::RuntimeSession,
        path: "session.compaction.reserved_tokens",
        label_key: "settings-field-session-compaction-reserved-tokens-label",
        description_key: "settings-field-session-compaction-reserved-tokens-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::PluginsTools,
        path: "plugins.policy.tool_presentation.default_mode",
        label_key: "settings-plugin-default-mode-label",
        description_key: "settings-plugin-default-mode-detail",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::PluginsTools,
        path: "plugins.policy.ui_presentation.default_mode",
        label_key: "settings-plugin-ui-default-mode-label",
        description_key: "settings-plugin-ui-default-mode-detail",
        kind: SettingsFieldKind::String,
    },
];

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub initial_session_id: Option<i64>,
    pub initial_session_search: Option<String>,
    pub tui_config: TuiConfig,
    pub terminal_background: Option<agena_tui_components::TerminalRgb>,
    pub terminal_context: Option<TerminalContext>,
    pub math_graphics: Option<MathGraphicsConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionLoadScope {
    pub(super) mode: SessionViewMode,
    pub(super) anchor_session_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum DraftSlot {
    Session(i64),
    NewSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RunActivityTarget {
    Session(i64),
    NewSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum RunOperation {
    CreateSession,
    SubmitMessage,
    Continue,
    Compact,
    Rewind,
    PermissionReply,
    UserInputReply,
}

#[derive(Debug, Default)]
pub(super) struct RunActivityTracker {
    requests: BTreeMap<(RunActivityTarget, RunOperation), usize>,
}

impl RunActivityTracker {
    pub(super) fn begin(&mut self, target: RunActivityTarget, operation: RunOperation) {
        *self.requests.entry((target, operation)).or_default() += 1;
    }

    pub(super) fn finish(&mut self, target: RunActivityTarget, operation: RunOperation) {
        let key = (target, operation);
        let Some(count) = self.requests.get_mut(&key) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.requests.remove(&key);
        }
    }

    pub(super) fn clear_session(&mut self, session_id: i64) {
        self.requests.retain(
            |(target, _), _| !matches!(target, RunActivityTarget::Session(id) if *id == session_id),
        );
    }

    pub(super) fn is_active(&self, target: RunActivityTarget) -> bool {
        self.requests
            .keys()
            .any(|(candidate, _)| *candidate == target)
    }

    pub(super) fn has_operation(&self, target: RunActivityTarget, operation: RunOperation) -> bool {
        self.requests.contains_key(&(target, operation))
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct DraftStore {
    pub(super) drafts: BTreeMap<DraftSlot, ComposerDraft>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PromptHistory {
    pub(super) items: Vec<String>,
}

pub struct App {
    pub(super) backend: Backend,
    pub(super) i18n: I18n,
    pub(super) tx: UnboundedSender<AppMessage>,
    pub(super) rx: UnboundedReceiver<AppMessage>,
    pub(super) launch: LaunchOptions,
    pub(super) math_renderer: Option<MathGraphicsRenderer>,
    pub(super) math_render_context: MathRenderContext,
    pub(super) should_quit: bool,
    pub(super) focus: Focus,
    pub(super) current_route: Route,
    pub(super) route_stack: Vec<Route>,
    pub(super) overlay: Option<Overlay>,
    pub(super) overlay_stack: Vec<Overlay>,
    pub(super) context_help: Option<HelpOverlay>,
    pub(super) seen_permission_request_ids: BTreeSet<String>,
    pub(super) seen_user_input_request_ids: BTreeSet<String>,
    pub(super) pending_permission_replay: Option<PermissionReplayState>,
    pub(super) flash: Option<FlashMessage>,
    pub(super) sessions: SessionListPresentation,
    pub(super) session_load: SessionListLoadState,
    pub(super) session_composer: SessionComposerState,
    pub(super) session_controller: agena_tui_session::SessionController,
    pub(super) transcript: TranscriptState,
    pub(super) run_options: RunOptionsState,
    pub(super) composer: Editor,
    pub(super) composer_items: Vec<ComposerItem>,
    pub(super) slash_command_suggestions: Option<SlashCommandSuggestionState>,
    pub(super) slash_command_suggestion_actions: BTreeMap<String, SlashCommandSuggestionAction>,
    pub(super) dismissed_slash_command_suggestions_for: Option<String>,
    pub(super) file_mention_suggestions: Option<FileMentionSuggestionState>,
    pub(super) file_mention_suggestion_actions: BTreeMap<String, FileMentionSuggestionAction>,
    pub(super) dismissed_file_mention_suggestions_for: Option<String>,
    pub(super) prompt_history_search: Option<PromptHistorySearchState>,
    pub(super) composer_item_selection: ComposerItemSelection,
    pub(super) draft_store: DraftStore,
    pub(super) draft_store_path: PathBuf,
    pub(super) draft_store_dirty: bool,
    pub(super) draft_store_last_persist_at: Instant,
    pub(super) draft_store_reported_error: Option<String>,
    pub(super) pending_draft_store_error: Option<String>,
    pub(super) prompt_history: PromptHistory,
    pub(super) prompt_history_path: PathBuf,
    pub(super) prompt_history_reported_error: Option<String>,
    pub(super) pending_prompt_history_error: Option<String>,
    pub(super) run_activity: RunActivityTracker,
    pub(super) next_pending_user_message_id: u64,
    pub(super) layout: LayoutCache,
    pub(super) transcript_scrollbar_drag: Option<TranscriptScrollbarDrag>,
    pub(super) transcript_pointer_gesture: Option<TranscriptPointerGesture>,
    pub(super) last_transcript_click: Option<TranscriptClick>,
    pub(super) mouse_events_seen: u64,
    pub(super) last_mouse_event: Option<String>,
    pub(super) bootstrap_done: bool,
    pub(super) last_refresh_at: Instant,
    pub(super) pending_ui_action: Option<UiAction>,
    pub(super) current_lineage: Option<CurrentLineageState>,
    /// Monotonic id for usage dashboard loads. Keeping it on the app prevents
    /// a late response from an older, closed dashboard matching a newly
    /// opened dashboard that is also on its first request.
    pub(super) next_usage_request_id: u64,
    /// Forwarder task that pumps `Backend::subscribe_session_events` into
    /// [`AppMessage::SessionEventArrived`]. Aborted whenever the active
    /// session changes so we don't accumulate stale subscriptions.
    pub(super) active_subscription: Option<tokio::task::JoinHandle<()>>,
    /// Pending messages typed by the user while the AI was working. Drained
    /// FIFO once the active run finishes. See `composer_queue.rs`.
    pub(super) queue: ComposerQueue,
    pub(super) status_line: Option<StatusLinePresentation>,
    pub(super) plugin_theme: Option<agena_plugin_host::HostThemePalette>,
    pub(super) keybindings: ComposerKeyBindings,
    pub(super) transcript_motion_prefix: Option<String>,
    pub(super) transcript_yank_pending: bool,
    pub(super) transcript_yank_origin: Option<TranscriptTextPosition>,
    pub(super) transcript_goto_pending: bool,
    pub(super) transcript_viewport_pending: bool,
    /// `(forward, till, count)` while waiting for the character in an
    /// `f`/`F`/`t`/`T` command.
    pub(super) transcript_find_pending: Option<(bool, bool, usize)>,
    /// Most recently completed `f`/`F`/`t`/`T` request, used by `;` and `,`.
    pub(super) transcript_last_find: Option<(bool, bool, char)>,
    /// `(yank, around)` after `a` or `i` starts a Transcript text object.
    /// `yank` distinguishes `yam`/`yaM` from `vam`/`vaM`; `around` keeps the
    /// normal Vim distinction for built-in text objects such as `aw`/`iw`.
    pub(super) transcript_text_object_pending: Option<(bool, bool)>,
    /// Direction selected when the transcript search overlay was opened.
    pub(super) transcript_search_forward: bool,
    /// Last time the user pressed Ctrl+C; a second press within the window
    /// exits the application.
    pub(super) last_ctrl_c_at: Option<Instant>,
    pub(super) double_esc_window: Duration,
}

impl Drop for App {
    fn drop(&mut self) {
        self.sync_current_draft_slot();
        let _ = self.try_persist_draft_store(true);
        cleanup_temporary_composer_items(self.composer_items.as_slice());
        self.cleanup_temporary_draft_store_items();
    }
}

#[derive(Debug, Clone)]
pub(super) enum AppMessage {
    UsageStatsLoaded {
        request_id: u64,
        result: UiResult<UsageStats>,
    },
    SessionsLoaded {
        scope: SessionLoadScope,
        subtree_root_id: Option<i64>,
        result: UiResult<Vec<SessionResource>>,
    },
    SessionCreated {
        submit_draft: Option<ComposerDraft>,
        pending_message_id: Option<u64>,
        result: UiResult<SessionResource>,
    },
    SessionStateLoaded {
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    },
    SessionRefreshed {
        session_id: i64,
        result: UiResult<SessionRefresh>,
    },
    SessionMessageSubmitted {
        session_id: i64,
        pending_message_id: u64,
        draft: ComposerDraft,
        result: UiResult<SessionExecutionResource>,
    },
    SessionContinued {
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    },
    SessionCompacted {
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    },
    SessionRenamed {
        session_id: i64,
        result: UiResult<SessionResource>,
    },
    PermissionReplied {
        session_id: i64,
        label: String,
        result: UiResult<SessionExecutionResource>,
    },
    UserInputReplied {
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    },
    ProvidersLoaded {
        purpose: ProviderPickerPurpose,
        result: UiResult<Vec<ProviderSummaryResource>>,
    },
    SessionSearchPageLoaded {
        mode: SessionViewMode,
        query: String,
        page_index: usize,
        result: UiResult<PaginatedResponse<SessionResource>>,
    },
    SessionSearchSubtreeLoaded {
        session_id: i64,
        query: String,
        result: UiResult<Vec<SessionResource>>,
    },
    LineageLoaded {
        session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    },
    RewindMessagesLoaded {
        session_id: i64,
        result: UiResult<Vec<MessageResource>>,
    },
    ModelCatalogLoaded {
        query: String,
        offset: usize,
        result: UiResult<ModelCatalogListResponse>,
    },
    ProviderStudioAdapterModelsLoaded {
        request_key: String,
        result: UiResult<ProviderAdapterModelsResponse>,
    },
    ProviderStudioAuthCompleted {
        request_key: String,
        result: std::result::Result<
            agena_tui_backend::ProviderDraftAuthActionResult,
            agena_tui_backend::ProviderDraftAuthError,
        >,
    },
    ProviderStudioSaved {
        provider_id: String,
        result: std::result::Result<
            agena_tui_backend::ProviderStudioSaveResult,
            agena_tui_backend::ProviderStudioSaveError,
        >,
    },
    ModelCatalogRefreshed {
        result: UiResult<()>,
    },
    ChildSessionsLoaded {
        parent_session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    },
    TimelineLoaded {
        session_id: i64,
        result: UiResult<Vec<agena_runtime::RuntimeTimelineEvent>>,
    },
    SessionRewound {
        session_id: i64,
        message_text: String,
        target: String,
        result: UiResult<SessionExecutionResource>,
    },
    /// Pushed by the unified event bus (`Backend::subscribe_session_events`).
    /// Callers receive each domain event in real time, with a hint about
    /// whether a refresh is needed.
    SessionEventArrived {
        session_id: i64,
        live: LiveEvent,
    },
    /// Result of a `request_steer_input` call. `result` is `Ok` when the
    /// backend accepted the steer; `Err` when the run was no longer
    /// steerable (e.g. terminal phase). On error we re-enqueue the draft
    /// so the user's intent isn't dropped.
    SteerSubmitted {
        session_id: i64,
        pending_message_id: u64,
        draft: ComposerDraft,
        result: UiResult<()>,
    },
    /// Result of a `request_cancel_run` call. We always treat the in-flight
    /// run as gone when this lands, regardless of success — the user has
    /// already signalled cancel intent.
    RunCancelled {
        session_id: i64,
        result: UiResult<()>,
    },
    StatusLineUpdated {
        output: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub(super) enum UiAction {
    CopyText { text: String, success: String },
    EditComposerExternally,
    DownloadTerminalFile { path: PathBuf },
    ExportTranscript { path: Option<PathBuf> },
    OpenPath { path: PathBuf },
    PageTranscript,
}

pub(super) type UiResult<T> = std::result::Result<T, String>;

#[derive(Debug, Clone)]
pub(super) enum Overlay {
    TranscriptSearch(LineInputOverlay),
    SessionRename(LineInputOverlay),
    SettingsValueEdit(SettingsValueEditOverlay),
    Choice(ChoiceOverlay),
    PathBrowser(PathBrowserOverlay),
    Permission(PermissionOverlay),
    UserInputReply(UserInputOverlay),
    Confirm(ConfirmOverlay),
    SessionSearch(SessionSearchOverlay),
    Timeline(TimelineOverlay),
    ProviderStudio(Box<ProviderStudioOverlay>),
    ModelCatalogStudio(ModelCatalogStudioOverlay),
}

#[derive(Debug, Clone)]
pub(super) enum Route {
    Main,
    Usage(UsageDashboardState),
    SettingsStudio(SettingsStudioOverlay),
    PermissionStudio(PermissionStudioOverlay),
    PermissionRuleStudio(PermissionRuleStudioOverlay),
    SessionSearch(SessionSearchOverlay),
    CommandPalette(CommandPaletteOverlay),
    SkillPicker(SkillPickerOverlay),
    SkillStudio(SkillStudioOverlay),
    SessionNavigation(SessionNavigationOverlay),
    SelectionPicker(SelectionPickerOverlay),
    SessionModelChooser(SessionModelChooserOverlay),
    Timeline(TimelineOverlay),
    PluginWorkbench(Box<PluginWorkbenchOverlay>),
    ProviderStudio(Box<ProviderStudioOverlay>),
    ModelCatalogStudio(ModelCatalogStudioOverlay),
}

#[derive(Debug, Clone)]
pub(super) struct UsageDashboardState {
    pub(super) period: UsagePeriod,
    pub(super) presentation: UsageDashboardPresentation,
    pub(super) available_providers: Vec<String>,
    pub(super) available_models: Vec<(String, String)>,
    pub(super) data: Option<UsageDashboardData>,
    pub(super) loading: bool,
    pub(super) request_id: u64,
    pub(super) error: Option<String>,
}

impl UsageDashboardState {
    pub(super) fn new(period: UsagePeriod) -> Self {
        Self {
            period,
            presentation: UsageDashboardPresentation::new(),
            available_providers: Vec::new(),
            available_models: Vec::new(),
            data: None,
            loading: false,
            request_id: 0,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DialogHost {
    Route,
    Overlay,
}

pub(super) type LineInputOverlay = InputDialogState<()>;

pub(super) use agena_tui::help::{HelpOverlay, HelpOverlayKind};

pub(super) use agena_tui_components::{
    HelpDialogEntry as HelpEntry, HelpDialogSection as HelpSection,
};
