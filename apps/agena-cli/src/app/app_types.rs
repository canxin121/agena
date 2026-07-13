use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, Instant},
};

use agena::{
    agents::AgentDescriptor,
    event::DomainEvent,
    model::ModelRef,
    permission::PermissionMode,
    session::{UsagePeriod, UsageStats},
};
use agena_api::{
    pagination::PaginatedResponse,
    resource::{
        MessageResource, ProviderAdapterModelsResponse, ProviderSummaryResource,
        SessionExecutionResource, SessionResource,
    },
};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::backend::{Backend, LiveEvent, SessionRefresh};
use crate::composer_queue::ComposerQueue;
use crate::i18n::I18n;
use crate::math_render::{MathGraphicsConfig, MathGraphicsRenderer, MathRenderContext};
use crate::terminal::TerminalContext;
use crate::tui_config::{TuiConfig, TuiStatusLineConfig};
use crate::tui_keymap::ComposerKeyBindings;
use agena_api_server::local_api::ModelCatalogListResponse;
use agena_tui_components::{Editor, InputDialogState};

use super::{
    PluginPolicyStudioOverlay, PluginWorkbenchOverlay, RenderedTranscriptNode,
    TranscriptBlockCursor, TranscriptNodeKey, cleanup_temporary_composer_items,
    persistent_draft_store_version,
};

mod composer;
pub(crate) use self::composer::*;
mod overlays;
pub(crate) use self::overlays::*;
mod session;
pub(crate) use self::session::*;

pub(super) const MESSAGE_PAGE_SIZE: u64 = 40;
pub(super) const TIMELINE_EVENT_LIMIT: u64 = 200;
pub(super) const UI_TICK_MS: u64 = 32;
pub(super) const REFRESH_INTERVAL_MS: u64 = 250;
pub(super) const DRAFT_PERSIST_INTERVAL_MS: u64 = 250;
pub(super) const TOOL_CARD_PREVIEW_LINES: usize = 8;
pub(super) const TOOL_CARD_PREVIEW_CHARS: usize = 2_500;
pub(super) const TOOL_EXPANDED_PREVIEW_LINES: usize = 40;
pub(super) const TOOL_EXPANDED_PREVIEW_CHARS: usize = 12_000;
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

pub(super) const PLUGIN_TOOL_PRESENTATION_PATH: &str = "plugins.policy.tool_presentation";
#[allow(dead_code)]
pub(super) const PLUGIN_TOOL_PRESENTATION_DEFAULT_MODE_PATH: &str =
    "plugins.policy.tool_presentation.default_mode";
pub(super) const PLUGIN_UI_PRESENTATION_PATH: &str = "plugins.policy.ui_presentation";
pub(super) const PLUGIN_UI_PRESENTATION_DEFAULT_MODE_PATH: &str =
    "plugins.policy.ui_presentation.default_mode";
pub(super) const SETTINGS_FIELDS: [SettingsFieldSpec; 28] = [
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigProviders,
        path: "providers.default",
        label_key: "settings-field-default-provider-label",
        description_key: "settings-field-default-provider-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigAgents,
        path: "agents.default",
        label_key: "settings-field-default-agent-label",
        description_key: "settings-field-default-agent-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigUi,
        path: "ui.locale",
        label_key: "settings-field-ui-locale-label",
        description_key: "settings-field-ui-locale-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigUi,
        path: "ui.tui.color_scheme",
        label_key: "settings-field-tui-color-scheme-label",
        description_key: "settings-field-tui-color-scheme-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigUi,
        path: "ui.tui.theme",
        label_key: "settings-field-tui-theme-label",
        description_key: "settings-field-tui-theme-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigTracing,
        path: "tracing.filter",
        label_key: "settings-field-tracing-filter-label",
        description_key: "settings-field-tracing-filter-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigTracing,
        path: "tracing.database",
        label_key: "settings-field-tracing-database-label",
        description_key: "settings-field-tracing-database-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigTracing,
        path: "tracing.adapter",
        label_key: "settings-field-tracing-adapter-label",
        description_key: "settings-field-tracing-adapter-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.client_versions.codex",
        label_key: "settings-field-runtime-codex-version-label",
        description_key: "settings-field-runtime-codex-version-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.client_versions.claude",
        label_key: "settings-field-runtime-claude-version-label",
        description_key: "settings-field-runtime-claude-version-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.client_versions.gemini",
        label_key: "settings-field-runtime-gemini-version-label",
        description_key: "settings-field-runtime-gemini-version-description",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.http.timeout_secs",
        label_key: "settings-field-runtime-provider-http-timeout-label",
        description_key: "settings-field-runtime-provider-http-timeout-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.http.connect_timeout_secs",
        label_key: "settings-field-runtime-provider-http-connect-timeout-label",
        description_key: "settings-field-runtime-provider-http-connect-timeout-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.retry.max_retries",
        label_key: "settings-field-runtime-request-retry-max-retries-label",
        description_key: "settings-field-runtime-request-retry-max-retries-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.retry.base_delay_ms",
        label_key: "settings-field-runtime-request-retry-base-delay-label",
        description_key: "settings-field-runtime-request-retry-base-delay-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.retry.max_delay_ms",
        label_key: "settings-field-runtime-request-retry-max-delay-label",
        description_key: "settings-field-runtime-request-retry-max-delay-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.stream_replay.max_retries_after_output",
        label_key: "settings-field-runtime-stream-replay-max-retries-after-output-label",
        description_key: "settings-field-runtime-stream-replay-max-retries-after-output-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.providers.stream_replay.max_tracked_events",
        label_key: "settings-field-runtime-stream-replay-max-tracked-events-label",
        description_key: "settings-field-runtime-stream-replay-max-tracked-events-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.reload.enabled",
        label_key: "settings-field-runtime-reload-enabled-label",
        description_key: "settings-field-runtime-reload-enabled-description",
        kind: SettingsFieldKind::Bool,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.reload.poll_interval_secs",
        label_key: "settings-field-runtime-reload-poll-interval-label",
        description_key: "settings-field-runtime-reload-poll-interval-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.model_catalog.cache_max_age_secs",
        label_key: "settings-field-runtime-model-catalog-cache-max-age-label",
        description_key: "settings-field-runtime-model-catalog-cache-max-age-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.session.cache.max_sessions",
        label_key: "settings-field-runtime-session-cache-max-sessions-label",
        description_key: "settings-field-runtime-session-cache-max-sessions-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.session.cache.ttl_secs",
        label_key: "settings-field-runtime-session-cache-ttl-label",
        description_key: "settings-field-runtime-session-cache-ttl-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.session.cache.max_bytes",
        label_key: "settings-field-runtime-session-cache-max-bytes-label",
        description_key: "settings-field-runtime-session-cache-max-bytes-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.session.gc.enabled",
        label_key: "settings-field-runtime-session-gc-enabled-label",
        description_key: "settings-field-runtime-session-gc-enabled-description",
        kind: SettingsFieldKind::Bool,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigRuntime,
        path: "runtime.session.gc.interval_secs",
        label_key: "settings-field-runtime-session-gc-interval-label",
        description_key: "settings-field-runtime-session-gc-interval-description",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigSession,
        path: "session.compaction.auto",
        label_key: "settings-field-session-compaction-auto-label",
        description_key: "settings-field-session-compaction-auto-description",
        kind: SettingsFieldKind::Bool,
    },
    SettingsFieldSpec {
        section: SettingsStudioSectionId::ConfigSession,
        path: "session.compaction.reserved_tokens",
        label_key: "settings-field-session-compaction-reserved-tokens-label",
        description_key: "settings-field-session-compaction-reserved-tokens-description",
        kind: SettingsFieldKind::Integer,
    },
];

pub(super) const RUNTIME_SETTINGS: [RuntimeSettingSpec; 7] = [
    RuntimeSettingSpec {
        id: RuntimeSettingId::ThinkingMode,
        kind: SettingsFieldKind::String,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::SpeedMode,
        kind: SettingsFieldKind::String,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::Verbosity,
        kind: SettingsFieldKind::String,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::ParallelToolCalls,
        kind: SettingsFieldKind::Bool,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::Temperature,
        kind: SettingsFieldKind::Float,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::MaxOutput,
        kind: SettingsFieldKind::Integer,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::System,
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
    pub(crate) math_graphics: Option<MathGraphicsConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum SessionViewMode {
    #[default]
    All,
    Roots,
    Subtree,
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

#[derive(Debug, Clone)]
pub(super) struct StatusLineState {
    pub(super) command: String,
    pub(super) refresh_interval: Duration,
    pub(super) next_refresh_at: Instant,
    pub(super) text: Option<String>,
    pub(super) refresh_in_flight: bool,
}

impl StatusLineState {
    pub(super) fn new(config: &TuiStatusLineConfig) -> Option<Self> {
        let command = config.command.as_ref()?.clone();
        Some(Self {
            command,
            refresh_interval: Duration::from_millis(config.refresh_interval_ms),
            next_refresh_at: Instant::now(),
            text: None,
            refresh_in_flight: false,
        })
    }
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
    pub(super) sessions: SessionListState,
    pub(super) transcript: TranscriptState,
    pub(super) run_options: RunOptionsState,
    pub(super) composer: Editor,
    pub(super) composer_items: Vec<ComposerItem>,
    pub(super) slash_command_suggestions: Option<SlashCommandSuggestionState>,
    pub(super) dismissed_slash_command_suggestions_for: Option<String>,
    pub(super) file_mention_suggestions: Option<FileMentionSuggestionState>,
    pub(super) dismissed_file_mention_suggestions_for: Option<String>,
    pub(super) prompt_history_search: Option<PromptHistorySearchState>,
    pub(super) selected_composer_item: Option<usize>,
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
    pub(super) status_line: Option<StatusLineState>,
    pub(super) plugin_theme: Option<agena::plugin::HostThemePalette>,
    pub(super) keybindings: ComposerKeyBindings,
    pub(super) transcript_motion_prefix: Option<String>,
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
        if let Some(draft) = self.transcript.pending_restore_draft.as_ref() {
            cleanup_temporary_composer_items(draft.items.as_slice());
        }
        self.cleanup_temporary_draft_store_items();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Focus {
    Sessions,
    Transcript,
    Composer,
}

impl Focus {
    pub(super) fn label(self) -> &'static str {
        match self {
            Focus::Sessions => "sessions",
            Focus::Transcript => "transcript",
            Focus::Composer => "composer",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum MessageLoadMode {
    Replace,
    Prepend,
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
    MessagesLoaded {
        session_id: i64,
        mode: MessageLoadMode,
        result: UiResult<PaginatedResponse<MessageResource>>,
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
    AgentsLoaded {
        result: UiResult<Vec<AgentDescriptor>>,
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
            crate::backend::ProviderDraftAuthActionResult,
            crate::backend::ProviderDraftAuthError,
        >,
    },
    ProviderStudioSaved {
        provider_id: String,
        result: std::result::Result<
            crate::backend::ProviderStudioSaveResult,
            crate::backend::ProviderStudioSaveError,
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
        result: UiResult<Vec<DomainEvent>>,
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
    CopyText {
        text: String,
        success: String,
    },
    EditComposerExternally,
    AttachClipboardImage,
    AttachTerminalFiles {
        source: TerminalUploadRequest,
        images_only: bool,
    },
    DownloadTerminalFile {
        path: PathBuf,
    },
    ExportTranscript {
        path: Option<PathBuf>,
    },
    OpenPath {
        path: PathBuf,
    },
    PageTranscript,
}

#[derive(Debug, Clone)]
pub(super) enum TerminalUploadRequest {
    Iterm2,
    Kitty { local_sources: Vec<String> },
}

pub(super) type UiResult<T> = std::result::Result<T, String>;

#[derive(Debug, Clone)]
pub(super) enum Overlay {
    TranscriptSearch(LineInputOverlay),
    SessionRename(LineInputOverlay),
    AgentCreate(LineInputOverlay),
    SettingsValueEdit(SettingsValueEditOverlay),
    RuntimeSettingEdit(RuntimeSettingEditOverlay),
    Choice(ChoiceOverlay),
    FileAttach(FileAttachOverlay),
    PathBrowser(PathBrowserOverlay),
    Permission(PermissionOverlay),
    UserInputReply(UserInputOverlay),
    Confirm(ConfirmOverlay),
    SessionSearch(SessionSearchOverlay),
    Picker(PickerOverlay),
    Timeline(TimelineOverlay),
    ProviderStudio(Box<ProviderStudioOverlay>),
    ModelCatalogStudio(ModelCatalogStudioOverlay),
}

#[derive(Debug, Clone)]
pub(super) enum Route {
    Main,
    Usage(UsageDashboardState),
    SettingsStudio(SettingsStudioOverlay),
    AgentStudio(AgentStudioOverlay),
    PermissionStudio(PermissionStudioOverlay),
    PermissionRuleStudio(PermissionRuleStudioOverlay),
    SessionSearch(SessionSearchOverlay),
    Picker(PickerOverlay),
    SessionModelChooser(SessionModelChooserOverlay),
    Timeline(TimelineOverlay),
    PluginPolicyStudio(Box<PluginPolicyStudioOverlay>),
    PluginWorkbench(Box<PluginWorkbenchOverlay>),
    ProviderStudio(Box<ProviderStudioOverlay>),
    ModelCatalogStudio(ModelCatalogStudioOverlay),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum UsageDashboardView {
    #[default]
    Overview,
    Daily,
    Providers,
    Models,
    Sessions,
}

impl UsageDashboardView {
    pub(super) const ALL: [Self; 5] = [
        Self::Overview,
        Self::Daily,
        Self::Providers,
        Self::Models,
        Self::Sessions,
    ];

    pub(super) fn cycle(self, delta: isize) -> Self {
        cycle_copy(Self::ALL.as_slice(), self, delta)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum UsageDashboardSort {
    #[default]
    Cost,
    Tokens,
    Runs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UsageDashboardControl {
    Period,
    View,
    Provider,
    Model,
    Subagents,
    Sort,
    Refresh,
}

impl UsageDashboardControl {
    pub(super) const ALL: [Self; 7] = [
        Self::Period,
        Self::View,
        Self::Provider,
        Self::Model,
        Self::Subagents,
        Self::Sort,
        Self::Refresh,
    ];
}

impl UsageDashboardSort {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Cost => Self::Tokens,
            Self::Tokens => Self::Runs,
            Self::Runs => Self::Cost,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct UsageDashboardState {
    pub(super) period: UsagePeriod,
    pub(super) view: UsageDashboardView,
    pub(super) sort: UsageDashboardSort,
    pub(super) include_subagents: bool,
    pub(super) provider_filter: Option<String>,
    pub(super) model_filter: Option<String>,
    pub(super) available_providers: Vec<String>,
    pub(super) available_models: Vec<(String, String)>,
    pub(super) stats: Option<UsageStats>,
    pub(super) loading: bool,
    pub(super) request_id: u64,
    pub(super) selected: usize,
    pub(super) scroll: usize,
    pub(super) error: Option<String>,
    pub(super) controls_focused: bool,
    pub(super) selected_control: usize,
}

impl UsageDashboardState {
    pub(super) fn new(period: UsagePeriod) -> Self {
        Self {
            period,
            view: UsageDashboardView::Overview,
            sort: UsageDashboardSort::Cost,
            include_subagents: true,
            provider_filter: None,
            model_filter: None,
            available_providers: Vec::new(),
            available_models: Vec::new(),
            stats: None,
            loading: false,
            request_id: 0,
            selected: 0,
            scroll: 0,
            error: None,
            controls_focused: false,
            selected_control: 0,
        }
    }
}

fn cycle_copy<T>(values: &[T], current: T, delta: isize) -> T
where
    T: Copy + PartialEq,
{
    let index = values
        .iter()
        .position(|value| *value == current)
        .unwrap_or(0);
    let next = (index as isize + delta).rem_euclid(values.len() as isize) as usize;
    values[next]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DialogHost {
    Route,
    Overlay,
}

pub(super) type LineInputOverlay = InputDialogState<()>;

pub(super) type HelpOverlay = agena_tui_components::HelpDialogState<InfoOverlayKind>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InfoOverlayKind {
    Help,
    Diagnostics,
}

pub(super) use agena_tui_components::{
    HelpDialogEntry as HelpEntry, HelpDialogSection as HelpSection,
};
