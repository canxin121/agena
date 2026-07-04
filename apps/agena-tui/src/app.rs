use std::{
    cmp::{max, min},
    collections::{BTreeMap, BTreeSet, HashSet},
    env, fs,
    ops::Range,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use agena::{
    agent::{
        NetworkPermissionConfig, PathAccessModes, PathAccessRuleConfig, PathPermissionConfig,
        PermissionConfig, ToolPermissionConfig, ToolPermissionRules,
    },
    agents::{AgentDescriptor, AgentFrontmatter, AgentProfile, AgentScope},
    config::get_json_path,
    event::{DomainEvent, EventKind as AgenaSessionEvent},
    message::{
        AttachmentKind, MessagePart, MessageStatus, OperationPart, PartContent, ToolInvocation,
        UserInputQuestion, UserInputReply, UserInputReplyKind, UserInputRequest,
    },
    model::ModelRef,
    permission::{
        DecisionTraceStep, PermissionAction, PermissionMode, PermissionReplyKind,
        PermissionRequest, PermissionRiskLevel, PermissionScope, PolicySourceKind,
    },
    provider::{ProviderModel, auth::CredentialIssuer},
};
use agena_api::{
    commands::UpsertPermissionRuleParams,
    pagination::PaginatedResponse,
    resource::{
        MessageResource, MessageRole, PendingInteractiveRequest, PermissionRuleResource,
        ProviderAdapterModelsResource, ProviderAdapterModelsResponse, ProviderSummaryResource,
        RunOptions, SessionExecutionContextResource, SessionExecutionResource, SessionResource,
        SessionRunState, SessionUsageResource,
    },
};
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
#[cfg(test)]
use indexmap::IndexMap;
use ratatui::{
    Frame, Terminal,
    backend::Backend as RatatuiBackend,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::interval,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthChar;

use crate::backend::{
    Backend, ConfigJsonSources, InspectorRow, LiveEvent, ProviderConfigDraft,
    ProviderDraftAdapterRule, ProviderDraftAuthKind, ProviderDraftInteractiveLoginKind,
    ProviderDraftSecretSourceKind, ProviderNativeToolsPreset, SessionPermissionStudioState,
    SessionRefresh, provider_native_tools_config_for_preset,
    provider_native_tools_preset_from_config,
};
use crate::clipboard::{
    normalize_pasted_path, paste_image_to_temp_png, pasted_image_format, set_clipboard_text,
};
use crate::commands::{self, CommandId, CommandSpec};
use crate::composer_queue::{ComposerQueue, QueuePriority, QueuedMessage};
use crate::external_editor::{edit_text, open_path};
use crate::external_pager::page_text;
use crate::i18n::{I18n, SUPPORTED_LOCALES};
use crate::keybindings::{ComposerAction, ComposerKeyBindings};
use crate::terminal;
use crate::tui_config::{TuiConfig, TuiStatusLineConfig};
use crate::ui_text;
use agena_api_server::local_api::{
    CatalogModelResource, ModelCatalogListResponse, ModelCatalogResponse,
};
use agena_tui_components::{
    ConfirmDialogState, DashboardSelectionState, DetailTextLine, DetailTextSpec, Editor,
    EditorDialogKeyResult, EditorDialogState, InputDialogKeyResult, InputDialogState,
    ListWorkbenchState, QuerySuggestionState, QuestionFlowScreen, QuestionFlowState, ScrollState,
    SearchInputKeyResult, SearchListClearAction, SearchListCustomValue, SearchListItem,
    SearchListNoCustom, SearchListOverlay, SearchListOverlayConfig, SearchListRow,
    SearchPanelsOverlay, SectionedListFocus, SectionedListSection, SectionedListState,
    SelectableListState, SelectionCursor, SuggestionPopupState, build_detail_document,
    build_detail_text, drive_editor_dialog_key, drive_input_dialog_key, format_key_value_segment,
    join_inline_segments, move_selected_index, refresh_search_list_overlay,
    refresh_search_panels_overlay,
};

mod plugin_workbench;
mod provider_studio;
mod transcript_view;
mod view;

use self::plugin_workbench::*;
use self::provider_studio::*;

#[cfg(test)]
use self::transcript_view::render_message;
use self::transcript_view::{
    render_message_detailed, render_message_export, render_transcript_export_markdown,
    rewind_message_preview, sanitize_terminal_text,
};

const MESSAGE_PAGE_SIZE: u64 = 40;
const TIMELINE_EVENT_LIMIT: u64 = 200;
const UI_TICK_MS: u64 = 32;
const REFRESH_INTERVAL_MS: u64 = 250;
const DRAFT_PERSIST_INTERVAL_MS: u64 = 250;
const LARGE_PASTE_CHAR_THRESHOLD: usize = 1000;
const TOOL_CARD_PREVIEW_LINES: usize = 8;
const TOOL_CARD_PREVIEW_CHARS: usize = 2_500;
const TOOL_EXPANDED_PREVIEW_LINES: usize = 40;
const TOOL_EXPANDED_PREVIEW_CHARS: usize = 12_000;
const MAX_SLASH_COMMAND_SUGGESTIONS: usize = 6;
const MAX_FILE_MENTION_SUGGESTIONS: usize = 8;
const MAX_PROMPT_HISTORY_SEARCH_RESULTS: usize = 6;
const MAX_PROMPT_HISTORY_ENTRIES: usize = 200;
const AWS_REGION_CHOICES: &[&str] = &[
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

const PLUGIN_TOOL_PRESENTATION_PATH: &str = "plugins.policy.tool_presentation";
#[allow(dead_code)]
const PLUGIN_TOOL_PRESENTATION_DEFAULT_MODE_PATH: &str =
    "plugins.policy.tool_presentation.default_mode";
const PLUGIN_UI_PRESENTATION_PATH: &str = "plugins.policy.ui_presentation";
const PLUGIN_UI_PRESENTATION_DEFAULT_MODE_PATH: &str =
    "plugins.policy.ui_presentation.default_mode";
const PROVIDER_DEFAULT_WIZARD_INHERIT: &str = "__agena_default__";

const SETTINGS_FIELDS: [SettingsFieldSpec; 23] = [
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

const RUNTIME_SETTINGS: [RuntimeSettingSpec; 7] = [
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SessionViewMode {
    #[default]
    All,
    Roots,
    Subtree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionLoadScope {
    mode: SessionViewMode,
    anchor_session_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DraftSlot {
    Session(i64),
    NewSession,
}

#[derive(Debug, Clone, Default)]
struct DraftStore {
    drafts: BTreeMap<DraftSlot, ComposerDraft>,
}

#[derive(Debug, Clone, Default)]
struct PromptHistory {
    items: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
enum PromptHistoryDirection {
    Older,
    Newer,
}

#[derive(Debug, Clone)]
struct StatusLineState {
    command: String,
    refresh_interval: Duration,
    next_refresh_at: Instant,
    text: Option<String>,
    running: bool,
}

impl StatusLineState {
    fn new(config: &TuiStatusLineConfig) -> Option<Self> {
        let command = config.command.as_ref()?.clone();
        Some(Self {
            command,
            refresh_interval: Duration::from_millis(config.refresh_interval_ms),
            next_refresh_at: Instant::now(),
            text: None,
            running: false,
        })
    }
}

pub struct App {
    backend: Backend,
    i18n: I18n,
    tx: UnboundedSender<AppMessage>,
    rx: UnboundedReceiver<AppMessage>,
    launch: LaunchOptions,
    should_quit: bool,
    focus: Focus,
    current_route: Route,
    route_stack: Vec<Route>,
    overlay: Option<Overlay>,
    overlay_stack: Vec<Overlay>,
    seen_permission_request_ids: BTreeSet<String>,
    seen_user_input_request_ids: BTreeSet<String>,
    pending_permission_replay: Option<PermissionReplayState>,
    flash: Option<FlashMessage>,
    sessions: SessionListState,
    transcript: TranscriptState,
    run_options: RunOptionsState,
    composer: Editor,
    composer_items: Vec<ComposerItem>,
    slash_command_suggestions: Option<SlashCommandSuggestionState>,
    dismissed_slash_command_suggestions_for: Option<String>,
    file_mention_suggestions: Option<FileMentionSuggestionState>,
    dismissed_file_mention_suggestions_for: Option<String>,
    prompt_history_search: Option<PromptHistorySearchState>,
    selected_composer_item: Option<usize>,
    draft_store: DraftStore,
    draft_store_path: PathBuf,
    draft_store_dirty: bool,
    draft_store_last_persist_at: Instant,
    draft_store_reported_error: Option<String>,
    pending_draft_store_error: Option<String>,
    prompt_history: PromptHistory,
    prompt_history_path: PathBuf,
    prompt_history_recall_original: Option<ComposerDraft>,
    prompt_history_recall_index: Option<usize>,
    prompt_history_reported_error: Option<String>,
    pending_prompt_history_error: Option<String>,
    submitting_session_ids: HashSet<i64>,
    layout: LayoutCache,
    bootstrap_done: bool,
    last_refresh_at: Instant,
    pending_ui_action: Option<UiAction>,
    current_lineage: Option<CurrentLineageState>,
    /// Forwarder task that pumps `Backend::subscribe_session_events` into
    /// [`AppMessage::SessionEventArrived`]. Aborted whenever the active
    /// session changes so we don't accumulate stale subscriptions.
    active_subscription: Option<tokio::task::JoinHandle<()>>,
    /// Pending messages typed by the user while the AI was working. Drained
    /// FIFO once the active run finishes. See `composer_queue.rs`.
    queue: ComposerQueue,
    status_line: Option<StatusLineState>,
    plugin_theme: Option<agena::plugin::HostThemePalette>,
    keybindings: ComposerKeyBindings,
    transcript_motion_prefix: Option<String>,
    /// Last time the user pressed Ctrl+C; a second press within the window
    /// exits the application.
    last_ctrl_c_at: Option<Instant>,
    double_esc_window: Duration,
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
enum Focus {
    Sessions,
    Transcript,
    Composer,
}

impl Focus {
    fn label(self) -> &'static str {
        match self {
            Focus::Sessions => "sessions",
            Focus::Transcript => "transcript",
            Focus::Composer => "composer",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum MessageLoadMode {
    Replace,
    Prepend,
}

#[derive(Debug, Clone)]
enum AppMessage {
    SessionsLoaded {
        scope: SessionLoadScope,
        subtree_root_id: Option<i64>,
        result: UiResult<Vec<SessionResource>>,
    },
    SessionCreated {
        submit_draft: Option<ComposerDraft>,
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
enum UiAction {
    EditComposerExternally,
    AttachClipboardImage,
    ExportTranscript { path: Option<PathBuf> },
    OpenPath { path: PathBuf },
    PageTranscript,
}

type UiResult<T> = std::result::Result<T, String>;

#[derive(Debug, Clone)]
enum Overlay {
    TranscriptSearch(LineInputOverlay),
    SessionRename(LineInputOverlay),
    AgentCreate(LineInputOverlay),
    SettingsValueEdit(SettingsValueEditOverlay),
    RuntimeSettingEdit(RuntimeSettingEditOverlay),
    Choice(ChoiceOverlay),
    PermissionRuleEdit(PermissionRuleEditOverlay),
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
enum Route {
    Main,
    Help(HelpOverlay),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogHost {
    Route,
    Overlay,
}

type LineInputOverlay = InputDialogState<()>;

type HelpOverlay = ScrollState;

#[derive(Debug, Clone)]
struct SettingsStudioOverlay {
    title: String,
    footer: String,
    state: SectionedListState<SettingsStudioSection>,
}

#[derive(Debug, Clone)]
struct AgentStudioOverlay {
    agent_name: String,
    profile: AgentProfile,
    storage: AgentProfileStorage,
    editable: bool,
    default_agent_name: Option<String>,
    workbench: ListWorkbenchState<AgentStudioItem, AgentStudioEditor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentProfileStorage {
    BuiltIn,
    Config,
    Markdown,
    Runtime,
}

impl AgentProfileStorage {
    fn editable(self) -> bool {
        matches!(self, Self::Config | Self::Markdown)
    }
}

#[derive(Debug, Clone)]
struct AgentStudioItem {
    label: String,
    value: String,
    detail: String,
    action: AgentStudioAction,
}

#[derive(Debug, Clone)]
enum AgentStudioAction {
    Edit(AgentStudioField),
    OpenPermissionWorkbench,
    OpenSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentStudioField {
    Description,
    Prompt,
    DefaultProvider,
    DefaultAdapter,
    DefaultModel,
}

type AgentStudioEditor = EditorDialogState<AgentStudioEditorAction>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentStudioEditorAction {
    Field(AgentStudioField),
}

#[derive(Debug, Clone)]
enum PermissionStudioSource {
    GlobalConfig,
    Agent { agent_name: String },
    Session { session_id: i64 },
    EffectiveSession { session_id: i64 },
}

#[derive(Debug, Clone)]
struct PermissionStudioOverlay {
    title: String,
    footer: String,
    source: PermissionStudioSource,
    title_context: String,
    source_label: String,
    scope_label: String,
    editable: bool,
    permission: PermissionConfig,
    nav: SelectableListState<PermissionStudioNavItem>,
    pane_focus: PermissionStudioPaneFocus,
    page: PermissionStudioPage,
    state: SectionedListState<PermissionStudioSection>,
    editor: Option<PermissionStudioEditor>,
}

#[derive(Debug, Clone)]
struct PermissionStudioNavItem {
    label: String,
    level: usize,
    page: PermissionStudioPage,
    section: Option<PermissionStudioSectionId>,
    selectable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionStudioPaneFocus {
    Navigation,
    Content,
}

#[derive(Debug, Clone)]
struct PermissionStudioItem {
    label: String,
    value: String,
    action: PermissionStudioAction,
}

#[derive(Debug, Clone)]
enum PermissionStudioAction {
    Noop,
    EditMode(PermissionStudioModeTarget),
    EditText(PermissionStudioTextTarget),
}

#[derive(Debug, Clone)]
struct PermissionStudioSection {
    id: PermissionStudioSectionId,
    label: String,
    items: Vec<PermissionStudioItem>,
}

type PermissionStudioFocus = SectionedListFocus;

impl SectionedListSection for PermissionStudioSection {
    type Item = PermissionStudioItem;

    fn items(&self) -> &[Self::Item] {
        self.items.as_slice()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionStudioSectionId {
    RootPath,
    RootNetwork,
    RootTools,
    PathDefaults,
    PathRules,
    NetworkZones,
    NetworkRules,
    ToolTags,
    ToolNames,
    ToolCommandRules,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PermissionStudioPage {
    Overview,
    PathDefaults,
    PathRules,
    NetworkZones,
    NetworkRules,
    ToolTags,
    ToolNames,
    ToolCommandRules,
}

type PermissionStudioEditor = EditorDialogState<PermissionStudioEditorAction>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum PermissionStudioEditorAction {
    Text(PermissionStudioTextTarget),
    AddPathRule { duplicate_from: Option<String> },
    AddNetworkRule { duplicate_from: Option<String> },
    AddToolTag { duplicate_from: Option<String> },
    AddToolName { duplicate_from: Option<String> },
    AddToolRule { duplicate_from: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PermissionStudioModeTarget {
    PathWorkspaceRead,
    PathWorkspaceWrite,
    PathExternalRead,
    PathExternalWrite,
    NetworkInternet,
    NetworkPrivate,
    NetworkLoopback,
    ToolDefault,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PermissionStudioTextTarget {
    PathRulePattern { pattern: String },
    NetworkRuleTarget { target: String },
    ToolTagKey { key: String },
    ToolNameKey { key: String },
    ToolRuleName { tool_name: String },
}

#[derive(Debug, Clone)]
struct SettingsStudioSection {
    id: SettingsStudioSectionId,
    label: String,
    summary: String,
    description: String,
    items: Vec<SettingsStudioItem>,
}

#[derive(Debug, Clone)]
struct SettingsStudioItem {
    label: String,
    value: String,
    detail: String,
    path: Option<String>,
    current_value: Option<String>,
    effective_value: Option<String>,
    source_rows: Vec<SettingsSourceRow>,
    action: SettingsPickerAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SettingsSourceRow {
    label: String,
    value: String,
}

impl SettingsSourceRow {
    fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

impl SettingsStudioItem {
    fn new(
        label: impl Into<String>,
        value: impl Into<String>,
        detail: impl Into<String>,
        action: SettingsPickerAction,
    ) -> Self {
        let value = value.into();
        let current_value = (!value.trim().is_empty()).then(|| value.clone());
        Self {
            label: label.into(),
            value,
            detail: detail.into(),
            path: None,
            current_value,
            effective_value: None,
            source_rows: Vec::new(),
            action,
        }
    }

    fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    fn with_current_value(mut self, value: impl Into<String>) -> Self {
        self.current_value = Some(value.into());
        self
    }

    fn with_effective_value(mut self, value: impl Into<String>) -> Self {
        self.effective_value = Some(value.into());
        self
    }

    fn with_source_rows(mut self, rows: Vec<SettingsSourceRow>) -> Self {
        self.source_rows = rows;
        self
    }

    fn without_value_details(mut self) -> Self {
        self.current_value = None;
        self.effective_value = None;
        self.path = None;
        self.source_rows.clear();
        self
    }
}

type SettingsStudioFocus = SectionedListFocus;

impl SectionedListSection for SettingsStudioSection {
    type Item = SettingsStudioItem;

    fn items(&self) -> &[Self::Item] {
        self.items.as_slice()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsStudioSectionId {
    ConfigProviders,
    ConfigAgents,
    ConfigPermission,
    ConfigPlugins,
    ConfigRuntime,
    ConfigSession,
    ConfigHarnesses,
    ConfigTracing,
    ConfigUi,
    RuntimeOverrides,
    RuntimeRules,
    Catalogs,
    Files,
}

type SettingsValueEditOverlay = InputDialogState<SettingsFieldSpec>;

type RuntimeSettingEditOverlay = InputDialogState<RuntimeSettingSpec>;

#[derive(Debug, Clone)]
struct ChoiceOverlayMeta {
    i18n: I18n,
    all_items: Vec<ChoiceItem>,
    action: ChoiceOverlayAction,
}

#[derive(Debug, Clone)]
struct ChoiceItem {
    label: String,
    detail: String,
    value: String,
    search_text: String,
}

#[derive(Debug, Clone)]
struct ChoiceCustomValue {
    raw: String,
}

type ChoiceOverlay = SearchListOverlay<ChoiceItem, ChoiceCustomValue, ChoiceOverlayMeta, Editor>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChoiceOverlayStyle {
    Searchable,
    SearchableSelect,
    SelectOnly,
}

#[derive(Debug, Clone)]
enum ChoiceOverlayAction {
    SettingsField(SettingsFieldSpec),
    RuntimeSetting(RuntimeSettingSpec),
    SessionModelVariant(SessionModelVariantStep),
    ProviderDefaultWizard(ProviderDefaultWizardStep, ProviderDefaultWizardDraft),
    ProviderStudioField(ProviderStudioField),
    ProviderStudioModelField(ProviderModelConfigField),
    PermissionRuleStudio(PermissionRuleStudioChoiceField),
    PermissionStudioMode(PermissionStudioModeTarget),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderDefaultWizardStep {
    Provider,
    Adapter,
    Model,
    ThinkingMode,
    SpeedMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionModelVariantStep {
    ThinkingMode,
    SpeedMode,
    Verbosity,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProviderDefaultWizardDraft {
    provider_id: String,
    adapter_id: Option<String>,
    model_id: Option<String>,
    thinking_mode: Option<String>,
    speed_mode: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionRuleStudioChoiceField {
    SubjectKind,
    PathAccessKind,
    Scope,
    Mode,
}

#[derive(Debug, Clone, Copy)]
struct SettingsFieldSpec {
    section: SettingsStudioSectionId,
    path: &'static str,
    label_key: &'static str,
    description_key: &'static str,
    kind: SettingsFieldKind,
}

#[derive(Debug, Clone, Copy)]
enum SettingsFieldKind {
    String,
    Bool,
    Integer,
    Float,
}

#[derive(Debug, Clone)]
enum SettingsPickerAction {
    EditField(SettingsFieldSpec),
    EditRuntimeSetting(RuntimeSettingSpec),
    OpenPluginPolicyStudio,
    OpenProviderDefaultWizard,
    OpenAgentList,
    OpenAgentPermissionWorkbench(String),
    OpenProviderList,
    OpenModelCatalogWorkbench,
    OpenRuntimeProviderOverride,
    OpenRuntimeModelOverride,
    ClearRuntimeModelStack,
    OpenGlobalPermissionWorkbench,
    OpenCurrentSessionPermissionWorkbench,
    OpenSessionEffectivePermissionView(i64),
    OpenPermissionRules,
    OpenPluginWorkbench,
    OpenConfigFile,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeSettingSpec {
    id: RuntimeSettingId,
    kind: SettingsFieldKind,
}

#[derive(Debug, Clone, Copy)]
enum RuntimeSettingId {
    ThinkingMode,
    SpeedMode,
    Verbosity,
    ParallelToolCalls,
    Temperature,
    MaxOutput,
    System,
}

#[derive(Debug, Clone)]
struct PermissionRuleEditOverlay {
    rule_id: Option<i64>,
    state: InputDialogState<()>,
    return_query: String,
    return_overlay: Option<Box<Overlay>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PermissionRuleDraft {
    subject_kind: PermissionRuleSubjectKind,
    tool_name: String,
    qualifier: String,
    path_access_kind: String,
    workspace_root: String,
    target_path: String,
    network_target: String,
    network_host: String,
    network_port: String,
    scope: String,
    session_id: String,
    mode: PermissionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionRuleSubjectKind {
    Tool,
    PathAccess,
    NetworkAccess,
}

#[derive(Debug, Clone)]
struct PermissionRuleStudioOverlay {
    rule_id: Option<i64>,
    draft: PermissionRuleDraft,
    workbench: ListWorkbenchState<PermissionRuleStudioItem, PermissionRuleStudioEditor>,
}

#[derive(Debug, Clone)]
struct PermissionRuleStudioItem {
    label: String,
    value: String,
    detail: String,
    action: PermissionRuleStudioAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionRuleStudioAction {
    SubjectKind,
    ToolName,
    Qualifier,
    PathAccessKind,
    WorkspaceRoot,
    BrowseWorkspaceRoot,
    TargetPath,
    BrowseTargetPath,
    NetworkTarget,
    Scope,
    SessionId,
    Mode,
    Save,
    Revoke,
}

type PermissionRuleStudioEditor = EditorDialogState<PermissionRuleStudioEditField>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionRuleStudioEditField {
    ToolName,
    Qualifier,
    WorkspaceRoot,
    TargetPath,
    NetworkTarget,
    SessionId,
}

#[derive(Debug, Clone)]
struct UserInputOverlay {
    session_id: i64,
    request: UserInputRequest,
    answers: BTreeMap<String, UserInputAnswerDraft>,
    state: QuestionFlowState,
    editing_custom: bool,
    custom_input: Editor,
    review_option: usize,
    review_scroll: u16,
}

#[derive(Debug, Clone)]
struct PermissionOverlay {
    session_id: i64,
    request: PermissionRequest,
    selection: SelectionCursor,
}

#[derive(Debug, Clone)]
enum PendingInteractiveOverlayTarget {
    Permission {
        session_id: i64,
        request: Box<PermissionRequest>,
    },
    UserInput {
        session_id: i64,
        request: UserInputRequest,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingInteractiveKind {
    Permission,
    UserInput,
}

#[derive(Debug, Clone, Copy)]
struct PermissionOverlayChoice {
    kind: PermissionReplyKind,
    scope: Option<PermissionScope>,
}

#[derive(Debug, Clone)]
struct PermissionReplayState {
    session_id: i64,
    fingerprint: String,
    last_request_id: String,
    kind: PermissionReplyKind,
    scope: Option<PermissionScope>,
    label: String,
}

type ConfirmOverlay = ConfirmDialogState<ConfirmAction>;

#[derive(Debug, Clone)]
enum ConfirmAction {
    Rewind {
        session_id: i64,
        message_id: i64,
        target: String,
    },
    RevokePermissionRule {
        rule_id: i64,
        label: String,
        return_query: String,
    },
    PermissionStudioDeletePathRule {
        pattern: String,
    },
    PermissionStudioDeleteNetworkRule {
        target: String,
    },
    PermissionStudioDeleteToolTag {
        key: String,
    },
    PermissionStudioDeleteToolName {
        key: String,
    },
    PermissionStudioDeleteToolRule {
        tool_name: String,
    },
    ExitSnapshot {
        session_id: i64,
        discard_changes: bool,
    },
    ProviderStudioDeleteProvider {
        provider_id: String,
    },
    ProviderStudioDeleteAdapter {
        adapter_id: String,
    },
    ProviderStudioDeleteModel {
        adapter_id: String,
        model_id: String,
    },
}

#[derive(Debug, Clone)]
struct FileAttachOverlayMeta {
    i18n: I18n,
}

#[derive(Debug, Clone)]
struct TypedPathValue {
    raw: String,
}

#[derive(Debug, Clone)]
struct PathBrowserOverlayMeta {
    i18n: I18n,
    mode: PathBrowserMode,
    target: PathBrowserTarget,
}

type FileAttachOverlay = SearchListOverlay<PathBuf, TypedPathValue, FileAttachOverlayMeta, Editor>;
type PathBrowserOverlay =
    SearchListOverlay<PathBrowserItem, TypedPathValue, PathBrowserOverlayMeta, Editor>;

#[derive(Debug, Clone)]
struct PathBrowserItem {
    path: PathBuf,
    label: String,
    detail: String,
    is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathBrowserMode {
    AnyPath,
    DirectoryOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathBrowserTarget {
    PermissionRuleStudio(PermissionRuleStudioPathField),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionRuleStudioPathField {
    WorkspaceRoot,
    TargetPath,
}

#[derive(Debug, Clone)]
struct TimelineOverlayMeta {
    session_id: i64,
}

type TimelineOverlay = SearchPanelsOverlay<TimelineItem, TimelineOverlayMeta, Editor>;

#[derive(Debug, Clone)]
struct TimelineItem {
    summary: String,
    detail_body: Text<'static>,
    search_text: String,
    copy_text: String,
    linked_message_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct ProviderStudioOverlay {
    title: String,
    footer: String,
    show_provider_list: bool,
    providers: SelectableListState<ProviderStudioProviderRow>,
    selection: DashboardSelectionState<ProviderStudioFocus>,
    draft: ProviderConfigDraft,
    adapter_models: Vec<ProviderAdapterModelsResource>,
    configured_adapter_ids: BTreeSet<String>,
    adapter_candidate_ids: Vec<String>,
    adapter_selection_touched: bool,
    selected_adapter_ids: BTreeSet<String>,
    selected_model_keys: BTreeSet<String>,
    catalog_matches: BTreeMap<String, CatalogModelResource>,
    listing_adapter_models: bool,
    saving: bool,
    pending_adapter_models_key: Option<String>,
    pending_auth_key: Option<String>,
    next_auth_poll_at: Option<Instant>,
    detail_page: Option<ProviderStudioDetailPage>,
    model_page: Option<ProviderStudioModelPage>,
    editor: Option<ProviderStudioEditor>,
}

#[derive(Debug, Clone)]
struct ProviderStudioProviderRow {
    provider_id: Option<String>,
    label: String,
    detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderStudioFocus {
    Fields,
    Adapters,
    Models,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderStudioField {
    ProviderId,
    AuthMode,
    AuthSubtype,
    AuthLoginMethod,
    StartAuthAction,
    ContinueAuthAction,
    EditAuthDetailsAction,
    DeleteProviderAction,
    BaseUrl,
    InstanceUrl,
    ApiKeySource,
    ApiKeyValue,
    RedirectUri,
    CallbackUrl,
    RefreshToken,
    AccessToken,
    ExpiresAtMs,
    AccountId,
    EnterpriseDomain,
    Region,
    Profile,
    AccessKeyId,
    SecretAccessKey,
    SessionToken,
    ServiceKeyEnv,
    DefaultAdapter,
    DefaultModel,
}

#[derive(Debug, Clone)]
struct ProviderStudioDetailPage {
    title: String,
    footer: String,
    selection: SelectionCursor,
}

#[derive(Debug, Clone)]
struct ProviderStudioModelPage {
    title: String,
    footer: String,
    adapter_id: String,
    original_model_id: String,
    draft: ProviderModelConfigDraft,
    selection: SelectionCursor,
}

#[derive(Debug, Clone)]
struct ProviderModelConfigDraft {
    model_id: String,
    enabled: bool,
    display_name: String,
    lifecycle: String,
    context_window_tokens: String,
    max_input_tokens: String,
    max_output_tokens: String,
    input_modalities: BTreeSet<String>,
    features: BTreeSet<String>,
    output_modalities: String,
    description: String,
    native_tools_preset: ProviderNativeToolsPreset,
    native_tools_custom: agena::config::ProviderNativeToolsConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderModelConfigField {
    ModelId,
    Enabled,
    DisplayName,
    Lifecycle,
    ContextWindowTokens,
    MaxInputTokens,
    MaxOutputTokens,
    InputModalities,
    Features,
    OutputModalities,
    Description,
    NativeTools,
}

type ProviderStudioEditor = EditorDialogState<ProviderStudioEditorAction>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderStudioEditorAction {
    Field(ProviderStudioField),
    NewModel { adapter_id: String },
    ModelField(ProviderModelConfigField),
}

#[derive(Debug, Clone)]
struct ModelCatalogStudioOverlay {
    query: String,
    summary: ModelCatalogResponse,
    total: usize,
    offset: usize,
    limit: usize,
    loading: bool,
    workbench: ListWorkbenchState<CatalogModelResource, LineInputOverlay>,
}

#[derive(Debug, Clone)]
struct SessionSearchItem {
    session: SessionResource,
    label: String,
    detail: String,
}

#[derive(Debug, Clone)]
struct SessionSearchOverlayMeta {
    all_items: Vec<SessionSearchItem>,
    mode: SessionViewMode,
    scope_session_id: Option<i64>,
    page_limit: usize,
    page_index: usize,
    offset: usize,
    cursors: Vec<Option<String>>,
    next_cursor: Option<String>,
    has_more: bool,
}

type SessionSearchOverlay =
    SearchListOverlay<SessionSearchItem, SearchListNoCustom, SessionSearchOverlayMeta, Editor>;

#[derive(Debug, Clone)]
struct PickerOverlayMeta {
    all_items: Vec<PickerItem>,
    kind: PickerKind,
}

type PickerOverlay = SearchListOverlay<PickerItem, SearchListNoCustom, PickerOverlayMeta, Editor>;

#[derive(Debug, Clone)]
struct SessionModelChooserOverlayMeta {
    all_items: Vec<SessionModelChoiceItem>,
    page_size: usize,
}

type SessionModelChooserOverlay = SearchListOverlay<
    SessionModelChoiceItem,
    SearchListNoCustom,
    SessionModelChooserOverlayMeta,
    Editor,
>;

#[derive(Debug, Clone)]
struct SessionModelChoiceItem {
    label: String,
    detail: String,
    search_text: String,
    model: ModelRef,
}

impl SearchListItem for ChoiceItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        (!self.detail.trim().is_empty()).then_some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.value.clone()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        query.trim().is_empty()
            || self
                .search_text
                .contains(query.trim().to_ascii_lowercase().as_str())
    }
}

impl SearchListCustomValue<ChoiceOverlayMeta> for ChoiceCustomValue {
    fn search_list_from_input(input: &str, _: &ChoiceOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_list_label(&self, meta: &ChoiceOverlayMeta) -> String {
        ui_text::t(&meta.i18n, "search-list-custom-value-label")
    }

    fn search_list_detail(&self, meta: &ChoiceOverlayMeta) -> Option<String> {
        Some(meta.i18n.text_args(
            "search-list-custom-value-detail",
            &crate::fl_args!(
                "value" => format_setting_value_inline(&JsonValue::String(self.raw.clone()))
            ),
        ))
    }

    fn search_list_input_text(&self) -> String {
        self.raw.clone()
    }
}

impl SearchListCustomValue<FileAttachOverlayMeta> for TypedPathValue {
    fn search_list_from_input(input: &str, _: &FileAttachOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_list_label(&self, meta: &FileAttachOverlayMeta) -> String {
        ui_text::t(&meta.i18n, "search-list-custom-path-label")
    }

    fn search_list_detail(&self, _: &FileAttachOverlayMeta) -> Option<String> {
        Some(self.raw.clone())
    }

    fn search_list_input_text(&self) -> String {
        self.raw.clone()
    }
}

impl SearchListCustomValue<PathBrowserOverlayMeta> for TypedPathValue {
    fn search_list_from_input(input: &str, _: &PathBrowserOverlayMeta) -> Option<Self> {
        let raw = input.trim().to_string();
        (!raw.is_empty()).then_some(Self { raw })
    }

    fn search_list_label(&self, meta: &PathBrowserOverlayMeta) -> String {
        ui_text::t(&meta.i18n, "search-list-custom-path-label")
    }

    fn search_list_detail(&self, _: &PathBrowserOverlayMeta) -> Option<String> {
        Some(self.raw.clone())
    }

    fn search_list_input_text(&self) -> String {
        self.raw.clone()
    }
}

impl SearchListItem for PathBrowserItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        Some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.path.display().to_string()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        query.trim().is_empty()
            || self
                .label
                .to_ascii_lowercase()
                .contains(query.trim().to_ascii_lowercase().as_str())
            || self
                .detail
                .to_ascii_lowercase()
                .contains(query.trim().to_ascii_lowercase().as_str())
    }

    fn search_list_label_style(&self) -> Style {
        if self.is_dir {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        }
    }
}

impl SearchListItem for SessionSearchItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        Some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.session.title.clone()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        session_matches_query(&self.session, query.trim())
    }
}

impl SearchListItem for PickerItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        (!self.detail.trim().is_empty()).then_some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.label.clone()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        let raw_query = query.trim();
        if raw_query.is_empty() {
            return true;
        }
        let query = raw_query.to_ascii_lowercase();
        let query = query.as_str();
        match &self.value {
            PickerValue::Command(spec) => {
                commands::command_matches_query(spec, raw_query)
                    || self.detail.to_ascii_lowercase().contains(query)
            }
            PickerValue::ProviderCreate | PickerValue::AgentCreate => true,
            PickerValue::RuntimeTool(_) => {
                self.label.to_ascii_lowercase().contains(query)
                    || self.detail.to_ascii_lowercase().contains(query)
            }
            PickerValue::Session(session_id) => {
                self.label.to_ascii_lowercase().contains(query)
                    || self.detail.to_ascii_lowercase().contains(query)
                    || format!("#{session_id}").contains(query)
            }
            PickerValue::Message(message_id) => {
                self.label.to_ascii_lowercase().contains(query)
                    || self.detail.to_ascii_lowercase().contains(query)
                    || format!("#{message_id}").contains(query)
            }
            _ => {
                self.label.to_ascii_lowercase().contains(query)
                    || self.detail.to_ascii_lowercase().contains(query)
            }
        }
    }
}

impl SearchListItem for SessionModelChoiceItem {
    fn search_list_label(&self) -> String {
        self.label.clone()
    }

    fn search_list_detail(&self) -> Option<String> {
        Some(self.detail.clone())
    }

    fn search_list_fill_value(&self) -> String {
        self.label.clone()
    }

    fn search_list_matches_query(&self, query: &str) -> bool {
        query.trim().is_empty()
            || self
                .search_text
                .contains(query.trim().to_ascii_lowercase().as_str())
    }
}

#[derive(Debug, Clone)]
struct PickerItem {
    label: String,
    detail: String,
    value: PickerValue,
}

#[derive(Debug, Clone)]
enum PickerValue {
    Command(&'static CommandSpec),
    RuntimeTool(String),
    ProviderCreate,
    Provider(ProviderSummaryResource),
    AgentCreate,
    Agent(Box<AgentDescriptor>),
    Session(i64),
    Message(i64),
    PermissionRuleCreate,
    PermissionRule(Box<PermissionRuleResource>),
    Inspector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderPickerPurpose {
    SetProvider,
    Configure,
}

#[derive(Debug, Clone)]
enum PickerKind {
    Commands,
    Lineage { session_id: i64 },
    RewindMessages { session_id: i64 },
    Providers(ProviderPickerPurpose),
    Agents,
    ChildSessions { parent_session_id: i64 },
    PermissionRules,
    Inspector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineageRelation {
    Ancestor,
    Current,
    Sibling,
    Child,
}

#[derive(Debug, Clone)]
struct LineageSessionItem {
    session: SessionResource,
    relation: LineageRelation,
    depth: usize,
    is_leaf: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SessionLineageSummary {
    root_id: i64,
    depth: usize,
    side_branch_count: usize,
    descendant_count: usize,
}

#[derive(Debug, Clone)]
struct SessionPathSegment {
    id: i64,
}

#[derive(Debug, Clone)]
struct CurrentLineageState {
    session_id: i64,
    summary: SessionLineageSummary,
    path: Vec<SessionPathSegment>,
}

#[derive(Debug, Clone)]
struct FlashMessage {
    text: String,
    level: FlashLevel,
    expires_at: Instant,
}

#[derive(Debug, Clone, Copy)]
enum FlashLevel {
    Success,
    Warning,
    Error,
    Info,
}

#[derive(Default)]
struct SessionListState {
    source_items: Vec<SessionResource>,
    list: SelectableListState<SessionResource>,
    search_query: String,
    view_mode: SessionViewMode,
    subtree_root_id: Option<i64>,
    pending_scope: Option<SessionLoadScope>,
    next_cursor: Option<String>,
    has_more: bool,
    loading: bool,
    loading_more: bool,
    initialized: bool,
}

struct TranscriptState {
    i18n: I18n,
    session_id: Option<i64>,
    session_title: String,
    messages: Vec<MessageResource>,
    older_cursor: Option<String>,
    has_more_older: bool,
    loading_initial: bool,
    loading_older: bool,
    refreshing: bool,
    state_loading: bool,
    submitting: bool,
    pending_restore_draft: Option<ComposerDraft>,
    follow_tail: bool,
    scroll: usize,
    cursor_line: usize,
    block_cursor: Option<TranscriptBlockCursor>,
    search_query: String,
    search_match_index: Option<usize>,
    execution: Option<SessionExecutionResource>,
    last_event_seq: Option<i64>,
    detail_expanded_by_default: TranscriptDetailDefaults,
    node_expansions: BTreeMap<TranscriptNodeKey, bool>,
    rendered: Option<RenderedTranscript>,
}

#[derive(Debug, Clone, Default)]
struct RunOptionsState {
    model: Option<ModelRef>,
    thinking_mode: Option<String>,
    speed_mode: Option<String>,
    verbosity: Option<String>,
    parallel_tool_calls: Option<bool>,
    system: Option<String>,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComposerDraft {
    pub text: String,
    pub items: Vec<ComposerItem>,
    pub elements: Vec<ComposerDraftElement>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerItem {
    Attachment(StagedAttachment),
    LargePaste(StagedPaste),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedAttachment {
    pub(crate) path: PathBuf,
    pub(crate) placeholder: String,
    pub(crate) label: String,
    pub(crate) is_temp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedPaste {
    pub(crate) placeholder: String,
    pub(crate) label: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerDraftElement {
    pub(crate) placeholder: String,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, Default)]
struct SlashCommandSuggestionMeta;

type SlashCommandSuggestionState =
    SuggestionPopupState<SlashCommandSuggestionItem, SlashCommandSuggestionMeta>;

#[derive(Debug, Clone)]
struct SlashCommandSuggestionItem {
    label: String,
    detail: String,
    value: SlashCommandSuggestionValue,
}

#[derive(Debug, Clone)]
enum SlashCommandSuggestionValue {
    Command(&'static CommandSpec),
    RuntimeTool(String),
}

#[derive(Debug, Clone)]
struct SlashCommandSuggestionContext {
    query: String,
    fingerprint: String,
    name_range: Range<usize>,
}

#[derive(Debug, Clone)]
struct FileMentionSuggestionMeta {
    mention_range: Range<usize>,
}

type FileMentionSuggestionState =
    SuggestionPopupState<FileMentionSuggestionItem, FileMentionSuggestionMeta>;

#[derive(Debug, Clone)]
struct FileMentionSuggestionItem {
    path: PathBuf,
    label: String,
    detail: String,
}

#[derive(Debug, Clone)]
struct FileMentionSuggestionContext {
    query: String,
    fingerprint: String,
    mention_range: Range<usize>,
}

#[derive(Debug, Clone)]
struct PromptHistorySearchMeta {
    original: ComposerDraft,
}

type PromptHistorySearchState =
    QuerySuggestionState<PromptHistorySearchResult, PromptHistorySearchMeta, Editor>;

#[derive(Debug, Clone)]
struct PromptHistorySearchResult {
    history_index: usize,
    text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct UserInputAnswerDraft {
    option_indexes: BTreeSet<usize>,
    custom_values: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
struct PersistentDraftStore {
    #[serde(default = "persistent_draft_store_version")]
    version: u32,
    #[serde(default)]
    sessions: BTreeMap<i64, PersistentComposerDraft>,
    #[serde(default)]
    new_session: Option<PersistentComposerDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistentComposerDraft {
    text: String,
    items: Vec<PersistentComposerItem>,
    elements: Vec<PersistentComposerDraftElement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum PersistentComposerItem {
    Attachment(PersistentAttachment),
    LargePaste(PersistentPaste),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistentAttachment {
    path: PathBuf,
    placeholder: String,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistentPaste {
    placeholder: String,
    label: String,
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PersistentComposerDraftElement {
    placeholder: String,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct PromptHistoryRecord {
    text: String,
}

#[derive(Debug, Clone)]
struct RenderedTranscript {
    width: u16,
    lines: Vec<RenderedLine>,
    search_matches: Vec<usize>,
    message_line_starts: Vec<(i64, usize)>,
    nodes: Vec<RenderedTranscriptNode>,
    line_nodes: Vec<Option<usize>>,
}

#[derive(Debug, Clone)]
struct RenderedLine {
    text: String,
    style: Style,
    rich_line: Option<Line<'static>>,
}

#[derive(Debug, Clone, Copy)]
struct TranscriptDetailDefaults {
    tool_output_expanded: bool,
    thinking_expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptMoveDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscriptBlockCursor {
    key: TranscriptNodeKey,
    direction: TranscriptMoveDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum TranscriptNodeKey {
    MessagePart {
        message_id: i64,
        part_id: Option<i64>,
    },
    Reasoning {
        message_id: i64,
        part_id: i64,
    },
    Tool {
        message_id: i64,
        part_id: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TranscriptNodeKind {
    Message,
    Reasoning,
    Tool,
}

fn transcript_node_kind_label(i18n: &I18n, kind: TranscriptNodeKind) -> String {
    let key = match kind {
        TranscriptNodeKind::Message => "transcript-node-kind-message",
        TranscriptNodeKind::Reasoning => "transcript-node-kind-reasoning",
        TranscriptNodeKind::Tool => "transcript-node-kind-tool",
    };
    ui_text::t(i18n, key)
}

#[derive(Debug, Clone)]
struct RenderedTranscriptNode {
    key: TranscriptNodeKey,
    kind: TranscriptNodeKind,
    start_line: usize,
    end_line: usize,
    copy_text: String,
    toggleable: bool,
    expanded: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct LayoutCache {
    transcript_body: Rect,
}

fn persistent_draft_store_version() -> u32 {
    1
}

impl App {
    fn current_route_is_main(&self) -> bool {
        matches!(self.current_route, Route::Main)
    }

    pub fn new(backend: Backend, launch: LaunchOptions, i18n: I18n) -> Self {
        let (tx, rx) = unbounded_channel();
        let draft_store_path = default_draft_store_path();
        let (draft_store, pending_draft_store_error) = match DraftStore::load(&draft_store_path) {
            Ok(store) => (store, None),
            Err(error) => (
                DraftStore::default(),
                Some(i18n.text_args(
                    "flash-composer-drafts-load-failed",
                    &crate::fl_args!("error" => error.to_string()),
                )),
            ),
        };
        let prompt_history_path = default_prompt_history_path();
        let (prompt_history, pending_prompt_history_error) =
            match PromptHistory::load(&prompt_history_path) {
                Ok(history) => (history, None),
                Err(error) => (
                    PromptHistory::default(),
                    Some(i18n.text_args(
                        "flash-prompt-history-load-failed",
                        &crate::fl_args!("error" => error.to_string()),
                    )),
                ),
            };
        let keybindings = launch.tui_config.keybindings.clone();
        let status_line = StatusLineState::new(&launch.tui_config.status_line);
        let double_esc_window = Duration::from_millis(launch.tui_config.double_esc_window_ms);
        let plugin_theme = launch.tui_config.theme.as_ref().and_then(|theme_id| {
            backend
                .plugin_theme_palettes()
                .into_iter()
                .find(|palette| palette.id == *theme_id)
        });
        let mut app = Self {
            backend,
            i18n: i18n.clone(),
            tx,
            rx,
            launch: launch.clone(),
            should_quit: false,
            focus: Focus::Transcript,
            current_route: Route::Main,
            route_stack: Vec::new(),
            overlay: None,
            overlay_stack: Vec::new(),
            seen_permission_request_ids: BTreeSet::new(),
            seen_user_input_request_ids: BTreeSet::new(),
            pending_permission_replay: None,
            flash: None,
            sessions: SessionListState {
                search_query: launch.initial_session_search.unwrap_or_default(),
                ..SessionListState::default()
            },
            transcript: TranscriptState::new(
                i18n,
                TranscriptDetailDefaults {
                    tool_output_expanded: launch.tui_config.transcript.tool_output_default_expanded,
                    thinking_expanded: launch.tui_config.transcript.thinking_default_expanded,
                },
            ),
            run_options: RunOptionsState::default(),
            composer: Editor::default(),
            composer_items: Vec::new(),
            slash_command_suggestions: None,
            dismissed_slash_command_suggestions_for: None,
            file_mention_suggestions: None,
            dismissed_file_mention_suggestions_for: None,
            prompt_history_search: None,
            selected_composer_item: None,
            draft_store,
            draft_store_path,
            draft_store_dirty: false,
            draft_store_last_persist_at: Instant::now()
                .checked_sub(Duration::from_millis(DRAFT_PERSIST_INTERVAL_MS))
                .unwrap_or_else(Instant::now),
            draft_store_reported_error: None,
            pending_draft_store_error,
            prompt_history,
            prompt_history_path,
            prompt_history_recall_original: None,
            prompt_history_recall_index: None,
            prompt_history_reported_error: None,
            pending_prompt_history_error,
            submitting_session_ids: HashSet::new(),
            layout: LayoutCache::default(),
            bootstrap_done: false,
            last_refresh_at: Instant::now()
                .checked_sub(Duration::from_millis(REFRESH_INTERVAL_MS))
                .unwrap_or_else(Instant::now),
            pending_ui_action: None,
            current_lineage: None,
            active_subscription: None,
            queue: ComposerQueue::new(),
            status_line,
            plugin_theme,
            keybindings,
            transcript_motion_prefix: None,
            last_ctrl_c_at: None,
            double_esc_window,
        };
        if let Some(draft) = app.draft_store.get(DraftSlot::NewSession).cloned() {
            app.restore_composer_draft(draft);
        }
        app
    }

    pub async fn run<B: RatatuiBackend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        self.bootstrap();

        let mut events = Some(EventStream::new());
        let mut ticker = interval(Duration::from_millis(UI_TICK_MS));

        loop {
            terminal
                .draw(|frame| self.draw(frame))
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;

            tokio::select! {
                maybe_event = async {
                    match events.as_mut() {
                        Some(events) => events.next().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match maybe_event {
                        Some(Ok(event)) => self.handle_terminal_event(event),
                        Some(Err(error)) => self.flash_error(self.i18n.text_args(
                            "flash-terminal-event-error",
                            &crate::fl_args!("error" => error.to_string()),
                        )),
                        None => self.should_quit = true,
                    }
                }
                maybe_message = self.rx.recv() => {
                    if let Some(message) = maybe_message {
                        self.handle_message(message);
                    } else {
                        self.should_quit = true;
                    }
                }
                _ = ticker.tick() => {
                    self.on_tick();
                }
            }

            if let Some(action) = self.pending_ui_action.take() {
                drop(events.take());
                self.run_ui_action(action, terminal)?;
                events = Some(EventStream::new());
            }

            if self.should_quit {
                break;
            }
        }

        Ok(())
    }

    fn bootstrap(&mut self) {
        if self.bootstrap_done {
            return;
        }

        self.bootstrap_done = true;
        self.request_sessions(false);

        if let Some(session_id) = self.launch.initial_session_id {
            self.open_session(
                session_id,
                ui_text::session_fallback_title(&self.i18n, session_id),
            );
        }
    }

    fn on_tick(&mut self) {
        let now = Instant::now();
        self.flush_input_buffers_if_due(now);
        self.refresh_status_line_if_due(now);
        self.poll_provider_studio_auth_if_due(now);

        if let Some(error) = self.pending_draft_store_error.take() {
            self.report_draft_store_error(error);
        }
        if let Some(error) = self.pending_prompt_history_error.take() {
            self.report_prompt_history_error(error);
        }

        if self
            .flash
            .as_ref()
            .is_some_and(|flash| Instant::now() >= flash.expires_at)
        {
            self.flash = None;
        }

        if let Some(session_id) = self.transcript.session_id
            && !self.transcript.loading_initial
            && !self.transcript.refreshing
            && !self.transcript.state_loading
            && self.last_refresh_at.elapsed() >= Duration::from_millis(REFRESH_INTERVAL_MS)
        {
            self.last_refresh_at = Instant::now();
            self.request_refresh(session_id, false);
        }

        self.sync_current_draft_slot();
        self.persist_draft_store_with_feedback(false);
    }

    fn poll_provider_studio_auth_if_due(&mut self, now: Instant) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            return;
        };

        if dialog.pending_auth_key.is_none()
            && let Some(interval) = provider_studio_auth_poll_interval(&dialog)
        {
            match dialog.next_auth_poll_at {
                Some(deadline) if now >= deadline => {
                    self.request_provider_studio_continue_auth(&mut dialog);
                }
                Some(_) => {}
                None => {
                    dialog.next_auth_poll_at = now.checked_add(interval).or(Some(now));
                }
            }
        }

        self.restore_provider_studio_dialog(host, dialog);
    }

    fn handle_terminal_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key_event(key),
            Event::Paste(text) => self.handle_paste(text),
            Event::Resize(_, _) => self.transcript.invalidate_render(),
            Event::Mouse(_) => {}
            Event::FocusGained | Event::FocusLost => {}
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        if matches!(key.kind, KeyEventKind::Release) {
            return;
        }

        self.flush_input_buffers_if_due(Instant::now());

        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            if let Some(session_id) = self.active_run_session_id() {
                self.request_cancel_run(session_id);
                self.last_ctrl_c_at = None;
                return;
            }
            let now = Instant::now();
            let double = self
                .last_ctrl_c_at
                .map(|prev| now.duration_since(prev) <= self.double_esc_window)
                .unwrap_or(false);
            if double {
                self.should_quit = true;
            } else {
                self.last_ctrl_c_at = Some(now);
                self.flash_warning(ui_text::t(&self.i18n, "flash-quit-confirm"));
            }
            return;
        }

        self.last_ctrl_c_at = None;

        if self.handle_overlay_key(key) {
            return;
        }

        if self.handle_route_key(key) {
            return;
        }

        self.maybe_capture_transcript_motion_prefix(key);

        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('s') | KeyCode::Char('S'))
        {
            self.open_resume_session_picker();
            return;
        }

        if !self.current_route_is_main() {
            return;
        }

        if self.focus != Focus::Composer {
            match key.code {
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('?') => {
                    self.route_stack.clear();
                    self.current_route = Route::Help(HelpOverlay::default());
                    return;
                }
                _ => {}
            }
        }

        if matches!(key.code, KeyCode::Tab)
            && self.focus != Focus::Composer
            && !(self.focus == Focus::Composer && self.slash_command_suggestions.is_some())
        {
            self.focus = Focus::Composer;
            self.slash_command_suggestions = None;
            return;
        }

        if matches!(key.code, KeyCode::BackTab) && self.focus != Focus::Composer {
            self.focus = Focus::Composer;
            self.slash_command_suggestions = None;
            return;
        }

        if matches!(key.code, KeyCode::Char('/')) && self.focus != Focus::Composer {
            match self.focus {
                Focus::Sessions => self.open_resume_session_picker(),
                Focus::Transcript => {
                    self.overlay = Some(Overlay::TranscriptSearch(
                        self.build_transcript_search_overlay(),
                    ));
                }
                Focus::Composer => unreachable!("composer focus is excluded above"),
            }
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('f')) {
            self.overlay = Some(Overlay::TranscriptSearch(
                self.build_transcript_search_overlay(),
            ));
            return;
        }

        if matches!(key.code, KeyCode::Char('p')) && key.modifiers.contains(KeyModifiers::ALT) {
            self.open_command_palette();
            return;
        }

        if self.focus == Focus::Transcript
            && !self.transcript.search_query.trim().is_empty()
            && matches!(key.code, KeyCode::Char('N'))
        {
            self.jump_search_match(false);
            return;
        }

        if self.focus == Focus::Transcript
            && !self.transcript.search_query.trim().is_empty()
            && matches!(key.code, KeyCode::Char('n'))
        {
            self.jump_search_match(true);
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('n')) {
            self.create_session(None);
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('r')) {
            self.continue_current_session();
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('s')) {
            self.open_resume_session_picker();
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('b')) {
            self.open_lineage_picker();
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('R')) {
            self.open_rename_session_overlay();
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('t')) {
            self.open_timeline_overlay(TIMELINE_EVENT_LIMIT);
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('P')) {
            self.open_plugin_workbench("");
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('[')) {
            self.open_parent_session();
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char(']')) {
            self.open_child_sessions_picker();
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('e')) {
            self.handle_export_command("");
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('v')) {
            self.pending_ui_action = Some(UiAction::PageTranscript);
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('u')) {
            self.open_user_input_overlay();
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('p')) {
            self.open_permission_overlay();
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('a')) {
            self.reply_permission(PermissionReplyKind::AllowOnce);
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('A')) {
            self.reply_permission(PermissionReplyKind::AllowAlways);
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('d')) {
            self.reply_permission(PermissionReplyKind::DenyOnce);
            return;
        }

        if self.focus != Focus::Composer && matches!(key.code, KeyCode::Char('D')) {
            self.reply_permission(PermissionReplyKind::DenyAlways);
            return;
        }

        match self.focus {
            Focus::Sessions => self.handle_sessions_key(key),
            Focus::Transcript => self.handle_transcript_key(key),
            Focus::Composer => self.handle_composer_key(key),
        }
        self.maybe_auto_open_pending_interactive_overlay();
    }

    fn maybe_capture_transcript_motion_prefix(&mut self, key: KeyEvent) {
        if self.focus != Focus::Transcript
            || !self.current_route_is_main()
            || self.overlay.is_some()
        {
            self.transcript_motion_prefix = None;
            return;
        }
        if !key.modifiers.is_empty() {
            self.transcript_motion_prefix = None;
            return;
        }
        match key.code {
            KeyCode::Char(digit @ '1'..='9') => {
                self.transcript_motion_prefix
                    .get_or_insert_with(String::new)
                    .push(digit);
            }
            KeyCode::Char(digit @ '0') if self.transcript_motion_prefix.is_some() => {
                if let Some(prefix) = self.transcript_motion_prefix.as_mut() {
                    prefix.push(digit);
                }
            }
            KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Char('h') | KeyCode::Char('l') => {}
            _ => {
                self.transcript_motion_prefix = None;
            }
        }
    }

    fn transcript_motion_count(&mut self) -> usize {
        self.transcript_motion_prefix
            .take()
            .and_then(|prefix| prefix.parse::<usize>().ok())
            .filter(|count| *count > 0)
            .unwrap_or(1)
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut overlay) = self.overlay.take() else {
            return false;
        };

        let close = match &mut overlay {
            Overlay::TranscriptSearch(dialog) => {
                self.handle_line_overlay_key(key, dialog, OverlayCommit::TranscriptSearch)
            }
            Overlay::SessionRename(dialog) => self.handle_session_rename_overlay_key(key, dialog),
            Overlay::AgentCreate(dialog) => self.handle_agent_create_overlay_key(key, dialog),
            Overlay::SettingsValueEdit(dialog) => {
                self.handle_settings_value_edit_overlay_key(key, dialog)
            }
            Overlay::RuntimeSettingEdit(dialog) => {
                self.handle_runtime_setting_edit_overlay_key(key, dialog)
            }
            Overlay::Choice(dialog) => self.handle_choice_overlay_key(key, dialog),
            Overlay::PermissionRuleEdit(dialog) => {
                self.handle_permission_rule_edit_overlay_key(key, dialog)
            }
            Overlay::FileAttach(dialog) => self.handle_file_attach_overlay_key(key, dialog),
            Overlay::PathBrowser(dialog) => self.handle_path_browser_overlay_key(key, dialog),
            Overlay::Permission(dialog) => self.handle_permission_overlay_key(key, dialog),
            Overlay::UserInputReply(dialog) => self.handle_user_input_overlay_key(key, dialog),
            Overlay::Confirm(dialog) => self.handle_confirm_overlay_key(key, dialog),
            Overlay::SessionSearch(dialog) => self.handle_session_search_overlay_key(key, dialog),
            Overlay::Picker(dialog) => self.handle_picker_overlay_key(key, dialog),
            Overlay::Timeline(dialog) => self.handle_timeline_overlay_key(key, dialog),
            Overlay::ProviderStudio(dialog) => self.handle_provider_studio_overlay_key(key, dialog),
            Overlay::ModelCatalogStudio(dialog) => {
                self.handle_model_catalog_studio_overlay_key(key, dialog)
            }
        };

        if !close {
            if self.overlay.is_none() {
                self.overlay = Some(overlay);
            }
        } else if self.overlay.is_none()
            && let Some(parent) = self.overlay_stack.pop()
        {
            self.overlay = Some(self.refresh_restored_overlay(parent));
        } else if self.overlay.is_none() {
            self.maybe_auto_open_pending_interactive_overlay();
        }

        true
    }

    fn handle_route_key(&mut self, key: KeyEvent) -> bool {
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        let mut route = match route {
            Route::Main => return false,
            route => route,
        };

        let close = match &mut route {
            Route::Main => false,
            Route::Help(dialog) => self.handle_help_overlay_key(key, dialog),
            Route::SettingsStudio(dialog) => self.handle_settings_studio_overlay_key(key, dialog),
            Route::AgentStudio(dialog) => self.handle_agent_studio_overlay_key(key, dialog),
            Route::PermissionStudio(dialog) => {
                self.handle_permission_studio_overlay_key(key, dialog)
            }
            Route::PermissionRuleStudio(dialog) => {
                self.handle_permission_rule_studio_overlay_key(key, dialog)
            }
            Route::SessionSearch(dialog) => self.handle_session_search_overlay_key(key, dialog),
            Route::Picker(dialog) => self.handle_picker_overlay_key(key, dialog),
            Route::SessionModelChooser(dialog) => {
                self.handle_session_model_chooser_overlay_key(key, dialog)
            }
            Route::Timeline(dialog) => self.handle_timeline_overlay_key(key, dialog),
            Route::PluginPolicyStudio(dialog) => self.handle_plugin_policy_studio_key(key, dialog),
            Route::PluginWorkbench(dialog) => self.handle_plugin_workbench_key(key, dialog),
            Route::ProviderStudio(dialog) => self.handle_provider_studio_overlay_key(key, dialog),
            Route::ModelCatalogStudio(dialog) => {
                self.handle_model_catalog_studio_overlay_key(key, dialog)
            }
        };

        if !close {
            if self.current_route_is_main() {
                self.current_route = route;
            }
        } else if self.current_route_is_main() {
            if let Some(parent) = self.route_stack.pop() {
                self.current_route = self.refresh_restored_route(parent);
            } else {
                self.current_route = Route::Main;
                self.maybe_auto_open_pending_interactive_overlay();
            }
        }

        true
    }

    fn handle_line_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut LineInputOverlay,
        commit: OverlayCommit,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(_, value) => {
                let value = value.trim().to_string();
                match commit {
                    OverlayCommit::TranscriptSearch => {
                        self.transcript.set_search_query(value);
                        self.jump_search_match(true);
                    }
                }
                true
            }
            InputDialogKeyResult::Continue => false,
        }
    }

    fn handle_confirm_overlay_key(&mut self, key: KeyEvent, dialog: &mut ConfirmOverlay) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.handle_confirm_action(dialog.action.clone());
                true
            }
            _ => false,
        }
    }

    fn handle_user_input_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        if Self::user_input_overlay_is_review(dialog) {
            return self.handle_user_input_review_decision_key(key, dialog);
        }
        if dialog.editing_custom {
            return match key.code {
                KeyCode::Esc => {
                    dialog.editing_custom = false;
                    false
                }
                KeyCode::Enter => {
                    let committed = Self::commit_user_input_custom_values(dialog);
                    let should_advance = dialog
                        .request
                        .questions
                        .get(dialog.state.selected_question())
                        .map(|question| committed && !question.multiple)
                        .unwrap_or(false);
                    if should_advance {
                        if Self::user_input_review_hidden(dialog) {
                            return self.submit_user_input_overlay(dialog);
                        }
                        Self::move_user_input_tab(dialog, 1);
                    }
                    false
                }
                _ => {
                    dialog.custom_input.handle_line_input_key(key);
                    false
                }
            };
        }

        match dialog.state.screen() {
            QuestionFlowScreen::Question => self.handle_user_input_question_key(key, dialog),
            QuestionFlowScreen::Review => self.handle_user_input_review_key(key, dialog),
        }
    }

    fn handle_user_input_review_decision_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        let option_count = Self::user_input_review_question(&dialog.request)
            .map(|question| question.options.len())
            .unwrap_or(0);
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => self.submit_user_input_overlay(dialog),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cancel_user_input_overlay(dialog)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                move_selected_index(&mut dialog.review_option, option_count, -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                move_selected_index(&mut dialog.review_option, option_count, 1);
                false
            }
            KeyCode::PageUp => {
                dialog.review_scroll = dialog.review_scroll.saturating_sub(12);
                false
            }
            KeyCode::PageDown => {
                dialog.review_scroll = dialog.review_scroll.saturating_add(12);
                false
            }
            KeyCode::Home => {
                dialog.review_scroll = 0;
                false
            }
            KeyCode::End => {
                dialog.review_scroll = u16::MAX;
                false
            }
            _ => false,
        }
    }

    fn handle_user_input_question_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => self.commit_user_input_question(dialog),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cancel_user_input_overlay(dialog)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                Self::move_user_input_option(dialog, -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                Self::move_user_input_option(dialog, 1);
                false
            }
            KeyCode::PageUp => {
                Self::move_user_input_question(dialog, -1);
                false
            }
            KeyCode::PageDown => {
                Self::move_user_input_question(dialog, 1);
                false
            }
            KeyCode::Home => {
                dialog.state.move_option_home();
                false
            }
            KeyCode::End => {
                Self::move_user_input_option_to_end(dialog);
                false
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                Self::move_user_input_tab(dialog, -1);
                false
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                Self::move_user_input_tab(dialog, 1);
                false
            }
            KeyCode::Char(' ') => {
                Self::toggle_user_input_option(dialog);
                false
            }
            KeyCode::Char('e') => {
                Self::begin_user_input_custom_edit(dialog);
                false
            }
            KeyCode::Char('c') | KeyCode::Delete | KeyCode::Backspace => {
                Self::clear_user_input_answer(dialog);
                false
            }
            _ => false,
        }
    }

    fn handle_user_input_review_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut UserInputOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => self.submit_user_input_overlay(dialog),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cancel_user_input_overlay(dialog)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                Self::move_user_input_question(dialog, -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                Self::move_user_input_question(dialog, 1);
                false
            }
            KeyCode::PageUp => {
                let review_mode = dialog.state.screen() == QuestionFlowScreen::Review;
                dialog
                    .state
                    .move_question_page(dialog.request.questions.len(), -1, 5);
                if !review_mode {
                    Self::sync_user_input_option_selection(dialog);
                }
                false
            }
            KeyCode::PageDown => {
                let review_mode = dialog.state.screen() == QuestionFlowScreen::Review;
                dialog
                    .state
                    .move_question_page(dialog.request.questions.len(), 1, 5);
                if !review_mode {
                    Self::sync_user_input_option_selection(dialog);
                }
                false
            }
            KeyCode::Home => {
                dialog
                    .state
                    .move_question_home(dialog.request.questions.len());
                false
            }
            KeyCode::End => {
                dialog
                    .state
                    .move_question_end(dialog.request.questions.len());
                false
            }
            KeyCode::Char('e') | KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                Self::focus_user_input_question(dialog, dialog.state.selected_question());
                false
            }
            KeyCode::Char('c') | KeyCode::Delete | KeyCode::Backspace => {
                Self::clear_user_input_answer(dialog);
                false
            }
            _ => false,
        }
    }

    fn cancel_user_input_overlay(&mut self, dialog: &UserInputOverlay) -> bool {
        let reply = UserInputReply {
            request_id: dialog.request.request_id.clone(),
            kind: UserInputReplyKind::Cancel,
            answers: BTreeMap::new(),
            reason: None,
        };
        self.request_user_input_reply(dialog.session_id, reply);
        true
    }

    fn submit_user_input_overlay(&mut self, dialog: &mut UserInputOverlay) -> bool {
        match Self::build_structured_user_input_reply(&self.i18n, dialog) {
            Ok(reply) => {
                self.request_user_input_reply(dialog.session_id, reply);
                true
            }
            Err(error) => {
                self.flash_warning(error);
                false
            }
        }
    }

    fn commit_user_input_question(&mut self, dialog: &mut UserInputOverlay) -> bool {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return false;
        };
        let is_custom = Self::selected_user_input_row_is_custom(dialog, question);
        let multiple = question.multiple;
        if is_custom {
            Self::begin_user_input_custom_edit(dialog);
            return false;
        }
        if multiple {
            Self::move_user_input_tab(dialog, 1);
            return false;
        }
        Self::select_user_input_option(dialog);
        if Self::user_input_review_hidden(dialog) {
            return self.submit_user_input_overlay(dialog);
        }
        Self::move_user_input_tab(dialog, 1);
        false
    }

    fn move_user_input_question(dialog: &mut UserInputOverlay, delta: isize) {
        if dialog.request.questions.is_empty() {
            dialog.state.clear();
            return;
        }
        let review_mode = dialog.state.screen() == QuestionFlowScreen::Review;
        dialog
            .state
            .move_question(dialog.request.questions.len(), delta);
        if review_mode {
            return;
        }
        Self::sync_user_input_option_selection(dialog);
    }

    fn focus_user_input_question(dialog: &mut UserInputOverlay, index: usize) {
        if dialog.request.questions.is_empty() {
            dialog.state.clear();
            return;
        }
        dialog
            .state
            .focus_question(index, dialog.request.questions.len());
        Self::sync_user_input_option_selection(dialog);
    }

    fn sync_user_input_option_selection(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            dialog.state.set_selected_option(0);
            return;
        };
        let row_count = Self::user_input_option_row_count(question);
        if row_count == 0 {
            dialog.state.set_selected_option(0);
            return;
        }
        let preferred = dialog
            .answers
            .get(&question.id)
            .map(|draft| Self::preferred_user_input_option_row(question, draft))
            .unwrap_or(0);
        dialog.state.set_selected_option(preferred);
        dialog.state.clamp_options(row_count);
    }

    fn move_user_input_option(dialog: &mut UserInputOverlay, delta: isize) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        let row_count = Self::user_input_option_row_count(question);
        if row_count == 0 {
            return;
        }
        dialog.state.move_option(row_count, delta);
    }

    fn move_user_input_option_to_end(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        let row_count = Self::user_input_option_row_count(question);
        if row_count == 0 {
            dialog.state.set_selected_option(0);
            return;
        }
        dialog.state.move_option_end(row_count);
    }

    fn move_user_input_tab(dialog: &mut UserInputOverlay, delta: isize) {
        if dialog.request.questions.is_empty() {
            dialog.state.clear();
            return;
        }
        if dialog.state.screen() == QuestionFlowScreen::Review {
            if delta < 0 {
                Self::focus_user_input_question(dialog, dialog.state.selected_question());
            }
            return;
        }
        let last_index = dialog.request.questions.len().saturating_sub(1);
        if delta < 0 {
            if dialog.state.selected_question() > 0 {
                Self::focus_user_input_question(dialog, dialog.state.selected_question() - 1);
            }
            return;
        }
        if dialog.state.selected_question() < last_index {
            Self::focus_user_input_question(dialog, dialog.state.selected_question() + 1);
            return;
        }
        if !Self::user_input_review_hidden(dialog) {
            dialog.state.focus_review(dialog.request.questions.len());
        }
    }

    fn toggle_user_input_option(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        let is_custom = Self::selected_user_input_row_is_custom(dialog, question);
        let allow_custom = question.allow_custom;
        let question_id = question.id.clone();
        let multiple = question.multiple;
        if is_custom || question.options.is_empty() {
            if allow_custom {
                Self::begin_user_input_custom_edit(dialog);
            }
            return;
        }
        let draft = dialog.answers.entry(question_id).or_default();
        if multiple {
            if !draft.option_indexes.insert(dialog.state.selected_option()) {
                draft.option_indexes.remove(&dialog.state.selected_option());
            }
        } else {
            draft.option_indexes.clear();
            draft.option_indexes.insert(dialog.state.selected_option());
            draft.custom_values.clear();
        }
    }

    fn select_user_input_option(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        if Self::selected_user_input_row_is_custom(dialog, question) {
            return;
        }
        let question_id = question.id.clone();
        let draft = dialog.answers.entry(question_id).or_default();
        draft.option_indexes.clear();
        draft.option_indexes.insert(dialog.state.selected_option());
        draft.custom_values.clear();
    }

    fn begin_user_input_custom_edit(dialog: &mut UserInputOverlay) -> bool {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return false;
        };
        let allow_custom = question.allow_custom;
        let selected_option = question.options.len();
        let question_id = question.id.clone();
        if !allow_custom {
            return false;
        }
        dialog.state.focus_question(
            dialog.state.selected_question(),
            dialog.request.questions.len(),
        );
        dialog.state.set_selected_option(selected_option);
        let existing = dialog
            .answers
            .get(&question_id)
            .map(|draft| draft.custom_values.join(", "))
            .unwrap_or_default();
        dialog.custom_input.set_text(existing);
        dialog.editing_custom = true;
        true
    }

    fn commit_user_input_custom_values(dialog: &mut UserInputOverlay) -> bool {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            dialog.editing_custom = false;
            return false;
        };
        let multiple = question.multiple;
        let question_id = question.id.clone();
        let custom_row = question.options.len();
        dialog.custom_input.flush_all_pending_input();
        let parsed = dialog
            .custom_input
            .text()
            .split([',', '\n'])
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        let draft = dialog.answers.entry(question_id).or_default();
        draft.custom_values = if multiple {
            parsed
        } else {
            parsed.into_iter().take(1).collect()
        };
        if !draft.custom_values.is_empty() && !multiple {
            draft.option_indexes.clear();
        }
        dialog.state.set_selected_option(custom_row);
        dialog.editing_custom = false;
        !draft.custom_values.is_empty()
    }

    fn clear_user_input_answer(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            return;
        };
        dialog.answers.remove(&question.id);
        dialog.custom_input.clear();
        dialog.editing_custom = false;
    }

    fn build_structured_user_input_reply(
        i18n: &I18n,
        dialog: &mut UserInputOverlay,
    ) -> std::result::Result<UserInputReply, String> {
        if let Some(question) = Self::user_input_review_question(&dialog.request) {
            let Some(option) = question.options.get(dialog.review_option) else {
                return Err(ui_text::t(i18n, "overlay-user-input-no-questions"));
            };
            return Ok(UserInputReply {
                request_id: dialog.request.request_id.clone(),
                kind: UserInputReplyKind::Submit,
                answers: BTreeMap::from([(question.id.clone(), vec![option.label.clone()])]),
                reason: None,
            });
        }

        let mut answers = BTreeMap::new();
        for index in 0..dialog.request.questions.len() {
            let question = &dialog.request.questions[index];
            let values = dialog
                .answers
                .get(&question.id)
                .map(|draft| user_input_answer_values(question, draft))
                .unwrap_or_default();
            if values.is_empty() {
                let label = user_input_question_label(question).to_string();
                Self::focus_user_input_question(dialog, index);
                return Err(i18n.text_args(
                    "overlay-user-input-missing-answer",
                    &crate::fl_args!("label" => label),
                ));
            }
            answers.insert(question.id.clone(), values);
        }

        Ok(UserInputReply {
            request_id: dialog.request.request_id.clone(),
            kind: UserInputReplyKind::Submit,
            answers,
            reason: None,
        })
    }

    fn user_input_review_hidden(dialog: &UserInputOverlay) -> bool {
        dialog.request.questions.len() == 1
            && dialog
                .request
                .questions
                .first()
                .map(|question| !question.multiple)
                .unwrap_or(false)
    }

    fn user_input_option_row_count(question: &UserInputQuestion) -> usize {
        question.options.len() + usize::from(question.allow_custom)
    }

    fn preferred_user_input_option_row(
        question: &UserInputQuestion,
        draft: &UserInputAnswerDraft,
    ) -> usize {
        if let Some(index) = draft.option_indexes.iter().next().copied() {
            return index.min(question.options.len().saturating_sub(1));
        }
        if !draft.custom_values.is_empty() && question.allow_custom {
            return question.options.len();
        }
        0
    }

    fn selected_user_input_row_is_custom(
        dialog: &UserInputOverlay,
        question: &UserInputQuestion,
    ) -> bool {
        question.allow_custom && dialog.state.selected_option() >= question.options.len()
    }

    fn handle_permission_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionOverlay,
    ) -> bool {
        let choice_count = permission_overlay_choices(&self.i18n).len();
        match key.code {
            KeyCode::Esc => true,
            _ if dialog.selection.handle_navigation_key(key, choice_count, 8) => false,
            KeyCode::Enter => {
                let choice = permission_overlay_choice(dialog.selection.selected);
                self.submit_permission_reply(
                    dialog.session_id,
                    dialog.request.clone(),
                    choice.kind,
                    choice.scope,
                    permission_overlay_choice_label(&self.i18n, choice),
                );
                true
            }
            KeyCode::Char('a') => {
                self.submit_permission_reply(
                    dialog.session_id,
                    dialog.request.clone(),
                    PermissionReplyKind::AllowOnce,
                    None,
                    ui_text::permission_reply_label(&self.i18n, PermissionReplyKind::AllowOnce),
                );
                true
            }
            KeyCode::Char('s') | KeyCode::Char('A') => {
                self.submit_permission_reply(
                    dialog.session_id,
                    dialog.request.clone(),
                    PermissionReplyKind::AllowAlways,
                    Some(PermissionScope::Session),
                    ui_text::permission_reply_label(&self.i18n, PermissionReplyKind::AllowAlways),
                );
                true
            }
            KeyCode::Char('d') => {
                self.submit_permission_reply(
                    dialog.session_id,
                    dialog.request.clone(),
                    PermissionReplyKind::DenyOnce,
                    None,
                    ui_text::permission_reply_label(&self.i18n, PermissionReplyKind::DenyOnce),
                );
                true
            }
            KeyCode::Char('e') => {
                let return_overlay = Overlay::Permission(dialog.clone());
                self.overlay_stack.push(return_overlay.clone());
                self.open_permission_rule_editor_from_request(&dialog.request);
                if let Some(Overlay::PermissionRuleEdit(edit)) = self.overlay.as_mut() {
                    edit.return_overlay = Some(Box::new(return_overlay));
                }
                true
            }
            _ => false,
        }
    }

    fn handle_session_rename_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut LineInputOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(_, value) => self.submit_session_rename(value.as_str()),
            InputDialogKeyResult::Continue => false,
        }
    }

    fn handle_agent_create_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut LineInputOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(_, value) => self.create_agent_from_list(value.as_str()),
            InputDialogKeyResult::Continue => false,
        }
    }

    fn handle_settings_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SettingsStudioOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('r') => {
                self.refresh_settings_studio_overlay(dialog);
                false
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l')
                if dialog.state.focus() == SettingsStudioFocus::Navigation =>
            {
                dialog.state.set_focus(SettingsStudioFocus::Items);
                false
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h')
                if dialog.state.focus() == SettingsStudioFocus::Items =>
            {
                dialog.state.set_focus(SettingsStudioFocus::Navigation);
                false
            }
            KeyCode::PageUp => {
                dialog.state.move_selection_page(-1, 10);
                false
            }
            KeyCode::PageDown => {
                dialog.state.move_selection_page(1, 10);
                false
            }
            KeyCode::Home => {
                dialog.state.move_selection_home();
                false
            }
            KeyCode::End => {
                dialog.state.move_selection_end();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.state.move_selection(-1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.state.move_selection(1);
                false
            }
            KeyCode::Enter => self.activate_settings_studio_selection(dialog),
            _ => false,
        }
    }

    fn handle_agent_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut AgentStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.workbench.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    dialog.workbench.editor = None;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) = self.commit_agent_studio_editor(dialog, action, input) {
                        self.flash_error(error);
                    } else {
                        dialog.workbench.editor = None;
                    }
                }
            }
            return false;
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('r') => {
                self.refresh_agent_studio_overlay(dialog);
                false
            }
            KeyCode::Char('o') => {
                self.open_agent_profile_source(&dialog.profile);
                false
            }
            KeyCode::Char('p') => {
                self.route_stack.push(Route::AgentStudio(dialog.clone()));
                self.open_agent_permission_studio(dialog.agent_name.as_str());
                false
            }
            _ if dialog.workbench.list.handle_navigation_key(key, 10) => false,
            KeyCode::Enter => self.activate_agent_studio_selection(dialog),
            _ => false,
        }
    }

    fn handle_permission_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) = self.commit_permission_studio_editor(dialog, action, input)
                    {
                        self.flash_error(error);
                    } else {
                        dialog.editor = None;
                    }
                }
            }
            return false;
        }

        match key.code {
            KeyCode::Esc => match dialog.pane_focus {
                PermissionStudioPaneFocus::Navigation => true,
                PermissionStudioPaneFocus::Content => {
                    set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Navigation);
                    false
                }
            },
            KeyCode::Char('r') => {
                self.refresh_permission_studio_overlay(dialog);
                false
            }
            KeyCode::Char('a') => {
                self.open_permission_studio_add_current(dialog);
                false
            }
            KeyCode::Char('e') => self.activate_permission_studio_selection(dialog),
            KeyCode::Char('y') => {
                self.open_permission_studio_duplicate_current(dialog);
                false
            }
            KeyCode::Char('d') => {
                self.open_permission_studio_delete_current(dialog);
                false
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l')
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
                false
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h')
                if dialog.pane_focus == PermissionStudioPaneFocus::Content =>
            {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Navigation);
                false
            }
            KeyCode::PageUp if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_page(&mut dialog.nav, -1, 10);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::PageDown if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_page(&mut dialog.nav, 1, 10);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::Home if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_home(&mut dialog.nav);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::End if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                permission_studio_nav_move_end(&mut dialog.nav);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::Up | KeyCode::Char('k')
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                permission_studio_nav_move_step(&mut dialog.nav, -1);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::Down | KeyCode::Char('j')
                if dialog.pane_focus == PermissionStudioPaneFocus::Navigation =>
            {
                permission_studio_nav_move_step(&mut dialog.nav, 1);
                self.apply_permission_studio_nav_selection(dialog);
                false
            }
            KeyCode::PageUp => {
                dialog.state.move_selection_page(-1, 10);
                false
            }
            KeyCode::PageDown => {
                dialog.state.move_selection_page(1, 10);
                false
            }
            KeyCode::Home => {
                dialog.state.move_selection_home();
                false
            }
            KeyCode::End => {
                dialog.state.move_selection_end();
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.state.move_selection(-1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.state.move_selection(1);
                false
            }
            KeyCode::Enter if dialog.pane_focus == PermissionStudioPaneFocus::Navigation => {
                set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
                false
            }
            KeyCode::Enter => self.activate_permission_studio_selection(dialog),
            _ => false,
        }
    }

    fn handle_permission_rule_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionRuleStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.workbench.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => {}
                EditorDialogKeyResult::Close => {
                    dialog.workbench.editor = None;
                }
                EditorDialogKeyResult::Submit(action, input) => {
                    if let Err(error) =
                        self.commit_permission_rule_studio_editor(dialog, action, input)
                    {
                        self.flash_error(error);
                    } else {
                        dialog.workbench.editor = None;
                    }
                }
            }
            return false;
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('r') => {
                self.refresh_permission_rule_studio(dialog);
                false
            }
            KeyCode::Char('b') => {
                self.open_selected_permission_rule_path_browser(dialog);
                false
            }
            _ if dialog.workbench.list.handle_navigation_key(key, 8) => false,
            KeyCode::Enter => self.activate_permission_rule_studio_selection(dialog),
            _ => false,
        }
    }

    fn activate_permission_rule_studio_selection(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
    ) -> bool {
        let Some(item) = dialog.workbench.list.selected_item().cloned() else {
            return false;
        };
        match item.action {
            PermissionRuleStudioAction::SubjectKind => self.open_permission_rule_choice_overlay(
                dialog,
                PermissionRuleStudioChoiceField::SubjectKind,
            ),
            PermissionRuleStudioAction::PathAccessKind => self.open_permission_rule_choice_overlay(
                dialog,
                PermissionRuleStudioChoiceField::PathAccessKind,
            ),
            PermissionRuleStudioAction::Scope => self.open_permission_rule_choice_overlay(
                dialog,
                PermissionRuleStudioChoiceField::Scope,
            ),
            PermissionRuleStudioAction::Mode => self
                .open_permission_rule_choice_overlay(dialog, PermissionRuleStudioChoiceField::Mode),
            PermissionRuleStudioAction::ToolName => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::ToolName,
            ),
            PermissionRuleStudioAction::Qualifier => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::Qualifier,
            ),
            PermissionRuleStudioAction::WorkspaceRoot => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::WorkspaceRoot,
            ),
            PermissionRuleStudioAction::TargetPath => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::TargetPath,
            ),
            PermissionRuleStudioAction::NetworkTarget => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::NetworkTarget,
            ),
            PermissionRuleStudioAction::SessionId => self.open_permission_rule_studio_editor(
                dialog,
                PermissionRuleStudioEditField::SessionId,
            ),
            PermissionRuleStudioAction::BrowseWorkspaceRoot => {
                self.open_permission_rule_path_browser(
                    dialog,
                    PermissionRuleStudioPathField::WorkspaceRoot,
                );
            }
            PermissionRuleStudioAction::BrowseTargetPath => {
                self.open_permission_rule_path_browser(
                    dialog,
                    PermissionRuleStudioPathField::TargetPath,
                );
            }
            PermissionRuleStudioAction::Save => {
                if let Err(error) = self.commit_permission_rule_studio_save(dialog) {
                    self.flash_error(error);
                }
            }
            PermissionRuleStudioAction::Revoke => {
                if let Some(rule_id) = dialog.rule_id {
                    match self.block_on_async(self.backend.revoke_permission_rule(rule_id)) {
                        Ok(_) => {
                            self.flash_success(self.i18n.text_args(
                                "flash-permission-rule-revoked",
                                &crate::fl_args!(
                                    "name" => permission_rule_draft_label(&self.i18n, &dialog.draft)
                                ),
                            ));
                            self.current_route = self.route_stack.pop().unwrap_or(Route::Main);
                        }
                        Err(error) => self.flash_error(error),
                    }
                }
            }
        }
        false
    }

    fn open_permission_rule_choice_overlay(
        &mut self,
        dialog: &PermissionRuleStudioOverlay,
        field: PermissionRuleStudioChoiceField,
    ) {
        let (title, prompt, input, all_items, allow_clear) =
            permission_rule_choice_overlay_spec(&self.i18n, &dialog.draft, field);
        self.open_choice_overlay(self.build_choice_overlay(
            title,
            prompt,
            input,
            all_items,
            ChoiceOverlayAction::PermissionRuleStudio(field),
            allow_clear,
            ChoiceOverlayStyle::SelectOnly,
        ));
    }

    fn open_permission_rule_studio_editor(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
        field: PermissionRuleStudioEditField,
    ) {
        let (title, prompt, footer, value) =
            permission_rule_editor_spec(&self.i18n, &dialog.draft, field);
        dialog.workbench.editor = Some(PermissionRuleStudioEditor::new(
            title,
            prompt,
            footer,
            false,
            Editor::from_text(value),
            field,
        ));
    }

    fn commit_permission_rule_studio_editor(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
        field: PermissionRuleStudioEditField,
        input: String,
    ) -> UiResult<()> {
        match field {
            PermissionRuleStudioEditField::ToolName => {
                dialog.draft.tool_name = input.trim().to_string();
            }
            PermissionRuleStudioEditField::Qualifier => {
                dialog.draft.qualifier = input.trim().to_string();
            }
            PermissionRuleStudioEditField::WorkspaceRoot => {
                dialog.draft.workspace_root = input.trim().to_string();
            }
            PermissionRuleStudioEditField::TargetPath => {
                dialog.draft.target_path = input.trim().to_string();
            }
            PermissionRuleStudioEditField::NetworkTarget => {
                dialog.draft.network_target = input.trim().to_string();
            }
            PermissionRuleStudioEditField::SessionId => {
                let trimmed = input.trim();
                if !trimmed.is_empty() && trimmed.parse::<i64>().is_err() {
                    return Err(ui_text::t(
                        &self.i18n,
                        "permission-rule-error-session-id-integer",
                    ));
                }
                dialog.draft.session_id = trimmed.to_string();
            }
        }
        self.refresh_permission_rule_studio(dialog);
        Ok(())
    }

    fn commit_permission_rule_studio_save(
        &mut self,
        dialog: &mut PermissionRuleStudioOverlay,
    ) -> UiResult<()> {
        let draft = dialog.draft.clone();
        match draft.subject_kind {
            PermissionRuleSubjectKind::Tool if draft.tool_name.trim().is_empty() => {
                return Err(ui_text::t(
                    &self.i18n,
                    "permission-rule-error-tool-name-required",
                ));
            }
            PermissionRuleSubjectKind::PathAccess => {
                if draft.path_access_kind.trim().is_empty() {
                    return Err(ui_text::t(
                        &self.i18n,
                        "permission-rule-error-path-access-kind-required",
                    ));
                }
                if draft.target_path.trim().is_empty() {
                    return Err(ui_text::t(
                        &self.i18n,
                        "permission-rule-error-target-path-required",
                    ));
                }
            }
            PermissionRuleSubjectKind::NetworkAccess if draft.network_target.trim().is_empty() => {
                return Err(ui_text::t(
                    &self.i18n,
                    "permission-rule-error-network-target-required",
                ));
            }
            _ => {}
        }
        if draft.scope == "session" {
            let trimmed = draft.session_id.trim();
            if trimmed.is_empty() {
                return Err(ui_text::t(
                    &self.i18n,
                    "permission-rule-error-session-id-required",
                ));
            }
            trimmed
                .parse::<i64>()
                .map_err(|_| ui_text::t(&self.i18n, "permission-rule-error-session-id-integer"))?;
        }
        let params = permission_rule_params_from_draft(&draft);
        let saved = match dialog.rule_id {
            Some(rule_id) => self
                .block_on_async(self.backend.replace_permission_rule(rule_id, params))
                .map_err(|error| error.to_string())?,
            None => self
                .block_on_async(self.backend.create_permission_rule(params))
                .map_err(|error| error.to_string())?,
        };
        dialog.rule_id = Some(saved.id);
        dialog.workbench.title = format!(
            "{} · {}",
            ui_text::t(&self.i18n, "overlay-permission-rule-workbench-title"),
            permission_rule_label(&self.i18n, &saved)
        );
        dialog.draft = permission_rule_draft_from_resource(&saved);
        self.flash_success(self.i18n.text_args(
            "flash-permission-rule-saved",
            &crate::fl_args!("name" => permission_rule_label(&self.i18n, &saved)),
        ));
        self.refresh_permission_rule_studio(dialog);
        Ok(())
    }

    fn open_selected_permission_rule_path_browser(&mut self, dialog: &PermissionRuleStudioOverlay) {
        let Some(item) = dialog.workbench.list.selected_item() else {
            return;
        };
        match item.action {
            PermissionRuleStudioAction::BrowseWorkspaceRoot => self
                .open_permission_rule_path_browser(
                    dialog,
                    PermissionRuleStudioPathField::WorkspaceRoot,
                ),
            PermissionRuleStudioAction::BrowseTargetPath => self.open_permission_rule_path_browser(
                dialog,
                PermissionRuleStudioPathField::TargetPath,
            ),
            _ => {}
        }
    }

    fn open_permission_rule_path_browser(
        &mut self,
        dialog: &PermissionRuleStudioOverlay,
        field: PermissionRuleStudioPathField,
    ) {
        let (title, prompt, mode, initial) =
            permission_rule_path_browser_spec(&self.i18n, &dialog.draft, field);
        self.overlay = Some(Overlay::PathBrowser(self.build_path_browser_overlay(
            title,
            prompt,
            mode,
            initial,
            PathBrowserTarget::PermissionRuleStudio(field),
        )));
    }

    fn refresh_path_browser_overlay_with_root(
        workspace_root: &Path,
        dialog: &mut PathBrowserOverlay,
    ) {
        dialog.items = Self::path_browser_entries_with_root(workspace_root, dialog);
        dialog.clamp_selection();
    }

    fn handle_path_browser_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PathBrowserOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Tab => {
                dialog.fill_input_from_selected();
                false
            }
            KeyCode::Char('h') => {
                self.path_browser_navigate_parent(dialog);
                false
            }
            KeyCode::Char('l') => {
                self.path_browser_open_entry(dialog);
                false
            }
            KeyCode::Enter => self.path_browser_activate(dialog),
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::refresh_path_browser_overlay_with_root(
                            self.backend.workspace_root(),
                            dialog,
                        );
                    }
                    false
                }
            },
        }
    }

    fn path_browser_activate(&mut self, dialog: &mut PathBrowserOverlay) -> bool {
        if let Some(selection) = dialog.selected_row() {
            return match selection {
                SearchListRow::Item(entry) => {
                    self.commit_path_browser_selection(dialog, entry.path)
                }
                SearchListRow::Custom(value) => {
                    let path = Self::resolve_browser_input_path_with_root(
                        self.backend.workspace_root(),
                        value.raw.as_str(),
                    );
                    self.commit_path_browser_selection(dialog, path)
                }
                SearchListRow::Clear(_) => false,
            };
        }
        let raw = dialog.input.text().trim();
        if raw.is_empty() {
            return false;
        }
        let path = Self::resolve_browser_input_path_with_root(self.backend.workspace_root(), raw);
        self.commit_path_browser_selection(dialog, path)
    }

    fn path_browser_open_entry(&self, dialog: &mut PathBrowserOverlay) {
        if let Some(entry) = dialog.items.get(dialog.selected) {
            if entry.is_dir {
                dialog.input.set_text(entry.path.display().to_string());
                Self::refresh_path_browser_overlay_with_root(self.backend.workspace_root(), dialog);
            }
        }
    }

    fn path_browser_navigate_parent(&self, dialog: &mut PathBrowserOverlay) {
        let current = Self::resolve_browser_input_path_with_root(
            self.backend.workspace_root(),
            dialog.input.text().trim(),
        );
        let parent = current.parent().map(Path::to_path_buf);
        if let Some(parent) = parent {
            dialog.input.set_text(parent.display().to_string());
            Self::refresh_path_browser_overlay_with_root(self.backend.workspace_root(), dialog);
        }
    }

    fn commit_path_browser_selection(
        &mut self,
        dialog: &PathBrowserOverlay,
        path: PathBuf,
    ) -> bool {
        match dialog.meta.target {
            PathBrowserTarget::PermissionRuleStudio(field) => {
                let workspace_root = self.backend.workspace_root();
                let value = match field {
                    PermissionRuleStudioPathField::WorkspaceRoot => path.display().to_string(),
                    PermissionRuleStudioPathField::TargetPath => path
                        .strip_prefix(workspace_root)
                        .ok()
                        .map(|relative| relative.display().to_string())
                        .filter(|relative| !relative.is_empty())
                        .unwrap_or_else(|| path.display().to_string()),
                };
                match &mut self.current_route {
                    Route::PermissionRuleStudio(route) => {
                        match field {
                            PermissionRuleStudioPathField::WorkspaceRoot => {
                                route.draft.workspace_root = value;
                            }
                            PermissionRuleStudioPathField::TargetPath => {
                                route.draft.target_path = value;
                            }
                        }
                        refresh_permission_rule_studio_dialog(&self.i18n, route);
                    }
                    _ => self
                        .flash_error(ui_text::t(&self.i18n, "flash-permission-rule-context-lost")),
                }
                true
            }
        }
    }

    fn resolve_browser_input_path_with_root(workspace_root: &Path, raw: &str) -> PathBuf {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return workspace_root.to_path_buf();
        }
        let path = PathBuf::from(trimmed);
        if path.is_absolute() {
            path
        } else {
            workspace_root.join(path)
        }
    }

    fn path_browser_entries_with_root(
        workspace_root: &Path,
        dialog: &PathBrowserOverlay,
    ) -> Vec<PathBrowserItem> {
        let raw = dialog.input.text().trim();
        let resolved = Self::resolve_browser_input_path_with_root(workspace_root, raw);
        let (directory, needle) = if resolved.is_dir() {
            (resolved, String::new())
        } else {
            (
                resolved
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| workspace_root.to_path_buf()),
                resolved
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default(),
            )
        };

        let mut entries = Vec::new();
        if let Some(parent) = directory.parent() {
            entries.push(PathBrowserItem {
                path: parent.to_path_buf(),
                label: "../".to_string(),
                detail: parent.display().to_string(),
                is_dir: true,
            });
        }
        let Ok(read_dir) = fs::read_dir(&directory) else {
            return entries;
        };

        let mut children = read_dir
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let path = entry.path();
                let is_dir = path.is_dir();
                if matches!(dialog.meta.mode, PathBrowserMode::DirectoryOnly) && !is_dir {
                    return None;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy().to_string();
                if !needle.is_empty() && !name.to_ascii_lowercase().contains(needle.as_str()) {
                    return None;
                }
                Some(PathBrowserItem {
                    label: if is_dir {
                        format!("{name}/")
                    } else {
                        name.clone()
                    },
                    detail: path.display().to_string(),
                    path,
                    is_dir,
                })
            })
            .collect::<Vec<_>>();
        children.sort_by(|left, right| {
            (!left.is_dir, left.label.to_ascii_lowercase())
                .cmp(&(!right.is_dir, right.label.to_ascii_lowercase()))
        });
        entries.extend(children);
        entries
    }

    fn activate_agent_studio_selection(&mut self, dialog: &mut AgentStudioOverlay) -> bool {
        let Some(item) = dialog.workbench.list.selected_item().cloned() else {
            return false;
        };
        match item.action {
            AgentStudioAction::Edit(field) => {
                if !dialog.editable {
                    self.flash_warning(agent_read_only_edit_message(&self.i18n));
                    return false;
                }
                self.open_agent_studio_editor(dialog, field);
            }
            AgentStudioAction::OpenPermissionWorkbench => {
                self.route_stack.push(Route::AgentStudio(dialog.clone()));
                self.open_agent_permission_studio(dialog.agent_name.as_str());
            }
            AgentStudioAction::OpenSource => self.open_agent_profile_source(&dialog.profile),
        }
        false
    }

    fn open_agent_studio_editor(
        &mut self,
        dialog: &mut AgentStudioOverlay,
        field: AgentStudioField,
    ) {
        let (title, prompt, footer, multiline, input) =
            agent_studio_editor_config(&self.i18n, &dialog.profile, field);
        dialog.workbench.editor = Some(AgentStudioEditor::new(
            title,
            prompt,
            footer,
            multiline,
            input,
            AgentStudioEditorAction::Field(field),
        ));
    }

    fn commit_agent_studio_editor(
        &mut self,
        dialog: &mut AgentStudioOverlay,
        action: AgentStudioEditorAction,
        input: String,
    ) -> UiResult<()> {
        match action {
            AgentStudioEditorAction::Field(field) => {
                match dialog.storage {
                    AgentProfileStorage::Config => {
                        let (path, value) = agent_studio_field_setting_value(
                            &self.i18n,
                            dialog.agent_name.as_str(),
                            field,
                            input.as_str(),
                        )?;
                        if let Some(value) = value {
                            self.block_on_async(
                                self.backend.set_config_setting(path.as_str(), value),
                            )
                            .map_err(|error| error.to_string())?;
                            self.flash_success(settings_path_updated_message(
                                &self.i18n,
                                path.as_str(),
                            ));
                        } else {
                            self.block_on_async(self.backend.delete_config_setting(path.as_str()))
                                .map_err(|error| error.to_string())?;
                            self.flash_success(settings_path_cleared_message(
                                &self.i18n,
                                path.as_str(),
                            ));
                        }
                    }
                    AgentProfileStorage::Markdown => {
                        let mut profile = dialog.profile.clone();
                        apply_agent_studio_field_to_profile(&mut profile, field, input.as_str());
                        self.persist_agent_markdown_profile(&profile)?;
                    }
                    AgentProfileStorage::BuiltIn | AgentProfileStorage::Runtime => {
                        return Err(agent_read_only_edit_message(&self.i18n));
                    }
                }
                self.refresh_agent_studio_overlay(dialog);
            }
        }
        Ok(())
    }

    fn persist_agent_markdown_profile(&mut self, profile: &AgentProfile) -> UiResult<()> {
        let path = profile
            .source_path
            .as_ref()
            .ok_or_else(|| agent_read_only_edit_message(&self.i18n))?;
        let text = agent_markdown_document(&profile.frontmatter, profile.prompt.as_str())?;
        fs::write(path, text).map_err(|error| {
            self.i18n.text_args(
                "flash-agent-source-write-failed",
                &crate::fl_args!(
                    "path" => path.display().to_string(),
                    "error" => error.to_string(),
                ),
            )
        })?;
        self.block_on_async(self.backend.reload_runtime())
            .map_err(|error| error.to_string())?;
        self.flash_success(self.i18n.text_args(
            "flash-agent-source-updated",
            &crate::fl_args!("path" => path.display().to_string()),
        ));
        Ok(())
    }

    fn activate_permission_studio_selection(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
    ) -> bool {
        if dialog.pane_focus == PermissionStudioPaneFocus::Navigation {
            set_permission_studio_pane_focus(dialog, PermissionStudioPaneFocus::Content);
            return false;
        }
        let Some(item) = dialog.state.selected_item().cloned() else {
            return false;
        };
        match item.action {
            PermissionStudioAction::Noop => return false,
            PermissionStudioAction::EditMode(target) => {
                if !dialog.editable {
                    self.flash_warning(permission_studio_read_only_message(
                        &self.i18n,
                        &dialog.source,
                    ));
                    return false;
                }
                self.open_permission_studio_mode_overlay(dialog, target);
                return false;
            }
            PermissionStudioAction::EditText(target) => {
                if !dialog.editable {
                    self.flash_warning(permission_studio_read_only_message(
                        &self.i18n,
                        &dialog.source,
                    ));
                    return false;
                }
                self.open_permission_studio_text_editor(dialog, target);
                return false;
            }
        }
    }

    fn apply_permission_studio_nav_selection(&mut self, dialog: &mut PermissionStudioOverlay) {
        let Some(item) = dialog.nav.selected_item().cloned() else {
            return;
        };
        if !item.selectable {
            return;
        };
        self.set_permission_studio_page_with_section(
            dialog,
            item.page,
            item.section,
            dialog.state.focus(),
        );
    }

    fn open_permission_studio_mode_overlay(
        &mut self,
        dialog: &PermissionStudioOverlay,
        target: PermissionStudioModeTarget,
    ) {
        self.open_choice_overlay(self.build_choice_overlay(
            settings_edit_title(
                &self.i18n,
                permission_studio_mode_target_label(&self.i18n, &target).as_str(),
            ),
            String::new(),
            Editor::from_text(permission_studio_mode_target_input_text(dialog, &target)),
            permission_mode_choice_items(&self.i18n),
            ChoiceOverlayAction::PermissionStudioMode(target),
            true,
            ChoiceOverlayStyle::SelectOnly,
        ));
    }

    fn open_permission_studio_text_editor(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        target: PermissionStudioTextTarget,
    ) {
        let title = settings_edit_title(
            &self.i18n,
            permission_studio_text_target_label(&self.i18n, &target).as_str(),
        );
        let prompt = String::new();
        let footer = editor_save_footer(&self.i18n, false);
        let input = Editor::from_text(permission_studio_text_target_input_text(&target));
        dialog.editor = Some(PermissionStudioEditor::new(
            title,
            prompt,
            footer,
            false,
            input,
            PermissionStudioEditorAction::Text(target),
        ));
    }

    fn open_permission_studio_creator(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        action: PermissionStudioEditorAction,
    ) {
        let (title, prompt) = permission_studio_creator_spec(&self.i18n, &action);
        let input = Editor::from_text(permission_studio_creator_input_text(&action));
        dialog.editor = Some(PermissionStudioEditor::new(
            title,
            prompt,
            editor_save_footer(&self.i18n, false),
            false,
            input,
            action,
        ));
    }

    fn commit_permission_studio_editor(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        action: PermissionStudioEditorAction,
        input: String,
    ) -> UiResult<()> {
        match action {
            PermissionStudioEditorAction::Text(target) => {
                let mut permission = dialog.permission.clone();
                let next_page = apply_permission_studio_text_input(
                    &self.i18n,
                    &mut permission,
                    &target,
                    input.as_str(),
                )?;
                self.persist_permission_studio(dialog, permission)?;
                let next_section = match next_page {
                    PermissionStudioPage::PathRules => Some(PermissionStudioSectionId::PathRules),
                    PermissionStudioPage::NetworkRules => {
                        Some(PermissionStudioSectionId::NetworkRules)
                    }
                    PermissionStudioPage::ToolTags => Some(PermissionStudioSectionId::ToolTags),
                    PermissionStudioPage::ToolNames => Some(PermissionStudioSectionId::ToolNames),
                    PermissionStudioPage::ToolCommandRules => {
                        Some(PermissionStudioSectionId::ToolCommandRules)
                    }
                    PermissionStudioPage::PathDefaults
                    | PermissionStudioPage::NetworkZones
                    | PermissionStudioPage::Overview => None,
                };
                self.set_permission_studio_page_with_section(
                    dialog,
                    next_page,
                    next_section,
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddPathRule { duplicate_from } => {
                let pattern = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-path-rules").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let rule = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .path
                            .as_ref()
                            .and_then(|path| path.rules.get(from.as_str()))
                            .cloned()
                    })
                    .unwrap_or_else(|| {
                        PathAccessRuleConfig::Modes(PathAccessModes {
                            read: Some(PermissionMode::Ask),
                            write: Some(PermissionMode::Ask),
                        })
                    });
                permission
                    .path
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(pattern.clone(), rule);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::PathRules,
                    Some(PermissionStudioSectionId::PathRules),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddNetworkRule { duplicate_from } => {
                let target = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-network-rules").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let mode = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .network
                            .as_ref()
                            .and_then(|network| network.rules.get(from.as_str()).copied())
                    })
                    .unwrap_or(PermissionMode::Ask);
                permission
                    .network
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(target.clone(), mode);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::NetworkRules,
                    Some(PermissionStudioSectionId::NetworkRules),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddToolTag { duplicate_from } => {
                let key = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-tool-tags").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let mode = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.tags.get(from.as_str()).copied())
                    })
                    .unwrap_or(PermissionMode::Ask);
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .tags
                    .insert(key.clone(), mode);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::ToolTags,
                    Some(PermissionStudioSectionId::ToolTags),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddToolName { duplicate_from } => {
                let key = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-tool-names").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let mode = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.names.get(from.as_str()).copied())
                    })
                    .unwrap_or(PermissionMode::Ask);
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .names
                    .insert(key.clone(), mode);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::ToolNames,
                    Some(PermissionStudioSectionId::ToolNames),
                    PermissionStudioFocus::Items,
                );
            }
            PermissionStudioEditorAction::AddToolRule { duplicate_from } => {
                let tool_name = parse_permission_studio_key_input(
                    &self.i18n,
                    ui_text::t(&self.i18n, "permission-studio-field-tool-rules").as_str(),
                    input.as_str(),
                )?;
                let mut permission = dialog.permission.clone();
                let rule = duplicate_from
                    .as_ref()
                    .and_then(|from| {
                        permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.rules.get(from.as_str()).cloned())
                    })
                    .unwrap_or(ToolPermissionRules::Mode(PermissionMode::Ask));
                permission
                    .tools
                    .get_or_insert_with(Default::default)
                    .rules
                    .insert(tool_name.clone(), rule);
                self.persist_permission_studio(dialog, permission)?;
                self.set_permission_studio_page_with_section(
                    dialog,
                    PermissionStudioPage::ToolCommandRules,
                    Some(PermissionStudioSectionId::ToolCommandRules),
                    PermissionStudioFocus::Items,
                );
            }
        }
        Ok(())
    }

    fn handle_settings_value_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SettingsValueEditOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(action, input) => {
                match parse_settings_field_input(&self.i18n, action, input.as_str()) {
                    Ok(Some(value)) => match self
                        .block_on_async(self.backend.set_config_setting(action.path, value))
                    {
                        Ok(_) => {
                            self.flash_success(settings_path_updated_message(
                                &self.i18n,
                                action.path,
                            ));
                            self.refresh_current_route_after_local_edit();
                            true
                        }
                        Err(error) => {
                            self.flash_error(error);
                            false
                        }
                    },
                    Ok(None) => {
                        match self.block_on_async(self.backend.delete_config_setting(action.path)) {
                            Ok(_) => {
                                self.flash_success(settings_path_cleared_message(
                                    &self.i18n,
                                    action.path,
                                ));
                                self.refresh_current_route_after_local_edit();
                                true
                            }
                            Err(error) => {
                                self.flash_error(error);
                                false
                            }
                        }
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            InputDialogKeyResult::Continue => false,
        }
    }

    fn handle_runtime_setting_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut RuntimeSettingEditOverlay,
    ) -> bool {
        match drive_input_dialog_key(dialog, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(action, input) => {
                match self.run_options.apply_runtime_setting_input(
                    &self.i18n,
                    action,
                    input.as_str(),
                ) {
                    Ok(message) => {
                        self.flash_success(message);
                        self.refresh_current_route_after_local_edit();
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            InputDialogKeyResult::Continue => false,
        }
    }

    fn handle_choice_overlay_key(&mut self, key: KeyEvent, dialog: &mut ChoiceOverlay) -> bool {
        match key.code {
            KeyCode::Tab => {
                if dialog.fill_input_from_selected() {
                    Self::sync_choice_overlay_input(dialog, true);
                }
                false
            }
            KeyCode::Enter => self.commit_choice_overlay(dialog),
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::sync_choice_overlay_input(dialog, true);
                    }
                    false
                }
            },
        }
    }

    fn handle_permission_rule_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionRuleEditOverlay,
    ) -> bool {
        match drive_input_dialog_key(&mut dialog.state, key) {
            InputDialogKeyResult::Close => true,
            InputDialogKeyResult::Submit(_, input) => {
                let draft = match parse_permission_rule_input(&self.i18n, input.as_str()) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        self.flash_warning(error);
                        return false;
                    }
                };
                let label = permission_rule_draft_label(&self.i18n, &draft);
                let params = permission_rule_params_from_draft(&draft);
                let result = match dialog.rule_id {
                    Some(rule_id) => {
                        self.block_on_async(self.backend.replace_permission_rule(rule_id, params))
                    }
                    None => self.block_on_async(self.backend.create_permission_rule(params)),
                };
                match result {
                    Ok(rule) => {
                        self.flash_success(self.i18n.text_args(
                            "flash-permission-rule-saved",
                            &crate::fl_args!("name" => permission_rule_label(&self.i18n, &rule)),
                        ));
                        if let Some(return_overlay) = dialog.return_overlay.take() {
                            self.overlay_stack.pop();
                            self.overlay = Some(*return_overlay);
                        } else {
                            self.refresh_permission_rules_route(dialog.return_query.as_str());
                        }
                        true
                    }
                    Err(error) => {
                        self.flash_error(format!("{}: {}", label, error));
                        false
                    }
                }
            }
            InputDialogKeyResult::Continue => false,
        }
    }

    fn handle_file_attach_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut FileAttachOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Tab => {
                dialog.fill_input_from_selected();
                false
            }
            KeyCode::Enter => {
                let Some(path) = dialog.selected_row().and_then(|selection| match selection {
                    SearchListRow::Clear(_) => None,
                    SearchListRow::Custom(value) => Some(PathBuf::from(value.raw)),
                    SearchListRow::Item(path) => Some(path),
                }) else {
                    return false;
                };
                match self.stage_attachment_from_path(path.as_path(), false) {
                    Ok(()) => true,
                    Err(error) => {
                        self.flash_error(error);
                        false
                    }
                }
            }
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        self.refresh_file_attach_overlay(dialog);
                    }
                    false
                }
            },
        }
    }

    fn handle_help_overlay_key(&mut self, key: KeyEvent, dialog: &mut HelpOverlay) -> bool {
        let max_scroll = ui_text::help_lines(&self.i18n)
            .len()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16;
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => true,
            _ => dialog.handle_navigation_key(key, max_scroll, 8),
        }
    }

    fn handle_session_search_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionSearchOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => {
                if dialog.loading || dialog.meta.page_index == 0 {
                    return false;
                }
                dialog.meta.page_index = dialog.meta.page_index.saturating_sub(1);
                dialog.selected = 0;
                match dialog.meta.mode {
                    SessionViewMode::Subtree => {
                        self.refresh_session_search_overlay_local(dialog);
                    }
                    SessionViewMode::All | SessionViewMode::Roots => {
                        let cursor = dialog
                            .meta
                            .cursors
                            .get(dialog.meta.page_index)
                            .cloned()
                            .flatten();
                        dialog.loading = true;
                        dialog.footer = self.session_search_footer(dialog);
                        self.request_session_search_page(
                            dialog.meta.mode,
                            dialog.input.text().trim().to_string(),
                            dialog.meta.page_index,
                            cursor,
                        );
                    }
                }
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if dialog.loading || !dialog.meta.has_more {
                    return false;
                }
                match dialog.meta.mode {
                    SessionViewMode::Subtree => {
                        dialog.meta.page_index = dialog.meta.page_index.saturating_add(1);
                        dialog.selected = 0;
                        self.refresh_session_search_overlay_local(dialog);
                    }
                    SessionViewMode::All | SessionViewMode::Roots => {
                        let Some(cursor) = dialog.meta.next_cursor.clone() else {
                            return false;
                        };
                        dialog.meta.page_index = dialog.meta.page_index.saturating_add(1);
                        if dialog.meta.cursors.len() <= dialog.meta.page_index {
                            dialog.meta.cursors.resize(dialog.meta.page_index + 1, None);
                        }
                        dialog.meta.cursors[dialog.meta.page_index] = Some(cursor.clone());
                        dialog.selected = 0;
                        dialog.loading = true;
                        dialog.footer = self.session_search_footer(dialog);
                        self.request_session_search_page(
                            dialog.meta.mode,
                            dialog.input.text().trim().to_string(),
                            dialog.meta.page_index,
                            Some(cursor),
                        );
                    }
                }
                false
            }
            KeyCode::Tab => {
                if let Some(session) = dialog.items.get(dialog.selected) {
                    let title = session.session.title.clone();
                    if dialog.input.text() != title {
                        dialog.input.set_text(title.clone());
                        self.reset_session_search_query(dialog, title);
                    }
                }
                false
            }
            KeyCode::Enter => {
                let Some(session) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                self.open_session(session.session.id, session.session.title);
                self.focus = Focus::Composer;
                true
            }
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        self.reset_session_search_query(
                            dialog,
                            dialog.input.text().trim().to_string(),
                        );
                    }
                    false
                }
            },
        }
    }

    fn reset_session_search_query(&mut self, dialog: &mut SessionSearchOverlay, query: String) {
        dialog.meta.page_index = 0;
        dialog.selected = 0;
        dialog.meta.offset = 0;
        dialog.meta.cursors.clear();
        dialog.meta.cursors.push(None);
        dialog.meta.next_cursor = None;
        dialog.meta.has_more = false;
        dialog.loading = true;
        dialog.footer = self.session_search_footer(dialog);
        dialog.meta.page_index = 0;
        match dialog.meta.mode {
            SessionViewMode::Subtree => {
                if let Some(session_id) = dialog.meta.scope_session_id {
                    self.request_session_search_subtree(session_id, query);
                }
            }
            SessionViewMode::All | SessionViewMode::Roots => {
                self.request_session_search_page(dialog.meta.mode, query, 0, None);
            }
        }
    }

    fn refresh_session_search_overlay_local(&self, dialog: &mut SessionSearchOverlay) {
        let query = dialog.input.text().trim();
        let filtered = dialog
            .meta
            .all_items
            .iter()
            .filter(|session| session.search_list_matches_query(query))
            .cloned()
            .collect::<Vec<_>>();
        let total = filtered.len();
        let page_limit = dialog.meta.page_limit.max(1);
        let max_page_index = total.saturating_sub(1) / page_limit;
        dialog.meta.page_index = min(dialog.meta.page_index, max_page_index);
        dialog.meta.offset = dialog.meta.page_index.saturating_mul(page_limit);
        dialog.items = filtered
            .into_iter()
            .skip(dialog.meta.offset)
            .take(page_limit)
            .collect();
        dialog.meta.has_more = dialog.meta.offset + dialog.items.len() < total;
        dialog.meta.next_cursor = None;
        dialog.clamp_selection();
        dialog.loading = false;
        dialog.footer = self.session_search_footer(dialog);
    }

    fn session_search_footer(&self, dialog: &SessionSearchOverlay) -> String {
        let scope = match dialog.meta.mode {
            SessionViewMode::All => ui_text::t(&self.i18n, "overlay-session-search-scope-all"),
            SessionViewMode::Roots => ui_text::t(&self.i18n, "overlay-session-search-scope-roots"),
            SessionViewMode::Subtree => {
                ui_text::t(&self.i18n, "overlay-session-search-scope-subtree")
            }
        };
        let start = if dialog.items.is_empty() {
            0
        } else {
            dialog.meta.offset.saturating_add(1)
        };
        let end = dialog.meta.offset.saturating_add(dialog.items.len());
        if dialog.meta.mode == SessionViewMode::Subtree {
            let total = dialog
                .meta
                .all_items
                .iter()
                .filter(|session| session.search_list_matches_query(dialog.input.text().trim()))
                .count();
            let page_total = if total == 0 {
                0
            } else {
                (total + dialog.meta.page_limit.saturating_sub(1)) / dialog.meta.page_limit.max(1)
            };
            return self.i18n.text_args(
                "overlay-session-search-footer-local",
                &crate::fl_args!(
                    "scope" => scope,
                    "start" => start as i64,
                    "end" => end as i64,
                    "total" => total as i64,
                    "page" => dialog.meta.page_index.saturating_add(1) as i64,
                    "pages" => page_total.max(1) as i64,
                ),
            );
        }

        let end_state = if dialog.meta.has_more {
            ui_text::t(&self.i18n, "overlay-session-search-tail-more")
        } else {
            ui_text::t(&self.i18n, "overlay-session-search-tail-end")
        };
        self.i18n.text_args(
            "overlay-session-search-footer-remote",
            &crate::fl_args!(
                "scope" => scope,
                "start" => start as i64,
                "end" => end as i64,
                "page" => dialog.meta.page_index.saturating_add(1) as i64,
                "tail" => end_state,
            ),
        )
    }

    fn handle_picker_overlay_key(&mut self, key: KeyEvent, dialog: &mut PickerOverlay) -> bool {
        match key.code {
            KeyCode::Tab => {
                if dialog.fill_input_from_selected() {
                    Self::refresh_picker_overlay(dialog);
                }
                false
            }
            KeyCode::Char('n') if matches!(dialog.meta.kind, PickerKind::Agents) => {
                self.open_agent_create_overlay();
                false
            }
            KeyCode::Char('n')
                if matches!(
                    dialog.meta.kind,
                    PickerKind::Providers(ProviderPickerPurpose::Configure)
                ) =>
            {
                self.route_stack.push(Route::Picker(dialog.clone()));
                self.open_provider_studio(None);
                false
            }
            KeyCode::Char('n') if matches!(dialog.meta.kind, PickerKind::PermissionRules) => {
                self.route_stack.push(Route::Picker(dialog.clone()));
                self.open_permission_rule_studio(None, None);
                false
            }
            KeyCode::Char('d') if matches!(dialog.meta.kind, PickerKind::PermissionRules) => {
                let Some(item) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                if let PickerValue::PermissionRule(rule) = item.value {
                    self.open_revoke_permission_rule_confirm(&rule, dialog.input.text());
                    false
                } else {
                    false
                }
            }
            KeyCode::Enter => {
                let Some(item) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                if matches!(dialog.meta.kind, PickerKind::Agents) {
                    match item.value {
                        PickerValue::AgentCreate => {
                            self.open_agent_create_overlay();
                            return false;
                        }
                        PickerValue::Agent(agent) => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_agent_studio(agent.name.as_str());
                            return false;
                        }
                        _ => {}
                    }
                }
                if matches!(
                    dialog.meta.kind,
                    PickerKind::Providers(ProviderPickerPurpose::Configure)
                ) {
                    match item.value {
                        PickerValue::ProviderCreate => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_provider_studio(None);
                            return false;
                        }
                        PickerValue::Provider(provider) => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_provider_studio(Some(provider.provider_id.as_str()));
                            return false;
                        }
                        _ => {}
                    }
                }
                if matches!(dialog.meta.kind, PickerKind::PermissionRules) {
                    match item.value {
                        PickerValue::PermissionRuleCreate => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_permission_rule_studio(None, None);
                            return false;
                        }
                        PickerValue::PermissionRule(rule) => {
                            self.route_stack.push(Route::Picker(dialog.clone()));
                            self.open_permission_rule_studio(Some(&rule), None);
                            return false;
                        }
                        _ => {}
                    }
                }
                self.handle_picker_selection(dialog.meta.kind.clone(), item);
                true
            }
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::refresh_picker_overlay(dialog);
                    }
                    false
                }
            },
        }
    }

    fn handle_session_model_chooser_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionModelChooserOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Left => {
                dialog.move_selection_page(-1, dialog.meta.page_size);
                false
            }
            KeyCode::Right => {
                dialog.move_selection_page(1, dialog.meta.page_size);
                false
            }
            KeyCode::Enter => {
                let Some(item) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                self.apply_model_override(item.model);
                true
            }
            _ => match dialog.handle_filter_input_key(key, dialog.meta.page_size) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::refresh_session_model_chooser_overlay(dialog, false, None);
                    }
                    false
                }
            },
        }
    }

    fn handle_timeline_overlay_key(&mut self, key: KeyEvent, dialog: &mut TimelineOverlay) -> bool {
        match key.code {
            KeyCode::Enter => {
                if let Some(item) = dialog.selected_item()
                    && let Some(message_id) = item.linked_message_id
                {
                    self.jump_to_message(message_id);
                    return true;
                }
                false
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(item) = dialog.selected_item() {
                    match set_clipboard_text(item.copy_text.as_str()) {
                        Ok(()) => self
                            .flash_success(ui_text::t(&self.i18n, "flash-timeline-event-copied")),
                        Err(error) => self.flash_error(self.i18n.text_args(
                            "flash-clipboard-copy-failed",
                            &crate::fl_args!("error" => error.to_string()),
                        )),
                    }
                }
                false
            }
            _ => match dialog.handle_filter_input_key(key, 10) {
                SearchInputKeyResult::Close => true,
                SearchInputKeyResult::Navigated => false,
                SearchInputKeyResult::Edited { changed } => {
                    if changed {
                        Self::refresh_timeline_overlay(dialog);
                    }
                    false
                }
            },
        }
    }

    fn handle_provider_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ProviderStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            match drive_editor_dialog_key(editor, key) {
                EditorDialogKeyResult::Continue => return false,
                EditorDialogKeyResult::Close => {
                    dialog.editor = None;
                    return false;
                }
                EditorDialogKeyResult::Submit(action, input) => match action {
                    ProviderStudioEditorAction::Field(field) => {
                        let value = input.trim().to_string();
                        if let Err(error) = self.commit_provider_studio_field(dialog, field, value)
                        {
                            self.flash_error(error);
                            return false;
                        }
                        dialog.editor = None;
                        return false;
                    }
                    ProviderStudioEditorAction::NewModel { adapter_id } => {
                        let value = input.trim().to_string();
                        match self.add_provider_studio_manual_model(dialog, adapter_id, value) {
                            Ok(()) => dialog.editor = None,
                            Err(error) => self.flash_error(error),
                        }
                        return false;
                    }
                    ProviderStudioEditorAction::ModelField(field) => {
                        let value = input.trim().to_string();
                        if let Err(error) =
                            self.commit_provider_studio_model_field(dialog, field, value)
                        {
                            self.flash_error(error);
                            return false;
                        }
                        dialog.editor = None;
                        return false;
                    }
                },
            }
        }

        if dialog.model_page.is_some() {
            return self.handle_provider_studio_model_page_key(key, dialog);
        }

        if dialog.detail_page.is_some() {
            return self.handle_provider_studio_detail_page_key(key, dialog);
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Tab => {
                dialog.selection.next_focus();
                false
            }
            KeyCode::BackTab => {
                dialog.selection.prev_focus();
                false
            }
            KeyCode::Char('n') => {
                self.load_provider_studio_draft(dialog, None, Some(String::new()));
                false
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                self.request_provider_studio_start_auth(dialog);
                false
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                self.request_provider_studio_continue_auth(dialog);
                false
            }
            KeyCode::Char('r') => {
                self.request_provider_studio_adapter_models(dialog);
                false
            }
            KeyCode::Char('+') if dialog.selection.focus() == ProviderStudioFocus::Models => {
                self.open_provider_studio_new_model_editor(dialog);
                false
            }
            KeyCode::Delete | KeyCode::Backspace
                if dialog.selection.focus() == ProviderStudioFocus::Adapters =>
            {
                self.open_provider_studio_delete_selected_adapter_confirm(dialog);
                false
            }
            KeyCode::Delete | KeyCode::Backspace
                if dialog.selection.focus() == ProviderStudioFocus::Models =>
            {
                self.open_provider_studio_delete_selected_model_confirm(dialog);
                false
            }
            KeyCode::Char('D') if dialog.draft.source_provider_id.is_some() => {
                if let Some(provider_id) = dialog.draft.source_provider_id.clone() {
                    self.open_provider_studio_delete_provider_confirm(provider_id);
                }
                false
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                dialog.saving = true;
                self.request_provider_studio_save_draft(dialog.clone());
                false
            }
            KeyCode::Char('a') => {
                if provider_studio_selected_adapter_models(dialog).is_none() {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-provider-studio-adapter-required",
                    ));
                    return false;
                }
                dialog.saving = true;
                self.request_provider_studio_save_selected_adapter(dialog.clone());
                false
            }
            KeyCode::Char('A') if dialog.selection.focus() == ProviderStudioFocus::Adapters => {
                Self::select_all_provider_studio_adapters(dialog);
                false
            }
            KeyCode::Char('A') if dialog.selection.focus() == ProviderStudioFocus::Models => {
                Self::select_all_provider_studio_models(dialog);
                false
            }
            KeyCode::Char('c') | KeyCode::Char('C')
                if dialog.selection.focus() == ProviderStudioFocus::Adapters =>
            {
                Self::clear_provider_studio_selected_adapters(dialog);
                false
            }
            KeyCode::Char('c') | KeyCode::Char('C')
                if dialog.selection.focus() == ProviderStudioFocus::Models =>
            {
                Self::clear_provider_studio_selected_models(dialog);
                false
            }
            KeyCode::Char('m') | KeyCode::Char('M') => {
                if provider_studio_selected_model_target(dialog).is_none() {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-provider-studio-model-required",
                    ));
                    return false;
                }
                dialog.saving = true;
                self.request_provider_studio_save_selected_model(dialog.clone());
                false
            }
            KeyCode::Char(' ') if dialog.selection.focus() == ProviderStudioFocus::Adapters => {
                self.toggle_provider_studio_selected_adapter(dialog);
                false
            }
            KeyCode::Char(' ') if dialog.selection.focus() == ProviderStudioFocus::Models => {
                self.toggle_provider_studio_selected_model(dialog);
                false
            }
            KeyCode::PageUp => {
                self.move_provider_studio_selection_page(dialog, -1, 10);
                false
            }
            KeyCode::PageDown => {
                self.move_provider_studio_selection_page(dialog, 1, 10);
                false
            }
            KeyCode::Home => {
                self.move_provider_studio_selection_home(dialog);
                false
            }
            KeyCode::End => {
                self.move_provider_studio_selection_end(dialog);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_provider_studio_selection(dialog, -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_provider_studio_selection(dialog, 1);
                false
            }
            KeyCode::Enter => {
                self.activate_provider_studio_focus(dialog);
                false
            }
            _ => false,
        }
    }

    fn handle_model_catalog_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ModelCatalogStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.workbench.editor.as_mut() {
            return match drive_input_dialog_key(editor, key) {
                InputDialogKeyResult::Close => {
                    dialog.workbench.editor = None;
                    false
                }
                InputDialogKeyResult::Submit(_, value) => {
                    dialog.query = value.trim().to_string();
                    dialog.offset = 0;
                    dialog.workbench.list.selected = 0;
                    dialog.loading = true;
                    dialog.workbench.editor = None;
                    self.request_model_catalog_page(dialog.query.clone(), 0);
                    false
                }
                InputDialogKeyResult::Continue => false,
            };
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('/') => {
                dialog.workbench.editor =
                    Some(self.build_model_catalog_search_overlay(dialog.query.as_str()));
                false
            }
            KeyCode::Char('R') => {
                dialog.loading = true;
                self.request_model_catalog_refresh();
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if dialog.offset == 0 {
                    return false;
                }
                let offset = dialog.offset.saturating_sub(dialog.limit.max(1));
                dialog.offset = offset;
                dialog.workbench.list.selected = 0;
                dialog.loading = true;
                self.request_model_catalog_page(dialog.query.clone(), offset);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if dialog.offset + dialog.workbench.list.items.len() >= dialog.total {
                    return false;
                }
                dialog.offset += dialog.limit.max(1);
                dialog.workbench.list.selected = 0;
                dialog.loading = true;
                self.request_model_catalog_page(dialog.query.clone(), dialog.offset);
                false
            }
            _ if dialog.workbench.list.handle_navigation_key(key, 10) => false,
            _ => false,
        }
    }

    fn handle_paste(&mut self, text: String) {
        let backend = self.backend.clone();
        let mut pending_session_search_request: Option<(SessionViewMode, Option<i64>, String)> =
            None;
        if self.overlay.is_none() {
            let mut handled_route = false;
            match &mut self.current_route {
                Route::Main => {}
                Route::Help(_) | Route::SettingsStudio(_) => {}
                Route::AgentStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::PermissionStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::PermissionRuleStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::SessionSearch(dialog) => {
                    let before = dialog.input.text().trim().to_string();
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    let after = dialog.input.text().trim().to_string();
                    if before != after {
                        dialog.meta.page_index = 0;
                        dialog.selected = 0;
                        dialog.meta.offset = 0;
                        dialog.meta.cursors.clear();
                        dialog.meta.cursors.push(None);
                        dialog.meta.next_cursor = None;
                        dialog.meta.has_more = false;
                        dialog.loading = true;
                        pending_session_search_request =
                            Some((dialog.meta.mode, dialog.meta.scope_session_id, after));
                    }
                    handled_route = true;
                }
                Route::Picker(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_picker_overlay(dialog);
                    handled_route = true;
                }
                Route::SessionModelChooser(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_session_model_chooser_overlay(dialog, false, None);
                    handled_route = true;
                }
                Route::Timeline(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_timeline_overlay(dialog);
                    handled_route = true;
                }
                Route::PluginPolicyStudio(_) => {
                    handled_route = true;
                }
                Route::PluginWorkbench(dialog) => {
                    Self::paste_plugin_workbench(dialog, text.as_str());
                    handled_route = true;
                }
                Route::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
            }
            if handled_route {
                if let Some((mode, scope_session_id, query)) = pending_session_search_request {
                    match mode {
                        SessionViewMode::Subtree => {
                            if let Some(session_id) = scope_session_id {
                                self.request_session_search_subtree(session_id, query);
                            }
                        }
                        SessionViewMode::All | SessionViewMode::Roots => {
                            self.request_session_search_page(mode, query, 0, None);
                        }
                    }
                }
                return;
            }
        }
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::TranscriptSearch(dialog)
                | Overlay::SessionRename(dialog)
                | Overlay::AgentCreate(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::SettingsValueEdit(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::RuntimeSettingEdit(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::Choice(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::sync_choice_overlay_input(dialog, true);
                }
                Overlay::PermissionRuleEdit(dialog) => {
                    dialog.state.input.flush_all_pending_input();
                    dialog.state.input.insert_str(text.as_str());
                }
                Overlay::FileAttach(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    dialog.items = backend
                        .search_workspace_files(dialog.input.text(), 24)
                        .unwrap_or_default();
                    dialog.clamp_selection();
                }
                Overlay::PathBrowser(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_path_browser_overlay_with_root(
                        self.backend.workspace_root(),
                        dialog,
                    );
                }
                Overlay::UserInputReply(dialog) => {
                    if dialog.state.screen() == QuestionFlowScreen::Review {
                        Self::focus_user_input_question(dialog, dialog.state.selected_question());
                    }
                    if !dialog.editing_custom && !Self::begin_user_input_custom_edit(dialog) {
                        return;
                    }
                    dialog.custom_input.flush_all_pending_input();
                    dialog.custom_input.insert_str(text.as_str());
                }
                Overlay::SessionSearch(dialog) => {
                    let before = dialog.input.text().trim().to_string();
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    let after = dialog.input.text().trim().to_string();
                    if before != after {
                        dialog.meta.page_index = 0;
                        dialog.selected = 0;
                        dialog.meta.offset = 0;
                        dialog.meta.cursors.clear();
                        dialog.meta.cursors.push(None);
                        dialog.meta.next_cursor = None;
                        dialog.meta.has_more = false;
                        dialog.loading = true;
                        pending_session_search_request =
                            Some((dialog.meta.mode, dialog.meta.scope_session_id, after));
                    }
                }
                Overlay::Picker(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_picker_overlay(dialog);
                }
                Overlay::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::Timeline(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_timeline_overlay(dialog);
                }
                Overlay::Confirm(_) => {}
                Overlay::Permission(_) => {}
            }
            if let Some((mode, scope_session_id, query)) = pending_session_search_request {
                match mode {
                    SessionViewMode::Subtree => {
                        if let Some(session_id) = scope_session_id {
                            self.request_session_search_subtree(session_id, query);
                        }
                    }
                    SessionViewMode::All | SessionViewMode::Roots => {
                        self.request_session_search_page(mode, query, 0, None);
                    }
                }
            }
            return;
        }

        if self.focus == Focus::Composer {
            self.reset_prompt_history_recall();
            self.composer.flush_all_pending_input();
            if self.try_stage_pasted_path(text.as_str()) {
                return;
            }

            let char_count = text.chars().count();
            if char_count > LARGE_PASTE_CHAR_THRESHOLD {
                self.stage_large_paste(text);
            } else {
                self.composer.insert_str(text.as_str());
            }
            self.after_composer_text_mutated();
        }
    }

    fn handle_sessions_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('1') => {
                self.set_session_view_mode(SessionViewMode::All);
            }
            KeyCode::Char('2') => {
                self.set_session_view_mode(SessionViewMode::Roots);
            }
            KeyCode::Char('3') => {
                self.set_session_view_mode(SessionViewMode::Subtree);
            }
            KeyCode::Char('m') => {
                self.cycle_session_view_mode();
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.sessions.move_selection(-1);
                self.maybe_request_more_sessions();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.sessions.move_selection(1);
                self.maybe_request_more_sessions();
            }
            KeyCode::PageUp => {
                self.sessions.move_selection(-10);
            }
            KeyCode::PageDown => {
                self.sessions.move_selection(10);
                self.maybe_request_more_sessions();
            }
            KeyCode::Enter => {
                if let Some(session) = self.sessions.current_selected() {
                    self.open_session(session.id, session.title.clone());
                    self.focus = Focus::Transcript;
                }
            }
            KeyCode::Home => self.sessions.list.move_selection_home(),
            KeyCode::End => {
                if !self.sessions.list.items.is_empty() {
                    self.sessions.list.move_selection_end();
                    self.maybe_request_more_sessions();
                }
            }
            _ => {}
        }
    }

    fn handle_transcript_key(&mut self, key: KeyEvent) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        if matches!(key.code, KeyCode::Char('1'..='9'))
            || matches!(key.code, KeyCode::Char('0')) && self.transcript_motion_prefix.is_some()
        {
            return;
        } else if matches!(key.code, KeyCode::Char('i')) {
            self.enter_insert_mode();
        } else if matches!(key.code, KeyCode::Char('y')) {
            self.copy_transcript_cursor_node();
        } else if matches!(key.code, KeyCode::Char('Y')) {
            self.copy_visible_transcript();
        } else if matches!(key.code, KeyCode::Char('C')) {
            self.copy_loaded_transcript();
        } else if matches!(key.code, KeyCode::Char('c')) {
            self.copy_last_assistant_message();
        } else if matches!(
            key.code,
            KeyCode::Enter | KeyCode::Char('o') | KeyCode::Char(' ')
        ) {
            self.toggle_transcript_cursor_node();
        } else if matches!(key.code, KeyCode::Char('h')) {
            let count = self.transcript_motion_count();
            self.transcript
                .move_by_blocks(width, height, TranscriptMoveDirection::Up, count);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Char('l')) {
            let count = self.transcript_motion_count();
            self.transcript
                .move_by_blocks(width, height, TranscriptMoveDirection::Down, count);
        } else if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
            let count = self.transcript_motion_count();
            self.transcript.scroll_by_lines_with_blocks(
                width,
                height,
                TranscriptMoveDirection::Up,
                count,
            );
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Down | KeyCode::Char('j')) {
            let count = self.transcript_motion_count();
            self.transcript.scroll_by_lines_with_blocks(
                width,
                height,
                TranscriptMoveDirection::Down,
                count,
            );
        } else if matches!(key.code, KeyCode::PageUp)
            || matches!(key.code, KeyCode::Char('b'))
                && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::PageDown)
            || matches!(key.code, KeyCode::Char('f'))
                && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_page(width, height, true);
        } else if matches!(key.code, KeyCode::Char(' '))
            && key.modifiers.contains(KeyModifiers::SHIFT)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Char(' ')) {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_page(width, height, true);
        } else if matches!(key.code, KeyCode::Char('u'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_half_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Char('d'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_by_half_page(width, height, true);
        } else if matches!(key.code, KeyCode::Home | KeyCode::Char('g')) {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_to_top(width, height);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::End | KeyCode::Char('G')) {
            self.transcript_motion_prefix = None;
            self.transcript.scroll_to_bottom(width, height);
        } else {
            self.transcript_motion_prefix = None;
        }
    }

    fn enter_insert_mode(&mut self) {
        self.focus = Focus::Composer;
    }

    fn toggle_transcript_cursor_node(&mut self) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        if !node.toggleable {
            return;
        }
        self.transcript
            .node_expansions
            .insert(node.key, !node.expanded);
        self.transcript.invalidate_render();
        self.transcript.clamp_scroll(width, height);
        if node.toggleable {
            self.flash_info(self.i18n.text_args(
                if node.expanded {
                    "flash-transcript-node-collapsed"
                } else {
                    "flash-transcript-node-expanded"
                },
                &crate::fl_args!("kind" => transcript_node_kind_label(&self.i18n, node.kind)),
            ));
        }
    }

    fn copy_transcript_cursor_node(&mut self) {
        let width = self.layout.transcript_body.width;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        match set_clipboard_text(node.copy_text.as_str()) {
            Ok(()) => self.flash_success(self.i18n.text_args(
                "flash-transcript-node-copied",
                &crate::fl_args!("kind" => transcript_node_kind_label(&self.i18n, node.kind)),
            )),
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-clipboard-copy-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
    }

    fn handle_composer_key(&mut self, key: KeyEvent) {
        if self.handle_prompt_history_search_key(key) {
            return;
        }
        if self.handle_file_mention_suggestion_key(key) {
            return;
        }
        if self.handle_slash_command_suggestion_key(key) {
            return;
        }
        if self.handle_selected_composer_item_key(key) {
            return;
        }
        // Esc handling is special — double-tap clears the input. We track
        // it before consulting the configurable bindings.
        if matches!(key.code, KeyCode::Esc) && key.modifiers.is_empty() {
            self.focus = Focus::Transcript;
            self.sync_composer_suggestions();
            return;
        }
        if matches!(key.code, KeyCode::Up) && key.modifiers.contains(KeyModifiers::ALT) {
            self.recall_prompt_history(PromptHistoryDirection::Older);
            return;
        }
        if matches!(key.code, KeyCode::Down) && key.modifiers.contains(KeyModifiers::ALT) {
            self.recall_prompt_history(PromptHistoryDirection::Newer);
            return;
        }
        // Configurable bindings define the composer map. The defaults preserve
        // the user's stated preference:
        // Enter = queue, Ctrl+Enter = submit, Shift+Enter / Ctrl+J = newline.
        if let Some(action) = self.keybindings.match_action(&key) {
            match action {
                ComposerAction::Submit => {
                    self.submit_or_steer();
                    return;
                }
                ComposerAction::Queue => {
                    self.queue_or_submit();
                    return;
                }
                ComposerAction::Newline => {
                    self.reset_prompt_history_recall();
                    self.composer.insert_explicit_newline();
                    self.after_composer_text_mutated();
                    return;
                }
                ComposerAction::EditQueue => {
                    if self.try_pop_queue_into_editor() {
                        self.reset_prompt_history_recall();
                        self.after_composer_text_mutated();
                        return;
                    }
                    // Fall through to normal cursor-up behavior when queue
                    // is empty.
                }
                ComposerAction::HistorySearch => {
                    self.open_prompt_history_search();
                    return;
                }
                ComposerAction::ClearInput => {
                    self.reset_prompt_history_recall();
                    self.clear_composer_state();
                    return;
                }
                ComposerAction::FocusItems => {
                    if self.toggle_composer_item_selection() {
                        return;
                    }
                }
                ComposerAction::AttachFile => {
                    self.reset_prompt_history_recall();
                    self.open_file_attach_overlay();
                    return;
                }
                ComposerAction::ExternalEditor => {
                    self.reset_prompt_history_recall();
                    self.composer.flush_all_pending_input();
                    self.pending_ui_action = Some(UiAction::EditComposerExternally);
                    return;
                }
                ComposerAction::AttachClipboardImage => {
                    self.reset_prompt_history_recall();
                    self.pending_ui_action = Some(UiAction::AttachClipboardImage);
                    return;
                }
                ComposerAction::OpenPendingUserInput => {
                    self.open_user_input_overlay();
                    return;
                }
                ComposerAction::OpenPendingPermission => {
                    self.open_permission_overlay();
                    return;
                }
            }
        }
        self.reset_prompt_history_recall();
        self.composer.handle_multiline_input_key(key);
        self.after_composer_text_mutated();
    }

    fn after_composer_text_mutated(&mut self) {
        self.sync_composer_items_with_editor();
        self.clamp_selected_composer_item();
        self.sync_composer_suggestions();
    }

    fn sync_composer_suggestions(&mut self) {
        if self.prompt_history_search.is_some() {
            self.slash_command_suggestions = None;
            self.file_mention_suggestions = None;
            return;
        }
        self.sync_file_mention_suggestions();
        self.sync_slash_command_suggestions();
    }

    fn toggle_composer_item_selection(&mut self) -> bool {
        if self.composer_items.is_empty() {
            self.selected_composer_item = None;
            return false;
        }
        self.selected_composer_item = match self.selected_composer_item {
            Some(_) => None,
            None => Some(0),
        };
        true
    }

    fn clamp_selected_composer_item(&mut self) {
        self.selected_composer_item = self
            .selected_composer_item
            .and_then(|index| (!self.composer_items.is_empty()).then_some(index))
            .map(|index| min(index, self.composer_items.len().saturating_sub(1)));
    }

    fn handle_selected_composer_item_key(&mut self, key: KeyEvent) -> bool {
        let Some(index) = self.selected_composer_item else {
            return false;
        };
        match key {
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.selected_composer_item = None;
                true
            }
            KeyEvent {
                code: KeyCode::BackTab,
                ..
            }
            | KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.selected_composer_item = Some(index.saturating_sub(1));
                true
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('l'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.selected_composer_item =
                    Some(min(index + 1, self.composer_items.len().saturating_sub(1)));
                true
            }
            KeyEvent {
                code: KeyCode::Delete | KeyCode::Backspace,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.remove_composer_item(index);
                true
            }
            KeyEvent {
                code: KeyCode::Enter,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('o'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.open_selected_composer_item(index);
                true
            }
            _ => false,
        }
    }

    fn remove_composer_item(&mut self, index: usize) {
        let Some(range) = self.composer.draft_elements().get(index).cloned() else {
            return;
        };
        self.composer.remove_range(range.start, range.end);
        self.after_composer_text_mutated();
    }

    fn open_selected_composer_item(&mut self, index: usize) {
        let Some(item) = self.composer_items.get(index) else {
            return;
        };
        match item {
            ComposerItem::Attachment(attachment) => {
                self.pending_ui_action = Some(UiAction::OpenPath {
                    path: attachment.path.clone(),
                });
            }
            ComposerItem::LargePaste(_) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-large-paste-no-file-view"));
            }
        }
    }

    fn handle_prompt_history_search_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut search) = self.prompt_history_search.take() else {
            return false;
        };
        let close = match key {
            KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.replace_composer_draft(search.meta.original.clone());
                true
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if let Some(result) = search.selected_item().cloned() {
                    self.replace_composer_draft(ComposerDraft {
                        text: result.text,
                        ..ComposerDraft::default()
                    });
                }
                true
            }
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                move_selected_index(&mut search.selected, search.items.len(), 1);
                false
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                move_selected_index(&mut search.selected, search.items.len(), -1);
                false
            }
            _ => {
                search.query.handle_line_input_key(key);
                Self::refresh_prompt_history_search(&self.prompt_history, &mut search);
                false
            }
        };

        if !close {
            self.prompt_history_search = Some(search);
        }
        true
    }

    fn open_prompt_history_search(&mut self) {
        self.composer.flush_all_pending_input();
        self.after_composer_text_mutated();
        let mut search = PromptHistorySearchState::new(
            Editor::default(),
            0,
            PromptHistorySearchMeta {
                original: self.current_composer_draft(),
            },
        );
        Self::refresh_prompt_history_search(&self.prompt_history, &mut search);
        self.slash_command_suggestions = None;
        self.file_mention_suggestions = None;
        self.selected_composer_item = None;
        self.prompt_history_search = Some(search);
    }

    fn refresh_prompt_history_search(
        prompt_history: &PromptHistory,
        search: &mut PromptHistorySearchState,
    ) {
        search.query.flush_all_pending_input();
        let query = search.query.text().trim().to_ascii_lowercase();
        search.items = prompt_history
            .items
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, entry)| query.is_empty() || entry.to_ascii_lowercase().contains(&query))
            .take(MAX_PROMPT_HISTORY_SEARCH_RESULTS)
            .map(|(history_index, text)| PromptHistorySearchResult {
                history_index,
                text: text.clone(),
            })
            .collect();
        search.clamp_selection();
    }

    fn handle_file_mention_suggestion_key(&mut self, key: KeyEvent) -> bool {
        if self.file_mention_suggestions.is_none() {
            return false;
        }

        match key {
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_file_mention_suggestion(-1);
                true
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_file_mention_suggestion(1);
                true
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.dismiss_file_mention_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            }
            | KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.complete_selected_file_mention();
                true
            }
            _ => false,
        }
    }

    fn move_file_mention_suggestion(&mut self, delta: isize) {
        let Some(state) = self.file_mention_suggestions.as_mut() else {
            return;
        };
        state.move_selection_cycle(delta);
    }

    fn dismiss_file_mention_suggestions(&mut self) {
        if let Some(state) = self.file_mention_suggestions.take() {
            self.dismissed_file_mention_suggestions_for = Some(state.fingerprint);
        }
    }

    fn complete_selected_file_mention(&mut self) {
        let Some(state) = self.file_mention_suggestions.clone() else {
            return;
        };
        let Some(item) = state.items.get(state.selected).cloned() else {
            return;
        };

        self.file_mention_suggestions = None;
        self.dismissed_file_mention_suggestions_for = None;
        self.composer
            .remove_range(state.meta.mention_range.start, state.meta.mention_range.end);
        if let Err(error) = self.stage_attachment_from_path(item.path.as_path(), false) {
            self.flash_error(error);
            return;
        }
        let after_cursor_is_space = self.composer.text()[self.composer.cursor()..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace);
        if !after_cursor_is_space {
            self.composer.insert_char(' ');
        }
        self.after_composer_text_mutated();
    }

    fn sync_file_mention_suggestions(&mut self) {
        let Some(context) = self.file_mention_suggestion_context() else {
            self.file_mention_suggestions = None;
            return;
        };
        if self.dismissed_file_mention_suggestions_for.as_deref()
            == Some(context.fingerprint.as_str())
        {
            self.file_mention_suggestions = None;
            return;
        }

        let items = self.file_mention_suggestion_items(context.query.as_str());
        if items.is_empty() {
            self.file_mention_suggestions = None;
            return;
        }

        let selected = self
            .file_mention_suggestions
            .as_ref()
            .filter(|state| state.query == context.query)
            .map(|state| min(state.selected, items.len().saturating_sub(1)))
            .unwrap_or(0);
        self.file_mention_suggestions = Some(FileMentionSuggestionState::new(
            context.query,
            context.fingerprint,
            items,
            selected,
            FileMentionSuggestionMeta {
                mention_range: context.mention_range,
            },
        ));
    }

    fn file_mention_suggestion_context(&self) -> Option<FileMentionSuggestionContext> {
        if self.focus != Focus::Composer || self.overlay.is_some() || !self.current_route_is_main()
        {
            return None;
        }
        if self.prompt_history_search.is_some() {
            return None;
        }
        file_mention_suggestion_context_for_text(self.composer.text(), self.composer.cursor())
    }

    fn file_mention_suggestion_items(&self, query: &str) -> Vec<FileMentionSuggestionItem> {
        self.backend
            .search_workspace_files(query, MAX_FILE_MENTION_SUGGESTIONS)
            .unwrap_or_default()
            .into_iter()
            .map(|path| {
                let label = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| path.display().to_string());
                FileMentionSuggestionItem {
                    detail: path.display().to_string(),
                    label,
                    path,
                }
            })
            .collect()
    }

    fn handle_slash_command_suggestion_key(&mut self, key: KeyEvent) -> bool {
        if self.slash_command_suggestions.is_none() {
            return false;
        }

        match key {
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_slash_command_suggestion(-1);
                true
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.move_slash_command_suggestion(1);
                true
            }
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.dismiss_slash_command_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                self.complete_selected_slash_command_suggestion(false);
                true
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.complete_selected_slash_command_suggestion(true);
                true
            }
            _ => false,
        }
    }

    fn move_slash_command_suggestion(&mut self, delta: isize) {
        let Some(state) = self.slash_command_suggestions.as_mut() else {
            return;
        };
        state.move_selection_cycle(delta);
    }

    fn dismiss_slash_command_suggestions(&mut self) {
        if let Some(state) = self.slash_command_suggestions.take() {
            self.dismissed_slash_command_suggestions_for = Some(state.fingerprint);
        }
    }

    fn complete_selected_slash_command_suggestion(&mut self, submit: bool) {
        let Some(item) = self.selected_slash_command_suggestion().cloned() else {
            return;
        };

        self.apply_slash_command_completion(&item);
        if submit {
            self.submit_composer();
        } else {
            self.sync_composer_suggestions();
        }
    }

    fn selected_slash_command_suggestion(&self) -> Option<&SlashCommandSuggestionItem> {
        let state = self.slash_command_suggestions.as_ref()?;
        state.selected_item()
    }

    fn apply_slash_command_completion(&mut self, item: &SlashCommandSuggestionItem) {
        let Some(context) = self.slash_command_suggestion_context() else {
            return;
        };

        let name = match &item.value {
            SlashCommandSuggestionValue::Command(spec) => spec.name,
            SlashCommandSuggestionValue::RuntimeTool(name) => name.as_str(),
        };
        let replacement = format!("/{name}");
        self.slash_command_suggestions = None;
        self.dismissed_slash_command_suggestions_for = None;

        self.composer
            .remove_range(context.name_range.start, context.name_range.end);
        self.composer
            .insert_str_at(context.name_range.start, replacement.as_str());

        let after_cursor_is_space = self.composer.text()[self.composer.cursor()..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace);
        if !after_cursor_is_space {
            self.composer.insert_char(' ');
        }
        self.after_composer_text_mutated();
    }

    fn sync_slash_command_suggestions(&mut self) {
        let Some(context) = self.slash_command_suggestion_context() else {
            self.slash_command_suggestions = None;
            return;
        };
        if self.dismissed_slash_command_suggestions_for.as_deref()
            == Some(context.fingerprint.as_str())
        {
            self.slash_command_suggestions = None;
            return;
        }

        let items = self.slash_command_suggestion_items(context.query.as_str());
        if items.is_empty() {
            self.slash_command_suggestions = None;
            return;
        }

        let selected = self
            .slash_command_suggestions
            .as_ref()
            .filter(|state| state.query == context.query)
            .map(|state| min(state.selected, items.len().saturating_sub(1)))
            .unwrap_or(0);
        self.slash_command_suggestions = Some(SlashCommandSuggestionState::new(
            context.query,
            context.fingerprint,
            items,
            selected,
            SlashCommandSuggestionMeta,
        ));
    }

    fn slash_command_suggestion_context(&self) -> Option<SlashCommandSuggestionContext> {
        if self.focus != Focus::Composer || self.overlay.is_some() || !self.current_route_is_main()
        {
            return None;
        }

        let context = slash_command_suggestion_context_for_text(
            self.composer.text(),
            self.composer.cursor(),
        )?;
        if !context.query.is_empty()
            && self
                .slash_command_suggestion_items(context.query.as_str())
                .is_empty()
        {
            return None;
        }
        Some(context)
    }

    fn slash_command_suggestion_items(&self, query: &str) -> Vec<SlashCommandSuggestionItem> {
        let query = query.trim().to_ascii_lowercase();
        let mut items = commands::command_suggestions_for_prefix(query.as_str())
            .into_iter()
            .map(|spec| SlashCommandSuggestionItem {
                label: format!("/{}", spec.name),
                detail: ui_text::t(&self.i18n, spec.summary_key),
                value: SlashCommandSuggestionValue::Command(spec),
            })
            .collect::<Vec<_>>();

        items.extend(
            self.runtime_tool_command_rows()
                .into_iter()
                .filter(|entry| runtime_tool_matches_slash_query(entry.label.as_str(), &query))
                .map(|entry| {
                    let label = entry.label;
                    SlashCommandSuggestionItem {
                        label: format!("/{label}"),
                        detail: entry.detail,
                        value: SlashCommandSuggestionValue::RuntimeTool(label),
                    }
                }),
        );
        items
    }

    /// UP / EditQueue binding: pull every editable queued message back into
    /// the editor for editing. Returns true if anything was pulled (so the
    /// caller skips the default cursor-up behavior).
    fn try_pop_queue_into_editor(&mut self) -> bool {
        // Only pull the queue when the cursor is at the top line of the
        // editor — otherwise UP is a normal cursor movement.
        if !self.composer.cursor_on_first_line() {
            return false;
        }
        let Some(combined) = self.queue.pop_all_editable() else {
            return false;
        };
        // Merge the queued draft on top of whatever's already in the
        // editor.
        let mut existing = self.take_composer_draft();
        if !existing.text.is_empty() && !existing.text.ends_with('\n') {
            existing.text.push_str("\n\n");
        }
        let prev_len = existing.text.len();
        existing.text.push_str(combined.text.as_str());
        for mut element in combined.elements {
            element.range = (element.range.start + prev_len)..(element.range.end + prev_len);
            existing.elements.push(element);
        }
        existing.items.extend(combined.items);
        self.restore_composer_draft(existing);
        true
    }

    fn handle_message(&mut self, message: AppMessage) {
        match message {
            AppMessage::SessionsLoaded {
                scope,
                subtree_root_id,
                result,
            } => self.handle_sessions_loaded(scope, subtree_root_id, result),
            AppMessage::SessionCreated {
                submit_draft,
                result,
            } => self.handle_session_created(submit_draft, result),
            AppMessage::SessionStateLoaded { session_id, result } => {
                self.handle_session_state_loaded(session_id, result)
            }
            AppMessage::MessagesLoaded {
                session_id,
                mode,
                result,
            } => self.handle_messages_loaded(session_id, mode, result),
            AppMessage::SessionRefreshed { session_id, result } => {
                self.handle_session_refreshed(session_id, result)
            }
            AppMessage::SessionMessageSubmitted {
                session_id,
                draft,
                result,
            } => self.handle_session_turn_submitted(session_id, draft, result),
            AppMessage::SessionContinued { session_id, result } => {
                self.handle_session_continued(session_id, result)
            }
            AppMessage::SessionCompacted { session_id, result } => {
                self.handle_session_continued(session_id, result)
            }
            AppMessage::SessionRenamed { session_id, result } => {
                self.handle_session_renamed(session_id, result)
            }
            AppMessage::PermissionReplied {
                session_id,
                label,
                result,
            } => self.handle_permission_replied(session_id, label, result),
            AppMessage::UserInputReplied { session_id, result } => {
                self.handle_user_input_replied(session_id, result)
            }
            AppMessage::SessionSearchPageLoaded {
                mode,
                query,
                page_index,
                result,
            } => self.handle_session_search_page_loaded(mode, query, page_index, result),
            AppMessage::SessionSearchSubtreeLoaded {
                session_id,
                query,
                result,
            } => self.handle_session_search_subtree_loaded(session_id, query, result),
            AppMessage::LineageLoaded { session_id, result } => {
                self.handle_lineage_loaded(session_id, result)
            }
            AppMessage::RewindMessagesLoaded { session_id, result } => {
                self.handle_rewind_messages_loaded(session_id, result)
            }
            AppMessage::ProvidersLoaded { purpose, result } => {
                self.handle_providers_loaded(purpose, result)
            }
            AppMessage::AgentsLoaded { result } => self.handle_agents_loaded(result),
            AppMessage::ModelCatalogLoaded {
                query,
                offset,
                result,
            } => self.handle_model_catalog_loaded(query, offset, result),
            AppMessage::ProviderStudioAdapterModelsLoaded {
                request_key,
                result,
            } => self.handle_provider_studio_adapter_models_loaded(request_key, result),
            AppMessage::ProviderStudioAuthCompleted {
                request_key,
                result,
            } => self.handle_provider_studio_auth_completed(request_key, result),
            AppMessage::ProviderStudioSaved {
                provider_id,
                result,
            } => self.handle_provider_studio_saved(provider_id, result),
            AppMessage::ModelCatalogRefreshed { result } => {
                self.handle_model_catalog_refreshed(result)
            }
            AppMessage::ChildSessionsLoaded {
                parent_session_id,
                result,
            } => self.handle_child_sessions_loaded(parent_session_id, result),
            AppMessage::TimelineLoaded { session_id, result } => {
                self.handle_timeline_loaded(session_id, result)
            }
            AppMessage::SessionRewound {
                session_id,
                target,
                result,
            } => self.handle_session_rewound(session_id, target, result),
            AppMessage::SessionEventArrived { session_id, live } => {
                self.handle_session_event_arrived(session_id, live)
            }
            AppMessage::SteerSubmitted {
                session_id,
                draft,
                result,
            } => self.handle_steer_submitted(session_id, draft, result),
            AppMessage::RunCancelled { session_id, result } => {
                self.handle_turn_cancelled(session_id, result)
            }
            AppMessage::StatusLineUpdated { output } => self.handle_status_line_updated(output),
        }
    }

    fn handle_status_line_updated(&mut self, output: Option<String>) {
        if let Some(status_line) = self.status_line.as_mut() {
            status_line.running = false;
            status_line.text = output;
        }
    }

    fn handle_sessions_loaded(
        &mut self,
        scope: SessionLoadScope,
        subtree_root_id: Option<i64>,
        result: UiResult<Vec<SessionResource>>,
    ) {
        if self.sessions.pending_scope.as_ref() != Some(&scope) {
            return;
        }

        self.sessions.pending_scope = None;
        self.sessions.loading = false;
        self.sessions.loading_more = false;

        let selected_id = self
            .sessions
            .current_selected_id()
            .or(self.transcript.session_id)
            .or(self.launch.initial_session_id);

        match result {
            Ok(items) => {
                self.sessions.source_items = items;
                self.sessions.subtree_root_id = subtree_root_id;
                self.sessions.initialized = true;
                self.rebuild_visible_sessions(selected_id);

                if let Some(id) = selected_id {
                    let _ = self.sessions.select_by_id(id);
                }

                if self.transcript.session_id.is_none()
                    && self.launch.initial_session_id.is_none()
                    && let Some(session) = self.sessions.current_selected().cloned()
                {
                    self.open_session(session.id, session.title);
                }
            }
            Err(error) => {
                self.flash_error(error);
            }
        }
    }

    fn handle_session_created(
        &mut self,
        submit_draft: Option<ComposerDraft>,
        result: UiResult<SessionResource>,
    ) {
        match result {
            Ok(session) => {
                self.request_sessions(false);
                if submit_draft.is_some() {
                    self.clear_draft_for_slot(DraftSlot::NewSession);
                }
                self.open_session(session.id, session.title.clone());
                self.focus = Focus::Composer;

                if let Some(draft) = submit_draft {
                    self.request_submit_message(session.id, draft);
                } else {
                    self.flash_success(self.i18n.text_args(
                        "flash-created-session",
                        &crate::fl_args!("title" => session.title.clone()),
                    ));
                }
            }
            Err(error) => {
                self.transcript.submitting = false;
                if let Some(draft) =
                    submit_draft.or_else(|| self.transcript.pending_restore_draft.take())
                {
                    self.transcript.pending_restore_draft = None;
                    self.restore_composer_draft(draft);
                }
                self.flash_error(error);
            }
        }
    }

    fn handle_session_state_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }

        self.transcript.state_loading = false;
        match result {
            Ok(execution) => {
                let session_id = execution.session.id;
                if self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn handle_messages_loaded(
        &mut self,
        session_id: i64,
        mode: MessageLoadMode,
        result: UiResult<PaginatedResponse<MessageResource>>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }

        match mode {
            MessageLoadMode::Replace => self.transcript.loading_initial = false,
            MessageLoadMode::Prepend => self.transcript.loading_older = false,
        }

        match result {
            Ok(page) => match mode {
                MessageLoadMode::Replace => {
                    self.transcript.replace_messages(
                        page,
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
                MessageLoadMode::Prepend => {
                    self.transcript.prepend_messages(
                        page,
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
            },
            Err(error) => self.flash_error(error),
        }
    }

    fn handle_session_refreshed(&mut self, session_id: i64, result: UiResult<SessionRefresh>) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }

        self.transcript.refreshing = false;

        match result {
            Ok(refresh) => {
                if execution_update_is_stale(
                    self.transcript.last_event_seq,
                    refresh.latest_event_seq,
                ) {
                    return;
                }
                if let Some(execution) = refresh.execution {
                    let session_id = execution.session.id;
                    if self.apply_transcript_execution(execution) {
                        self.sync_pending_interactive_after_execution(session_id);
                        self.sync_session_list_selection_to_current_execution();
                    }
                }
                if let Some(page) = refresh.latest_messages {
                    self.transcript.merge_latest_messages(
                        page,
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
                if refresh.event_count > 0 {
                    self.sync_session_list_selection_to_current_execution();
                }
                if refresh.latest_event_seq.is_some() {
                    self.transcript.last_event_seq = refresh.latest_event_seq;
                }
                self.maybe_request_older_messages();
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn handle_session_turn_submitted(
        &mut self,
        session_id: i64,
        draft: ComposerDraft,
        result: UiResult<SessionExecutionResource>,
    ) {
        if self.transcript.session_id == Some(session_id) {
            self.transcript.submitting = false;
        }
        self.submitting_session_ids.remove(&session_id);
        match result {
            Ok(execution) => {
                self.transcript.pending_restore_draft = None;
                self.clear_draft_for_slot(DraftSlot::Session(session_id));
                cleanup_temporary_composer_items(draft.items.as_slice());
                if self.transcript.session_id != Some(session_id) {
                    self.open_session(session_id, execution.session.title.clone());
                }
                if self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
                self.request_refresh(session_id, true);
                self.request_sessions(false);
                // Pop the next pending message and submit it after the run.
                self.try_drain_queue_one();
            }
            Err(error) => {
                self.transcript.pending_restore_draft = None;
                if self.transcript.session_id == Some(session_id) {
                    self.restore_composer_draft(draft);
                }
                self.flash_error(error);
                // Pause draining: a failed run typically means the user
                // wants to inspect the error rather than fire the next
                // queued message blindly. They can press Up to recover
                // the queue contents.
            }
        }
    }

    /// Pop one editable message from the queue and submit it. Called
    /// whenever an in-flight run completes successfully so the user sees
    /// their pending messages run automatically.
    fn try_drain_queue_one(&mut self) {
        if self.transcript.submitting || self.current_session_pending_interactive_kind().is_some() {
            return;
        }
        let Some(msg) = self.queue.pop_next() else {
            return;
        };
        // Reuse the normal submit path. We stash it into the editor
        // first so any error path can put the text back in front of the
        // user.
        self.restore_composer_draft(msg.draft);
        self.submit_composer();
    }

    fn handle_steer_submitted(
        &mut self,
        _session_id: i64,
        draft: ComposerDraft,
        result: UiResult<()>,
    ) {
        match result {
            Ok(()) => {}
            Err(error) => {
                // Backend rejected the steer (run no longer steerable).
                // Don't drop the user's message — push it onto the front
                // of the queue so it goes out at the next run boundary.
                self.queue.push(QueuedMessage {
                    draft,
                    priority: QueuePriority::Now,
                    editable: true,
                });
                self.flash_warning(format!(
                    "{}: {}",
                    ui_text::t(&self.i18n, "flash-steer-failed-fallback-queue"),
                    error
                ));
            }
        }
    }

    fn handle_turn_cancelled(&mut self, session_id: i64, result: UiResult<()>) {
        if self.transcript.session_id == Some(session_id) {
            self.transcript.submitting = false;
        }
        self.submitting_session_ids.remove(&session_id);
        if self.transcript.session_id == Some(session_id) {
            self.request_refresh(session_id, true);
        }
        match result {
            Ok(()) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-run-cancelled"));
            }
            Err(error) => {
                // Even on error we already cleared submitting locally —
                // surface the failure but don't try to recover state.
                self.flash_warning(format!(
                    "{}: {}",
                    ui_text::t(&self.i18n, "flash-cancel-failed"),
                    error
                ));
            }
        }
    }

    fn handle_session_execution_updated(
        &mut self,
        session_id: i64,
        execution: SessionExecutionResource,
        refresh: bool,
    ) {
        let transcript_is_target = self.transcript.session_id == Some(session_id);
        if transcript_is_target {
            self.transcript.submitting = false;
            if self.apply_transcript_execution(execution) {
                self.sync_pending_interactive_after_execution(session_id);
                self.sync_session_list_selection_to_current_execution();
            }
        }
        self.submitting_session_ids.remove(&session_id);
        if refresh && transcript_is_target {
            self.request_refresh(session_id, true);
        }
        self.request_sessions(false);
    }

    fn handle_session_continued(
        &mut self,
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    ) {
        match result {
            Ok(execution) => self.handle_session_execution_updated(session_id, execution, true),
            Err(error) => {
                self.transcript.submitting = false;
                self.submitting_session_ids.remove(&session_id);
                self.flash_error(error);
            }
        }
    }

    fn handle_session_renamed(&mut self, session_id: i64, result: UiResult<SessionResource>) {
        match result {
            Ok(session) => {
                if let Some(existing) = self
                    .sessions
                    .source_items
                    .iter_mut()
                    .find(|item| item.id == session_id)
                {
                    *existing = session.clone();
                }
                if let Some(existing) = self
                    .sessions
                    .list
                    .items
                    .iter_mut()
                    .find(|item| item.id == session_id)
                {
                    *existing = session.clone();
                }
                if self.transcript.session_id == Some(session_id) {
                    self.transcript.session_title = session.title.clone();
                    if let Some(execution) = self.transcript.execution.as_mut() {
                        execution.session = session.clone();
                    }
                }
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-session-renamed",
                    &crate::fl_args!("title" => session.title),
                ));
                self.overlay = None;
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn handle_permission_replied(
        &mut self,
        session_id: i64,
        label: String,
        result: UiResult<SessionExecutionResource>,
    ) {
        match result {
            Ok(execution) => {
                let transcript_is_target = self.transcript.session_id == Some(session_id);
                if transcript_is_target {
                    self.transcript.submitting = false;
                    if self.apply_transcript_execution(execution) {
                        self.sync_pending_interactive_after_execution(session_id);
                        self.sync_session_list_selection_to_current_execution();
                    }
                }
                self.submitting_session_ids.remove(&session_id);
                if transcript_is_target {
                    self.request_refresh(session_id, true);
                }
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-permission-reply-sent",
                    &crate::fl_args!("label" => label),
                ));
            }
            Err(error) => {
                self.transcript.submitting = false;
                self.submitting_session_ids.remove(&session_id);
                self.pending_permission_replay = None;
                self.flash_error(error);
            }
        }
    }

    fn handle_user_input_replied(
        &mut self,
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    ) {
        match result {
            Ok(execution) => {
                self.handle_session_execution_updated(session_id, execution, true);
                self.flash_success(ui_text::t(&self.i18n, "flash-user-input-reply-sent"));
            }
            Err(error) => {
                self.transcript.submitting = false;
                self.submitting_session_ids.remove(&session_id);
                self.flash_error(error);
            }
        }
    }

    fn handle_providers_loaded(
        &mut self,
        purpose: ProviderPickerPurpose,
        result: UiResult<Vec<ProviderSummaryResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_picker_dialog() else {
            return;
        };
        let PickerKind::Providers(current_purpose) = &dialog.meta.kind else {
            self.restore_picker_dialog(host, dialog);
            return;
        };
        if *current_purpose != purpose {
            self.restore_picker_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match result {
            Ok(providers) => {
                let fallback_adapter = settings_choice_adapter_fallback(&self.i18n);
                let mut items = Vec::new();
                if purpose == ProviderPickerPurpose::Configure {
                    items.push(provider_list_create_item(&self.i18n));
                }
                items.extend(providers.into_iter().map(|provider| {
                    let detail = if purpose == ProviderPickerPurpose::Configure {
                        i18n_provider_list_detail(&self.i18n, &provider)
                    } else {
                        settings_choice_default_provider_detail(
                            &self.i18n,
                            provider
                                .defaults
                                .adapter
                                .as_deref()
                                .unwrap_or(fallback_adapter.as_str()),
                            provider.defaults.model.as_str(),
                        )
                    };
                    PickerItem {
                        label: provider.provider_id.clone(),
                        detail,
                        value: PickerValue::Provider(provider),
                    }
                }));
                dialog.meta.all_items = items;
                Self::refresh_picker_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_picker_dialog(host, dialog);
    }

    fn handle_agents_loaded(&mut self, result: UiResult<Vec<AgentDescriptor>>) {
        let Some((host, mut dialog)) = self.take_picker_dialog() else {
            return;
        };
        if !matches!(dialog.meta.kind, PickerKind::Agents) {
            self.restore_picker_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match result {
            Ok(agents) => {
                dialog.meta.all_items = agent_list_items(
                    &self.i18n,
                    agents,
                    self.backend.default_agent_name().as_deref(),
                    &self.backend.config_agent_names(),
                );
                Self::refresh_picker_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_picker_dialog(host, dialog);
    }

    fn handle_session_search_page_loaded(
        &mut self,
        mode: SessionViewMode,
        query: String,
        page_index: usize,
        result: UiResult<PaginatedResponse<SessionResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_session_search_dialog() else {
            return;
        };
        if dialog.meta.mode != mode
            || dialog.meta.page_index != page_index
            || dialog.input.text().trim() != query
        {
            self.restore_session_search_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-resume-empty");
        match result {
            Ok(page) => {
                dialog.items = page
                    .items
                    .into_iter()
                    .map(|session| self.session_search_item(session))
                    .collect();
                dialog.meta.offset = dialog
                    .meta
                    .page_index
                    .saturating_mul(dialog.meta.page_limit);
                dialog.meta.next_cursor = page.page.next_cursor;
                dialog.meta.has_more = page.page.has_more;
                dialog.clamp_selection();
                dialog.footer = self.session_search_footer(&dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_session_search_dialog(host, dialog);
    }

    fn handle_session_search_subtree_loaded(
        &mut self,
        session_id: i64,
        query: String,
        result: UiResult<Vec<SessionResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_session_search_dialog() else {
            return;
        };
        if dialog.meta.mode != SessionViewMode::Subtree
            || dialog.meta.scope_session_id != Some(session_id)
            || dialog.input.text().trim() != query
        {
            self.restore_session_search_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-resume-empty");
        match result {
            Ok(mut sessions) => {
                sessions.sort_by(|left, right| {
                    right
                        .updated_at
                        .cmp(&left.updated_at)
                        .then_with(|| right.id.cmp(&left.id))
                });
                dialog.meta.all_items = sessions
                    .into_iter()
                    .map(|session| self.session_search_item(session))
                    .collect();
                self.refresh_session_search_overlay_local(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_session_search_dialog(host, dialog);
    }

    fn handle_lineage_loaded(&mut self, session_id: i64, result: UiResult<Vec<SessionResource>>) {
        match result {
            Ok(sessions) => {
                let items = build_lineage_session_items(sessions.as_slice(), session_id);
                if self.transcript.session_id == Some(session_id)
                    && let Some(summary) = summarize_lineage_session_items(items.as_slice())
                {
                    self.current_lineage = Some(CurrentLineageState {
                        session_id,
                        summary,
                        path: lineage_path_segments(sessions.as_slice(), session_id),
                    });
                }

                let Some((host, mut dialog)) = self.take_picker_dialog() else {
                    return;
                };
                let PickerKind::Lineage {
                    session_id: current_session_id,
                } = &dialog.meta.kind
                else {
                    self.restore_picker_dialog(host, dialog);
                    return;
                };
                if *current_session_id != session_id {
                    self.restore_picker_dialog(host, dialog);
                    return;
                }

                dialog.loading = false;
                dialog.empty_message = ui_text::t(&self.i18n, "overlay-lineage-empty");
                dialog.meta.all_items = items
                    .into_iter()
                    .map(|item| self.lineage_session_picker_item(item))
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
                self.restore_picker_dialog(host, dialog);
            }
            Err(error) => {
                if let Some((host, mut dialog)) = self.take_picker_dialog() {
                    if matches!(dialog.meta.kind, PickerKind::Lineage { session_id: current_session_id } if current_session_id == session_id)
                    {
                        dialog.loading = false;
                        dialog.empty_message = ui_text::t(&self.i18n, "overlay-lineage-empty");
                    }
                    self.restore_picker_dialog(host, dialog);
                }
                self.flash_error(error);
            }
        }
    }

    fn handle_rewind_messages_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<Vec<MessageResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_picker_dialog() else {
            return;
        };
        let PickerKind::RewindMessages {
            session_id: current_session_id,
        } = &dialog.meta.kind
        else {
            self.restore_picker_dialog(host, dialog);
            return;
        };
        if *current_session_id != session_id {
            self.restore_picker_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-rewind-empty");
        match result {
            Ok(messages) => {
                dialog.meta.all_items = messages
                    .into_iter()
                    .filter(is_rewind_target_message)
                    .rev()
                    .map(|message| self.rewind_message_picker_item(message))
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_picker_dialog(host, dialog);
    }

    fn handle_model_catalog_loaded(
        &mut self,
        query: String,
        offset: usize,
        result: UiResult<ModelCatalogListResponse>,
    ) {
        let Some((host, mut dialog)) = self.take_model_catalog_dialog() else {
            return;
        };
        if dialog.query != query || dialog.offset != offset {
            self.restore_model_catalog_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        match result {
            Ok(response) => {
                dialog.workbench.list.items = response.items;
                dialog.summary = response.summary;
                dialog.total = response.total;
                dialog.offset = response.offset;
                dialog.limit = response.limit;
                dialog.workbench.list.clamp_selection();
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_model_catalog_dialog(host, dialog);
    }

    fn handle_provider_studio_adapter_models_loaded(
        &mut self,
        request_key: String,
        result: UiResult<ProviderAdapterModelsResponse>,
    ) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            return;
        };
        if dialog.pending_adapter_models_key.as_deref() != Some(request_key.as_str()) {
            self.restore_provider_studio_dialog(host, dialog);
            return;
        }

        dialog.listing_adapter_models = false;
        dialog.pending_adapter_models_key = None;
        match result {
            Ok(response) => {
                let preserved_model_keys = dialog.selected_model_keys.clone();
                dialog.adapter_models = response.adapters;
                dialog
                    .selection
                    .clamp_left(dialog.adapter_candidate_ids.len());
                dialog.selection.set_right_selected(0);
                self.reload_provider_studio_catalog_matches(&mut dialog);
                dialog.selected_model_keys = preserved_model_keys;
                provider_studio_restore_model_selection(&mut dialog);
                provider_studio_ensure_default_selection(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    fn handle_provider_studio_auth_completed(
        &mut self,
        request_key: String,
        result: std::result::Result<
            crate::backend::ProviderDraftAuthActionResult,
            crate::backend::ProviderDraftAuthError,
        >,
    ) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            match result {
                Ok(action) => {
                    if !provider_draft_auth_message_is_pending(&action.message) {
                        self.flash_success(provider_draft_auth_action_message(
                            &self.i18n,
                            &action.message,
                        ));
                    }
                }
                Err(error) => {
                    self.flash_error(provider_draft_auth_error_message(&self.i18n, &error))
                }
            }
            return;
        };
        if dialog.pending_auth_key.as_deref() != Some(request_key.as_str()) {
            self.restore_provider_studio_dialog(host, dialog);
            return;
        }

        dialog.pending_auth_key = None;
        match result {
            Ok(action) => {
                dialog.draft = action.draft;
                self.sync_provider_studio_shape(&mut dialog);
                self.sync_provider_studio_auth_poll_deadline(&mut dialog, Instant::now(), true);
                let preferred_detail_field = provider_studio_preferred_detail_field_index(&dialog);
                if let Some(detail_page) = dialog.detail_page.as_mut() {
                    detail_page.selection.selected = preferred_detail_field;
                }
                if let Some(text) = action.clipboard_text {
                    let _ = set_clipboard_text(text.as_str());
                }
                if !provider_draft_auth_message_is_pending(&action.message) {
                    self.flash_success(provider_draft_auth_action_message(
                        &self.i18n,
                        &action.message,
                    ));
                }
            }
            Err(error) => {
                self.sync_provider_studio_auth_poll_deadline(&mut dialog, Instant::now(), true);
                self.flash_error(provider_draft_auth_error_message(&self.i18n, &error));
            }
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    fn handle_provider_studio_saved(
        &mut self,
        provider_id: String,
        result: std::result::Result<
            crate::backend::ProviderStudioSaveResult,
            crate::backend::ProviderStudioSaveError,
        >,
    ) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            match result {
                Ok(message) => {
                    self.flash_success(provider_studio_save_result_message(&self.i18n, &message))
                }
                Err(error) => {
                    self.flash_error(provider_studio_save_error_message(&self.i18n, &error))
                }
            }
            return;
        };
        dialog.saving = false;
        match result {
            Ok(message) => {
                let preserved_selected_adapter_ids = dialog.selected_adapter_ids.clone();
                let preserved_selected_adapter_id = provider_studio_selected_adapter_id(&dialog);
                let mut preserved_selected_model_keys = dialog.selected_model_keys.clone();
                let mut preserved_selected_adapter_ids = preserved_selected_adapter_ids;
                let mut preserved_selected_adapter_id = preserved_selected_adapter_id;
                match &message {
                    crate::backend::ProviderStudioSaveResult::ModelDeleted {
                        adapter_id,
                        model_id,
                        ..
                    } => {
                        preserved_selected_model_keys
                            .remove(provider_studio_model_key(adapter_id, model_id).as_str());
                    }
                    crate::backend::ProviderStudioSaveResult::AdapterDeleted {
                        adapter_id, ..
                    } => {
                        preserved_selected_adapter_ids.remove(adapter_id.as_str());
                        if preserved_selected_adapter_id.as_deref() == Some(adapter_id.as_str()) {
                            preserved_selected_adapter_id = None;
                        }
                        let prefix = format!("{adapter_id}\u{1f}");
                        preserved_selected_model_keys
                            .retain(|key| !key.starts_with(prefix.as_str()));
                    }
                    crate::backend::ProviderStudioSaveResult::ProviderDeleted { .. } => {}
                    crate::backend::ProviderStudioSaveResult::ProviderDraftSaved { .. }
                    | crate::backend::ProviderStudioSaveResult::AdapterMatchesSaved { .. }
                    | crate::backend::ProviderStudioSaveResult::ModelSaved { .. }
                    | crate::backend::ProviderStudioSaveResult::ConfiguredModelSaved { .. } => {}
                }
                self.flash_success(provider_studio_save_result_message(&self.i18n, &message));
                if matches!(
                    &message,
                    crate::backend::ProviderStudioSaveResult::ProviderDeleted { .. }
                ) {
                    self.restore_provider_list_after_provider_delete();
                    return;
                }
                let providers = self.backend.list_configured_providers();
                let provider_rows = provider_studio_provider_rows(&self.i18n, providers.as_slice());
                let selected_provider = provider_rows
                    .iter()
                    .position(|row| row.provider_id.as_deref() == Some(provider_id.as_str()))
                    .unwrap_or(0);
                dialog.providers = SelectableListState::new(provider_rows, selected_provider);
                self.load_provider_studio_draft(&mut dialog, Some(provider_id.as_str()), None);
                restore_provider_studio_adapter_selection(
                    &mut dialog,
                    &preserved_selected_adapter_ids,
                    preserved_selected_adapter_id.as_deref(),
                );
                dialog.selected_model_keys = preserved_selected_model_keys;
                match &message {
                    crate::backend::ProviderStudioSaveResult::AdapterDeleted { .. } => {
                        dialog.selection.set_focus(ProviderStudioFocus::Adapters);
                    }
                    crate::backend::ProviderStudioSaveResult::ModelDeleted { .. } => {
                        dialog.selection.set_focus(ProviderStudioFocus::Models);
                    }
                    _ => {}
                }
                provider_studio_ensure_default_selection(&mut dialog);
            }
            Err(error) => self.flash_error(provider_studio_save_error_message(&self.i18n, &error)),
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    fn restore_provider_list_after_provider_delete(&mut self) {
        let provider_picker = self.route_stack.last().and_then(|route| match route {
            Route::Picker(dialog)
                if matches!(
                    dialog.meta.kind,
                    PickerKind::Providers(ProviderPickerPurpose::Configure)
                ) =>
            {
                Some(dialog.input.text().to_string())
            }
            _ => None,
        });
        if provider_picker.is_some() {
            let _ = self.route_stack.pop();
        }
        self.current_route = Route::Picker(
            self.build_provider_list_overlay(provider_picker.as_deref().unwrap_or(""), false),
        );
        self.overlay = None;
    }

    fn handle_model_catalog_refreshed(&mut self, result: UiResult<()>) {
        let Some((host, mut dialog)) = self.take_model_catalog_dialog() else {
            match result {
                Ok(()) => self.flash_success(ui_text::t(
                    &self.i18n,
                    "flash-provider-studio-catalog-refreshed",
                )),
                Err(error) => self.flash_error(error),
            }
            return;
        };

        match result {
            Ok(()) => {
                self.flash_success(ui_text::t(
                    &self.i18n,
                    "flash-provider-studio-catalog-refreshed",
                ));
                dialog.loading = true;
                dialog.offset = 0;
                dialog.workbench.list.selected = 0;
                self.request_model_catalog_page(dialog.query.clone(), 0);
            }
            Err(error) => {
                dialog.loading = false;
                self.flash_error(error);
            }
        }
        self.restore_model_catalog_dialog(host, dialog);
    }

    fn handle_child_sessions_loaded(
        &mut self,
        parent_session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    ) {
        let Some((host, mut dialog)) = self.take_picker_dialog() else {
            return;
        };
        let PickerKind::ChildSessions {
            parent_session_id: current_parent_id,
        } = &dialog.meta.kind
        else {
            self.restore_picker_dialog(host, dialog);
            return;
        };
        if *current_parent_id != parent_session_id {
            self.restore_picker_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-children-empty");
        match result {
            Ok(sessions) => {
                dialog.meta.all_items = sessions
                    .into_iter()
                    .map(|session| PickerItem {
                        label: session.title.clone(),
                        detail: format!(
                            "#{} | {} msg | {} child",
                            session.id, session.message_count, session.child_session_count
                        ),
                        value: PickerValue::Session(session.id),
                    })
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_picker_dialog(host, dialog);
    }

    fn handle_timeline_loaded(&mut self, session_id: i64, result: UiResult<Vec<DomainEvent>>) {
        let Some((host, mut dialog)) = self.take_timeline_dialog() else {
            return;
        };
        if dialog.meta.session_id != session_id {
            self.restore_timeline_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-timeline-empty");
        match result {
            Ok(events) => {
                dialog.all_items = events
                    .iter()
                    .map(|event| build_timeline_item(&self.i18n, event))
                    .collect();
                Self::refresh_timeline_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_timeline_dialog(host, dialog);
    }

    fn handle_session_rewound(
        &mut self,
        session_id: i64,
        target: String,
        result: UiResult<SessionExecutionResource>,
    ) {
        match result {
            Ok(execution) => {
                let rewound_session_id = execution.session.id;
                let preserved_draft = (self.transcript.session_id == Some(session_id))
                    .then(|| self.current_composer_draft());
                if let Some(draft) = preserved_draft.clone() {
                    self.set_draft_for_slot(DraftSlot::Session(rewound_session_id), draft);
                }
                self.open_session(rewound_session_id, execution.session.title.clone());
                if let Some(draft) = preserved_draft {
                    self.replace_composer_draft(draft);
                    self.persist_draft_store_with_feedback(true);
                }
                if self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(rewound_session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
                self.submitting_session_ids.remove(&session_id);
                self.focus = Focus::Composer;
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-session-rewound",
                    &crate::fl_args!("target" => target),
                ));
            }
            Err(error) => {
                if self.transcript.session_id == Some(session_id) {
                    self.transcript.submitting = false;
                }
                self.submitting_session_ids.remove(&session_id);
                self.flash_error(error);
            }
        }
    }

    fn request_sessions(&mut self, append: bool) {
        if append {
            return;
        }
        if self.sessions.loading {
            return;
        }
        self.sessions.loading = true;
        self.sessions.loading_more = false;
        self.sessions.has_more = false;
        self.sessions.next_cursor = None;

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let scope = SessionLoadScope {
            mode: self.sessions.view_mode,
            anchor_session_id: match self.sessions.view_mode {
                SessionViewMode::Subtree => self.current_or_selected_session_id(),
                SessionViewMode::All | SessionViewMode::Roots => None,
            },
        };
        if scope.mode == SessionViewMode::Subtree && scope.anchor_session_id.is_none() {
            self.sessions.loading = false;
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }
        self.sessions.pending_scope = Some(scope.clone());

        tokio::spawn(async move {
            let (result, subtree_root_id) = match scope.mode {
                SessionViewMode::All => (
                    backend
                        .list_workspace_sessions(false)
                        .await
                        .map_err(|error| error.to_string()),
                    None,
                ),
                SessionViewMode::Roots => (
                    backend
                        .list_workspace_sessions(true)
                        .await
                        .map_err(|error| error.to_string()),
                    None,
                ),
                SessionViewMode::Subtree => {
                    let anchor_session_id = scope
                        .anchor_session_id
                        .expect("subtree scope requires anchor");
                    let result = backend
                        .list_session_subtree(anchor_session_id)
                        .await
                        .map_err(|error| error.to_string());
                    let subtree_root_id = result.as_ref().ok().and_then(|items| {
                        items
                            .iter()
                            .find(|item| item.parent_id.is_none())
                            .map(|item| item.id)
                    });
                    (result, subtree_root_id)
                }
            };
            let _ = tx.send(AppMessage::SessionsLoaded {
                scope,
                subtree_root_id,
                result,
            });
        });
    }

    fn request_providers(&mut self, purpose: ProviderPickerPurpose) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = Ok(match purpose {
                ProviderPickerPurpose::SetProvider => backend.list_providers(),
                ProviderPickerPurpose::Configure => backend.list_configured_providers(),
            });
            let _ = tx.send(AppMessage::ProvidersLoaded { purpose, result });
        });
    }

    fn request_agent_list(&mut self) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = Ok(backend.list_agent_descriptors());
            let _ = tx.send(AppMessage::AgentsLoaded { result });
        });
    }

    fn request_session_search_page(
        &mut self,
        mode: SessionViewMode,
        query: String,
        page_index: usize,
        cursor: Option<String>,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_workspace_sessions_page(
                    mode == SessionViewMode::Roots,
                    (!query.trim().is_empty()).then_some(query.as_str()),
                    cursor,
                    50,
                )
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionSearchPageLoaded {
                mode,
                query,
                page_index,
                result,
            });
        });
    }

    fn request_session_search_subtree(&mut self, session_id: i64, query: String) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_subtree(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionSearchSubtreeLoaded {
                session_id,
                query,
                result,
            });
        });
    }

    fn request_lineage(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_subtree(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::LineageLoaded { session_id, result });
        });
    }

    fn request_rewind_messages(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_all_messages(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::RewindMessagesLoaded { session_id, result });
        });
    }

    fn request_child_sessions(&mut self, parent_session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_child_sessions(parent_session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ChildSessionsLoaded {
                parent_session_id,
                result,
            });
        });
    }

    fn request_session_rename(&mut self, session_id: i64, title: String) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .rename_session(session_id, title)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionRenamed { session_id, result });
        });
    }

    fn request_timeline(&mut self, session_id: i64, limit: u64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_timeline(session_id, limit)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::TimelineLoaded { session_id, result });
        });
    }

    fn request_session_rewind(&mut self, session_id: i64, message_id: i64, target: String) {
        self.sync_current_draft_slot();
        self.persist_draft_store_with_feedback(true);
        if self.transcript.session_id == Some(session_id) {
            self.transcript.submitting = true;
        }

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .rewind_session_to_message(session_id, message_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionRewound {
                session_id,
                target,
                result,
            });
        });
    }

    fn request_session_state(&mut self, session_id: i64) {
        if self.transcript.state_loading {
            return;
        }

        self.transcript.state_loading = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .get_session_state(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionStateLoaded { session_id, result });
        });
    }

    fn request_messages(&mut self, session_id: i64, mode: MessageLoadMode) {
        match mode {
            MessageLoadMode::Replace => {
                if self.transcript.loading_initial {
                    return;
                }
                self.transcript.loading_initial = true;
            }
            MessageLoadMode::Prepend => {
                if self.transcript.loading_older || !self.transcript.has_more_older {
                    return;
                }
                self.transcript.loading_older = true;
            }
        }

        let cursor = match mode {
            MessageLoadMode::Replace => None,
            MessageLoadMode::Prepend => self.transcript.older_cursor.clone(),
        };

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_messages(session_id, cursor, MESSAGE_PAGE_SIZE)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::MessagesLoaded {
                session_id,
                mode,
                result,
            });
        });
    }

    fn request_refresh(&mut self, session_id: i64, force: bool) {
        if self.transcript.refreshing {
            return;
        }
        self.transcript.refreshing = true;
        self.last_refresh_at = Instant::now();

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let after_seq = self.transcript.last_event_seq;
        tokio::spawn(async move {
            let result = backend
                .refresh_session(session_id, after_seq, MESSAGE_PAGE_SIZE, force)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionRefreshed { session_id, result });
        });
    }

    fn request_submit_message(&mut self, session_id: i64, draft: ComposerDraft) {
        if self
            .pending_interactive_kind_for_session(session_id)
            .is_some()
        {
            self.restore_composer_draft(draft);
            self.focus = Focus::Composer;
            self.prompt_for_pending_interactive_on_session(session_id);
            return;
        }
        self.transcript.submitting = true;
        self.transcript.pending_restore_draft = Some(draft.clone());
        self.submitting_session_ids.insert(session_id);
        self.set_draft_for_slot(DraftSlot::Session(session_id), draft.clone());
        self.persist_draft_store_with_feedback(true);

        let parts = match self.build_submission_parts(&draft) {
            Ok(parts) => parts,
            Err(error) => {
                self.transcript.submitting = false;
                self.transcript.pending_restore_draft = None;
                self.submitting_session_ids.remove(&session_id);
                self.restore_composer_draft(draft);
                self.flash_error(error);
                return;
            }
        };
        self.record_prompt_history_from_draft(&draft);

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .submit_parts_message_with_options(session_id, parts, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionMessageSubmitted {
                session_id,
                draft,
                result,
            });
        });
    }

    fn request_continue(&mut self, session_id: i64) {
        self.transcript.submitting = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .continue_session_with_options(session_id, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionContinued { session_id, result });
        });
    }

    fn request_compact(&mut self, session_id: i64) {
        self.transcript.submitting = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .compact_session_with_options(session_id, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionCompacted { session_id, result });
        });
    }

    /// Steer the in-flight run by injecting `parts` as a new user message
    /// the model will see on its next step. If the backend reports the
    /// run is no longer steerable, fall back to enqueueing the original
    /// draft so it isn't lost.
    fn request_steer_input(
        &mut self,
        session_id: i64,
        parts: Vec<PartContent>,
        draft: ComposerDraft,
    ) {
        self.record_prompt_history_from_draft(&draft);
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .steer_input(session_id, parts)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SteerSubmitted {
                session_id,
                draft,
                result,
            });
        });
        self.flash_info(ui_text::t(&self.i18n, "flash-steer-sent"));
    }

    /// Ask the backend to cancel the in-flight run for `session_id`.
    /// Best-effort: even if the backend hasn't fully wired cancellation,
    /// we clear the local `submitting` flag so the user regains control.
    fn request_cancel_run(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .cancel_run(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::RunCancelled { session_id, result });
        });
    }

    fn request_permission_reply(
        &mut self,
        session_id: i64,
        request_id: String,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        label: String,
    ) {
        self.transcript.submitting = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .reply_permission_with_options(session_id, request_id, kind, scope, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::PermissionReplied {
                session_id,
                label,
                result,
            });
        });
    }

    fn request_user_input_reply(&mut self, session_id: i64, reply: UserInputReply) {
        self.transcript.submitting = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .reply_user_input_with_options(session_id, reply, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::UserInputReplied { session_id, result });
        });
    }

    fn open_session(&mut self, session_id: i64, title: String) {
        self.sync_current_draft_slot();
        self.clear_composer_state();
        self.current_lineage = None;
        self.focus = Focus::Transcript;
        self.transcript.reset(session_id, title);
        self.seen_permission_request_ids.clear();
        self.seen_user_input_request_ids.clear();
        self.pending_permission_replay = None;
        let _ = self.sessions.select_by_id(session_id);
        self.restore_draft_for_slot(DraftSlot::Session(session_id));
        self.persist_draft_store_with_feedback(true);
        self.subscribe_session_events(session_id);
        self.request_lineage(session_id);
        self.request_session_state(session_id);
        self.request_messages(session_id, MessageLoadMode::Replace);
        if self.sessions.view_mode == SessionViewMode::Subtree {
            self.request_sessions(false);
        }
    }

    /// Spawn a forwarder task that pumps live `LiveEvent`s from the unified
    /// bus into [`AppMessage::SessionEventArrived`]. Aborts any previous
    /// subscription so we never accumulate stale receivers.
    fn subscribe_session_events(&mut self, session_id: i64) {
        if let Some(handle) = self.active_subscription.take() {
            handle.abort();
        }
        let Some(mut rx) = self.backend.subscribe_session_events(session_id) else {
            return;
        };
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
            while let Some(live) = rx.recv().await {
                if tx
                    .send(AppMessage::SessionEventArrived { session_id, live })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.active_subscription = Some(handle);
    }

    fn apply_transcript_execution(&mut self, execution: SessionExecutionResource) -> bool {
        if execution_update_is_stale(self.transcript.last_event_seq, execution.latest_event_seq) {
            return false;
        }
        self.transcript.apply_execution(execution);
        self.sync_seen_pending_request_ids();
        self.sync_open_pending_interactive_overlay();
        true
    }

    fn sync_open_pending_interactive_overlay(&mut self) {
        let keep_overlay = match self.overlay.as_ref() {
            Some(Overlay::Permission(dialog)) => permission_overlay_matches_pending_request(
                dialog,
                self.transcript.session_id,
                self.transcript.execution.as_ref(),
            ),
            Some(Overlay::UserInputReply(dialog)) => user_input_overlay_matches_pending_request(
                dialog,
                self.transcript.session_id,
                self.transcript.execution.as_ref(),
            ),
            _ => true,
        };

        if !keep_overlay {
            self.overlay = None;
        }
    }

    fn sync_seen_pending_request_ids(&mut self) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            self.seen_permission_request_ids.clear();
            self.seen_user_input_request_ids.clear();
            return;
        };
        self.seen_permission_request_ids.retain(|request_id| {
            execution
                .pending_interactive_requests
                .iter()
                .any(|request| {
                    pending_interactive_request_matches_kind(
                        request,
                        PendingInteractiveKind::Permission,
                    ) && pending_interactive_request_id(request) == request_id
                })
        });
        self.seen_user_input_request_ids.retain(|request_id| {
            execution
                .pending_interactive_requests
                .iter()
                .any(|request| {
                    pending_interactive_request_matches_kind(
                        request,
                        PendingInteractiveKind::UserInput,
                    ) && pending_interactive_request_id(request) == request_id
                })
        });
    }

    fn handle_session_event_arrived(&mut self, session_id: i64, live: LiveEvent) {
        // Ignore events for sessions the user has already navigated away
        // from. The forwarder is normally aborted in that case but a few
        // in-flight messages may still land.
        if self.transcript.session_id != Some(session_id) {
            return;
        }
        let refresh_needed_from_event = live.event.as_ref().is_some_and(|event| {
            self.transcript.apply_live_event(
                event,
                self.layout.transcript_body.width,
                self.layout.transcript_body.height,
            )
        });
        if live.force_refresh || live.triggers_refresh || refresh_needed_from_event {
            self.request_refresh(session_id, live.force_refresh);
        }
    }

    fn refresh_status_line_if_due(&mut self, now: Instant) {
        let Some(status_line) = self.status_line.as_mut() else {
            return;
        };
        if status_line.running || now < status_line.next_refresh_at {
            return;
        }

        status_line.running = true;
        status_line.next_refresh_at = now + status_line.refresh_interval;
        let command = status_line.command.clone();
        let tx = self.tx.clone();
        let session_id = self.transcript.session_id.map(|id| id.to_string());
        let focus = self.focus.label().to_string();
        tokio::task::spawn_blocking(move || {
            let output = run_status_line_command(command, session_id, focus);
            let _ = tx.send(AppMessage::StatusLineUpdated { output });
        });
    }

    fn create_session(&mut self, submit_draft: Option<ComposerDraft>) {
        self.create_session_with_parent(submit_draft, None);
    }

    fn create_session_with_parent(
        &mut self,
        submit_draft: Option<ComposerDraft>,
        parent_id: Option<i64>,
    ) {
        if let Some(draft) = submit_draft.as_ref().cloned() {
            self.transcript.submitting = true;
            self.transcript.pending_restore_draft = Some(draft.clone());
            self.set_draft_for_slot(self.current_draft_slot(), draft);
            self.persist_draft_store_with_feedback(true);
        }

        let title = submit_draft
            .as_ref()
            .and_then(draft_title_source)
            .map(|text| derive_session_title(&self.i18n, text.as_str()))
            .unwrap_or_else(|| ui_text::default_session_title(&self.i18n));

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .create_session(title, parent_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionCreated {
                submit_draft,
                result,
            });
        });
    }

    fn is_local_command(&self, input: &str) -> bool {
        if commands::parse_command(input).is_some() {
            return true;
        }
        let Some((name, _)) = commands::parse_invocation(input) else {
            return false;
        };
        self.backend.runtime_tool_exists(name)
    }

    /// Primary submit action (Ctrl+Enter by default). When the AI is
    /// idle, submits the message immediately. When the AI is mid-run, attempts to
    /// `steer_input` (Phase 3) — i.e. inject the message into the live
    /// run so the model sees it on its next step. If the backend rejects
    /// the steer (e.g. the run is in a non-steerable phase), we fall
    /// back to enqueueing the message so it isn't lost.
    fn submit_or_steer(&mut self) {
        self.composer.flush_all_pending_input();
        let draft = self.take_composer_draft();
        if draft.is_empty() {
            return;
        }
        self.reset_prompt_history_recall();
        // Slash-commands always run locally regardless of AI state.
        if self.is_local_command(draft.text.as_str()) {
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        }
        if !self.transcript.submitting {
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        }
        let Some(session_id) = self.transcript.session_id else {
            // No active session — fall back to normal submit which will
            // create one.
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        };
        let parts = match self.build_submission_parts(&draft) {
            Ok(parts) => parts,
            Err(error) => {
                self.restore_composer_draft(draft);
                self.flash_error(error);
                return;
            }
        };
        self.request_steer_input(session_id, parts, draft);
    }

    /// Secondary submit action (bare Enter by default). When the AI is idle,
    /// sends immediately. When the AI is mid-run, the message is appended to
    /// the local pending queue and drained on run completion.
    fn queue_or_submit(&mut self) {
        // During a multi-character paste burst, an Enter inside it should be
        // treated as a literal newline rather than a submit/queue.
        if self.composer.should_insert_newline_on_enter() {
            self.composer.insert_newline_from_enter();
            return;
        }
        self.composer.flush_all_pending_input();
        let draft = self.take_composer_draft();
        if draft.is_empty() {
            return;
        }
        self.reset_prompt_history_recall();
        // Slash-commands always run locally — never queue.
        if self.is_local_command(draft.text.as_str()) {
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        }
        if self.transcript.submitting {
            self.queue.enqueue(draft);
            self.flash_info(ui_text::t(&self.i18n, "flash-message-queued"));
            return;
        }
        self.restore_composer_draft(draft);
        self.submit_composer();
    }

    fn submit_composer(&mut self) {
        self.composer.flush_all_pending_input();
        let draft = self.take_composer_draft();
        if draft.is_empty() || self.transcript.submitting {
            self.restore_composer_draft(draft);
            return;
        }
        self.reset_prompt_history_recall();

        if let Some(parsed) = commands::parse_command(draft.text.as_str()) {
            if !draft.items.is_empty() {
                self.restore_composer_draft(draft);
                self.flash_warning(ui_text::t(
                    &self.i18n,
                    "flash-command-does-not-support-attachments",
                ));
                return;
            }
            self.execute_command(parsed.spec, parsed.args.as_str());
            return;
        }

        if let Some((name, args)) = commands::parse_invocation(draft.text.as_str()) {
            if !draft.items.is_empty() {
                self.restore_composer_draft(draft);
                self.flash_warning(ui_text::t(
                    &self.i18n,
                    "flash-command-does-not-support-attachments",
                ));
                return;
            }
            if self.backend.runtime_tool_exists(name) {
                self.execute_runtime_tool_prompt(name, args);
                return;
            }
        }

        let draft = if draft.text.starts_with("//") {
            draft.with_text_prefix_stripped(1)
        } else {
            draft
        };

        let target_session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id());

        match target_session_id {
            Some(session_id) => self.request_submit_message(session_id, draft),
            None => self.create_session(Some(draft)),
        }
    }

    fn continue_current_session(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        if self.prompt_for_pending_interactive_on_session(session_id) {
            return;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
            return;
        }
        self.request_continue(session_id);
    }

    fn compact_current_session(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        if self.prompt_for_pending_interactive_on_session(session_id) {
            return;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
            return;
        }
        self.request_compact(session_id);
    }

    fn reply_permission(&mut self, kind: PermissionReplyKind) {
        let Some((session_id, request)) = self.pending_permission_overlay_target() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        self.submit_permission_reply(
            session_id,
            request,
            kind,
            None,
            ui_text::permission_reply_label(&self.i18n, kind),
        );
    }

    fn submit_permission_reply(
        &mut self,
        session_id: i64,
        request: PermissionRequest,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        label: String,
    ) {
        self.pending_permission_replay = Some(PermissionReplayState {
            session_id,
            fingerprint: permission_request_fingerprint(&request),
            last_request_id: request.request_id.clone(),
            kind,
            scope,
            label: label.clone(),
        });
        self.seen_permission_request_ids
            .insert(request.request_id.clone());
        self.request_permission_reply(session_id, request.request_id, kind, scope, label);
    }

    fn maybe_auto_reply_duplicate_permission_request(&mut self, session_id: i64) -> bool {
        let Some(replay) = self.pending_permission_replay.clone() else {
            return false;
        };
        if replay.session_id != session_id || self.transcript.session_id != Some(session_id) {
            self.pending_permission_replay = None;
            return false;
        }

        let Some((pending_session_id, request)) = self.pending_permission_overlay_target() else {
            self.pending_permission_replay = None;
            return false;
        };
        if pending_session_id != session_id {
            self.pending_permission_replay = None;
            return false;
        }

        if permission_request_fingerprint(&request) != replay.fingerprint {
            self.pending_permission_replay = None;
            return false;
        }

        if request.request_id == replay.last_request_id {
            return false;
        }

        self.pending_permission_replay = Some(PermissionReplayState {
            last_request_id: request.request_id.clone(),
            ..replay.clone()
        });
        self.seen_permission_request_ids
            .insert(request.request_id.clone());
        self.overlay = None;
        self.request_permission_reply(
            session_id,
            request.request_id,
            replay.kind,
            replay.scope,
            replay.label,
        );
        true
    }

    fn sync_pending_interactive_after_execution(&mut self, session_id: i64) {
        if !self.maybe_auto_reply_duplicate_permission_request(session_id) {
            self.maybe_auto_open_pending_interactive_overlay();
        }
    }

    fn open_user_input_overlay(&mut self) {
        let Some((session_id, request)) = self.pending_user_input_overlay_target() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-user-input-request"));
            return;
        };
        self.seen_user_input_request_ids
            .insert(request.request_id.clone());
        self.overlay = Some(Overlay::UserInputReply(Self::build_user_input_overlay(
            session_id, request,
        )));
    }

    fn pending_user_input_overlay_target(&self) -> Option<(i64, UserInputRequest)> {
        let execution = self.transcript.execution.as_ref()?;
        let request = first_pending_interactive_request_by_kind(
            execution.pending_interactive_requests.as_slice(),
            PendingInteractiveKind::UserInput,
        )?
        .as_user_input()?
        .clone();
        let session_id = self.transcript.session_id?;
        Some((session_id, request))
    }

    fn pending_permission_overlay_target(&self) -> Option<(i64, PermissionRequest)> {
        let execution = self.transcript.execution.as_ref()?;
        let request = first_pending_interactive_request_by_kind(
            execution.pending_interactive_requests.as_slice(),
            PendingInteractiveKind::Permission,
        )?
        .as_permission()?
        .clone();
        let session_id = self.transcript.session_id?;
        Some((session_id, request))
    }

    fn build_user_input_overlay(session_id: i64, request: UserInputRequest) -> UserInputOverlay {
        let mut overlay = UserInputOverlay {
            session_id,
            request,
            answers: BTreeMap::new(),
            state: QuestionFlowState::default(),
            editing_custom: false,
            custom_input: Editor::default(),
            review_option: 0,
            review_scroll: 0,
        };
        Self::sync_user_input_option_selection(&mut overlay);
        overlay
    }

    fn user_input_review_question(request: &UserInputRequest) -> Option<&UserInputQuestion> {
        let question = request.questions.first()?;
        if request.kind.trim() != "review" || request.questions.len() != 1 || question.multiple {
            return None;
        }
        (!question.options.is_empty()).then_some(question)
    }

    fn user_input_overlay_is_review(dialog: &UserInputOverlay) -> bool {
        Self::user_input_review_question(&dialog.request).is_some()
    }

    fn build_permission_overlay(session_id: i64, request: PermissionRequest) -> PermissionOverlay {
        PermissionOverlay {
            session_id,
            request,
            selection: SelectionCursor::default(),
        }
    }

    fn next_pending_interactive_overlay_target(&self) -> Option<PendingInteractiveOverlayTarget> {
        let execution = self.transcript.execution.as_ref()?;
        let session_id = self.transcript.session_id?;
        match first_unseen_pending_interactive_request(
            execution.pending_interactive_requests.as_slice(),
            &self.seen_permission_request_ids,
            &self.seen_user_input_request_ids,
        )? {
            PendingInteractiveRequest::Permission { request } => {
                Some(PendingInteractiveOverlayTarget::Permission {
                    session_id,
                    request: Box::new(request.clone()),
                })
            }
            PendingInteractiveRequest::UserInput { request } => {
                Some(PendingInteractiveOverlayTarget::UserInput {
                    session_id,
                    request: request.clone(),
                })
            }
        }
    }

    fn current_session_pending_interactive_kind(&self) -> Option<PendingInteractiveKind> {
        self.transcript
            .execution
            .as_ref()
            .and_then(pending_interactive_kind_for_execution)
    }

    fn pending_interactive_kind_for_session(
        &self,
        session_id: i64,
    ) -> Option<PendingInteractiveKind> {
        (self.transcript.session_id == Some(session_id))
            .then_some(())
            .and(self.current_session_pending_interactive_kind())
    }

    fn current_session_wait_state_text(&self) -> Option<String> {
        let execution = self.transcript.execution.as_ref()?;
        execution_wait_state_key(execution).map(|key| ui_text::t(&self.i18n, key))
    }

    fn open_pending_interactive_overlay_for_kind(&mut self, kind: PendingInteractiveKind) {
        match kind {
            PendingInteractiveKind::Permission => self.open_permission_overlay(),
            PendingInteractiveKind::UserInput => self.open_user_input_overlay(),
        }
    }

    fn prompt_for_pending_interactive_on_session(&mut self, session_id: i64) -> bool {
        let Some(kind) = self.pending_interactive_kind_for_session(session_id) else {
            return false;
        };
        let key = self
            .transcript
            .execution
            .as_ref()
            .and_then(execution_pending_flash_key)
            .unwrap_or(match kind {
                PendingInteractiveKind::Permission => "flash-session-awaiting-approval",
                PendingInteractiveKind::UserInput => "flash-session-awaiting-user-input",
            });
        self.flash_warning(ui_text::t(&self.i18n, key));
        self.open_pending_interactive_overlay_for_kind(kind);
        true
    }

    fn has_unseen_pending_interactive_request(&self) -> bool {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return false;
        };
        first_unseen_pending_interactive_request(
            execution.pending_interactive_requests.as_slice(),
            &self.seen_permission_request_ids,
            &self.seen_user_input_request_ids,
        )
        .is_some()
    }

    fn should_suppress_pending_interactive_overlay(&self) -> bool {
        if !self.current_route_is_main() {
            return true;
        }
        composer_input_is_active(
            self.focus,
            !self.composer.text().trim().is_empty() || !self.composer_items.is_empty(),
            self.prompt_history_search.is_some()
                || self.file_mention_suggestions.is_some()
                || self.slash_command_suggestions.is_some()
                || self.selected_composer_item.is_some(),
        )
    }

    fn has_suppressed_pending_interactive_overlay(&self) -> bool {
        self.has_unseen_pending_interactive_request()
            && self.should_suppress_pending_interactive_overlay()
    }

    fn maybe_auto_open_pending_interactive_overlay(&mut self) {
        if self.overlay.is_some()
            || !self.current_route_is_main()
            || self.should_suppress_pending_interactive_overlay()
        {
            return;
        }
        match self.next_pending_interactive_overlay_target() {
            Some(PendingInteractiveOverlayTarget::Permission {
                session_id,
                request,
            }) => {
                self.seen_permission_request_ids
                    .insert(request.request_id.clone());
                self.overlay = Some(Overlay::Permission(Self::build_permission_overlay(
                    session_id, *request,
                )));
            }
            Some(PendingInteractiveOverlayTarget::UserInput {
                session_id,
                request,
            }) => {
                self.seen_user_input_request_ids
                    .insert(request.request_id.clone());
                self.overlay = Some(Overlay::UserInputReply(Self::build_user_input_overlay(
                    session_id, request,
                )));
            }
            None => {}
        }
    }

    fn open_permission_overlay(&mut self) {
        let Some((session_id, request)) = self.pending_permission_overlay_target() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        self.seen_permission_request_ids
            .insert(request.request_id.clone());
        self.overlay = Some(Overlay::Permission(Self::build_permission_overlay(
            session_id, request,
        )));
    }

    fn standard_choice_overlay_config() -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            target_width: 96,
            input_enabled: true,
            search_enabled: true,
            custom_value_enabled: true,
            fill_selected_into_input: true,
            min_list_body_height: 3,
            max_list_body_height: 12,
        }
    }

    fn searchable_select_choice_overlay_config() -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            custom_value_enabled: false,
            ..Self::standard_choice_overlay_config()
        }
    }

    fn select_only_choice_overlay_config() -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            target_width: 96,
            input_enabled: false,
            search_enabled: false,
            custom_value_enabled: false,
            fill_selected_into_input: false,
            min_list_body_height: 3,
            max_list_body_height: 12,
        }
    }

    fn standard_picker_overlay_config() -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            target_width: 120,
            input_enabled: true,
            search_enabled: true,
            custom_value_enabled: false,
            fill_selected_into_input: true,
            min_list_body_height: 4,
            max_list_body_height: 10,
        }
    }

    fn path_browser_overlay_config() -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            target_width: 96,
            input_enabled: true,
            search_enabled: false,
            custom_value_enabled: true,
            fill_selected_into_input: true,
            min_list_body_height: 6,
            max_list_body_height: 14,
        }
    }

    fn file_attach_overlay_config() -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            target_width: 88,
            input_enabled: true,
            search_enabled: false,
            custom_value_enabled: true,
            fill_selected_into_input: true,
            min_list_body_height: 4,
            max_list_body_height: 10,
        }
    }

    fn session_search_overlay_config() -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            target_width: 128,
            input_enabled: true,
            search_enabled: false,
            custom_value_enabled: false,
            fill_selected_into_input: true,
            min_list_body_height: 5,
            max_list_body_height: 12,
        }
    }

    fn session_model_chooser_overlay_config() -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            target_width: 128,
            input_enabled: true,
            search_enabled: true,
            custom_value_enabled: false,
            fill_selected_into_input: false,
            min_list_body_height: 5,
            max_list_body_height: 12,
        }
    }

    fn choice_overlay_config(style: ChoiceOverlayStyle) -> SearchListOverlayConfig {
        match style {
            ChoiceOverlayStyle::Searchable => Self::standard_choice_overlay_config(),
            ChoiceOverlayStyle::SearchableSelect => Self::searchable_select_choice_overlay_config(),
            ChoiceOverlayStyle::SelectOnly => Self::select_only_choice_overlay_config(),
        }
    }

    fn choice_overlay_footer(&self, style: ChoiceOverlayStyle) -> String {
        match style {
            ChoiceOverlayStyle::Searchable | ChoiceOverlayStyle::SearchableSelect => {
                ui_text::t(&self.i18n, "overlay-choice-footer")
            }
            ChoiceOverlayStyle::SelectOnly => {
                ui_text::t(&self.i18n, "overlay-choice-footer-select")
            }
        }
    }

    fn choice_overlay_clear_action(&self, action: ChoiceOverlayAction) -> SearchListClearAction {
        SearchListClearAction {
            label: settings_clear_label(&self.i18n),
            detail: choice_overlay_clear_detail(&self.i18n, &action),
        }
    }

    fn build_choice_overlay(
        &self,
        title: String,
        prompt: String,
        input: Editor,
        all_items: Vec<ChoiceItem>,
        action: ChoiceOverlayAction,
        allow_clear: bool,
        style: ChoiceOverlayStyle,
    ) -> ChoiceOverlay {
        let clear_action = allow_clear.then(|| self.choice_overlay_clear_action(action.clone()));
        ChoiceOverlay::new(
            title,
            prompt,
            self.choice_overlay_footer(style),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            input,
            Self::choice_overlay_config(style),
            clear_action,
            ChoiceOverlayMeta {
                i18n: self.i18n.clone(),
                all_items,
                action,
            },
        )
    }

    fn build_picker_overlay(
        &self,
        title: String,
        prompt: String,
        footer: String,
        empty_message: String,
        input: Editor,
        all_items: Vec<PickerItem>,
        kind: PickerKind,
        loading: bool,
    ) -> PickerOverlay {
        let mut overlay = PickerOverlay::new(
            title,
            prompt,
            footer,
            empty_message,
            input,
            Self::standard_picker_overlay_config(),
            None,
            PickerOverlayMeta { all_items, kind },
        );
        overlay.loading = loading;
        Self::refresh_picker_overlay(&mut overlay);
        overlay
    }

    fn build_path_browser_overlay(
        &self,
        title: String,
        prompt: String,
        mode: PathBrowserMode,
        initial: String,
        target: PathBrowserTarget,
    ) -> PathBrowserOverlay {
        let mut overlay = PathBrowserOverlay::new(
            title,
            prompt,
            ui_text::t(&self.i18n, "overlay-permission-rule-browser-footer"),
            ui_text::t(&self.i18n, "overlay-permission-rule-browser-empty"),
            Editor::from_text(initial),
            Self::path_browser_overlay_config(),
            None,
            PathBrowserOverlayMeta {
                i18n: self.i18n.clone(),
                mode,
                target,
            },
        );
        Self::refresh_path_browser_overlay_with_root(self.backend.workspace_root(), &mut overlay);
        overlay
    }

    fn build_file_attach_overlay(&self) -> FileAttachOverlay {
        let mut overlay = FileAttachOverlay::new(
            ui_text::t(&self.i18n, "overlay-attach-title"),
            ui_text::t(&self.i18n, "overlay-attach-prompt"),
            ui_text::t(&self.i18n, "overlay-attach-footer"),
            ui_text::t(&self.i18n, "overlay-attach-no-match"),
            Editor::default(),
            Self::file_attach_overlay_config(),
            None,
            FileAttachOverlayMeta {
                i18n: self.i18n.clone(),
            },
        );
        self.refresh_file_attach_overlay(&mut overlay);
        overlay
    }

    fn build_session_search_overlay(
        &self,
        input: Editor,
        mode: SessionViewMode,
        scope_session_id: Option<i64>,
    ) -> SessionSearchOverlay {
        let mut dialog = SessionSearchOverlay::new(
            ui_text::t(&self.i18n, "overlay-resume-title"),
            ui_text::t(&self.i18n, "overlay-resume-prompt"),
            String::new(),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            input,
            Self::session_search_overlay_config(),
            None,
            SessionSearchOverlayMeta {
                all_items: Vec::new(),
                mode,
                scope_session_id,
                page_limit: 50,
                page_index: 0,
                offset: 0,
                cursors: vec![None],
                next_cursor: None,
                has_more: false,
            },
        );
        dialog.loading = true;
        dialog.footer = self.session_search_footer(&dialog);
        dialog
    }

    fn build_session_model_chooser_overlay(&self) -> SessionModelChooserOverlay {
        let mut dialog = SessionModelChooserOverlay::new(
            ui_text::t(&self.i18n, "overlay-session-model-title"),
            ui_text::t(&self.i18n, "overlay-session-model-prompt"),
            ui_text::t(&self.i18n, "overlay-session-model-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Self::session_model_chooser_overlay_config(),
            None,
            SessionModelChooserOverlayMeta {
                all_items: Vec::new(),
                page_size: 18,
            },
        );
        dialog.loading = true;
        dialog
    }

    fn build_line_input_overlay(
        &self,
        title: String,
        prompt: String,
        input: Editor,
    ) -> LineInputOverlay {
        LineInputOverlay::new(title, prompt, input, ())
    }

    fn build_transcript_search_overlay(&self) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-transcript-search-title"),
            ui_text::t(&self.i18n, "overlay-transcript-search-prompt"),
            Editor::from_text(self.transcript.search_query.clone()),
        )
    }

    fn build_model_catalog_search_overlay(&self, query: &str) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-model-catalog-search-title"),
            ui_text::t(&self.i18n, "overlay-model-catalog-search-prompt"),
            Editor::from_text(query.to_string()),
        )
    }

    fn build_session_rename_overlay(&self, title: String) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-rename-title"),
            ui_text::t(&self.i18n, "overlay-rename-prompt"),
            Editor::from_text(title),
        )
    }

    fn build_agent_create_overlay(&self) -> LineInputOverlay {
        self.build_line_input_overlay(
            ui_text::t(&self.i18n, "overlay-agent-list-create-title"),
            ui_text::t(&self.i18n, "overlay-agent-list-create-prompt"),
            Editor::default(),
        )
    }

    fn build_confirm_overlay(
        &self,
        title: String,
        body_lines: Vec<String>,
        action: ConfirmAction,
    ) -> ConfirmOverlay {
        ConfirmDialogState::new(
            title,
            body_lines,
            ui_text::t(&self.i18n, "overlay-confirm-footer"),
            action,
        )
    }

    fn build_search_panels_overlay<TItem, TMeta>(
        &self,
        title: String,
        prompt: String,
        empty_message: String,
        footer: String,
        input: Editor,
        loading: bool,
        meta: TMeta,
    ) -> SearchPanelsOverlay<TItem, TMeta, Editor> {
        SearchPanelsOverlay::new(title, prompt, empty_message, footer, input, loading, meta)
    }

    fn build_timeline_overlay(&self, session_id: i64) -> TimelineOverlay {
        self.build_search_panels_overlay(
            self.i18n.text_args(
                "overlay-timeline-title",
                &crate::fl_args!("session" => session_id),
            ),
            ui_text::t(&self.i18n, "overlay-timeline-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            ui_text::t(&self.i18n, "overlay-timeline-footer"),
            Editor::default(),
            true,
            TimelineOverlayMeta { session_id },
        )
    }

    fn open_file_attach_overlay(&mut self) {
        self.overlay = Some(Overlay::FileAttach(self.build_file_attach_overlay()));
    }

    fn open_rename_session_overlay(&mut self) {
        let Some(title) = self.current_or_selected_session_title() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.overlay = Some(Overlay::SessionRename(
            self.build_session_rename_overlay(title),
        ));
    }

    fn open_timeline_overlay(&mut self, limit: u64) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.current_route = Route::Timeline(self.build_timeline_overlay(session_id));
        self.request_timeline(session_id, limit);
    }

    fn open_settings_studio(&mut self, query: &str) {
        match self.build_settings_studio_overlay(None, None, SettingsStudioFocus::Navigation) {
            Ok(mut dialog) => {
                self.select_settings_studio_query(&mut dialog, query);
                self.route_stack.clear();
                self.current_route = Route::SettingsStudio(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn build_settings_studio_overlay(
        &self,
        preferred_section: Option<SettingsStudioSectionId>,
        preferred_item_label: Option<&str>,
        focus: SettingsStudioFocus,
    ) -> UiResult<SettingsStudioOverlay> {
        let sources = self
            .backend
            .config_json_sources()
            .map_err(|error| error.to_string())?;
        let agents = self.backend.list_agent_descriptors();
        let default_agent = self.backend.default_agent_name();
        let configured_providers = self.backend.list_configured_providers();
        let permission_rule_count = self
            .block_on_async(self.backend.list_permission_rules())
            .map(|rules| rules.len())
            .unwrap_or_default();
        let global_permission = permission_config_from_json_value(
            &get_json_path(&sources.effective, Some("permission")).unwrap_or(JsonValue::Null),
        )?;
        let global_permission_file = permission_config_from_json_value(
            &get_json_path(&sources.file, Some("permission")).unwrap_or(JsonValue::Null),
        )?;
        let current_session_permission =
            self.current_or_selected_session_id()
                .and_then(|session_id| {
                    self.block_on_async(
                        self.backend.get_session_permission_studio_state(session_id),
                    )
                    .ok()
                });
        let model_catalog = self
            .backend
            .list_model_catalog_models("", 0, 1)
            .map_err(|error| error.to_string())?;

        let runtime_override_items = settings_studio_runtime_items(&self.i18n, &self.run_options);
        let plugin_items = settings_studio_plugin_items(&self.i18n, &sources);
        let mut agent_items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ConfigAgents,
        );
        agent_items.push(settings_studio_agent_browser_item(
            &self.i18n,
            agents.len(),
            default_agent.as_deref(),
        ));
        let provider_items =
            settings_studio_provider_items(&self.i18n, &sources, &configured_providers);
        let runtime_config_items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ConfigRuntime,
        );
        let session_config_items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ConfigSession,
        );
        let tracing_items = settings_studio_field_items(
            &self.i18n,
            &sources,
            SettingsStudioSectionId::ConfigTracing,
        );
        let ui_items =
            settings_studio_field_items(&self.i18n, &sources, SettingsStudioSectionId::ConfigUi);
        let harness_items = settings_studio_harness_items(&self.i18n, &sources);
        let model_catalog_items = settings_studio_model_catalog_items(&self.i18n, &model_catalog);
        let file_items = settings_studio_file_items(&self.i18n, &sources);
        let permission_items = settings_studio_permission_items(
            &self.i18n,
            &sources,
            &global_permission_file,
            &global_permission,
            current_session_permission.as_ref(),
        );
        let mut runtime_rule_items = Vec::new();
        runtime_rule_items.push(SettingsStudioItem::new(
            ui_text::t(&self.i18n, "overlay-settings-manage-permission-rules"),
            permission_rule_count.to_string(),
            ui_text::t(
                &self.i18n,
                "overlay-settings-manage-permission-rules-detail",
            ),
            SettingsPickerAction::OpenPermissionRules,
        ));
        let agent_count = agents.len();
        let mut sections = vec![
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigProviders,
                label: ui_text::t(&self.i18n, "overlay-settings-section-providers-label"),
                summary: self.i18n.text_args(
                    "overlay-settings-section-providers-summary",
                    &crate::fl_args!("count" => configured_providers.len() as i64),
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-providers-description",
                ),
                items: provider_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigAgents,
                label: ui_text::t(&self.i18n, "overlay-settings-section-agents-label"),
                summary: match default_agent.as_deref() {
                    Some(default) => self.i18n.text_args(
                        "overlay-settings-section-agents-summary-default",
                        &crate::fl_args!(
                            "count" => agent_count as i64,
                            "default" => default.to_string(),
                        ),
                    ),
                    None => self.i18n.text_args(
                        "overlay-settings-section-agents-summary",
                        &crate::fl_args!(
                            "count" => agent_count as i64,
                        ),
                    ),
                },
                description: ui_text::t(&self.i18n, "overlay-settings-section-agents-description"),
                items: agent_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigPermission,
                label: ui_text::t(&self.i18n, "overlay-settings-section-permissions-label"),
                summary: current_session_permission
                    .as_ref()
                    .map(|state| {
                        permission_override_summary(&self.i18n, &state.effective_permission)
                    })
                    .unwrap_or_else(|| permission_override_summary(&self.i18n, &global_permission)),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-permissions-description",
                ),
                items: permission_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigPlugins,
                label: ui_text::t(&self.i18n, "overlay-settings-section-plugins-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-plugins-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-plugins-description"),
                items: plugin_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigRuntime,
                label: ui_text::t(&self.i18n, "overlay-settings-section-runtime-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-runtime-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-runtime-description"),
                items: runtime_config_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigSession,
                label: ui_text::t(&self.i18n, "overlay-settings-section-session-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-session-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-session-description"),
                items: session_config_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigHarnesses,
                label: ui_text::t(&self.i18n, "overlay-settings-section-harnesses-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-harnesses-summary"),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-harnesses-description",
                ),
                items: harness_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigTracing,
                label: ui_text::t(&self.i18n, "overlay-settings-section-tracing-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-tracing-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-tracing-description"),
                items: tracing_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ConfigUi,
                label: ui_text::t(&self.i18n, "overlay-settings-section-ui-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-ui-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-ui-description"),
                items: ui_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::RuntimeOverrides,
                label: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-overrides-label",
                ),
                summary: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-overrides-summary",
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-overrides-description",
                ),
                items: runtime_override_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::RuntimeRules,
                label: ui_text::t(&self.i18n, "overlay-settings-section-runtime-rules-label"),
                summary: self.i18n.text_args(
                    "overlay-settings-section-runtime-rules-summary",
                    &crate::fl_args!("count" => permission_rule_count as i64),
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-runtime-rules-description",
                ),
                items: runtime_rule_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Catalogs,
                label: ui_text::t(&self.i18n, "overlay-settings-section-model-catalog-label"),
                summary: self.i18n.text_args(
                    "overlay-settings-section-model-catalog-summary",
                    &crate::fl_args!("count" => model_catalog.summary.model_count as i64),
                ),
                description: ui_text::t(
                    &self.i18n,
                    "overlay-settings-section-model-catalog-description",
                ),
                items: model_catalog_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Files,
                label: ui_text::t(&self.i18n, "overlay-settings-section-files-label"),
                summary: ui_text::t(&self.i18n, "overlay-settings-section-files-summary"),
                description: ui_text::t(&self.i18n, "overlay-settings-section-files-description"),
                items: file_items,
            },
        ];

        let selected_section = preferred_section
            .and_then(|target| sections.iter().position(|section| section.id == target))
            .unwrap_or(0);
        let mut selected_item = preferred_item_label
            .and_then(|label| {
                sections
                    .get(selected_section)
                    .and_then(|section| section.items.iter().position(|item| item.label == label))
            })
            .unwrap_or(0);
        if sections
            .get(selected_section)
            .is_none_or(|section| section.items.is_empty())
        {
            selected_item = 0;
        } else {
            selected_item = min(
                selected_item,
                sections[selected_section].items.len().saturating_sub(1),
            );
        }

        Ok(SettingsStudioOverlay {
            title: ui_text::t(&self.i18n, "overlay-settings-title"),
            footer: ui_text::t(&self.i18n, "overlay-settings-footer"),
            state: SectionedListState::new(
                std::mem::take(&mut sections),
                selected_section,
                selected_item,
                focus,
            ),
        })
    }

    fn refresh_settings_studio_overlay(&mut self, dialog: &mut SettingsStudioOverlay) {
        let preferred_section = dialog.state.selected_section().map(|section| section.id);
        let preferred_item = dialog.state.selected_item().map(|item| item.label.as_str());
        match self.build_settings_studio_overlay(
            preferred_section,
            preferred_item,
            dialog.state.focus(),
        ) {
            Ok(updated) => *dialog = updated,
            Err(error) => self.flash_error(error),
        }
    }

    fn open_agent_studio(&mut self, agent_name: &str) {
        match self.build_agent_studio_overlay(agent_name, None) {
            Ok(dialog) => self.current_route = Route::AgentStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    fn agent_profile_storage(&self, profile: &AgentProfile) -> AgentProfileStorage {
        agent_profile_storage(
            profile,
            self.backend.config_has_agent(profile.name.as_str()),
        )
    }

    fn build_agent_studio_overlay(
        &self,
        agent_name: &str,
        preferred_item_label: Option<&str>,
    ) -> UiResult<AgentStudioOverlay> {
        let profile = self
            .backend
            .get_agent_profile(agent_name)
            .ok_or_else(|| format!("agent not found: {agent_name}"))?;
        let storage = self.agent_profile_storage(&profile);
        let editable = storage.editable();
        let default_agent_name = self.backend.default_agent_name();
        let items = agent_studio_items(&self.i18n, &profile, storage);
        let selected = preferred_item_label
            .and_then(|label| items.iter().position(|item| item.label == label))
            .unwrap_or(0);
        let title = format!(
            "{} · {}",
            ui_text::t(&self.i18n, "overlay-agent-studio-title"),
            profile.name
        );
        let footer = ui_text::t(&self.i18n, "overlay-agent-studio-footer");
        Ok(AgentStudioOverlay {
            agent_name: profile.name.clone(),
            profile,
            storage,
            editable,
            default_agent_name,
            workbench: ListWorkbenchState::new(
                title,
                footer,
                SelectableListState::new(items, selected),
            ),
        })
    }

    fn refresh_agent_studio_overlay(&mut self, dialog: &mut AgentStudioOverlay) {
        let preferred_item = dialog
            .workbench
            .list
            .selected_item()
            .map(|item| item.label.as_str());
        match self.build_agent_studio_overlay(dialog.agent_name.as_str(), preferred_item) {
            Ok(updated) => *dialog = updated,
            Err(error) => self.flash_error(error),
        }
    }

    fn open_global_permission_studio(&mut self) {
        match self.build_permission_studio_overlay(
            PermissionStudioSource::GlobalConfig,
            PermissionStudioPage::Overview,
            Some(PermissionStudioSectionId::RootPath),
            None,
            PermissionStudioFocus::Navigation,
        ) {
            Ok(dialog) => self.current_route = Route::PermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    fn open_agent_permission_studio(&mut self, agent_name: &str) {
        match self.build_permission_studio_overlay(
            PermissionStudioSource::Agent {
                agent_name: agent_name.to_string(),
            },
            PermissionStudioPage::Overview,
            Some(PermissionStudioSectionId::RootPath),
            None,
            PermissionStudioFocus::Navigation,
        ) {
            Ok(dialog) => self.current_route = Route::PermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    fn open_session_permission_studio(&mut self, session_id: i64) {
        match self.build_permission_studio_overlay(
            PermissionStudioSource::Session { session_id },
            PermissionStudioPage::Overview,
            Some(PermissionStudioSectionId::RootPath),
            None,
            PermissionStudioFocus::Navigation,
        ) {
            Ok(dialog) => self.current_route = Route::PermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    fn build_permission_studio_overlay(
        &self,
        source: PermissionStudioSource,
        page: PermissionStudioPage,
        preferred_section: Option<PermissionStudioSectionId>,
        preferred_item_label: Option<&str>,
        preferred_focus: PermissionStudioFocus,
    ) -> UiResult<PermissionStudioOverlay> {
        let (title_context, source_label, scope_label, editable, permission) = match &source {
            PermissionStudioSource::GlobalConfig => {
                let sources = self
                    .backend
                    .config_json_sources()
                    .map_err(|error| error.to_string())?;
                let permission = permission_config_from_json_value(
                    &get_json_path(&sources.file, Some("permission")).unwrap_or(JsonValue::Null),
                )?;
                (
                    ui_text::t(&self.i18n, "settings-permission-global-label"),
                    sources.config_path.display().to_string(),
                    ui_text::t(&self.i18n, "permission-studio-source-global"),
                    true,
                    permission.clone(),
                )
            }
            PermissionStudioSource::Agent { agent_name } => {
                let profile = self
                    .backend
                    .get_agent_profile(agent_name)
                    .ok_or_else(|| format!("agent not found: {agent_name}"))?;
                let storage = self.agent_profile_storage(&profile);
                let permission = profile.frontmatter.permission.clone();
                (
                    profile.name.clone(),
                    agent_profile_source_label_localized(&self.i18n, &profile, storage),
                    agent_profile_scope_label_localized(&self.i18n, &profile),
                    storage.editable(),
                    permission.clone(),
                )
            }
            PermissionStudioSource::Session { session_id } => {
                let state = self
                    .block_on_async(
                        self.backend
                            .get_session_permission_studio_state(*session_id),
                    )
                    .map_err(|error| error.to_string())?;
                (
                    state.session_title,
                    session_id.to_string(),
                    ui_text::t(&self.i18n, "permission-studio-source-session"),
                    true,
                    state.permission,
                )
            }
            PermissionStudioSource::EffectiveSession { session_id } => {
                let state = self
                    .block_on_async(
                        self.backend
                            .get_session_permission_studio_state(*session_id),
                    )
                    .map_err(|error| error.to_string())?;
                (
                    ui_text::t(&self.i18n, "settings-permission-effective-label"),
                    state.session_title,
                    ui_text::t(&self.i18n, "permission-studio-source-effective"),
                    false,
                    state.effective_permission,
                )
            }
        };
        let mut dialog = PermissionStudioOverlay {
            title: String::new(),
            footer: String::new(),
            source,
            title_context,
            source_label,
            scope_label,
            editable,
            permission,
            nav: SelectableListState::new(Vec::new(), 0),
            pane_focus: PermissionStudioPaneFocus::Navigation,
            page,
            state: SectionedListState::new(Vec::new(), 0, 0, PermissionStudioFocus::Navigation),
            editor: None,
        };
        refresh_permission_studio_dialog(
            &self.i18n,
            &mut dialog,
            preferred_section,
            preferred_item_label,
            Some(preferred_focus),
        );
        Ok(dialog)
    }

    fn refresh_permission_studio_overlay(&mut self, dialog: &mut PermissionStudioOverlay) {
        let preferred_section = dialog.state.selected_section().map(|section| section.id);
        let preferred_item = dialog.state.selected_item().map(|item| item.label.as_str());
        let pane_focus = dialog.pane_focus;
        match self.build_permission_studio_overlay(
            dialog.source.clone(),
            dialog.page.clone(),
            preferred_section,
            preferred_item,
            dialog.state.focus(),
        ) {
            Ok(mut updated) => {
                set_permission_studio_pane_focus(&mut updated, pane_focus);
                *dialog = updated;
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn set_permission_studio_page_with_section(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        page: PermissionStudioPage,
        section: Option<PermissionStudioSectionId>,
        focus: PermissionStudioFocus,
    ) {
        dialog.page = page;
        refresh_permission_studio_dialog(&self.i18n, dialog, section, None, Some(focus));
    }

    fn persist_permission_studio(
        &mut self,
        dialog: &mut PermissionStudioOverlay,
        permission: PermissionConfig,
    ) -> UiResult<()> {
        match &dialog.source {
            PermissionStudioSource::GlobalConfig => {
                if permission.is_empty() {
                    self.block_on_async(self.backend.delete_config_setting("permission"))
                        .map_err(|error| error.to_string())?;
                    self.flash_success(settings_path_cleared_message(&self.i18n, "permission"));
                } else {
                    self.block_on_async(self.backend.set_config_setting(
                        "permission",
                        serde_json::to_value(&permission).map_err(|error| error.to_string())?,
                    ))
                    .map_err(|error| error.to_string())?;
                    self.flash_success(settings_path_updated_message(&self.i18n, "permission"));
                }
                self.refresh_current_transcript_execution_state();
            }
            PermissionStudioSource::Agent { agent_name } => {
                let mut profile = self
                    .backend
                    .get_agent_profile(agent_name)
                    .ok_or_else(|| format!("agent not found: {agent_name}"))?;
                match self.agent_profile_storage(&profile) {
                    AgentProfileStorage::Config => {
                        let path = agent_config_path(agent_name.as_str(), "permission");
                        if permission.is_empty() {
                            self.block_on_async(self.backend.delete_config_setting(path.as_str()))
                                .map_err(|error| error.to_string())?;
                            self.flash_success(settings_path_cleared_message(
                                &self.i18n,
                                path.as_str(),
                            ));
                        } else {
                            self.block_on_async(
                                self.backend.set_config_setting(
                                    path.as_str(),
                                    serde_json::to_value(&permission)
                                        .map_err(|error| error.to_string())?,
                                ),
                            )
                            .map_err(|error| error.to_string())?;
                            self.flash_success(settings_path_updated_message(
                                &self.i18n,
                                path.as_str(),
                            ));
                        }
                    }
                    AgentProfileStorage::Markdown => {
                        profile.frontmatter.permission = permission;
                        self.persist_agent_markdown_profile(&profile)?;
                    }
                    AgentProfileStorage::BuiltIn | AgentProfileStorage::Runtime => {
                        return Err(permission_studio_read_only_message(
                            &self.i18n,
                            &dialog.source,
                        ));
                    }
                }
                self.refresh_current_transcript_execution_state();
            }
            PermissionStudioSource::Session { session_id } => {
                let execution = self
                    .block_on_async(self.backend.set_session_permission(*session_id, permission))
                    .map_err(|error| error.to_string())?;
                if self.transcript.session_id == Some(*session_id) {
                    let _ = self.apply_transcript_execution(execution);
                }
                self.flash_success(ui_text::t(&self.i18n, "flash-session-permission-updated"));
            }
            PermissionStudioSource::EffectiveSession { .. } => {
                return Err(permission_studio_read_only_message(
                    &self.i18n,
                    &dialog.source,
                ));
            }
        }
        self.refresh_permission_studio_overlay(dialog);
        Ok(())
    }

    fn permission_studio_selected_item_label(
        &self,
        dialog: &PermissionStudioOverlay,
    ) -> Option<String> {
        dialog
            .state
            .selected_item()
            .map(|item| item.label.as_str().to_string())
    }

    fn open_permission_studio_add_current(&mut self, dialog: &mut PermissionStudioOverlay) {
        if !dialog.editable {
            self.flash_warning(permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        let action = match &dialog.page {
            PermissionStudioPage::PathDefaults
            | PermissionStudioPage::NetworkZones
            | PermissionStudioPage::Overview => {
                self.flash_warning(ui_text::t(&self.i18n, "flash-permission-studio-no-add"));
                return;
            }
            PermissionStudioPage::PathRules => PermissionStudioEditorAction::AddPathRule {
                duplicate_from: None,
            },
            PermissionStudioPage::NetworkRules => PermissionStudioEditorAction::AddNetworkRule {
                duplicate_from: None,
            },
            PermissionStudioPage::ToolTags => PermissionStudioEditorAction::AddToolTag {
                duplicate_from: None,
            },
            PermissionStudioPage::ToolNames => PermissionStudioEditorAction::AddToolName {
                duplicate_from: None,
            },
            PermissionStudioPage::ToolCommandRules => PermissionStudioEditorAction::AddToolRule {
                duplicate_from: None,
            },
        };
        self.open_permission_studio_creator(dialog, action);
    }

    fn open_permission_studio_duplicate_current(&mut self, dialog: &mut PermissionStudioOverlay) {
        if !dialog.editable {
            self.flash_warning(permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        let action = match &dialog.page {
            PermissionStudioPage::PathDefaults
            | PermissionStudioPage::NetworkZones
            | PermissionStudioPage::Overview => {
                self.flash_warning(ui_text::t(
                    &self.i18n,
                    "flash-permission-studio-no-duplicate",
                ));
                return;
            }
            PermissionStudioPage::PathRules => {
                let Some(duplicate_from) = self.permission_studio_selected_item_label(dialog)
                else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                PermissionStudioEditorAction::AddPathRule {
                    duplicate_from: Some(duplicate_from),
                }
            }
            PermissionStudioPage::NetworkRules => {
                let Some(duplicate_from) = self.permission_studio_selected_item_label(dialog)
                else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                PermissionStudioEditorAction::AddNetworkRule {
                    duplicate_from: Some(duplicate_from),
                }
            }
            PermissionStudioPage::ToolTags => {
                let Some(key) = permission_studio_selected_tool_tag_key(dialog) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-duplicate",
                    ));
                    return;
                };
                PermissionStudioEditorAction::AddToolTag {
                    duplicate_from: Some(key),
                }
            }
            PermissionStudioPage::ToolNames => {
                let Some(duplicate_from) = self.permission_studio_selected_item_label(dialog)
                else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                PermissionStudioEditorAction::AddToolName {
                    duplicate_from: Some(duplicate_from),
                }
            }
            PermissionStudioPage::ToolCommandRules => {
                let Some(duplicate_from) = self.permission_studio_selected_item_label(dialog)
                else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                PermissionStudioEditorAction::AddToolRule {
                    duplicate_from: Some(duplicate_from),
                }
            }
        };
        self.open_permission_studio_creator(dialog, action);
    }

    fn open_permission_studio_delete_current(&mut self, dialog: &mut PermissionStudioOverlay) {
        if !dialog.editable {
            self.flash_warning(permission_studio_read_only_message(
                &self.i18n,
                &dialog.source,
            ));
            return;
        }
        let (title, body, action) = match &dialog.page {
            PermissionStudioPage::PathDefaults
            | PermissionStudioPage::NetworkZones
            | PermissionStudioPage::Overview => {
                self.flash_warning(ui_text::t(&self.i18n, "flash-permission-studio-no-delete"));
                return;
            }
            PermissionStudioPage::PathRules => {
                let Some(label) = self.permission_studio_selected_item_label(dialog) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-path-rules"),
                            "value" => label.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeletePathRule { pattern: label },
                )
            }
            PermissionStudioPage::NetworkRules => {
                let Some(label) = self.permission_studio_selected_item_label(dialog) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-network-rules"),
                            "value" => label.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeleteNetworkRule { target: label },
                )
            }
            PermissionStudioPage::ToolTags => {
                let Some(key) = permission_studio_selected_tool_tag_key(dialog) else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-permission-studio-no-delete"));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-tags"),
                            "value" => key.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeleteToolTag { key },
                )
            }
            PermissionStudioPage::ToolNames => {
                let Some(label) = self.permission_studio_selected_item_label(dialog) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-names"),
                            "value" => label.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeleteToolName { key: label },
                )
            }
            PermissionStudioPage::ToolCommandRules => {
                let Some(label) = self.permission_studio_selected_item_label(dialog) else {
                    self.flash_warning(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-no-selection",
                    ));
                    return;
                };
                (
                    ui_text::t(&self.i18n, "overlay-permission-studio-delete-title"),
                    vec![self.i18n.text_args(
                        "overlay-permission-studio-delete-body",
                        &crate::fl_args!(
                            "kind" => ui_text::t(&self.i18n, "permission-studio-page-tool-rules"),
                            "value" => label.clone(),
                        ),
                    )],
                    ConfirmAction::PermissionStudioDeleteToolRule { tool_name: label },
                )
            }
        };
        self.overlay = Some(Overlay::Confirm(
            self.build_confirm_overlay(title, body, action),
        ));
    }

    fn delete_permission_studio_config<F>(&mut self, mutator: F)
    where
        F: FnOnce(&mut PermissionConfig),
    {
        let Some((host, mut dialog)) = self.take_permission_studio_dialog() else {
            self.flash_error(ui_text::t(
                &self.i18n,
                "flash-permission-studio-context-lost",
            ));
            return;
        };

        let mut permission = dialog.permission.clone();
        mutator(&mut permission);
        normalize_permission_config(&mut permission);
        match self.persist_permission_studio(&mut dialog, permission) {
            Ok(()) => self.restore_permission_studio_dialog(host, dialog),
            Err(error) => {
                self.restore_permission_studio_dialog(host, dialog);
                self.flash_error(error);
            }
        }
    }

    fn delete_permission_studio_path_rule(&mut self, pattern: &str) {
        let pattern = pattern.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(path) = permission.path.as_mut() {
                path.rules.shift_remove(pattern.as_str());
            }
        });
    }

    fn delete_permission_studio_network_rule(&mut self, target: &str) {
        let target = target.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(network) = permission.network.as_mut() {
                network.rules.shift_remove(target.as_str());
            }
        });
    }

    fn delete_permission_studio_tool_tag(&mut self, key: &str) {
        let key = key.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(tools) = permission.tools.as_mut() {
                tools.tags.remove(key.as_str());
            }
        });
    }

    fn delete_permission_studio_tool_name(&mut self, key: &str) {
        let key = key.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(tools) = permission.tools.as_mut() {
                tools.names.remove(key.as_str());
            }
        });
    }

    fn delete_permission_studio_tool_rule(&mut self, tool_name: &str) {
        let tool_name = tool_name.to_string();
        self.delete_permission_studio_config(move |permission| {
            if let Some(tools) = permission.tools.as_mut() {
                tools.rules.remove(tool_name.as_str());
            }
        });
    }

    fn refresh_current_transcript_execution_state(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        match self.block_on_async(self.backend.get_session_state(session_id)) {
            Ok(execution) => {
                let _ = self.apply_transcript_execution(execution);
            }
            Err(error) => self.flash_error(error.to_string()),
        }
    }

    fn select_settings_studio_query(&self, dialog: &mut SettingsStudioOverlay, query: &str) {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return;
        }
        for (section_index, section) in dialog.state.sections().iter().enumerate() {
            for (item_index, item) in section.items.iter().enumerate() {
                if section.label.to_ascii_lowercase().contains(query.as_str())
                    || section
                        .summary
                        .to_ascii_lowercase()
                        .contains(query.as_str())
                    || section
                        .description
                        .to_ascii_lowercase()
                        .contains(query.as_str())
                    || item.label.to_ascii_lowercase().contains(query.as_str())
                    || item.value.to_ascii_lowercase().contains(query.as_str())
                    || item.detail.to_ascii_lowercase().contains(query.as_str())
                {
                    dialog.state.set_indices(section_index, item_index);
                    dialog.state.set_focus(SettingsStudioFocus::Items);
                    return;
                }
            }
        }
    }

    fn activate_settings_studio_selection(&mut self, dialog: &mut SettingsStudioOverlay) -> bool {
        if dialog.state.focus() == SettingsStudioFocus::Navigation {
            dialog.state.set_focus(SettingsStudioFocus::Items);
            return false;
        }
        let Some(item) = dialog.state.selected_item().cloned() else {
            return false;
        };
        match item.action {
            SettingsPickerAction::EditField(field) => {
                self.open_settings_field_editor(field, "");
                false
            }
            SettingsPickerAction::EditRuntimeSetting(field) => {
                self.open_runtime_setting_editor(field, "");
                false
            }
            SettingsPickerAction::OpenPluginPolicyStudio => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                match self.build_plugin_policy_studio() {
                    Ok(policy) => {
                        self.current_route = Route::PluginPolicyStudio(Box::new(policy));
                    }
                    Err(error) => self.flash_error(error),
                }
                false
            }
            SettingsPickerAction::OpenProviderDefaultWizard => {
                self.open_provider_default_wizard();
                false
            }
            SettingsPickerAction::OpenAgentList => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_agent_list("");
                false
            }
            SettingsPickerAction::OpenAgentPermissionWorkbench(agent_name) => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_agent_permission_studio(agent_name.as_str());
                false
            }
            SettingsPickerAction::OpenProviderList => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_provider_list("");
                false
            }
            SettingsPickerAction::OpenModelCatalogWorkbench => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_model_catalog_studio();
                false
            }
            SettingsPickerAction::OpenRuntimeProviderOverride => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_provider_picker(ProviderPickerPurpose::SetProvider);
                false
            }
            SettingsPickerAction::OpenRuntimeModelOverride => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_session_model_chooser();
                false
            }
            SettingsPickerAction::ClearRuntimeModelStack => {
                self.clear_provider_model_overrides();
                self.flash_success(ui_text::t(&self.i18n, "flash-runtime-model-stack-cleared"));
                self.refresh_settings_studio_overlay(dialog);
                false
            }
            SettingsPickerAction::OpenGlobalPermissionWorkbench => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_global_permission_studio();
                false
            }
            SettingsPickerAction::OpenCurrentSessionPermissionWorkbench => {
                let Some(session_id) = self.current_or_selected_session_id() else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
                    return false;
                };
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_session_permission_studio(session_id);
                false
            }
            SettingsPickerAction::OpenSessionEffectivePermissionView(session_id) => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                match self.build_permission_studio_overlay(
                    PermissionStudioSource::EffectiveSession { session_id },
                    PermissionStudioPage::Overview,
                    Some(PermissionStudioSectionId::RootPath),
                    None,
                    PermissionStudioFocus::Navigation,
                ) {
                    Ok(permission) => self.current_route = Route::PermissionStudio(permission),
                    Err(error) => self.flash_error(error),
                }
                false
            }
            SettingsPickerAction::OpenPermissionRules => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_permission_rule_picker("");
                false
            }
            SettingsPickerAction::OpenPluginWorkbench => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                match self.build_plugin_workbench("") {
                    Ok(workbench) => {
                        self.current_route = Route::PluginWorkbench(Box::new(workbench));
                    }
                    Err(error) => self.flash_error(error),
                }
                false
            }
            SettingsPickerAction::OpenConfigFile => {
                self.open_runtime_config_in_editor();
                false
            }
        }
    }

    fn refresh_restored_route(&self, route: Route) -> Route {
        match route {
            Route::SettingsStudio(dialog) => self
                .build_settings_studio_overlay(
                    dialog.state.selected_section().map(|section| section.id),
                    dialog.state.selected_item().map(|item| item.label.as_str()),
                    dialog.state.focus(),
                )
                .map(Route::SettingsStudio)
                .unwrap_or(Route::SettingsStudio(dialog)),
            Route::AgentStudio(dialog) => self
                .build_agent_studio_overlay(
                    dialog.agent_name.as_str(),
                    dialog
                        .workbench
                        .list
                        .selected_item()
                        .map(|item| item.label.as_str()),
                )
                .map(Route::AgentStudio)
                .unwrap_or(Route::AgentStudio(dialog)),
            Route::PermissionStudio(dialog) => self
                .build_permission_studio_overlay(
                    dialog.source.clone(),
                    dialog.page.clone(),
                    dialog.state.selected_section().map(|section| section.id),
                    dialog.state.selected_item().map(|item| item.label.as_str()),
                    dialog.state.focus(),
                )
                .map(|mut updated| {
                    updated.pane_focus = dialog.pane_focus;
                    Route::PermissionStudio(updated)
                })
                .unwrap_or(Route::PermissionStudio(dialog)),
            Route::Picker(dialog) if matches!(dialog.meta.kind, PickerKind::PermissionRules) => {
                self.build_permission_rule_picker_overlay(dialog.input.text())
                    .map(Route::Picker)
                    .unwrap_or(Route::Picker(dialog))
            }
            Route::Picker(dialog) if matches!(dialog.meta.kind, PickerKind::Agents) => {
                Route::Picker(self.build_agent_list_overlay(dialog.input.text(), false))
            }
            Route::Picker(dialog)
                if matches!(
                    dialog.meta.kind,
                    PickerKind::Providers(ProviderPickerPurpose::Configure)
                ) =>
            {
                Route::Picker(self.build_provider_list_overlay(dialog.input.text(), false))
            }
            Route::PluginPolicyStudio(dialog) => Route::PluginPolicyStudio(Box::new(
                self.refresh_restored_plugin_policy_studio(*dialog),
            )),
            Route::PluginWorkbench(dialog) => {
                Route::PluginWorkbench(Box::new(self.refresh_restored_plugin_workbench(*dialog)))
            }
            other => other,
        }
    }

    fn refresh_restored_overlay(&self, overlay: Overlay) -> Overlay {
        overlay
    }

    fn refresh_permission_rules_route(&mut self, query: &str) {
        let route_query = query.to_string();
        let should_refresh_current = matches!(
            &self.current_route,
            Route::Picker(dialog) if matches!(dialog.meta.kind, PickerKind::PermissionRules)
        );
        if should_refresh_current {
            self.open_permission_rule_picker(route_query.as_str());
            return;
        }
        self.open_permission_rule_picker(route_query.as_str());
    }

    fn refresh_current_route_after_local_edit(&mut self) {
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        self.current_route = self.refresh_restored_route(route);
    }

    fn take_picker_dialog(&mut self) -> Option<(DialogHost, PickerOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::Picker(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::Picker(dialog)) => Some((DialogHost::Overlay, dialog)),
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    fn restore_picker_dialog(&mut self, host: DialogHost, dialog: PickerOverlay) {
        match host {
            DialogHost::Route => self.current_route = Route::Picker(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::Picker(dialog)),
        }
    }

    fn take_session_search_dialog(&mut self) -> Option<(DialogHost, SessionSearchOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::SessionSearch(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::SessionSearch(dialog)) => Some((DialogHost::Overlay, dialog)),
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    fn restore_session_search_dialog(&mut self, host: DialogHost, dialog: SessionSearchOverlay) {
        match host {
            DialogHost::Route => self.current_route = Route::SessionSearch(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::SessionSearch(dialog)),
        }
    }

    fn take_provider_studio_dialog(&mut self) -> Option<(DialogHost, ProviderStudioOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::ProviderStudio(dialog) => Some((DialogHost::Route, *dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::ProviderStudio(dialog)) => Some((DialogHost::Overlay, *dialog)),
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    fn take_permission_studio_dialog(&mut self) -> Option<(DialogHost, PermissionStudioOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::PermissionStudio(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                None
            }
        }
    }

    fn restore_permission_studio_dialog(
        &mut self,
        host: DialogHost,
        dialog: PermissionStudioOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::PermissionStudio(dialog),
            DialogHost::Overlay => {}
        }
    }

    fn restore_provider_studio_dialog(&mut self, host: DialogHost, dialog: ProviderStudioOverlay) {
        match host {
            DialogHost::Route => self.current_route = Route::ProviderStudio(Box::new(dialog)),
            DialogHost::Overlay => self.overlay = Some(Overlay::ProviderStudio(Box::new(dialog))),
        }
    }

    fn take_model_catalog_dialog(&mut self) -> Option<(DialogHost, ModelCatalogStudioOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::ModelCatalogStudio(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::ModelCatalogStudio(dialog)) => {
                        Some((DialogHost::Overlay, dialog))
                    }
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    fn restore_model_catalog_dialog(
        &mut self,
        host: DialogHost,
        dialog: ModelCatalogStudioOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::ModelCatalogStudio(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::ModelCatalogStudio(dialog)),
        }
    }

    fn take_timeline_dialog(&mut self) -> Option<(DialogHost, TimelineOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::Timeline(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::Timeline(dialog)) => Some((DialogHost::Overlay, dialog)),
                    overlay => {
                        self.overlay = overlay;
                        None
                    }
                }
            }
        }
    }

    fn restore_timeline_dialog(&mut self, host: DialogHost, dialog: TimelineOverlay) {
        match host {
            DialogHost::Route => self.current_route = Route::Timeline(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::Timeline(dialog)),
        }
    }

    fn open_choice_overlay(&mut self, mut dialog: ChoiceOverlay) {
        Self::refresh_choice_overlay(&mut dialog);
        dialog.selected = Self::preferred_choice_overlay_selection(&dialog);
        self.overlay = Some(Overlay::Choice(dialog));
    }

    fn refresh_choice_overlay(dialog: &mut ChoiceOverlay) {
        let all_items = dialog.meta.all_items.clone();
        refresh_search_list_overlay(dialog, all_items.as_slice());
    }

    fn sync_choice_overlay_input(dialog: &mut ChoiceOverlay, prefer_input_value: bool) {
        Self::refresh_choice_overlay(dialog);
        if prefer_input_value {
            dialog.selected = Self::preferred_choice_overlay_selection(dialog);
        }
    }

    fn preferred_choice_overlay_selection(dialog: &ChoiceOverlay) -> usize {
        let trimmed = dialog.input.text().trim();
        if trimmed.is_empty() {
            return 0;
        }

        let clear_offset = usize::from(dialog.clear_action.is_some());
        if let Some(index) = dialog.items.iter().position(|item| {
            item.value.eq_ignore_ascii_case(trimmed) || item.label.eq_ignore_ascii_case(trimmed)
        }) {
            let custom_offset = usize::from(
                dialog.config.custom_value_enabled
                    && ChoiceCustomValue::search_list_from_input(dialog.input.text(), &dialog.meta)
                        .is_some(),
            );
            return clear_offset + custom_offset + index;
        }

        clear_offset.min(dialog.row_count().saturating_sub(1))
    }

    fn commit_choice_overlay(&mut self, dialog: &mut ChoiceOverlay) -> bool {
        let Some(selection) = dialog.selected_row() else {
            return false;
        };
        match dialog.meta.action.clone() {
            ChoiceOverlayAction::SettingsField(field) => {
                let input = match selection {
                    SearchListRow::Clear(_) => String::new(),
                    SearchListRow::Custom(value) => value.raw,
                    SearchListRow::Item(item) => item.value,
                };
                match parse_settings_field_input(&self.i18n, field, input.as_str()) {
                    Ok(Some(value)) => match self
                        .block_on_async(self.backend.set_config_setting(field.path, value))
                    {
                        Ok(_) => {
                            self.flash_success(settings_path_updated_message(
                                &self.i18n, field.path,
                            ));
                            self.refresh_current_route_after_local_edit();
                            true
                        }
                        Err(error) => {
                            self.flash_error(error);
                            false
                        }
                    },
                    Ok(None) => {
                        match self.block_on_async(self.backend.delete_config_setting(field.path)) {
                            Ok(_) => {
                                self.flash_success(settings_path_cleared_message(
                                    &self.i18n, field.path,
                                ));
                                self.refresh_current_route_after_local_edit();
                                true
                            }
                            Err(error) => {
                                self.flash_error(error);
                                false
                            }
                        }
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::RuntimeSetting(field) => {
                let input = match selection {
                    SearchListRow::Clear(_) => String::new(),
                    SearchListRow::Custom(value) => value.raw,
                    SearchListRow::Item(item) => item.value,
                };
                match self.run_options.apply_runtime_setting_input(
                    &self.i18n,
                    field,
                    input.as_str(),
                ) {
                    Ok(message) => {
                        self.flash_success(message);
                        self.refresh_current_route_after_local_edit();
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::SessionModelVariant(step) => {
                let input = match selection {
                    SearchListRow::Clear(_) => String::new(),
                    SearchListRow::Custom(value) => value.raw,
                    SearchListRow::Item(item) => item.value,
                };
                let field = session_model_variant_field(step);
                match self.run_options.apply_runtime_setting_input(
                    &self.i18n,
                    field,
                    input.as_str(),
                ) {
                    Ok(_) => {
                        self.advance_session_model_variant_step(step);
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::ProviderDefaultWizard(step, draft) => {
                let input = match selection {
                    SearchListRow::Clear(_) => String::new(),
                    SearchListRow::Custom(value) => value.raw,
                    SearchListRow::Item(item) => item.value,
                };
                self.commit_provider_default_wizard_step(step, draft, input)
            }
            ChoiceOverlayAction::ProviderStudioField(field) => {
                let value = match selection {
                    SearchListRow::Clear(_) => String::new(),
                    SearchListRow::Custom(value) => value.raw,
                    SearchListRow::Item(item) => item.value,
                };
                let Some((host, mut parent)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return true;
                };
                match self.commit_provider_studio_field(&mut parent, field, value) {
                    Ok(()) => {
                        self.restore_provider_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_provider_studio_dialog(host, parent);
                        self.flash_error(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::ProviderStudioModelField(field) => {
                let value = match selection {
                    SearchListRow::Clear(_) => String::new(),
                    SearchListRow::Custom(value) => value.raw,
                    SearchListRow::Item(item) => item.value,
                };
                let Some((host, mut parent)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return true;
                };
                match self.commit_provider_studio_model_field(&mut parent, field, value) {
                    Ok(()) => {
                        self.restore_provider_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_provider_studio_dialog(host, parent);
                        self.flash_error(error);
                        false
                    }
                }
            }
            ChoiceOverlayAction::PermissionRuleStudio(field) => {
                let value = match selection {
                    SearchListRow::Clear(_) => String::new(),
                    SearchListRow::Custom(value) => value.raw,
                    SearchListRow::Item(item) => item.value,
                };
                let current_session_id = self.current_or_selected_session_id();
                match &mut self.current_route {
                    Route::PermissionRuleStudio(parent) => {
                        match field {
                            PermissionRuleStudioChoiceField::SubjectKind => {
                                parent.draft.subject_kind = match value.as_str() {
                                    "path_access" => PermissionRuleSubjectKind::PathAccess,
                                    "network_access" => PermissionRuleSubjectKind::NetworkAccess,
                                    _ => PermissionRuleSubjectKind::Tool,
                                };
                            }
                            PermissionRuleStudioChoiceField::PathAccessKind => {
                                if !value.trim().is_empty() {
                                    parent.draft.path_access_kind = value;
                                }
                            }
                            PermissionRuleStudioChoiceField::Scope => {
                                parent.draft.scope = if value.trim().is_empty() {
                                    "workspace".to_string()
                                } else {
                                    value
                                };
                                if parent.draft.scope != "session" {
                                    parent.draft.session_id.clear();
                                } else if parent.draft.session_id.trim().is_empty()
                                    && let Some(session_id) = current_session_id
                                {
                                    parent.draft.session_id = session_id.to_string();
                                }
                            }
                            PermissionRuleStudioChoiceField::Mode => {
                                parent.draft.mode = match value.as_str() {
                                    "allow" => PermissionMode::Allow,
                                    "deny" => PermissionMode::Deny,
                                    _ => PermissionMode::Ask,
                                };
                            }
                        }
                        refresh_permission_rule_studio_dialog(&self.i18n, parent);
                        true
                    }
                    _ => {
                        self.flash_error(ui_text::t(
                            &self.i18n,
                            "flash-permission-rule-context-lost",
                        ));
                        true
                    }
                }
            }
            ChoiceOverlayAction::PermissionStudioMode(target) => {
                let value = match selection {
                    SearchListRow::Clear(_) => String::new(),
                    SearchListRow::Custom(value) => value.raw,
                    SearchListRow::Item(item) => item.value,
                };
                let Some((host, mut parent)) = self.take_permission_studio_dialog() else {
                    self.flash_error(ui_text::t(
                        &self.i18n,
                        "flash-permission-studio-context-lost",
                    ));
                    return true;
                };
                let mut permission = parent.permission.clone();
                let result = apply_permission_studio_mode_input(
                    &self.i18n,
                    &mut permission,
                    &target,
                    value.as_str(),
                )
                .and_then(|_| self.persist_permission_studio(&mut parent, permission));
                match result {
                    Ok(()) => {
                        self.restore_permission_studio_dialog(host, parent);
                        true
                    }
                    Err(error) => {
                        self.restore_permission_studio_dialog(host, parent);
                        self.flash_warning(error);
                        false
                    }
                }
            }
        }
    }

    fn open_settings_field_editor(&mut self, field: SettingsFieldSpec, _return_query: &str) {
        let sources = match self.backend.config_json_sources() {
            Ok(sources) => sources,
            Err(error) => {
                self.flash_error(error.to_string());
                return;
            }
        };
        let file_value = get_json_path(&sources.file, Some(field.path)).unwrap_or(JsonValue::Null);
        let effective_value =
            get_json_path(&sources.effective, Some(field.path)).unwrap_or(JsonValue::Null);
        let prefill = if !file_value.is_null() {
            file_value.clone()
        } else {
            JsonValue::Null
        };
        if let Some(all_items) = self.settings_field_choice_items(field) {
            self.open_choice_overlay(self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    settings_field_edit_title(&self.i18n, field).as_str(),
                ),
                settings_value_edit_prompt(&self.i18n, field, &file_value, &effective_value),
                Editor::from_text(setting_value_input_text(&prefill)),
                all_items,
                ChoiceOverlayAction::SettingsField(field),
                true,
                Self::settings_field_choice_overlay_style(field),
            ));
            return;
        }
        self.overlay = Some(Overlay::SettingsValueEdit(SettingsValueEditOverlay::new(
            settings_edit_title(
                &self.i18n,
                settings_field_edit_title(&self.i18n, field).as_str(),
            ),
            settings_value_edit_prompt(&self.i18n, field, &file_value, &effective_value),
            Editor::from_text(setting_value_input_text(&prefill)),
            field,
        )));
    }

    fn open_provider_default_wizard(&mut self) {
        let provider_id = self
            .backend
            .config_json_sources()
            .ok()
            .and_then(|sources| get_json_path(&sources.effective, Some("providers.default")).ok())
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        let draft = ProviderDefaultWizardDraft {
            provider_id,
            ..Default::default()
        };
        self.open_provider_default_provider_step(draft);
    }

    fn open_provider_default_provider_step(&mut self, draft: ProviderDefaultWizardDraft) -> bool {
        let providers = self.configured_defaultable_provider_summaries();
        if providers.is_empty() {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-default-no-providers",
            ));
            return false;
        }
        let items = providers
            .iter()
            .map(|provider| {
                choice_item(
                    provider.provider_id.clone(),
                    provider_default_route_summary(&self.i18n, provider),
                )
            })
            .collect();
        self.open_provider_default_choice_overlay(
            "overlay-provider-default-provider-title",
            "overlay-provider-default-provider-prompt",
            Editor::from_text(draft.provider_id.clone()),
            items,
            ProviderDefaultWizardStep::Provider,
            draft,
            ChoiceOverlayStyle::SearchableSelect,
        )
    }

    fn open_provider_default_adapter_step(&mut self, draft: ProviderDefaultWizardDraft) -> bool {
        let Some(provider) = self.configured_provider_summary(draft.provider_id.as_str()) else {
            self.flash_warning(self.i18n.text_args(
                "flash-provider-default-provider-missing",
                &crate::fl_args!("provider" => draft.provider_id.clone()),
            ));
            return false;
        };
        let mut items = provider
            .adapters
            .iter()
            .filter(|adapter| adapter.enabled)
            .map(|adapter| {
                choice_item(
                    adapter.adapter_id.clone(),
                    provider_default_adapter_detail(&self.i18n, adapter.configured_model_count),
                )
            })
            .collect::<Vec<_>>();
        if items.is_empty()
            && let Some(adapter) = provider
                .defaults
                .adapter
                .as_deref()
                .filter(|value| !value.trim().is_empty())
        {
            items.push(choice_item(
                adapter.to_owned(),
                ui_text::t(
                    &self.i18n,
                    "settings-provider-default-current-adapter-detail",
                ),
            ));
        }
        let input = draft
            .adapter_id
            .clone()
            .or_else(|| provider.defaults.adapter.clone())
            .unwrap_or_default();
        self.open_provider_default_choice_overlay(
            "overlay-provider-default-adapter-title",
            "overlay-provider-default-adapter-prompt",
            Editor::from_text(input),
            items,
            ProviderDefaultWizardStep::Adapter,
            draft,
            ChoiceOverlayStyle::SearchableSelect,
        )
    }

    fn open_provider_default_model_step(&mut self, draft: ProviderDefaultWizardDraft) -> bool {
        let Some(adapter_id) = draft.adapter_id.as_deref() else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-default-adapter-required",
            ));
            return false;
        };
        let Some(provider) = self.configured_provider_summary(draft.provider_id.as_str()) else {
            self.flash_warning(self.i18n.text_args(
                "flash-provider-default-provider-missing",
                &crate::fl_args!("provider" => draft.provider_id.clone()),
            ));
            return false;
        };
        let items = match self.provider_default_model_choice_items(
            provider.provider_id.as_str(),
            adapter_id,
            provider.defaults.adapter.as_deref(),
        ) {
            Ok(items) => items,
            Err(error) => {
                self.flash_warning(error);
                Vec::new()
            }
        };
        let input = draft
            .model_id
            .clone()
            .or_else(|| {
                (!provider.defaults.model.trim().is_empty())
                    .then(|| provider.defaults.model.clone())
            })
            .unwrap_or_default();
        self.open_provider_default_choice_overlay(
            "overlay-provider-default-model-title",
            "overlay-provider-default-model-prompt",
            Editor::from_text(input),
            items,
            ProviderDefaultWizardStep::Model,
            draft,
            ChoiceOverlayStyle::SearchableSelect,
        )
    }

    fn open_provider_default_thinking_step_or_next(
        &mut self,
        draft: ProviderDefaultWizardDraft,
    ) -> bool {
        match self
            .provider_default_mode_choice_items(&draft, ProviderDefaultWizardStep::ThinkingMode)
        {
            Ok(items) if !items.is_empty() => self.open_provider_default_choice_overlay(
                "overlay-provider-default-thinking-title",
                "overlay-provider-default-thinking-prompt",
                Editor::from_text(
                    draft
                        .thinking_mode
                        .clone()
                        .unwrap_or_else(|| ui_text::t(&self.i18n, "value-default")),
                ),
                items,
                ProviderDefaultWizardStep::ThinkingMode,
                draft,
                ChoiceOverlayStyle::SearchableSelect,
            ),
            Ok(_) => self.open_provider_default_speed_step_or_finish(draft),
            Err(error) => {
                self.flash_warning(error);
                self.open_provider_default_speed_step_or_finish(draft)
            }
        }
    }

    fn open_provider_default_speed_step_or_finish(
        &mut self,
        draft: ProviderDefaultWizardDraft,
    ) -> bool {
        match self.provider_default_mode_choice_items(&draft, ProviderDefaultWizardStep::SpeedMode)
        {
            Ok(items) if !items.is_empty() => self.open_provider_default_choice_overlay(
                "overlay-provider-default-speed-title",
                "overlay-provider-default-speed-prompt",
                Editor::from_text(
                    draft
                        .speed_mode
                        .clone()
                        .unwrap_or_else(|| ui_text::t(&self.i18n, "value-default")),
                ),
                items,
                ProviderDefaultWizardStep::SpeedMode,
                draft,
                ChoiceOverlayStyle::SearchableSelect,
            ),
            Ok(_) => self.finish_provider_default_wizard(draft),
            Err(error) => {
                self.flash_warning(error);
                self.finish_provider_default_wizard(draft)
            }
        }
    }

    fn open_provider_default_choice_overlay(
        &mut self,
        title_key: &str,
        prompt_key: &str,
        input: Editor,
        items: Vec<ChoiceItem>,
        step: ProviderDefaultWizardStep,
        draft: ProviderDefaultWizardDraft,
        style: ChoiceOverlayStyle,
    ) -> bool {
        if items.is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-provider-default-empty-step"));
            return false;
        }
        self.open_choice_overlay(self.build_choice_overlay(
            ui_text::t(&self.i18n, title_key),
            ui_text::t(&self.i18n, prompt_key),
            input,
            items,
            ChoiceOverlayAction::ProviderDefaultWizard(step, draft),
            false,
            style,
        ));
        true
    }

    fn configured_provider_summary(&self, provider_id: &str) -> Option<ProviderSummaryResource> {
        self.configured_defaultable_provider_summaries()
            .into_iter()
            .find(|provider| provider.provider_id == provider_id.trim())
    }

    fn configured_defaultable_provider_summaries(&self) -> Vec<ProviderSummaryResource> {
        let active_provider_ids = self
            .backend
            .list_providers()
            .into_iter()
            .map(|provider| provider.provider_id)
            .collect::<HashSet<_>>();
        self.backend
            .list_configured_providers()
            .into_iter()
            .filter(|provider| active_provider_ids.contains(provider.provider_id.as_str()))
            .collect()
    }

    fn provider_default_model_choice_items(
        &self,
        provider_id: &str,
        adapter_id: &str,
        default_adapter: Option<&str>,
    ) -> UiResult<Vec<ChoiceItem>> {
        let mut items = match self.block_on_async(self.backend.list_provider_models(provider_id)) {
            Ok(models) => models
                .into_iter()
                .filter(|model| {
                    let model_adapter = model
                        .adapter_id
                        .as_ref()
                        .map(ToString::to_string)
                        .or_else(|| default_adapter.map(str::to_owned));
                    model_adapter.as_deref() == Some(adapter_id)
                })
                .map(|model| {
                    choice_item(
                        model.id.to_string(),
                        provider_default_model_detail(&self.i18n, &model),
                    )
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                let fallback = self.configured_provider_model_choice_items(provider_id, adapter_id);
                if fallback.is_empty() {
                    return Err(error);
                }
                fallback
            }
        };

        items.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(dedupe_choice_items(items))
    }

    fn configured_provider_model_choice_items(
        &self,
        provider_id: &str,
        adapter_id: &str,
    ) -> Vec<ChoiceItem> {
        self.backend
            .configured_provider_adapter_models(Some(provider_id))
            .into_iter()
            .find(|models| models.adapter_id == adapter_id)
            .map(|adapter_models| {
                adapter_models
                    .models
                    .into_iter()
                    .map(|model| {
                        choice_item(
                            model.id.to_string(),
                            ui_text::t(&self.i18n, "overlay-provider-studio-configured"),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    fn provider_default_mode_choice_items(
        &self,
        draft: &ProviderDefaultWizardDraft,
        step: ProviderDefaultWizardStep,
    ) -> UiResult<Vec<ChoiceItem>> {
        let Some(model) = provider_default_wizard_model_ref(draft) else {
            return Ok(Vec::new());
        };
        let request = RunOptions {
            model: Some(model),
            ..Default::default()
        };
        let rows = match step {
            ProviderDefaultWizardStep::ThinkingMode => {
                self.backend.runtime_thinking_mode_rows(&request)
            }
            ProviderDefaultWizardStep::SpeedMode => self.backend.runtime_speed_mode_rows(&request),
            _ => Ok(Vec::new()),
        }
        .map_err(|error| error.to_string())?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let mut items = vec![choice_item_with_value(
            ui_text::t(&self.i18n, "value-default"),
            PROVIDER_DEFAULT_WIZARD_INHERIT,
            ui_text::t(&self.i18n, "settings-provider-default-mode-inherit-detail"),
        )];
        items.extend(match step {
            ProviderDefaultWizardStep::ThinkingMode => {
                inspector_rows_to_mode_choice_items(rows, ui_text::thinking_mode_display_value)
            }
            ProviderDefaultWizardStep::SpeedMode => {
                inspector_rows_to_mode_choice_items(rows, ui_text::speed_mode_display_value)
            }
            _ => inspector_rows_to_choice_items(rows),
        });
        Ok(items)
    }

    fn commit_provider_default_wizard_step(
        &mut self,
        step: ProviderDefaultWizardStep,
        mut draft: ProviderDefaultWizardDraft,
        input: String,
    ) -> bool {
        let value = input.trim();
        if value.is_empty() {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-default-selection-required",
            ));
            return false;
        }

        match step {
            ProviderDefaultWizardStep::Provider => {
                draft.provider_id = value.to_owned();
                draft.adapter_id = None;
                draft.model_id = None;
                draft.thinking_mode = None;
                draft.speed_mode = None;
                self.open_provider_default_adapter_step(draft)
            }
            ProviderDefaultWizardStep::Adapter => {
                draft.adapter_id = Some(value.to_owned());
                draft.model_id = None;
                draft.thinking_mode = None;
                draft.speed_mode = None;
                self.open_provider_default_model_step(draft)
            }
            ProviderDefaultWizardStep::Model => {
                draft.model_id = Some(value.to_owned());
                draft.thinking_mode = None;
                draft.speed_mode = None;
                self.open_provider_default_thinking_step_or_next(draft)
            }
            ProviderDefaultWizardStep::ThinkingMode => {
                draft.thinking_mode = provider_default_wizard_optional_value(value);
                self.open_provider_default_speed_step_or_finish(draft)
            }
            ProviderDefaultWizardStep::SpeedMode => {
                draft.speed_mode = provider_default_wizard_optional_value(value);
                self.finish_provider_default_wizard(draft)
            }
        }
    }

    fn finish_provider_default_wizard(&mut self, draft: ProviderDefaultWizardDraft) -> bool {
        match self.persist_provider_default_wizard(draft.clone()) {
            Ok(()) => {
                self.flash_success(self.i18n.text_args(
                    "flash-provider-default-updated",
                    &crate::fl_args!(
                        "provider" => draft.provider_id,
                        "model" => draft.model_id.unwrap_or_default(),
                    ),
                ));
                self.refresh_current_route_after_local_edit();
                true
            }
            Err(error) => {
                self.flash_error(error);
                false
            }
        }
    }

    fn persist_provider_default_wizard(&self, draft: ProviderDefaultWizardDraft) -> UiResult<()> {
        let provider_id = draft.provider_id.trim();
        let adapter_id = draft
            .adapter_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ui_text::t(&self.i18n, "flash-provider-default-adapter-required"))?;
        let model_id = draft
            .model_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ui_text::t(&self.i18n, "flash-provider-default-model-required"))?;
        let sources = self
            .backend
            .config_json_sources()
            .map_err(|error| error.to_string())?;
        let defaults_path = provider_defaults_settings_path(provider_id);
        let mut defaults = get_json_path(&sources.file, Some(defaults_path.as_str()))
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        defaults.insert(
            "adapter".to_string(),
            JsonValue::String(adapter_id.to_owned()),
        );
        defaults.insert("model".to_string(), JsonValue::String(model_id.to_owned()));
        set_optional_string_object_value(
            &mut defaults,
            "thinking_mode",
            draft.thinking_mode.as_deref(),
        );
        set_optional_string_object_value(&mut defaults, "speed_mode", draft.speed_mode.as_deref());

        self.block_on_async(
            self.backend
                .set_config_setting(defaults_path.as_str(), JsonValue::Object(defaults)),
        )?;
        self.block_on_async(self.backend.set_config_setting(
            "providers.default",
            JsonValue::String(provider_id.to_owned()),
        ))?;
        Ok(())
    }

    fn session_model_variant_choice_items(
        &self,
        field: RuntimeSettingSpec,
    ) -> UiResult<Vec<ChoiceItem>> {
        let mut items = match field.id {
            RuntimeSettingId::ThinkingMode => inspector_rows_to_mode_choice_items(
                self.backend
                    .runtime_thinking_mode_rows(&self.run_options.to_request())
                    .map_err(|error| error.to_string())?,
                ui_text::thinking_mode_display_value,
            ),
            RuntimeSettingId::SpeedMode => inspector_rows_to_mode_choice_items(
                self.backend
                    .runtime_speed_mode_rows(&self.run_options.to_request())
                    .map_err(|error| error.to_string())?,
                ui_text::speed_mode_display_value,
            ),
            RuntimeSettingId::Verbosity => self
                .backend
                .runtime_verbosity_values(&self.run_options.to_request())
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(|value| {
                    choice_item(
                        value,
                        runtime_setting_choice_supported_model_detail(&self.i18n),
                    )
                })
                .collect::<Vec<_>>(),
            RuntimeSettingId::ParallelToolCalls
            | RuntimeSettingId::Temperature
            | RuntimeSettingId::MaxOutput
            | RuntimeSettingId::System => Vec::new(),
        };
        if items.len() <= 1 {
            return Ok(Vec::new());
        }
        items.insert(
            0,
            choice_item_with_value(
                ui_text::t(&self.i18n, "value-default"),
                "",
                ui_text::t(&self.i18n, "settings-provider-default-mode-inherit-detail"),
            ),
        );
        Ok(items)
    }

    fn open_session_model_variant_overlay(
        &mut self,
        step: SessionModelVariantStep,
    ) -> UiResult<bool> {
        let field = session_model_variant_field(step);
        let items = self.session_model_variant_choice_items(field)?;
        if items.is_empty() {
            return Ok(false);
        }
        let current_summary = self.run_options.runtime_setting_summary(&self.i18n, field);
        self.open_choice_overlay(
            self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    runtime_setting_display_label(&self.i18n, field).as_str(),
                ),
                [
                    runtime_setting_display_description(&self.i18n, field),
                    self.i18n.text_args(
                        "overlay-runtime-setting-current-value",
                        &crate::fl_args!("value" => current_summary),
                    ),
                ]
                .join("\n"),
                Editor::default(),
                items,
                ChoiceOverlayAction::SessionModelVariant(step),
                false,
                ChoiceOverlayStyle::SelectOnly,
            ),
        );
        Ok(true)
    }

    fn open_session_model_thinking_step_or_next(&mut self) {
        match self.open_session_model_variant_overlay(SessionModelVariantStep::ThinkingMode) {
            Ok(true) => {}
            Ok(false) => self.open_session_model_speed_step_or_next(),
            Err(error) => self.flash_warning(error),
        }
    }

    fn open_session_model_speed_step_or_next(&mut self) {
        match self.open_session_model_variant_overlay(SessionModelVariantStep::SpeedMode) {
            Ok(true) => {}
            Ok(false) => self.open_session_model_verbosity_step_or_finish(),
            Err(error) => self.flash_warning(error),
        }
    }

    fn open_session_model_verbosity_step_or_finish(&mut self) {
        if let Err(error) =
            self.open_session_model_variant_overlay(SessionModelVariantStep::Verbosity)
        {
            self.flash_warning(error);
        }
    }

    fn advance_session_model_variant_step(&mut self, step: SessionModelVariantStep) {
        match step {
            SessionModelVariantStep::ThinkingMode => self.open_session_model_speed_step_or_next(),
            SessionModelVariantStep::SpeedMode => {
                self.open_session_model_verbosity_step_or_finish()
            }
            SessionModelVariantStep::Verbosity => {}
        }
    }

    fn open_runtime_setting_editor(&mut self, field: RuntimeSettingSpec, _return_query: &str) {
        let current_summary = self.run_options.runtime_setting_summary(&self.i18n, field);
        if let Some(all_items) = self.runtime_setting_choice_items(field) {
            self.open_choice_overlay(self.build_choice_overlay(
                settings_edit_title(
                    &self.i18n,
                    runtime_setting_display_label(&self.i18n, field).as_str(),
                ),
                runtime_setting_edit_prompt(&self.i18n, field, current_summary.as_str()),
                Editor::from_text(self.run_options.runtime_setting_input_text(field)),
                all_items,
                ChoiceOverlayAction::RuntimeSetting(field),
                true,
                Self::runtime_setting_choice_overlay_style(field),
            ));
            return;
        }
        self.overlay = Some(Overlay::RuntimeSettingEdit(RuntimeSettingEditOverlay::new(
            settings_edit_title(
                &self.i18n,
                runtime_setting_display_label(&self.i18n, field).as_str(),
            ),
            runtime_setting_edit_prompt(&self.i18n, field, current_summary.as_str()),
            Editor::from_text(self.run_options.runtime_setting_input_text(field)),
            field,
        )));
    }

    fn settings_field_choice_items(&self, field: SettingsFieldSpec) -> Option<Vec<ChoiceItem>> {
        match field.path {
            "providers.default" => {
                let fallback_adapter = settings_choice_adapter_fallback(&self.i18n);
                Some(
                    self.backend
                        .list_providers()
                        .into_iter()
                        .map(|provider| {
                            choice_item(
                                provider.provider_id,
                                settings_choice_default_provider_detail(
                                    &self.i18n,
                                    provider
                                        .defaults
                                        .adapter
                                        .as_deref()
                                        .unwrap_or(fallback_adapter.as_str()),
                                    provider.defaults.model.as_str(),
                                ),
                            )
                        })
                        .collect(),
                )
            }
            "agents.default" => Some(
                self.backend
                    .list_agent_names()
                    .into_iter()
                    .map(|agent| {
                        choice_item(agent, settings_choice_registered_agent_detail(&self.i18n))
                    })
                    .collect(),
            ),
            "ui.locale" => Some(
                SUPPORTED_LOCALES
                    .iter()
                    .map(|(code, detail)| choice_item(*code, *detail))
                    .collect(),
            ),
            "tracing.filter" | "tracing.database" | "tracing.adapter" => Some(
                ["off", "error", "warn", "info", "debug", "trace"]
                    .into_iter()
                    .map(|level| choice_item(level, "log level"))
                    .collect(),
            ),
            _ if matches!(field.kind, SettingsFieldKind::Bool) => Some(boolean_choice_items(
                settings_choice_bool_override_detail(&self.i18n).as_str(),
            )),
            _ => None,
        }
    }

    fn settings_field_choice_overlay_style(field: SettingsFieldSpec) -> ChoiceOverlayStyle {
        match field.path {
            "providers.default" | "agents.default" => ChoiceOverlayStyle::Searchable,
            "ui.locale" | "tracing.filter" | "tracing.database" | "tracing.adapter" => {
                ChoiceOverlayStyle::SelectOnly
            }
            _ if matches!(field.kind, SettingsFieldKind::Bool) => ChoiceOverlayStyle::SelectOnly,
            _ => ChoiceOverlayStyle::Searchable,
        }
    }

    fn runtime_setting_choice_items(
        &mut self,
        field: RuntimeSettingSpec,
    ) -> Option<Vec<ChoiceItem>> {
        match field.id {
            RuntimeSettingId::ThinkingMode => match self
                .backend
                .runtime_thinking_mode_rows(&self.run_options.to_request())
            {
                Ok(rows) => Some(inspector_rows_to_mode_choice_items(
                    rows,
                    ui_text::thinking_mode_display_value,
                )),
                Err(error) => {
                    self.flash_warning(error.to_string());
                    Some(Vec::new())
                }
            },
            RuntimeSettingId::SpeedMode => match self
                .backend
                .runtime_speed_mode_rows(&self.run_options.to_request())
            {
                Ok(rows) => Some(inspector_rows_to_mode_choice_items(
                    rows,
                    ui_text::speed_mode_display_value,
                )),
                Err(error) => {
                    self.flash_warning(error.to_string());
                    Some(Vec::new())
                }
            },
            RuntimeSettingId::Verbosity => match self
                .backend
                .runtime_verbosity_values(&self.run_options.to_request())
            {
                Ok(values) => Some(
                    values
                        .into_iter()
                        .map(|value| {
                            choice_item(
                                value,
                                runtime_setting_choice_supported_model_detail(&self.i18n),
                            )
                        })
                        .collect(),
                ),
                Err(error) => {
                    self.flash_warning(error.to_string());
                    Some(Vec::new())
                }
            },
            RuntimeSettingId::ParallelToolCalls => Some(boolean_choice_items(
                runtime_setting_choice_parallel_detail(&self.i18n).as_str(),
            )),
            RuntimeSettingId::Temperature
            | RuntimeSettingId::MaxOutput
            | RuntimeSettingId::System => None,
        }
    }

    fn runtime_setting_choice_overlay_style(field: RuntimeSettingSpec) -> ChoiceOverlayStyle {
        match field.id {
            RuntimeSettingId::ParallelToolCalls => ChoiceOverlayStyle::SelectOnly,
            RuntimeSettingId::ThinkingMode
            | RuntimeSettingId::SpeedMode
            | RuntimeSettingId::Verbosity
            | RuntimeSettingId::Temperature
            | RuntimeSettingId::MaxOutput
            | RuntimeSettingId::System => ChoiceOverlayStyle::Searchable,
        }
    }

    fn provider_studio_field_choice_items(
        &self,
        dialog: &ProviderStudioOverlay,
        field: ProviderStudioField,
    ) -> Option<Vec<ChoiceItem>> {
        match field {
            ProviderStudioField::AuthMode => Some(vec![
                choice_item(
                    "none",
                    ui_text::t(&self.i18n, "provider-auth-mode-none-detail"),
                ),
                choice_item(
                    "api",
                    ui_text::t(&self.i18n, "provider-auth-mode-api-detail"),
                ),
                choice_item(
                    "credential",
                    ui_text::t(&self.i18n, "provider-auth-mode-credential-detail"),
                ),
            ]),
            ProviderStudioField::AuthSubtype => match dialog.draft.auth_kind {
                ProviderDraftAuthKind::ApiPending
                | ProviderDraftAuthKind::Api
                | ProviderDraftAuthKind::ClineApi
                | ProviderDraftAuthKind::Gitlab
                | ProviderDraftAuthKind::BedrockSigv4 => Some(vec![
                    choice_item(
                        "custom",
                        ui_text::t(&self.i18n, "provider-auth-subtype-custom-detail"),
                    ),
                    choice_item(
                        "cline_api",
                        ui_text::t(&self.i18n, "provider-auth-subtype-cline-api-detail"),
                    ),
                    choice_item(
                        "gitlab_api",
                        ui_text::t(&self.i18n, "provider-auth-subtype-gitlab-api-detail"),
                    ),
                    choice_item(
                        "bedrock_sigv4",
                        ui_text::t(&self.i18n, "provider-auth-subtype-bedrock-detail"),
                    ),
                ]),
                ProviderDraftAuthKind::Credential(_) => Some(vec![
                    choice_item(
                        "openai_chatgpt",
                        ui_text::t(&self.i18n, "provider-issuer-openai-chatgpt-detail"),
                    ),
                    choice_item(
                        "github_copilot",
                        ui_text::t(&self.i18n, "provider-issuer-github-copilot-detail"),
                    ),
                    choice_item(
                        "gitlab",
                        ui_text::t(&self.i18n, "provider-issuer-gitlab-detail"),
                    ),
                    choice_item(
                        "google_adc",
                        ui_text::t(&self.i18n, "provider-issuer-google-adc-detail"),
                    ),
                    choice_item(
                        "sap_ai_core",
                        ui_text::t(&self.i18n, "provider-issuer-sap-ai-core-detail"),
                    ),
                ]),
                ProviderDraftAuthKind::Unset | ProviderDraftAuthKind::None => None,
            },
            ProviderStudioField::AuthLoginMethod => {
                let items = match dialog.draft.auth_kind.credential_issuer() {
                    Some(CredentialIssuer::OpenaiChatgpt) => vec![
                        choice_item(
                            "device",
                            ui_text::t(&self.i18n, "provider-auth-login-kind-device-detail"),
                        ),
                        choice_item(
                            "browser",
                            ui_text::t(&self.i18n, "provider-auth-login-kind-browser-detail"),
                        ),
                    ],
                    Some(CredentialIssuer::GithubCopilot) => vec![choice_item(
                        "device",
                        ui_text::t(&self.i18n, "provider-auth-login-kind-device-detail"),
                    )],
                    Some(CredentialIssuer::Gitlab) => vec![choice_item(
                        "browser",
                        ui_text::t(&self.i18n, "provider-auth-login-kind-browser-detail"),
                    )],
                    Some(CredentialIssuer::GoogleAdc | CredentialIssuer::SapAiCore) | None => {
                        Vec::new()
                    }
                };
                (!items.is_empty()).then_some(items)
            }
            ProviderStudioField::InstanceUrl => Some(vec![choice_item(
                "https://gitlab.com",
                ui_text::t(&self.i18n, "provider-instance-url-gitlab-detail"),
            )]),
            ProviderStudioField::RedirectUri => Some(vec![choice_item(
                "http://localhost:1455/auth/callback",
                ui_text::t(&self.i18n, "provider-redirect-local-copy-detail"),
            )]),
            ProviderStudioField::Region => Some(
                AWS_REGION_CHOICES
                    .iter()
                    .map(|region| {
                        choice_item(
                            *region,
                            ui_text::t(&self.i18n, "provider-region-choice-detail"),
                        )
                    })
                    .collect(),
            ),
            ProviderStudioField::Profile => Some(provider_studio_profile_choice_items(
                &self.i18n,
                &self.backend,
            )),
            ProviderStudioField::ApiKeySource => Some(vec![
                choice_item(
                    "inline",
                    ui_text::t(&self.i18n, "provider-api-key-source-inline-detail"),
                ),
                choice_item(
                    "env",
                    ui_text::t(&self.i18n, "provider-api-key-source-env-detail"),
                ),
            ]),
            ProviderStudioField::ApiKeyValue
                if matches!(
                    dialog.draft.auth.secret_source_kind,
                    ProviderDraftSecretSourceKind::Env
                ) =>
            {
                Some(provider_studio_api_key_env_choice_items(&self.i18n))
            }
            ProviderStudioField::ApiKeyValue => None,
            ProviderStudioField::ServiceKeyEnv => Some(vec![choice_item(
                "AICORE_SERVICE_KEY",
                ui_text::t(&self.i18n, "provider-service-key-env-detail"),
            )]),
            ProviderStudioField::DefaultAdapter => Some(
                dialog
                    .adapter_candidate_ids
                    .iter()
                    .map(|adapter_id| {
                        let detail = provider_studio_adapter_rule(dialog, adapter_id.as_str())
                            .map(|rule| {
                                let mut parts =
                                    vec![provider_studio_adapter_rule_detail(&self.i18n, rule)];
                                if dialog.configured_adapter_ids.contains(adapter_id) {
                                    parts.push(ui_text::t(
                                        &self.i18n,
                                        "overlay-provider-studio-configured",
                                    ));
                                }
                                join_inline_segments(parts)
                            })
                            .unwrap_or_else(|| {
                                if dialog.configured_adapter_ids.contains(adapter_id) {
                                    ui_text::t(
                                        &self.i18n,
                                        "overlay-provider-studio-configured-disk",
                                    )
                                } else {
                                    ui_text::t(&self.i18n, "overlay-provider-studio-not-supported")
                                }
                            });
                        choice_item(adapter_id.clone(), detail)
                    })
                    .collect(),
            ),
            ProviderStudioField::DefaultModel => Some(provider_studio_default_model_choice_items(
                &self.i18n, dialog,
            )),
            _ => None,
        }
    }

    fn provider_studio_field_choice_overlay_style(
        field: ProviderStudioField,
    ) -> ChoiceOverlayStyle {
        match field {
            ProviderStudioField::AuthMode
            | ProviderStudioField::AuthSubtype
            | ProviderStudioField::AuthLoginMethod
            | ProviderStudioField::InstanceUrl
            | ProviderStudioField::RedirectUri
            | ProviderStudioField::ApiKeySource
            | ProviderStudioField::ServiceKeyEnv => ChoiceOverlayStyle::SelectOnly,
            ProviderStudioField::Region
            | ProviderStudioField::Profile
            | ProviderStudioField::DefaultAdapter
            | ProviderStudioField::DefaultModel => ChoiceOverlayStyle::Searchable,
            _ => ChoiceOverlayStyle::Searchable,
        }
    }

    fn provider_model_config_field_choice_items(
        &self,
        dialog: &ProviderStudioOverlay,
        field: ProviderModelConfigField,
    ) -> Option<Vec<ChoiceItem>> {
        match field {
            ProviderModelConfigField::Enabled => Some(boolean_choice_items(
                ui_text::t(&self.i18n, "provider-model-enabled-detail").as_str(),
            )),
            ProviderModelConfigField::Lifecycle => Some(
                [
                    "active",
                    "preview",
                    "beta",
                    "alpha",
                    "experimental",
                    "deprecated",
                ]
                .into_iter()
                .map(|value| {
                    choice_item(
                        value,
                        ui_text::t(&self.i18n, "provider-model-lifecycle-detail"),
                    )
                })
                .collect(),
            ),
            ProviderModelConfigField::NativeTools => {
                let mut items = vec![choice_item(
                    ProviderNativeToolsPreset::Disabled.token(),
                    ui_text::t(&self.i18n, "provider-native-tools-disabled-detail"),
                )];
                if let Some(adapter_id) = dialog
                    .model_page
                    .as_ref()
                    .map(|page| page.adapter_id.as_str())
                    && let Some(preset) =
                        provider_native_tools_available_preset_for_adapter(adapter_id)
                {
                    let detail_key = match preset {
                        ProviderNativeToolsPreset::OpenAiHostedDefaults => {
                            "provider-native-tools-openai-detail"
                        }
                        ProviderNativeToolsPreset::AnthropicHostedDefaults => {
                            "provider-native-tools-anthropic-detail"
                        }
                        ProviderNativeToolsPreset::GeminiHostedDefaults => {
                            "provider-native-tools-gemini-detail"
                        }
                        ProviderNativeToolsPreset::Disabled | ProviderNativeToolsPreset::Custom => {
                            unreachable!()
                        }
                    };
                    items.push(choice_item(
                        preset.token(),
                        ui_text::t(&self.i18n, detail_key),
                    ));
                }
                if dialog.model_page.as_ref().is_some_and(|page| {
                    page.draft.native_tools_preset == ProviderNativeToolsPreset::Custom
                }) {
                    items.push(choice_item(
                        ProviderNativeToolsPreset::Custom.token(),
                        ui_text::t(&self.i18n, "provider-native-tools-custom-detail"),
                    ));
                }
                Some(items)
            }
            ProviderModelConfigField::ModelId
            | ProviderModelConfigField::DisplayName
            | ProviderModelConfigField::ContextWindowTokens
            | ProviderModelConfigField::MaxInputTokens
            | ProviderModelConfigField::MaxOutputTokens
            | ProviderModelConfigField::InputModalities
            | ProviderModelConfigField::Features
            | ProviderModelConfigField::OutputModalities
            | ProviderModelConfigField::Description => None,
        }
    }

    fn provider_model_config_field_choice_overlay_style(
        field: ProviderModelConfigField,
    ) -> ChoiceOverlayStyle {
        match field {
            ProviderModelConfigField::Enabled | ProviderModelConfigField::NativeTools => {
                ChoiceOverlayStyle::SelectOnly
            }
            ProviderModelConfigField::Lifecycle => ChoiceOverlayStyle::Searchable,
            ProviderModelConfigField::ModelId
            | ProviderModelConfigField::DisplayName
            | ProviderModelConfigField::ContextWindowTokens
            | ProviderModelConfigField::MaxInputTokens
            | ProviderModelConfigField::MaxOutputTokens
            | ProviderModelConfigField::InputModalities
            | ProviderModelConfigField::Features
            | ProviderModelConfigField::OutputModalities
            | ProviderModelConfigField::Description => ChoiceOverlayStyle::Searchable,
        }
    }

    fn open_runtime_config_in_editor(&mut self) {
        let path = self.backend.config_path();
        if !path.exists() {
            if let Some(parent) = path.parent()
                && let Err(error) = fs::create_dir_all(parent)
            {
                self.flash_error(self.i18n.text_args(
                    "flash-config-dir-prepare-failed",
                    &crate::fl_args!(
                        "path" => parent.display().to_string(),
                        "error" => error.to_string(),
                    ),
                ));
                return;
            }
            if let Err(error) = fs::write(&path, "") {
                self.flash_error(self.i18n.text_args(
                    "flash-config-file-create-failed",
                    &crate::fl_args!(
                        "path" => path.display().to_string(),
                        "error" => error.to_string(),
                    ),
                ));
                return;
            }
        }
        self.pending_ui_action = Some(UiAction::OpenPath { path });
    }

    fn open_agent_profile_source(&mut self, profile: &AgentProfile) {
        match self.agent_profile_storage(profile) {
            AgentProfileStorage::Markdown => {
                if let Some(path) = profile.source_path.clone() {
                    self.pending_ui_action = Some(UiAction::OpenPath { path });
                }
            }
            AgentProfileStorage::Config => self.open_runtime_config_in_editor(),
            AgentProfileStorage::BuiltIn => {
                self.flash_info(ui_text::t(&self.i18n, "flash-agent-built-in-no-source"));
            }
            AgentProfileStorage::Runtime => {
                self.flash_info(ui_text::t(&self.i18n, "flash-agent-runtime-no-source"));
            }
        }
    }

    fn open_inspector_picker(
        &mut self,
        title: String,
        prompt: String,
        query: &str,
        rows: Vec<crate::backend::InspectorRow>,
    ) {
        let overlay = self.build_picker_overlay(
            title,
            prompt,
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            Editor::from_text(query.to_string()),
            rows.into_iter()
                .map(|row| PickerItem {
                    label: row.label,
                    detail: row.detail,
                    value: PickerValue::Inspector,
                })
                .collect(),
            PickerKind::Inspector,
            false,
        );
        self.current_route = Route::Picker(overlay);
    }

    fn open_permission_rule_picker(&mut self, query: &str) {
        match self.build_permission_rule_picker_overlay(query) {
            Ok(overlay) => self.current_route = Route::Picker(overlay),
            Err(error) => self.flash_error(error),
        }
    }

    fn build_permission_rule_picker_overlay(&self, query: &str) -> UiResult<PickerOverlay> {
        let rules = self
            .block_on_async(self.backend.list_permission_rules())
            .map_err(|error| error.to_string())?;
        let mut all_items = vec![PickerItem {
            label: ui_text::t(&self.i18n, "permission-rule-create-label"),
            detail: ui_text::t(&self.i18n, "permission-rule-create-detail"),
            value: PickerValue::PermissionRuleCreate,
        }];
        all_items.extend(rules.into_iter().map(|rule| PickerItem {
            label: permission_rule_label(&self.i18n, &rule),
            detail: permission_rule_detail(&self.i18n, &rule),
            value: PickerValue::PermissionRule(Box::new(rule)),
        }));
        let overlay = self.build_picker_overlay(
            ui_text::t(&self.i18n, "overlay-permission-rules-title"),
            ui_text::t(&self.i18n, "overlay-permission-rules-prompt"),
            ui_text::t(&self.i18n, "overlay-permission-rules-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            Editor::from_text(query.to_string()),
            all_items,
            PickerKind::PermissionRules,
            false,
        );
        Ok(overlay)
    }

    fn build_permission_rule_studio_overlay(
        &self,
        rule_id: Option<i64>,
        title: String,
        draft: PermissionRuleDraft,
        preferred_item_label: Option<&str>,
    ) -> PermissionRuleStudioOverlay {
        let items = permission_rule_studio_items(&self.i18n, &draft, rule_id);
        let selected = preferred_item_label
            .and_then(|label| items.iter().position(|item| item.label == label))
            .unwrap_or(0);
        let footer = ui_text::t(&self.i18n, "overlay-permission-rule-studio-footer");
        PermissionRuleStudioOverlay {
            rule_id,
            draft,
            workbench: ListWorkbenchState::new(
                title,
                footer,
                SelectableListState::new(items, selected),
            ),
        }
    }

    fn open_current_session_permission_studio(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.route_stack.clear();
        self.open_session_permission_studio(session_id);
    }

    fn open_permission_rule_studio(
        &mut self,
        rule: Option<&PermissionRuleResource>,
        draft_override: Option<PermissionRuleDraft>,
    ) {
        let (rule_id, title, draft) = match (rule, draft_override) {
            (_, Some(draft)) => (
                rule.map(|rule| rule.id),
                ui_text::t(&self.i18n, "overlay-permission-rule-workbench-title"),
                draft,
            ),
            (Some(rule), None) => (
                Some(rule.id),
                format!(
                    "{} · {}",
                    ui_text::t(&self.i18n, "overlay-permission-rule-workbench-title"),
                    permission_rule_label(&self.i18n, rule)
                ),
                permission_rule_draft_from_resource(rule),
            ),
            (None, None) => (
                None,
                ui_text::t(&self.i18n, "overlay-permission-rule-workbench-title"),
                PermissionRuleDraft::default(),
            ),
        };
        self.current_route = Route::PermissionRuleStudio(
            self.build_permission_rule_studio_overlay(rule_id, title, draft, None),
        );
    }

    fn refresh_permission_rule_studio(&mut self, dialog: &mut PermissionRuleStudioOverlay) {
        refresh_permission_rule_studio_dialog(&self.i18n, dialog);
    }

    fn open_permission_rule_editor_from_request(&mut self, request: &PermissionRequest) {
        let draft = permission_rule_draft_from_request(request);
        let input = Editor::from_text(render_permission_rule_draft(&draft));
        self.overlay = Some(Overlay::PermissionRuleEdit(PermissionRuleEditOverlay {
            rule_id: None,
            state: InputDialogState::new(
                ui_text::t(&self.i18n, "overlay-permission-rule-create-title"),
                ui_text::t(&self.i18n, "overlay-permission-rule-prompt"),
                input,
                (),
            ),
            return_query: String::new(),
            return_overlay: None,
        }));
    }

    fn open_revoke_permission_rule_confirm(
        &mut self,
        rule: &PermissionRuleResource,
        return_query: &str,
    ) {
        let label = permission_rule_label(&self.i18n, rule);
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-permission-rule-delete-title"),
            vec![self.i18n.text_args(
                "overlay-permission-rule-delete-body",
                &crate::fl_args!("name" => label.clone()),
            )],
            ConfirmAction::RevokePermissionRule {
                rule_id: rule.id,
                label,
                return_query: return_query.to_string(),
            },
        )));
    }

    fn open_snapshot_remove_confirm(&mut self, session_id: i64, discard_changes: bool) {
        let mut body_lines = vec![ui_text::t(&self.i18n, "overlay-snapshot-remove-body")];
        if discard_changes {
            body_lines.push(ui_text::t(&self.i18n, "overlay-snapshot-remove-force"));
        }
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-snapshot-remove-title"),
            body_lines,
            ConfirmAction::ExitSnapshot {
                session_id,
                discard_changes,
            },
        )));
    }

    fn open_command_palette(&mut self) {
        let mut all_items = commands::COMMANDS
            .iter()
            .map(|spec| PickerItem {
                label: spec.invocation(),
                detail: ui_text::t(&self.i18n, spec.summary_key),
                value: PickerValue::Command(spec),
            })
            .collect::<Vec<_>>();
        all_items.extend(
            self.runtime_tool_command_rows()
                .into_iter()
                .map(|entry| PickerItem {
                    label: format!("/{}", entry.label),
                    detail: entry.detail,
                    value: PickerValue::RuntimeTool(entry.label),
                }),
        );
        let overlay = self.build_picker_overlay(
            ui_text::t(&self.i18n, "overlay-commands-title"),
            ui_text::t(&self.i18n, "overlay-commands-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-empty"),
            Editor::default(),
            all_items,
            PickerKind::Commands,
            false,
        );
        self.current_route = Route::Picker(overlay);
    }

    fn runtime_tool_command_rows(&self) -> Vec<crate::backend::InspectorRow> {
        self.backend
            .runtime_tool_rows()
            .into_iter()
            .filter(|entry| commands::find_command(entry.label.as_str()).is_none())
            .collect()
    }

    fn open_resume_session_picker(&mut self) {
        self.open_resume_session_picker_with_query("");
    }

    fn open_resume_session_picker_with_query(&mut self, query: &str) {
        let input = Editor::from_text(query.trim().to_string());
        let scope_session_id = (self.sessions.view_mode == SessionViewMode::Subtree)
            .then(|| self.current_or_selected_session_id())
            .flatten();
        let dialog =
            self.build_session_search_overlay(input, self.sessions.view_mode, scope_session_id);
        match dialog.meta.mode {
            SessionViewMode::Subtree => {
                let Some(session_id) = dialog.meta.scope_session_id else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
                    return;
                };
                self.request_session_search_subtree(
                    session_id,
                    dialog.input.text().trim().to_string(),
                );
            }
            SessionViewMode::All | SessionViewMode::Roots => {
                self.request_session_search_page(
                    dialog.meta.mode,
                    dialog.input.text().trim().to_string(),
                    0,
                    None,
                );
            }
        }
        self.current_route = Route::SessionSearch(dialog);
    }

    fn open_lineage_picker(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let dialog = self.build_picker_overlay(
            self.i18n.text_args(
                "overlay-lineage-title",
                &crate::fl_args!("session" => session_id),
            ),
            ui_text::t(&self.i18n, "overlay-lineage-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Vec::new(),
            PickerKind::Lineage { session_id },
            true,
        );
        self.current_route = Route::Picker(dialog);
        self.request_lineage(session_id);
    }

    fn open_rewind_messages_picker(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        if self.prompt_for_pending_interactive_on_session(session_id) {
            return;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
            return;
        }
        let dialog = self.build_picker_overlay(
            self.i18n.text_args(
                "overlay-rewind-title",
                &crate::fl_args!("session" => session_id),
            ),
            ui_text::t(&self.i18n, "overlay-rewind-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Vec::new(),
            PickerKind::RewindMessages { session_id },
            true,
        );
        self.current_route = Route::Picker(dialog);
        self.request_rewind_messages(session_id);
    }

    fn open_provider_picker(&mut self, purpose: ProviderPickerPurpose) {
        let dialog = self.build_picker_overlay(
            ui_text::t(&self.i18n, "overlay-providers-title"),
            ui_text::t(&self.i18n, "overlay-providers-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Vec::new(),
            PickerKind::Providers(purpose),
            true,
        );
        self.current_route = Route::Picker(dialog);
        self.request_providers(purpose);
    }

    fn open_provider_list(&mut self, query: &str) {
        let dialog = self.build_provider_list_overlay(query, true);
        self.current_route = Route::Picker(dialog);
        self.request_providers(ProviderPickerPurpose::Configure);
    }

    fn build_provider_list_overlay(&self, query: &str, loading: bool) -> PickerOverlay {
        let all_items =
            if loading {
                Vec::new()
            } else {
                let mut items = vec![provider_list_create_item(&self.i18n)];
                items.extend(self.backend.list_configured_providers().into_iter().map(
                    |provider| PickerItem {
                        label: provider.provider_id.clone(),
                        detail: i18n_provider_list_detail(&self.i18n, &provider),
                        value: PickerValue::Provider(provider),
                    },
                ));
                items
            };
        self.build_picker_overlay(
            ui_text::t(&self.i18n, "overlay-provider-list-title"),
            ui_text::t(&self.i18n, "overlay-provider-list-prompt"),
            ui_text::t(&self.i18n, "overlay-provider-list-footer"),
            ui_text::t(
                &self.i18n,
                if loading {
                    "overlay-picker-loading"
                } else {
                    "overlay-picker-empty"
                },
            ),
            Editor::from_text(query.trim().to_string()),
            all_items,
            PickerKind::Providers(ProviderPickerPurpose::Configure),
            loading,
        )
    }

    fn open_agent_list(&mut self, query: &str) {
        let dialog = self.build_agent_list_overlay(query, true);
        self.current_route = Route::Picker(dialog);
        self.request_agent_list();
    }

    fn open_agent_create_overlay(&mut self) {
        self.overlay = Some(Overlay::AgentCreate(self.build_agent_create_overlay()));
    }

    fn create_agent_from_list(&mut self, input: &str) -> bool {
        let agent_name = input.trim();
        if agent_name.is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-agent-create-name-required"));
            return false;
        }
        if self
            .backend
            .list_agent_descriptors()
            .iter()
            .any(|agent| agent.name == agent_name)
        {
            self.flash_warning(self.i18n.text_args(
                "flash-agent-create-name-exists",
                &crate::fl_args!("name" => agent_name),
            ));
            return false;
        }

        let path = format!("agents.{}", quoted_settings_segment(agent_name));
        match self.block_on_async(self.backend.set_config_setting(path.as_str(), json!({}))) {
            Ok(_) => {
                self.flash_success(self.i18n.text_args(
                    "flash-agent-created",
                    &crate::fl_args!("name" => agent_name),
                ));
                if matches!(&self.current_route, Route::Picker(dialog) if matches!(dialog.meta.kind, PickerKind::Agents))
                {
                    self.route_stack.push(self.current_route.clone());
                }
                self.open_agent_studio(agent_name);
                true
            }
            Err(error) => {
                self.flash_error(error.to_string());
                false
            }
        }
    }

    fn build_agent_list_overlay(&self, query: &str, loading: bool) -> PickerOverlay {
        let all_items = if loading {
            Vec::new()
        } else {
            agent_list_items(
                &self.i18n,
                self.backend.list_agent_descriptors(),
                self.backend.default_agent_name().as_deref(),
                &self.backend.config_agent_names(),
            )
        };
        self.build_picker_overlay(
            ui_text::t(&self.i18n, "overlay-agent-list-title"),
            ui_text::t(&self.i18n, "overlay-agent-list-prompt"),
            ui_text::t(&self.i18n, "overlay-agent-list-footer"),
            ui_text::t(
                &self.i18n,
                if loading {
                    "overlay-picker-loading"
                } else {
                    "overlay-picker-empty"
                },
            ),
            Editor::from_text(query.trim().to_string()),
            all_items,
            PickerKind::Agents,
            loading,
        )
    }

    fn open_session_model_chooser(&mut self) {
        let mut dialog = self.build_session_model_chooser_overlay();
        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match self.session_model_chooser_items() {
            Ok(items) => {
                dialog.meta.all_items = items;
                let current_model = self.current_session_model_ref();
                Self::refresh_session_model_chooser_overlay(
                    &mut dialog,
                    true,
                    current_model.as_ref(),
                );
            }
            Err(error) => self.flash_error(error),
        }
        self.current_route = Route::SessionModelChooser(dialog);
    }

    fn open_provider_studio(&mut self, initial_provider: Option<&str>) {
        let providers = self.backend.list_configured_providers();
        let provider_rows = provider_studio_provider_rows(&self.i18n, providers.as_slice());
        let selected_provider = initial_provider
            .and_then(|provider_id| {
                provider_rows
                    .iter()
                    .position(|row| row.provider_id.as_deref() == Some(provider_id.trim()))
            })
            .unwrap_or(0);
        let draft_prefill = initial_provider
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|provider_id| {
                !providers
                    .iter()
                    .any(|provider| provider.provider_id == *provider_id)
            })
            .map(str::to_owned);
        let mut overlay = ProviderStudioOverlay {
            title: ui_text::t(&self.i18n, "overlay-provider-studio-title"),
            footer: ui_text::t(&self.i18n, "overlay-provider-studio-footer"),
            show_provider_list: false,
            providers: SelectableListState::new(provider_rows, selected_provider),
            selection: DashboardSelectionState::new(
                [
                    ProviderStudioFocus::Fields,
                    ProviderStudioFocus::Adapters,
                    ProviderStudioFocus::Models,
                ],
                ProviderStudioFocus::Fields,
                0,
                0,
                0,
            ),
            draft: self
                .backend
                .provider_config_draft(None)
                .unwrap_or_else(|_| {
                    let mut draft = ProviderConfigDraft {
                        source_provider_id: None,
                        provider_id: String::new(),
                        auth_kind: ProviderDraftAuthKind::Unset,
                        auth: Default::default(),
                        credential_drafts: Default::default(),
                        default_adapter: String::new(),
                        default_model: String::new(),
                    };
                    draft.normalize_shape();
                    draft
                }),
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: Vec::new(),
            adapter_selection_touched: false,
            selected_adapter_ids: BTreeSet::new(),
            selected_model_keys: BTreeSet::new(),
            catalog_matches: BTreeMap::new(),
            listing_adapter_models: false,
            saving: false,
            pending_adapter_models_key: None,
            pending_auth_key: None,
            next_auth_poll_at: None,
            detail_page: None,
            model_page: None,
            editor: None,
        };
        let selected_id = overlay
            .providers
            .selected_item()
            .and_then(|row| row.provider_id.clone());
        self.load_provider_studio_draft(&mut overlay, selected_id.as_deref(), draft_prefill);
        self.current_route = Route::ProviderStudio(Box::new(overlay));
    }

    fn load_provider_studio_draft(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        provider_id: Option<&str>,
        prefill_new_id: Option<String>,
    ) {
        match self.backend.provider_config_draft(provider_id) {
            Ok(mut draft) => {
                if provider_id.is_none()
                    && let Some(prefill) = prefill_new_id
                {
                    draft.provider_id = prefill;
                    draft.normalize_shape();
                }
                dialog.draft = draft;
                dialog.selection.set_top_selected(0);
                dialog.adapter_models =
                    self.backend.configured_provider_adapter_models(provider_id);
                let configured_adapter_ids = provider_id
                    .and_then(|id| {
                        self.backend
                            .list_configured_providers()
                            .into_iter()
                            .find(|provider| provider.provider_id == id)
                    })
                    .map(|provider| {
                        provider
                            .adapters
                            .into_iter()
                            .map(|adapter| adapter.adapter_id)
                            .collect::<BTreeSet<_>>()
                    })
                    .unwrap_or_default();
                dialog.configured_adapter_ids = configured_adapter_ids.clone();
                dialog.selection.set_left_selected(0);
                dialog.selection.set_right_selected(0);
                dialog.pending_adapter_models_key = None;
                dialog.pending_auth_key = None;
                dialog.detail_page = None;
                dialog.model_page = None;
                dialog.listing_adapter_models = false;
                dialog.adapter_selection_touched = provider_id.is_some();
                dialog.selected_adapter_ids = configured_adapter_ids;
                dialog.selected_model_keys = self
                    .backend
                    .configured_provider_model_routes(provider_id)
                    .into_iter()
                    .map(|(adapter_id, model_id)| {
                        provider_studio_model_key(adapter_id.as_str(), model_id.as_str())
                    })
                    .collect();
                dialog.catalog_matches.clear();
                self.reload_provider_studio_catalog_matches(dialog);
                self.sync_provider_studio_shape(dialog);
                if let Some(index) = dialog
                    .adapter_candidate_ids
                    .iter()
                    .position(|candidate| candidate == dialog.draft.default_adapter.trim())
                {
                    dialog.selection.set_left_selected(index);
                } else if let Some(first_selected) = dialog
                    .adapter_candidate_ids
                    .iter()
                    .position(|candidate| dialog.selected_adapter_ids.contains(candidate.as_str()))
                {
                    dialog.selection.set_left_selected(first_selected);
                }
            }
            Err(error) => self.flash_error(error.to_string()),
        }
    }

    fn open_model_catalog_studio(&mut self) {
        let dialog = ModelCatalogStudioOverlay {
            query: String::new(),
            summary: ModelCatalogResponse {
                refreshing: false,
                last_refresh_at: None,
                last_successful_source: None,
                last_error: None,
                model_count: 0,
            },
            total: 0,
            offset: 0,
            limit: 50,
            loading: true,
            workbench: ListWorkbenchState::new(
                ui_text::t(&self.i18n, "overlay-model-catalog-title"),
                ui_text::t(&self.i18n, "overlay-model-catalog-footer"),
                SelectableListState::new(Vec::new(), 0),
            ),
        };
        self.request_model_catalog_page(String::new(), 0);
        self.current_route = Route::ModelCatalogStudio(dialog.clone());
    }

    fn request_model_catalog_page(&mut self, query: String, offset: usize) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_model_catalog_models(query.as_str(), offset, 50)
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ModelCatalogLoaded {
                query,
                offset,
                result,
            });
        });
    }

    fn request_model_catalog_refresh(&mut self) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .refresh_model_catalog()
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ModelCatalogRefreshed { result });
        });
    }

    fn request_provider_studio_adapter_models(&mut self, dialog: &mut ProviderStudioOverlay) {
        let adapter_ids = provider_studio_request_adapter_ids(dialog);
        if adapter_ids.is_empty() {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-required",
            ));
            return;
        }
        if !provider_studio_can_request_adapter_models(dialog) {
            self.flash_warning(provider_studio_live_listing_unavailable_message(
                &self.i18n,
                &dialog.draft.auth_kind,
            ));
            return;
        }

        if dialog.draft.auth_kind.supports_draft_model_listing() {
            let unsupported = adapter_ids
                .iter()
                .filter(|adapter_id| {
                    provider_studio_adapter_rule(dialog, adapter_id.as_str())
                        .map(|rule| !rule.supports_draft_model_listing)
                        .unwrap_or(true)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !unsupported.is_empty() {
                self.flash_error(provider_studio_draft_listing_unsupported_message(
                    &self.i18n,
                    unsupported.as_slice(),
                ));
                return;
            }
        }

        let request_key = provider_studio_request_key(&dialog.draft, &adapter_ids);
        dialog.pending_adapter_models_key = Some(request_key.clone());
        dialog.listing_adapter_models = true;
        let backend = self.backend.clone();
        let i18n = self.i18n.clone();
        let tx = self.tx.clone();
        let draft = dialog.draft.clone();
        tokio::spawn(async move {
            let result = if draft.auth_kind.supports_draft_model_listing() {
                backend
                    .list_draft_provider_adapter_models(&draft, &adapter_ids)
                    .await
                    .map_err(|error| error.to_string())
            } else if let Some(provider_id) = draft.source_provider_id.as_deref() {
                backend
                    .list_saved_provider_adapter_models(provider_id, &adapter_ids)
                    .await
                    .map_err(|error| error.to_string())
            } else {
                Err(provider_studio_listing_auth_required_message(
                    &i18n,
                    &draft.auth_kind,
                ))
            };
            let _ = tx.send(AppMessage::ProviderStudioAdapterModelsLoaded {
                request_key,
                result,
            });
        });
    }

    fn request_provider_studio_start_auth(&mut self, dialog: &mut ProviderStudioOverlay) {
        if dialog.pending_auth_key.is_some() {
            return;
        }
        let request_key = provider_studio_auth_request_key(&dialog.draft, "start");
        dialog.pending_auth_key = Some(request_key.clone());
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let draft = dialog.draft.clone();
        tokio::spawn(async move {
            let result = backend.start_provider_draft_auth(draft).await;
            let _ = tx.send(AppMessage::ProviderStudioAuthCompleted {
                request_key,
                result,
            });
        });
    }

    fn request_provider_studio_continue_auth(&mut self, dialog: &mut ProviderStudioOverlay) {
        if dialog.pending_auth_key.is_some() {
            return;
        }
        let request_key = provider_studio_auth_request_key(&dialog.draft, "continue");
        dialog.pending_auth_key = Some(request_key.clone());
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let draft = dialog.draft.clone();
        tokio::spawn(async move {
            let result = backend.continue_provider_draft_auth(draft).await;
            let _ = tx.send(AppMessage::ProviderStudioAuthCompleted {
                request_key,
                result,
            });
        });
    }

    fn session_model_chooser_items(&self) -> UiResult<Vec<SessionModelChoiceItem>> {
        let providers = self.backend.list_configured_providers();
        let mut items = Vec::new();
        for provider in providers {
            let default_adapter = provider.defaults.adapter.clone();
            let models = self
                .backend
                .list_local_provider_models(provider.provider_id.as_str())
                .map_err(|error| error.to_string())?;
            for model in models {
                items.push(session_model_choice_item(
                    &self.i18n,
                    provider.provider_id.as_str(),
                    default_adapter.as_deref(),
                    model,
                ));
            }
        }
        items.sort_by(|left, right| {
            (
                left.model.provider_id.to_string(),
                left.model
                    .adapter_id
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                left.model.model_id.to_string(),
            )
                .cmp(&(
                    right.model.provider_id.to_string(),
                    right
                        .model
                        .adapter_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    right.model.model_id.to_string(),
                ))
        });
        Ok(items)
    }

    fn request_provider_studio_save_draft(&mut self, dialog: ProviderStudioOverlay) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let adapter_ids = provider_studio_request_adapter_ids(&dialog);
        tokio::spawn(async move {
            let result = backend
                .save_provider_draft(
                    dialog.draft.clone(),
                    dialog.adapter_models.as_slice(),
                    &adapter_ids,
                    &dialog.selected_model_keys,
                )
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: dialog.draft.provider_id.clone(),
                result,
            });
        });
    }

    fn request_provider_studio_save_selected_adapter(&mut self, dialog: ProviderStudioOverlay) {
        let Some(adapter_models) = provider_studio_selected_adapter_models_for_save(&dialog) else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-required",
            ));
            return;
        };
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .save_provider_adapter_matches(dialog.draft.clone(), adapter_models)
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: dialog.draft.provider_id.clone(),
                result,
            });
        });
    }

    fn request_provider_studio_save_selected_model(&mut self, dialog: ProviderStudioOverlay) {
        let Some((adapter_id, model_id, provider_model)) =
            provider_studio_selected_model_target(&dialog)
        else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-model-required",
            ));
            return;
        };
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .save_provider_model(
                    dialog.draft.clone(),
                    adapter_id.as_str(),
                    model_id.as_str(),
                    provider_model,
                    true,
                )
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: dialog.draft.provider_id.clone(),
                result,
            });
        });
    }

    fn request_provider_studio_save_model_value(
        &mut self,
        draft: ProviderConfigDraft,
        adapter_id: String,
        model_id: String,
        model_value: JsonValue,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .save_provider_model_value(
                    draft.clone(),
                    adapter_id.as_str(),
                    model_id.as_str(),
                    model_value,
                    false,
                )
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: draft.provider_id.clone(),
                result,
            });
        });
    }

    fn request_provider_studio_delete_model(
        &mut self,
        draft: ProviderConfigDraft,
        adapter_id: String,
        model_id: String,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .delete_provider_model(draft.clone(), adapter_id.as_str(), model_id.as_str())
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: draft.provider_id.clone(),
                result,
            });
        });
    }

    fn request_provider_studio_delete_provider(&mut self, provider_id: String) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend.delete_provider(provider_id.as_str()).await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id,
                result,
            });
        });
    }

    fn request_provider_studio_delete_adapter(
        &mut self,
        draft: ProviderConfigDraft,
        adapter_id: String,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .delete_provider_adapter(draft.clone(), adapter_id.as_str())
                .await;
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: draft.provider_id.clone(),
                result,
            });
        });
    }

    fn move_provider_studio_selection(&mut self, dialog: &mut ProviderStudioOverlay, delta: isize) {
        match dialog.selection.focus() {
            ProviderStudioFocus::Fields => dialog
                .selection
                .move_top(provider_studio_visible_fields(dialog).len(), delta),
            ProviderStudioFocus::Adapters => {
                dialog
                    .selection
                    .move_left(dialog.adapter_candidate_ids.len(), delta);
                dialog.selection.clamp_right(
                    provider_studio_selected_adapter_models(dialog)
                        .map(|adapter| adapter.models.len())
                        .unwrap_or_default(),
                );
            }
            ProviderStudioFocus::Models => dialog.selection.move_right(
                provider_studio_selected_adapter_models(dialog)
                    .map(|adapter| adapter.models.len())
                    .unwrap_or_default(),
                delta,
            ),
        }
    }

    fn move_provider_studio_selection_page(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        delta: isize,
        page_size: usize,
    ) {
        match dialog.selection.focus() {
            ProviderStudioFocus::Fields => {
                dialog.selection.move_top_page(
                    provider_studio_visible_fields(dialog).len(),
                    delta,
                    page_size,
                );
            }
            ProviderStudioFocus::Adapters => {
                dialog.selection.move_left_page(
                    dialog.adapter_candidate_ids.len(),
                    delta,
                    page_size,
                );
                dialog.selection.clamp_right(
                    provider_studio_selected_adapter_models(dialog)
                        .map(|adapter| adapter.models.len())
                        .unwrap_or_default(),
                );
            }
            ProviderStudioFocus::Models => {
                dialog.selection.move_right_page(
                    provider_studio_selected_adapter_models(dialog)
                        .map(|adapter| adapter.models.len())
                        .unwrap_or_default(),
                    delta,
                    page_size,
                );
            }
        }
    }

    fn move_provider_studio_selection_home(&mut self, dialog: &mut ProviderStudioOverlay) {
        match dialog.selection.focus() {
            ProviderStudioFocus::Fields => dialog.selection.move_top_home(),
            ProviderStudioFocus::Adapters => {
                dialog.selection.move_left_home();
                dialog.selection.clamp_right(
                    provider_studio_selected_adapter_models(dialog)
                        .map(|adapter| adapter.models.len())
                        .unwrap_or_default(),
                );
            }
            ProviderStudioFocus::Models => dialog.selection.move_right_home(),
        }
    }

    fn move_provider_studio_selection_end(&mut self, dialog: &mut ProviderStudioOverlay) {
        match dialog.selection.focus() {
            ProviderStudioFocus::Fields => dialog
                .selection
                .move_top_end(provider_studio_visible_fields(dialog).len()),
            ProviderStudioFocus::Adapters => {
                dialog
                    .selection
                    .move_left_end(dialog.adapter_candidate_ids.len());
                dialog.selection.clamp_right(
                    provider_studio_selected_adapter_models(dialog)
                        .map(|adapter| adapter.models.len())
                        .unwrap_or_default(),
                );
            }
            ProviderStudioFocus::Models => {
                dialog.selection.move_right_end(
                    provider_studio_selected_adapter_models(dialog)
                        .map(|adapter| adapter.models.len())
                        .unwrap_or_default(),
                );
            }
        }
    }

    fn open_provider_studio_detail_page(&mut self, dialog: &mut ProviderStudioOverlay) {
        if provider_studio_detail_fields(dialog).is_empty() {
            self.flash_warning(provider_studio_no_auth_details_message(&self.i18n));
            return;
        }
        dialog.model_page = None;
        let mut selection = SelectionCursor::default();
        selection.selected = provider_studio_preferred_detail_field_index(dialog);
        dialog.detail_page = Some(ProviderStudioDetailPage {
            title: ui_text::t(&self.i18n, "overlay-provider-studio-detail"),
            footer: ui_text::t(&self.i18n, "overlay-provider-studio-detail-footer"),
            selection,
        });
    }

    fn guide_provider_studio_auth_field(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderStudioField,
    ) -> bool {
        if provider_studio_detail_field_index(dialog, field).is_none() {
            return false;
        }
        dialog.detail_page = None;
        self.activate_provider_studio_field_editor(dialog, field);
        true
    }

    fn activate_provider_studio_start_auth(&mut self, dialog: &mut ProviderStudioOverlay) {
        if let Some(field) = provider_studio_missing_start_auth_field(dialog) {
            let _ = self.guide_provider_studio_auth_field(dialog, field);
            return;
        }
        self.request_provider_studio_start_auth(dialog);
    }

    fn activate_provider_studio_continue_auth(&mut self, dialog: &mut ProviderStudioOverlay) {
        if let Some(field) = provider_studio_missing_continue_auth_field(dialog) {
            let _ = self.guide_provider_studio_auth_field(dialog, field);
            return;
        }
        self.request_provider_studio_continue_auth(dialog);
    }

    fn activate_provider_studio_field_editor(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderStudioField,
    ) {
        if !provider_studio_field_editable(dialog, field) {
            return;
        }
        if let Some(all_items) = self.provider_studio_field_choice_items(dialog, field) {
            self.open_choice_overlay(self.build_choice_overlay(
                ui_text::t(&self.i18n, "overlay-provider-studio-edit-title"),
                provider_studio_field_prompt(&self.i18n, field),
                Editor::from_text(provider_studio_field_value(&dialog.draft, field)),
                all_items,
                ChoiceOverlayAction::ProviderStudioField(field),
                provider_studio_field_allows_clear(field),
                Self::provider_studio_field_choice_overlay_style(field),
            ));
            return;
        }
        dialog.editor = Some(ProviderStudioEditor::new(
            ui_text::t(&self.i18n, "overlay-provider-studio-edit-title"),
            provider_studio_field_prompt(&self.i18n, field),
            ui_text::t(&self.i18n, "overlay-provider-studio-edit-footer"),
            false,
            Editor::from_text(provider_studio_field_value(&dialog.draft, field)),
            ProviderStudioEditorAction::Field(field),
        ));
    }

    fn activate_provider_studio_detail_page_selection(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(selected_field) = dialog
            .detail_page
            .as_ref()
            .map(|page| page.selection.selected)
        else {
            return;
        };
        let fields = provider_studio_detail_fields(dialog);
        let Some(field) = fields.get(selected_field).copied() else {
            return;
        };
        self.activate_provider_studio_field_editor(dialog, field);
    }

    fn handle_provider_studio_detail_page_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ProviderStudioOverlay,
    ) -> bool {
        let field_count = provider_studio_detail_fields(dialog).len();
        let Some(detail_page) = dialog.detail_page.as_mut() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => {
                dialog.detail_page = None;
                false
            }
            KeyCode::Char('o') | KeyCode::Char('O') if dialog.draft.supports_interactive_auth() => {
                self.activate_provider_studio_start_auth(dialog);
                false
            }
            KeyCode::Char('p') | KeyCode::Char('P') if dialog.draft.supports_interactive_auth() => {
                self.activate_provider_studio_continue_auth(dialog);
                false
            }
            KeyCode::Enter => {
                self.activate_provider_studio_detail_page_selection(dialog);
                false
            }
            _ if detail_page
                .selection
                .handle_navigation_key(key, field_count, 10) =>
            {
                false
            }
            _ => false,
        }
    }

    fn open_provider_studio_model_page(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        adapter_id: String,
        model_id: String,
        provider_model: Option<ProviderModel>,
    ) {
        match self.backend.provider_model_draft_value(
            &dialog.draft,
            adapter_id.as_str(),
            model_id.as_str(),
            provider_model.as_ref(),
        ) {
            Ok(model_value) => {
                let native_tools_present = model_value
                    .as_object()
                    .is_some_and(|object| object.contains_key("native_tools"));
                match provider_model_config_draft_from_value(model_id.as_str(), model_value) {
                    Ok(mut draft) => {
                        apply_provider_model_config_native_tools_suggestion(
                            &dialog.draft,
                            adapter_id.as_str(),
                            native_tools_present,
                            &mut draft,
                        );
                        dialog.detail_page = None;
                        dialog.model_page = Some(ProviderStudioModelPage {
                            title: self.i18n.text_args(
                                "overlay-provider-studio-model-title",
                                &crate::fl_args!(
                                    "adapter" => adapter_id.clone(),
                                    "model" => model_id.clone(),
                                ),
                            ),
                            footer: ui_text::t(&self.i18n, "overlay-provider-studio-model-footer"),
                            adapter_id,
                            original_model_id: model_id,
                            draft,
                            selection: SelectionCursor::default(),
                        });
                    }
                    Err(error) => self.flash_error(error),
                }
            }
            Err(error) => self.flash_error(error.to_string()),
        }
    }

    fn open_provider_studio_new_model_editor(&mut self, dialog: &mut ProviderStudioOverlay) {
        let Some(adapter_id) = provider_studio_selected_adapter_id(dialog) else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-required",
            ));
            return;
        };
        if !provider_studio_adapter_selectable(dialog, adapter_id.as_str()) {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-unavailable",
            ));
            return;
        }
        dialog.editor = Some(ProviderStudioEditor::new(
            ui_text::t(&self.i18n, "overlay-provider-studio-new-model-title"),
            ui_text::t(&self.i18n, "overlay-provider-studio-new-model-prompt"),
            ui_text::t(&self.i18n, "overlay-provider-studio-edit-footer"),
            false,
            Editor::from_text(String::new()),
            ProviderStudioEditorAction::NewModel { adapter_id },
        ));
    }

    fn open_provider_studio_delete_provider_confirm(&mut self, provider_id: String) {
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-provider-delete-title"),
            vec![self.i18n.text_args(
                "overlay-provider-delete-body",
                &crate::fl_args!("provider" => provider_id.clone()),
            )],
            ConfirmAction::ProviderStudioDeleteProvider { provider_id },
        )));
    }

    fn open_provider_studio_delete_adapter_confirm(
        &mut self,
        dialog: &ProviderStudioOverlay,
        adapter_id: String,
    ) {
        let mut body_lines = vec![self.i18n.text_args(
            "overlay-provider-delete-adapter-body",
            &crate::fl_args!(
                "provider" => dialog.draft.provider_id.clone(),
                "adapter" => adapter_id.clone(),
            ),
        )];
        if dialog.draft.source_provider_id.is_some()
            && dialog.configured_adapter_ids.len() == 1
            && dialog.configured_adapter_ids.contains(adapter_id.as_str())
        {
            body_lines.push(ui_text::t(
                &self.i18n,
                "overlay-provider-delete-adapter-last-body",
            ));
        }
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-provider-delete-adapter-title"),
            body_lines,
            ConfirmAction::ProviderStudioDeleteAdapter { adapter_id },
        )));
    }

    fn open_provider_studio_delete_model_confirm(
        &mut self,
        dialog: &ProviderStudioOverlay,
        adapter_id: String,
        model_id: String,
    ) {
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-provider-delete-model-title"),
            vec![self.i18n.text_args(
                "overlay-provider-delete-model-body",
                &crate::fl_args!(
                    "provider" => dialog.draft.provider_id.clone(),
                    "adapter" => adapter_id.clone(),
                    "model" => model_id.clone(),
                ),
            )],
            ConfirmAction::ProviderStudioDeleteModel {
                adapter_id,
                model_id,
            },
        )));
    }

    fn open_provider_studio_delete_selected_adapter_confirm(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(adapter_id) = provider_studio_selected_adapter_id(dialog) else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-required",
            ));
            return;
        };
        let has_state = dialog.configured_adapter_ids.contains(adapter_id.as_str())
            || dialog.selected_adapter_ids.contains(adapter_id.as_str())
            || dialog
                .adapter_models
                .iter()
                .any(|adapter_models| adapter_models.adapter_id == adapter_id)
            || dialog.draft.default_adapter == adapter_id;
        if !has_state {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-delete-empty",
            ));
            return;
        }
        self.open_provider_studio_delete_adapter_confirm(dialog, adapter_id);
    }

    fn open_provider_studio_delete_selected_model_confirm(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let target = if let Some(page) = dialog.model_page.as_ref() {
            Some((page.adapter_id.clone(), page.original_model_id.clone()))
        } else {
            provider_studio_selected_model_target(dialog)
                .map(|(adapter_id, model_id, _)| (adapter_id, model_id))
        };
        let Some((adapter_id, model_id)) = target else {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-model-required",
            ));
            return;
        };
        self.open_provider_studio_delete_model_confirm(dialog, adapter_id, model_id);
    }

    fn add_provider_studio_manual_model(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        adapter_id: String,
        model_id: String,
    ) -> UiResult<()> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return Err(ui_text::t(
                &self.i18n,
                "flash-provider-studio-model-id-required",
            ));
        }
        if !dialog.selected_adapter_ids.contains(adapter_id.as_str()) {
            dialog.selected_adapter_ids.insert(adapter_id.clone());
            dialog.adapter_selection_touched = true;
        }
        if !dialog
            .adapter_models
            .iter()
            .any(|adapter_models| adapter_models.adapter_id == adapter_id)
        {
            dialog.adapter_models.push(ProviderAdapterModelsResource {
                adapter_id: adapter_id.clone(),
                enabled: true,
                resolved_base_url: None,
                models: Vec::new(),
                error: None,
            });
        }
        let adapter_index = dialog
            .adapter_models
            .iter()
            .position(|adapter_models| adapter_models.adapter_id == adapter_id)
            .expect("adapter models entry must exist");
        if !dialog.adapter_models[adapter_index]
            .models
            .iter()
            .any(|model| model.id.as_str() == model_id)
        {
            dialog.adapter_models[adapter_index]
                .models
                .push(ProviderModel::new(adapter_id.as_str(), model_id));
            dialog.adapter_models[adapter_index]
                .models
                .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        }
        let selected_model_index = dialog.adapter_models[adapter_index]
            .models
            .iter()
            .position(|model| model.id.as_str() == model_id)
            .unwrap_or_default();
        if let Some(left_index) = dialog
            .adapter_candidate_ids
            .iter()
            .position(|candidate| candidate == &adapter_id)
        {
            dialog.selection.set_left_selected(left_index);
        }
        dialog.selection.set_right_selected(selected_model_index);
        dialog
            .selected_model_keys
            .insert(provider_studio_model_key(adapter_id.as_str(), model_id));
        provider_studio_ensure_default_selection(dialog);
        self.open_provider_studio_model_page(dialog, adapter_id, model_id.to_owned(), None);
        Ok(())
    }

    fn activate_provider_studio_model_field_editor(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderModelConfigField,
    ) {
        if !provider_model_config_field_editable(field) {
            return;
        }
        if let Some(items) = self.provider_model_config_field_choice_items(dialog, field) {
            let current = dialog
                .model_page
                .as_ref()
                .map(|page| provider_model_config_field_value(&page.draft, field))
                .unwrap_or_default();
            self.open_choice_overlay(self.build_choice_overlay(
                ui_text::t(&self.i18n, "overlay-provider-studio-model-edit-title"),
                provider_model_config_field_prompt(&self.i18n, field),
                Editor::from_text(current),
                items,
                ChoiceOverlayAction::ProviderStudioModelField(field),
                !matches!(field, ProviderModelConfigField::Enabled),
                Self::provider_model_config_field_choice_overlay_style(field),
            ));
            return;
        }
        let current = dialog
            .model_page
            .as_ref()
            .map(|page| provider_model_config_field_value(&page.draft, field))
            .unwrap_or_default();
        dialog.editor = Some(ProviderStudioEditor::new(
            ui_text::t(&self.i18n, "overlay-provider-studio-model-edit-title"),
            provider_model_config_field_prompt(&self.i18n, field),
            ui_text::t(&self.i18n, "overlay-provider-studio-edit-footer"),
            matches!(field, ProviderModelConfigField::Description),
            Editor::from_text(current),
            ProviderStudioEditorAction::ModelField(field),
        ));
    }

    fn activate_provider_studio_model_page_selection(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(selected) = dialog
            .model_page
            .as_ref()
            .map(|page| page.selection.selected)
        else {
            return;
        };
        let Some(field) = provider_model_config_fields().get(selected).copied() else {
            return;
        };
        self.activate_provider_studio_model_field_editor(dialog, field);
    }

    fn commit_provider_studio_model_field(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderModelConfigField,
        value: String,
    ) -> UiResult<()> {
        let Some(page) = dialog.model_page.as_mut() else {
            return Err(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
        };
        commit_provider_model_config_field(&mut page.draft, field, value)
    }

    fn save_provider_studio_model_page(&mut self, dialog: &mut ProviderStudioOverlay) {
        let Some(page) = dialog.model_page.as_ref() else {
            return;
        };
        let (model_id, model_value) = match provider_model_config_draft_to_model_value(&page.draft)
        {
            Ok(value) => value,
            Err(error) => {
                self.flash_error(error);
                return;
            }
        };
        dialog.saving = true;
        self.request_provider_studio_save_model_value(
            dialog.draft.clone(),
            page.adapter_id.clone(),
            model_id,
            model_value,
        );
    }

    fn delete_provider_studio_model(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        adapter_id: String,
        model_id: String,
    ) {
        if dialog.draft.source_provider_id.is_some() {
            dialog.saving = true;
            self.request_provider_studio_delete_model(dialog.draft.clone(), adapter_id, model_id);
        } else {
            remove_provider_studio_model_from_dialog(
                dialog,
                adapter_id.as_str(),
                model_id.as_str(),
            );
            dialog.selection.set_focus(ProviderStudioFocus::Models);
        }
    }

    fn delete_provider_studio_adapter(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        adapter_id: String,
    ) {
        if dialog.draft.source_provider_id.is_some()
            && dialog.configured_adapter_ids.contains(adapter_id.as_str())
        {
            dialog.saving = true;
            self.request_provider_studio_delete_adapter(dialog.draft.clone(), adapter_id);
        } else {
            remove_provider_studio_adapter_from_dialog(dialog, adapter_id.as_str());
            dialog.selection.set_focus(ProviderStudioFocus::Adapters);
        }
    }

    fn handle_provider_studio_model_page_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ProviderStudioOverlay,
    ) -> bool {
        let field_count = provider_model_config_fields().len();
        if dialog.model_page.is_none() {
            return false;
        }
        match key.code {
            KeyCode::Esc => {
                dialog.model_page = None;
                false
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                self.save_provider_studio_model_page(dialog);
                false
            }
            KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
                self.open_provider_studio_delete_selected_model_confirm(dialog);
                false
            }
            KeyCode::Enter => {
                self.activate_provider_studio_model_page_selection(dialog);
                false
            }
            _ if dialog
                .model_page
                .as_mut()
                .is_some_and(|page| page.selection.handle_navigation_key(key, field_count, 10)) =>
            {
                false
            }
            _ => false,
        }
    }

    fn activate_provider_studio_focus(&mut self, dialog: &mut ProviderStudioOverlay) {
        match dialog.selection.focus() {
            ProviderStudioFocus::Fields => {
                let fields = provider_studio_visible_fields(dialog);
                let Some(field) = fields.get(dialog.selection.top_selected()).copied() else {
                    return;
                };
                match field {
                    ProviderStudioField::StartAuthAction => {
                        self.activate_provider_studio_start_auth(dialog);
                    }
                    ProviderStudioField::ContinueAuthAction => {
                        self.activate_provider_studio_continue_auth(dialog);
                    }
                    ProviderStudioField::EditAuthDetailsAction => {
                        self.open_provider_studio_detail_page(dialog);
                    }
                    ProviderStudioField::DeleteProviderAction => {
                        if let Some(provider_id) = dialog.draft.source_provider_id.clone() {
                            self.open_provider_studio_delete_provider_confirm(provider_id);
                        }
                    }
                    _ => self.activate_provider_studio_field_editor(dialog, field),
                }
            }
            ProviderStudioFocus::Adapters => {
                if let Some(adapter_id) = provider_studio_selected_adapter_models(dialog)
                    .map(|adapter_models| adapter_models.adapter_id.clone())
                {
                    dialog.draft.default_adapter = adapter_id;
                }
            }
            ProviderStudioFocus::Models => {
                if let Some((adapter_id, model_id, provider_model)) =
                    provider_studio_selected_model_target(dialog)
                {
                    self.open_provider_studio_model_page(
                        dialog,
                        adapter_id,
                        model_id,
                        provider_model,
                    );
                }
            }
        }
    }

    fn commit_provider_studio_field(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
        field: ProviderStudioField,
        value: String,
    ) -> UiResult<()> {
        match field {
            ProviderStudioField::ProviderId => {
                dialog.draft.provider_id = value;
                dialog.draft.normalize_shape();
                self.refresh_provider_studio_adapter_state(dialog);
            }
            ProviderStudioField::StartAuthAction
            | ProviderStudioField::ContinueAuthAction
            | ProviderStudioField::EditAuthDetailsAction
            | ProviderStudioField::DeleteProviderAction => {}
            ProviderStudioField::AuthMode => {
                match ProviderDraftAuthKind::parse_category(
                    value.as_str(),
                    dialog.draft.auth_kind.clone(),
                ) {
                    Ok(auth_kind) => {
                        dialog.draft.auth_kind = auth_kind;
                        dialog.draft.normalize_shape();
                        self.refresh_provider_studio_adapter_state(dialog);
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            ProviderStudioField::AuthSubtype => {
                match ProviderDraftAuthKind::parse_subtype(
                    value.as_str(),
                    dialog.draft.auth_kind.clone(),
                ) {
                    Ok(auth_kind) => {
                        dialog.draft.auth_kind = auth_kind;
                        dialog.draft.normalize_shape();
                        self.refresh_provider_studio_adapter_state(dialog);
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            ProviderStudioField::AuthLoginMethod => {
                let Some(kind) = ProviderDraftInteractiveLoginKind::parse(value.as_str()) else {
                    return Err(ui_text::t(
                        &self.i18n,
                        "flash-provider-studio-invalid-auth-login-method",
                    ));
                };
                dialog.draft.set_interactive_login_kind(kind);
            }
            ProviderStudioField::BaseUrl => {
                dialog.draft.auth.base_url = value;
            }
            ProviderStudioField::InstanceUrl => {
                dialog.draft.auth.instance_url = value;
            }
            ProviderStudioField::ApiKeySource => {
                dialog.draft.auth.secret_source_kind =
                    ProviderDraftSecretSourceKind::parse(value.as_str())
                        .map_err(|error| error.to_string())?;
            }
            ProviderStudioField::ApiKeyValue => {
                dialog.draft.auth.secret_source_value = value;
            }
            ProviderStudioField::RedirectUri => {
                dialog.draft.set_redirect_uri(value);
            }
            ProviderStudioField::CallbackUrl => {
                dialog.draft.set_callback_url(value);
            }
            ProviderStudioField::RefreshToken => {
                dialog.draft.set_refresh_token(value);
            }
            ProviderStudioField::AccessToken => {
                dialog.draft.set_access_token(value);
            }
            ProviderStudioField::ExpiresAtMs => {
                dialog.draft.set_expires_at_ms(value);
            }
            ProviderStudioField::AccountId => {
                dialog.draft.set_account_id(value);
            }
            ProviderStudioField::EnterpriseDomain => {
                dialog
                    .draft
                    .credential_drafts
                    .github_copilot
                    .enterprise_domain = value;
            }
            ProviderStudioField::Region => {
                dialog.draft.auth.region = value;
            }
            ProviderStudioField::Profile => {
                dialog.draft.auth.profile = value;
            }
            ProviderStudioField::AccessKeyId => {
                dialog.draft.auth.access_key_id = value;
            }
            ProviderStudioField::SecretAccessKey => {
                dialog.draft.auth.secret_access_key = value;
            }
            ProviderStudioField::SessionToken => {
                dialog.draft.auth.session_token = value;
            }
            ProviderStudioField::ServiceKeyEnv => {
                dialog.draft.auth.service_key_env = value;
            }
            ProviderStudioField::DefaultAdapter => {
                dialog.draft.default_adapter = value;
                self.sync_provider_studio_shape(dialog);
            }
            ProviderStudioField::DefaultModel => {
                dialog.draft.default_model = value;
            }
        }
        Ok(())
    }

    fn toggle_provider_studio_selected_adapter(&mut self, dialog: &mut ProviderStudioOverlay) {
        let Some(adapter_id) = provider_studio_selected_adapter_id(dialog) else {
            return;
        };
        if !provider_studio_adapter_selectable(dialog, adapter_id.as_str()) {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-provider-studio-adapter-unavailable",
            ));
            return;
        }
        if !dialog.selected_adapter_ids.remove(adapter_id.as_str()) {
            dialog.selected_adapter_ids.insert(adapter_id);
        }
        dialog.adapter_selection_touched = true;
        self.sync_provider_studio_shape(dialog);
    }

    fn toggle_provider_studio_selected_model(&mut self, dialog: &mut ProviderStudioOverlay) {
        let Some((adapter_id, model_id, _)) = provider_studio_selected_model_target(dialog) else {
            return;
        };
        let key = provider_studio_model_key(adapter_id.as_str(), model_id.as_str());
        if !dialog.selected_model_keys.remove(key.as_str()) {
            dialog.selected_model_keys.insert(key);
        }
        provider_studio_ensure_default_selection(dialog);
    }

    fn select_all_provider_studio_adapters(dialog: &mut ProviderStudioOverlay) {
        dialog.selected_adapter_ids = dialog
            .adapter_candidate_ids
            .iter()
            .filter(|adapter_id| provider_studio_adapter_selectable(dialog, adapter_id.as_str()))
            .cloned()
            .collect();
        dialog.adapter_selection_touched = true;
    }

    fn clear_provider_studio_selected_adapters(dialog: &mut ProviderStudioOverlay) {
        dialog.selected_adapter_ids.clear();
        dialog.adapter_selection_touched = true;
        provider_studio_ensure_default_selection(dialog);
    }

    fn select_all_provider_studio_models(dialog: &mut ProviderStudioOverlay) {
        let Some(adapter_models) = provider_studio_selected_adapter_models(dialog) else {
            return;
        };
        let adapter_id = adapter_models.adapter_id.clone();
        let model_ids = adapter_models
            .models
            .iter()
            .map(|model| model.id.to_string())
            .collect::<Vec<_>>();
        for model_id in model_ids {
            dialog.selected_model_keys.insert(provider_studio_model_key(
                adapter_id.as_str(),
                model_id.as_str(),
            ));
        }
        provider_studio_ensure_default_selection(dialog);
    }

    fn clear_provider_studio_selected_models(dialog: &mut ProviderStudioOverlay) {
        let Some(adapter_models) = provider_studio_selected_adapter_models(dialog) else {
            return;
        };
        let adapter_id = adapter_models.adapter_id.clone();
        dialog
            .selected_model_keys
            .retain(|key| !key.starts_with(format!("{adapter_id}\u{1f}").as_str()));
        provider_studio_ensure_default_selection(dialog);
    }

    fn sync_provider_studio_shape(&mut self, dialog: &mut ProviderStudioOverlay) {
        dialog.draft.normalize_shape();
        dialog.adapter_candidate_ids = provider_studio_candidate_adapter_ids(
            &dialog.draft,
            dialog.configured_adapter_ids.clone(),
        );
        let selectable_adapter_ids = dialog
            .adapter_candidate_ids
            .iter()
            .filter(|adapter_id| provider_studio_adapter_selectable(dialog, adapter_id.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>();
        dialog
            .selection
            .clamp_top(provider_studio_visible_fields(dialog).len());
        let detail_field_count = provider_studio_detail_fields(dialog).len();
        if let Some(detail_page) = dialog.detail_page.as_mut() {
            detail_page.selection.clamp(detail_field_count);
        }
        dialog.selected_adapter_ids.retain(|adapter_id| {
            dialog
                .adapter_candidate_ids
                .iter()
                .any(|candidate| candidate == adapter_id)
                && selectable_adapter_ids.contains(adapter_id)
        });
        if dialog.adapter_candidate_ids.len() == 1 {
            dialog.selected_adapter_ids = selectable_adapter_ids.clone();
        } else if !dialog.adapter_selection_touched && dialog.selected_adapter_ids.is_empty() {
            dialog.selected_adapter_ids = selectable_adapter_ids.clone();
        }
        dialog
            .selection
            .clamp_left(dialog.adapter_candidate_ids.len());
        dialog.selection.clamp_right(
            provider_studio_selected_adapter_models(dialog)
                .map(|adapter| adapter.models.len())
                .unwrap_or_default(),
        );
        if !dialog.adapter_models.is_empty() {
            provider_studio_restore_model_selection(dialog);
        }
        provider_studio_ensure_default_selection(dialog);
        self.sync_provider_studio_auth_poll_deadline(dialog, Instant::now(), false);
    }

    fn sync_provider_studio_auth_poll_deadline(
        &self,
        dialog: &mut ProviderStudioOverlay,
        now: Instant,
        reset: bool,
    ) {
        match provider_studio_auth_poll_interval(dialog) {
            Some(interval) if reset || dialog.next_auth_poll_at.is_none() => {
                dialog.next_auth_poll_at = now.checked_add(interval).or(Some(now));
            }
            Some(_) => {}
            None => {
                dialog.next_auth_poll_at = None;
            }
        }
    }

    fn reload_provider_studio_catalog_matches(&self, dialog: &mut ProviderStudioOverlay) {
        let lookup_ids = dialog
            .adapter_models
            .iter()
            .flat_map(|adapter| {
                adapter.models.iter().flat_map(|model| {
                    [
                        model.id.to_string(),
                        provider_model_catalog_lookup_id(model),
                    ]
                })
            })
            .collect::<Vec<_>>();
        let catalog_entries = self.backend.lookup_model_catalog_models(&lookup_ids);
        dialog.catalog_matches = dialog
            .adapter_models
            .iter()
            .flat_map(|adapter| {
                adapter.models.iter().filter_map(|provider_model| {
                    provider_studio_catalog_match_model(provider_model, &catalog_entries).map(
                        |catalog_model| {
                            (
                                provider_studio_model_key(
                                    adapter.adapter_id.as_str(),
                                    provider_model.id.as_str(),
                                ),
                                catalog_model.clone(),
                            )
                        },
                    )
                })
            })
            .collect();
    }

    fn refresh_provider_studio_adapter_state(&mut self, dialog: &mut ProviderStudioOverlay) {
        dialog.adapter_models.clear();
        dialog.selected_model_keys.clear();
        dialog.catalog_matches.clear();
        self.sync_provider_studio_shape(dialog);
        dialog.selection.set_right_selected(0);
        dialog.pending_adapter_models_key = None;
        dialog.listing_adapter_models = false;
    }

    fn open_child_sessions_picker(&mut self) {
        let Some(parent_session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let dialog = self.build_picker_overlay(
            self.i18n.text_args(
                "overlay-children-title",
                &crate::fl_args!("session" => parent_session_id),
            ),
            ui_text::t(&self.i18n, "overlay-children-prompt"),
            ui_text::t(&self.i18n, "overlay-picker-footer"),
            ui_text::t(&self.i18n, "overlay-picker-loading"),
            Editor::default(),
            Vec::new(),
            PickerKind::ChildSessions { parent_session_id },
            true,
        );
        self.current_route = Route::Picker(dialog);
        self.request_child_sessions(parent_session_id);
    }

    fn open_rewind_confirm_overlay(&mut self, session_id: i64, message_id: i64, target: String) {
        self.overlay = Some(Overlay::Confirm(self.build_confirm_overlay(
            ui_text::t(&self.i18n, "overlay-rewind-confirm-title"),
            vec![
                self.i18n.text_args(
                    "overlay-rewind-confirm-keep",
                    &crate::fl_args!("target" => target.clone()),
                ),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-warning"),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-draft"),
            ],
            ConfirmAction::Rewind {
                session_id,
                message_id,
                target,
            },
        )));
    }

    fn lineage_session_picker_item(&self, item: LineageSessionItem) -> PickerItem {
        let session = item.session;
        let mut detail_parts = vec![ui_text::session_meta(
            &self.i18n,
            session.id,
            session.message_count,
            session.updated_at,
        )];
        detail_parts.push(ui_text::t(
            &self.i18n,
            lineage_relation_tag_key(item.relation),
        ));
        if item.is_leaf {
            detail_parts.push(ui_text::t(&self.i18n, "session-tag-leaf"));
        }
        if let Some(parent_id) = session.parent_id {
            detail_parts.push(self.i18n.text_args(
                "session-summary-parent",
                &crate::fl_args!("id" => parent_id),
            ));
        }
        if session.child_session_count > 0 {
            detail_parts.push(self.i18n.text_args(
                "session-summary-children",
                &crate::fl_args!("count" => session.child_session_count as i64),
            ));
        }

        PickerItem {
            label: format!(
                "{}{}{}",
                "  ".repeat(item.depth),
                if item.depth == 0 { "◆ " } else { "↳ " },
                session.title
            ),
            detail: detail_parts.join(" | "),
            value: PickerValue::Session(session.id),
        }
    }

    fn session_search_item(&self, session: SessionResource) -> SessionSearchItem {
        let mut detail_parts = vec![ui_text::session_meta(
            &self.i18n,
            session.id,
            session.message_count,
            session.updated_at,
        )];
        if self.transcript.session_id == Some(session.id) {
            detail_parts.push(ui_text::t(&self.i18n, "session-tag-current"));
        }
        if let Some(parent_id) = session.parent_id {
            detail_parts.push(self.i18n.text_args(
                "session-summary-parent",
                &crate::fl_args!("id" => parent_id),
            ));
        }
        if session.child_session_count > 0 {
            detail_parts.push(self.i18n.text_args(
                "session-summary-children",
                &crate::fl_args!("count" => session.child_session_count as i64),
            ));
        }

        SessionSearchItem {
            label: session.title.clone(),
            detail: detail_parts.join(" | "),
            session,
        }
    }

    fn rewind_message_picker_item(&self, message: MessageResource) -> PickerItem {
        PickerItem {
            label: self.rewind_message_target_label(&message),
            detail: format!(
                "#{} | {} | {}",
                message.id,
                ui_text::message_state_label(&self.i18n, message.state),
                format_timestamp(message.created_at)
            ),
            value: PickerValue::Message(message.id),
        }
    }

    fn rewind_message_target_label(&self, message: &MessageResource) -> String {
        format!(
            "[{}] {}",
            ui_text::role_label(&self.i18n, message.role),
            rewind_message_preview(message, &self.i18n)
        )
    }

    fn refresh_picker_overlay(dialog: &mut PickerOverlay) {
        let all_items = dialog.meta.all_items.clone();
        refresh_search_list_overlay(dialog, all_items.as_slice());
    }

    fn refresh_session_model_chooser_overlay(
        dialog: &mut SessionModelChooserOverlay,
        prefer_current_model: bool,
        current_model: Option<&ModelRef>,
    ) {
        let query = dialog.input.text().trim().to_ascii_lowercase();
        let previous_model = dialog
            .items
            .get(dialog.selected)
            .map(|item| item.model.clone());
        dialog.items = dialog
            .meta
            .all_items
            .iter()
            .filter(|item| query.is_empty() || item.search_text.contains(query.as_str()))
            .cloned()
            .collect();
        if dialog.items.is_empty() {
            dialog.selected = 0;
            return;
        }

        if let Some(previous_model) = previous_model
            && let Some(index) = dialog
                .items
                .iter()
                .position(|item| item.model == previous_model)
        {
            dialog.selected = index;
            return;
        }

        if prefer_current_model
            && let Some(current_model) = current_model
            && let Some(index) = dialog
                .items
                .iter()
                .position(|item| session_model_matches_current(&item.model, current_model))
        {
            dialog.selected = index;
            return;
        }

        dialog.selected = min(dialog.selected, dialog.items.len().saturating_sub(1));
    }

    fn refresh_timeline_overlay(dialog: &mut TimelineOverlay) {
        refresh_search_panels_overlay(dialog, |item, query| item.search_text.contains(query));
    }

    fn handle_picker_selection(&mut self, kind: PickerKind, item: PickerItem) {
        match (kind, item.value) {
            (PickerKind::Commands, PickerValue::Command(spec)) => {
                self.execute_command(spec, "");
            }
            (PickerKind::Commands, PickerValue::RuntimeTool(tool_name)) => {
                self.composer
                    .set_text(format!("/{tool_name} ").trim_end().to_string());
                self.focus = Focus::Composer;
                self.sync_composer_suggestions();
            }
            (PickerKind::Lineage { .. }, PickerValue::Session(session_id)) => {
                self.open_session(
                    session_id,
                    ui_text::session_fallback_title(&self.i18n, session_id),
                );
                self.focus = Focus::Transcript;
            }
            (PickerKind::RewindMessages { session_id }, PickerValue::Message(message_id)) => {
                let target = format!(
                    "{} ({})",
                    item.label,
                    item.detail.split(" | ").next().unwrap_or_default()
                );
                self.open_rewind_confirm_overlay(session_id, message_id, target);
            }
            (
                PickerKind::Providers(ProviderPickerPurpose::SetProvider),
                PickerValue::Provider(provider),
            ) => {
                self.apply_provider_override(provider);
            }
            (PickerKind::ChildSessions { .. }, PickerValue::Session(session_id)) => {
                self.open_session(
                    session_id,
                    ui_text::session_fallback_title(&self.i18n, session_id),
                );
                self.focus = Focus::Transcript;
            }
            (PickerKind::PermissionRules, PickerValue::PermissionRuleCreate) => {
                self.open_permission_rule_studio(None, None);
            }
            (PickerKind::PermissionRules, PickerValue::PermissionRule(rule)) => {
                self.open_permission_rule_studio(Some(&rule), None);
            }
            (PickerKind::Inspector, PickerValue::Inspector) => {}
            _ => {}
        }
    }

    fn apply_provider_override(&mut self, provider: ProviderSummaryResource) {
        self.run_options.model = Some(match provider.defaults.adapter.clone() {
            Some(adapter_id) => ModelRef::new_with_adapter(
                provider.provider_id.clone(),
                adapter_id,
                provider.defaults.model.clone(),
            ),
            None => ModelRef::new(
                provider.provider_id.clone(),
                provider.defaults.model.clone(),
            ),
        });
        self.run_options.thinking_mode = None;
        self.run_options.speed_mode = None;
        self.run_options.verbosity = None;
        self.run_options.parallel_tool_calls = None;
        self.focus = Focus::Composer;
        self.flash_success(self.i18n.text_args(
            "flash-provider-selected",
            &crate::fl_args!(
                "provider" => provider.provider_id,
                "model" => provider.defaults.model,
            ),
        ));
    }

    fn apply_model_override(&mut self, model: ModelRef) {
        self.run_options.model = Some(model.clone());
        self.run_options.thinking_mode = None;
        self.run_options.speed_mode = None;
        self.run_options.verbosity = None;
        self.run_options.parallel_tool_calls = None;
        self.focus = Focus::Composer;
        self.flash_success(self.i18n.text_args(
            "flash-model-selected",
            &crate::fl_args!(
                "model" => format!("{}/{}", model.provider_id, model.model_id),
            ),
        ));
        self.open_session_model_thinking_step_or_next();
    }

    fn current_or_selected_session_id(&self) -> Option<i64> {
        if self.focus == Focus::Sessions {
            self.sessions
                .current_selected_id()
                .or(self.transcript.session_id)
        } else {
            self.transcript
                .session_id
                .or_else(|| self.sessions.current_selected_id())
        }
    }

    fn current_transcript_has_active_run(&self) -> bool {
        self.transcript.submitting
            || self.transcript.execution.as_ref().is_some_and(|execution| {
                execution.run_state != SessionRunState::Idle
                    && pending_interactive_kind_for_execution(execution).is_none()
            })
    }

    fn active_run_session_id(&self) -> Option<i64> {
        self.transcript
            .session_id
            .filter(|_| self.current_transcript_has_active_run())
    }

    fn session_is_busy(&self, session_id: i64) -> bool {
        self.submitting_session_ids.contains(&session_id)
            || (self.transcript.session_id == Some(session_id)
                && self.current_transcript_has_active_run())
    }

    fn current_or_selected_session_title(&self) -> Option<String> {
        if self.focus == Focus::Sessions
            && let Some(session) = self.sessions.current_selected()
        {
            return Some(session.title.clone());
        }
        if let Some(execution) = self.transcript.execution.as_ref() {
            return Some(execution.session.title.clone());
        }
        if self.transcript.session_id.is_some() && !self.transcript.session_title.trim().is_empty()
        {
            return Some(self.transcript.session_title.clone());
        }
        self.sessions
            .current_selected()
            .map(|session| session.title.clone())
    }

    fn current_parent_session_id(&self) -> Option<i64> {
        self.transcript.execution.as_ref()?.session.parent_id
    }

    fn sync_session_list_selection_to_current_execution(&mut self) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return;
        };
        if self.transcript.session_id != Some(execution.session.id) {
            return;
        }
        if let Some(session_id) = preferred_visible_session_selection(
            &execution.session,
            self.sessions.list.items.as_slice(),
        ) {
            let _ = self.sessions.select_by_id(session_id);
        }
    }

    fn current_lineage_context_parts(&self) -> Vec<String> {
        let Some(lineage) = self.current_lineage.as_ref() else {
            return Vec::new();
        };
        if self.transcript.session_id != Some(lineage.session_id) {
            return Vec::new();
        }

        let mut parts = vec![
            self.i18n.text_args(
                "session-summary-root",
                &crate::fl_args!("id" => lineage.summary.root_id),
            ),
            self.i18n.text_args(
                "session-summary-depth",
                &crate::fl_args!("depth" => lineage.summary.depth as i64),
            ),
        ];
        if lineage.summary.side_branch_count > 0 {
            parts.push(self.i18n.text_args(
                "session-summary-side-branches",
                &crate::fl_args!("count" => lineage.summary.side_branch_count as i64),
            ));
        }
        if lineage.summary.descendant_count > 0 {
            parts.push(self.i18n.text_args(
                "session-summary-descendants",
                &crate::fl_args!("count" => lineage.summary.descendant_count as i64),
            ));
        }
        parts
    }

    fn current_session_path_label(&self) -> Option<String> {
        let lineage = self.current_lineage.as_ref()?;
        if self.transcript.session_id != Some(lineage.session_id) || lineage.path.is_empty() {
            return None;
        }

        Some(format!(
            "path {}",
            lineage
                .path
                .iter()
                .map(|segment| format!("#{}", segment.id))
                .collect::<Vec<_>>()
                .join(" > ")
        ))
    }

    fn open_parent_session(&mut self) {
        let Some(parent_id) = self.current_parent_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-parent-session-missing"));
            return;
        };
        self.open_session(
            parent_id,
            ui_text::session_fallback_title(&self.i18n, parent_id),
        );
        self.focus = Focus::Transcript;
    }

    fn current_session_model_ref(&self) -> Option<ModelRef> {
        if let Some(model) = self.run_options.model.as_ref() {
            return Some(model.clone());
        }
        let execution = self.transcript.execution.as_ref()?;
        let provider_id = execution.execution.model_provider_id.as_deref()?;
        let model_id = execution.execution.model_id.as_deref()?;
        Some(
            execution
                .execution
                .model_adapter_id
                .as_deref()
                .map(|adapter_id| ModelRef::new_with_adapter(provider_id, adapter_id, model_id))
                .unwrap_or_else(|| ModelRef::new(provider_id, model_id)),
        )
    }

    fn handle_confirm_action(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Rewind {
                session_id,
                message_id,
                target,
            } => self.request_session_rewind(session_id, message_id, target),
            ConfirmAction::RevokePermissionRule {
                rule_id,
                label,
                return_query,
            } => match self.block_on_async(self.backend.revoke_permission_rule(rule_id)) {
                Ok(_) => {
                    self.flash_success(self.i18n.text_args(
                        "flash-permission-rule-revoked",
                        &crate::fl_args!("name" => label),
                    ));
                    self.refresh_permission_rules_route(return_query.as_str());
                }
                Err(error) => self.flash_error(error),
            },
            ConfirmAction::PermissionStudioDeletePathRule { pattern } => {
                self.delete_permission_studio_path_rule(pattern.as_str())
            }
            ConfirmAction::PermissionStudioDeleteNetworkRule { target } => {
                self.delete_permission_studio_network_rule(target.as_str())
            }
            ConfirmAction::PermissionStudioDeleteToolTag { key } => {
                self.delete_permission_studio_tool_tag(key.as_str())
            }
            ConfirmAction::PermissionStudioDeleteToolName { key } => {
                self.delete_permission_studio_tool_name(key.as_str())
            }
            ConfirmAction::PermissionStudioDeleteToolRule { tool_name } => {
                self.delete_permission_studio_tool_rule(tool_name.as_str())
            }
            ConfirmAction::ExitSnapshot {
                session_id,
                discard_changes,
            } => match self
                .backend
                .exit_snapshot(session_id, "remove".to_string(), discard_changes)
            {
                Ok(output) => self.flash_success(ui_text::snapshot_exit_message(
                    &self.i18n,
                    output.action.as_deref(),
                    output.path.as_str(),
                )),
                Err(error) => self.flash_error(error.to_string()),
            },
            ConfirmAction::ProviderStudioDeleteProvider { provider_id } => {
                let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return;
                };
                dialog.saving = true;
                self.request_provider_studio_delete_provider(provider_id);
                self.restore_provider_studio_dialog(host, dialog);
            }
            ConfirmAction::ProviderStudioDeleteAdapter { adapter_id } => {
                let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return;
                };
                self.delete_provider_studio_adapter(&mut dialog, adapter_id);
                self.restore_provider_studio_dialog(host, dialog);
            }
            ConfirmAction::ProviderStudioDeleteModel {
                adapter_id,
                model_id,
            } => {
                let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
                    self.flash_error(ui_text::t(&self.i18n, "flash-provider-studio-context-lost"));
                    return;
                };
                self.delete_provider_studio_model(&mut dialog, adapter_id, model_id);
                self.restore_provider_studio_dialog(host, dialog);
            }
        }
    }

    fn execute_command(&mut self, spec: &'static CommandSpec, args: &str) {
        match spec.id {
            CommandId::Help => {
                self.route_stack.clear();
                self.current_route = Route::Help(HelpOverlay::default());
            }
            CommandId::Commands => self.open_command_palette(),
            CommandId::New => self.create_session(None),
            CommandId::Sessions => self.handle_sessions_command(spec, args),
            CommandId::Lineage => self.open_lineage_picker(),
            CommandId::Rewind => self.open_rewind_messages_picker(),
            CommandId::Find => {
                self.focus = Focus::Transcript;
                if args.trim().is_empty() {
                    self.overlay = Some(Overlay::TranscriptSearch(
                        self.build_transcript_search_overlay(),
                    ));
                } else {
                    self.transcript.set_search_query(args.trim().to_string());
                    self.jump_search_match(true);
                }
            }
            CommandId::Rename => self.handle_rename_command(spec, args),
            CommandId::Timeline => self.handle_timeline_command(spec, args),
            CommandId::Plugins => self.handle_plugins_command(spec, args),
            CommandId::Settings => self.handle_settings_command(args),
            CommandId::Permissions => self.handle_permissions_command(args),
            CommandId::Model => self.open_session_model_chooser(),
            CommandId::Review => self.handle_review_command(args),
            CommandId::Snapshot => self.handle_snapshot_command(args),
            CommandId::Commit => self.handle_commit_command(args),
            CommandId::Pr => self.handle_pr_command(args),
            CommandId::Export => self.handle_export_command(args),
            CommandId::Memory => self.handle_memory_command(spec, args),
            CommandId::Pager => self.pending_ui_action = Some(UiAction::PageTranscript),
            CommandId::Continue => self.continue_current_session(),
            CommandId::Compact => self.compact_current_session(),
            CommandId::UserInput => self.open_user_input_overlay(),
            CommandId::Allow => self.reply_permission(PermissionReplyKind::AllowOnce),
            CommandId::AllowAlways => self.reply_permission(PermissionReplyKind::AllowAlways),
            CommandId::Deny => self.reply_permission(PermissionReplyKind::DenyOnce),
            CommandId::DenyAlways => self.reply_permission(PermissionReplyKind::DenyAlways),
            CommandId::Attach => {
                self.focus = Focus::Composer;
                self.open_file_attach_overlay();
            }
            CommandId::Editor => {
                self.composer.flush_all_pending_input();
                self.pending_ui_action = Some(UiAction::EditComposerExternally);
            }
            CommandId::Image => {
                self.pending_ui_action = Some(UiAction::AttachClipboardImage);
            }
            CommandId::Copy => self.copy_loaded_transcript(),
            CommandId::CopyMessage => self.copy_last_assistant_message(),
            CommandId::CopyVisible => self.copy_visible_transcript(),
            CommandId::Fork => {
                let Some(session_id) = self.transcript.session_id else {
                    self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
                    return;
                };
                self.create_session_with_parent(None, Some(session_id));
            }
            CommandId::Children => self.open_child_sessions_picker(),
            CommandId::Parent => self.open_parent_session(),
            CommandId::Diagnostics => {
                self.flash_success(self.current_diagnostics_summary());
            }
            CommandId::Status => {
                self.flash_success(self.current_runtime_status_summary());
            }
            CommandId::Btw => self.handle_btw_command(args),
            CommandId::Queue => self.handle_queue_command(args),
        }
    }

    fn execute_runtime_tool_prompt(&mut self, tool_name: &str, args: &str) {
        let target_session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
            .unwrap_or(-1);
        let prompt = match self
            .backend
            .runtime_tool_prompt(target_session_id, tool_name, args)
        {
            Ok(prompt) => prompt,
            Err(error) => {
                self.flash_error(error.to_string());
                return;
            }
        };
        if prompt.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-user-command-empty"));
            return;
        }
        let draft = ComposerDraft {
            text: prompt,
            ..ComposerDraft::default()
        };
        match self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
        {
            Some(session_id) => self.request_submit_message(session_id, draft),
            None => self.create_session(Some(draft)),
        }
    }

    /// `/btw <question>` forks a child session and submits the question
    /// there without touching the parent transcript. The parent run keeps
    /// running (or stays idle) untouched; the user can switch to the new
    /// session via the sessions pane to read the answer.
    fn handle_btw_command(&mut self, args: &str) {
        let question = args.trim();
        if question.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => "/btw <question>"),
            ));
            return;
        }
        let parent_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id());
        let title = format!("btw: {}", derive_session_title(&self.i18n, question));
        let prompt = question.to_string();
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let create = backend.create_session(title, parent_id).await;
            match create {
                Ok(session) => {
                    let session_id = session.id;
                    let parts = vec![PartContent::text(prompt)];
                    let result = backend
                        .submit_parts_message_with_options(session_id, parts, options)
                        .await
                        .map_err(|error| error.to_string());
                    // Reuse the existing run-submitted message — the
                    // handler will route the new session into the UI if
                    // appropriate, otherwise just refresh the list.
                    let _ = tx.send(AppMessage::SessionMessageSubmitted {
                        session_id,
                        draft: ComposerDraft::default(),
                        result,
                    });
                }
                Err(err) => {
                    let _ = tx.send(AppMessage::SessionCreated {
                        submit_draft: None,
                        result: Err(err.to_string()),
                    });
                }
            }
        });
        self.flash_info(ui_text::t(&self.i18n, "flash-btw-spawned"));
    }

    /// `/queue [list|clear|pop]` — inspect or manage the pending message
    /// queue.
    ///   * `list` (default): flash a one-liner showing how many entries
    ///     and the first preview.
    ///   * `clear`: drop every queued message.
    ///   * `pop`: pull the head editable entry back into the editor.
    fn handle_queue_command(&mut self, args: &str) {
        let action = args.trim().to_lowercase();
        match action.as_str() {
            "" | "list" | "ls" | "show" => {
                if self.queue.is_empty() {
                    self.flash_info(ui_text::t(&self.i18n, "flash-queue-empty"));
                    return;
                }
                let preview = self.queue.first_preview(60).unwrap_or_default();
                self.flash_info(self.i18n.text_args(
                    "flash-queue-list",
                    &crate::fl_args!(
                        "count" => self.queue.len() as i64,
                        "preview" => preview,
                    ),
                ));
            }
            "clear" | "drop" => {
                if self.queue.is_empty() {
                    self.flash_info(ui_text::t(&self.i18n, "flash-queue-empty"));
                    return;
                }
                let count = self.queue.len();
                self.queue.clear();
                self.flash_success(self.i18n.text_args(
                    "flash-queue-cleared",
                    &crate::fl_args!("count" => count as i64),
                ));
            }
            "pop" | "edit" => {
                if !self.try_pop_queue_into_editor() {
                    self.flash_info(ui_text::t(&self.i18n, "flash-queue-empty"));
                }
            }
            _ => {
                self.flash_warning(self.i18n.text_args(
                    "flash-command-usage",
                    &crate::fl_args!("usage" => "/queue [list|clear|pop]"),
                ));
            }
        }
    }

    fn handle_rename_command(&mut self, spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            if self.current_or_selected_session_id().is_some() {
                self.open_rename_session_overlay();
            } else {
                self.flash_warning(self.i18n.text_args(
                    "flash-command-usage",
                    &crate::fl_args!("usage" => spec.invocation()),
                ));
            }
            return;
        }
        self.submit_session_rename(trimmed);
    }

    fn handle_timeline_command(&mut self, spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        let limit = if trimmed.is_empty() {
            TIMELINE_EVENT_LIMIT
        } else {
            match trimmed.parse::<u64>() {
                Ok(value) if value > 0 => value,
                _ => {
                    self.flash_warning(self.i18n.text_args(
                        "flash-command-usage",
                        &crate::fl_args!("usage" => spec.invocation()),
                    ));
                    return;
                }
            }
        };
        self.open_timeline_overlay(limit);
    }

    fn handle_plugins_command(&mut self, _spec: &'static CommandSpec, args: &str) {
        self.open_plugin_workbench(args.trim());
    }

    fn handle_settings_command(&mut self, args: &str) {
        self.open_settings_studio(args.trim());
    }

    fn handle_permissions_command(&mut self, args: &str) {
        match args.trim() {
            "" | "new" | "session" | "current" => self.open_current_session_permission_studio(),
            "list" | "rules" | "manage" => self.open_permission_rule_picker(""),
            other => self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => format!("/permissions [new|list] · got `{other}`")),
            )),
        }
    }

    fn handle_review_command(&mut self, args: &str) {
        self.execute_runtime_tool_prompt("review", args);
    }

    fn handle_snapshot_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("list") {
            self.open_inspector_picker(
                ui_text::snapshot_picker_title(&self.i18n),
                ui_text::snapshot_picker_prompt(&self.i18n),
                "",
                self.backend.snapshot_inspector_rows(),
            );
            return;
        }

        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        let (action, rest) = split_command_args_once(trimmed).unwrap_or((trimmed, ""));
        match action.to_ascii_lowercase().as_str() {
            "enter" => {
                let argument = rest.trim();
                let result = if argument.is_empty() {
                    self.backend.enter_snapshot(session_id, None, None)
                } else {
                    self.backend
                        .enter_snapshot(session_id, Some(argument.to_string()), None)
                };
                match result {
                    Ok(output) => {
                        let mut message = ui_text::snapshot_ready_message(
                            &self.i18n,
                            output.path.as_str(),
                            output.branch.as_deref(),
                        );
                        if let Some(backend) = output.backend.as_deref() {
                            message.push_str(format!(" | backend={backend}").as_str());
                        }
                        if let Some(note) = output.note.as_deref() {
                            message.push_str(format!(" | {note}").as_str());
                        }
                        self.flash_success(message)
                    }
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "attach" => {
                let path = rest.trim();
                if path.is_empty() {
                    self.flash_warning(self.i18n.text_args(
                        "flash-command-usage",
                        &crate::fl_args!("usage" => "/snapshot attach <path>"),
                    ));
                    return;
                }
                match self
                    .backend
                    .enter_snapshot(session_id, None, Some(path.to_string()))
                {
                    Ok(output) => {
                        let mut message = ui_text::snapshot_attached_message(
                            &self.i18n,
                            output.path.as_str(),
                            output.branch.as_deref(),
                        );
                        if let Some(backend) = output.backend.as_deref() {
                            message.push_str(format!(" | backend={backend}").as_str());
                        }
                        if let Some(note) = output.note.as_deref() {
                            message.push_str(format!(" | {note}").as_str());
                        }
                        self.flash_success(message)
                    }
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "exit" | "leave" => {
                let exit_args = rest.trim();
                let (mode, extra) = split_command_args_once(exit_args).unwrap_or((exit_args, ""));
                match mode.to_ascii_lowercase().as_str() {
                    "" | "keep" => match self.backend.exit_snapshot(session_id, "keep".to_string(), false) {
                        Ok(output) => self.flash_success(ui_text::snapshot_exit_message(
                            &self.i18n,
                            output.action.as_deref(),
                            output.path.as_str(),
                        )),
                        Err(error) => self.flash_error(error.to_string()),
                    },
                    "remove" => {
                        let discard_changes =
                            matches!(extra.trim().to_ascii_lowercase().as_str(), "force" | "discard");
                        self.open_snapshot_remove_confirm(session_id, discard_changes);
                    }
                    _ => {
                        self.flash_warning(self.i18n.text_args(
                            "flash-command-usage",
                            &crate::fl_args!("usage" => "/snapshot exit [keep|remove [force]]"),
                        ));
                    }
                }
            }
            _ => self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => "/snapshot [list|enter [name]|attach <path>|exit [keep|remove [force]]]"),
            )),
        }
    }

    fn handle_commit_command(&mut self, args: &str) {
        let message = args.trim();
        if message.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => "/commit <message>"),
            ));
            return;
        }

        match self.block_on_async(self.backend.create_commit(message.to_string())) {
            Ok((commit, summary)) => {
                self.flash_success(ui_text::commit_created_message(
                    &self.i18n,
                    &commit[..commit.len().min(12)],
                    summary.as_str(),
                ));
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn handle_pr_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => "/pr <title> [--body <text>] [--base <branch>] [--head <branch>]"),
            ));
            return;
        }

        let (title, body, base, head) = match parse_pr_command_args(trimmed) {
            Ok(parsed) => parsed,
            Err(_) => {
                self.flash_warning(self.i18n.text_args(
                    "flash-command-usage",
                    &crate::fl_args!("usage" => "/pr <title> [--body <text>] [--base <branch>] [--head <branch>]"),
                ));
                return;
            }
        };

        match self.block_on_async(self.backend.create_pr(title, body, base, head)) {
            Ok(url) => self.flash_success(ui_text::pull_request_created_message(
                &self.i18n,
                url.as_str(),
            )),
            Err(error) => self.flash_error(error),
        }
    }

    fn handle_export_command(&mut self, args: &str) {
        let requested_path = non_empty_owned(args.to_string()).map(|value| {
            self.backend
                .resolve_workspace_path(Path::new(value.as_str()))
        });
        self.pending_ui_action = Some(UiAction::ExportTranscript {
            path: requested_path,
        });
    }

    fn handle_memory_command(&mut self, spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("list") {
            match self.backend.memory_index_path() {
                Ok(path) => self.pending_ui_action = Some(UiAction::OpenPath { path }),
                Err(error) => self.flash_error(error.to_string()),
            }
            return;
        }

        let (action, rest) = split_command_args_once(trimmed).unwrap_or((trimmed, ""));
        let action = action.to_ascii_lowercase();
        match action.as_str() {
            "list" if rest.is_empty() => match self.backend.memory_index_path() {
                Ok(path) => self.pending_ui_action = Some(UiAction::OpenPath { path }),
                Err(error) => self.flash_error(error.to_string()),
            },
            "edit" | "open" => {
                let result = if rest.is_empty() {
                    self.backend.memory_index_path()
                } else {
                    self.backend.memory_entry_path(rest)
                };
                match result {
                    Ok(path) => self.pending_ui_action = Some(UiAction::OpenPath { path }),
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "forget" | "rm" | "remove" | "delete" if !rest.is_empty() => {
                match self.backend.forget_memory(rest) {
                    Ok(()) => self.flash_success(
                        self.i18n
                            .text_args("flash-memory-forgotten", &crate::fl_args!("name" => rest)),
                    ),
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            _ => self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => spec.invocation()),
            )),
        }
    }

    fn handle_sessions_command(&mut self, _spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.open_resume_session_picker();
            return;
        }

        let next_mode = match trimmed.to_ascii_lowercase().as_str() {
            "all" | "recent" => SessionViewMode::All,
            "roots" | "root" => SessionViewMode::Roots,
            "subtree" | "tree" | "branch" => SessionViewMode::Subtree,
            _ => {
                self.open_resume_session_picker_with_query(trimmed);
                return;
            }
        };
        self.set_session_view_mode(next_mode);
        self.open_resume_session_picker();
    }

    fn set_session_view_mode(&mut self, mode: SessionViewMode) {
        if mode == SessionViewMode::Subtree && self.current_or_selected_session_id().is_none() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }
        self.sessions.view_mode = mode;
        self.flash_success(self.i18n.text_args(
            "flash-session-view-mode",
            &crate::fl_args!("mode" => self.current_session_view_summary()),
        ));
        self.request_sessions(false);
    }

    fn cycle_session_view_mode(&mut self) {
        self.set_session_view_mode(self.sessions.view_mode.next());
    }

    fn rebuild_visible_sessions(&mut self, preferred_id: Option<i64>) {
        self.sessions.list.items = build_visible_session_items(
            self.sessions.source_items.as_slice(),
            self.sessions.view_mode,
            self.sessions.search_query.as_str(),
        );
        self.sessions.has_more = false;
        self.sessions.next_cursor = None;
        self.sessions.clamp_selection();
        if let Some(id) = preferred_id {
            let _ = self.sessions.select_by_id(id);
        }
        self.sessions.clamp_selection();
    }

    fn submit_session_rename(&mut self, title: &str) -> bool {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-title-empty"));
            return false;
        }
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return false;
        };
        self.request_session_rename(session_id, trimmed.to_string());
        true
    }

    fn clear_provider_model_overrides(&mut self) {
        self.run_options.clear_model_stack();
    }

    fn block_on_async<F, T, E>(&self, fut: F) -> UiResult<T>
    where
        F: std::future::Future<Output = std::result::Result<T, E>>,
        E: ToString,
    {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => match handle.runtime_flavor() {
                tokio::runtime::RuntimeFlavor::MultiThread => {
                    tokio::task::block_in_place(|| handle.block_on(fut))
                        .map_err(|error| error.to_string())
                }
                _ => Err(
                    "cannot synchronously wait for async work inside the current-thread runtime"
                        .to_string(),
                ),
            },
            Err(_) => {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?;
                runtime.block_on(fut).map_err(|error| error.to_string())
            }
        }
    }

    fn current_runtime_status_summary(&self) -> String {
        let mut parts = vec![
            self.run_options
                .summary(&self.i18n)
                .unwrap_or_else(|| ui_text::t(&self.i18n, "runtime-status-default")),
        ];
        parts.extend(self.current_execution_context_parts(false));
        parts.push(self.workspace_context_label());
        parts.push(self.i18n.text_args(
            "runtime-status-keys",
            &crate::fl_args!(
                "queue" => self.keybindings.queue.len() as i64,
                "send" => self.keybindings.submit.len() as i64,
            ),
        ));
        parts.push(self.i18n.text_args(
            "runtime-status-statusline",
            &crate::fl_args!(
                "value" => ui_text::t(
                    &self.i18n,
                    if self.backend.plugin_statusline_segments().is_empty() {
                        "runtime-status-statusline-default"
                    } else {
                        "runtime-status-statusline-plugin"
                    },
                )
            ),
        ));
        let tui_blocks = self.backend.plugin_tui_content_blocks().len();
        if tui_blocks > 0 {
            parts.push(self.i18n.text_args(
                "runtime-status-tui-blocks",
                &crate::fl_args!("count" => tui_blocks as i64),
            ));
        }
        if let Some(theme) = self.plugin_theme.as_ref() {
            parts.push(self.i18n.text_args(
                "runtime-status-theme",
                &crate::fl_args!("value" => theme.id.clone()),
            ));
        }
        self.i18n.text_args(
            "flash-runtime-status",
            &crate::fl_args!("summary" => parts.join(" | ")),
        )
    }

    fn current_diagnostics_summary(&self) -> String {
        let runtime = self
            .run_options
            .summary(&self.i18n)
            .unwrap_or_else(|| ui_text::t(&self.i18n, "runtime-status-default"));
        let cwd = self.backend.workspace_root().display().to_string();
        let session = self
            .transcript
            .session_id
            .map(|id| format!("#{id}"))
            .unwrap_or_else(|| "<none>".to_owned());
        self.i18n.text_args(
            "flash-diagnostics-summary",
            &crate::fl_args!(
                "cwd" => cwd,
                "session" => session,
                "queue" => self.queue.len() as i64,
                "runtime" => runtime,
            ),
        )
    }

    fn current_session_view_summary(&self) -> String {
        self.sessions
            .view_mode
            .label(&self.i18n, self.sessions.subtree_root_id)
    }

    fn workspace_context_label(&self) -> String {
        self.i18n.text_args(
            "status-part-workspace",
            &crate::fl_args!("value" => self.backend.workspace_name()),
        )
    }

    fn current_execution_context_parts(&self, include_workspace_root: bool) -> Vec<String> {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return Vec::new();
        };

        let mut parts = Vec::new();
        parts.push(self.i18n.text_args(
            "status-part-state",
            &crate::fl_args!(
                "value" => ui_text::session_workflow_state_label(&self.i18n, execution)
            ),
        ));
        if let Some(agent_profile) = execution.execution.agent_profile.as_deref()
            && !agent_profile.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "status-part-agent",
                &crate::fl_args!("value" => agent_profile),
            ));
        }
        if let Some(skill_name) = execution.execution.active_skill_name.as_deref()
            && !skill_name.trim().is_empty()
        {
            parts.push(
                self.i18n
                    .text_args("status-part-skill", &crate::fl_args!("value" => skill_name)),
            );
        }
        if let Some(task_id) = execution.execution.task_id.as_deref()
            && !task_id.trim().is_empty()
        {
            parts.push(
                self.i18n
                    .text_args("status-part-task", &crate::fl_args!("value" => task_id)),
            );
        }
        if let Some(model_label) = execution_model_status_label(&execution.execution) {
            parts.push(self.i18n.text_args(
                "status-part-model",
                &crate::fl_args!("value" => model_label),
            ));
        }
        if let Some(thinking_mode) = execution.execution.model_thinking_mode.as_deref()
            && !thinking_mode.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "status-part-thinking",
                &crate::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            ));
        }
        if let Some(speed_mode) = execution.execution.model_speed_mode.as_deref()
            && !speed_mode.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "status-part-speed",
                &crate::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
            ));
        }
        if let Some(verbosity) = execution.execution.model_verbosity.as_deref()
            && !verbosity.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "status-part-verbosity",
                &crate::fl_args!("value" => verbosity),
            ));
        }
        if let Some(parallel_tool_calls) = execution.execution.model_parallel_tool_calls {
            parts.push(self.i18n.text_args(
                "status-part-parallel-tools",
                &crate::fl_args!(
                    "value" => ui_text::t(
                        &self.i18n,
                        if parallel_tool_calls {
                            "value-on"
                        } else {
                            "value-off"
                        },
                    )
                ),
            ));
        }
        if let Some(workspace_root) = execution.execution.effective_workspace_root.as_deref()
            && !workspace_root.trim().is_empty()
            && include_workspace_root
        {
            parts.push(self.i18n.text_args(
                "status-part-cwd",
                &crate::fl_args!("value" => workspace_root),
            ));
        }
        if !execution.execution.effective_permission.is_empty() {
            parts.push(self.i18n.text_args(
                "status-part-permission",
                &crate::fl_args!(
                    "value" => permission_override_summary(
                        &self.i18n,
                        &execution.execution.effective_permission,
                    )
                ),
            ));
        }
        let (permission_count, user_input_count) =
            pending_interactive_counts_for_execution(execution);
        if permission_count > 0 {
            parts.push(self.i18n.text_args(
                "status-part-permissions",
                &crate::fl_args!("count" => permission_count as i64),
            ));
        }
        if user_input_count > 0 {
            parts.push(self.i18n.text_args(
                "status-part-user-input",
                &crate::fl_args!("count" => user_input_count as i64),
            ));
        }
        parts
    }

    fn current_session_status_parts(&self) -> Vec<String> {
        let fallback_model = || {
            self.backend
                .resolved_model_for_run_options(&self.run_options.to_request())
                .ok()
                .map(|model| model_status_label(&model))
        };
        let fallback_agent = || self.backend.default_agent_name();

        if let Some(execution) = self.transcript.execution.as_ref() {
            let model_part =
                execution_model_status_label(&execution.execution).or_else(fallback_model);
            let agent = execution
                .execution
                .agent_profile
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .or_else(fallback_agent);
            let token_usage = status_line_token_usage(&execution.usage);
            let mut parts = Vec::new();
            if let Some(wait_state) = self.current_session_wait_state_text() {
                parts.push(wait_state);
            } else if self.transcript.submitting {
                parts.push(ui_text::t(&self.i18n, "transcript-header-busy"));
            } else if execution.run_state != SessionRunState::Idle {
                parts.push(ui_text::t(&self.i18n, "session-running"));
            }
            parts.extend(session_summary_status_parts(model_part, agent, token_usage));
            if let Some(thinking_mode) = execution.execution.model_thinking_mode.as_deref()
                && !thinking_mode.trim().is_empty()
            {
                parts.push(self.i18n.text_args(
                    "session-status-thinking",
                    &crate::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
                ));
            }
            if let Some(speed_mode) = execution.execution.model_speed_mode.as_deref()
                && !speed_mode.trim().is_empty()
            {
                parts.push(self.i18n.text_args(
                    "session-status-speed",
                    &crate::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
                ));
            }
            if !parts.is_empty() {
                return parts;
            }
        }

        let mut parts = session_summary_status_parts(
            self.run_options
                .model
                .as_ref()
                .map(model_status_label)
                .or_else(fallback_model),
            fallback_agent(),
            None,
        );
        if self.transcript.submitting {
            parts.insert(0, ui_text::t(&self.i18n, "transcript-header-busy"));
        }
        if let Some(thinking_mode) = self.run_options.thinking_mode.as_deref()
            && !thinking_mode.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "session-status-thinking",
                &crate::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            ));
        }
        if let Some(speed_mode) = self.run_options.speed_mode.as_deref()
            && !speed_mode.trim().is_empty()
        {
            parts.push(self.i18n.text_args(
                "session-status-speed",
                &crate::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
            ));
        }
        parts
    }

    fn jump_search_match(&mut self, forward: bool) {
        self.transcript.jump_search_match(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            forward,
        );
    }

    fn jump_to_message(&mut self, message_id: i64) {
        self.transcript.jump_to_message(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
            message_id,
        );
        self.focus = Focus::Transcript;
    }

    fn flush_input_buffers_if_due(&mut self, now: Instant) {
        self.composer.flush_pending_input_if_due(now);
        if let Some(search) = self.prompt_history_search.as_mut() {
            search.query.flush_pending_input_if_due(now);
            Self::refresh_prompt_history_search(&self.prompt_history, search);
        }
        self.sync_composer_suggestions();
        match &mut self.current_route {
            Route::Main => {}
            Route::Help(_) => {}
            Route::SettingsStudio(_) => {}
            Route::AgentStudio(dialog) => {
                if let Some(editor) = dialog.workbench.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::PermissionStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::PermissionRuleStudio(dialog) => {
                if let Some(editor) = dialog.workbench.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::SessionSearch(dialog) => dialog.input.flush_pending_input_if_due(now),
            Route::Picker(dialog) => dialog.input.flush_pending_input_if_due(now),
            Route::SessionModelChooser(dialog) => {
                dialog.input.flush_pending_input_if_due(now);
                Self::refresh_session_model_chooser_overlay(dialog, false, None);
            }
            Route::Timeline(dialog) => dialog.input.flush_pending_input_if_due(now),
            Route::PluginPolicyStudio(_) => {}
            Route::PluginWorkbench(dialog) => Self::flush_plugin_workbench_input(dialog, now),
            Route::ProviderStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::ModelCatalogStudio(dialog) => {
                if let Some(editor) = dialog.workbench.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
        }
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::AgentCreate(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::SettingsValueEdit(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::RuntimeSettingEdit(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::Choice(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                    Self::sync_choice_overlay_input(dialog, false);
                }
                Overlay::PermissionRuleEdit(dialog) => {
                    dialog.state.input.flush_pending_input_if_due(now);
                }
                Overlay::FileAttach(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::PathBrowser(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                    Self::refresh_path_browser_overlay_with_root(
                        self.backend.workspace_root(),
                        dialog,
                    );
                }
                Overlay::UserInputReply(dialog) => {
                    if dialog.editing_custom {
                        dialog.custom_input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::SessionSearch(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::Picker(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::Timeline(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.workbench.editor.as_mut() {
                        editor.input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::Confirm(_) => {}
                Overlay::Permission(_) => {}
            }
        }
    }

    fn refresh_file_attach_overlay(&self, dialog: &mut FileAttachOverlay) {
        dialog.items = self
            .backend
            .search_workspace_files(dialog.input.text(), 24)
            .unwrap_or_default();
        dialog.clamp_selection();
    }

    fn try_stage_pasted_path(&mut self, pasted: &str) -> bool {
        let Some(path) = normalize_pasted_path(pasted) else {
            return false;
        };
        let resolved = self.backend.resolve_workspace_path(path.as_path());
        if !resolved.exists() || !resolved.is_file() {
            return false;
        }

        match self.stage_attachment_from_path(path.as_path(), false) {
            Ok(()) => true,
            Err(error) => {
                self.flash_warning(error);
                true
            }
        }
    }

    fn stage_large_paste(&mut self, text: String) {
        let char_count = text.chars().count();
        let label = ui_text::staged_paste_label(&self.i18n, char_count, text.contains('\n'));
        let placeholder = self.make_unique_composer_placeholder(ui_text::staged_paste_placeholder(
            &self.i18n, char_count,
        ));
        self.composer.insert_element(placeholder.as_str());
        self.composer_items
            .push(ComposerItem::LargePaste(StagedPaste {
                placeholder,
                label,
                text,
            }));
        self.flash_success(ui_text::t(&self.i18n, "flash-large-paste-staged"));
    }

    fn stage_attachment_from_path(&mut self, path: &Path, is_temp: bool) -> UiResult<()> {
        let resolved = self.backend.resolve_workspace_path(path);
        let metadata = std::fs::metadata(&resolved).map_err(|error| {
            ui_text::attachment_inspect_failed_message(
                &self.i18n,
                resolved.as_path(),
                error.to_string().as_str(),
            )
        })?;
        let prepared = self
            .backend
            .prepare_attachment_from_path(path)
            .map_err(|error| error.to_string())?;
        let label = attachment_chip_label(
            &self.i18n,
            resolved.as_path(),
            prepared.kind,
            prepared.width,
            prepared.height,
            metadata.len(),
        );
        let placeholder = self.make_unique_composer_placeholder(attachment_placeholder_base(
            &self.i18n,
            resolved.as_path(),
            prepared.kind,
        ));

        self.composer.insert_element(placeholder.as_str());
        self.composer_items
            .push(ComposerItem::Attachment(StagedAttachment {
                path: resolved.clone(),
                placeholder,
                label,
                is_temp,
            }));
        self.flash_success(self.i18n.text_args(
            "flash-attached",
            &crate::fl_args!("path" => resolved.display().to_string()),
        ));
        Ok(())
    }

    fn make_unique_composer_placeholder(&self, base: String) -> String {
        let mut existing = self
            .composer_items
            .iter()
            .map(|item| item.placeholder().to_string())
            .collect::<HashSet<_>>();
        existing.extend(self.composer.element_texts());
        if !existing.contains(base.as_str()) {
            return base;
        }

        let stem = base.strip_suffix(']').unwrap_or(base.as_str());
        for index in 2.. {
            let candidate = if base.ends_with(']') {
                format!("{stem} #{index}]")
            } else {
                format!("{stem} #{index}")
            };
            if !existing.contains(candidate.as_str()) {
                return candidate;
            }
        }

        base
    }

    fn sync_composer_items_with_editor(&mut self) {
        let mut by_placeholder = std::mem::take(&mut self.composer_items)
            .into_iter()
            .map(|item| (item.placeholder().to_string(), item))
            .collect::<BTreeMap<_, _>>();

        let mut synced = Vec::new();
        for placeholder in self.composer.element_texts() {
            if let Some(item) = by_placeholder.remove(placeholder.as_str()) {
                synced.push(item);
            }
        }

        for (_, item) in by_placeholder {
            cleanup_temporary_composer_item(&item);
        }

        self.composer_items = synced;
    }

    fn current_draft_slot(&self) -> DraftSlot {
        self.transcript
            .session_id
            .map(DraftSlot::Session)
            .unwrap_or(DraftSlot::NewSession)
    }

    fn current_slot_has_in_flight_draft(&self) -> bool {
        if !self.composer.text().trim().is_empty() || !self.composer_items.is_empty() {
            return false;
        }

        match self.current_draft_slot() {
            DraftSlot::Session(session_id) => self.submitting_session_ids.contains(&session_id),
            DraftSlot::NewSession => {
                self.transcript.submitting && self.transcript.pending_restore_draft.is_some()
            }
        }
    }

    fn clear_composer_state(&mut self) {
        self.composer.clear();
        self.composer_items.clear();
        self.slash_command_suggestions = None;
        self.dismissed_slash_command_suggestions_for = None;
        self.file_mention_suggestions = None;
        self.dismissed_file_mention_suggestions_for = None;
        self.prompt_history_search = None;
        self.selected_composer_item = None;
    }

    fn current_composer_draft(&mut self) -> ComposerDraft {
        self.composer.flush_all_pending_input();
        self.sync_composer_items_with_editor();
        ComposerDraft {
            text: self.composer.text().to_string(),
            items: self.composer_items.clone(),
            elements: self
                .composer
                .draft_elements()
                .into_iter()
                .filter_map(|range| {
                    self.composer.text().get(range.clone()).map(|placeholder| {
                        ComposerDraftElement {
                            placeholder: placeholder.to_string(),
                            range,
                        }
                    })
                })
                .collect(),
        }
    }

    fn sync_current_draft_slot(&mut self) {
        if self.current_slot_has_in_flight_draft() {
            return;
        }
        let slot = self.current_draft_slot();
        let draft = self.current_composer_draft();
        self.set_draft_for_slot(slot, draft);
    }

    fn set_draft_for_slot(&mut self, slot: DraftSlot, draft: ComposerDraft) {
        if self.draft_store.set(slot, draft) {
            self.draft_store_dirty = true;
        }
    }

    fn clear_draft_for_slot(&mut self, slot: DraftSlot) {
        if self.draft_store.clear(slot) {
            self.draft_store_dirty = true;
        }
    }

    fn restore_draft_for_slot(&mut self, slot: DraftSlot) {
        if let DraftSlot::Session(session_id) = slot
            && self.submitting_session_ids.contains(&session_id)
        {
            return;
        }
        if let Some(draft) = self.draft_store.get(slot).cloned() {
            self.restore_composer_draft(draft);
        }
    }

    fn try_persist_draft_store(&mut self, force: bool) -> UiResult<()> {
        if !self.draft_store_dirty {
            return Ok(());
        }
        if !force
            && self.draft_store_last_persist_at.elapsed()
                < Duration::from_millis(DRAFT_PERSIST_INTERVAL_MS)
        {
            return Ok(());
        }

        self.draft_store
            .persist(&self.draft_store_path)
            .map_err(|error| {
                ui_text::composer_drafts_save_failed_message(&self.i18n, error.to_string().as_str())
            })?;
        self.draft_store_dirty = false;
        self.draft_store_last_persist_at = Instant::now();
        self.draft_store_reported_error = None;
        Ok(())
    }

    fn persist_draft_store_with_feedback(&mut self, force: bool) {
        if let Err(error) = self.try_persist_draft_store(force) {
            self.report_draft_store_error(error);
        }
    }

    fn report_draft_store_error(&mut self, error: String) {
        let should_report = self.draft_store_reported_error.as_deref() != Some(error.as_str());
        self.draft_store_reported_error = Some(error.clone());
        if should_report {
            self.flash_error(error);
        }
    }

    fn record_prompt_history_from_draft(&mut self, draft: &ComposerDraft) {
        if !draft.items.is_empty() || !draft.elements.is_empty() {
            return;
        }
        let Some(text) = PromptHistory::normalized_text(draft.text.as_str()) else {
            return;
        };
        self.reset_prompt_history_recall();
        if !self.prompt_history.push(text) {
            return;
        }
        if let Err(error) = self.prompt_history.persist(&self.prompt_history_path) {
            self.report_prompt_history_error(ui_text::prompt_history_save_failed_message(
                &self.i18n,
                error.to_string().as_str(),
            ));
        } else {
            self.prompt_history_reported_error = None;
        }
    }

    fn report_prompt_history_error(&mut self, error: String) {
        let should_report = self.prompt_history_reported_error.as_deref() != Some(error.as_str());
        self.prompt_history_reported_error = Some(error.clone());
        if should_report {
            self.flash_error(error);
        }
    }

    fn reset_prompt_history_recall(&mut self) {
        self.prompt_history_recall_original = None;
        self.prompt_history_recall_index = None;
    }

    fn replace_composer_draft(&mut self, draft: ComposerDraft) {
        cleanup_temporary_composer_items(self.composer_items.as_slice());
        self.clear_composer_state();
        self.restore_composer_draft(draft);
    }

    fn recall_prompt_history(&mut self, direction: PromptHistoryDirection) {
        self.composer.flush_all_pending_input();
        self.sync_composer_items_with_editor();
        if !self.composer_items.is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-prompt-history-items"));
            return;
        }
        if self.prompt_history.is_empty() {
            self.flash_info(ui_text::t(&self.i18n, "flash-prompt-history-empty"));
            return;
        }

        match direction {
            PromptHistoryDirection::Older => self.recall_older_prompt_history(),
            PromptHistoryDirection::Newer => self.recall_newer_prompt_history(),
        }
    }

    fn recall_older_prompt_history(&mut self) {
        let len = self.prompt_history.len();
        if self.prompt_history_recall_index.is_none() {
            self.prompt_history_recall_original = Some(self.current_composer_draft());
            self.prompt_history_recall_index = Some(len);
        }

        let Some(current) = self.prompt_history_recall_index else {
            return;
        };
        if current == 0 {
            return;
        }
        let next = current - 1;
        let Some(text) = self.prompt_history.get(next).map(str::to_string) else {
            return;
        };
        self.prompt_history_recall_index = Some(next);
        self.replace_composer_draft(ComposerDraft {
            text,
            ..ComposerDraft::default()
        });
    }

    fn recall_newer_prompt_history(&mut self) {
        let Some(current) = self.prompt_history_recall_index else {
            return;
        };
        let next = current + 1;
        if next >= self.prompt_history.len() {
            self.prompt_history_recall_index = None;
            let draft = self
                .prompt_history_recall_original
                .take()
                .unwrap_or_default();
            self.replace_composer_draft(draft);
            return;
        }

        let Some(text) = self.prompt_history.get(next).map(str::to_string) else {
            return;
        };
        self.prompt_history_recall_index = Some(next);
        self.replace_composer_draft(ComposerDraft {
            text,
            ..ComposerDraft::default()
        });
    }

    fn cleanup_temporary_draft_store_items(&self) {
        for draft in self.draft_store.drafts.values() {
            cleanup_temporary_composer_items(draft.items.as_slice());
        }
    }

    fn take_composer_draft(&mut self) -> ComposerDraft {
        let draft = self.current_composer_draft();
        self.clear_composer_state();
        draft
    }

    fn restore_composer_draft(&mut self, draft: ComposerDraft) {
        if self.composer.text().trim().is_empty() && self.composer_items.is_empty() {
            let ComposerDraft {
                text,
                items,
                elements,
            } = draft;
            self.composer.set_text(text);
            self.composer
                .set_elements(elements.into_iter().map(|element| element.range).collect());
            self.composer_items = items;
            self.sync_composer_items_with_editor();
            self.sync_composer_suggestions();
        }
    }

    fn apply_external_editor_text(&mut self, text: String) {
        let mut occupied = Vec::new();
        let mut retained = Vec::new();
        for item in std::mem::take(&mut self.composer_items) {
            if let Some(range) =
                find_placeholder_occurrence(text.as_str(), item.placeholder(), &occupied)
            {
                occupied.push(range.clone());
                retained.push((range, item));
            } else {
                cleanup_temporary_composer_item(&item);
            }
        }

        retained.sort_by_key(|(range, _)| range.start);
        let ranges = retained
            .iter()
            .map(|(range, _)| range.clone())
            .collect::<Vec<_>>();
        let kept = retained
            .into_iter()
            .map(|(_, item)| item)
            .collect::<Vec<_>>();

        self.composer.set_text(text);
        self.composer.set_elements(ranges);
        self.composer_items = kept;
        self.sync_composer_suggestions();
    }

    fn build_submission_parts(&self, draft: &ComposerDraft) -> UiResult<Vec<PartContent>> {
        let mut parts = Vec::new();

        let mut items_by_placeholder = draft
            .items
            .iter()
            .map(|item| (item.placeholder().to_string(), item))
            .collect::<BTreeMap<_, _>>();
        let mut elements = draft.elements.clone();
        elements.sort_by_key(|element| element.range.start);

        let mut cursor = 0;
        for element in elements {
            let start = min(element.range.start, draft.text.len());
            let end = min(element.range.end, draft.text.len());
            if cursor < start {
                push_submission_text(&mut parts, &draft.text[cursor..start]);
            }

            let actual_placeholder = draft
                .text
                .get(start..end)
                .ok_or_else(|| ui_text::composer_placeholder_range_invalid_error(&self.i18n))?;
            if actual_placeholder != element.placeholder {
                return Err(ui_text::composer_placeholder_out_of_sync_error(&self.i18n));
            }

            let item = items_by_placeholder
                .remove(element.placeholder.as_str())
                .ok_or_else(|| {
                    ui_text::composer_missing_staged_item_error(
                        &self.i18n,
                        element.placeholder.as_str(),
                    )
                })?;
            match item {
                ComposerItem::Attachment(attachment) => {
                    let prepared = self
                        .backend
                        .prepare_attachment_from_path(attachment.path.as_path())
                        .map_err(|error| error.to_string())?;
                    parts.push(PartContent::attachments(vec![prepared]));
                }
                ComposerItem::LargePaste(paste) => {
                    push_submission_text(&mut parts, paste.text.as_str());
                }
            }
            cursor = end;
        }

        if cursor < draft.text.len() {
            push_submission_text(&mut parts, &draft.text[cursor..]);
        }

        Ok(parts)
    }

    fn run_ui_action<B: RatatuiBackend>(
        &mut self,
        action: UiAction,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        match action {
            UiAction::EditComposerExternally => self.edit_composer_externally(terminal),
            UiAction::AttachClipboardImage => {
                self.attach_clipboard_image();
                Ok(())
            }
            UiAction::ExportTranscript { path } => {
                self.export_transcript_to_editor(terminal, path.as_deref())
            }
            UiAction::OpenPath { path } => self.open_path_in_editor(terminal, path.as_path()),
            UiAction::PageTranscript => self.page_transcript(terminal),
        }
    }

    fn edit_composer_externally<B: RatatuiBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<()> {
        self.composer.flush_all_pending_input();
        terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        terminal::suspend_stdio_terminal()?;
        let result = edit_text(self.composer.text());
        terminal::resume_terminal(terminal)?;
        match result {
            Ok(text) => {
                self.apply_external_editor_text(text);
                self.focus = Focus::Composer;
                self.flash_success(ui_text::t(&self.i18n, "flash-composer-updated"));
            }
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-external-editor-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
        Ok(())
    }

    fn open_path_in_editor<B: RatatuiBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
        path: &Path,
    ) -> Result<()> {
        terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        terminal::suspend_stdio_terminal()?;
        let result = open_path(path);
        terminal::resume_terminal(terminal)?;
        if let Err(error) = result {
            self.flash_error(self.i18n.text_args(
                "flash-external-editor-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
        }
        Ok(())
    }

    fn attach_clipboard_image(&mut self) {
        match paste_image_to_temp_png() {
            Ok((path, info)) => {
                let format_label = pasted_image_format(path.as_path()).label();
                if let Err(error) = self.stage_attachment_from_path(path.as_path(), true) {
                    let _ = std::fs::remove_file(path);
                    self.flash_error(error);
                } else {
                    self.flash_success(self.i18n.text_args(
                        "flash-clipboard-image-attached",
                        &crate::fl_args!(
                            "width" => info.width as i64,
                            "height" => info.height as i64,
                            "format" => format_label,
                        ),
                    ));
                }
            }
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-clipboard-image-attach-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
    }

    fn copy_loaded_transcript(&mut self) {
        let text = self.transcript_export_text();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-loaded-transcript"));
            return;
        }

        match set_clipboard_text(text.as_str()) {
            Ok(()) => self.flash_success(ui_text::t(&self.i18n, "flash-copied-loaded-transcript")),
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-clipboard-copy-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
    }

    fn copy_last_assistant_message(&mut self) {
        let Some(message) = self
            .transcript
            .messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Assistant)
        else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-assistant-message"));
            return;
        };

        let Some(text) = assistant_message_text(message) else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-assistant-message-text"));
            return;
        };

        match set_clipboard_text(text.as_str()) {
            Ok(()) => self.flash_success(ui_text::t(&self.i18n, "flash-copied-assistant-message")),
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-clipboard-copy-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
    }

    fn copy_visible_transcript(&mut self) {
        let text = self.visible_transcript_text();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-visible-transcript"));
            return;
        }

        match set_clipboard_text(text.as_str()) {
            Ok(()) => self.flash_success(ui_text::t(&self.i18n, "flash-copied-visible-transcript")),
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-clipboard-copy-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
    }

    fn export_transcript_to_editor<B: RatatuiBackend>(
        &mut self,
        terminal: &mut Terminal<B>,
        requested_path: Option<&Path>,
    ) -> Result<()> {
        let text = self.transcript_export_markdown();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-loaded-transcript"));
            return Ok(());
        }

        let path = match self.resolve_transcript_export_path(requested_path) {
            Ok(path) => path,
            Err(error) => {
                self.flash_error(self.i18n.text_args(
                    "flash-transcript-export-failed",
                    &crate::fl_args!("error" => error),
                ));
                return Ok(());
            }
        };

        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
            return Ok(());
        }

        if let Err(error) = std::fs::write(&path, text) {
            self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
            return Ok(());
        }

        terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        terminal::suspend_stdio_terminal()?;
        let result = open_path(path.as_path());
        terminal::resume_terminal(terminal)?;

        match result {
            Ok(()) => self.flash_success(self.i18n.text_args(
                "flash-transcript-exported",
                &crate::fl_args!("path" => path.display().to_string()),
            )),
            Err(error) => self.flash_error(self.i18n.text_args(
                "flash-transcript-export-failed",
                &crate::fl_args!("error" => error.to_string()),
            )),
        }
        Ok(())
    }

    fn page_transcript<B: RatatuiBackend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        let text = self.transcript_pager_text();
        if text.trim().is_empty() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-loaded-transcript"));
            return Ok(());
        }

        terminal
            .flush()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        terminal::suspend_stdio_terminal()?;
        let result = page_text(text.as_str());
        terminal::resume_terminal(terminal)?;

        if let Err(error) = result {
            self.flash_error(self.i18n.text_args(
                "flash-transcript-pager-failed",
                &crate::fl_args!("error" => error.to_string()),
            ));
        }

        Ok(())
    }

    fn transcript_export_text(&self) -> String {
        if self.transcript.messages.is_empty() {
            return String::new();
        }

        self.transcript
            .messages
            .iter()
            .map(|message| {
                render_message_export(
                    message,
                    u16::MAX,
                    &self.i18n,
                    TranscriptDetailDefaults {
                        tool_output_expanded: true,
                        thinking_expanded: self
                            .transcript
                            .detail_expanded_by_default
                            .thinking_expanded,
                    },
                )
                .into_iter()
                .map(|line| line.text)
                .collect::<Vec<_>>()
                .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn transcript_pager_text(&self) -> String {
        let body = self.transcript_export_text();
        if body.trim().is_empty() {
            return String::new();
        }

        let mut lines = Vec::new();
        lines.push(
            self.current_or_selected_session_title()
                .unwrap_or_else(|| ui_text::t(&self.i18n, "pane-transcript")),
        );
        if let Some(session_id) = self.transcript.session_id {
            lines.push(format!("#{}", session_id));
        }

        let mut meta = Vec::new();
        if let Some(execution) = self.transcript.execution.as_ref() {
            if let Some(parent_id) = execution.session.parent_id {
                meta.push(self.i18n.text_args(
                    "session-summary-parent",
                    &crate::fl_args!("id" => parent_id),
                ));
            }
            if execution.session.child_session_count > 0 {
                meta.push(self.i18n.text_args(
                    "session-summary-children",
                    &crate::fl_args!("count" => execution.session.child_session_count as i64),
                ));
            }
        }
        meta.extend(self.current_lineage_context_parts());
        meta.push(self.current_session_view_summary());
        if let Some(summary) = self.run_options.summary(&self.i18n) {
            meta.push(summary);
        }
        if !meta.is_empty() {
            lines.push(meta.join(" | "));
        }
        lines.push(String::new());
        lines.push(body);
        lines.join("\n")
    }

    fn transcript_export_markdown(&self) -> String {
        render_transcript_export_markdown(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
            self.transcript.execution.as_ref(),
            self.transcript.messages.as_slice(),
            self.transcript.has_more_older,
        )
    }

    fn resolve_transcript_export_path(&self, requested_path: Option<&Path>) -> UiResult<PathBuf> {
        if let Some(path) = requested_path {
            if path.exists() && path.is_dir() {
                return Err(ui_text::transcript_export_path_is_directory_error(
                    &self.i18n, path,
                ));
            }
            return Ok(path.to_path_buf());
        }

        let session_id = self.transcript.session_id.unwrap_or_default();
        let timestamp = Local::now().format("%Y%m%d-%H%M%S");
        Ok(std::env::temp_dir().join(format!("agena-session-{session_id}-{timestamp}.md")))
    }

    fn visible_transcript_text(&mut self) -> String {
        let width = self.layout.transcript_body.width.max(1);
        let height = self.layout.transcript_body.height.max(1) as usize;
        if self.transcript.session_id.is_none() {
            return ui_text::no_session_selected_text(&self.i18n);
        }

        let rendered = self.transcript.rendered(width).clone();
        let start = min(self.transcript.scroll, rendered.lines.len());
        let end = min(start.saturating_add(height), rendered.lines.len());
        rendered.lines[start..end]
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn maybe_request_more_sessions(&mut self) {
        if self.sessions.should_load_more() {
            self.request_sessions(true);
        }
    }

    fn maybe_request_older_messages(&mut self) {
        if self.transcript.should_load_older()
            && let Some(session_id) = self.transcript.session_id
        {
            self.request_messages(session_id, MessageLoadMode::Prepend);
        }
    }

    fn flash(&mut self, level: FlashLevel, text: impl Into<String>) {
        self.flash = Some(FlashMessage {
            text: text.into(),
            level,
            expires_at: Instant::now() + Duration::from_secs(5),
        });
    }

    fn flash_error(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Error, text);
    }

    fn flash_warning(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Warning, text);
    }

    fn flash_success(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Success, text);
    }

    fn flash_info(&mut self, text: impl Into<String>) {
        self.flash(FlashLevel::Info, text);
    }
}

#[derive(Debug, Clone, Copy)]
enum OverlayCommit {
    TranscriptSearch,
}

fn settings_edit_title(i18n: &I18n, field: &str) -> String {
    i18n.text_args(
        "overlay-settings-edit-title",
        &crate::fl_args!("field" => field),
    )
}

fn editor_save_footer(i18n: &I18n, multiline: bool) -> String {
    ui_text::t(
        i18n,
        if multiline {
            "overlay-editor-footer-multiline"
        } else {
            "overlay-editor-footer-single-line"
        },
    )
}

fn settings_clear_label(i18n: &I18n) -> String {
    ui_text::t(i18n, "overlay-choice-clear-value")
}

fn settings_path_updated_message(i18n: &I18n, path: &str) -> String {
    i18n.text_args("flash-settings-updated", &crate::fl_args!("path" => path))
}

fn settings_path_cleared_message(i18n: &I18n, path: &str) -> String {
    i18n.text_args("flash-settings-cleared", &crate::fl_args!("path" => path))
}

fn agent_read_only_edit_message(i18n: &I18n) -> String {
    ui_text::t(i18n, "flash-agent-read-only-edit")
}

fn agent_read_only_permissions_message(i18n: &I18n) -> String {
    ui_text::t(i18n, "flash-agent-read-only-permissions")
}

fn provider_studio_no_auth_details_message(i18n: &I18n) -> String {
    ui_text::t(i18n, "flash-provider-studio-no-auth-details")
}

fn provider_draft_auth_action_message(
    i18n: &I18n,
    message: &crate::backend::ProviderDraftAuthMessage,
) -> String {
    match message {
        crate::backend::ProviderDraftAuthMessage::OpenaiBrowserStarted => {
            ui_text::t(i18n, "flash-provider-auth-openai-browser-started")
        }
        crate::backend::ProviderDraftAuthMessage::OpenaiDeviceStarted { user_code } => i18n
            .text_args(
                "flash-provider-auth-openai-device-started",
                &crate::fl_args!("code" => user_code.clone()),
            ),
        crate::backend::ProviderDraftAuthMessage::CopilotDeviceStarted { user_code } => i18n
            .text_args(
                "flash-provider-auth-copilot-device-started",
                &crate::fl_args!("code" => user_code.clone()),
            ),
        crate::backend::ProviderDraftAuthMessage::GitlabBrowserStarted => {
            ui_text::t(i18n, "flash-provider-auth-gitlab-browser-started")
        }
        crate::backend::ProviderDraftAuthMessage::OpenaiCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-openai-captured")
        }
        crate::backend::ProviderDraftAuthMessage::OpenaiPending => {
            ui_text::t(i18n, "flash-provider-auth-openai-pending")
        }
        crate::backend::ProviderDraftAuthMessage::CopilotPending => {
            ui_text::t(i18n, "flash-provider-auth-copilot-pending")
        }
        crate::backend::ProviderDraftAuthMessage::CopilotCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-copilot-captured")
        }
        crate::backend::ProviderDraftAuthMessage::GitlabCredentialCaptured => {
            ui_text::t(i18n, "flash-provider-auth-gitlab-captured")
        }
    }
}

fn provider_draft_auth_message_is_pending(
    message: &crate::backend::ProviderDraftAuthMessage,
) -> bool {
    matches!(
        message,
        crate::backend::ProviderDraftAuthMessage::OpenaiPending
            | crate::backend::ProviderDraftAuthMessage::CopilotPending
    )
}

fn provider_draft_auth_error_message(
    i18n: &I18n,
    error: &crate::backend::ProviderDraftAuthError,
) -> String {
    match error {
        crate::backend::ProviderDraftAuthError::UnsupportedInteractiveLogin => {
            ui_text::t(i18n, "flash-provider-auth-error-unsupported")
        }
        crate::backend::ProviderDraftAuthError::StartBrowserAuthFirst => {
            ui_text::t(i18n, "flash-provider-auth-error-start-browser-first")
        }
        crate::backend::ProviderDraftAuthError::StartDeviceAuthFirst => {
            ui_text::t(i18n, "flash-provider-auth-error-start-device-first")
        }
        crate::backend::ProviderDraftAuthError::RequiredField(field) => i18n.text_args(
            "flash-provider-auth-error-required-field",
            &crate::fl_args!("field" => provider_draft_auth_field_label(i18n, field)),
        ),
        crate::backend::ProviderDraftAuthError::Other(error) => error.clone(),
    }
}

fn provider_draft_auth_field_label(
    i18n: &I18n,
    field: &crate::backend::ProviderDraftAuthField,
) -> String {
    ui_text::t(
        i18n,
        match field {
            crate::backend::ProviderDraftAuthField::RedirectUri => "provider-field-redirect-uri",
            crate::backend::ProviderDraftAuthField::InstanceUrl => "provider-field-instance-url",
            crate::backend::ProviderDraftAuthField::CallbackUrl => "provider-field-callback-url",
        },
    )
}

fn provider_studio_save_result_message(
    i18n: &I18n,
    result: &crate::backend::ProviderStudioSaveResult,
) -> String {
    match result {
        crate::backend::ProviderStudioSaveResult::ProviderDraftSaved {
            provider_id,
            default_adapter,
            default_model,
        } => match default_model {
            Some(default_model) => i18n.text_args(
                "flash-provider-save-draft",
                &crate::fl_args!(
                    "provider" => provider_id.clone(),
                    "adapter" => default_adapter.clone(),
                    "model" => default_model.clone(),
                ),
            ),
            None => i18n.text_args(
                "flash-provider-save-draft-no-model",
                &crate::fl_args!(
                    "provider" => provider_id.clone(),
                    "adapter" => default_adapter.clone(),
                ),
            ),
        },
        crate::backend::ProviderStudioSaveResult::AdapterMatchesSaved {
            provider_id,
            adapter_id,
            listed_model_count,
            matched_model_count,
        } => i18n.text_args(
            "flash-provider-save-adapter-matches",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "listed" => *listed_model_count as i64,
                "matched" => *matched_model_count as i64,
            ),
        ),
        crate::backend::ProviderStudioSaveResult::ModelSaved {
            provider_id,
            adapter_id,
            model_id,
        } => i18n.text_args(
            "flash-provider-save-model",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "model" => model_id.clone(),
            ),
        ),
        crate::backend::ProviderStudioSaveResult::ConfiguredModelSaved {
            provider_id,
            adapter_id,
            model_id,
        } => i18n.text_args(
            "flash-provider-save-configured-model",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "model" => model_id.clone(),
            ),
        ),
        crate::backend::ProviderStudioSaveResult::ProviderDeleted { provider_id } => i18n
            .text_args(
                "flash-provider-delete-provider",
                &crate::fl_args!("provider" => provider_id.clone()),
            ),
        crate::backend::ProviderStudioSaveResult::AdapterDeleted {
            provider_id,
            adapter_id,
            removed_model_count,
        } => i18n.text_args(
            "flash-provider-delete-adapter",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "count" => *removed_model_count as i64,
            ),
        ),
        crate::backend::ProviderStudioSaveResult::ModelDeleted {
            provider_id,
            adapter_id,
            model_id,
        } => i18n.text_args(
            "flash-provider-delete-model",
            &crate::fl_args!(
                "provider" => provider_id.clone(),
                "adapter" => adapter_id.clone(),
                "model" => model_id.clone(),
            ),
        ),
    }
}

fn provider_studio_save_error_message(
    i18n: &I18n,
    error: &crate::backend::ProviderStudioSaveError,
) -> String {
    match error {
        crate::backend::ProviderStudioSaveError::Validation(error) => {
            provider_studio_save_validation_error_message(i18n, error)
        }
        crate::backend::ProviderStudioSaveError::ExistingProviderSettingsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-settings-object")
        }
        crate::backend::ProviderStudioSaveError::ProviderAdapterMustBeObject { adapter_id } => i18n
            .text_args(
                "flash-provider-save-error-adapter-object",
                &crate::fl_args!("adapter" => adapter_id.clone()),
            ),
        crate::backend::ProviderStudioSaveError::ProviderModelConfigMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-model-object")
        }
        crate::backend::ProviderStudioSaveError::ConfiguredProviderAdapterSettingsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-configured-adapter-object")
        }
        crate::backend::ProviderStudioSaveError::ConfiguredProviderAdapterModelsMustBeObject => {
            ui_text::t(i18n, "flash-provider-save-error-configured-models-object")
        }
        crate::backend::ProviderStudioSaveError::Other(error) => error.clone(),
    }
}

fn provider_studio_save_validation_error_message(
    i18n: &I18n,
    error: &crate::backend::ProviderStudioSaveValidationError,
) -> String {
    match error {
        crate::backend::ProviderStudioSaveValidationError::FieldRequired(field) => i18n.text_args(
            "flash-provider-save-error-required-field",
            &crate::fl_args!("field" => provider_studio_save_field_label(i18n, field)),
        ),
        crate::backend::ProviderStudioSaveValidationError::UnsupportedDefaultAdapter {
            auth_kind,
            adapter,
            supported,
        } => i18n.text_args(
            "flash-provider-save-error-unsupported-default-adapter",
            &crate::fl_args!(
                "auth" => provider_draft_auth_kind_label(i18n, auth_kind),
                "adapter" => adapter.clone(),
                "supported" => supported.clone(),
            ),
        ),
        crate::backend::ProviderStudioSaveValidationError::UnsupportedAdapters {
            auth_kind,
            adapters,
            supported,
        } => i18n.text_args(
            "flash-provider-save-error-unsupported-adapters",
            &crate::fl_args!(
                "auth" => provider_draft_auth_kind_label(i18n, auth_kind),
                "adapters" => adapters.join(", "),
                "supported" => supported.clone(),
            ),
        ),
        crate::backend::ProviderStudioSaveValidationError::ApiBaseUrlRequired => {
            ui_text::t(i18n, "flash-provider-save-error-api-base-url")
        }
        crate::backend::ProviderStudioSaveValidationError::GitlabApiKeyOrEnvRequired => {
            ui_text::t(i18n, "flash-provider-save-error-gitlab-token")
        }
        crate::backend::ProviderStudioSaveValidationError::CredentialBaseUrlRequired { issuer } => {
            i18n.text_args(
                "flash-provider-save-error-credential-base-url",
                &crate::fl_args!(
                    "issuer" => provider_credential_issuer_label_localized(i18n, *issuer),
                ),
            )
        }
        crate::backend::ProviderStudioSaveValidationError::CredentialServiceKeyEnvRequired {
            issuer,
        } => i18n.text_args(
            "flash-provider-save-error-credential-service-key-env",
            &crate::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer),
            ),
        ),
        crate::backend::ProviderStudioSaveValidationError::BedrockKeyPairRequired => {
            ui_text::t(i18n, "flash-provider-save-error-bedrock-key-pair")
        }
    }
}

fn provider_studio_save_field_label(
    i18n: &I18n,
    field: &crate::backend::ProviderStudioSaveField,
) -> String {
    ui_text::t(
        i18n,
        match field {
            crate::backend::ProviderStudioSaveField::ProviderId => "provider-field-provider-id",
            crate::backend::ProviderStudioSaveField::DefaultAdapter => {
                "provider-field-default-adapter"
            }
            crate::backend::ProviderStudioSaveField::AdapterId => "provider-field-adapter-id",
            crate::backend::ProviderStudioSaveField::ModelId => "provider-field-model-id",
            crate::backend::ProviderStudioSaveField::AuthMode => "provider-field-auth-mode",
            crate::backend::ProviderStudioSaveField::AuthSubtype => "provider-field-auth-subtype",
            crate::backend::ProviderStudioSaveField::CredentialIssuer => {
                "provider-field-auth-subtype"
            }
        },
    )
}

fn provider_credential_issuer_label_localized(i18n: &I18n, issuer: CredentialIssuer) -> String {
    ui_text::t(
        i18n,
        match issuer {
            CredentialIssuer::OpenaiChatgpt => "provider-issuer-openai-chatgpt-label",
            CredentialIssuer::GithubCopilot => "provider-issuer-github-copilot-label",
            CredentialIssuer::Gitlab => "provider-issuer-gitlab-label",
            CredentialIssuer::GoogleAdc => "provider-issuer-google-adc-label",
            CredentialIssuer::SapAiCore => "provider-issuer-sap-ai-core-label",
        },
    )
}

fn provider_draft_auth_kind_label(i18n: &I18n, auth_kind: &ProviderDraftAuthKind) -> String {
    match auth_kind {
        ProviderDraftAuthKind::Unset => ui_text::t(i18n, "provider-auth-kind-unset"),
        ProviderDraftAuthKind::None => ui_text::t(i18n, "provider-auth-kind-none"),
        ProviderDraftAuthKind::ApiPending => ui_text::t(i18n, "provider-auth-kind-api"),
        ProviderDraftAuthKind::Api => ui_text::t(i18n, "provider-auth-kind-api"),
        ProviderDraftAuthKind::ClineApi => ui_text::t(i18n, "provider-auth-kind-cline"),
        ProviderDraftAuthKind::Gitlab => ui_text::t(i18n, "provider-auth-kind-gitlab"),
        ProviderDraftAuthKind::Credential(Some(issuer)) => i18n.text_args(
            "provider-auth-kind-credential-with-issuer",
            &crate::fl_args!(
                "issuer" => provider_credential_issuer_label_localized(i18n, *issuer)
            ),
        ),
        ProviderDraftAuthKind::Credential(None) => {
            ui_text::t(i18n, "provider-auth-kind-credential")
        }
        ProviderDraftAuthKind::BedrockSigv4 => ui_text::t(i18n, "provider-auth-kind-bedrock"),
    }
}

fn provider_draft_auth_mode_label(i18n: &I18n, auth_kind: &ProviderDraftAuthKind) -> String {
    match auth_kind {
        ProviderDraftAuthKind::Unset => ui_text::t(i18n, "provider-auth-kind-unset"),
        ProviderDraftAuthKind::None => ui_text::t(i18n, "provider-auth-kind-none"),
        ProviderDraftAuthKind::ApiPending
        | ProviderDraftAuthKind::Api
        | ProviderDraftAuthKind::ClineApi
        | ProviderDraftAuthKind::Gitlab
        | ProviderDraftAuthKind::BedrockSigv4 => ui_text::t(i18n, "provider-auth-kind-api"),
        ProviderDraftAuthKind::Credential(_) => ui_text::t(i18n, "provider-auth-kind-credential"),
    }
}

fn provider_draft_auth_subtype_label(i18n: &I18n, auth_kind: &ProviderDraftAuthKind) -> String {
    match auth_kind {
        ProviderDraftAuthKind::Unset
        | ProviderDraftAuthKind::None
        | ProviderDraftAuthKind::ApiPending
        | ProviderDraftAuthKind::Credential(None) => String::new(),
        ProviderDraftAuthKind::Api => ui_text::t(i18n, "provider-auth-subtype-custom-label"),
        ProviderDraftAuthKind::ClineApi => ui_text::t(i18n, "provider-auth-kind-cline"),
        ProviderDraftAuthKind::Gitlab => ui_text::t(i18n, "provider-auth-kind-gitlab"),
        ProviderDraftAuthKind::Credential(Some(issuer)) => {
            provider_credential_issuer_label_localized(i18n, *issuer)
        }
        ProviderDraftAuthKind::BedrockSigv4 => ui_text::t(i18n, "provider-auth-kind-bedrock"),
    }
}

fn provider_studio_adapter_rule_detail(i18n: &I18n, rule: &ProviderDraftAdapterRule) -> String {
    ui_text::t(i18n, rule.detail_key)
}

fn provider_studio_model_count_label(i18n: &I18n, count: usize) -> String {
    i18n.text_args(
        "overlay-provider-studio-model-count",
        &crate::fl_args!("count" => count as i64),
    )
}

fn provider_studio_catalog_match_label(i18n: &I18n, model_id: Option<&str>) -> String {
    model_id
        .map(|model| {
            i18n.text_args(
                "overlay-provider-studio-catalog-match",
                &crate::fl_args!("model" => model.to_string()),
            )
        })
        .unwrap_or_else(|| ui_text::t(i18n, "overlay-provider-studio-catalog-unmatched"))
}

fn provider_studio_model_list_detail(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
    model_id: &str,
) -> String {
    let key = provider_studio_model_key(adapter_id, model_id);
    let mut parts = vec![provider_studio_catalog_match_label(
        i18n,
        dialog
            .catalog_matches
            .get(key.as_str())
            .map(|entry| entry.model_id.as_str()),
    )];
    if dialog.draft.default_adapter == adapter_id && dialog.draft.default_model == model_id {
        parts.push(ui_text::t(i18n, "overlay-provider-studio-default"));
    }
    join_inline_segments(parts)
}

fn provider_studio_adapter_list_detail(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    adapter_id: &str,
) -> String {
    if let Some(adapter) = dialog
        .adapter_models
        .iter()
        .find(|adapter_models| adapter_models.adapter_id == adapter_id)
    {
        if adapter.error.is_none() {
            return join_inline_segments(vec![
                provider_studio_model_count_label(i18n, adapter.models.len()),
                adapter
                    .resolved_base_url
                    .clone()
                    .unwrap_or_else(|| ui_text::t(i18n, "overlay-provider-studio-loaded")),
            ]);
        }
        return adapter
            .error
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| ui_text::t(i18n, "overlay-provider-studio-error"));
    }
    if let Some(rule) = provider_studio_adapter_rule(dialog, adapter_id) {
        let mut parts = vec![provider_studio_adapter_rule_detail(i18n, rule)];
        if rule.supports_draft_model_listing {
            parts.push(ui_text::t(i18n, "overlay-provider-studio-live-list"));
        }
        if dialog.configured_adapter_ids.contains(adapter_id) {
            parts.push(ui_text::t(i18n, "overlay-provider-studio-configured"));
        }
        return join_inline_segments(parts);
    }
    if dialog.configured_adapter_ids.contains(adapter_id) {
        ui_text::t(i18n, "overlay-provider-studio-configured-disk")
    } else {
        ui_text::t(i18n, "overlay-provider-studio-not-listed")
    }
}

fn provider_studio_live_listing_unavailable_message(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    i18n.text_args(
        "flash-provider-studio-live-listing-unavailable",
        &crate::fl_args!("auth" => provider_draft_auth_kind_label(i18n, auth_kind)),
    )
}

fn provider_studio_draft_listing_unsupported_message(
    i18n: &I18n,
    unsupported: &[String],
) -> String {
    i18n.text_args(
        "flash-provider-studio-draft-listing-unsupported",
        &crate::fl_args!("adapters" => unsupported.join(", ")),
    )
}

fn provider_studio_listing_auth_required_message(
    i18n: &I18n,
    auth_kind: &ProviderDraftAuthKind,
) -> String {
    i18n.text_args(
        "flash-provider-studio-listing-auth-required",
        &crate::fl_args!("auth" => provider_draft_auth_kind_label(i18n, auth_kind)),
    )
}

fn settings_field_display_description(i18n: &I18n, field: SettingsFieldSpec) -> String {
    ui_text::t(i18n, field.description_key)
}

fn settings_field_display_label(i18n: &I18n, field: SettingsFieldSpec) -> String {
    ui_text::t(i18n, field.label_key)
}

fn settings_field_edit_title(i18n: &I18n, field: SettingsFieldSpec) -> String {
    format!(
        "{} ({})",
        settings_field_display_label(i18n, field),
        field.path
    )
}

fn runtime_setting_display_label(i18n: &I18n, field: RuntimeSettingSpec) -> String {
    let key = match field.id {
        RuntimeSettingId::ThinkingMode => "settings-runtime-thinking-label",
        RuntimeSettingId::SpeedMode => "settings-runtime-speed-label",
        RuntimeSettingId::Verbosity => "settings-runtime-verbosity-label",
        RuntimeSettingId::ParallelToolCalls => "settings-runtime-parallel-label",
        RuntimeSettingId::Temperature => "settings-runtime-temperature-label",
        RuntimeSettingId::MaxOutput => "settings-runtime-max-output-label",
        RuntimeSettingId::System => "settings-runtime-system-label",
    };
    ui_text::t(i18n, key)
}

fn runtime_setting_display_description(i18n: &I18n, field: RuntimeSettingSpec) -> String {
    let key = match field.id {
        RuntimeSettingId::ThinkingMode => "settings-runtime-thinking-description",
        RuntimeSettingId::SpeedMode => "settings-runtime-speed-description",
        RuntimeSettingId::Verbosity => "settings-runtime-verbosity-description",
        RuntimeSettingId::ParallelToolCalls => "settings-runtime-parallel-description",
        RuntimeSettingId::Temperature => "settings-runtime-temperature-description",
        RuntimeSettingId::MaxOutput => "settings-runtime-max-output-description",
        RuntimeSettingId::System => "settings-runtime-system-description",
    };
    ui_text::t(i18n, key)
}

fn session_model_variant_field(step: SessionModelVariantStep) -> RuntimeSettingSpec {
    match step {
        SessionModelVariantStep::ThinkingMode => RUNTIME_SETTINGS[0],
        SessionModelVariantStep::SpeedMode => RUNTIME_SETTINGS[1],
        SessionModelVariantStep::Verbosity => RUNTIME_SETTINGS[2],
    }
}

fn settings_choice_adapter_fallback(i18n: &I18n) -> String {
    ui_text::t(i18n, "settings-choice-adapter-fallback")
}

fn settings_choice_default_provider_detail(i18n: &I18n, adapter: &str, model: &str) -> String {
    i18n.text_args(
        "settings-choice-default-provider-detail",
        &crate::fl_args!("adapter" => adapter, "model" => model),
    )
}

fn settings_choice_registered_agent_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "settings-choice-agent-profile-detail")
}

fn settings_choice_bool_override_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "settings-choice-bool-override")
}

fn runtime_setting_choice_supported_model_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "runtime-setting-choice-supported-model")
}

fn runtime_setting_choice_parallel_detail(i18n: &I18n) -> String {
    ui_text::t(i18n, "runtime-setting-choice-parallel-detail")
}

fn runtime_setting_override_summary(i18n: &I18n, value: &str) -> String {
    i18n.text_args(
        "runtime-setting-summary-override-value",
        &crate::fl_args!("value" => value),
    )
}

fn settings_layers_summary(sources: &ConfigJsonSources) -> String {
    if sources.applied_layers.is_empty() {
        return "built-in defaults".to_owned();
    }
    sources.applied_layers.join(" -> ")
}

fn settings_config_file_source_summary(i18n: &I18n, sources: &ConfigJsonSources) -> String {
    let status_key = if sources.config_found {
        "settings-source-file-found"
    } else {
        "settings-source-file-missing"
    };
    i18n.text_args(
        status_key,
        &crate::fl_args!("path" => sources.config_path.display().to_string()),
    )
}

fn settings_source_rows_for_config_path(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
    file_summary: impl Into<String>,
    effective_summary: impl Into<String>,
) -> Vec<SettingsSourceRow> {
    vec![
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-config-file"),
            settings_config_file_source_summary(i18n, sources),
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-file-value"),
            file_summary,
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-effective-value"),
            effective_summary,
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-write-target"),
            format!("{path} -> {}", sources.config_path.display()),
        ),
        SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-layers"),
            settings_layers_summary(sources),
        ),
    ]
}

fn settings_studio_field_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    section: SettingsStudioSectionId,
) -> Vec<SettingsStudioItem> {
    SETTINGS_FIELDS
        .iter()
        .filter(|field| field.section == section)
        .map(|field| {
            let file_value =
                get_json_path(&sources.file, Some(field.path)).unwrap_or(JsonValue::Null);
            let effective_value =
                get_json_path(&sources.effective, Some(field.path)).unwrap_or(JsonValue::Null);
            let effective_summary = format_setting_value_inline(&effective_value);
            let current_summary = if file_value.is_null() {
                ui_text::t(i18n, "settings-source-unset")
            } else {
                format_setting_value_inline(&file_value)
            };
            let source_rows = settings_source_rows_for_config_path(
                i18n,
                sources,
                field.path,
                current_summary.clone(),
                effective_summary.clone(),
            );
            SettingsStudioItem::new(
                settings_field_display_label(i18n, *field),
                effective_summary.clone(),
                settings_field_display_description(i18n, *field),
                SettingsPickerAction::EditField(*field),
            )
            .with_path(field.path)
            .with_current_value(current_summary)
            .with_effective_value(effective_summary)
            .with_source_rows(source_rows)
        })
        .collect()
}

fn settings_studio_provider_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    providers: &[ProviderSummaryResource],
) -> Vec<SettingsStudioItem> {
    let mut items =
        settings_studio_field_items(i18n, sources, SettingsStudioSectionId::ConfigProviders)
            .into_iter()
            .map(|item| {
                if item.path.as_deref() == Some("providers.default") {
                    settings_studio_provider_default_item(i18n, sources, providers)
                } else {
                    item
                }
            })
            .collect::<Vec<_>>();
    items.push(settings_studio_provider_workbench_item(i18n, providers));
    items
}

fn settings_studio_provider_default_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    providers: &[ProviderSummaryResource],
) -> SettingsStudioItem {
    let field = SETTINGS_FIELDS
        .iter()
        .find(|field| field.path == "providers.default")
        .copied()
        .expect("providers.default settings field must exist");
    let file_value = get_json_path(&sources.file, Some(field.path)).unwrap_or(JsonValue::Null);
    let effective_value =
        get_json_path(&sources.effective, Some(field.path)).unwrap_or(JsonValue::Null);
    let effective_summary = provider_default_selection_summary(i18n, providers, &effective_value);
    let current_summary = if file_value.is_null() {
        ui_text::t(i18n, "settings-source-unset")
    } else {
        provider_default_selection_summary(i18n, providers, &file_value)
    };
    let source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        field.path,
        current_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::new(
        settings_field_display_label(i18n, field),
        effective_summary.clone(),
        settings_field_display_description(i18n, field),
        SettingsPickerAction::OpenProviderDefaultWizard,
    )
    .with_path(field.path)
    .with_current_value(current_summary)
    .with_effective_value(effective_summary)
    .with_source_rows(source_rows)
}

fn provider_default_selection_summary(
    i18n: &I18n,
    providers: &[ProviderSummaryResource],
    value: &JsonValue,
) -> String {
    let Some(provider_id) = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return format_setting_value_inline(value);
    };
    providers
        .iter()
        .find(|provider| provider.provider_id == provider_id)
        .map(|provider| provider_default_route_summary(i18n, provider))
        .unwrap_or_else(|| provider_id.to_owned())
}

fn provider_default_route_summary(i18n: &I18n, provider: &ProviderSummaryResource) -> String {
    let mut route = vec![provider.provider_id.clone()];
    if let Some(adapter) = provider
        .defaults
        .adapter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        route.push(adapter.trim().to_owned());
    }
    if !provider.defaults.model.trim().is_empty() {
        route.push(provider.defaults.model.trim().to_owned());
    }

    let mut parts = vec![route.join(" / ")];
    if let Some(thinking_mode) = provider
        .defaults
        .thinking_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "run-options-summary-thinking",
            &crate::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
        ));
    }
    if let Some(speed_mode) = provider
        .defaults
        .speed_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "run-options-summary-speed",
            &crate::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
        ));
    }
    if let Some(verbosity) = provider
        .defaults
        .verbosity
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "run-options-summary-verbosity",
            &crate::fl_args!("value" => verbosity.to_string()),
        ));
    }
    if let Some(parallel_tool_calls) = provider.defaults.parallel_tool_calls {
        parts.push(i18n.text_args(
            "run-options-summary-parallel-tools",
            &crate::fl_args!(
                "value" => ui_text::t(
                    i18n,
                    if parallel_tool_calls {
                        "value-on"
                    } else {
                        "value-off"
                    },
                )
            ),
        ));
    }
    join_inline_segments(parts)
}

fn provider_default_adapter_detail(i18n: &I18n, configured_model_count: usize) -> String {
    join_inline_segments(vec![
        ui_text::t(i18n, "value-enabled"),
        provider_studio_model_count_label(i18n, configured_model_count),
    ])
}

fn provider_default_model_detail(i18n: &I18n, model: &ProviderModel) -> String {
    let mut parts = Vec::new();
    if let Some(display_name) = model
        .display_name
        .as_deref()
        .filter(|value| !value.trim().is_empty() && *value != model.id.as_str())
    {
        parts.push(display_name.trim().to_owned());
    }
    if let Some(context_window) = model.metadata.limits.context_window_tokens {
        parts.push(i18n.text_args(
            "session-model-context-window",
            &crate::fl_args!("value" => context_window as i64),
        ));
    }
    if !model.thinking_modes.is_empty() {
        parts.push(i18n.text_args(
            "run-options-summary-thinking",
            &crate::fl_args!(
                "value" => model
                    .thinking_modes
                    .keys()
                    .map(|name| ui_text::thinking_mode_display_value(name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    if !model.speed_modes.is_empty() {
        parts.push(i18n.text_args(
            "run-options-summary-speed",
            &crate::fl_args!(
                "value" => model
                    .speed_modes
                    .keys()
                    .map(|name| ui_text::speed_mode_display_value(name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "settings-provider-default-model-detail")
    } else {
        join_inline_segments(parts)
    }
}

fn provider_default_wizard_model_ref(draft: &ProviderDefaultWizardDraft) -> Option<ModelRef> {
    let provider_id = draft.provider_id.trim();
    let adapter_id = draft.adapter_id.as_deref()?.trim();
    let model_id = draft.model_id.as_deref()?.trim();
    if provider_id.is_empty() || adapter_id.is_empty() || model_id.is_empty() {
        return None;
    }
    Some(ModelRef::new_with_adapter(
        provider_id,
        adapter_id,
        model_id,
    ))
}

fn provider_default_wizard_optional_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value == PROVIDER_DEFAULT_WIZARD_INHERIT {
        None
    } else {
        Some(value.to_owned())
    }
}

fn provider_defaults_settings_path(provider_id: &str) -> String {
    format!(
        "providers.{}.defaults",
        quoted_settings_segment(provider_id.trim())
    )
}

fn set_optional_string_object_value(
    object: &mut JsonMap<String, JsonValue>,
    key: &str,
    value: Option<&str>,
) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        object.insert(key.to_owned(), JsonValue::String(value.to_owned()));
    } else {
        object.remove(key);
    }
}

fn settings_studio_harness_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem> {
    ["harnesses.browser", "harnesses.shell", "harnesses.editor"]
        .into_iter()
        .map(|path| settings_studio_config_path_item(i18n, sources, path))
        .collect()
}

fn settings_studio_config_path_item(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    path: &str,
) -> SettingsStudioItem {
    let file_value = get_json_path(&sources.file, Some(path)).unwrap_or(JsonValue::Null);
    let effective_value = get_json_path(&sources.effective, Some(path)).unwrap_or(JsonValue::Null);
    let effective_summary = format_setting_value_inline(&effective_value);
    let current_summary = if file_value.is_null() {
        ui_text::t(i18n, "settings-source-unset")
    } else {
        format_setting_value_inline(&file_value)
    };
    let source_rows = settings_source_rows_for_config_path(
        i18n,
        sources,
        path,
        current_summary.clone(),
        effective_summary.clone(),
    );
    SettingsStudioItem::new(
        settings_config_path_display_label(i18n, path),
        effective_summary.clone(),
        ui_text::t(i18n, "settings-config-open-file-detail"),
        SettingsPickerAction::OpenConfigFile,
    )
    .with_path(path)
    .with_current_value(current_summary)
    .with_effective_value(effective_summary)
    .with_source_rows(source_rows)
}

fn settings_config_path_display_label(i18n: &I18n, path: &str) -> String {
    match path {
        "harnesses.browser" => ui_text::t(i18n, "settings-harness-browser-label"),
        "harnesses.shell" => ui_text::t(i18n, "settings-harness-shell-label"),
        "harnesses.editor" => ui_text::t(i18n, "settings-harness-editor-label"),
        _ => path.to_string(),
    }
}

fn settings_studio_runtime_items(
    i18n: &I18n,
    run_options: &RunOptionsState,
) -> Vec<SettingsStudioItem> {
    let runtime_model = run_options
        .model
        .as_ref()
        .map(|model| format!("{}/{}", model.provider_id, model.model_id))
        .unwrap_or_else(|| ui_text::t(i18n, "value-default"));
    let runtime_provider = run_options
        .model
        .as_ref()
        .map(|model| model.provider_id.to_string())
        .unwrap_or_else(|| ui_text::t(i18n, "value-default"));
    let mut items = vec![
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-runtime-provider-override-label"),
            runtime_provider,
            ui_text::t(i18n, "settings-runtime-provider-override-detail"),
            SettingsPickerAction::OpenRuntimeProviderOverride,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-runtime-model-override-label"),
            runtime_model,
            ui_text::t(i18n, "settings-runtime-model-override-detail"),
            SettingsPickerAction::OpenRuntimeModelOverride,
        ),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-runtime-clear-stack-label"),
            ui_text::t(i18n, "value-reset"),
            ui_text::t(i18n, "settings-runtime-clear-stack-detail"),
            SettingsPickerAction::ClearRuntimeModelStack,
        ),
    ];
    for item in &mut items {
        item.source_rows = vec![SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-write-target"),
            ui_text::t(i18n, "settings-source-current-session-runtime"),
        )];
    }
    items.extend(RUNTIME_SETTINGS.iter().map(|field| {
        let summary = run_options.runtime_setting_summary(i18n, *field);
        SettingsStudioItem::new(
            runtime_setting_display_label(i18n, *field),
            summary.clone(),
            runtime_setting_display_description(i18n, *field),
            SettingsPickerAction::EditRuntimeSetting(*field),
        )
        .with_current_value(summary.clone())
        .with_effective_value(summary)
        .with_source_rows(vec![SettingsSourceRow::new(
            ui_text::t(i18n, "settings-source-row-write-target"),
            ui_text::t(i18n, "settings-source-current-session-runtime"),
        )])
    }));
    items
}

fn quoted_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn settings_studio_plugin_items(
    i18n: &I18n,
    _sources: &ConfigJsonSources,
) -> Vec<SettingsStudioItem> {
    vec![
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-plugin-policy-label"),
            ui_text::t(i18n, "value-open"),
            ui_text::t(i18n, "settings-plugin-policy-detail"),
            SettingsPickerAction::OpenPluginPolicyStudio,
        )
        .without_value_details(),
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-plugin-workbench-label"),
            ui_text::t(i18n, "value-open"),
            ui_text::t(i18n, "settings-plugin-workbench-detail"),
            SettingsPickerAction::OpenPluginWorkbench,
        )
        .without_value_details(),
    ]
}

fn agent_default_summary(i18n: &I18n, default: &agena::agents::AgentSelectionConfig) -> String {
    let mut parts = Vec::new();
    if let Some(provider) = default
        .provider
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-provider").as_str(),
            provider,
        ));
    }
    if let Some(adapter) = default
        .adapter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-adapter").as_str(),
            adapter,
        ));
    }
    if let Some(model) = default
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-model").as_str(),
            model,
        ));
    }
    if let Some(thinking_mode) = default
        .thinking_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-thinking").as_str(),
            ui_text::thinking_mode_display_value(thinking_mode).as_str(),
        ));
    }
    if let Some(speed_mode) = default
        .speed_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-speed").as_str(),
            ui_text::speed_mode_display_value(speed_mode).as_str(),
        ));
    }
    if let Some(verbosity) = default
        .verbosity
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-verbosity").as_str(),
            verbosity,
        ));
    }
    if let Some(parallel_tool_calls) = default.parallel_tool_calls {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-parallel-tools").as_str(),
            if parallel_tool_calls { "on" } else { "off" },
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-inherits-runtime-model-defaults")
    } else {
        join_inline_segments(parts)
    }
}

fn agent_permission_summary(
    i18n: &I18n,
    permission: &agena::agent::AgentPermissionConfig,
) -> String {
    if permission.is_empty() {
        return ui_text::t(i18n, "value-inherits-runtime-defaults");
    }

    let mut parts = Vec::new();
    if let Some(path) = permission.path.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if path.workspace.is_some() {
            detail.push(ui_text::t(i18n, "value-workspace"));
        }
        if path.external.is_some() {
            detail.push(ui_text::t(i18n, "value-external"));
        }
        if !path.rules.is_empty() {
            detail.push(i18n.text_args(
                "value-rule-count",
                &crate::fl_args!("count" => path.rules.len() as i64),
            ));
        }
        parts.push(i18n.text_args(
            "value-path-summary",
            &crate::fl_args!(
                "detail" => if detail.is_empty() {
                    ui_text::t(i18n, "value-custom")
                } else {
                    join_inline_segments(detail)
                }
            ),
        ));
    }
    if let Some(network) = permission.network.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if network.internet.is_some() {
            detail.push(ui_text::t(i18n, "value-internet"));
        }
        if network.private.is_some() {
            detail.push(ui_text::t(i18n, "value-private"));
        }
        if network.loopback.is_some() {
            detail.push(ui_text::t(i18n, "value-loopback"));
        }
        if !network.rules.is_empty() {
            detail.push(i18n.text_args(
                "value-rule-count",
                &crate::fl_args!("count" => network.rules.len() as i64),
            ));
        }
        parts.push(i18n.text_args(
            "value-network-summary",
            &crate::fl_args!(
                "detail" => if detail.is_empty() {
                    ui_text::t(i18n, "value-custom")
                } else {
                    join_inline_segments(detail)
                }
            ),
        ));
    }
    if let Some(tools) = permission.tools.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if !tools.tags.is_empty() {
            detail.push(i18n.text_args(
                "value-tag-count",
                &crate::fl_args!("count" => tools.tags.len() as i64),
            ));
        }
        if !tools.names.is_empty() {
            detail.push(i18n.text_args(
                "value-name-count",
                &crate::fl_args!("count" => tools.names.len() as i64),
            ));
        }
        if !tools.plugin.is_empty() {
            detail.push(i18n.text_args(
                "value-plugin-override-count",
                &crate::fl_args!("count" => tools.plugin.len() as i64),
            ));
        }
        if !tools.rules.is_empty() {
            detail.push(i18n.text_args(
                "value-rule-set-count",
                &crate::fl_args!("count" => tools.rules.len() as i64),
            ));
        }
        parts.push(i18n.text_args(
            "value-tools-summary",
            &crate::fl_args!(
                "detail" => if detail.is_empty() {
                    ui_text::t(i18n, "value-custom")
                } else {
                    join_inline_segments(detail)
                }
            ),
        ));
    }

    if parts.is_empty() {
        ui_text::t(i18n, "value-inherits-runtime-defaults")
    } else {
        join_inline_segments(parts)
    }
}

fn settings_studio_agent_browser_item(
    i18n: &I18n,
    agent_count: usize,
    default_agent: Option<&str>,
) -> SettingsStudioItem {
    SettingsStudioItem::new(
        ui_text::t(i18n, "settings-agent-browser-label"),
        match default_agent {
            Some(default) => i18n.text_args(
                "settings-agent-browser-value-default",
                &crate::fl_args!(
                    "count" => agent_count as i64,
                    "default" => default.to_string(),
                ),
            ),
            None => i18n.text_args(
                "settings-agent-browser-value",
                &crate::fl_args!("count" => agent_count as i64),
            ),
        },
        ui_text::t(i18n, "settings-agent-browser-detail"),
        SettingsPickerAction::OpenAgentList,
    )
}

fn permission_layer_source_rows(
    i18n: &I18n,
    global_permission: &PermissionConfig,
    session: Option<&SessionPermissionStudioState>,
) -> Vec<SettingsSourceRow> {
    let mut rows = vec![SettingsSourceRow::new(
        ui_text::t(i18n, "settings-permission-layer-global"),
        permission_override_summary(i18n, global_permission),
    )];
    if let Some(session) = session {
        rows.push(SettingsSourceRow::new(
            session
                .agent_name
                .as_deref()
                .map(|name| {
                    i18n.text_args(
                        "settings-permission-layer-agent-named",
                        &crate::fl_args!("agent" => name.to_string()),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "settings-permission-layer-agent")),
            session
                .agent_permission
                .as_ref()
                .map(|permission| permission_override_summary(i18n, permission))
                .unwrap_or_else(|| ui_text::t(i18n, "settings-source-unset")),
        ));
        rows.push(SettingsSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-session"),
            permission_override_summary(i18n, &session.permission),
        ));
        rows.push(SettingsSourceRow::new(
            ui_text::t(i18n, "settings-permission-layer-effective"),
            permission_override_summary(i18n, &session.effective_permission),
        ));
    }
    rows
}

fn settings_studio_permission_items(
    i18n: &I18n,
    sources: &ConfigJsonSources,
    global_file_permission: &PermissionConfig,
    global_effective_permission: &PermissionConfig,
    current_session: Option<&SessionPermissionStudioState>,
) -> Vec<SettingsStudioItem> {
    let mut items = Vec::new();
    if let Some(session) = current_session {
        let effective_summary = permission_override_summary(i18n, &session.effective_permission);
        items.push(
            SettingsStudioItem::new(
                ui_text::t(i18n, "settings-permission-effective-label"),
                effective_summary.clone(),
                i18n.text_args(
                    "settings-permission-effective-detail",
                    &crate::fl_args!("session" => session.session_title.clone()),
                ),
                SettingsPickerAction::OpenSessionEffectivePermissionView(session.session_id),
            )
            .with_current_value(effective_summary.clone())
            .with_effective_value(effective_summary)
            .with_source_rows(permission_layer_source_rows(
                i18n,
                global_effective_permission,
                Some(session),
            )),
        );
        let session_summary = permission_override_summary(i18n, &session.permission);
        items.push(
            SettingsStudioItem::new(
                ui_text::t(i18n, "settings-permission-current-label"),
                session_summary.clone(),
                i18n.text_args(
                    "settings-permission-current-detail",
                    &crate::fl_args!("session" => session.session_title.clone()),
                ),
                SettingsPickerAction::OpenCurrentSessionPermissionWorkbench,
            )
            .with_current_value(session_summary.clone())
            .with_effective_value(permission_override_summary(
                i18n,
                &session.effective_permission,
            ))
            .with_source_rows({
                let mut rows =
                    permission_layer_source_rows(i18n, global_effective_permission, Some(session));
                rows.push(SettingsSourceRow::new(
                    ui_text::t(i18n, "settings-source-row-write-target"),
                    ui_text::t(i18n, "settings-source-current-session"),
                ));
                rows
            }),
        );
        if let Some(agent_name) = session.agent_name.as_deref() {
            let agent_permission = session.agent_permission.clone().unwrap_or_default();
            let agent_summary = permission_override_summary(i18n, &agent_permission);
            items.push(
                SettingsStudioItem::new(
                    i18n.text_args(
                        "settings-permission-agent-label",
                        &crate::fl_args!("agent" => agent_name.to_string()),
                    ),
                    agent_summary.clone(),
                    ui_text::t(i18n, "settings-permission-agent-detail"),
                    SettingsPickerAction::OpenAgentPermissionWorkbench(agent_name.to_string()),
                )
                .with_current_value(agent_summary.clone())
                .with_effective_value(permission_override_summary(
                    i18n,
                    &session.effective_permission,
                ))
                .with_source_rows(vec![
                    SettingsSourceRow::new(
                        ui_text::t(i18n, "settings-permission-layer-agent"),
                        agent_summary,
                    ),
                    SettingsSourceRow::new(
                        ui_text::t(i18n, "settings-source-row-write-target"),
                        i18n.text_args(
                            "settings-source-agent-profile",
                            &crate::fl_args!("agent" => agent_name.to_string()),
                        ),
                    ),
                ]),
            );
        }
    }

    let file_summary = permission_override_summary(i18n, global_file_permission);
    let effective_summary = permission_override_summary(i18n, global_effective_permission);
    items.push(
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-permission-global-label"),
            effective_summary.clone(),
            ui_text::t(i18n, "settings-permission-global-detail"),
            SettingsPickerAction::OpenGlobalPermissionWorkbench,
        )
        .with_path("permission")
        .with_current_value(file_summary.clone())
        .with_effective_value(effective_summary.clone())
        .with_source_rows(settings_source_rows_for_config_path(
            i18n,
            sources,
            "permission",
            file_summary,
            effective_summary,
        )),
    );
    items
}

fn agent_picker_item(
    i18n: &I18n,
    agent: AgentDescriptor,
    default_agent: Option<&str>,
    config_owned: bool,
) -> PickerItem {
    let storage = agent_descriptor_storage(&agent, config_owned);
    let source = match storage {
        AgentProfileStorage::BuiltIn => ui_text::t(i18n, "value-built-in"),
        AgentProfileStorage::Config => ui_text::t(i18n, "value-runtime-config"),
        AgentProfileStorage::Markdown => agent
            .source_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ui_text::t(i18n, "value-markdown-backed")),
        AgentProfileStorage::Runtime => ui_text::t(i18n, "value-runtime-registered"),
    };
    let mut detail = vec![
        agent_scope_label_localized(i18n, agent.scope),
        agent_profile_storage_label_localized(i18n, storage),
        source,
    ];
    if default_agent.is_some_and(|name| name == agent.name.as_str()) {
        detail.push(ui_text::t(i18n, "value-default"));
    }
    let description = agent.description.trim();
    if !description.is_empty() {
        detail.push(description.to_string());
    }
    PickerItem {
        label: agent.name.clone(),
        detail: join_inline_segments(detail),
        value: PickerValue::Agent(Box::new(agent)),
    }
}

fn agent_descriptor_storage(agent: &AgentDescriptor, config_owned: bool) -> AgentProfileStorage {
    if matches!(agent.scope, AgentScope::Default) {
        AgentProfileStorage::BuiltIn
    } else if agent.source_path.is_some() {
        AgentProfileStorage::Markdown
    } else if config_owned {
        AgentProfileStorage::Config
    } else {
        AgentProfileStorage::Runtime
    }
}

fn agent_scope_label_localized(i18n: &I18n, scope: AgentScope) -> String {
    ui_text::t(
        i18n,
        match scope {
            AgentScope::Project => "value-agent-scope-project",
            AgentScope::User => "value-agent-scope-user",
            AgentScope::Default => "value-agent-scope-default",
        },
    )
}

fn agent_list_create_item(i18n: &I18n) -> PickerItem {
    PickerItem {
        label: ui_text::t(i18n, "overlay-agent-list-create-label"),
        detail: ui_text::t(i18n, "overlay-agent-list-create-detail"),
        value: PickerValue::AgentCreate,
    }
}

fn agent_list_items(
    i18n: &I18n,
    mut agents: Vec<AgentDescriptor>,
    default_agent: Option<&str>,
    config_agents: &HashSet<String>,
) -> Vec<PickerItem> {
    agents.sort_by(|left, right| left.name.cmp(&right.name));
    let mut items = vec![agent_list_create_item(i18n)];
    items.extend(agents.into_iter().map(|agent| {
        let config_owned = config_agents.contains(agent.name.as_str());
        agent_picker_item(i18n, agent, default_agent, config_owned)
    }));
    items
}

fn agent_profile_storage(profile: &AgentProfile, config_owned: bool) -> AgentProfileStorage {
    if matches!(profile.scope, AgentScope::Default) {
        AgentProfileStorage::BuiltIn
    } else if profile.source_path.is_some() {
        AgentProfileStorage::Markdown
    } else if config_owned {
        AgentProfileStorage::Config
    } else {
        AgentProfileStorage::Runtime
    }
}

fn agent_profile_storage_label_localized(i18n: &I18n, storage: AgentProfileStorage) -> String {
    ui_text::t(
        i18n,
        match storage {
            AgentProfileStorage::BuiltIn => "value-built-in",
            AgentProfileStorage::Config => "value-config-backed",
            AgentProfileStorage::Markdown => "value-markdown-backed",
            AgentProfileStorage::Runtime => "value-runtime-registered",
        },
    )
}

fn agent_profile_scope_label_localized(i18n: &I18n, profile: &AgentProfile) -> String {
    ui_text::t(
        i18n,
        match profile.scope {
            AgentScope::Project => "value-agent-scope-project",
            AgentScope::User => "value-agent-scope-user",
            AgentScope::Default => "value-agent-scope-default",
        },
    )
}

fn agent_profile_source_label_localized(
    i18n: &I18n,
    profile: &AgentProfile,
    storage: AgentProfileStorage,
) -> String {
    match storage {
        AgentProfileStorage::BuiltIn => ui_text::t(i18n, "value-built-in-defaults"),
        AgentProfileStorage::Config => ui_text::t(i18n, "value-runtime-config-file"),
        AgentProfileStorage::Markdown => profile
            .source_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| ui_text::t(i18n, "value-markdown-backed")),
        AgentProfileStorage::Runtime => ui_text::t(i18n, "value-runtime-registered"),
    }
}

fn agent_prompt_summary(i18n: &I18n, prompt: &str) -> String {
    if prompt.trim().is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        i18n.text_args(
            "value-char-count",
            &crate::fl_args!("count" => prompt.chars().count() as i64),
        )
    }
}

fn agent_optional_string_summary(i18n: &I18n, value: Option<&str>, empty_key: &str) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| ui_text::t(i18n, empty_key))
}

fn agent_studio_items(
    i18n: &I18n,
    profile: &AgentProfile,
    storage: AgentProfileStorage,
) -> Vec<AgentStudioItem> {
    vec![
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-description"),
            value: agent_optional_string_summary(
                i18n,
                (!profile.frontmatter.description.trim().is_empty())
                    .then_some(profile.frontmatter.description.as_str()),
                "value-unset",
            ),
            detail: ui_text::t(i18n, "agent-studio-item-description-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::Description),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-prompt"),
            value: agent_prompt_summary(i18n, profile.prompt.as_str()),
            detail: ui_text::t(i18n, "agent-studio-item-prompt-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::Prompt),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-default-provider"),
            value: agent_optional_string_summary(
                i18n,
                profile.frontmatter.defaults.provider.as_deref(),
                "value-inherit",
            ),
            detail: ui_text::t(i18n, "agent-studio-item-default-provider-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultProvider),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-default-adapter"),
            value: agent_optional_string_summary(
                i18n,
                profile.frontmatter.defaults.adapter.as_deref(),
                "value-inherit",
            ),
            detail: ui_text::t(i18n, "agent-studio-item-default-adapter-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultAdapter),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-field-default-model"),
            value: agent_optional_string_summary(
                i18n,
                profile.frontmatter.defaults.model.as_deref(),
                "value-inherit",
            ),
            detail: ui_text::t(i18n, "agent-studio-item-default-model-detail"),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultModel),
        },
        AgentStudioItem {
            label: ui_text::t(i18n, "agent-studio-item-permission-policy-label"),
            value: agent_permission_summary(i18n, &profile.frontmatter.permission),
            detail: ui_text::t(i18n, "agent-studio-item-permission-policy-detail"),
            action: AgentStudioAction::OpenPermissionWorkbench,
        },
        AgentStudioItem {
            label: match storage {
                AgentProfileStorage::Markdown => {
                    ui_text::t(i18n, "agent-studio-item-open-source-file")
                }
                AgentProfileStorage::Config => {
                    ui_text::t(i18n, "agent-studio-item-open-config-file")
                }
                AgentProfileStorage::BuiltIn | AgentProfileStorage::Runtime => {
                    ui_text::t(i18n, "agent-studio-item-source-label")
                }
            },
            value: agent_profile_source_label_localized(i18n, profile, storage),
            detail: match storage {
                AgentProfileStorage::Config => {
                    ui_text::t(i18n, "agent-studio-item-open-config-detail")
                }
                AgentProfileStorage::Markdown => {
                    ui_text::t(i18n, "agent-studio-item-open-source-detail")
                }
                AgentProfileStorage::BuiltIn => {
                    ui_text::t(i18n, "agent-studio-item-open-built-in-detail")
                }
                AgentProfileStorage::Runtime => {
                    ui_text::t(i18n, "agent-studio-item-open-runtime-detail")
                }
            },
            action: AgentStudioAction::OpenSource,
        },
    ]
}

fn agent_studio_item_detail_text(
    i18n: &I18n,
    profile: &AgentProfile,
    item: &AgentStudioItem,
    storage: AgentProfileStorage,
) -> Text<'static> {
    match &item.action {
        AgentStudioAction::Edit(AgentStudioField::Description) => {
            let mut lines = vec![app_detail_plain_line(ui_text::t(
                i18n,
                "overlay-agent-detail-description-help",
            ))];
            lines.push(app_detail_plain_line(String::new()));
            if profile.frontmatter.description.trim().is_empty() {
                lines.push(app_detail_plain_line(ui_text::t(
                    i18n,
                    "overlay-agent-detail-description-unset",
                )));
            } else {
                lines.push(app_detail_plain_line(
                    profile.frontmatter.description.clone(),
                ));
            }
            lines.push(app_detail_plain_line(String::new()));
            lines.push(app_detail_plain_line(agent_editability_hint(i18n, storage)));
            build_app_detail_text(lines)
        }
        AgentStudioAction::Edit(AgentStudioField::Prompt) => {
            let mut lines = vec![app_detail_labeled_line(
                ui_text::t(i18n, "overlay-agent-detail-prompt-length"),
                i18n.text_args(
                    "overlay-agent-detail-prompt-chars",
                    &crate::fl_args!("count" => profile.prompt.chars().count() as i64),
                ),
            )];
            lines.push(app_detail_plain_line(String::new()));
            if profile.prompt.trim().is_empty() {
                lines.push(app_detail_plain_line(ui_text::t(
                    i18n,
                    "overlay-agent-detail-prompt-unset",
                )));
            } else {
                lines.push(app_detail_plain_line(profile.prompt.clone()));
            }
            lines.push(app_detail_plain_line(String::new()));
            lines.push(app_detail_plain_line(agent_editability_hint(i18n, storage)));
            build_app_detail_text(lines)
        }
        AgentStudioAction::OpenPermissionWorkbench => {
            let mut lines = vec![
                app_detail_labeled_line(
                    ui_text::t(i18n, "overlay-agent-overview-permission"),
                    agent_permission_summary(i18n, &profile.frontmatter.permission),
                ),
                app_detail_plain_line(String::new()),
            ];
            lines.extend(agent_permission_document_detail_lines(
                i18n,
                &profile.frontmatter.permission,
            ));
            lines.push(app_detail_plain_line(String::new()));
            lines.push(app_detail_plain_line(ui_text::t(
                i18n,
                if storage.editable() {
                    "overlay-agent-detail-open-permission"
                } else {
                    "overlay-agent-detail-open-permission-read-only"
                },
            )));
            build_app_detail_text(lines)
        }
        AgentStudioAction::OpenSource => build_app_detail_text(vec![
            app_detail_labeled_line(
                ui_text::t(i18n, "overlay-agent-overview-source"),
                agent_profile_source_label_localized(i18n, profile, storage),
            ),
            app_detail_labeled_line(
                ui_text::t(i18n, "overlay-agent-overview-scope"),
                agent_profile_scope_label_localized(i18n, profile),
            ),
            app_detail_plain_line(String::new()),
            app_detail_plain_line(item.detail.clone()),
        ]),
        AgentStudioAction::Edit(_) => build_app_detail_text(vec![
            app_detail_plain_line(item.detail.clone()),
            app_detail_labeled_line(
                ui_text::t(i18n, "overlay-detail-current-value"),
                item.value.clone(),
            ),
            app_detail_plain_line(String::new()),
            app_detail_plain_line(agent_editability_hint(i18n, storage)),
        ]),
    }
}

fn agent_studio_overview_text(
    i18n: &I18n,
    profile: &AgentProfile,
    default_agent_name: Option<&str>,
    storage: AgentProfileStorage,
) -> Text<'static> {
    let mut lines = vec![
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-name"),
            profile.name.clone(),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-scope"),
            agent_profile_scope_label_localized(i18n, profile),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-storage"),
            agent_profile_storage_label_localized(i18n, storage),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-default-agent"),
            localized_yes_no(
                i18n,
                default_agent_name.is_some_and(|name| name == profile.name.as_str()),
            ),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-source"),
            agent_profile_source_label_localized(i18n, profile, storage),
        ),
        app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-permission"),
            agent_permission_summary(i18n, &profile.frontmatter.permission),
        ),
    ];
    if !profile.frontmatter.defaults.is_empty() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "overlay-agent-overview-model-defaults"),
            agent_default_summary(i18n, &profile.frontmatter.defaults),
        ));
    }
    if !profile.frontmatter.description.trim().is_empty() {
        lines.push(app_detail_plain_line(String::new()));
        lines.push(app_detail_plain_line(
            profile.frontmatter.description.clone(),
        ));
    }
    lines.push(app_detail_plain_line(String::new()));
    lines.push(app_detail_plain_line(ui_text::t(
        i18n,
        agent_profile_overview_hint_key(storage),
    )));
    build_app_detail_text(lines)
}

fn build_app_detail_text(lines: Vec<DetailTextLine<'static>>) -> Text<'static> {
    build_detail_text(lines, &DetailTextSpec::with_label_width(14))
}

fn app_detail_labeled_line(
    label: impl Into<String>,
    value: impl Into<String>,
) -> DetailTextLine<'static> {
    let label = label.into();
    let value = value.into();
    DetailTextLine::labeled(
        label,
        sanitize_terminal_text(value.as_str()),
        Style::default().fg(Color::DarkGray),
        Style::default(),
    )
}

fn app_detail_plain_line(text: impl Into<String>) -> DetailTextLine<'static> {
    let text = text.into();
    DetailTextLine::plain(sanitize_terminal_text(text.as_str()), Style::default())
}

fn app_detail_heading_line(text: impl Into<String>) -> DetailTextLine<'static> {
    let text = text.into();
    DetailTextLine::plain(
        sanitize_terminal_text(text.as_str()),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}

fn localized_yes_no(i18n: &I18n, value: bool) -> String {
    ui_text::t(i18n, if value { "value-yes" } else { "value-no" })
}

fn agent_profile_overview_hint_key(storage: AgentProfileStorage) -> &'static str {
    match storage {
        AgentProfileStorage::BuiltIn => "overlay-agent-overview-built-in",
        AgentProfileStorage::Config => "overlay-agent-overview-config-editable",
        AgentProfileStorage::Markdown => "overlay-agent-overview-markdown-editable",
        AgentProfileStorage::Runtime => "overlay-agent-overview-runtime-read-only",
    }
}

fn agent_editability_hint(i18n: &I18n, storage: AgentProfileStorage) -> String {
    ui_text::t(
        i18n,
        match storage {
            AgentProfileStorage::BuiltIn => "overlay-agent-detail-built-in-hint",
            AgentProfileStorage::Config => "overlay-agent-detail-config-editable-hint",
            AgentProfileStorage::Markdown => "overlay-agent-detail-markdown-editable-hint",
            AgentProfileStorage::Runtime => "overlay-agent-detail-runtime-read-only-hint",
        },
    )
}

fn agent_studio_editor_config(
    i18n: &I18n,
    profile: &AgentProfile,
    field: AgentStudioField,
) -> (String, String, String, bool, Editor) {
    let multiline = matches!(
        field,
        AgentStudioField::Description | AgentStudioField::Prompt
    );
    let title = settings_edit_title(i18n, agent_studio_field_label(i18n, field).as_str());
    let prompt = agent_studio_field_prompt(i18n, field);
    let footer = editor_save_footer(i18n, multiline);
    let input = Editor::from_text(agent_studio_field_input_text(profile, field));
    (title, prompt, footer, multiline, input)
}

fn agent_studio_field_label(i18n: &I18n, field: AgentStudioField) -> String {
    ui_text::t(
        i18n,
        match field {
            AgentStudioField::Description => "agent-studio-field-description",
            AgentStudioField::Prompt => "agent-studio-field-prompt",
            AgentStudioField::DefaultProvider => "agent-studio-field-default-provider",
            AgentStudioField::DefaultAdapter => "agent-studio-field-default-adapter",
            AgentStudioField::DefaultModel => "agent-studio-field-default-model",
        },
    )
}

fn agent_studio_field_prompt(i18n: &I18n, field: AgentStudioField) -> String {
    ui_text::t(
        i18n,
        match field {
            AgentStudioField::Description => "agent-studio-field-prompt-description",
            AgentStudioField::Prompt => "agent-studio-field-prompt-prompt",
            AgentStudioField::DefaultProvider => "agent-studio-field-prompt-default-provider",
            AgentStudioField::DefaultAdapter => "agent-studio-field-prompt-default-adapter",
            AgentStudioField::DefaultModel => "agent-studio-field-prompt-default-model",
        },
    )
}

fn agent_studio_field_input_text(profile: &AgentProfile, field: AgentStudioField) -> String {
    match field {
        AgentStudioField::Description => profile.frontmatter.description.clone(),
        AgentStudioField::Prompt => profile.prompt.clone(),
        AgentStudioField::DefaultProvider => profile
            .frontmatter
            .defaults
            .provider
            .clone()
            .unwrap_or_default(),
        AgentStudioField::DefaultAdapter => profile
            .frontmatter
            .defaults
            .adapter
            .clone()
            .unwrap_or_default(),
        AgentStudioField::DefaultModel => profile
            .frontmatter
            .defaults
            .model
            .clone()
            .unwrap_or_default(),
    }
}

fn apply_agent_studio_field_to_profile(
    profile: &mut AgentProfile,
    field: AgentStudioField,
    input: &str,
) {
    let trimmed = input.trim();
    match field {
        AgentStudioField::Description => {
            profile.frontmatter.description = if trimmed.is_empty() {
                String::new()
            } else {
                input.to_string()
            };
        }
        AgentStudioField::Prompt => {
            profile.prompt = if trimmed.is_empty() {
                String::new()
            } else {
                input.to_string()
            };
        }
        AgentStudioField::DefaultProvider => {
            profile.frontmatter.defaults.provider =
                (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        AgentStudioField::DefaultAdapter => {
            profile.frontmatter.defaults.adapter =
                (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
        AgentStudioField::DefaultModel => {
            profile.frontmatter.defaults.model = (!trimmed.is_empty()).then(|| trimmed.to_string());
        }
    }
}

fn agent_studio_field_setting_value(
    _i18n: &I18n,
    agent_name: &str,
    field: AgentStudioField,
    input: &str,
) -> UiResult<(String, Option<JsonValue>)> {
    let trimmed = input.trim();
    let path = match field {
        AgentStudioField::Description => agent_config_path(agent_name, "description"),
        AgentStudioField::Prompt => agent_config_path(agent_name, "prompt"),
        AgentStudioField::DefaultProvider => agent_config_path(agent_name, "defaults.provider"),
        AgentStudioField::DefaultAdapter => agent_config_path(agent_name, "defaults.adapter"),
        AgentStudioField::DefaultModel => agent_config_path(agent_name, "defaults.model"),
    };
    let value = match field {
        AgentStudioField::Description | AgentStudioField::Prompt => {
            (!trimmed.is_empty()).then_some(JsonValue::String(input.to_string()))
        }
        AgentStudioField::DefaultProvider
        | AgentStudioField::DefaultAdapter
        | AgentStudioField::DefaultModel => {
            (!trimmed.is_empty()).then_some(JsonValue::String(trimmed.to_string()))
        }
    };
    Ok((path, value))
}

fn agent_frontmatter_empty(frontmatter: &AgentFrontmatter) -> bool {
    frontmatter.description.trim().is_empty()
        && frontmatter.permission.is_empty()
        && frontmatter.defaults.is_empty()
}

fn agent_markdown_document(frontmatter: &AgentFrontmatter, prompt: &str) -> UiResult<String> {
    let prompt = prompt.trim_start_matches('\n');
    if agent_frontmatter_empty(frontmatter) {
        return Ok(if prompt.is_empty() {
            String::new()
        } else {
            format!("{}\n", prompt.trim_end_matches('\n'))
        });
    }
    let yaml = serde_yaml::to_string(frontmatter).map_err(|error| error.to_string())?;
    let yaml = yaml
        .strip_prefix("---\n")
        .unwrap_or(yaml.as_str())
        .trim_end();
    Ok(format!(
        "---\n{yaml}\n---\n{}\n",
        prompt.trim_end_matches('\n')
    ))
}

fn permission_studio_nav_items(i18n: &I18n) -> Vec<PermissionStudioNavItem> {
    vec![
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-overview"),
            level: 0,
            page: PermissionStudioPage::Overview,
            section: Some(PermissionStudioSectionId::RootPath),
            selectable: true,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-filesystem"),
            level: 0,
            page: PermissionStudioPage::PathDefaults,
            section: Some(PermissionStudioSectionId::PathDefaults),
            selectable: false,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-default-zones"),
            level: 1,
            page: PermissionStudioPage::PathDefaults,
            section: Some(PermissionStudioSectionId::PathDefaults),
            selectable: true,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-path-rules"),
            level: 1,
            page: PermissionStudioPage::PathRules,
            section: Some(PermissionStudioSectionId::PathRules),
            selectable: true,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-network"),
            level: 0,
            page: PermissionStudioPage::NetworkZones,
            section: Some(PermissionStudioSectionId::NetworkZones),
            selectable: false,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-network-zones"),
            level: 1,
            page: PermissionStudioPage::NetworkZones,
            section: Some(PermissionStudioSectionId::NetworkZones),
            selectable: true,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-domain-rules"),
            level: 1,
            page: PermissionStudioPage::NetworkRules,
            section: Some(PermissionStudioSectionId::NetworkRules),
            selectable: true,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-tool-access"),
            level: 0,
            page: PermissionStudioPage::ToolTags,
            section: Some(PermissionStudioSectionId::ToolTags),
            selectable: false,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-tag-rules"),
            level: 1,
            page: PermissionStudioPage::ToolTags,
            section: Some(PermissionStudioSectionId::ToolTags),
            selectable: true,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-name-rules"),
            level: 1,
            page: PermissionStudioPage::ToolNames,
            section: Some(PermissionStudioSectionId::ToolNames),
            selectable: true,
        },
        PermissionStudioNavItem {
            label: ui_text::t(i18n, "permission-studio-nav-command-rules"),
            level: 1,
            page: PermissionStudioPage::ToolCommandRules,
            section: Some(PermissionStudioSectionId::ToolCommandRules),
            selectable: true,
        },
    ]
}

fn permission_studio_nav_index_for_page(page: &PermissionStudioPage) -> usize {
    match page {
        PermissionStudioPage::Overview => 0,
        PermissionStudioPage::PathDefaults => 2,
        PermissionStudioPage::PathRules => 3,
        PermissionStudioPage::NetworkZones => 5,
        PermissionStudioPage::NetworkRules => 6,
        PermissionStudioPage::ToolTags => 8,
        PermissionStudioPage::ToolNames => 9,
        PermissionStudioPage::ToolCommandRules => 10,
    }
}

fn permission_studio_nav_is_selectable(item: &PermissionStudioNavItem) -> bool {
    item.selectable
}

fn permission_studio_nav_normalize_selection(
    nav: &mut SelectableListState<PermissionStudioNavItem>,
) {
    if nav.items.is_empty() {
        nav.selected = 0;
        return;
    }
    if nav.selected >= nav.items.len() {
        nav.selected = nav.items.len() - 1;
    }
    if nav
        .selected_item()
        .is_some_and(permission_studio_nav_is_selectable)
    {
        return;
    }
    if let Some(index) = nav
        .items
        .iter()
        .position(permission_studio_nav_is_selectable)
    {
        nav.selected = index;
    } else {
        nav.selected = 0;
    }
}

fn permission_studio_nav_move_step(
    nav: &mut SelectableListState<PermissionStudioNavItem>,
    delta: isize,
) {
    if nav.items.is_empty() || delta == 0 {
        return;
    }
    let len = nav.items.len();
    let mut index = nav.selected;
    'search: for _ in 0..len {
        let next = if delta < 0 {
            match index.checked_sub(1) {
                Some(next) => next,
                None => break 'search,
            }
        } else {
            match index.checked_add(1).filter(|next| *next < len) {
                Some(next) => next,
                None => break 'search,
            }
        };
        index = next;
        if nav.items[index].selectable {
            nav.selected = index;
            return;
        }
    }
}

fn permission_studio_nav_move_page(
    nav: &mut SelectableListState<PermissionStudioNavItem>,
    delta: isize,
    page_size: usize,
) {
    for _ in 0..page_size {
        permission_studio_nav_move_step(nav, delta);
    }
}

fn permission_studio_nav_move_home(nav: &mut SelectableListState<PermissionStudioNavItem>) {
    if let Some(index) = nav
        .items
        .iter()
        .position(permission_studio_nav_is_selectable)
    {
        nav.selected = index;
    }
}

fn permission_studio_nav_move_end(nav: &mut SelectableListState<PermissionStudioNavItem>) {
    if let Some(index) = nav
        .items
        .iter()
        .rposition(permission_studio_nav_is_selectable)
    {
        nav.selected = index;
    }
}

fn set_permission_studio_pane_focus(
    dialog: &mut PermissionStudioOverlay,
    pane_focus: PermissionStudioPaneFocus,
) {
    dialog.pane_focus = pane_focus;
    dialog.state.set_focus(match pane_focus {
        PermissionStudioPaneFocus::Navigation => PermissionStudioFocus::Navigation,
        PermissionStudioPaneFocus::Content => PermissionStudioFocus::Items,
    });
}

fn refresh_permission_studio_dialog(
    i18n: &I18n,
    dialog: &mut PermissionStudioOverlay,
    preferred_section: Option<PermissionStudioSectionId>,
    preferred_item_label: Option<&str>,
    preferred_focus: Option<PermissionStudioFocus>,
) {
    let nav_items = permission_studio_nav_items(i18n);
    let nav_selected =
        permission_studio_nav_index_for_page(&dialog.page).min(nav_items.len().saturating_sub(1));
    dialog.nav = SelectableListState::new(nav_items, nav_selected);
    permission_studio_nav_normalize_selection(&mut dialog.nav);
    if let Some(nav_item) = dialog.nav.selected_item() {
        dialog.page = nav_item.page.clone();
    }
    let current_section = dialog.state.selected_section().map(|section| section.id);
    let current_item_label = dialog
        .state
        .selected_item()
        .map(|item| item.label.as_str().to_string());
    let sections = permission_studio_sections(i18n, dialog);
    let selected_section = preferred_section
        .or(current_section)
        .and_then(|id| sections.iter().position(|section| section.id == id))
        .unwrap_or(0)
        .min(sections.len().saturating_sub(1));
    let section_items = sections
        .get(selected_section)
        .map(|section| section.items.as_slice())
        .unwrap_or(&[]);
    let selected_item = preferred_item_label
        .or(current_item_label.as_deref())
        .and_then(|label| section_items.iter().position(|item| item.label == label))
        .unwrap_or(0)
        .min(section_items.len().saturating_sub(1));
    let focus = preferred_focus
        .or(Some(dialog.state.focus()))
        .unwrap_or_else(|| permission_studio_default_focus(&dialog.page));
    dialog.title = permission_studio_title(i18n, dialog);
    dialog.footer = permission_studio_footer(i18n, &dialog.page);
    dialog.state = SectionedListState::new(sections, selected_section, selected_item, focus);
    if dialog.state.selected_item().is_none()
        && dialog.state.focus() == PermissionStudioFocus::Items
    {
        dialog.state.set_focus(PermissionStudioFocus::Navigation);
    }
}

fn permission_studio_title(i18n: &I18n, dialog: &PermissionStudioOverlay) -> String {
    match &dialog.page {
        PermissionStudioPage::Overview => format!(
            "{} · {}",
            ui_text::t(i18n, "overlay-permission-studio-title"),
            dialog.title_context
        ),
        page => format!(
            "{} · {} · {}",
            ui_text::t(i18n, "overlay-permission-studio-title"),
            dialog.title_context,
            permission_studio_page_label(i18n, page)
        ),
    }
}

fn permission_studio_footer(i18n: &I18n, page: &PermissionStudioPage) -> String {
    match page {
        PermissionStudioPage::Overview => ui_text::t(i18n, "overlay-permission-studio-footer"),
        PermissionStudioPage::PathDefaults
        | PermissionStudioPage::PathRules
        | PermissionStudioPage::NetworkZones
        | PermissionStudioPage::NetworkRules
        | PermissionStudioPage::ToolTags
        | PermissionStudioPage::ToolNames
        | PermissionStudioPage::ToolCommandRules => {
            ui_text::t(i18n, "overlay-permission-studio-footer-nested")
        }
    }
}

fn permission_studio_default_focus(page: &PermissionStudioPage) -> PermissionStudioFocus {
    match page {
        PermissionStudioPage::Overview => PermissionStudioFocus::Navigation,
        _ => PermissionStudioFocus::Items,
    }
}

fn permission_studio_page_label(i18n: &I18n, page: &PermissionStudioPage) -> String {
    match page {
        PermissionStudioPage::Overview => ui_text::t(i18n, "permission-studio-page-overview"),
        PermissionStudioPage::PathDefaults => {
            ui_text::t(i18n, "permission-studio-page-path-defaults")
        }
        PermissionStudioPage::PathRules => ui_text::t(i18n, "permission-studio-page-path-rules"),
        PermissionStudioPage::NetworkZones => {
            ui_text::t(i18n, "permission-studio-page-network-zones")
        }
        PermissionStudioPage::NetworkRules => {
            ui_text::t(i18n, "permission-studio-page-network-rules")
        }
        PermissionStudioPage::ToolTags => ui_text::t(i18n, "permission-studio-page-tool-tags"),
        PermissionStudioPage::ToolNames => ui_text::t(i18n, "permission-studio-page-tool-names"),
        PermissionStudioPage::ToolCommandRules => {
            ui_text::t(i18n, "permission-studio-page-tool-command-rules")
        }
    }
}

fn permission_studio_selected_tool_tag_key(dialog: &PermissionStudioOverlay) -> Option<String> {
    match dialog.state.selected_item()?.action.clone() {
        PermissionStudioAction::EditText(PermissionStudioTextTarget::ToolTagKey { key }) => {
            Some(key)
        }
        _ => None,
    }
}

fn permission_studio_sections(
    i18n: &I18n,
    dialog: &PermissionStudioOverlay,
) -> Vec<PermissionStudioSection> {
    match &dialog.page {
        PermissionStudioPage::PathDefaults => vec![PermissionStudioSection {
            id: PermissionStudioSectionId::PathDefaults,
            label: ui_text::t(i18n, "permission-studio-page-path-defaults"),
            items: vec![
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-workspace-read"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.workspace.as_ref())
                            .and_then(|modes| modes.read),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathWorkspaceRead,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-workspace-write"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.workspace.as_ref())
                            .and_then(|modes| modes.write),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathWorkspaceWrite,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-external-read"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.external.as_ref())
                            .and_then(|modes| modes.read),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathExternalRead,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-path-external-write"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.external.as_ref())
                            .and_then(|modes| modes.write),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::PathExternalWrite,
                    ),
                },
            ],
        }],
        PermissionStudioPage::PathRules => {
            let mut rules = dialog
                .permission
                .path
                .as_ref()
                .map(|path| path.rules.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            rules.sort();
            let rule_items = rules
                .into_iter()
                .map(|pattern| PermissionStudioItem {
                    label: pattern.clone(),
                    value: path_rule_summary(
                        i18n,
                        dialog
                            .permission
                            .path
                            .as_ref()
                            .and_then(|path| path.rules.get(pattern.as_str())),
                    ),
                    action: PermissionStudioAction::EditText(
                        PermissionStudioTextTarget::PathRulePattern { pattern },
                    ),
                })
                .collect::<Vec<_>>();
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::PathRules,
                label: ui_text::t(i18n, "permission-studio-page-path-rules"),
                items: rule_items,
            }]
        }
        PermissionStudioPage::NetworkZones => vec![PermissionStudioSection {
            id: PermissionStudioSectionId::NetworkZones,
            label: ui_text::t(i18n, "permission-studio-page-network-zones"),
            items: vec![
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-network-internet"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .network
                            .as_ref()
                            .and_then(|network| network.internet),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::NetworkInternet,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-network-private"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .network
                            .as_ref()
                            .and_then(|network| network.private),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::NetworkPrivate,
                    ),
                },
                PermissionStudioItem {
                    label: ui_text::t(i18n, "permission-studio-network-loopback"),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .network
                            .as_ref()
                            .and_then(|network| network.loopback),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditMode(
                        PermissionStudioModeTarget::NetworkLoopback,
                    ),
                },
            ],
        }],
        PermissionStudioPage::NetworkRules => {
            let mut rules = dialog
                .permission
                .network
                .as_ref()
                .map(|network| network.rules.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            rules.sort();
            let rule_items = rules
                .into_iter()
                .map(|target| PermissionStudioItem {
                    label: target.clone(),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .network
                            .as_ref()
                            .and_then(|network| network.rules.get(target.as_str()).copied()),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditText(
                        PermissionStudioTextTarget::NetworkRuleTarget { target },
                    ),
                })
                .collect::<Vec<_>>();
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::NetworkRules,
                label: ui_text::t(i18n, "permission-studio-page-network-rules"),
                items: rule_items,
            }]
        }
        PermissionStudioPage::ToolTags => {
            let mut keys = dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.tags.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            keys.sort();
            let mut tag_items = vec![PermissionStudioItem {
                label: ui_text::t(i18n, "permission-studio-tool-default"),
                value: permission_mode_input_text(
                    dialog
                        .permission
                        .tools
                        .as_ref()
                        .and_then(|tools| tools.default),
                    i18n,
                ),
                action: PermissionStudioAction::EditMode(PermissionStudioModeTarget::ToolDefault),
            }];
            tag_items.extend(
                keys.into_iter()
                    .map(|key| PermissionStudioItem {
                        label: key.clone(),
                        value: permission_mode_input_text(
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .and_then(|tools| tools.tags.get(key.as_str()).copied()),
                            i18n,
                        ),
                        action: PermissionStudioAction::EditText(
                            PermissionStudioTextTarget::ToolTagKey { key },
                        ),
                    })
                    .collect::<Vec<_>>(),
            );
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::ToolTags,
                label: ui_text::t(i18n, "permission-studio-page-tags"),
                items: tag_items,
            }]
        }
        PermissionStudioPage::ToolNames => {
            let mut keys = dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.names.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            keys.sort();
            let name_items = keys
                .into_iter()
                .map(|key| PermissionStudioItem {
                    label: key.clone(),
                    value: permission_mode_input_text(
                        dialog
                            .permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.names.get(key.as_str()).copied()),
                        i18n,
                    ),
                    action: PermissionStudioAction::EditText(
                        PermissionStudioTextTarget::ToolNameKey { key },
                    ),
                })
                .collect::<Vec<_>>();
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::ToolNames,
                label: ui_text::t(i18n, "permission-studio-page-names"),
                items: name_items,
            }]
        }
        PermissionStudioPage::ToolCommandRules => {
            let mut keys = dialog
                .permission
                .tools
                .as_ref()
                .map(|tools| tools.rules.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            keys.sort();
            let tool_rule_items = keys
                .into_iter()
                .map(|tool_name| PermissionStudioItem {
                    label: tool_name.clone(),
                    value: tool_permission_rules_summary(
                        i18n,
                        dialog
                            .permission
                            .tools
                            .as_ref()
                            .and_then(|tools| tools.rules.get(tool_name.as_str())),
                    ),
                    action: PermissionStudioAction::EditText(
                        PermissionStudioTextTarget::ToolRuleName { tool_name },
                    ),
                })
                .collect::<Vec<_>>();
            vec![PermissionStudioSection {
                id: PermissionStudioSectionId::ToolCommandRules,
                label: ui_text::t(i18n, "permission-studio-page-tool-rules"),
                items: tool_rule_items,
            }]
        }
        PermissionStudioPage::Overview => vec![
            PermissionStudioSection {
                id: PermissionStudioSectionId::RootPath,
                label: ui_text::t(i18n, "permission-studio-page-path"),
                items: vec![
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-workspace"),
                        value: path_access_modes_summary(
                            i18n,
                            dialog
                                .permission
                                .path
                                .as_ref()
                                .and_then(|path| path.workspace.as_ref()),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-external"),
                        value: path_access_modes_summary(
                            i18n,
                            dialog
                                .permission
                                .path
                                .as_ref()
                                .and_then(|path| path.external.as_ref()),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-rules"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .path
                                .as_ref()
                                .map(|path| path.rules.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                ],
            },
            PermissionStudioSection {
                id: PermissionStudioSectionId::RootNetwork,
                label: ui_text::t(i18n, "permission-studio-page-network"),
                items: vec![
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-defaults"),
                        value: network_defaults_summary(i18n, dialog.permission.network.as_ref()),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-section-rules"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .network
                                .as_ref()
                                .map(|network| network.rules.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                ],
            },
            PermissionStudioSection {
                id: PermissionStudioSectionId::RootTools,
                label: ui_text::t(i18n, "permission-studio-page-tools"),
                items: vec![
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-tool-default"),
                        value: permission_mode_input_text(
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .and_then(|tools| tools.default),
                            i18n,
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-page-tags"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .map(|tools| tools.tags.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-page-names"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .map(|tools| tools.names.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                    PermissionStudioItem {
                        label: ui_text::t(i18n, "permission-studio-page-tool-rules"),
                        value: permission_rule_count_summary(
                            i18n,
                            dialog
                                .permission
                                .tools
                                .as_ref()
                                .map(|tools| tools.rules.len())
                                .unwrap_or_default(),
                        ),
                        action: PermissionStudioAction::Noop,
                    },
                ],
            },
        ],
    }
}

fn permission_studio_mode_target_label(i18n: &I18n, target: &PermissionStudioModeTarget) -> String {
    ui_text::t(
        i18n,
        match target {
            PermissionStudioModeTarget::PathWorkspaceRead => {
                "permission-studio-path-workspace-read"
            }
            PermissionStudioModeTarget::PathWorkspaceWrite => {
                "permission-studio-path-workspace-write"
            }
            PermissionStudioModeTarget::PathExternalRead => "permission-studio-path-external-read",
            PermissionStudioModeTarget::PathExternalWrite => {
                "permission-studio-path-external-write"
            }
            PermissionStudioModeTarget::NetworkInternet => "permission-studio-network-internet",
            PermissionStudioModeTarget::NetworkPrivate => "permission-studio-network-private",
            PermissionStudioModeTarget::NetworkLoopback => "permission-studio-network-loopback",
            PermissionStudioModeTarget::ToolDefault => "permission-studio-tool-default",
        },
    )
}

fn permission_studio_mode_target_input_text(
    dialog: &PermissionStudioOverlay,
    target: &PermissionStudioModeTarget,
) -> String {
    permission_mode_token_text(permission_studio_mode_target_value(
        &dialog.permission,
        target,
    ))
}

fn permission_studio_text_target_label(i18n: &I18n, target: &PermissionStudioTextTarget) -> String {
    ui_text::t(
        i18n,
        match target {
            PermissionStudioTextTarget::PathRulePattern { .. } => "permission-studio-rule-pattern",
            PermissionStudioTextTarget::NetworkRuleTarget { .. } => "permission-studio-rule-target",
            PermissionStudioTextTarget::ToolTagKey { .. }
            | PermissionStudioTextTarget::ToolNameKey { .. }
            | PermissionStudioTextTarget::ToolRuleName { .. } => "permission-studio-rule-key",
        },
    )
}

fn permission_studio_text_target_input_text(target: &PermissionStudioTextTarget) -> String {
    match target {
        PermissionStudioTextTarget::PathRulePattern { pattern }
        | PermissionStudioTextTarget::NetworkRuleTarget { target: pattern }
        | PermissionStudioTextTarget::ToolTagKey { key: pattern }
        | PermissionStudioTextTarget::ToolNameKey { key: pattern }
        | PermissionStudioTextTarget::ToolRuleName { tool_name: pattern } => pattern.clone(),
    }
}

fn permission_studio_creator_spec(
    i18n: &I18n,
    action: &PermissionStudioEditorAction,
) -> (String, String) {
    match action {
        PermissionStudioEditorAction::AddPathRule { .. } => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-path-rule").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddNetworkRule { .. } => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-network-rule").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolTag { .. } => (
            settings_edit_title(i18n, ui_text::t(i18n, "permission-studio-add-tag").as_str()),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolName { .. } => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-name").as_str(),
            ),
            String::new(),
        ),
        PermissionStudioEditorAction::AddToolRule { .. } => (
            settings_edit_title(
                i18n,
                ui_text::t(i18n, "permission-studio-add-tool-rule").as_str(),
            ),
            String::new(),
        ),
        _ => (String::new(), String::new()),
    }
}

fn permission_studio_creator_input_text(action: &PermissionStudioEditorAction) -> String {
    match action {
        PermissionStudioEditorAction::AddPathRule { duplicate_from }
        | PermissionStudioEditorAction::AddNetworkRule { duplicate_from }
        | PermissionStudioEditorAction::AddToolTag { duplicate_from }
        | PermissionStudioEditorAction::AddToolName { duplicate_from }
        | PermissionStudioEditorAction::AddToolRule { duplicate_from } => {
            duplicate_from.clone().unwrap_or_default()
        }
        _ => String::new(),
    }
}

fn apply_permission_studio_mode_input(
    i18n: &I18n,
    permission: &mut PermissionConfig,
    target: &PermissionStudioModeTarget,
    input: &str,
) -> UiResult<()> {
    let mode = parse_permission_studio_optional_mode_input(i18n, input)?;
    match target {
        PermissionStudioModeTarget::PathWorkspaceRead => {
            set_path_default_mode(permission, false, true, mode);
        }
        PermissionStudioModeTarget::PathWorkspaceWrite => {
            set_path_default_mode(permission, false, false, mode);
        }
        PermissionStudioModeTarget::PathExternalRead => {
            set_path_default_mode(permission, true, true, mode);
        }
        PermissionStudioModeTarget::PathExternalWrite => {
            set_path_default_mode(permission, true, false, mode);
        }
        PermissionStudioModeTarget::NetworkInternet => {
            permission
                .network
                .get_or_insert_with(Default::default)
                .internet = mode;
        }
        PermissionStudioModeTarget::NetworkPrivate => {
            permission
                .network
                .get_or_insert_with(Default::default)
                .private = mode;
        }
        PermissionStudioModeTarget::NetworkLoopback => {
            permission
                .network
                .get_or_insert_with(Default::default)
                .loopback = mode;
        }
        PermissionStudioModeTarget::ToolDefault => {
            permission
                .tools
                .get_or_insert_with(Default::default)
                .default = mode;
        }
    }
    normalize_permission_config(permission);
    Ok(())
}

fn apply_permission_studio_text_input(
    i18n: &I18n,
    permission: &mut PermissionConfig,
    target: &PermissionStudioTextTarget,
    input: &str,
) -> UiResult<PermissionStudioPage> {
    let value = parse_permission_studio_key_input(
        i18n,
        permission_studio_text_target_label(i18n, target).as_str(),
        input,
    )?;
    let page = match target {
        PermissionStudioTextTarget::PathRulePattern { pattern } => {
            rename_path_rule(permission, pattern.as_str(), value.as_str());
            PermissionStudioPage::PathRules
        }
        PermissionStudioTextTarget::NetworkRuleTarget { target } => {
            rename_network_rule(permission, target.as_str(), value.as_str());
            PermissionStudioPage::NetworkRules
        }
        PermissionStudioTextTarget::ToolTagKey { key } => {
            rename_tool_tag(permission, key.as_str(), value.as_str());
            PermissionStudioPage::ToolTags
        }
        PermissionStudioTextTarget::ToolNameKey { key } => {
            rename_tool_name(permission, key.as_str(), value.as_str());
            PermissionStudioPage::ToolNames
        }
        PermissionStudioTextTarget::ToolRuleName { tool_name } => {
            rename_tool_rule(permission, tool_name.as_str(), value.as_str());
            PermissionStudioPage::ToolCommandRules
        }
    };
    normalize_permission_config(permission);
    Ok(page)
}

fn permission_override_summary(i18n: &I18n, permission: &PermissionConfig) -> String {
    let mut parts = Vec::new();
    if permission.path.is_some() {
        parts.push(agent_path_permission_summary(
            i18n,
            permission.path.as_ref(),
        ));
    }
    if permission.network.is_some() {
        parts.push(agent_network_permission_summary(
            i18n,
            permission.network.as_ref(),
        ));
    }
    if permission.tools.is_some() {
        parts.push(agent_tool_permission_summary(
            i18n,
            permission.tools.as_ref(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

fn permission_studio_read_only_message(i18n: &I18n, source: &PermissionStudioSource) -> String {
    match source {
        PermissionStudioSource::Agent { .. } => agent_read_only_permissions_message(i18n),
        PermissionStudioSource::EffectiveSession { .. } => {
            ui_text::t(i18n, "settings-permission-effective-read-only")
        }
        PermissionStudioSource::GlobalConfig | PermissionStudioSource::Session { .. } => {
            ui_text::t(i18n, "permission-studio-detail-read-only")
        }
    }
}

fn agent_path_permission_summary(i18n: &I18n, path: Option<&PathPermissionConfig>) -> String {
    let Some(path) = path else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if path.workspace.is_some() {
        parts.push(ui_text::t(i18n, "value-workspace"));
    }
    if path.external.is_some() {
        parts.push(ui_text::t(i18n, "value-external"));
    }
    if !path.rules.is_empty() {
        parts.push(i18n.text_args(
            "value-rule-count",
            &crate::fl_args!("count" => path.rules.len() as i64),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-custom")
    } else {
        join_inline_segments(parts)
    }
}

fn agent_network_permission_summary(
    i18n: &I18n,
    network: Option<&NetworkPermissionConfig>,
) -> String {
    let Some(network) = network else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if network.internet.is_some() {
        parts.push(ui_text::t(i18n, "value-internet"));
    }
    if network.private.is_some() {
        parts.push(ui_text::t(i18n, "value-private"));
    }
    if network.loopback.is_some() {
        parts.push(ui_text::t(i18n, "value-loopback"));
    }
    if !network.rules.is_empty() {
        parts.push(i18n.text_args(
            "value-rule-count",
            &crate::fl_args!("count" => network.rules.len() as i64),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-custom")
    } else {
        join_inline_segments(parts)
    }
}

fn agent_tool_permission_summary(i18n: &I18n, tools: Option<&ToolPermissionConfig>) -> String {
    let Some(tools) = tools else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if let Some(mode) = tools.default {
        parts.push(i18n.text_args(
            "permission-studio-tool-default-summary",
            &crate::fl_args!("value" => permission_mode_label(i18n, mode)),
        ));
    }
    if !tools.tags.is_empty() {
        parts.push(i18n.text_args(
            "value-tag-count",
            &crate::fl_args!("count" => tools.tags.len() as i64),
        ));
    }
    if !tools.names.is_empty() {
        parts.push(i18n.text_args(
            "value-name-count",
            &crate::fl_args!("count" => tools.names.len() as i64),
        ));
    }
    if !tools.rules.is_empty() {
        parts.push(i18n.text_args(
            "value-rule-set-count",
            &crate::fl_args!("count" => tools.rules.len() as i64),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-custom")
    } else {
        join_inline_segments(parts)
    }
}

fn path_access_modes_summary(i18n: &I18n, modes: Option<&PathAccessModes>) -> String {
    let Some(modes) = modes else {
        return ui_text::t(i18n, "value-unset");
    };
    match (modes.read, modes.write) {
        (Some(read), Some(write)) if read == write => permission_mode_label(i18n, read),
        (read, write) => join_inline_segments(vec![
            i18n.text_args(
                "permission-studio-mode-read",
                &crate::fl_args!(
                    "value" => read
                        .map(|mode| permission_mode_label(i18n, mode))
                        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
                ),
            ),
            i18n.text_args(
                "permission-studio-mode-write",
                &crate::fl_args!(
                    "value" => write
                        .map(|mode| permission_mode_label(i18n, mode))
                        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
                ),
            ),
        ]),
    }
}

fn agent_permission_document_detail_lines(
    i18n: &I18n,
    permission: &PermissionConfig,
) -> Vec<DetailTextLine<'static>> {
    if permission.is_empty() {
        return vec![app_detail_plain_line(ui_text::t(
            i18n,
            "overlay-agent-permission-document-unset",
        ))];
    }

    let mut lines = Vec::new();
    if let Some(path) = permission.path.as_ref() {
        push_agent_permission_section_gap(&mut lines);
        push_path_permission_detail_lines(i18n, &mut lines, path);
    }
    if let Some(network) = permission.network.as_ref() {
        push_agent_permission_section_gap(&mut lines);
        push_network_permission_detail_lines(i18n, &mut lines, network);
    }
    if let Some(tools) = permission.tools.as_ref() {
        push_agent_permission_section_gap(&mut lines);
        push_tool_permission_detail_lines(i18n, &mut lines, tools);
    }
    lines
}

fn push_agent_permission_section_gap(lines: &mut Vec<DetailTextLine<'static>>) {
    if !lines.is_empty() {
        lines.push(app_detail_plain_line(String::new()));
    }
}

fn push_path_permission_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    path: &PathPermissionConfig,
) {
    lines.push(app_detail_heading_line(ui_text::t(
        i18n,
        "agent-permission-field-path-section",
    )));
    if path.workspace.is_some() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-workspace"),
            path_access_modes_summary(i18n, path.workspace.as_ref()),
        ));
    }
    if path.external.is_some() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-external"),
            path_access_modes_summary(i18n, path.external.as_ref()),
        ));
    }
    if !path.rules.is_empty() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "permission-studio-section-rules"),
            permission_rule_count_summary(i18n, path.rules.len()),
        ));
        for (pattern, rule) in &path.rules {
            lines.push(app_detail_labeled_line(
                pattern.clone(),
                path_rule_summary(i18n, Some(rule)),
            ));
        }
    }
}

fn push_network_permission_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    network: &NetworkPermissionConfig,
) {
    lines.push(app_detail_heading_line(ui_text::t(
        i18n,
        "agent-permission-field-network-section",
    )));
    if let Some(mode) = network.internet {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-internet"),
            permission_mode_label(i18n, mode),
        ));
    }
    if let Some(mode) = network.private {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-private"),
            permission_mode_label(i18n, mode),
        ));
    }
    if let Some(mode) = network.loopback {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "value-loopback"),
            permission_mode_label(i18n, mode),
        ));
    }
    push_permission_mode_entries(
        i18n,
        lines,
        ui_text::t(i18n, "permission-studio-section-rules"),
        network.rules.iter(),
    );
}

fn push_tool_permission_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    tools: &ToolPermissionConfig,
) {
    lines.push(app_detail_heading_line(ui_text::t(
        i18n,
        "agent-permission-field-tool-section",
    )));
    if let Some(mode) = tools.default {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "permission-studio-tool-default"),
            permission_mode_label(i18n, mode),
        ));
    }
    push_permission_mode_entries(
        i18n,
        lines,
        ui_text::t(i18n, "permission-studio-field-tool-tags"),
        tools.tags.iter(),
    );
    push_permission_mode_entries(
        i18n,
        lines,
        ui_text::t(i18n, "permission-studio-field-tool-names"),
        tools.names.iter(),
    );
    push_permission_mode_entries(
        i18n,
        lines,
        ui_text::t(i18n, "value-plugin-tools"),
        tools.plugin.iter(),
    );
    if !tools.rules.is_empty() {
        lines.push(app_detail_labeled_line(
            ui_text::t(i18n, "permission-studio-page-tool-rules"),
            i18n.text_args(
                "value-rule-set-count",
                &crate::fl_args!("count" => tools.rules.len() as i64),
            ),
        ));
        for (tool_name, rules) in &tools.rules {
            lines.push(app_detail_labeled_line(
                tool_name.clone(),
                tool_permission_rules_summary(i18n, Some(rules)),
            ));
        }
    }
}

fn push_permission_mode_entries<'a, I>(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    label: String,
    entries: I,
) where
    I: IntoIterator<Item = (&'a String, &'a PermissionMode)>,
{
    let entries = entries.into_iter().collect::<Vec<_>>();
    if entries.is_empty() {
        return;
    }
    lines.push(app_detail_labeled_line(
        label,
        i18n.text_args(
            "value-item-count",
            &crate::fl_args!("count" => entries.len() as i64),
        ),
    ));
    for (name, mode) in entries {
        lines.push(app_detail_labeled_line(
            name.clone(),
            permission_mode_label(i18n, *mode),
        ));
    }
}

fn network_defaults_summary(i18n: &I18n, network: Option<&NetworkPermissionConfig>) -> String {
    let Some(network) = network else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if let Some(mode) = network.internet {
        parts.push(i18n.text_args(
            "permission-studio-network-default",
            &crate::fl_args!(
                "label" => ui_text::t(i18n, "value-internet"),
                "value" => permission_mode_label(i18n, mode),
            ),
        ));
    }
    if let Some(mode) = network.private {
        parts.push(i18n.text_args(
            "permission-studio-network-default",
            &crate::fl_args!(
                "label" => ui_text::t(i18n, "value-private"),
                "value" => permission_mode_label(i18n, mode),
            ),
        ));
    }
    if let Some(mode) = network.loopback {
        parts.push(i18n.text_args(
            "permission-studio-network-default",
            &crate::fl_args!(
                "label" => ui_text::t(i18n, "value-loopback"),
                "value" => permission_mode_label(i18n, mode),
            ),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

fn permission_rule_count_summary(i18n: &I18n, count: usize) -> String {
    match count {
        0 => ui_text::t(i18n, "value-unset"),
        count => i18n.text_args(
            "value-rule-count",
            &crate::fl_args!("count" => count as i64),
        ),
    }
}

fn permission_mode_input_text(mode: Option<PermissionMode>, i18n: &I18n) -> String {
    mode.map(|mode| permission_mode_label(i18n, mode))
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
}

fn permission_mode_token_text(mode: Option<PermissionMode>) -> String {
    mode.map(permission_mode_token)
        .unwrap_or_default()
        .to_string()
}

fn permission_config_from_json_value(value: &JsonValue) -> UiResult<PermissionConfig> {
    if value.is_null() {
        Ok(PermissionConfig::default())
    } else {
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())
    }
}

fn permission_studio_mode_target_value(
    permission: &PermissionConfig,
    target: &PermissionStudioModeTarget,
) -> Option<PermissionMode> {
    match target {
        PermissionStudioModeTarget::PathWorkspaceRead => permission
            .path
            .as_ref()
            .and_then(|path| path.workspace.as_ref())
            .and_then(|modes| modes.read),
        PermissionStudioModeTarget::PathWorkspaceWrite => permission
            .path
            .as_ref()
            .and_then(|path| path.workspace.as_ref())
            .and_then(|modes| modes.write),
        PermissionStudioModeTarget::PathExternalRead => permission
            .path
            .as_ref()
            .and_then(|path| path.external.as_ref())
            .and_then(|modes| modes.read),
        PermissionStudioModeTarget::PathExternalWrite => permission
            .path
            .as_ref()
            .and_then(|path| path.external.as_ref())
            .and_then(|modes| modes.write),
        PermissionStudioModeTarget::NetworkInternet => permission
            .network
            .as_ref()
            .and_then(|network| network.internet),
        PermissionStudioModeTarget::NetworkPrivate => permission
            .network
            .as_ref()
            .and_then(|network| network.private),
        PermissionStudioModeTarget::NetworkLoopback => permission
            .network
            .as_ref()
            .and_then(|network| network.loopback),
        PermissionStudioModeTarget::ToolDefault => {
            permission.tools.as_ref().and_then(|tools| tools.default)
        }
    }
}

fn path_rule_modes(rule: Option<&PathAccessRuleConfig>) -> Option<PathAccessModes> {
    match rule? {
        PathAccessRuleConfig::Modes(modes) => Some(modes.clone()),
        PathAccessRuleConfig::Shorthand(value) => path_access_shorthand_modes(value.as_str()),
    }
}

fn path_access_shorthand_modes(value: &str) -> Option<PathAccessModes> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    let both = |mode| PathAccessModes {
        read: Some(mode),
        write: Some(mode),
    };
    match normalized.as_str() {
        "allow" | "read_write" | "rw" => Some(both(PermissionMode::Allow)),
        "ask" => Some(both(PermissionMode::Ask)),
        "deny" | "none" => Some(both(PermissionMode::Deny)),
        "read" | "read_only" | "ro" => Some(PathAccessModes {
            read: Some(PermissionMode::Allow),
            write: Some(PermissionMode::Deny),
        }),
        "write" | "write_only" | "wo" => Some(PathAccessModes {
            read: Some(PermissionMode::Deny),
            write: Some(PermissionMode::Allow),
        }),
        _ => None,
    }
}

fn path_rule_summary(i18n: &I18n, rule: Option<&PathAccessRuleConfig>) -> String {
    path_rule_modes(rule)
        .map(|modes| path_access_modes_summary(i18n, Some(&modes)))
        .unwrap_or_else(|| ui_text::t(i18n, "value-custom"))
}

fn tool_permission_rules_summary(i18n: &I18n, rules: Option<&ToolPermissionRules>) -> String {
    let Some(rules) = rules else {
        return ui_text::t(i18n, "value-unset");
    };
    match rules {
        ToolPermissionRules::Mode(mode) => permission_mode_label(i18n, *mode),
        ToolPermissionRules::Ordered(entries) => {
            let fallback = entries.get("*").copied();
            let qualifier_count = entries
                .keys()
                .filter(|pattern| pattern.as_str() != "*")
                .count();
            let mut parts = Vec::new();
            if let Some(mode) = fallback {
                parts.push(permission_mode_label(i18n, mode));
            }
            if qualifier_count > 0 {
                parts.push(i18n.text_args(
                    "value-rule-count",
                    &crate::fl_args!("count" => qualifier_count as i64),
                ));
            }
            if parts.is_empty() {
                ui_text::t(i18n, "value-custom")
            } else {
                join_inline_segments(parts)
            }
        }
    }
}

fn parse_permission_studio_optional_mode_input(
    i18n: &I18n,
    input: &str,
) -> UiResult<Option<PermissionMode>> {
    let trimmed = input.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("clear")
        || trimmed.eq_ignore_ascii_case("unset")
    {
        return Ok(None);
    }
    parse_permission_mode_token(i18n, trimmed).map(Some)
}

fn parse_permission_studio_key_input(i18n: &I18n, field: &str, input: &str) -> UiResult<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(i18n.text_args(
            "permission-studio-error-empty-value",
            &crate::fl_args!("field" => field.to_string()),
        ));
    }
    Ok(trimmed.to_string())
}

fn permission_mode_token(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Allow => "allow",
        PermissionMode::Ask => "ask",
        PermissionMode::Deny => "deny",
    }
}

fn set_path_default_mode(
    permission: &mut PermissionConfig,
    external: bool,
    read: bool,
    mode: Option<PermissionMode>,
) {
    let path = permission.path.get_or_insert_with(Default::default);
    let target = if external {
        &mut path.external
    } else {
        &mut path.workspace
    };
    let modes = target.get_or_insert_with(Default::default);
    if read {
        modes.read = mode;
    } else {
        modes.write = mode;
    }
    if modes.read.is_none() && modes.write.is_none() {
        *target = None;
    }
}

fn rename_path_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(path) = permission.path.as_mut() else {
        return;
    };
    if let Some(rule) = path.rules.shift_remove(from) {
        path.rules.insert(to.to_string(), rule);
    }
}

fn rename_network_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(network) = permission.network.as_mut() else {
        return;
    };
    if let Some(mode) = network.rules.shift_remove(from) {
        network.rules.insert(to.to_string(), mode);
    }
}

fn rename_tool_tag(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(tools) = permission.tools.as_mut() else {
        return;
    };
    if let Some(mode) = tools.tags.remove(from) {
        tools.tags.insert(to.to_string(), mode);
    }
}

fn rename_tool_name(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(tools) = permission.tools.as_mut() else {
        return;
    };
    if let Some(mode) = tools.names.remove(from) {
        tools.names.insert(to.to_string(), mode);
    }
}

fn rename_tool_rule(permission: &mut PermissionConfig, from: &str, to: &str) {
    if from == to {
        return;
    }
    let Some(tools) = permission.tools.as_mut() else {
        return;
    };
    if let Some(rule) = tools.rules.remove(from) {
        tools.rules.insert(to.to_string(), rule);
    }
}

#[cfg(test)]
fn tool_rule_fallback_mode(
    permission: &PermissionConfig,
    tool_name: &str,
) -> Option<PermissionMode> {
    match permission.tools.as_ref()?.rules.get(tool_name)? {
        ToolPermissionRules::Mode(mode) => Some(*mode),
        ToolPermissionRules::Ordered(entries) => entries.get("*").copied(),
    }
}

#[cfg(test)]
fn tool_qualifier_rules(
    permission: &PermissionConfig,
    tool_name: &str,
) -> Vec<(String, PermissionMode)> {
    match permission
        .tools
        .as_ref()
        .and_then(|tools| tools.rules.get(tool_name))
    {
        Some(ToolPermissionRules::Ordered(entries)) => entries
            .iter()
            .filter(|(pattern, _)| pattern.as_str() != "*")
            .map(|(pattern, mode)| (pattern.clone(), *mode))
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
fn tool_qualifier_mode(
    permission: &PermissionConfig,
    tool_name: &str,
    pattern: &str,
) -> Option<PermissionMode> {
    match permission.tools.as_ref()?.rules.get(tool_name)? {
        ToolPermissionRules::Ordered(entries) => entries.get(pattern).copied(),
        ToolPermissionRules::Mode(_) => None,
    }
}

#[cfg(test)]
fn add_tool_qualifier_rule(
    permission: &mut PermissionConfig,
    tool_name: &str,
    pattern: &str,
    mode: PermissionMode,
) {
    set_tool_qualifier_mode(permission, tool_name, pattern, Some(mode));
}

#[cfg(test)]
fn remove_tool_qualifier_rule(permission: &mut PermissionConfig, tool_name: &str, pattern: &str) {
    let Some(tools) = permission.tools.as_mut() else {
        return;
    };
    let Some(rule) = tools.rules.get_mut(tool_name) else {
        return;
    };
    match rule {
        ToolPermissionRules::Mode(_) => {}
        ToolPermissionRules::Ordered(entries) => {
            entries.shift_remove(pattern);
            if entries.is_empty() {
                tools.rules.remove(tool_name);
            } else if entries.len() == 1
                && entries.contains_key("*")
                && let Some(mode) = entries.get("*").copied()
            {
                tools
                    .rules
                    .insert(tool_name.to_string(), ToolPermissionRules::Mode(mode));
            }
        }
    }
}

#[cfg(test)]
fn set_tool_qualifier_mode(
    permission: &mut PermissionConfig,
    tool_name: &str,
    pattern: &str,
    mode: Option<PermissionMode>,
) {
    if let Some(mode) = mode {
        let tools = permission.tools.get_or_insert_with(Default::default);
        match tools.rules.get_mut(tool_name) {
            Some(ToolPermissionRules::Mode(current)) => {
                let fallback = *current;
                let mut entries = IndexMap::new();
                entries.insert("*".to_string(), fallback);
                entries.insert(pattern.to_string(), mode);
                tools
                    .rules
                    .insert(tool_name.to_string(), ToolPermissionRules::Ordered(entries));
            }
            Some(ToolPermissionRules::Ordered(entries)) => {
                entries.insert(pattern.to_string(), mode);
            }
            None => {
                let mut entries = IndexMap::new();
                entries.insert(pattern.to_string(), mode);
                tools
                    .rules
                    .insert(tool_name.to_string(), ToolPermissionRules::Ordered(entries));
            }
        }
    } else {
        remove_tool_qualifier_rule(permission, tool_name, pattern);
    }
}

fn normalize_permission_config(permission: &mut PermissionConfig) {
    if permission
        .path
        .as_ref()
        .is_some_and(PathPermissionConfig::is_empty)
    {
        permission.path = None;
    }
    if permission
        .network
        .as_ref()
        .is_some_and(NetworkPermissionConfig::is_empty)
    {
        permission.network = None;
    }
    if permission
        .tools
        .as_ref()
        .is_some_and(ToolPermissionConfig::is_empty)
    {
        permission.tools = None;
    }
}

fn permission_mode_label(i18n: &I18n, mode: PermissionMode) -> String {
    ui_text::t(
        i18n,
        match mode {
            PermissionMode::Allow => "value-allow",
            PermissionMode::Ask => "value-ask",
            PermissionMode::Deny => "value-deny",
        },
    )
}

fn permission_mode_choice_items(i18n: &I18n) -> Vec<ChoiceItem> {
    [
        PermissionMode::Allow,
        PermissionMode::Ask,
        PermissionMode::Deny,
    ]
    .into_iter()
    .map(|mode| ChoiceItem {
        label: permission_mode_label(i18n, mode),
        detail: String::new(),
        value: permission_mode_token(mode).to_string(),
        search_text: format!(
            "{} {}",
            permission_mode_label(i18n, mode),
            permission_mode_token(mode)
        ),
    })
    .collect()
}

fn agent_config_path(agent_name: &str, suffix: &str) -> String {
    format!("agents.{}.{}", quoted_settings_segment(agent_name), suffix)
}

fn settings_studio_provider_workbench_item(
    i18n: &I18n,
    providers: &[ProviderSummaryResource],
) -> SettingsStudioItem {
    SettingsStudioItem::new(
        ui_text::t(i18n, "settings-provider-workbench-label"),
        i18n.text_args(
            "settings-provider-workbench-value",
            &crate::fl_args!("count" => providers.len() as i64),
        ),
        ui_text::t(i18n, "settings-provider-workbench-detail"),
        SettingsPickerAction::OpenProviderList,
    )
}

fn settings_studio_model_catalog_items(
    i18n: &I18n,
    response: &ModelCatalogListResponse,
) -> Vec<SettingsStudioItem> {
    vec![SettingsStudioItem::new(
        ui_text::t(i18n, "settings-model-catalog-open-label"),
        response.summary.model_count.to_string(),
        ui_text::t(i18n, "settings-model-catalog-open-detail"),
        SettingsPickerAction::OpenModelCatalogWorkbench,
    )]
}

fn settings_studio_file_items(i18n: &I18n, sources: &ConfigJsonSources) -> Vec<SettingsStudioItem> {
    vec![
        SettingsStudioItem::new(
            ui_text::t(i18n, "settings-files-open-config-label"),
            if sources.config_found {
                ui_text::t(i18n, "settings-files-open-config-present")
            } else {
                ui_text::t(i18n, "settings-files-open-config-create")
            },
            sources.config_path.display().to_string(),
            SettingsPickerAction::OpenConfigFile,
        )
        .with_path(sources.config_path.display().to_string()),
    ]
}

#[cfg(test)]
fn format_setting_field_summary(
    i18n: &I18n,
    file_value: &JsonValue,
    effective_value: &JsonValue,
) -> String {
    if !file_value.is_null() {
        if file_value == effective_value {
            i18n.text_args(
                "settings-source-configured",
                &crate::fl_args!("value" => format_setting_value_inline(file_value)),
            )
        } else {
            i18n.text_args(
                "settings-source-file-effective",
                &crate::fl_args!(
                    "file" => format_setting_value_inline(file_value),
                    "effective" => format_setting_value_inline(effective_value),
                ),
            )
        }
    } else if !effective_value.is_null() {
        i18n.text_args(
            "settings-source-effective",
            &crate::fl_args!("value" => format_setting_value_inline(effective_value)),
        )
    } else {
        ui_text::t(i18n, "settings-source-unset")
    }
}

fn format_setting_value_inline(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "unset".to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                "\"\"".to_string()
            } else if trimmed.chars().count() > 64 {
                format!("\"{}…\"", trimmed.chars().take(64).collect::<String>())
            } else {
                format!("\"{trimmed}\"")
            }
        }
        other => {
            let rendered = other.to_string();
            if rendered.chars().count() > 72 {
                format!("{}…", rendered.chars().take(72).collect::<String>())
            } else {
                rendered
            }
        }
    }
}

fn setting_value_input_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => value.clone(),
        other => other.to_string(),
    }
}

fn settings_value_edit_prompt(
    i18n: &I18n,
    field: SettingsFieldSpec,
    file_value: &JsonValue,
    effective_value: &JsonValue,
) -> String {
    let mut lines = vec![
        settings_field_display_description(i18n, field),
        i18n.text_args(
            "overlay-settings-detail-path",
            &crate::fl_args!("path" => field.path),
        ),
    ];
    if !file_value.is_null() {
        lines.push(i18n.text_args(
            "overlay-settings-edit-file-value",
            &crate::fl_args!("value" => format_setting_value_inline(file_value)),
        ));
        if file_value != effective_value {
            lines.push(i18n.text_args(
                "overlay-settings-edit-effective-value",
                &crate::fl_args!("value" => format_setting_value_inline(effective_value)),
            ));
        }
    } else {
        lines.push(i18n.text_args(
            "overlay-settings-edit-effective-value",
            &crate::fl_args!("value" => format_setting_value_inline(effective_value)),
        ));
    }
    lines.push(settings_field_help_suffix(i18n, field.kind));
    lines.join("\n")
}

fn runtime_setting_edit_prompt(
    i18n: &I18n,
    field: RuntimeSettingSpec,
    current_summary: &str,
) -> String {
    [
        runtime_setting_display_description(i18n, field),
        i18n.text_args(
            "overlay-runtime-setting-current-value",
            &crate::fl_args!("value" => current_summary.to_string()),
        ),
        settings_field_help_suffix(i18n, field.kind),
    ]
    .join("\n")
}

fn settings_field_help_suffix(i18n: &I18n, kind: SettingsFieldKind) -> String {
    match kind {
        SettingsFieldKind::String => ui_text::t(i18n, "overlay-settings-help-string"),
        SettingsFieldKind::Bool => ui_text::t(i18n, "overlay-settings-help-bool"),
        SettingsFieldKind::Integer => ui_text::t(i18n, "overlay-settings-help-integer"),
        SettingsFieldKind::Float => ui_text::t(i18n, "overlay-settings-help-float"),
    }
}

fn choice_item(value: impl Into<String>, detail: impl Into<String>) -> ChoiceItem {
    let value = value.into();
    let detail = detail.into();
    let search_text = format!("{} {}", value.to_lowercase(), detail.to_lowercase());
    ChoiceItem {
        label: value.clone(),
        detail,
        value,
        search_text,
    }
}

fn choice_item_with_value(
    label: impl Into<String>,
    value: impl Into<String>,
    detail: impl Into<String>,
) -> ChoiceItem {
    let label = label.into();
    let value = value.into();
    let detail = detail.into();
    let search_text = format!(
        "{} {} {}",
        label.to_lowercase(),
        value.to_lowercase(),
        detail.to_lowercase()
    );
    ChoiceItem {
        label,
        detail,
        value,
        search_text,
    }
}

fn dedupe_choice_items(items: Vec<ChoiceItem>) -> Vec<ChoiceItem> {
    let mut deduped = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        if seen.insert(item.value.clone()) {
            deduped.push(item);
        }
    }
    deduped
}

fn inspector_rows_to_choice_items(rows: Vec<InspectorRow>) -> Vec<ChoiceItem> {
    rows.into_iter()
        .map(|row| choice_item(row.label, row.detail))
        .collect()
}

fn inspector_rows_to_mode_choice_items(
    rows: Vec<InspectorRow>,
    display_value: fn(&str) -> String,
) -> Vec<ChoiceItem> {
    rows.into_iter()
        .map(|row| {
            let label = display_value(row.label.as_str());
            let detail = if label == row.label {
                row.detail
            } else if row.detail.trim().is_empty() {
                row.label.clone()
            } else {
                format!("{} · {}", row.label, row.detail)
            };
            choice_item_with_value(label, row.label, detail)
        })
        .collect()
}

fn boolean_choice_items(detail: &str) -> Vec<ChoiceItem> {
    vec![choice_item("true", detail), choice_item("false", detail)]
}

fn provider_studio_default_model_choice_items(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
) -> Vec<ChoiceItem> {
    let mut preferred_adapter_id = dialog.draft.default_adapter.trim().to_owned();
    if preferred_adapter_id.is_empty() {
        preferred_adapter_id = provider_studio_selected_adapter_id(dialog).unwrap_or_default();
    }
    let mut items = Vec::new();
    let mut adapter_models = dialog.adapter_models.iter().collect::<Vec<_>>();
    adapter_models.sort_by_key(|adapter_models| {
        (
            adapter_models.adapter_id != preferred_adapter_id,
            adapter_models.adapter_id.clone(),
        )
    });
    for adapter_models in adapter_models {
        for model in &adapter_models.models {
            if !dialog.selected_model_keys.is_empty()
                && !provider_studio_model_selected(
                    dialog,
                    adapter_models.adapter_id.as_str(),
                    model.id.as_str(),
                )
            {
                continue;
            }
            let mut detail_parts = vec![adapter_models.adapter_id.clone()];
            if let Some(display_name) = model
                .display_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                detail_parts.push(display_name.trim().to_owned());
            }
            let key =
                provider_studio_model_key(adapter_models.adapter_id.as_str(), model.id.as_str());
            detail_parts.push(provider_studio_catalog_match_label(
                i18n,
                dialog
                    .catalog_matches
                    .get(key.as_str())
                    .map(|entry| entry.model_id.as_str()),
            ));
            items.push(choice_item(
                model.id.to_string(),
                join_inline_segments(detail_parts),
            ));
        }
    }
    dedupe_choice_items(items)
}

fn provider_studio_profile_choice_items(i18n: &I18n, backend: &Backend) -> Vec<ChoiceItem> {
    let mut items = backend
        .list_aws_profile_names()
        .into_iter()
        .map(|profile| choice_item(profile, ui_text::t(i18n, "provider-profile-choice-detail")))
        .collect::<Vec<_>>();
    if !items.iter().any(|item| item.value == "default") {
        items.insert(
            0,
            choice_item(
                "default",
                ui_text::t(i18n, "provider-profile-default-detail"),
            ),
        );
    }
    dedupe_choice_items(items)
}

fn provider_studio_api_key_env_choice_items(i18n: &I18n) -> Vec<ChoiceItem> {
    let items = vec![
        choice_item(
            "OPENAI_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-openai-detail"),
        ),
        choice_item(
            "ANTHROPIC_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-anthropic-detail"),
        ),
        choice_item(
            "GEMINI_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-gemini-detail"),
        ),
        choice_item(
            "GITLAB_TOKEN",
            ui_text::t(i18n, "provider-api-key-env-gitlab-detail"),
        ),
        choice_item(
            "GOOGLE_VERTEX_ACCESS_TOKEN",
            ui_text::t(i18n, "provider-api-key-env-vertex-detail"),
        ),
        choice_item(
            "SHARED_GATEWAY_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-shared-gateway-detail"),
        ),
        choice_item(
            "OPENCODE_API_KEY",
            ui_text::t(i18n, "provider-api-key-env-opencode-detail"),
        ),
    ];
    dedupe_choice_items(items)
}

fn provider_studio_field_allows_clear(field: ProviderStudioField) -> bool {
    matches!(
        field,
        ProviderStudioField::AuthMode
            | ProviderStudioField::AuthSubtype
            | ProviderStudioField::BaseUrl
            | ProviderStudioField::InstanceUrl
            | ProviderStudioField::ApiKeySource
            | ProviderStudioField::ApiKeyValue
            | ProviderStudioField::RedirectUri
            | ProviderStudioField::CallbackUrl
            | ProviderStudioField::RefreshToken
            | ProviderStudioField::AccessToken
            | ProviderStudioField::ExpiresAtMs
            | ProviderStudioField::AccountId
            | ProviderStudioField::EnterpriseDomain
            | ProviderStudioField::Region
            | ProviderStudioField::Profile
            | ProviderStudioField::AccessKeyId
            | ProviderStudioField::SecretAccessKey
            | ProviderStudioField::SessionToken
            | ProviderStudioField::ServiceKeyEnv
            | ProviderStudioField::DefaultAdapter
            | ProviderStudioField::DefaultModel
    )
}

fn choice_overlay_clear_detail(i18n: &I18n, action: &ChoiceOverlayAction) -> String {
    match action {
        ChoiceOverlayAction::SettingsField(field) => i18n.text_args(
            "overlay-choice-clear-settings-detail",
            &crate::fl_args!("field" => field.path),
        ),
        ChoiceOverlayAction::RuntimeSetting(field) => i18n.text_args(
            "overlay-choice-clear-runtime-detail",
            &crate::fl_args!("field" => runtime_setting_display_label(i18n, *field)),
        ),
        ChoiceOverlayAction::SessionModelVariant(step) => i18n.text_args(
            "overlay-choice-clear-runtime-detail",
            &crate::fl_args!(
                "field" => runtime_setting_display_label(i18n, session_model_variant_field(*step))
            ),
        ),
        ChoiceOverlayAction::ProviderDefaultWizard(_, _) => {
            ui_text::t(i18n, "overlay-choice-clear-provider-default-detail")
        }
        ChoiceOverlayAction::ProviderStudioField(field) => i18n.text_args(
            "overlay-choice-clear-provider-detail",
            &crate::fl_args!("field" => provider_studio_field_label(i18n, *field)),
        ),
        ChoiceOverlayAction::ProviderStudioModelField(field) => i18n.text_args(
            "overlay-choice-clear-provider-detail",
            &crate::fl_args!("field" => provider_model_config_field_label(i18n, *field)),
        ),
        ChoiceOverlayAction::PermissionRuleStudio(field) => match field {
            PermissionRuleStudioChoiceField::SubjectKind => {
                ui_text::t(i18n, "overlay-choice-clear-permission-subject")
            }
            PermissionRuleStudioChoiceField::PathAccessKind => {
                ui_text::t(i18n, "overlay-choice-clear-permission-access-kind")
            }
            PermissionRuleStudioChoiceField::Scope => {
                ui_text::t(i18n, "overlay-choice-clear-permission-scope")
            }
            PermissionRuleStudioChoiceField::Mode => {
                ui_text::t(i18n, "overlay-choice-clear-permission-mode")
            }
        },
        ChoiceOverlayAction::PermissionStudioMode(target) => i18n.text_args(
            "overlay-choice-clear-permission-override-detail",
            &crate::fl_args!("field" => permission_studio_mode_target_label(i18n, target)),
        ),
    }
}

fn parse_settings_field_input(
    i18n: &I18n,
    field: SettingsFieldSpec,
    input: &str,
) -> std::result::Result<Option<JsonValue>, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("clear") {
        return Ok(None);
    }
    match field.kind {
        SettingsFieldKind::String => Ok(Some(JsonValue::String(trimmed.to_string()))),
        SettingsFieldKind::Bool => {
            let value = match trimmed.to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" | "1" => true,
                "false" | "off" | "no" | "0" => false,
                _ => {
                    return Err(i18n.text_args(
                        "settings-field-parse-bool",
                        &crate::fl_args!("field" => field.path),
                    ));
                }
            };
            Ok(Some(JsonValue::Bool(value)))
        }
        SettingsFieldKind::Integer => {
            let value = trimmed.parse::<u64>().map_err(|_| {
                i18n.text_args(
                    "settings-field-parse-integer",
                    &crate::fl_args!("field" => field.path),
                )
            })?;
            Ok(Some(JsonValue::from(value)))
        }
        SettingsFieldKind::Float => {
            let value = trimmed.parse::<f64>().map_err(|_| {
                i18n.text_args(
                    "settings-field-parse-float",
                    &crate::fl_args!("field" => field.path),
                )
            })?;
            Ok(Some(JsonValue::from(value)))
        }
    }
}

fn provider_studio_provider_rows(
    i18n: &I18n,
    providers: &[ProviderSummaryResource],
) -> Vec<ProviderStudioProviderRow> {
    let mut rows = vec![ProviderStudioProviderRow {
        provider_id: None,
        label: ui_text::t(i18n, "settings-provider-new-label"),
        detail: ui_text::t(i18n, "overlay-provider-studio-new-provider-detail"),
    }];
    rows.extend(providers.iter().map(|provider| ProviderStudioProviderRow {
        provider_id: Some(provider.provider_id.clone()),
        label: provider.provider_id.clone(),
        detail: i18n.text_args(
            "overlay-provider-studio-provider-row-detail",
            &crate::fl_args!(
                "adapter" => provider
                    .defaults
                    .adapter
                    .clone()
                    .unwrap_or_else(|| settings_choice_adapter_fallback(i18n)),
                "model" => provider.defaults.model.clone(),
                "count" => provider.adapters.len() as i64,
            ),
        ),
    }));
    rows
}

fn provider_list_create_item(i18n: &I18n) -> PickerItem {
    PickerItem {
        label: ui_text::t(i18n, "overlay-provider-list-create-label"),
        detail: ui_text::t(i18n, "overlay-provider-list-create-detail"),
        value: PickerValue::ProviderCreate,
    }
}

fn i18n_provider_list_detail(i18n: &I18n, provider: &ProviderSummaryResource) -> String {
    i18n.text_args(
        "overlay-provider-list-row-detail",
        &crate::fl_args!(
            "adapter" => provider
                .defaults
                .adapter
                .clone()
                .unwrap_or_else(|| settings_choice_adapter_fallback(i18n)),
            "model" => provider.defaults.model.clone(),
            "count" => provider.adapters.len() as i64,
        ),
    )
}

fn session_model_choice_item(
    i18n: &I18n,
    provider_id: &str,
    default_adapter: Option<&str>,
    model: ProviderModel,
) -> SessionModelChoiceItem {
    let adapter_id = model
        .adapter_id
        .as_ref()
        .map(ToString::to_string)
        .or_else(|| default_adapter.map(str::to_owned));
    let model_ref = adapter_id
        .as_deref()
        .map(|adapter_id| ModelRef::new_with_adapter(provider_id, adapter_id, model.id.as_str()))
        .unwrap_or_else(|| ModelRef::new(provider_id, model.id.as_str()));
    let display_name = model
        .display_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    let adapter_label = adapter_id
        .clone()
        .unwrap_or_else(|| ui_text::t(i18n, "value-default"));
    let context_window = model
        .metadata
        .limits
        .context_window_tokens
        .map(|value| {
            i18n.text_args(
                "session-model-context-window",
                &crate::fl_args!("value" => value as i64),
            )
        })
        .unwrap_or_else(|| {
            i18n.text_args(
                "session-model-context-window",
                &crate::fl_args!("value" => ui_text::t(i18n, "value-unknown")),
            )
        });
    let mut detail_parts = vec![provider_id.to_owned(), adapter_label, context_window];
    if !display_name.is_empty() && display_name != model.id.as_str() {
        detail_parts.push(display_name.clone());
    }
    let search_text = format!(
        "{} {} {} {}",
        provider_id,
        model_ref
            .adapter_id
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        model.id,
        display_name
    )
    .to_ascii_lowercase();
    SessionModelChoiceItem {
        label: format!(
            "{provider_id} / {} / {}",
            model_ref
                .adapter_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            model.id
        ),
        detail: join_inline_segments(detail_parts),
        search_text,
        model: model_ref,
    }
}

fn session_model_matches_current(candidate: &ModelRef, current: &ModelRef) -> bool {
    candidate.provider_id == current.provider_id
        && candidate.model_id == current.model_id
        && (candidate.adapter_id == current.adapter_id || current.adapter_id.is_none())
}

fn provider_model_catalog_lookup_id(model: &ProviderModel) -> String {
    model
        .catalog_model_id
        .as_ref()
        .map(ToString::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| agena::model_catalog::canonical_model_catalog_id(model.id.as_str()))
}

fn provider_studio_catalog_match_model<'a>(
    model: &ProviderModel,
    catalog_models: &'a [CatalogModelResource],
) -> Option<&'a CatalogModelResource> {
    let lookup_id = provider_model_catalog_lookup_id(model);
    catalog_models
        .iter()
        .filter(|catalog_model| {
            catalog_model.model_id == model.id.as_str() || catalog_model.model_id == lookup_id
        })
        .min_by_key(|catalog_model| catalog_model.model_id.as_str())
}

impl SessionViewMode {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Roots,
            Self::Roots => Self::Subtree,
            Self::Subtree => Self::All,
        }
    }

    fn label(self, i18n: &I18n, subtree_root_id: Option<i64>) -> String {
        match (self, subtree_root_id) {
            (Self::All, _) => ui_text::t(i18n, "session-view-all"),
            (Self::Roots, _) => ui_text::t(i18n, "session-view-roots"),
            (Self::Subtree, Some(root_id)) => i18n.text_args(
                "session-view-subtree-root",
                &crate::fl_args!("id" => root_id),
            ),
            (Self::Subtree, None) => ui_text::t(i18n, "session-view-subtree"),
        }
    }
}

impl SessionListState {
    fn current_selected(&self) -> Option<&SessionResource> {
        self.list.selected_item()
    }

    fn current_selected_id(&self) -> Option<i64> {
        self.current_selected().map(|item| item.id)
    }

    fn clamp_selection(&mut self) {
        self.list.clamp_selection();
    }

    fn move_selection(&mut self, delta: isize) {
        self.list.move_selection(delta);
    }

    fn should_load_more(&self) -> bool {
        false
    }

    fn select_by_id(&mut self, session_id: i64) -> bool {
        if let Some(index) = self
            .list
            .items
            .iter()
            .position(|item| item.id == session_id)
        {
            self.list.selected = index;
            true
        } else {
            false
        }
    }
}

impl DraftStore {
    fn load(path: &Path) -> UiResult<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.to_string()),
        };
        let persistent = serde_json::from_str::<PersistentDraftStore>(raw.as_str())
            .map_err(|error| format!("invalid draft store {}: {error}", path.display()))?;
        Ok(persistent.into_store())
    }

    fn persist(&self, path: &Path) -> UiResult<()> {
        let persistent = PersistentDraftStore::from_store(self);
        if persistent.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            return Ok(());
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let raw = serde_json::to_string_pretty(&persistent).map_err(|error| error.to_string())?;
        let tmp_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("{name}.tmp"))
            .unwrap_or_else(|| "tui-drafts.json.tmp".to_string());
        let tmp_path = path.with_file_name(tmp_name);
        fs::write(&tmp_path, raw).map_err(|error| error.to_string())?;
        fs::rename(&tmp_path, path).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn get(&self, slot: DraftSlot) -> Option<&ComposerDraft> {
        self.drafts.get(&slot)
    }

    fn set(&mut self, slot: DraftSlot, draft: ComposerDraft) -> bool {
        if draft.is_empty() {
            return self.clear(slot);
        }
        match self.drafts.get(&slot) {
            Some(existing) if existing == &draft => false,
            _ => {
                self.drafts.insert(slot, draft);
                true
            }
        }
    }

    fn clear(&mut self, slot: DraftSlot) -> bool {
        self.drafts.remove(&slot).is_some()
    }
}

impl PromptHistory {
    fn load(path: &Path) -> UiResult<Self> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.to_string()),
        };

        let mut history = Self::default();
        for (index, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let entry = serde_json::from_str::<PromptHistoryRecord>(line).map_err(|error| {
                format!(
                    "invalid prompt history {}:{}: {error}",
                    path.display(),
                    index + 1
                )
            })?;
            if let Some(text) = Self::normalized_text(entry.text.as_str()) {
                history.push(text);
            }
        }
        Ok(history)
    }

    fn persist(&self, path: &Path) -> UiResult<()> {
        if self.items.is_empty() {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            return Ok(());
        }

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }

        let mut raw = String::new();
        for text in &self.items {
            let line = serde_json::to_string(&PromptHistoryRecord { text: text.clone() })
                .map_err(|error| error.to_string())?;
            raw.push_str(line.as_str());
            raw.push('\n');
        }

        let tmp_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| format!("{name}.tmp"))
            .unwrap_or_else(|| "tui-prompt-history.jsonl.tmp".to_string());
        let tmp_path = path.with_file_name(tmp_name);
        fs::write(&tmp_path, raw).map_err(|error| error.to_string())?;
        fs::rename(&tmp_path, path).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn normalized_text(text: &str) -> Option<String> {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }

    fn push(&mut self, text: String) -> bool {
        if self.items.last().is_some_and(|item| item == &text) {
            return false;
        }
        self.items.retain(|item| item != &text);
        self.items.push(text);
        if self.items.len() > MAX_PROMPT_HISTORY_ENTRIES {
            let excess = self.items.len() - MAX_PROMPT_HISTORY_ENTRIES;
            self.items.drain(0..excess);
        }
        true
    }

    fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn len(&self) -> usize {
        self.items.len()
    }

    fn get(&self, index: usize) -> Option<&str> {
        self.items.get(index).map(String::as_str)
    }
}

impl ComposerDraft {
    fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.items.is_empty()
    }

    fn persistent_snapshot(&self) -> Option<PersistentComposerDraft> {
        let mut items_by_placeholder = self
            .items
            .iter()
            .filter_map(|item| {
                item.persistent_item()
                    .map(|persistent| (item.placeholder().to_string(), persistent))
            })
            .collect::<BTreeMap<_, _>>();
        let mut elements = self.elements.clone();
        elements.sort_by_key(|element| element.range.start);

        let mut text = String::new();
        let mut persistent_items = Vec::new();
        let mut persistent_elements = Vec::new();
        let mut cursor = 0;

        for element in elements {
            let start = min(element.range.start, self.text.len());
            let end = min(element.range.end, self.text.len());
            if cursor < start {
                text.push_str(&self.text[cursor..start]);
            }

            if let Some(placeholder) = self.text.get(start..end)
                && let Some(item) = items_by_placeholder.remove(placeholder)
            {
                let range = text.len()..text.len() + placeholder.len();
                text.push_str(placeholder);
                persistent_items.push(item);
                persistent_elements.push(PersistentComposerDraftElement {
                    placeholder: placeholder.to_string(),
                    start: range.start,
                    end: range.end,
                });
            }

            cursor = end;
        }

        if cursor < self.text.len() {
            text.push_str(&self.text[cursor..]);
        }

        let draft = PersistentComposerDraft {
            text,
            items: persistent_items,
            elements: persistent_elements,
        };
        (!draft.text.trim().is_empty() || !draft.items.is_empty()).then_some(draft)
    }
}

impl ComposerItem {
    fn placeholder(&self) -> &str {
        match self {
            Self::Attachment(attachment) => attachment.placeholder.as_str(),
            Self::LargePaste(paste) => paste.placeholder.as_str(),
        }
    }

    fn short_label(&self) -> &str {
        match self {
            Self::Attachment(attachment) => attachment.label.as_str(),
            Self::LargePaste(paste) => paste.label.as_str(),
        }
    }

    fn persistent_item(&self) -> Option<PersistentComposerItem> {
        match self {
            Self::Attachment(attachment) => (!attachment.is_temp).then(|| {
                PersistentComposerItem::Attachment(PersistentAttachment {
                    path: attachment.path.clone(),
                    placeholder: attachment.placeholder.clone(),
                    label: attachment.label.clone(),
                })
            }),
            Self::LargePaste(paste) => Some(PersistentComposerItem::LargePaste(PersistentPaste {
                placeholder: paste.placeholder.clone(),
                label: paste.label.clone(),
                text: paste.text.clone(),
            })),
        }
    }
}

impl PersistentDraftStore {
    fn is_empty(&self) -> bool {
        self.new_session.is_none() && self.sessions.is_empty()
    }

    fn from_store(store: &DraftStore) -> Self {
        let mut sessions = BTreeMap::new();
        let mut new_session = None;

        for (slot, draft) in &store.drafts {
            let Some(persistent) = draft.persistent_snapshot() else {
                continue;
            };
            match slot {
                DraftSlot::Session(session_id) => {
                    sessions.insert(*session_id, persistent);
                }
                DraftSlot::NewSession => {
                    new_session = Some(persistent);
                }
            }
        }

        Self {
            version: persistent_draft_store_version(),
            sessions,
            new_session,
        }
    }

    fn into_store(self) -> DraftStore {
        let mut drafts = BTreeMap::new();
        if let Some(draft) = self.new_session {
            drafts.insert(DraftSlot::NewSession, draft.into_draft());
        }
        for (session_id, draft) in self.sessions {
            drafts.insert(DraftSlot::Session(session_id), draft.into_draft());
        }
        DraftStore { drafts }
    }
}

impl PersistentComposerDraft {
    fn into_draft(self) -> ComposerDraft {
        ComposerDraft {
            text: self.text,
            items: self
                .items
                .into_iter()
                .map(|item| item.into_item())
                .collect(),
            elements: self
                .elements
                .into_iter()
                .map(|element| ComposerDraftElement {
                    placeholder: element.placeholder,
                    range: element.start..element.end,
                })
                .collect(),
        }
    }
}

impl PersistentComposerItem {
    fn into_item(self) -> ComposerItem {
        match self {
            Self::Attachment(attachment) => ComposerItem::Attachment(StagedAttachment {
                path: attachment.path,
                placeholder: attachment.placeholder,
                label: attachment.label,
                is_temp: false,
            }),
            Self::LargePaste(paste) => ComposerItem::LargePaste(StagedPaste {
                placeholder: paste.placeholder,
                label: paste.label,
                text: paste.text,
            }),
        }
    }
}

impl Default for TranscriptState {
    fn default() -> Self {
        Self::new(
            I18n::english(),
            TranscriptDetailDefaults {
                tool_output_expanded: false,
                thinking_expanded: false,
            },
        )
    }
}

impl TranscriptState {
    fn new(i18n: I18n, detail_expanded_by_default: TranscriptDetailDefaults) -> Self {
        Self {
            i18n,
            session_id: None,
            session_title: String::new(),
            messages: Vec::new(),
            older_cursor: None,
            has_more_older: false,
            loading_initial: false,
            loading_older: false,
            refreshing: false,
            state_loading: false,
            submitting: false,
            pending_restore_draft: None,
            follow_tail: true,
            scroll: 0,
            cursor_line: 0,
            block_cursor: None,
            search_query: String::new(),
            search_match_index: None,
            execution: None,
            last_event_seq: None,
            detail_expanded_by_default,
            node_expansions: BTreeMap::new(),
            rendered: None,
        }
    }

    fn reset(&mut self, session_id: i64, title: String) {
        self.session_id = Some(session_id);
        self.session_title = title;
        self.messages.clear();
        self.older_cursor = None;
        self.has_more_older = false;
        self.loading_initial = false;
        self.loading_older = false;
        self.refreshing = false;
        self.state_loading = false;
        self.submitting = false;
        self.pending_restore_draft = None;
        self.follow_tail = true;
        self.scroll = 0;
        self.cursor_line = 0;
        self.block_cursor = None;
        self.execution = None;
        self.last_event_seq = None;
        self.search_query.clear();
        self.search_match_index = None;
        self.node_expansions.clear();
        self.invalidate_render();
    }

    fn apply_execution(&mut self, execution: SessionExecutionResource) {
        self.session_title = execution.session.title.clone();
        self.last_event_seq = execution.latest_event_seq;
        self.execution = Some(execution);
        self.invalidate_render();
    }

    fn replace_messages(
        &mut self,
        page: PaginatedResponse<MessageResource>,
        width: u16,
        height: u16,
    ) {
        self.messages = page.items;
        self.older_cursor = page.page.next_cursor;
        self.has_more_older = page.page.has_more;
        self.invalidate_render();
        if self.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }
    }

    fn prepend_messages(
        &mut self,
        page: PaginatedResponse<MessageResource>,
        width: u16,
        height: u16,
    ) {
        let old_total = self.rendered(width).lines.len();
        let mut merged = page.items;
        merged.extend(self.messages.clone());
        merged.sort_by_key(message_sort_key);
        merged.dedup_by_key(|message| message.id);
        self.messages = merged;
        self.older_cursor = page.page.next_cursor;
        self.has_more_older = page.page.has_more;
        self.invalidate_render();
        let new_total = self.rendered(width).lines.len();
        self.scroll = self
            .scroll
            .saturating_add(new_total.saturating_sub(old_total));
        self.cursor_line = self
            .cursor_line
            .saturating_add(new_total.saturating_sub(old_total));
        self.clamp_scroll(width, height);
    }

    fn merge_latest_messages(
        &mut self,
        page: PaginatedResponse<MessageResource>,
        width: u16,
        height: u16,
    ) {
        let latest_ids = page
            .items
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        let mut merged = self
            .messages
            .iter()
            .filter(|message| !latest_ids.contains(&message.id))
            .cloned()
            .collect::<Vec<_>>();
        for incoming in page.items {
            if let Some(existing) = self
                .messages
                .iter()
                .find(|message| message.id == incoming.id)
            {
                merged.push(merge_message_resources(existing, &incoming));
            } else {
                merged.push(incoming);
            }
        }
        merged.sort_by_key(message_sort_key);
        merged.dedup_by_key(|message| message.id);
        self.messages = merged;
        self.invalidate_render();
        if self.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }
    }

    fn apply_live_event(&mut self, event: &DomainEvent, width: u16, height: u16) -> bool {
        let refresh_needed = match &event.kind {
            // The transcript now comes from the server-side collapsed
            // conversation projection. Raw live message events can describe
            // intermediate assistant passes that are intentionally hidden
            // from the user-visible transcript, so we always re-fetch the
            // latest projection instead of mutating the local message list.
            AgenaSessionEvent::UserMessageAppended(_)
            | AgenaSessionEvent::MessagePartUpdated(_)
            | AgenaSessionEvent::MessagePartDelta(_)
            | AgenaSessionEvent::AssistantMessageCompleted(_) => true,
            _ => false,
        };

        if !refresh_needed && let Some(seq) = event.meta.seq_session {
            self.last_event_seq = Some(seq);
        }

        if self.follow_tail {
            self.scroll_to_bottom(width, height);
        } else {
            self.clamp_scroll(width, height);
        }

        refresh_needed
    }

    fn set_search_query(&mut self, query: String) {
        self.search_query = query;
        self.search_match_index = None;
        self.invalidate_render();
    }

    fn current_search_match_count(&self) -> usize {
        self.rendered
            .as_ref()
            .map(|rendered| rendered.search_matches.len())
            .unwrap_or(0)
    }

    fn current_search_match_number(&self) -> usize {
        match (self.search_match_index, self.current_search_match_count()) {
            (Some(index), count) if count > 0 => min(index + 1, count),
            _ => 0,
        }
    }

    fn jump_search_match(&mut self, width: u16, height: u16, forward: bool) {
        let matches = self.rendered(width).search_matches.clone();
        if matches.is_empty() {
            self.search_match_index = None;
            return;
        }

        let next_index = match (self.search_match_index, forward) {
            (None, true) => 0,
            (None, false) => matches.len().saturating_sub(1),
            (Some(index), true) => (index + 1) % matches.len(),
            (Some(0), false) => matches.len().saturating_sub(1),
            (Some(index), false) => index.saturating_sub(1),
        };

        self.search_match_index = Some(next_index);
        let line = matches[next_index];
        self.set_cursor_line(width, height, line);
    }

    fn jump_to_message(&mut self, width: u16, height: u16, message_id: i64) {
        let rendered = self.rendered(width);
        let Some((_, line)) = rendered
            .message_line_starts
            .iter()
            .find(|(candidate_id, _)| *candidate_id == message_id)
            .copied()
        else {
            return;
        };
        self.set_cursor_line(width, height, line);
    }

    fn highlighted_block_key(&self) -> Option<TranscriptNodeKey> {
        self.block_cursor.as_ref().map(|cursor| cursor.key.clone())
    }

    fn highlighted_block_range(&mut self, width: u16) -> Option<Range<usize>> {
        let key = self.highlighted_block_key()?;
        let rendered = self.rendered(width);
        rendered
            .nodes
            .iter()
            .find(|node| node.key == key)
            .map(|node| node.start_line..node.end_line)
    }

    fn step_line_with_block_selection(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
    ) {
        let total_lines = self.rendered(width).lines.len();
        if total_lines == 0 {
            self.block_cursor = None;
            self.cursor_line = 0;
            return;
        }

        let current_line = min(self.cursor_line, total_lines.saturating_sub(1));
        if let Some(block_cursor) = self.block_cursor.as_ref() {
            let highlighted_key = block_cursor.key.clone();
            let highlighted_direction = block_cursor.direction;
            if let Some(current_node) = self.current_cursor_node_cloned(width) {
                if current_node.key == highlighted_key && highlighted_direction == direction {
                    self.block_cursor = None;
                    self.set_cursor_line(width, height, current_line);
                    return;
                }
            }
        }

        let next_line = match direction {
            TranscriptMoveDirection::Up => current_line.saturating_sub(1),
            TranscriptMoveDirection::Down => min(
                current_line.saturating_add(1),
                total_lines.saturating_sub(1),
            ),
        };
        if next_line == current_line {
            self.block_cursor = None;
            self.set_cursor_line(width, height, current_line);
            return;
        }

        let (current_node, next_node) = {
            let rendered = self.rendered(width);
            (
                rendered
                    .line_nodes
                    .get(current_line)
                    .and_then(|value| *value),
                rendered.line_nodes.get(next_line).and_then(|value| *value),
            )
        };
        if let Some(next_node_index) = next_node
            && current_node != Some(next_node_index)
        {
            self.set_block_cursor(width, height, next_node_index, direction);
            return;
        }

        self.block_cursor = None;
        self.set_cursor_line(width, height, next_line);
    }

    fn step_block(&mut self, width: u16, height: u16, direction: TranscriptMoveDirection) {
        let target_node = match self.current_highlighted_node_index(width) {
            Some(current) => {
                let rendered = self.rendered(width);
                match direction {
                    TranscriptMoveDirection::Up => current.checked_sub(1),
                    TranscriptMoveDirection::Down => {
                        (current + 1 < rendered.nodes.len()).then_some(current + 1)
                    }
                }
            }
            None => {
                let cursor_line = self.cursor_line;
                let rendered = self.rendered(width);
                match direction {
                    TranscriptMoveDirection::Up => rendered
                        .nodes
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, node)| node.end_line <= cursor_line)
                        .map(|(index, _)| index),
                    TranscriptMoveDirection::Down => rendered
                        .nodes
                        .iter()
                        .enumerate()
                        .find(|(_, node)| node.start_line > cursor_line)
                        .map(|(index, _)| index),
                }
            }
        };
        if let Some(target_node) = target_node {
            self.set_block_cursor(width, height, target_node, direction);
        }
    }

    fn move_by_blocks(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
        count: usize,
    ) {
        for _ in 0..count.max(1) {
            self.step_block(width, height, direction);
        }
    }

    fn scroll_by_lines_with_blocks(
        &mut self,
        width: u16,
        height: u16,
        direction: TranscriptMoveDirection,
        count: usize,
    ) {
        for _ in 0..count.max(1) {
            self.step_line_with_block_selection(width, height, direction);
        }
    }

    fn should_load_older(&self) -> bool {
        self.session_id.is_some()
            && self.has_more_older
            && !self.loading_initial
            && !self.loading_older
            && self.scroll <= 2
    }

    fn rendered(&mut self, width: u16) -> &RenderedTranscript {
        if self
            .rendered
            .as_ref()
            .is_some_and(|rendered| rendered.width == width)
        {
            return self.rendered.as_ref().expect("render cache should exist");
        }

        let mut lines = Vec::new();
        let mut message_line_starts = Vec::new();
        let mut nodes = Vec::new();
        let mut line_nodes = Vec::new();
        if self.session_id.is_some() {
            if self.loading_older {
                lines.push(RenderedLine::dim(ui_text::t(
                    &self.i18n,
                    "transcript-loading-older",
                )));
                line_nodes.push(None);
            } else if self.has_more_older {
                lines.push(RenderedLine::dim(ui_text::t(
                    &self.i18n,
                    "transcript-more-older",
                )));
                line_nodes.push(None);
            }
        }

        if self.messages.is_empty() && self.session_id.is_some() && !self.loading_initial {
            lines.push(RenderedLine::dim(ui_text::t(
                &self.i18n,
                "transcript-empty-session",
            )));
            line_nodes.push(None);
        }

        for message in &self.messages {
            message_line_starts.push((message.id, lines.len()));
            let rendered = render_message_detailed(
                message,
                width,
                &self.i18n,
                self.detail_expanded_by_default,
                &self.node_expansions,
            );
            let base_line = lines.len();
            let base_node = nodes.len();
            lines.extend(rendered.lines);
            nodes.extend(
                rendered
                    .nodes
                    .into_iter()
                    .map(|node| RenderedTranscriptNode {
                        start_line: node.start_line.saturating_add(base_line),
                        end_line: node.end_line.saturating_add(base_line),
                        ..node
                    }),
            );
            let added_lines = lines.len().saturating_sub(base_line);
            line_nodes.extend((0..added_lines).map(|offset| {
                nodes
                    .iter()
                    .enumerate()
                    .skip(base_node)
                    .find(|(_, node)| {
                        let line_index = base_line.saturating_add(offset);
                        line_index >= node.start_line && line_index < node.end_line
                    })
                    .map(|(index, _)| index)
            }));
        }

        let search_matches = if self.search_query.trim().is_empty() {
            Vec::new()
        } else {
            lines
                .iter()
                .enumerate()
                .filter(|(_, line)| {
                    contains_case_insensitive(&line.text, self.search_query.as_str())
                })
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        };

        self.rendered = Some(RenderedTranscript {
            width,
            lines,
            search_matches,
            message_line_starts,
            nodes,
            line_nodes,
        });
        self.rendered.as_ref().expect("render cache should exist")
    }

    fn invalidate_render(&mut self) {
        self.rendered = None;
    }

    fn clamp_scroll(&mut self, width: u16, height: u16) {
        let max_scroll = self.max_scroll(width, height);
        self.scroll = min(self.scroll, max_scroll);
        self.cursor_line = min(
            self.cursor_line,
            self.rendered(width).lines.len().saturating_sub(1),
        );
        if self.current_highlighted_node_index(width).is_none() {
            self.block_cursor = None;
        }
    }

    fn scroll_to_bottom(&mut self, width: u16, height: u16) {
        self.scroll = self.max_scroll(width, height);
        self.follow_tail = true;
        self.cursor_line = self.rendered(width).lines.len().saturating_sub(1);
        self.block_cursor = None;
    }

    fn scroll_to_top(&mut self, width: u16, height: u16) {
        self.scroll = 0;
        self.cursor_line = 0;
        self.block_cursor = None;
        self.follow_tail = self.is_at_bottom(width, height);
    }

    fn scroll_by_lines(&mut self, width: u16, height: u16, delta: isize) {
        self.follow_tail = false;
        self.block_cursor = None;
        let next = if delta.is_negative() {
            self.cursor_line.saturating_sub(delta.unsigned_abs())
        } else {
            self.cursor_line.saturating_add(delta as usize)
        };
        self.set_cursor_line(width, height, next);
    }

    fn scroll_by_page(&mut self, width: u16, height: u16, forward: bool) {
        let page = height.max(1) as usize;
        self.scroll_by_lines(
            width,
            height,
            if forward {
                page as isize
            } else {
                -(page as isize)
            },
        );
    }

    fn scroll_by_half_page(&mut self, width: u16, height: u16, forward: bool) {
        let half_page = (height.max(1) as usize).saturating_add(1) / 2;
        self.scroll_by_lines(
            width,
            height,
            if forward {
                half_page as isize
            } else {
                -(half_page as isize)
            },
        );
    }

    fn max_scroll(&mut self, width: u16, height: u16) -> usize {
        let visible = height.max(1) as usize;
        self.rendered(width).lines.len().saturating_sub(visible)
    }

    fn is_at_bottom(&mut self, width: u16, height: u16) -> bool {
        self.scroll >= self.max_scroll(width, height)
    }

    fn set_cursor_line(&mut self, width: u16, height: u16, target: usize) {
        let total_lines = self.rendered(width).lines.len();
        self.cursor_line = if total_lines == 0 {
            0
        } else {
            min(target, total_lines.saturating_sub(1))
        };
        self.block_cursor = None;
        self.follow_tail = false;
        let visible = height.max(1) as usize;
        if self.cursor_line < self.scroll {
            self.scroll = self.cursor_line;
        } else if self.cursor_line >= self.scroll.saturating_add(visible) {
            self.scroll = self.cursor_line.saturating_add(1).saturating_sub(visible);
        }
        self.clamp_scroll(width, height);
        self.follow_tail = self.is_at_bottom(width, height);
    }

    fn current_cursor_node<'a>(&'a mut self, width: u16) -> Option<&'a RenderedTranscriptNode> {
        let node_index = self.current_highlighted_node_index(width)?;
        let rendered = self.rendered(width);
        rendered.nodes.get(node_index)
    }

    fn current_cursor_node_cloned(&mut self, width: u16) -> Option<RenderedTranscriptNode> {
        self.current_cursor_node(width).cloned()
    }

    fn current_highlighted_node_index(&mut self, width: u16) -> Option<usize> {
        if let Some(block_cursor) = self.block_cursor.as_ref() {
            let highlighted_key = block_cursor.key.clone();
            let block_index = {
                let rendered = self.rendered(width);
                rendered
                    .nodes
                    .iter()
                    .position(|node| node.key == highlighted_key)
            };
            if let Some(index) = block_index {
                return Some(index);
            }
            self.block_cursor = None;
        }
        let cursor_line = self.cursor_line;
        let rendered = self.rendered(width);
        rendered
            .line_nodes
            .get(cursor_line)
            .and_then(|value| *value)
    }

    fn set_block_cursor(
        &mut self,
        width: u16,
        height: u16,
        node_index: usize,
        direction: TranscriptMoveDirection,
    ) {
        let target_line = {
            let rendered = self.rendered(width);
            let Some(node) = rendered.nodes.get(node_index) else {
                return;
            };
            match direction {
                TranscriptMoveDirection::Up => node.end_line.saturating_sub(1),
                TranscriptMoveDirection::Down => node.start_line,
            }
        };
        self.set_cursor_line(width, height, target_line);
        let key = {
            let rendered = self.rendered(width);
            rendered.nodes.get(node_index).map(|node| node.key.clone())
        };
        self.block_cursor = key.map(|key| TranscriptBlockCursor { key, direction });
    }
}

impl RenderedLine {
    fn plain(text: impl Into<String>, style: Style) -> Self {
        let text = text.into();
        Self {
            rich_line: Some(Line::from(Span::styled(text.clone(), style))),
            text,
            style,
        }
    }

    fn rich(line: Line<'static>) -> Self {
        let text = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let style = line.style;
        Self {
            text,
            style,
            rich_line: Some(line),
        }
    }

    fn dim(text: impl Into<String>) -> Self {
        Self::plain(text, Style::default().fg(Color::DarkGray))
    }
}

fn message_sort_key(message: &MessageResource) -> (i64, i64) {
    (message.created_at.timestamp_millis(), message.id)
}

fn merge_message_resources(
    current: &MessageResource,
    incoming: &MessageResource,
) -> MessageResource {
    let mut merged = if incoming.updated_at >= current.updated_at {
        incoming.clone()
    } else {
        current.clone()
    };

    let current_parts_score = message_parts_score(current.parts.as_ref());
    let incoming_parts_score = message_parts_score(incoming.parts.as_ref());
    merged.parts = if (incoming.parts.is_none() && current.parts.is_some())
        || current_parts_score > incoming_parts_score
    {
        current.parts.clone()
    } else {
        incoming.parts.clone()
    };

    if message_status_rank(current.state) > message_status_rank(merged.state) {
        merged.state = current.state;
    }
    if current.updated_at > merged.updated_at {
        merged.updated_at = current.updated_at;
    }
    if merged.usage.is_none() {
        merged.usage = current.usage.clone();
    }
    if let Some(parts) = merged.parts.as_mut() {
        parts.sort_by_key(|part| part.part_index);
        merged.part_count = parts.len() as u64;
    } else {
        merged.part_count = merged.part_count.max(current.part_count);
    }

    merged
}

fn message_parts_score(parts: Option<&Vec<MessagePart>>) -> usize {
    parts
        .map(|parts| parts.iter().map(message_part_score).sum())
        .unwrap_or(0)
}

fn message_part_score(part: &MessagePart) -> usize {
    let mut score = 0;
    if part.content.is_some() {
        score += 1;
    }
    if part
        .summary
        .as_deref()
        .is_some_and(|summary| !summary.trim().is_empty())
    {
        score += 4;
    }
    match part.content.as_ref() {
        Some(PartContent::Text(text)) if !text.text.trim().is_empty() => score += 16,
        Some(PartContent::Reasoning(reasoning))
            if !reasoning.summary.is_empty() || !reasoning.raw_content.is_empty() =>
        {
            score += 16;
        }
        Some(PartContent::Operation(operation)) => {
            if operation.output_text().is_some() {
                score += 16;
            } else if operation.title().is_some() || operation.error_message().is_some() {
                score += 8;
            }
        }
        Some(PartContent::Attachment(_))
        | Some(PartContent::Request(_))
        | Some(PartContent::Error(_)) => {
            score += 16;
        }
        _ => {}
    }
    score
}

fn message_status_rank(status: MessageStatus) -> u8 {
    match status {
        MessageStatus::Pending => 0,
        MessageStatus::InProgress => 1,
        MessageStatus::Completed => 2,
        MessageStatus::Failed => 3,
        MessageStatus::Cancelled => 4,
    }
}

fn assistant_message_text(message: &MessageResource) -> Option<String> {
    let parts = message.parts.as_ref()?;
    let text = parts
        .iter()
        .filter_map(|part| match part.content.as_ref()? {
            PartContent::Text(text) if !text.synthetic && !text.ignored => {
                let trimmed = text.text.trim();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

fn pending_interactive_kind_from_request(
    request: &PendingInteractiveRequest,
) -> PendingInteractiveKind {
    match request {
        PendingInteractiveRequest::Permission { .. } => PendingInteractiveKind::Permission,
        PendingInteractiveRequest::UserInput { .. } => PendingInteractiveKind::UserInput,
    }
}

fn pending_interactive_request_id(request: &PendingInteractiveRequest) -> &str {
    request.request_id()
}

fn pending_interactive_request_matches_kind(
    request: &PendingInteractiveRequest,
    kind: PendingInteractiveKind,
) -> bool {
    pending_interactive_kind_from_request(request) == kind
}

fn pending_interactive_request_is_seen(
    request: &PendingInteractiveRequest,
    seen_permission_request_ids: &BTreeSet<String>,
    seen_user_input_request_ids: &BTreeSet<String>,
) -> bool {
    let request_id = pending_interactive_request_id(request);
    match pending_interactive_kind_from_request(request) {
        PendingInteractiveKind::Permission => seen_permission_request_ids.contains(request_id),
        PendingInteractiveKind::UserInput => seen_user_input_request_ids.contains(request_id),
    }
}

fn first_unseen_pending_interactive_request<'a>(
    requests: &'a [PendingInteractiveRequest],
    seen_permission_request_ids: &BTreeSet<String>,
    seen_user_input_request_ids: &BTreeSet<String>,
) -> Option<&'a PendingInteractiveRequest> {
    requests.iter().find(|request| {
        !pending_interactive_request_is_seen(
            request,
            seen_permission_request_ids,
            seen_user_input_request_ids,
        )
    })
}

fn first_pending_interactive_request_by_kind<'a>(
    requests: &'a [PendingInteractiveRequest],
    kind: PendingInteractiveKind,
) -> Option<&'a PendingInteractiveRequest> {
    requests
        .iter()
        .find(|request| pending_interactive_request_matches_kind(request, kind))
}

fn pending_interactive_kind(
    requests: &[PendingInteractiveRequest],
) -> Option<PendingInteractiveKind> {
    requests.first().map(pending_interactive_kind_from_request)
}

fn pending_interactive_kind_for_execution(
    execution: &SessionExecutionResource,
) -> Option<PendingInteractiveKind> {
    pending_interactive_kind(execution.pending_interactive_requests.as_slice())
}

fn execution_update_is_stale(
    current_latest_event_seq: Option<i64>,
    incoming_latest_event_seq: Option<i64>,
) -> bool {
    match (current_latest_event_seq, incoming_latest_event_seq) {
        (Some(current), Some(incoming)) => incoming < current,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

fn permission_overlay_matches_pending_request(
    overlay: &PermissionOverlay,
    session_id: Option<i64>,
    execution: Option<&SessionExecutionResource>,
) -> bool {
    if session_id != Some(overlay.session_id) {
        return false;
    }

    first_pending_interactive_request_by_kind(
        execution
            .map(|resource| resource.pending_interactive_requests.as_slice())
            .unwrap_or(&[]),
        PendingInteractiveKind::Permission,
    )
    .and_then(PendingInteractiveRequest::as_permission)
    .is_some_and(|request| request.request_id == overlay.request.request_id)
}

fn user_input_overlay_matches_pending_request(
    overlay: &UserInputOverlay,
    session_id: Option<i64>,
    execution: Option<&SessionExecutionResource>,
) -> bool {
    if session_id != Some(overlay.session_id) {
        return false;
    }

    first_pending_interactive_request_by_kind(
        execution
            .map(|resource| resource.pending_interactive_requests.as_slice())
            .unwrap_or(&[]),
        PendingInteractiveKind::UserInput,
    )
    .and_then(PendingInteractiveRequest::as_user_input)
    .is_some_and(|request| request.request_id == overlay.request.request_id)
}

fn execution_wait_state_key(execution: &SessionExecutionResource) -> Option<&'static str> {
    match execution.pending_interactive_requests.first() {
        Some(PendingInteractiveRequest::Permission { .. }) => Some("session-awaiting-approval"),
        Some(PendingInteractiveRequest::UserInput { .. }) => Some("session-awaiting-user-input"),
        None if execution.blocked => Some("session-blocked"),
        None => None,
    }
}

fn execution_pending_flash_key(execution: &SessionExecutionResource) -> Option<&'static str> {
    match execution.pending_interactive_requests.first() {
        Some(PendingInteractiveRequest::Permission { .. }) => {
            Some("flash-session-awaiting-approval")
        }
        Some(PendingInteractiveRequest::UserInput { .. }) => {
            Some("flash-session-awaiting-user-input")
        }
        None => None,
    }
}

fn pending_interactive_counts_for_execution(
    execution: &SessionExecutionResource,
) -> (usize, usize) {
    execution.pending_interactive_requests.iter().fold(
        (0, 0),
        |(permission_count, user_input_count), request| match request {
            PendingInteractiveRequest::Permission { .. } => {
                (permission_count + 1, user_input_count)
            }
            PendingInteractiveRequest::UserInput { .. } => (permission_count, user_input_count + 1),
        },
    )
}

fn composer_input_is_active(
    focus: Focus,
    has_text_or_items: bool,
    has_auxiliary_input_ui: bool,
) -> bool {
    focus == Focus::Composer && (has_text_or_items || has_auxiliary_input_ui)
}

fn preferred_visible_session_selection(
    session: &SessionResource,
    visible_sessions: &[SessionResource],
) -> Option<i64> {
    [
        Some(session.id),
        session.parent_id,
        (session.root_id != session.id).then_some(session.root_id),
    ]
    .into_iter()
    .flatten()
    .find(|candidate| visible_sessions.iter().any(|item| item.id == *candidate))
}

fn permission_request_fingerprint(request: &PermissionRequest) -> String {
    json!({
        "action": &request.action,
        "related_actions": &request.related_actions,
        "requested_actions": &request.requested_actions,
        "reason": &request.reason,
        "explanation": &request.explanation,
        "source": &request.source,
        "scope": &request.scope,
        "operator": &request.operator,
        "risk": request.risk,
        "trace": &request.trace,
    })
    .to_string()
}

fn permission_overlay_choice(selected: usize) -> PermissionOverlayChoice {
    match selected {
        0 => PermissionOverlayChoice {
            kind: PermissionReplyKind::AllowOnce,
            scope: None,
        },
        1 => PermissionOverlayChoice {
            kind: PermissionReplyKind::AllowAlways,
            scope: Some(PermissionScope::Session),
        },
        2 => PermissionOverlayChoice {
            kind: PermissionReplyKind::AllowAlways,
            scope: Some(PermissionScope::Workspace),
        },
        3 => PermissionOverlayChoice {
            kind: PermissionReplyKind::AllowAlways,
            scope: Some(PermissionScope::Global),
        },
        4 => PermissionOverlayChoice {
            kind: PermissionReplyKind::DenyOnce,
            scope: None,
        },
        5 => PermissionOverlayChoice {
            kind: PermissionReplyKind::DenyAlways,
            scope: Some(PermissionScope::Session),
        },
        6 => PermissionOverlayChoice {
            kind: PermissionReplyKind::DenyAlways,
            scope: Some(PermissionScope::Workspace),
        },
        _ => PermissionOverlayChoice {
            kind: PermissionReplyKind::DenyAlways,
            scope: Some(PermissionScope::Global),
        },
    }
}

fn permission_overlay_choice_label(i18n: &I18n, choice: PermissionOverlayChoice) -> String {
    match (choice.kind, choice.scope) {
        (PermissionReplyKind::AllowAlways, Some(PermissionScope::Session)) => {
            ui_text::t(i18n, "overlay-permission-choice-allow-always-session")
        }
        (PermissionReplyKind::AllowAlways, Some(PermissionScope::Workspace)) => {
            ui_text::t(i18n, "overlay-permission-choice-allow-always-workspace")
        }
        (PermissionReplyKind::AllowAlways, Some(PermissionScope::Global)) => {
            ui_text::t(i18n, "overlay-permission-choice-allow-always-global")
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Session)) => {
            ui_text::t(i18n, "overlay-permission-choice-deny-always-session")
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Workspace)) => {
            ui_text::t(i18n, "overlay-permission-choice-deny-always-workspace")
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Global)) => {
            ui_text::t(i18n, "overlay-permission-choice-deny-always-global")
        }
        _ => ui_text::permission_reply_label(i18n, choice.kind),
    }
}

fn permission_overlay_choices(i18n: &I18n) -> [String; 8] {
    [
        permission_overlay_choice_label(i18n, permission_overlay_choice(0)),
        permission_overlay_choice_label(i18n, permission_overlay_choice(1)),
        permission_overlay_choice_label(i18n, permission_overlay_choice(2)),
        permission_overlay_choice_label(i18n, permission_overlay_choice(3)),
        permission_overlay_choice_label(i18n, permission_overlay_choice(4)),
        permission_overlay_choice_label(i18n, permission_overlay_choice(5)),
        permission_overlay_choice_label(i18n, permission_overlay_choice(6)),
        permission_overlay_choice_label(i18n, permission_overlay_choice(7)),
    ]
}

fn permission_action_label(i18n: &I18n, action: &PermissionAction) -> String {
    match action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => {
            let base = i18n.text_args(
                "overlay-permission-action-tool",
                &crate::fl_args!("tool" => tool_name.clone()),
            );
            qualifier
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("{base} · {value}"))
                .unwrap_or(base)
        }
        PermissionAction::PathAccess {
            access_kind,
            target_path,
            ..
        } => i18n.text_args(
            "overlay-permission-action-path",
            &crate::fl_args!(
                "access" => access_kind.clone(),
                "path" => target_path.clone(),
            ),
        ),
        PermissionAction::NetworkAccess { host, port, .. } => i18n.text_args(
            "overlay-permission-action-network",
            &crate::fl_args!(
                "target" => match port {
                    Some(port) => format!("{host}:{port}"),
                    None => host.clone(),
                }
            ),
        ),
    }
}

fn permission_requested_actions_for_display<'a>(
    primary: Option<&'a PermissionAction>,
    requested: &'a [PermissionAction],
) -> Vec<&'a PermissionAction> {
    if requested.is_empty() {
        return Vec::new();
    }
    if requested.len() == 1 && primary.is_some_and(|primary| requested.first() == Some(&primary)) {
        return Vec::new();
    }
    requested.iter().collect()
}

fn permission_related_actions_for_display<'a>(
    primary: Option<&'a PermissionAction>,
    related: &'a [PermissionAction],
    requested: &'a [PermissionAction],
) -> Vec<&'a PermissionAction> {
    related
        .iter()
        .filter(|action| {
            !primary.is_some_and(|primary| *action == primary)
                && !requested.iter().any(|candidate| candidate == *action)
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolOutputPreview {
    text: String,
    omitted_lines: usize,
}

fn style_for_role(role: MessageRole) -> Style {
    match role {
        MessageRole::User => Style::default().fg(Color::Green),
        MessageRole::Assistant => Style::default().fg(Color::Cyan),
        MessageRole::System => Style::default().fg(Color::Magenta),
    }
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    DateTime::<Local>::from(timestamp)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn build_timeline_item(i18n: &I18n, record: &DomainEvent) -> TimelineItem {
    let event_type = timeline_event_type_label(i18n, record);
    let summary_suffix = timeline_event_summary(i18n, record);
    let summary = if summary_suffix.is_empty() {
        format!("#{}  {}", record.meta.seq_global, event_type)
    } else {
        format!(
            "#{}  {}  {}",
            record.meta.seq_global, event_type, summary_suffix
        )
    };

    let mut detail_lines = vec![
        timeline_detail_labeled_line(
            i18n,
            "timeline-label-seq",
            record.meta.seq_global.to_string(),
        ),
        timeline_detail_labeled_line(
            i18n,
            "timeline-label-created",
            format_timestamp(record.meta.created_at),
        ),
        timeline_detail_labeled_line(i18n, "timeline-label-type", event_type.clone()),
        timeline_detail_labeled_line(i18n, "timeline-label-event-id", record.meta.id.to_string()),
    ];
    if let Some(causation_id) = record.meta.causation_id {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            "timeline-label-causation-id",
            causation_id.to_string(),
        ));
    }
    if let Some(correlation_id) = record.meta.correlation_id {
        detail_lines.push(timeline_detail_labeled_line(
            i18n,
            "timeline-label-correlation-id",
            correlation_id.to_string(),
        ));
    }
    detail_lines.push(app_detail_plain_line(String::new()));
    detail_lines.extend(timeline_event_detail_lines(i18n, record));

    let detail_document = build_detail_document(
        detail_lines.as_slice(),
        &DetailTextSpec::with_label_width(16),
    );
    let copy_text = format!("{summary}\n\n{}", detail_document.plain);
    let search_text = format!(
        "{} {} {}",
        summary.to_ascii_lowercase(),
        detail_document.plain.to_ascii_lowercase(),
        record.kind.tag_str().to_ascii_lowercase(),
    );
    let linked_message_id = timeline_event_message_id(record);

    TimelineItem {
        summary,
        detail_body: detail_document.text,
        search_text,
        copy_text,
        linked_message_id,
    }
}

fn timeline_event_message_id(record: &DomainEvent) -> Option<i64> {
    match &record.kind {
        AgenaSessionEvent::MessagePartUpdated(event) => Some(event.message_id),
        AgenaSessionEvent::MessagePartDelta(event) => Some(event.message_id),
        AgenaSessionEvent::CommandBegin(event) => event.context.message_id,
        AgenaSessionEvent::CommandOutputDelta(event) => event.context.message_id,
        AgenaSessionEvent::CommandEnd(event) => event.context.message_id,
        AgenaSessionEvent::UserMessageAppended(event) => Some(event.message_id.into()),
        AgenaSessionEvent::AssistantMessageCompleted(event) => Some(event.message_id.into()),
        AgenaSessionEvent::SystemNoticeAppended(event) => Some(event.message_id.into()),
        AgenaSessionEvent::ExecutionStarted(_)
        | AgenaSessionEvent::ExecutionFailed(_)
        | AgenaSessionEvent::StreamError(_)
        | AgenaSessionEvent::PermissionRequested(_)
        | AgenaSessionEvent::PermissionReplied(_)
        | AgenaSessionEvent::PermissionRuleCreated(_)
        | AgenaSessionEvent::PermissionRuleUpdated(_)
        | AgenaSessionEvent::PermissionRuleRevoked(_)
        | AgenaSessionEvent::RunStarted(_)
        | AgenaSessionEvent::RunCompleted(_)
        | AgenaSessionEvent::RunAborted(_)
        | AgenaSessionEvent::ToolCallIssued(_)
        | AgenaSessionEvent::ToolCallCompleted(_)
        | AgenaSessionEvent::PluginEvent(_)
        | AgenaSessionEvent::PluginToolRegistryChanged(_) => None,
    }
}

fn timeline_event_type_key(record: &DomainEvent) -> &'static str {
    match &record.kind {
        AgenaSessionEvent::ExecutionStarted(_) => "timeline-type-execution-started",
        AgenaSessionEvent::ExecutionFailed(_) => "timeline-type-execution-failed",
        AgenaSessionEvent::StreamError(_) => "timeline-type-stream-error",
        AgenaSessionEvent::MessagePartUpdated(_) => "timeline-type-message-part-updated",
        AgenaSessionEvent::MessagePartDelta(_) => "timeline-type-message-part-delta",
        AgenaSessionEvent::CommandBegin(_) => "timeline-type-command-begin",
        AgenaSessionEvent::CommandOutputDelta(_) => "timeline-type-command-output-delta",
        AgenaSessionEvent::CommandEnd(_) => "timeline-type-command-end",
        AgenaSessionEvent::PermissionRequested(_) => "timeline-type-permission-requested",
        AgenaSessionEvent::PermissionReplied(_) => "timeline-type-permission-replied",
        AgenaSessionEvent::PermissionRuleCreated(_) => "timeline-type-permission-rule-created",
        AgenaSessionEvent::PermissionRuleUpdated(_) => "timeline-type-permission-rule-updated",
        AgenaSessionEvent::PermissionRuleRevoked(_) => "timeline-type-permission-rule-revoked",
        AgenaSessionEvent::RunStarted(_) => "timeline-type-run-started",
        AgenaSessionEvent::RunCompleted(_) => "timeline-type-run-completed",
        AgenaSessionEvent::RunAborted(_) => "timeline-type-run-aborted",
        AgenaSessionEvent::UserMessageAppended(_) => "timeline-type-user-message-appended",
        AgenaSessionEvent::AssistantMessageCompleted(_) => {
            "timeline-type-assistant-message-completed"
        }
        AgenaSessionEvent::ToolCallIssued(_) => "timeline-type-tool-call-issued",
        AgenaSessionEvent::ToolCallCompleted(_) => "timeline-type-tool-call-completed",
        AgenaSessionEvent::SystemNoticeAppended(_) => "timeline-type-system-notice-appended",
        AgenaSessionEvent::PluginEvent(_) | AgenaSessionEvent::PluginToolRegistryChanged(_) => {
            "timeline-type-plugin-event"
        }
    }
}

fn timeline_event_type_label(i18n: &I18n, record: &DomainEvent) -> String {
    ui_text::t(i18n, timeline_event_type_key(record))
}

fn timeline_event_summary(i18n: &I18n, record: &DomainEvent) -> String {
    match &record.kind {
        AgenaSessionEvent::ExecutionStarted(event) => i18n.text_args(
            "timeline-summary-execution-started",
            &crate::fl_args!("id" => event.session_id),
        ),
        AgenaSessionEvent::ExecutionFailed(event) => {
            format!(
                "{}: {}",
                event.error.code,
                timeline_excerpt(i18n, event.error.message.as_str(), 72)
            )
        }
        AgenaSessionEvent::MessagePartUpdated(event) => i18n.text_args(
            "timeline-summary-message-part-updated",
            &crate::fl_args!(
                "message_id" => event.message_id,
                "part_id" => event.part.id,
                "kind" => event.part.kind.to_string(),
            ),
        ),
        AgenaSessionEvent::MessagePartDelta(event) => i18n.text_args(
            "timeline-summary-message-part-delta",
            &crate::fl_args!(
                "message_id" => event.message_id,
                "part_id" => event.part_id,
                "field" => timeline_part_delta_field_token(&event.field),
                "count" => event.delta.chars().count() as i64,
            ),
        ),
        AgenaSessionEvent::CommandBegin(event) => {
            timeline_excerpt(i18n, event.command.as_str(), 72)
        }
        AgenaSessionEvent::CommandOutputDelta(event) => {
            let preview = if event.preview_text.trim().is_empty() {
                i18n.text_args(
                    "timeline-summary-command-output-bytes",
                    &crate::fl_args!("count" => event.chunk.len() as i64),
                )
            } else {
                timeline_excerpt(i18n, event.preview_text.as_str(), 56)
            };
            i18n.text_args(
                "timeline-summary-command-output-delta",
                &crate::fl_args!(
                    "stream" => timeline_command_output_stream_token(event.stream.clone()),
                    "preview" => preview,
                ),
            )
        }
        AgenaSessionEvent::CommandEnd(event) => i18n.text_args(
            "timeline-summary-command-end",
            &crate::fl_args!(
                "status" => ui_text::execution_status_label(i18n, event.status),
                "exit_code" => event.exit_code,
                "duration_ms" => event.duration_ms as i64,
            ),
        ),
        AgenaSessionEvent::StreamError(event) => {
            format!(
                "{}: {}",
                event.error.code,
                timeline_excerpt(i18n, event.error.message.as_str(), 72)
            )
        }
        AgenaSessionEvent::PermissionRequested(event) => i18n.text_args(
            "timeline-summary-permission-requested",
            &crate::fl_args!(
                "risk" => permission_risk_label(i18n, event.risk),
                "reason" => timeline_excerpt(i18n, event.reason.as_str(), 72),
            ),
        ),
        AgenaSessionEvent::PermissionReplied(event) => i18n.text_args(
            "timeline-summary-permission-replied",
            &crate::fl_args!("kind" => ui_text::permission_reply_label(i18n, event.kind)),
        ),
        AgenaSessionEvent::PermissionRuleCreated(event) => i18n.text_args(
            "timeline-summary-permission-rule-created",
            &crate::fl_args!("id" => event.rule_id),
        ),
        AgenaSessionEvent::PermissionRuleUpdated(event) => i18n.text_args(
            "timeline-summary-permission-rule-updated",
            &crate::fl_args!("id" => event.rule_id),
        ),
        AgenaSessionEvent::PermissionRuleRevoked(event) => i18n.text_args(
            "timeline-summary-permission-rule-revoked",
            &crate::fl_args!("id" => event.rule_id),
        ),
        AgenaSessionEvent::RunStarted(p) => i18n.text_args(
            "timeline-summary-run-started",
            &crate::fl_args!("id" => p.run_id),
        ),
        AgenaSessionEvent::RunCompleted(p) => i18n.text_args(
            "timeline-summary-run-completed",
            &crate::fl_args!(
                "id" => p.run_id,
                "finish" => p.finish_reason.to_string(),
            ),
        ),
        AgenaSessionEvent::RunAborted(p) => i18n.text_args(
            "timeline-summary-run-aborted",
            &crate::fl_args!(
                "id" => p.run_id,
                "reason" => p.reason.to_string(),
            ),
        ),
        AgenaSessionEvent::UserMessageAppended(p) => i18n.text_args(
            "timeline-summary-user-message-appended",
            &crate::fl_args!("id" => p.message_id),
        ),
        AgenaSessionEvent::AssistantMessageCompleted(p) => i18n.text_args(
            "timeline-summary-assistant-message-completed",
            &crate::fl_args!(
                "id" => p.message_id,
                "finish" => p.finish_reason.to_string(),
            ),
        ),
        AgenaSessionEvent::ToolCallIssued(p) => i18n.text_args(
            "timeline-summary-tool-call-issued",
            &crate::fl_args!(
                "name" => p.name.as_str(),
                "call_id" => p.call_id,
            ),
        ),
        AgenaSessionEvent::ToolCallCompleted(p) => i18n.text_args(
            "timeline-summary-tool-call-completed",
            &crate::fl_args!("call_id" => p.call_id),
        ),
        AgenaSessionEvent::SystemNoticeAppended(p) => i18n.text_args(
            "timeline-summary-system-notice-appended",
            &crate::fl_args!(
                "message_id" => p.message_id,
                "kind" => p.kind.to_string(),
            ),
        ),
        AgenaSessionEvent::PluginEvent(p) => i18n.text_args(
            "timeline-summary-plugin-event",
            &crate::fl_args!(
                "plugin_id" => p.plugin_id.clone(),
                "kind_label" => p.kind_label.clone(),
            ),
        ),
        AgenaSessionEvent::PluginToolRegistryChanged(event) => format!(
            "{} {} {}",
            event.plugin_id,
            match event.kind {
                agena::plugin::sdk::host_api::ToolRegistryChangeKind::Registered => "registered",
                agena::plugin::sdk::host_api::ToolRegistryChangeKind::Updated => "updated",
                agena::plugin::sdk::host_api::ToolRegistryChangeKind::Removed => "removed",
            },
            event.exposed_name
        ),
    }
}

fn timeline_event_detail_lines(i18n: &I18n, record: &DomainEvent) -> Vec<DetailTextLine<'static>> {
    match &record.kind {
        AgenaSessionEvent::ExecutionStarted(event) => vec![timeline_detail_labeled_line(
            i18n,
            "timeline-label-session-id",
            event.session_id.to_string(),
        )],
        AgenaSessionEvent::ExecutionFailed(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-error-code",
                event.error.code.clone(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-error-message",
                event.error.message.clone(),
            ),
        ],
        AgenaSessionEvent::MessagePartUpdated(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                event.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-part-id", event.part.id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-part-kind",
                event.part.kind.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-status",
                ui_text::execution_status_label(i18n, event.part.status),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-summary",
                event
                    .part
                    .summary
                    .clone()
                    .unwrap_or_else(|| ui_text::t(i18n, "value-none")),
            ),
        ],
        AgenaSessionEvent::MessagePartDelta(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                event.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-part-id", event.part_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-field",
                timeline_part_delta_field_token(&event.field),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-seq", event.seq.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-delta",
                timeline_excerpt(i18n, event.delta.as_str(), 200),
            ),
        ],
        AgenaSessionEvent::CommandBegin(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.context.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-call-id",
                event.context.call_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-command", event.command.clone()),
            timeline_detail_labeled_line(i18n, "timeline-label-cwd", event.cwd.clone()),
        ],
        AgenaSessionEvent::CommandOutputDelta(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.context.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-call-id",
                event.context.call_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-stream",
                timeline_command_output_stream_token(event.stream.clone()),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-seq", event.seq.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-bytes",
                event.chunk.len().to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-preview",
                timeline_excerpt(i18n, event.preview_text.as_str(), 200),
            ),
        ],
        AgenaSessionEvent::CommandEnd(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.context.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-call-id",
                event.context.call_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-status",
                ui_text::execution_status_label(i18n, event.status),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-exit-code",
                event.exit_code.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-duration-ms",
                event.duration_ms.to_string(),
            ),
        ],
        AgenaSessionEvent::StreamError(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-error-code",
                event.error.code.clone(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-error-message",
                event.error.message.clone(),
            ),
        ],
        AgenaSessionEvent::PermissionRequested(event) => {
            let mut lines = vec![
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-session-id",
                    event.session_id.to_string(),
                ),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-request-id",
                    event.request_id.clone(),
                ),
                app_detail_plain_line(permission_action_label(i18n, &event.action)),
                timeline_detail_labeled_line(i18n, "timeline-label-reason", event.reason.clone()),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-risk",
                    permission_risk_label(i18n, event.risk),
                ),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-explanation",
                    timeline_excerpt(i18n, event.explanation.as_str(), 200),
                ),
            ];
            if let Some(source) = event.source.as_deref() {
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-source",
                    source.to_string(),
                ));
            }
            if let Some(scope) = event.scope.as_deref() {
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-scope",
                    scope.to_string(),
                ));
            }
            if let Some(operator) = event.operator.as_deref() {
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-operator",
                    operator.to_string(),
                ));
            }
            append_permission_action_detail_lines(
                i18n,
                &mut lines,
                "timeline-label-requested-actions",
                permission_requested_actions_for_display(
                    Some(&event.action),
                    event.requested_actions.as_slice(),
                )
                .as_slice(),
            );
            append_permission_action_detail_lines(
                i18n,
                &mut lines,
                "timeline-label-related-actions",
                permission_related_actions_for_display(
                    Some(&event.action),
                    event.related_actions.as_slice(),
                    event.requested_actions.as_slice(),
                )
                .as_slice(),
            );
            append_permission_trace_detail_lines(i18n, &mut lines, &event.trace);
            lines
        }
        AgenaSessionEvent::PermissionReplied(event) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-session-id",
                event.session_id.to_string(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-request-id",
                event.request_id.clone(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-reply-kind",
                ui_text::permission_reply_label(i18n, event.kind),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-reason",
                timeline_value_or_none(i18n, event.reason.clone()),
            ),
        ],
        AgenaSessionEvent::PermissionRuleCreated(event)
        | AgenaSessionEvent::PermissionRuleUpdated(event)
        | AgenaSessionEvent::PermissionRuleRevoked(event) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-rule-id", event.rule_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-action-key",
                event.action_key.clone(),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-mode",
                permission_mode_token_display(i18n, event.mode.as_str()),
            ),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-scope",
                permission_rule_scope_display(i18n, event.scope.as_str()),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-source", event.source.clone()),
        ],
        AgenaSessionEvent::RunStarted(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-model",
                format!("{}/{}", p.provider_id, p.model_id),
            ),
        ],
        AgenaSessionEvent::RunCompleted(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-finish",
                p.finish_reason.to_string(),
            ),
        ],
        AgenaSessionEvent::RunAborted(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-reason", p.reason.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message",
                timeline_value_or_none(i18n, p.message.clone()),
            ),
        ],
        AgenaSessionEvent::UserMessageAppended(p) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                p.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
        ],
        AgenaSessionEvent::AssistantMessageCompleted(p) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                p.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-finish",
                p.finish_reason.to_string(),
            ),
        ],
        AgenaSessionEvent::ToolCallIssued(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-call-id", p.call_id.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-name", p.name.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
        ],
        AgenaSessionEvent::ToolCallCompleted(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-call-id", p.call_id.to_string()),
            timeline_detail_labeled_line(i18n, "timeline-label-run-id", p.run_id.to_string()),
        ],
        AgenaSessionEvent::SystemNoticeAppended(p) => vec![
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-message-id",
                p.message_id.to_string(),
            ),
            timeline_detail_labeled_line(i18n, "timeline-label-kind", p.kind.to_string()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-text",
                timeline_excerpt(i18n, p.text.as_str(), 200),
            ),
        ],
        AgenaSessionEvent::PluginEvent(p) => vec![
            timeline_detail_labeled_line(i18n, "timeline-label-plugin-id", p.plugin_id.clone()),
            timeline_detail_labeled_line(i18n, "timeline-label-kind-label", p.kind_label.clone()),
            timeline_detail_labeled_line(
                i18n,
                "timeline-label-payload",
                timeline_excerpt(i18n, &p.payload.to_string(), 200),
            ),
        ],
        AgenaSessionEvent::PluginToolRegistryChanged(event) => {
            let mut lines = vec![
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-plugin-id",
                    event.plugin_id.clone(),
                ),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-kind",
                    match event.kind {
                        agena::plugin::sdk::host_api::ToolRegistryChangeKind::Registered => {
                            "registered"
                        }
                        agena::plugin::sdk::host_api::ToolRegistryChangeKind::Updated => "updated",
                        agena::plugin::sdk::host_api::ToolRegistryChangeKind::Removed => "removed",
                    },
                ),
                timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-name",
                    event.original_name.clone(),
                ),
                app_detail_plain_line(format!("exposed_name: {}", event.exposed_name)),
                app_detail_plain_line(format!("generation: {}", event.generation)),
                app_detail_plain_line(format!("timestamp_ms: {}", event.timestamp_ms)),
            ];
            if let Some(tool) = &event.tool {
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-summary",
                    tool.description
                        .clone()
                        .unwrap_or_else(|| tool.name.clone()),
                ));
                lines.push(timeline_detail_labeled_line(
                    i18n,
                    "timeline-label-payload",
                    timeline_excerpt(i18n, &tool.input_schema.to_string(), 200),
                ));
            }
            lines
        }
    }
}

fn timeline_detail_labeled_line(
    i18n: &I18n,
    label_key: &str,
    value: impl Into<String>,
) -> DetailTextLine<'static> {
    app_detail_labeled_line(ui_text::t(i18n, label_key), value.into())
}

fn timeline_excerpt(i18n: &I18n, text: &str, max_chars: usize) -> String {
    if text.trim().is_empty() {
        ui_text::t(i18n, "value-none")
    } else {
        detail_excerpt(text, max_chars)
    }
}

fn timeline_value_or_none<T: ToString>(i18n: &I18n, value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| ui_text::t(i18n, "value-none"))
}

fn timeline_part_delta_field_token(field: &agena::event::PartDeltaField) -> String {
    match field {
        agena::event::PartDeltaField::Text => "text".to_string(),
        agena::event::PartDeltaField::ReasoningSummary => "reasoning_summary".to_string(),
        agena::event::PartDeltaField::ReasoningRawContent => "reasoning_raw_content".to_string(),
        agena::event::PartDeltaField::CommandStdout => "command_stdout".to_string(),
        agena::event::PartDeltaField::CommandStderr => "command_stderr".to_string(),
        agena::event::PartDeltaField::ToolOutputText => "tool_output_text".to_string(),
        agena::event::PartDeltaField::Custom { name } => format!("custom/{name}"),
    }
}

fn timeline_command_output_stream_token(stream: agena::event::CommandOutputStream) -> &'static str {
    match stream {
        agena::event::CommandOutputStream::Stdout => "stdout",
        agena::event::CommandOutputStream::Stderr => "stderr",
    }
}

fn append_permission_action_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    label_key: &str,
    actions: &[&PermissionAction],
) {
    if actions.is_empty() {
        return;
    }
    lines.push(app_detail_plain_line(format!(
        "{}:",
        ui_text::t(i18n, label_key)
    )));
    lines.extend(actions.iter().map(|action| {
        app_detail_plain_line(format!("  {}", permission_action_label(i18n, action)))
    }));
}

fn append_permission_trace_detail_lines(
    i18n: &I18n,
    lines: &mut Vec<DetailTextLine<'static>>,
    trace: &[DecisionTraceStep],
) {
    if trace.is_empty() {
        return;
    }
    lines.push(app_detail_plain_line(format!(
        "{}:",
        ui_text::t(i18n, "timeline-label-trace")
    )));
    lines.extend(trace.iter().map(|step| {
        app_detail_plain_line(format!("  {}", permission_trace_step_label(i18n, step)))
    }));
}

fn permission_risk_label(i18n: &I18n, risk: PermissionRiskLevel) -> String {
    ui_text::t(
        i18n,
        match risk {
            PermissionRiskLevel::Low => "value-risk-low",
            PermissionRiskLevel::Medium => "value-risk-medium",
            PermissionRiskLevel::High => "value-risk-high",
            PermissionRiskLevel::Critical => "value-risk-critical",
        },
    )
}

fn permission_trace_step_label(i18n: &I18n, step: &DecisionTraceStep) -> String {
    let source_kind = match step.source_kind {
        PolicySourceKind::StaticPolicy => "static_policy",
        PolicySourceKind::PersistedRule => "persisted_rule",
        PolicySourceKind::PluginAdvice => "plugin_advice",
        PolicySourceKind::ManagedPolicy => "managed_policy",
    };
    let mut facts = vec![source_kind.to_string()];
    if let Some(source) = step.source.as_deref() {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-source").as_str(),
            source,
        ));
    }
    if let Some(scope) = step.scope {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-scope").as_str(),
            permission_scope_label(i18n, scope).as_str(),
        ));
    }
    if let Some(operator) = step.operator.as_deref() {
        facts.push(format_key_value_segment(
            ui_text::t(i18n, "inline-fact-operator").as_str(),
            operator,
        ));
    }
    format!("- {} — {}", join_inline_segments(facts), step.summary)
}

fn append_permission_trace_lines(
    i18n: &I18n,
    lines: &mut Vec<Line<'static>>,
    trace: &[DecisionTraceStep],
) {
    if trace.is_empty() {
        return;
    }
    lines.push(Line::from("Trace:"));
    lines.extend(
        trace
            .iter()
            .map(|step| Line::from(permission_trace_step_label(i18n, step))),
    );
}

fn permission_scope_label(i18n: &I18n, scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => ui_text::t(i18n, "value-session"),
        PermissionScope::Workspace => ui_text::t(i18n, "value-workspace"),
        PermissionScope::Global => ui_text::t(i18n, "value-global"),
    }
}

fn detail_excerpt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "(empty)".to_string();
    }
    let compact = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = compact.chars().take(max_chars).collect::<String>();
    if compact.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn build_visible_session_items(
    items: &[SessionResource],
    mode: SessionViewMode,
    query: &str,
) -> Vec<SessionResource> {
    let trimmed_query = query.trim();
    match mode {
        SessionViewMode::Roots => {
            let mut roots = items
                .iter()
                .filter(|session| session.parent_id.is_none())
                .cloned()
                .collect::<Vec<_>>();
            roots.sort_by(session_sort_recent);
            if trimmed_query.is_empty() {
                roots
            } else {
                roots
                    .into_iter()
                    .filter(|session| session_matches_query(session, trimmed_query))
                    .collect()
            }
        }
        SessionViewMode::All | SessionViewMode::Subtree => {
            let by_id = items
                .iter()
                .cloned()
                .map(|session| (session.id, session))
                .collect::<BTreeMap<_, _>>();
            let mut children = BTreeMap::<Option<i64>, Vec<i64>>::new();
            for session in items {
                let parent_id = session
                    .parent_id
                    .filter(|parent_id| by_id.contains_key(parent_id));
                children.entry(parent_id).or_default().push(session.id);
            }
            for child_ids in children.values_mut() {
                child_ids.sort_by(|left, right| session_sort_recent(&by_id[left], &by_id[right]));
            }

            let kept_ids = if trimmed_query.is_empty() {
                by_id.keys().copied().collect::<HashSet<_>>()
            } else {
                let mut kept = HashSet::new();
                for session in items
                    .iter()
                    .filter(|session| session_matches_query(session, trimmed_query))
                {
                    let mut current = Some(session.id);
                    while let Some(id) = current {
                        if !kept.insert(id) {
                            break;
                        }
                        current = by_id.get(&id).and_then(|item| item.parent_id);
                    }
                }
                kept
            };

            let root_ids = children.get(&None).cloned().unwrap_or_default();
            let mut out = Vec::new();
            for root_id in root_ids {
                append_session_subtree(root_id, &children, &by_id, &kept_ids, &mut out);
            }
            out
        }
    }
}

fn append_session_subtree(
    session_id: i64,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
    by_id: &BTreeMap<i64, SessionResource>,
    kept_ids: &HashSet<i64>,
    out: &mut Vec<SessionResource>,
) {
    if !kept_ids.contains(&session_id) {
        return;
    }
    if let Some(session) = by_id.get(&session_id) {
        out.push(session.clone());
    }
    if let Some(child_ids) = children.get(&Some(session_id)) {
        for child_id in child_ids {
            append_session_subtree(*child_id, children, by_id, kept_ids, out);
        }
    }
}

fn lineage_relation_tag_key(relation: LineageRelation) -> &'static str {
    match relation {
        LineageRelation::Ancestor => "session-tag-ancestor",
        LineageRelation::Current => "session-tag-current",
        LineageRelation::Sibling => "session-tag-sibling",
        LineageRelation::Child => "session-tag-child",
    }
}

fn build_lineage_session_items(
    items: &[SessionResource],
    current_session_id: i64,
) -> Vec<LineageSessionItem> {
    let by_id = items
        .iter()
        .cloned()
        .map(|session| (session.id, session))
        .collect::<BTreeMap<_, _>>();
    if !by_id.contains_key(&current_session_id) {
        return Vec::new();
    }

    let lineage_chain = session_lineage_chain(current_session_id, &by_id);
    let lineage_ids = lineage_chain.iter().copied().collect::<HashSet<_>>();

    let mut children = BTreeMap::<Option<i64>, Vec<i64>>::new();
    for session in items {
        let parent_id = session
            .parent_id
            .filter(|parent_id| by_id.contains_key(parent_id));
        children.entry(parent_id).or_default().push(session.id);
    }
    for child_ids in children.values_mut() {
        child_ids.sort_by(|left, right| {
            let left_on_path = lineage_ids.contains(left);
            let right_on_path = lineage_ids.contains(right);
            right_on_path
                .cmp(&left_on_path)
                .then_with(|| session_sort_recent(&by_id[left], &by_id[right]))
        });
    }

    let Some(root_id) = lineage_chain.first().copied() else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut visited = HashSet::new();
    append_lineage_items(
        root_id,
        0,
        false,
        current_session_id,
        &lineage_ids,
        &children,
        &by_id,
        &mut visited,
        &mut out,
    );
    out
}

fn summarize_lineage_session_items(items: &[LineageSessionItem]) -> Option<SessionLineageSummary> {
    let root_id = items.first()?.session.id;
    let current = items
        .iter()
        .find(|item| item.relation == LineageRelation::Current)?;
    Some(SessionLineageSummary {
        root_id,
        depth: current.depth,
        side_branch_count: items
            .iter()
            .filter(|item| item.relation == LineageRelation::Sibling)
            .count(),
        descendant_count: items
            .iter()
            .filter(|item| item.relation == LineageRelation::Child)
            .count(),
    })
}

fn lineage_path_segments(
    items: &[SessionResource],
    current_session_id: i64,
) -> Vec<SessionPathSegment> {
    let by_id = items
        .iter()
        .cloned()
        .map(|session| (session.id, session))
        .collect::<BTreeMap<_, _>>();

    session_lineage_chain(current_session_id, &by_id)
        .into_iter()
        .map(|id| SessionPathSegment { id })
        .collect()
}

fn model_status_label(model: &ModelRef) -> String {
    model
        .adapter_id
        .as_ref()
        .map(|adapter_id| format!("{}/{}/{}", model.provider_id, adapter_id, model.model_id))
        .unwrap_or_else(|| format!("{}/{}", model.provider_id, model.model_id))
}

fn execution_model_status_label(execution: &SessionExecutionContextResource) -> Option<String> {
    let provider_id = execution
        .model_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let adapter_id = execution
        .model_adapter_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model_id = execution
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if provider_id.is_none() && model_id.is_none() {
        return None;
    }

    Some(match adapter_id {
        Some(adapter_id) => format!(
            "{}/{}/{}",
            provider_id.unwrap_or("auto"),
            adapter_id,
            model_id.unwrap_or("default")
        ),
        None => format!(
            "{}/{}",
            provider_id.unwrap_or("auto"),
            model_id.unwrap_or("default")
        ),
    })
}

fn session_summary_status_parts(
    model_part: Option<String>,
    agent: Option<String>,
    token_usage: Option<TokenUsageStatus>,
) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(model_part) = model_part
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(model_part);
    }
    if let Some(agent) = agent
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        parts.push(agent);
    }
    if let Some(token_usage) = token_usage {
        parts.push(token_usage.label());
    }
    parts
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenUsageStatus {
    PercentUsed(u64),
    UsedTokens(u64),
}

impl TokenUsageStatus {
    fn label(self) -> String {
        match self {
            Self::PercentUsed(percent_used) => format_token_progress_label(percent_used),
            Self::UsedTokens(tokens) => format!("{} used", format_tokens_k(tokens)),
        }
    }
}

fn status_line_token_usage(usage: &SessionUsageResource) -> Option<TokenUsageStatus> {
    let current_tokens = usage.projected_tokens.unwrap_or(usage.current_tokens);
    if let Some(context_window_tokens) = usage.model_context_window_tokens {
        return Some(TokenUsageStatus::PercentUsed(
            agena::session::context_usage_percent_used(current_tokens, context_window_tokens),
        ));
    }

    Some(TokenUsageStatus::UsedTokens(current_tokens))
}

fn format_token_progress_label(percent_used: u64) -> String {
    format!("{}%", percent_used.min(100))
}

fn format_tokens_k(tokens: u64) -> String {
    if tokens == 0 {
        return "0k".to_string();
    }

    let value = tokens as f64 / 1_000.0;
    if value < 10.0 {
        return format!("{value:.1}k");
    }
    format!("{value:.0}k")
}

fn session_lineage_chain(
    current_session_id: i64,
    by_id: &BTreeMap<i64, SessionResource>,
) -> Vec<i64> {
    let mut chain = Vec::new();
    let mut current = Some(current_session_id);
    let mut seen = HashSet::new();

    while let Some(session_id) = current {
        if !seen.insert(session_id) {
            break;
        }
        let Some(session) = by_id.get(&session_id) else {
            break;
        };
        chain.push(session_id);
        current = session
            .parent_id
            .filter(|parent_id| by_id.contains_key(parent_id));
    }

    chain.reverse();
    chain
}

fn is_rewind_target_message(message: &MessageResource) -> bool {
    matches!(message.role, MessageRole::User | MessageRole::Assistant)
        && message.state == MessageStatus::Completed
}

#[allow(clippy::too_many_arguments)]
fn append_lineage_items(
    session_id: i64,
    depth: usize,
    under_current_branch: bool,
    current_session_id: i64,
    lineage_ids: &HashSet<i64>,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
    by_id: &BTreeMap<i64, SessionResource>,
    visited: &mut HashSet<i64>,
    out: &mut Vec<LineageSessionItem>,
) {
    if !visited.insert(session_id) {
        return;
    }
    let Some(session) = by_id.get(&session_id).cloned() else {
        return;
    };

    let child_ids = children.get(&Some(session_id)).cloned().unwrap_or_default();
    let relation = if session_id == current_session_id {
        LineageRelation::Current
    } else if lineage_ids.contains(&session_id) {
        LineageRelation::Ancestor
    } else if under_current_branch {
        LineageRelation::Child
    } else {
        LineageRelation::Sibling
    };

    out.push(LineageSessionItem {
        session,
        relation,
        depth,
        is_leaf: child_ids.is_empty(),
    });

    let next_under_current_branch = under_current_branch || session_id == current_session_id;
    for child_id in child_ids {
        append_lineage_items(
            child_id,
            depth.saturating_add(1),
            next_under_current_branch,
            current_session_id,
            lineage_ids,
            children,
            by_id,
            visited,
            out,
        );
    }
}

fn session_matches_query(session: &SessionResource, query: &str) -> bool {
    let query = query.to_ascii_lowercase();
    session.title.to_ascii_lowercase().contains(query.as_str())
        || session.id.to_string().contains(query.as_str())
}

fn session_sort_recent(left: &SessionResource, right: &SessionResource) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| right.id.cmp(&left.id))
}

fn derive_session_title(i18n: &I18n, text: &str) -> String {
    let fallback = ui_text::t(i18n, "composer-session-new");
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback.as_str());
    truncate_display_width(first_line, 60)
}

fn draft_title_source(draft: &ComposerDraft) -> Option<String> {
    let mut labels = draft
        .items
        .iter()
        .map(|item| {
            (
                item.placeholder().to_string(),
                item.short_label().to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut elements = draft.elements.clone();
    elements.sort_by_key(|element| element.range.start);

    let mut preview = String::new();
    let mut cursor = 0;
    for element in elements {
        let start = min(element.range.start, draft.text.len());
        let end = min(element.range.end, draft.text.len());
        if cursor < start {
            preview.push_str(&draft.text[cursor..start]);
        }
        if let Some(label) = labels.remove(element.placeholder.as_str()) {
            preview.push_str(label.as_str());
        }
        cursor = end;
    }
    if cursor < draft.text.len() {
        preview.push_str(&draft.text[cursor..]);
    }

    if preview.trim().is_empty() {
        draft
            .items
            .first()
            .map(ComposerItem::short_label)
            .map(str::to_owned)
    } else {
        Some(preview)
    }
}

fn truncate_display_width(text: &str, max_width: usize) -> String {
    let text = sanitize_terminal_text(text);
    let mut width = 0_usize;
    let mut out = String::new();
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width.saturating_add(ch_width) > max_width {
            break;
        }
        out.push(ch);
        width = width.saturating_add(ch_width);
    }
    if out.is_empty() {
        text.chars().take(max_width).collect()
    } else {
        out
    }
}

fn user_input_answer_values(
    question: &UserInputQuestion,
    draft: &UserInputAnswerDraft,
) -> Vec<String> {
    let mut values = draft
        .option_indexes
        .iter()
        .filter_map(|index| question.options.get(*index))
        .map(|option| option.label.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.extend(
        draft
            .custom_values
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    );
    if question.multiple {
        values
    } else {
        values.into_iter().take(1).collect()
    }
}

fn user_input_question_label(question: &UserInputQuestion) -> &str {
    let header = question.header.trim();
    if !header.is_empty() {
        header
    } else if !question.question.trim().is_empty() {
        question.question.trim()
    } else {
        question.id.as_str()
    }
}

fn contains_case_insensitive(text: &str, query: &str) -> bool {
    let trimmed = query.trim();
    !trimmed.is_empty()
        && text
            .to_lowercase()
            .contains(trimmed.to_lowercase().as_str())
}

fn find_search_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.trim();
    if query.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut search_start = 0;
    while search_start < text.len() {
        let Some((start, end)) = find_query_match_from(text, query, search_start) else {
            break;
        };
        ranges.push(start..end);
        search_start = if end > start {
            end
        } else {
            next_grapheme_boundary(text, start)
        };
    }
    ranges
}

fn find_query_match_from(text: &str, query: &str, start_at: usize) -> Option<(usize, usize)> {
    if query.is_ascii() {
        let mut starts = text[start_at..]
            .char_indices()
            .map(|(offset, _)| start_at + offset)
            .collect::<Vec<_>>();
        if !starts.contains(&text.len()) {
            starts.push(text.len());
        }
        for start in starts {
            let end = start.saturating_add(query.len());
            if let Some(slice) = text.get(start..end)
                && slice.eq_ignore_ascii_case(query)
            {
                return Some((start, end));
            }
        }
        None
    } else {
        text[start_at..]
            .find(query)
            .map(|offset| start_at + offset)
            .map(|start| (start, start + query.len()))
    }
}

fn run_status_line_command(
    command: String,
    session_id: Option<String>,
    focus: String,
) -> Option<String> {
    let mut cmd = if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/d", "/s", "/c", command.as_str()]);
        cmd
    } else {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-lc", command.as_str()]);
        cmd
    };
    cmd.stdin(Stdio::null()).stderr(Stdio::null());
    cmd.env("AGENA_TUI_FOCUS", focus);
    if let Some(session_id) = session_id {
        cmd.env("AGENA_SESSION_ID", session_id);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next().unwrap_or_default().trim();
    (!line.is_empty()).then(|| line.to_string())
}

fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let grapheme = text[index..].graphemes(true).next().unwrap_or_default();
    index + grapheme.len()
}

fn attachment_chip_label(
    i18n: &I18n,
    path: &Path,
    kind: AttachmentKind,
    width: Option<u32>,
    height: Option<u32>,
    size_bytes: u64,
) -> String {
    ui_text::attachment_chip_label(i18n, path, kind, width, height, size_bytes)
}

fn cleanup_temporary_composer_items(items: &[ComposerItem]) {
    for item in items {
        cleanup_temporary_composer_item(item);
    }
}

fn cleanup_temporary_composer_item(item: &ComposerItem) {
    if let ComposerItem::Attachment(attachment) = item
        && attachment.is_temp
    {
        let _ = std::fs::remove_file(&attachment.path);
    }
}

fn push_submission_text(parts: &mut Vec<PartContent>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = parts.last_mut()
        && last.append_text_delta(text)
    {
        return;
    }
    parts.push(PartContent::text(text.to_string()));
}

fn attachment_placeholder_base(i18n: &I18n, path: &Path, kind: AttachmentKind) -> String {
    ui_text::attachment_placeholder_base(i18n, path, kind)
}

fn find_placeholder_occurrence(
    text: &str,
    placeholder: &str,
    occupied: &[Range<usize>],
) -> Option<Range<usize>> {
    if placeholder.is_empty() {
        return None;
    }

    let mut search_start = 0;
    while search_start < text.len() {
        let relative = text.get(search_start..)?.find(placeholder)?;
        let start = search_start + relative;
        let end = start + placeholder.len();
        let candidate = start..end;
        if occupied
            .iter()
            .all(|range| range.end <= candidate.start || range.start >= candidate.end)
        {
            return Some(candidate);
        }
        search_start = next_grapheme_boundary(text, start);
    }
    None
}

impl RunOptionsState {
    fn clear_model_stack(&mut self) {
        self.model = None;
        self.thinking_mode = None;
        self.speed_mode = None;
        self.verbosity = None;
        self.parallel_tool_calls = None;
    }

    fn runtime_setting_summary(&self, i18n: &I18n, field: RuntimeSettingSpec) -> String {
        match field.id {
            RuntimeSettingId::ThinkingMode => self
                .thinking_mode
                .as_deref()
                .map(|value| {
                    runtime_setting_override_summary(
                        i18n,
                        ui_text::thinking_mode_display_value(value).as_str(),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::SpeedMode => self
                .speed_mode
                .as_deref()
                .map(|value| {
                    runtime_setting_override_summary(
                        i18n,
                        ui_text::speed_mode_display_value(value).as_str(),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::Verbosity => self
                .verbosity
                .as_deref()
                .map(|value| runtime_setting_override_summary(i18n, value))
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::ParallelToolCalls => self
                .parallel_tool_calls
                .map(|value| {
                    runtime_setting_override_summary(
                        i18n,
                        ui_text::t(i18n, if value { "value-on" } else { "value-off" }).as_str(),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::Temperature => self
                .temperature
                .map(|value| runtime_setting_override_summary(i18n, format!("{value:.2}").as_str()))
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::MaxOutput => self
                .max_output_tokens
                .map(|value| runtime_setting_override_summary(i18n, value.to_string().as_str()))
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
            RuntimeSettingId::System => self
                .system
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    runtime_setting_override_summary(
                        i18n,
                        format_setting_value_inline(&JsonValue::String(value.clone())).as_str(),
                    )
                })
                .unwrap_or_else(|| ui_text::t(i18n, "value-default")),
        }
    }

    fn runtime_setting_input_text(&self, field: RuntimeSettingSpec) -> String {
        match field.id {
            RuntimeSettingId::ThinkingMode => self.thinking_mode.clone().unwrap_or_default(),
            RuntimeSettingId::SpeedMode => self.speed_mode.clone().unwrap_or_default(),
            RuntimeSettingId::Verbosity => self.verbosity.clone().unwrap_or_default(),
            RuntimeSettingId::ParallelToolCalls => self
                .parallel_tool_calls
                .map(|value| {
                    if value {
                        "on".to_string()
                    } else {
                        "off".to_string()
                    }
                })
                .unwrap_or_default(),
            RuntimeSettingId::Temperature => self
                .temperature
                .map(|value| format!("{value:.2}"))
                .unwrap_or_default(),
            RuntimeSettingId::MaxOutput => self
                .max_output_tokens
                .map(|value| value.to_string())
                .unwrap_or_default(),
            RuntimeSettingId::System => self.system.clone().unwrap_or_default(),
        }
    }

    fn apply_runtime_setting_input(
        &mut self,
        i18n: &I18n,
        field: RuntimeSettingSpec,
        input: &str,
    ) -> std::result::Result<String, String> {
        let trimmed = input.trim();
        let field_label = runtime_setting_display_label(i18n, field);
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("clear") {
            match field.id {
                RuntimeSettingId::ThinkingMode => self.thinking_mode = None,
                RuntimeSettingId::SpeedMode => self.speed_mode = None,
                RuntimeSettingId::Verbosity => self.verbosity = None,
                RuntimeSettingId::ParallelToolCalls => self.parallel_tool_calls = None,
                RuntimeSettingId::Temperature => self.temperature = None,
                RuntimeSettingId::MaxOutput => self.max_output_tokens = None,
                RuntimeSettingId::System => self.system = None,
            }
            return Ok(i18n.text_args(
                "runtime-setting-apply-cleared",
                &crate::fl_args!("field" => field_label),
            ));
        }

        match field.id {
            RuntimeSettingId::ThinkingMode => self.thinking_mode = Some(trimmed.to_string()),
            RuntimeSettingId::SpeedMode => self.speed_mode = Some(trimmed.to_string()),
            RuntimeSettingId::Verbosity => self.verbosity = Some(trimmed.to_ascii_lowercase()),
            RuntimeSettingId::ParallelToolCalls => {
                let value = match trimmed.to_ascii_lowercase().as_str() {
                    "true" | "on" | "yes" | "1" | "enabled" => true,
                    "false" | "off" | "no" | "0" | "disabled" => false,
                    _ => {
                        return Err(i18n.text_args(
                            "runtime-setting-error-bool",
                            &crate::fl_args!("field" => field_label.clone()),
                        ));
                    }
                };
                self.parallel_tool_calls = Some(value);
            }
            RuntimeSettingId::Temperature => {
                let value = trimmed.parse::<f32>().map_err(|_| {
                    i18n.text_args(
                        "runtime-setting-error-number",
                        &crate::fl_args!("field" => field_label.clone()),
                    )
                })?;
                if !value.is_finite() {
                    return Err(i18n.text_args(
                        "runtime-setting-error-finite",
                        &crate::fl_args!("field" => field_label.clone()),
                    ));
                }
                self.temperature = Some(value);
            }
            RuntimeSettingId::MaxOutput => {
                let value = trimmed.parse::<u32>().map_err(|_| {
                    i18n.text_args(
                        "runtime-setting-error-positive-int",
                        &crate::fl_args!("field" => field_label.clone()),
                    )
                })?;
                if value == 0 {
                    return Err(i18n.text_args(
                        "runtime-setting-error-positive-int",
                        &crate::fl_args!("field" => field_label.clone()),
                    ));
                }
                self.max_output_tokens = Some(value);
            }
            RuntimeSettingId::System => self.system = Some(trimmed.to_string()),
        }

        Ok(i18n.text_args(
            "runtime-setting-apply-updated",
            &crate::fl_args!("field" => field_label),
        ))
    }

    fn to_request(&self) -> RunOptions {
        RunOptions {
            model: self.model.clone(),
            thinking_mode: self.thinking_mode.clone(),
            speed_mode: self.speed_mode.clone(),
            verbosity: self.verbosity.clone(),
            parallel_tool_calls: self.parallel_tool_calls,
            agent_profile: None,
            system: self.system.clone(),
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
        }
    }

    fn summary(&self, i18n: &I18n) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(model) = self.model.as_ref() {
            parts.push(format!("{}/{}", model.provider_id, model.model_id));
        }
        if let Some(thinking_mode) = self.thinking_mode.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-thinking",
                &crate::fl_args!("value" => ui_text::thinking_mode_display_value(thinking_mode)),
            ));
        }
        if let Some(speed_mode) = self.speed_mode.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-speed",
                &crate::fl_args!("value" => ui_text::speed_mode_display_value(speed_mode)),
            ));
        }
        if let Some(verbosity) = self.verbosity.as_ref() {
            parts.push(i18n.text_args(
                "run-options-summary-verbosity",
                &crate::fl_args!("value" => verbosity),
            ));
        }
        if let Some(parallel_tool_calls) = self.parallel_tool_calls {
            parts.push(i18n.text_args(
                "run-options-summary-parallel-tools",
                &crate::fl_args!(
                    "value" => ui_text::t(
                        i18n,
                        if parallel_tool_calls {
                            "value-on"
                        } else {
                            "value-off"
                        },
                    )
                ),
            ));
        }
        if let Some(temperature) = self.temperature {
            parts.push(i18n.text_args(
                "run-options-summary-temperature",
                &crate::fl_args!("value" => format!("{temperature:.2}")),
            ));
        }
        if let Some(max_output_tokens) = self.max_output_tokens {
            parts.push(i18n.text_args(
                "run-options-summary-max-output",
                &crate::fl_args!("value" => max_output_tokens as i64),
            ));
        }
        if self
            .system
            .as_ref()
            .is_some_and(|system| !system.trim().is_empty())
        {
            parts.push(ui_text::t(i18n, "run-options-summary-system"));
        }
        (!parts.is_empty()).then(|| parts.join(" | "))
    }
}

impl ComposerDraft {
    fn with_text_prefix_stripped(mut self, count: usize) -> Self {
        let mut boundary = 0;
        let mut chars = self.text.char_indices();
        for _ in 0..count {
            let Some((index, ch)) = chars.next() else {
                return self;
            };
            boundary = index + ch.len_utf8();
        }
        self.text.drain(..boundary);
        self
    }
}

fn default_draft_store_path() -> PathBuf {
    let mut base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.push("agena");
    base.push("tui-drafts.json");
    base
}

fn default_prompt_history_path() -> PathBuf {
    let mut base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.push("agena");
    base.push("tui-prompt-history.jsonl");
    base
}

fn non_empty_owned(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn permission_mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Allow => "allow",
        PermissionMode::Ask => "ask",
        PermissionMode::Deny => "deny",
    }
}

impl Default for PermissionRuleDraft {
    fn default() -> Self {
        Self {
            subject_kind: PermissionRuleSubjectKind::Tool,
            tool_name: String::new(),
            qualifier: String::new(),
            path_access_kind: "read".to_string(),
            workspace_root: String::new(),
            target_path: String::new(),
            network_target: String::new(),
            network_host: String::new(),
            network_port: String::new(),
            scope: "workspace".to_string(),
            session_id: String::new(),
            mode: PermissionMode::Ask,
        }
    }
}

fn permission_rule_draft_from_resource(rule: &PermissionRuleResource) -> PermissionRuleDraft {
    PermissionRuleDraft {
        subject_kind: match rule.subject_kind.as_str() {
            "path_access" => PermissionRuleSubjectKind::PathAccess,
            "network_access" => PermissionRuleSubjectKind::NetworkAccess,
            _ => PermissionRuleSubjectKind::Tool,
        },
        tool_name: rule.tool_name.clone().unwrap_or_default(),
        qualifier: rule.qualifier.clone().unwrap_or_default(),
        path_access_kind: rule
            .path_access_kind
            .clone()
            .unwrap_or_else(|| "read".to_string()),
        workspace_root: rule.workspace_root.clone().unwrap_or_default(),
        target_path: rule.target_path.clone().unwrap_or_default(),
        network_target: rule
            .network_target
            .clone()
            .or_else(|| rule.network_host.clone())
            .unwrap_or_default(),
        network_host: rule.network_host.clone().unwrap_or_default(),
        network_port: rule
            .network_port
            .map(|port| port.to_string())
            .unwrap_or_default(),
        scope: rule.scope.clone(),
        session_id: rule.session_id.map(|id| id.to_string()).unwrap_or_default(),
        mode: rule.mode,
    }
}

fn permission_rule_draft_from_request(request: &PermissionRequest) -> PermissionRuleDraft {
    let mut draft = PermissionRuleDraft {
        mode: PermissionMode::Allow,
        scope: request
            .scope
            .map(|scope| scope.to_string())
            .unwrap_or_else(|| {
                if request.session_id.is_some() {
                    "session".to_string()
                } else {
                    "workspace".to_string()
                }
            }),
        session_id: request
            .session_id
            .map(|session_id| session_id.to_string())
            .unwrap_or_default(),
        ..PermissionRuleDraft::default()
    };
    match &request.action {
        PermissionAction::Tool {
            tool_name,
            qualifier,
        } => {
            draft.subject_kind = PermissionRuleSubjectKind::Tool;
            draft.tool_name = tool_name.clone();
            draft.qualifier = qualifier.clone().unwrap_or_default();
        }
        PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => {
            draft.subject_kind = PermissionRuleSubjectKind::PathAccess;
            draft.path_access_kind = access_kind.clone();
            draft.workspace_root = workspace_root.clone();
            draft.target_path = target_path.clone();
        }
        PermissionAction::NetworkAccess {
            target,
            host: _,
            port: _,
        } => {
            draft.subject_kind = PermissionRuleSubjectKind::NetworkAccess;
            draft.network_target = target.clone();
        }
    }
    draft
}

fn permission_rule_label(i18n: &I18n, rule: &PermissionRuleResource) -> String {
    match rule.subject_kind.as_str() {
        "tool" => match (rule.tool_name.as_deref(), rule.qualifier.as_deref()) {
            (Some(tool_name), Some(qualifier)) if !qualifier.trim().is_empty() => {
                format!("{tool_name} · {qualifier}")
            }
            (Some(tool_name), _) => tool_name.to_string(),
            _ => rule.action_key.clone(),
        },
        "path_access" => i18n.text_args(
            "permission-rule-label-path",
            &crate::fl_args!(
                "access" => permission_rule_path_access_kind_display(
                    i18n,
                    rule.path_access_kind.as_deref().unwrap_or("path"),
                ),
                "path" => rule
                    .target_path
                    .as_deref()
                    .unwrap_or(rule.action_key.as_str())
                    .to_string(),
            ),
        ),
        "network_access" => {
            let host = rule
                .network_host
                .as_deref()
                .or(rule.network_target.as_deref())
                .unwrap_or(rule.action_key.as_str());
            let target = match rule.network_port {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            };
            i18n.text_args(
                "permission-rule-label-network",
                &crate::fl_args!("target" => target),
            )
        }
        _ => rule.action_key.clone(),
    }
}

fn permission_rule_scope_label(i18n: &I18n, rule: &PermissionRuleResource) -> String {
    match rule.scope.as_str() {
        "session" => rule
            .session_id
            .map(|id| {
                i18n.text_args(
                    "permission-rule-scope-session",
                    &crate::fl_args!("id" => id),
                )
            })
            .unwrap_or_else(|| ui_text::t(i18n, "permission-rule-scope-session-generic")),
        "workspace" => rule
            .workspace_id
            .map(|id| {
                i18n.text_args(
                    "permission-rule-scope-workspace",
                    &crate::fl_args!("id" => id),
                )
            })
            .unwrap_or_else(|| ui_text::t(i18n, "permission-rule-scope-workspace-generic")),
        "global" => ui_text::t(i18n, "value-global"),
        other => other.to_string(),
    }
}

fn permission_rule_detail(i18n: &I18n, rule: &PermissionRuleResource) -> String {
    let mut facts = vec![
        i18n.text_args(
            "permission-rule-detail-mode",
            &crate::fl_args!("mode" => permission_mode_display(i18n, rule.mode)),
        ),
        i18n.text_args(
            "permission-rule-detail-scope",
            &crate::fl_args!("scope" => permission_rule_scope_label(i18n, rule)),
        ),
        i18n.text_args(
            "permission-rule-detail-source",
            &crate::fl_args!("source" => rule.source.clone()),
        ),
    ];
    if let Some(operator) = rule
        .operator
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        facts.push(i18n.text_args(
            "permission-rule-detail-operator",
            &crate::fl_args!("operator" => operator.to_string()),
        ));
    }
    if let Some(reason) = rule
        .reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        facts.push(i18n.text_args(
            "permission-rule-detail-reason",
            &crate::fl_args!("reason" => reason.to_string()),
        ));
    }
    facts.push(i18n.text_args(
        "permission-rule-detail-updated",
        &crate::fl_args!("updated" => rule.updated_at.to_string()),
    ));
    join_inline_segments(facts)
}

fn permission_rule_draft_label(i18n: &I18n, draft: &PermissionRuleDraft) -> String {
    match draft.subject_kind {
        PermissionRuleSubjectKind::Tool => {
            let tool_name = draft.tool_name.trim();
            let qualifier = draft.qualifier.trim();
            if qualifier.is_empty() {
                tool_name.to_string()
            } else {
                format!("{tool_name} · {qualifier}")
            }
        }
        PermissionRuleSubjectKind::PathAccess => format!(
            "{} · {}",
            permission_rule_path_access_kind_display(i18n, draft.path_access_kind.trim()),
            draft.target_path.trim()
        ),
        PermissionRuleSubjectKind::NetworkAccess => {
            let target = draft.network_target.trim();
            if target.is_empty() {
                ui_text::t(i18n, "value-network")
            } else {
                i18n.text_args(
                    "permission-rule-label-network",
                    &crate::fl_args!("target" => target.to_string()),
                )
            }
        }
    }
}

fn permission_rule_subject_kind_name(kind: PermissionRuleSubjectKind) -> &'static str {
    match kind {
        PermissionRuleSubjectKind::Tool => "tool",
        PermissionRuleSubjectKind::PathAccess => "path_access",
        PermissionRuleSubjectKind::NetworkAccess => "network_access",
    }
}

fn permission_rule_mode_label(mode: PermissionMode) -> &'static str {
    permission_mode_name(mode)
}

fn permission_mode_display(i18n: &I18n, mode: PermissionMode) -> String {
    ui_text::t(
        i18n,
        match mode {
            PermissionMode::Allow => "value-allow",
            PermissionMode::Ask => "value-ask",
            PermissionMode::Deny => "value-deny",
        },
    )
}

fn permission_mode_token_display(i18n: &I18n, mode: &str) -> String {
    match mode.trim() {
        "allow" => ui_text::t(i18n, "value-allow"),
        "ask" => ui_text::t(i18n, "value-ask"),
        "deny" => ui_text::t(i18n, "value-deny"),
        other => other.to_string(),
    }
}

fn permission_rule_subject_kind_display(i18n: &I18n, kind: PermissionRuleSubjectKind) -> String {
    ui_text::t(
        i18n,
        match kind {
            PermissionRuleSubjectKind::Tool => "value-permission-rule-subject-tool",
            PermissionRuleSubjectKind::PathAccess => "value-permission-rule-subject-path-access",
            PermissionRuleSubjectKind::NetworkAccess => {
                "value-permission-rule-subject-network-access"
            }
        },
    )
}

fn permission_rule_path_access_kind_display(i18n: &I18n, kind: &str) -> String {
    match kind.trim() {
        "read" => ui_text::t(i18n, "value-read"),
        "write" => ui_text::t(i18n, "value-write"),
        "read_write" => ui_text::t(i18n, "value-read-write"),
        "path" => ui_text::t(i18n, "value-path"),
        other => other.to_string(),
    }
}

fn permission_rule_scope_display(i18n: &I18n, scope: &str) -> String {
    match scope.trim() {
        "session" => ui_text::t(i18n, "value-session"),
        "workspace" => ui_text::t(i18n, "value-workspace"),
        "global" => ui_text::t(i18n, "value-global"),
        other => other.to_string(),
    }
}

fn permission_rule_value_or(i18n: &I18n, value: &str, fallback_key: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        ui_text::t(i18n, fallback_key)
    } else {
        value.to_string()
    }
}

fn permission_rule_studio_item(
    i18n: &I18n,
    label_key: &str,
    value: String,
    detail_key: &str,
    action: PermissionRuleStudioAction,
) -> PermissionRuleStudioItem {
    PermissionRuleStudioItem {
        label: ui_text::t(i18n, label_key),
        value,
        detail: ui_text::t(i18n, detail_key),
        action,
    }
}

fn permission_rule_choice_overlay_spec(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    field: PermissionRuleStudioChoiceField,
) -> (String, String, Editor, Vec<ChoiceItem>, bool) {
    match field {
        PermissionRuleStudioChoiceField::SubjectKind => (
            ui_text::t(i18n, "overlay-permission-rule-choice-subject-title"),
            ui_text::t(i18n, "overlay-permission-rule-choice-subject-prompt"),
            Editor::from_text(permission_rule_subject_kind_name(draft.subject_kind).to_string()),
            vec![
                choice_item(
                    "tool",
                    ui_text::t(i18n, "overlay-permission-rule-choice-subject-tool-detail"),
                ),
                choice_item(
                    "path_access",
                    ui_text::t(
                        i18n,
                        "overlay-permission-rule-choice-subject-path-access-detail",
                    ),
                ),
                choice_item(
                    "network_access",
                    ui_text::t(
                        i18n,
                        "overlay-permission-rule-choice-subject-network-access-detail",
                    ),
                ),
            ],
            false,
        ),
        PermissionRuleStudioChoiceField::PathAccessKind => (
            ui_text::t(i18n, "overlay-permission-rule-choice-access-title"),
            ui_text::t(i18n, "overlay-permission-rule-choice-access-prompt"),
            Editor::from_text(draft.path_access_kind.clone()),
            vec![
                choice_item(
                    "read",
                    ui_text::t(i18n, "overlay-permission-rule-choice-access-read-detail"),
                ),
                choice_item(
                    "write",
                    ui_text::t(i18n, "overlay-permission-rule-choice-access-write-detail"),
                ),
                choice_item(
                    "read_write",
                    ui_text::t(
                        i18n,
                        "overlay-permission-rule-choice-access-read-write-detail",
                    ),
                ),
            ],
            false,
        ),
        PermissionRuleStudioChoiceField::Scope => (
            ui_text::t(i18n, "overlay-permission-rule-choice-scope-title"),
            ui_text::t(i18n, "overlay-permission-rule-choice-scope-prompt"),
            Editor::from_text(draft.scope.clone()),
            vec![
                choice_item(
                    "session",
                    ui_text::t(i18n, "overlay-permission-rule-choice-scope-session-detail"),
                ),
                choice_item(
                    "workspace",
                    ui_text::t(
                        i18n,
                        "overlay-permission-rule-choice-scope-workspace-detail",
                    ),
                ),
                choice_item(
                    "global",
                    ui_text::t(i18n, "overlay-permission-rule-choice-scope-global-detail"),
                ),
            ],
            false,
        ),
        PermissionRuleStudioChoiceField::Mode => (
            ui_text::t(i18n, "overlay-permission-rule-choice-mode-title"),
            ui_text::t(i18n, "overlay-permission-rule-choice-mode-prompt"),
            Editor::from_text(permission_rule_mode_label(draft.mode).to_string()),
            vec![
                choice_item(
                    "allow",
                    ui_text::t(i18n, "overlay-permission-rule-choice-mode-allow-detail"),
                ),
                choice_item(
                    "ask",
                    ui_text::t(i18n, "overlay-permission-rule-choice-mode-ask-detail"),
                ),
                choice_item(
                    "deny",
                    ui_text::t(i18n, "overlay-permission-rule-choice-mode-deny-detail"),
                ),
            ],
            false,
        ),
    }
}

fn permission_rule_editor_spec(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    field: PermissionRuleStudioEditField,
) -> (String, String, String, String) {
    let footer = ui_text::t(i18n, "overlay-permission-rule-editor-footer");
    match field {
        PermissionRuleStudioEditField::ToolName => (
            ui_text::t(i18n, "overlay-permission-rule-editor-tool-name-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-tool-name-prompt"),
            footer,
            draft.tool_name.clone(),
        ),
        PermissionRuleStudioEditField::Qualifier => (
            ui_text::t(i18n, "overlay-permission-rule-editor-qualifier-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-qualifier-prompt"),
            footer,
            draft.qualifier.clone(),
        ),
        PermissionRuleStudioEditField::WorkspaceRoot => (
            ui_text::t(i18n, "overlay-permission-rule-editor-workspace-root-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-workspace-root-prompt"),
            footer,
            draft.workspace_root.clone(),
        ),
        PermissionRuleStudioEditField::TargetPath => (
            ui_text::t(i18n, "overlay-permission-rule-editor-target-path-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-target-path-prompt"),
            footer,
            draft.target_path.clone(),
        ),
        PermissionRuleStudioEditField::NetworkTarget => (
            ui_text::t(i18n, "overlay-permission-rule-editor-network-target-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-network-target-prompt"),
            footer,
            draft.network_target.clone(),
        ),
        PermissionRuleStudioEditField::SessionId => (
            ui_text::t(i18n, "overlay-permission-rule-editor-session-id-title"),
            ui_text::t(i18n, "overlay-permission-rule-editor-session-id-prompt"),
            footer,
            draft.session_id.clone(),
        ),
    }
}

fn permission_rule_path_browser_spec(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    field: PermissionRuleStudioPathField,
) -> (String, String, PathBrowserMode, String) {
    match field {
        PermissionRuleStudioPathField::WorkspaceRoot => (
            ui_text::t(i18n, "overlay-permission-rule-browser-workspace-root-title"),
            ui_text::t(
                i18n,
                "overlay-permission-rule-browser-workspace-root-prompt",
            ),
            PathBrowserMode::DirectoryOnly,
            draft.workspace_root.clone(),
        ),
        PermissionRuleStudioPathField::TargetPath => (
            ui_text::t(i18n, "overlay-permission-rule-browser-target-path-title"),
            ui_text::t(i18n, "overlay-permission-rule-browser-target-path-prompt"),
            PathBrowserMode::AnyPath,
            draft.target_path.clone(),
        ),
    }
}

fn permission_rule_preview_lines(i18n: &I18n, draft: &PermissionRuleDraft) -> Vec<String> {
    let mut lines = vec![
        i18n.text_args(
            "overlay-permission-rule-preview-label",
            &crate::fl_args!("label" => permission_rule_draft_label(i18n, draft)),
        ),
        i18n.text_args(
            "overlay-permission-rule-preview-mode",
            &crate::fl_args!("mode" => permission_mode_display(i18n, draft.mode)),
        ),
        i18n.text_args(
            "overlay-permission-rule-preview-scope",
            &crate::fl_args!("scope" => permission_rule_scope_display(i18n, draft.scope.as_str())),
        ),
    ];
    match draft.subject_kind {
        PermissionRuleSubjectKind::Tool => {
            lines.push(i18n.text_args(
                "overlay-permission-rule-preview-subject-tool",
                &crate::fl_args!("tool" => draft.tool_name.trim().to_string()),
            ));
            if !draft.qualifier.trim().is_empty() {
                lines.push(i18n.text_args(
                    "overlay-permission-rule-preview-qualifier",
                    &crate::fl_args!("qualifier" => draft.qualifier.trim().to_string()),
                ));
            }
        }
        PermissionRuleSubjectKind::PathAccess => {
            lines.push(i18n.text_args(
                "overlay-permission-rule-preview-subject-path",
                &crate::fl_args!(
                    "access" => permission_rule_path_access_kind_display(
                        i18n,
                        draft.path_access_kind.trim(),
                    ),
                ),
            ));
            lines.push(i18n.text_args(
                "overlay-permission-rule-preview-target",
                &crate::fl_args!("target" => draft.target_path.trim().to_string()),
            ));
            if !draft.workspace_root.trim().is_empty() {
                lines.push(i18n.text_args(
                    "overlay-permission-rule-preview-workspace-root",
                    &crate::fl_args!("path" => draft.workspace_root.trim().to_string()),
                ));
            }
        }
        PermissionRuleSubjectKind::NetworkAccess => {
            lines.push(ui_text::t(
                i18n,
                "overlay-permission-rule-preview-subject-network",
            ));
            lines.push(i18n.text_args(
                "overlay-permission-rule-preview-target",
                &crate::fl_args!("target" => draft.network_target.trim().to_string()),
            ));
        }
    }
    if draft.scope == "session" && !draft.session_id.trim().is_empty() {
        lines.push(i18n.text_args(
            "overlay-permission-rule-preview-session-id",
            &crate::fl_args!("session" => draft.session_id.trim().to_string()),
        ));
    }
    lines
}

fn permission_rule_studio_items(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    rule_id: Option<i64>,
) -> Vec<PermissionRuleStudioItem> {
    let mut items = vec![
        permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-subject-kind",
            permission_rule_subject_kind_display(i18n, draft.subject_kind),
            "overlay-permission-rule-item-subject-kind-detail",
            PermissionRuleStudioAction::SubjectKind,
        ),
        permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-mode",
            permission_mode_display(i18n, draft.mode),
            "overlay-permission-rule-item-mode-detail",
            PermissionRuleStudioAction::Mode,
        ),
        permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-scope",
            permission_rule_scope_display(i18n, draft.scope.as_str()),
            "overlay-permission-rule-item-scope-detail",
            PermissionRuleStudioAction::Scope,
        ),
    ];

    if draft.scope == "session" {
        items.push(permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-session-id",
            permission_rule_value_or(i18n, draft.session_id.as_str(), "value-unset"),
            "overlay-permission-rule-item-session-id-detail",
            PermissionRuleStudioAction::SessionId,
        ));
    }

    match draft.subject_kind {
        PermissionRuleSubjectKind::Tool => {
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-tool-name",
                permission_rule_value_or(i18n, draft.tool_name.as_str(), "value-unset"),
                "overlay-permission-rule-item-tool-name-detail",
                PermissionRuleStudioAction::ToolName,
            ));
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-qualifier",
                permission_rule_value_or(i18n, draft.qualifier.as_str(), "value-none"),
                "overlay-permission-rule-item-qualifier-detail",
                PermissionRuleStudioAction::Qualifier,
            ));
        }
        PermissionRuleSubjectKind::PathAccess => {
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-access-kind",
                permission_rule_path_access_kind_display(i18n, draft.path_access_kind.as_str()),
                "overlay-permission-rule-item-access-kind-detail",
                PermissionRuleStudioAction::PathAccessKind,
            ));
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-target-path",
                permission_rule_value_or(i18n, draft.target_path.as_str(), "value-unset"),
                "overlay-permission-rule-item-target-path-detail",
                PermissionRuleStudioAction::TargetPath,
            ));
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-browse-target-path",
                ui_text::t(i18n, "overlay-permission-rule-item-browser-value"),
                "overlay-permission-rule-item-browse-target-path-detail",
                PermissionRuleStudioAction::BrowseTargetPath,
            ));
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-workspace-root",
                permission_rule_value_or(
                    i18n,
                    draft.workspace_root.as_str(),
                    "value-runtime-default",
                ),
                "overlay-permission-rule-item-workspace-root-detail",
                PermissionRuleStudioAction::WorkspaceRoot,
            ));
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-browse-workspace-root",
                ui_text::t(i18n, "overlay-permission-rule-item-browser-value"),
                "overlay-permission-rule-item-browse-workspace-root-detail",
                PermissionRuleStudioAction::BrowseWorkspaceRoot,
            ));
        }
        PermissionRuleSubjectKind::NetworkAccess => {
            items.push(permission_rule_studio_item(
                i18n,
                "overlay-permission-rule-item-network-target",
                permission_rule_value_or(i18n, draft.network_target.as_str(), "value-unset"),
                "overlay-permission-rule-item-network-target-detail",
                PermissionRuleStudioAction::NetworkTarget,
            ));
        }
    }

    items.push(permission_rule_studio_item(
        i18n,
        "overlay-permission-rule-item-save",
        permission_rule_draft_label(i18n, draft),
        "overlay-permission-rule-item-save-detail",
        PermissionRuleStudioAction::Save,
    ));

    if rule_id.is_some() {
        items.push(permission_rule_studio_item(
            i18n,
            "overlay-permission-rule-item-revoke",
            ui_text::t(i18n, "value-inactive"),
            "overlay-permission-rule-item-revoke-detail",
            PermissionRuleStudioAction::Revoke,
        ));
    }

    items
}

fn refresh_permission_rule_studio_dialog(i18n: &I18n, dialog: &mut PermissionRuleStudioOverlay) {
    let preferred_item = dialog
        .workbench
        .list
        .selected_item()
        .map(|item| item.label.as_str());
    let items = permission_rule_studio_items(i18n, &dialog.draft, dialog.rule_id);
    let selected = preferred_item
        .and_then(|label| items.iter().position(|item| item.label == label))
        .unwrap_or(0);
    dialog.workbench.list = SelectableListState::new(items, selected);
}

fn permission_rule_studio_detail_text(
    i18n: &I18n,
    draft: &PermissionRuleDraft,
    item: &PermissionRuleStudioItem,
) -> String {
    match item.action {
        PermissionRuleStudioAction::SubjectKind => {
            ui_text::t(i18n, "overlay-permission-rule-detail-subject-kind")
        }
        PermissionRuleStudioAction::ToolName => {
            ui_text::t(i18n, "overlay-permission-rule-detail-tool-name")
        }
        PermissionRuleStudioAction::Qualifier => {
            ui_text::t(i18n, "overlay-permission-rule-detail-qualifier")
        }
        PermissionRuleStudioAction::PathAccessKind => {
            ui_text::t(i18n, "overlay-permission-rule-detail-path-access-kind")
        }
        PermissionRuleStudioAction::WorkspaceRoot => {
            ui_text::t(i18n, "overlay-permission-rule-detail-workspace-root")
        }
        PermissionRuleStudioAction::BrowseWorkspaceRoot => {
            ui_text::t(i18n, "overlay-permission-rule-detail-browse-workspace-root")
        }
        PermissionRuleStudioAction::TargetPath => {
            ui_text::t(i18n, "overlay-permission-rule-detail-target-path")
        }
        PermissionRuleStudioAction::BrowseTargetPath => {
            ui_text::t(i18n, "overlay-permission-rule-detail-browse-target-path")
        }
        PermissionRuleStudioAction::NetworkTarget => {
            ui_text::t(i18n, "overlay-permission-rule-detail-network-target")
        }
        PermissionRuleStudioAction::Scope => {
            ui_text::t(i18n, "overlay-permission-rule-detail-scope")
        }
        PermissionRuleStudioAction::SessionId => {
            ui_text::t(i18n, "overlay-permission-rule-detail-session-id")
        }
        PermissionRuleStudioAction::Mode => ui_text::t(i18n, "overlay-permission-rule-detail-mode"),
        PermissionRuleStudioAction::Save => {
            let mut lines = vec![ui_text::t(i18n, "overlay-permission-rule-preview-heading")];
            lines.push(String::new());
            lines.extend(permission_rule_preview_lines(i18n, draft));
            lines.join("\n")
        }
        PermissionRuleStudioAction::Revoke => {
            ui_text::t(i18n, "overlay-permission-rule-detail-revoke")
        }
    }
}

fn render_permission_rule_draft(draft: &PermissionRuleDraft) -> String {
    match draft.subject_kind {
        PermissionRuleSubjectKind::Tool => {
            let mut parts = vec![
                "tool".to_string(),
                shell_quote_or_dash(draft.tool_name.trim()),
                permission_mode_name(draft.mode).to_string(),
                format!("scope={}", draft.scope.trim()),
            ];
            if !draft.qualifier.trim().is_empty() {
                parts.push(format!(
                    "qualifier={}",
                    shell_quote_or_dash(draft.qualifier.trim())
                ));
            }
            if draft.scope.trim() == "session" && !draft.session_id.trim().is_empty() {
                parts.push(format!("session={}", draft.session_id.trim()));
            }
            parts.join(" ")
        }
        PermissionRuleSubjectKind::PathAccess => {
            let mut parts = vec![
                "path".to_string(),
                shell_quote_or_dash(draft.path_access_kind.trim()),
                shell_quote_or_dash(draft.target_path.trim()),
                permission_mode_name(draft.mode).to_string(),
                format!("scope={}", draft.scope.trim()),
            ];
            if !draft.workspace_root.trim().is_empty() {
                parts.push(format!(
                    "workspace_root={}",
                    shell_quote_or_dash(draft.workspace_root.trim())
                ));
            }
            if draft.scope.trim() == "session" && !draft.session_id.trim().is_empty() {
                parts.push(format!("session={}", draft.session_id.trim()));
            }
            parts.join(" ")
        }
        PermissionRuleSubjectKind::NetworkAccess => {
            let mut parts = vec![
                "network".to_string(),
                shell_quote_or_dash(draft.network_target.trim()),
                permission_mode_name(draft.mode).to_string(),
                format!("scope={}", draft.scope.trim()),
            ];
            if draft.scope.trim() == "session" && !draft.session_id.trim().is_empty() {
                parts.push(format!("session={}", draft.session_id.trim()));
            }
            parts.join(" ")
        }
    }
}

fn render_permission_rule_preview(i18n: &I18n, input: &str) -> String {
    match parse_permission_rule_input(i18n, input) {
        Ok(draft) => permission_rule_preview_lines(i18n, &draft).join("\n"),
        Err(error) => i18n.text_args(
            "overlay-permission-rule-preview-invalid",
            &crate::fl_args!("error" => error),
        ),
    }
}

fn permission_rule_edit_help() -> String {
    [
        "tool <tool_name> <allow|ask|deny> [qualifier=<text>] [scope=session|workspace|global] [session=<id>]",
        "path <read|write|read_write> <target_path> <allow|ask|deny> [scope=session|workspace|global] [session=<id>] [workspace_root=<path>]",
        "network <target|host:port|url> <allow|ask|deny> [scope=session|workspace|global] [session=<id>]",
    ]
    .join("\n")
}

fn permission_rule_params_from_draft(draft: &PermissionRuleDraft) -> UpsertPermissionRuleParams {
    match draft.subject_kind {
        PermissionRuleSubjectKind::Tool => UpsertPermissionRuleParams {
            action_key: None,
            subject_kind: Some("tool".to_string()),
            tool_name: Some(draft.tool_name.trim().to_string()),
            qualifier: non_empty_owned(draft.qualifier.clone()),
            path_access_kind: None,
            workspace_root: None,
            target_path: None,
            network_target: None,
            network_host: None,
            network_port: None,
            scope: Some(draft.scope.trim().to_string()),
            session_id: if draft.scope.trim() == "session" {
                draft.session_id.trim().parse::<i64>().ok()
            } else {
                None
            },
            mode: draft.mode,
        },
        PermissionRuleSubjectKind::PathAccess => UpsertPermissionRuleParams {
            action_key: None,
            subject_kind: Some("path_access".to_string()),
            tool_name: None,
            qualifier: None,
            path_access_kind: Some(draft.path_access_kind.trim().to_string()),
            workspace_root: non_empty_owned(draft.workspace_root.clone()),
            target_path: Some(draft.target_path.trim().to_string()),
            network_target: None,
            network_host: None,
            network_port: None,
            scope: Some(draft.scope.trim().to_string()),
            session_id: if draft.scope.trim() == "session" {
                draft.session_id.trim().parse::<i64>().ok()
            } else {
                None
            },
            mode: draft.mode,
        },
        PermissionRuleSubjectKind::NetworkAccess => UpsertPermissionRuleParams {
            action_key: None,
            subject_kind: Some("network_access".to_string()),
            tool_name: None,
            qualifier: None,
            path_access_kind: None,
            workspace_root: None,
            target_path: None,
            network_target: Some(draft.network_target.trim().to_string()),
            network_host: None,
            network_port: None,
            scope: Some(draft.scope.trim().to_string()),
            session_id: if draft.scope.trim() == "session" {
                draft.session_id.trim().parse::<i64>().ok()
            } else {
                None
            },
            mode: draft.mode,
        },
    }
}

fn parse_permission_rule_input(
    i18n: &I18n,
    input: &str,
) -> std::result::Result<PermissionRuleDraft, String> {
    let tokens = shlex::split(input)
        .ok_or_else(|| ui_text::t(i18n, "permission-rule-error-invalid-shell-args"))?;
    if tokens.len() < 4 {
        return Err(ui_text::t(
            i18n,
            "permission-rule-error-expected-structured",
        ));
    }
    let subject = tokens[0].to_ascii_lowercase();
    let mut draft = PermissionRuleDraft::default();
    match subject.as_str() {
        "tool" => {
            draft.subject_kind = PermissionRuleSubjectKind::Tool;
            draft.tool_name = tokens[1].clone();
            draft.mode = parse_permission_mode_token(i18n, tokens[2].as_str())?;
            for token in &tokens[3..] {
                let (key, value) = split_permission_rule_option(i18n, token)?;
                match key {
                    "qualifier" => draft.qualifier = value.to_string(),
                    "scope" => draft.scope = parse_permission_scope_token(i18n, value)?.to_string(),
                    "session" => draft.session_id = value.to_string(),
                    _ => {
                        return Err(i18n.text_args(
                            "permission-rule-error-unknown-option",
                            &crate::fl_args!("key" => key.to_string()),
                        ));
                    }
                }
            }
            if draft.tool_name.trim().is_empty() {
                return Err(ui_text::t(i18n, "permission-rule-error-tool-name-required"));
            }
        }
        "path" => {
            draft.subject_kind = PermissionRuleSubjectKind::PathAccess;
            draft.path_access_kind = tokens[1].clone();
            draft.target_path = tokens[2].clone();
            draft.mode = parse_permission_mode_token(i18n, tokens[3].as_str())?;
            for token in &tokens[4..] {
                let (key, value) = split_permission_rule_option(i18n, token)?;
                match key {
                    "scope" => draft.scope = parse_permission_scope_token(i18n, value)?.to_string(),
                    "session" => draft.session_id = value.to_string(),
                    "workspace_root" => draft.workspace_root = value.to_string(),
                    _ => {
                        return Err(i18n.text_args(
                            "permission-rule-error-unknown-option",
                            &crate::fl_args!("key" => key.to_string()),
                        ));
                    }
                }
            }
            if draft.path_access_kind.trim().is_empty() {
                return Err(ui_text::t(
                    i18n,
                    "permission-rule-error-path-access-kind-required",
                ));
            }
            if draft.target_path.trim().is_empty() {
                return Err(ui_text::t(
                    i18n,
                    "permission-rule-error-target-path-required",
                ));
            }
        }
        "network" => {
            draft.subject_kind = PermissionRuleSubjectKind::NetworkAccess;
            draft.network_target = tokens[1].clone();
            draft.mode = parse_permission_mode_token(i18n, tokens[2].as_str())?;
            for token in &tokens[3..] {
                let (key, value) = split_permission_rule_option(i18n, token)?;
                match key {
                    "scope" => draft.scope = parse_permission_scope_token(i18n, value)?.to_string(),
                    "session" => draft.session_id = value.to_string(),
                    _ => {
                        return Err(i18n.text_args(
                            "permission-rule-error-unknown-option",
                            &crate::fl_args!("key" => key.to_string()),
                        ));
                    }
                }
            }
            if draft.network_target.trim().is_empty() {
                return Err(ui_text::t(
                    i18n,
                    "permission-rule-error-network-target-required",
                ));
            }
        }
        _ => {
            return Err(ui_text::t(i18n, "permission-rule-error-invalid-subject"));
        }
    }
    if draft.scope == "session" && draft.session_id.trim().is_empty() {
        return Err(ui_text::t(
            i18n,
            "permission-rule-error-session-token-required",
        ));
    }
    Ok(draft)
}

fn parse_permission_mode_token(
    i18n: &I18n,
    token: &str,
) -> std::result::Result<PermissionMode, String> {
    match token.to_ascii_lowercase().as_str() {
        "allow" => Ok(PermissionMode::Allow),
        "ask" => Ok(PermissionMode::Ask),
        "deny" => Ok(PermissionMode::Deny),
        _ => Err(ui_text::t(i18n, "permission-rule-error-invalid-mode")),
    }
}

fn parse_permission_scope_token(
    i18n: &I18n,
    token: &str,
) -> std::result::Result<&'static str, String> {
    match token.to_ascii_lowercase().as_str() {
        "session" => Ok("session"),
        "workspace" => Ok("workspace"),
        "global" => Ok("global"),
        _ => Err(ui_text::t(i18n, "permission-rule-error-invalid-scope")),
    }
}

fn split_permission_rule_option<'a>(
    i18n: &I18n,
    token: &'a str,
) -> std::result::Result<(&'a str, &'a str), String> {
    token.split_once('=').ok_or_else(|| {
        i18n.text_args(
            "permission-rule-error-invalid-option-format",
            &crate::fl_args!("token" => token.to_string()),
        )
    })
}

fn shell_quote_or_dash(value: &str) -> String {
    if value.is_empty() {
        "<required>".to_string()
    } else {
        shlex::try_quote(value)
            .map(|quoted| quoted.into_owned())
            .unwrap_or_else(|_| value.to_string())
    }
}

fn parse_pr_command_args(
    args: &str,
) -> Result<(String, Option<String>, Option<String>, Option<String>)> {
    let tokens =
        shlex::split(args).ok_or_else(|| anyhow::anyhow!("invalid shell-style arguments"))?;
    let mut title_parts = Vec::new();
    let mut body = None;
    let mut base = None;
    let mut head = None;
    let mut index = 0;
    let mut parsing_options = false;

    while index < tokens.len() {
        let token = tokens[index].as_str();
        match token {
            "--body" => {
                parsing_options = true;
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --body"))?;
                body = Some(value.clone());
            }
            "--base" => {
                parsing_options = true;
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --base"))?;
                base = Some(value.clone());
            }
            "--head" => {
                parsing_options = true;
                index += 1;
                let value = tokens
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --head"))?;
                head = Some(value.clone());
            }
            _ if token.starts_with("--") => {
                return Err(anyhow::anyhow!("unknown /pr option: {token}"));
            }
            _ if parsing_options => {
                return Err(anyhow::anyhow!("unexpected positional argument: {token}"));
            }
            _ => title_parts.push(tokens[index].clone()),
        }
        index += 1;
    }

    if title_parts.is_empty() {
        return Err(anyhow::anyhow!("pull request title is required"));
    }

    Ok((title_parts.join(" "), body, base, head))
}

fn split_command_args_once(value: &str) -> Option<(&str, &str)> {
    let mut parts = value.splitn(2, char::is_whitespace);
    let first = parts.next()?.trim();
    let second = parts.next()?.trim();
    if first.is_empty() || second.is_empty() {
        None
    } else {
        Some((first, second))
    }
}

fn runtime_tool_matches_slash_query(label: &str, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    let label = label.to_ascii_lowercase();
    label == query || label.starts_with(query.as_str())
}

fn file_mention_suggestion_context_for_text(
    text: &str,
    cursor: usize,
) -> Option<FileMentionSuggestionContext> {
    let token_start = text[..cursor]
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0);
    let token = text.get(token_start..cursor)?;
    if !token.starts_with('@') || token.starts_with("@@") {
        return None;
    }
    if token[1..].contains('@') || token.contains('\n') {
        return None;
    }
    Some(FileMentionSuggestionContext {
        query: token[1..].to_string(),
        fingerprint: format!("{token}:{cursor}"),
        mention_range: token_start..cursor,
    })
}

fn slash_command_suggestion_context_for_text(
    text: &str,
    cursor: usize,
) -> Option<SlashCommandSuggestionContext> {
    let first_line_end = text.find('\n').unwrap_or(text.len());
    if cursor > first_line_end {
        return None;
    }
    let first_line = &text[..first_line_end];
    if !first_line.starts_with('/') || first_line.starts_with("//") {
        return None;
    }

    let name_start = 1;
    let name_end = first_line[name_start..]
        .find(char::is_whitespace)
        .map(|index| name_start + index)
        .unwrap_or(first_line.len());
    if cursor > name_end {
        return None;
    }

    let name = &first_line[name_start..name_end];
    if name.contains('/') {
        return None;
    }
    let rest_after_name = first_line[name_end..].trim_start();
    if name.is_empty() && !rest_after_name.is_empty() {
        return None;
    }

    Some(SlashCommandSuggestionContext {
        query: name.to_ascii_lowercase(),
        fingerprint: format!("{first_line}:{cursor}"),
        name_range: 0..name_end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena::message::{
        ExecutionStatus, MessagePart, OperationBlock, OperationPart, PartContent, UserInputOption,
    };
    use chrono::Utc;
    use serde_json::json;
    use tempfile::tempdir;

    fn permission_request(request_id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: request_id.to_string(),
            session_id: Some(1),
            action: PermissionAction::Tool {
                tool_name: "shell".to_string(),
                qualifier: None,
            },
            related_actions: Vec::new(),
            requested_actions: Vec::new(),
            reason: "needs approval".to_string(),
            explanation: String::new(),
            source: None,
            scope: None,
            operator: None,
            risk: PermissionRiskLevel::Medium,
            trace: Vec::new(),
            created_at: Utc::now(),
        }
    }

    fn user_input_request(request_id: &str) -> UserInputRequest {
        UserInputRequest {
            request_id: request_id.to_string(),
            session_id: Some(1),
            title: String::new(),
            body_markdown: String::new(),
            kind: String::new(),
            submit_label: String::new(),
            cancel_label: String::new(),
            questions: Vec::new(),
            created_at: Utc::now(),
        }
    }

    fn user_input_question(
        id: &str,
        option_labels: &[&str],
        allow_custom: bool,
    ) -> UserInputQuestion {
        UserInputQuestion {
            id: id.to_string(),
            header: String::new(),
            question: format!("question-{id}"),
            options: option_labels
                .iter()
                .map(|label| UserInputOption {
                    label: (*label).to_string(),
                    description: String::new(),
                })
                .collect(),
            multiple: false,
            allow_custom,
        }
    }

    fn pending_permission_request(request_id: &str) -> PendingInteractiveRequest {
        PendingInteractiveRequest::Permission {
            request: permission_request(request_id),
        }
    }

    fn pending_user_input_request(request_id: &str) -> PendingInteractiveRequest {
        PendingInteractiveRequest::UserInput {
            request: user_input_request(request_id),
        }
    }

    fn transcript_message(id: i64, role: MessageRole, text: &str) -> MessageResource {
        let created_at = Utc::now();
        let part = MessagePart::with_content(
            id.saturating_mul(10),
            id,
            created_at,
            ExecutionStatus::Completed,
            PartContent::text(text),
        );
        MessageResource {
            id,
            session_id: 1,
            role,
            state: MessageStatus::Completed,
            created_at,
            updated_at: created_at,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![part]),
        }
    }

    fn transcript_tool_message(id: i64, command: &str, stdout: &str) -> MessageResource {
        let created_at = Utc::now();
        let invocation = ToolInvocation::new(
            "bash",
            serde_json::from_value(json!({ "command": command }))
                .expect("valid structured tool input"),
        );
        let tool = OperationPart::completed(
            id.saturating_mul(10),
            invocation,
            stdout.to_string(),
            vec![OperationBlock::Command {
                command: command.to_string(),
                cwd: None,
                exit_code: Some(0),
                stdout: Some(stdout.to_string()),
                stderr: None,
            }],
            Vec::new(),
            agena::message::ToolOutput::default(),
            agena::message::TimeRange::default(),
        );
        let part = MessagePart::with_content(
            id.saturating_mul(100),
            id,
            created_at,
            ExecutionStatus::Completed,
            PartContent::Operation(tool),
        );
        MessageResource {
            id,
            session_id: 1,
            role: MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at,
            updated_at: created_at,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![part]),
        }
    }

    fn session_resource(
        id: i64,
        parent_id: Option<i64>,
        root_id: i64,
        is_subagent: bool,
    ) -> SessionResource {
        let now = Utc::now();
        SessionResource {
            id,
            parent_id,
            depth: match parent_id {
                Some(_) => 1,
                None => 0,
            },
            root_id,
            workspace_id: 1,
            title: format!("session-{id}"),
            version: 1,
            is_subagent,
            created_at: now,
            updated_at: now,
            message_count: 0,
            child_session_count: 0,
            last_message_at: None,
        }
    }

    fn session_execution_resource(
        run_state: SessionRunState,
        blocked: bool,
    ) -> SessionExecutionResource {
        SessionExecutionResource {
            session: session_resource(1, None, 1, false),
            blocked,
            run_state,
            latest_event_seq: None,
            automation: None,
            execution: SessionExecutionContextResource {
                agent_profile: None,
                active_skill_name: None,
                system_prompt_override: None,
                effective_permission: Default::default(),
                model_provider_id: None,
                model_adapter_id: None,
                model_id: None,
                model_thinking_mode: None,
                model_speed_mode: None,
                model_verbosity: None,
                model_parallel_tool_calls: None,
                effective_workspace_root: None,
                task_id: None,
            },
            pending_interactive_requests: Vec::new(),
            pending_permission_requests: Vec::new(),
            pending_user_input_requests: Vec::new(),
            usage: SessionUsageResource {
                measured_prompt_tokens: None,
                current_tokens: 0,
                projected_tokens: None,
                limit_tokens: None,
                limit_basis: None,
                reserved_tokens: None,
                model_context_window_tokens: None,
                model_max_input_tokens: None,
                model_max_output_tokens: None,
            },
        }
    }

    fn search_list_config(
        input_enabled: bool,
        search_enabled: bool,
        custom_value_enabled: bool,
    ) -> SearchListOverlayConfig {
        SearchListOverlayConfig {
            target_width: 96,
            input_enabled,
            search_enabled,
            custom_value_enabled,
            fill_selected_into_input: true,
            min_list_body_height: 3,
            max_list_body_height: 12,
        }
    }

    #[test]
    fn search_list_overlay_can_disable_custom_rows_per_instance() {
        let all_items = vec![choice_item("allow", "always allow matching actions")];
        let mut without_custom = ChoiceOverlay::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            "Empty".to_string(),
            Editor::from_text("typed".to_string()),
            search_list_config(true, false, false),
            Some(SearchListClearAction {
                label: "Clear value".to_string(),
                detail: "reset field".to_string(),
            }),
            ChoiceOverlayMeta {
                i18n: I18n::english(),
                all_items: all_items.clone(),
                action: ChoiceOverlayAction::PermissionRuleStudio(
                    PermissionRuleStudioChoiceField::Mode,
                ),
            },
        );
        App::refresh_choice_overlay(&mut without_custom);

        let mut with_custom = ChoiceOverlay::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            "Empty".to_string(),
            Editor::from_text("typed".to_string()),
            search_list_config(true, false, true),
            Some(SearchListClearAction {
                label: "Clear value".to_string(),
                detail: "reset field".to_string(),
            }),
            ChoiceOverlayMeta {
                i18n: I18n::english(),
                all_items,
                action: ChoiceOverlayAction::PermissionRuleStudio(
                    PermissionRuleStudioChoiceField::Mode,
                ),
            },
        );
        App::refresh_choice_overlay(&mut with_custom);

        assert_eq!(without_custom.row_count(), 2);
        assert_eq!(with_custom.row_count(), 3);
        assert!(matches!(
            with_custom.selected_row(),
            Some(SearchListRow::Clear(_))
        ));
        assert!(matches!(
            with_custom.rows().get(1),
            Some(SearchListRow::Custom(_))
        ));
    }

    #[test]
    fn search_list_custom_rows_follow_overlay_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let mut overlay = ChoiceOverlay::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            "Empty".to_string(),
            Editor::from_text("typed".to_string()),
            search_list_config(true, false, true),
            None,
            ChoiceOverlayMeta {
                i18n: i18n.clone(),
                all_items: vec![choice_item("allow", "always allow matching actions")],
                action: ChoiceOverlayAction::PermissionRuleStudio(
                    PermissionRuleStudioChoiceField::Mode,
                ),
            },
        );
        App::refresh_choice_overlay(&mut overlay);

        let rows = overlay.rows();
        match rows.get(0) {
            Some(SearchListRow::Custom(value)) => {
                assert_eq!(value.search_list_label(&overlay.meta), "使用输入值");
                assert_eq!(
                    value.search_list_detail(&overlay.meta),
                    Some(i18n.text_args(
                        "search-list-custom-value-detail",
                        &crate::fl_args!("value" => "\"typed\""),
                    ))
                );
            }
            other => panic!("expected localized custom row, got {other:?}"),
        }
    }

    #[test]
    fn search_list_overlay_respects_search_enabled_config() {
        let all_items = vec![
            choice_item("allow", "always allow matching actions"),
            choice_item("deny", "always deny matching actions"),
        ];
        let mut local_search = ChoiceOverlay::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            "Empty".to_string(),
            Editor::from_text("deny".to_string()),
            search_list_config(true, true, false),
            None,
            ChoiceOverlayMeta {
                i18n: I18n::english(),
                all_items: all_items.clone(),
                action: ChoiceOverlayAction::PermissionRuleStudio(
                    PermissionRuleStudioChoiceField::Mode,
                ),
            },
        );
        App::refresh_choice_overlay(&mut local_search);

        let mut remote_query = ChoiceOverlay::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            "Empty".to_string(),
            Editor::from_text("deny".to_string()),
            search_list_config(true, false, false),
            None,
            ChoiceOverlayMeta {
                i18n: I18n::english(),
                all_items,
                action: ChoiceOverlayAction::PermissionRuleStudio(
                    PermissionRuleStudioChoiceField::Mode,
                ),
            },
        );
        App::refresh_choice_overlay(&mut remote_query);

        assert_eq!(local_search.items.len(), 1);
        assert_eq!(remote_query.items.len(), 2);
    }

    #[test]
    fn choice_overlay_periodic_refresh_preserves_manual_selection() {
        let mut overlay = ChoiceOverlay::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            "Empty".to_string(),
            Editor::default(),
            search_list_config(true, false, false),
            None,
            ChoiceOverlayMeta {
                i18n: I18n::english(),
                all_items: vec![
                    choice_item("allow", "always allow matching actions"),
                    choice_item("deny", "always deny matching actions"),
                ],
                action: ChoiceOverlayAction::PermissionRuleStudio(
                    PermissionRuleStudioChoiceField::Mode,
                ),
            },
        );
        App::refresh_choice_overlay(&mut overlay);
        overlay.selected = 1;

        App::sync_choice_overlay_input(&mut overlay, false);
        assert_eq!(overlay.selected, 1);

        App::sync_choice_overlay_input(&mut overlay, true);
        assert_eq!(overlay.selected, 0);
    }

    #[test]
    fn first_unseen_pending_interactive_request_preserves_runtime_order() {
        let requests = vec![
            pending_user_input_request("input-1"),
            pending_permission_request("perm-1"),
        ];

        assert_eq!(
            first_unseen_pending_interactive_request(
                requests.as_slice(),
                &BTreeSet::new(),
                &BTreeSet::new(),
            )
            .map(pending_interactive_request_id),
            Some("input-1")
        );
    }

    #[test]
    fn seen_first_request_falls_back_to_next_pending_request() {
        let requests = vec![
            pending_permission_request("perm-1"),
            pending_user_input_request("input-1"),
        ];
        let seen_permissions = BTreeSet::from(["perm-1".to_string()]);

        assert_eq!(
            first_unseen_pending_interactive_request(
                requests.as_slice(),
                &seen_permissions,
                &BTreeSet::new(),
            )
            .map(pending_interactive_request_id),
            Some("input-1")
        );
    }

    #[test]
    fn pending_interactive_kind_reports_first_pending_request_kind() {
        let requests = vec![pending_user_input_request("input-1")];

        assert_eq!(
            pending_interactive_kind(requests.as_slice()),
            Some(PendingInteractiveKind::UserInput)
        );
    }

    #[test]
    fn execution_update_is_stale_when_latest_event_seq_moves_backwards() {
        assert!(execution_update_is_stale(Some(10), Some(9)));
        assert!(execution_update_is_stale(Some(10), None));
        assert!(!execution_update_is_stale(Some(10), Some(10)));
        assert!(!execution_update_is_stale(Some(10), Some(11)));
        assert!(!execution_update_is_stale(None, Some(1)));
    }

    #[test]
    fn permission_overlay_only_matches_same_pending_request() {
        let request = permission_request("perm-1");
        let overlay = PermissionOverlay {
            session_id: 1,
            request: request.clone(),
            selection: SelectionCursor::default(),
        };
        let mut execution = session_execution_resource(SessionRunState::Idle, false);

        execution.pending_interactive_requests =
            vec![PendingInteractiveRequest::Permission { request }];
        assert!(permission_overlay_matches_pending_request(
            &overlay,
            Some(1),
            Some(&execution),
        ));

        execution.pending_interactive_requests = vec![pending_permission_request("perm-2")];
        assert!(!permission_overlay_matches_pending_request(
            &overlay,
            Some(1),
            Some(&execution),
        ));

        execution.pending_interactive_requests.clear();
        assert!(!permission_overlay_matches_pending_request(
            &overlay,
            Some(1),
            Some(&execution),
        ));
    }

    #[test]
    fn user_input_selection_helpers_clamp_questions_and_options() {
        let request = UserInputRequest {
            request_id: "input-1".to_string(),
            session_id: Some(1),
            title: String::new(),
            body_markdown: String::new(),
            kind: String::new(),
            submit_label: String::new(),
            cancel_label: String::new(),
            questions: vec![
                user_input_question("q1", &["One", "Two"], false),
                user_input_question("q2", &["Only"], true),
            ],
            created_at: Utc::now(),
        };
        let mut overlay = App::build_user_input_overlay(1, request);

        App::move_user_input_option_to_end(&mut overlay);
        assert_eq!(overlay.state.selected_question(), 0);
        assert_eq!(overlay.state.selected_option(), 1);

        overlay.answers.insert(
            "q2".to_string(),
            UserInputAnswerDraft {
                option_indexes: BTreeSet::new(),
                custom_values: vec!["typed".to_string()],
            },
        );
        App::focus_user_input_question(&mut overlay, usize::MAX);
        assert_eq!(overlay.state.selected_question(), 1);
        assert_eq!(overlay.state.selected_option(), 1);

        App::move_user_input_option(&mut overlay, 10);
        assert_eq!(overlay.state.selected_option(), 1);

        overlay.state.set_screen(QuestionFlowScreen::Review);
        App::move_user_input_question(&mut overlay, -10);
        assert_eq!(overlay.state.selected_question(), 0);
        assert_eq!(overlay.state.screen(), QuestionFlowScreen::Review);
    }

    #[test]
    fn review_user_input_overlay_uses_fullscreen_decision_mode() {
        let request = UserInputRequest {
            request_id: "input-review".to_string(),
            session_id: Some(1),
            title: "Review Plan".to_string(),
            body_markdown: "# Plan\n\n- inspect the plan body".to_string(),
            kind: "review".to_string(),
            submit_label: "Submit decision".to_string(),
            cancel_label: "Keep in planning".to_string(),
            questions: vec![UserInputQuestion {
                id: "decision".to_string(),
                header: "Decision".to_string(),
                question: "Choose what should happen next.".to_string(),
                options: vec![
                    UserInputOption {
                        label: "Approve and run".to_string(),
                        description: "Approve the plan and run it.".to_string(),
                    },
                    UserInputOption {
                        label: "Keep in planning".to_string(),
                        description: "Return to draft for edits.".to_string(),
                    },
                ],
                multiple: false,
                allow_custom: false,
            }],
            created_at: Utc::now(),
        };
        let mut overlay = App::build_user_input_overlay(1, request);
        assert!(App::user_input_overlay_is_review(&overlay));

        overlay.review_option = 1;
        let reply = App::build_structured_user_input_reply(&I18n::english(), &mut overlay)
            .expect("review overlays should build a reply from the selected decision");
        assert_eq!(
            reply.answers.get("decision"),
            Some(&vec!["Keep in planning".to_string()])
        );
    }

    #[test]
    fn composer_input_is_active_only_when_the_composer_is_engaged() {
        assert!(composer_input_is_active(Focus::Composer, true, false));
        assert!(composer_input_is_active(Focus::Composer, false, true));
        assert!(!composer_input_is_active(Focus::Composer, false, false));
        assert!(!composer_input_is_active(Focus::Transcript, true, true));
    }

    #[test]
    fn preferred_visible_session_selection_falls_back_to_parent_for_hidden_subagent() {
        let subagent = session_resource(42, Some(7), 3, true);
        let visible = vec![
            session_resource(3, None, 3, false),
            session_resource(7, Some(3), 3, false),
        ];

        assert_eq!(
            preferred_visible_session_selection(&subagent, visible.as_slice()),
            Some(7)
        );
    }

    #[test]
    fn permission_request_fingerprint_ignores_runtime_request_identity() {
        let mut first = permission_request("perm-1");
        let mut second = permission_request("perm-2");
        second.created_at = second.created_at + chrono::Duration::milliseconds(250);

        assert_eq!(
            permission_request_fingerprint(&first),
            permission_request_fingerprint(&second)
        );

        first.reason = "different".to_string();
        assert_ne!(
            permission_request_fingerprint(&first),
            permission_request_fingerprint(&second)
        );
    }

    #[test]
    fn permission_action_label_includes_tool_qualifier() {
        let i18n = I18n::english();
        let label = permission_action_label(
            &i18n,
            &PermissionAction::Tool {
                tool_name: "bash".to_string(),
                qualifier: Some("npm test".to_string()),
            },
        );

        assert!(label.contains("tool:"));
        assert!(label.contains("bash"));
        assert!(label.contains("npm test"));
    }

    #[test]
    fn transcript_render_does_not_insert_blank_lines_between_messages() {
        let first = transcript_message(1, MessageRole::User, "first");
        let second = transcript_message(2, MessageRole::Assistant, "second");
        let i18n = I18n::english();
        let expected_line_count =
            render_message(&first, 80, &i18n).len() + render_message(&second, 80, &i18n).len();

        let mut transcript = TranscriptState {
            session_id: Some(1),
            messages: vec![first.clone(), second.clone()],
            ..TranscriptState::default()
        };

        let rendered = transcript.rendered(80);

        assert_eq!(rendered.lines.len(), expected_line_count);
        assert!(
            !rendered.lines.iter().any(|line| line.text.is_empty()),
            "expected transcript rendering to avoid inserting blank separator lines"
        );
    }

    #[test]
    fn transcript_line_motion_selects_next_block_before_entering_it() {
        let first = transcript_message(1, MessageRole::User, "first");
        let second = transcript_message(2, MessageRole::Assistant, "alpha\nbeta");
        let width = 80;
        let height = 10;
        let mut transcript = TranscriptState {
            session_id: Some(1),
            messages: vec![first, second],
            ..TranscriptState::default()
        };
        let nodes = transcript.rendered(width).nodes.clone();
        let first_node = nodes[0].clone();
        let second_node = nodes[1].clone();

        transcript.set_cursor_line(width, height, first_node.end_line.saturating_sub(1));
        transcript.scroll_by_lines_with_blocks(width, height, TranscriptMoveDirection::Down, 1);

        assert_eq!(
            transcript.highlighted_block_key(),
            Some(second_node.key.clone())
        );
        assert_eq!(transcript.cursor_line, second_node.start_line);
        assert_eq!(
            transcript.highlighted_block_range(width),
            Some(second_node.start_line..second_node.end_line)
        );

        transcript.scroll_by_lines_with_blocks(width, height, TranscriptMoveDirection::Down, 1);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(transcript.cursor_line, second_node.start_line);

        transcript.scroll_by_lines_with_blocks(width, height, TranscriptMoveDirection::Down, 1);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(
            transcript.cursor_line,
            second_node.start_line.saturating_add(1)
        );
    }

    #[test]
    fn transcript_block_motion_jumps_by_node() {
        let width = 80;
        let height = 10;
        let mut transcript = TranscriptState {
            session_id: Some(1),
            messages: vec![
                transcript_message(1, MessageRole::User, "first"),
                transcript_message(2, MessageRole::Assistant, "second"),
                transcript_message(3, MessageRole::User, "third"),
            ],
            ..TranscriptState::default()
        };
        let nodes = transcript.rendered(width).nodes.clone();

        transcript.move_by_blocks(width, height, TranscriptMoveDirection::Down, 2);

        assert_eq!(
            transcript.highlighted_block_key(),
            Some(nodes[2].key.clone())
        );
        assert_eq!(transcript.cursor_line, nodes[2].start_line);
        assert_eq!(
            transcript.highlighted_block_range(width),
            Some(nodes[2].start_line..nodes[2].end_line)
        );

        transcript.move_by_blocks(width, height, TranscriptMoveDirection::Up, 1);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(nodes[1].key.clone())
        );
        assert_eq!(transcript.cursor_line, nodes[1].end_line.saturating_sub(1));
    }

    #[test]
    fn collapsed_tool_output_is_a_single_block_and_expansion_round_trips() {
        let width = 120;
        let height = 10;
        let mut transcript = TranscriptState {
            session_id: Some(1),
            messages: vec![transcript_tool_message(
                1,
                "ls -la src",
                "file-a\nfile-b\nfile-c",
            )],
            ..TranscriptState::default()
        };

        let collapsed = transcript.rendered(width).nodes[0].clone();
        assert_eq!(collapsed.kind, TranscriptNodeKind::Tool);
        assert!(collapsed.toggleable);
        assert!(!collapsed.expanded);
        let collapsed_line_count = collapsed.end_line.saturating_sub(collapsed.start_line);
        assert!(collapsed_line_count >= 1);

        transcript.set_block_cursor(width, height, 0, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_range(width),
            Some(collapsed.start_line..collapsed.end_line)
        );

        transcript
            .node_expansions
            .insert(collapsed.key.clone(), true);
        transcript.invalidate_render();

        let expanded = transcript.rendered(width).nodes[0].clone();
        assert!(expanded.expanded);
        assert!(expanded.end_line.saturating_sub(expanded.start_line) > collapsed_line_count);

        transcript
            .node_expansions
            .insert(collapsed.key.clone(), false);
        transcript.invalidate_render();

        let collapsed_again = transcript.rendered(width).nodes[0].clone();
        assert!(!collapsed_again.expanded);
        assert_eq!(
            collapsed_again
                .end_line
                .saturating_sub(collapsed_again.start_line),
            collapsed_line_count
        );
    }

    fn usage_resource(
        current_tokens: u64,
        projected_tokens: Option<u64>,
        context_window_tokens: Option<u32>,
        limit_tokens: Option<u64>,
    ) -> SessionUsageResource {
        SessionUsageResource {
            measured_prompt_tokens: None,
            current_tokens,
            projected_tokens,
            limit_tokens,
            limit_basis: None,
            reserved_tokens: None,
            model_context_window_tokens: context_window_tokens,
            model_max_input_tokens: None,
            model_max_output_tokens: None,
        }
    }

    fn provider_studio_test_overlay(draft: ProviderConfigDraft) -> ProviderStudioOverlay {
        ProviderStudioOverlay {
            title: String::new(),
            footer: String::new(),
            show_provider_list: false,
            providers: SelectableListState::new(Vec::new(), 0),
            selection: DashboardSelectionState::new(
                [
                    ProviderStudioFocus::Fields,
                    ProviderStudioFocus::Adapters,
                    ProviderStudioFocus::Models,
                ],
                ProviderStudioFocus::Fields,
                0,
                0,
                0,
            ),
            draft,
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: Vec::new(),
            adapter_selection_touched: false,
            selected_adapter_ids: BTreeSet::new(),
            selected_model_keys: BTreeSet::new(),
            catalog_matches: BTreeMap::new(),
            listing_adapter_models: false,
            saving: false,
            pending_adapter_models_key: None,
            pending_auth_key: None,
            next_auth_poll_at: None,
            detail_page: None,
            model_page: None,
            editor: None,
        }
    }

    #[test]
    fn status_line_token_usage_uses_codex_style_context_percent_used() {
        let usage = usage_resource(135_200, None, Some(272_000), Some(244_800));

        assert_eq!(
            status_line_token_usage(&usage),
            Some(TokenUsageStatus::PercentUsed(50))
        );
        assert_eq!(
            session_summary_status_parts(None, None, status_line_token_usage(&usage)),
            vec!["50%".to_string()]
        );
    }

    #[test]
    fn status_line_token_usage_prefers_projected_tokens() {
        let usage = usage_resource(12_000, Some(135_200), Some(272_000), Some(244_800));

        assert_eq!(
            status_line_token_usage(&usage),
            Some(TokenUsageStatus::PercentUsed(50))
        );
    }

    #[test]
    fn status_line_token_usage_shows_used_tokens_when_context_window_unknown() {
        let usage = usage_resource(50, None, None, Some(100));

        assert_eq!(
            status_line_token_usage(&usage),
            Some(TokenUsageStatus::UsedTokens(50))
        );
        assert_eq!(
            session_summary_status_parts(None, None, status_line_token_usage(&usage)),
            vec!["0.1k used".to_string()]
        );
        assert_eq!(format_tokens_k(12_400), "12k");
        assert_eq!(format_token_progress_label(101), "100%");
    }

    #[test]
    fn permission_rule_studio_items_localize_non_english_display_values() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let draft = PermissionRuleDraft {
            subject_kind: PermissionRuleSubjectKind::PathAccess,
            path_access_kind: "read_write".to_string(),
            scope: "global".to_string(),
            ..PermissionRuleDraft::default()
        };
        let items = permission_rule_studio_items(&i18n, &draft, None);

        assert_eq!(items[0].value, "路径访问");
        assert_eq!(items[1].value, "询问");
        assert_eq!(items[2].value, "全局");
        assert!(
            items
                .iter()
                .any(|item| item.label == "访问类型" && item.value == "读写"),
            "expected localized access-kind display in permission rule studio"
        );
    }

    #[test]
    fn permission_rule_label_localizes_path_prefixes() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let now = Utc::now();
        let rule = PermissionRuleResource {
            id: 7,
            action_key: "path_access".to_string(),
            subject_kind: "path_access".to_string(),
            tool_name: None,
            qualifier: None,
            path_access_kind: Some("read".to_string()),
            workspace_root: None,
            target_path: Some("/tmp/demo.txt".to_string()),
            network_target: None,
            network_host: None,
            network_port: None,
            scope: "workspace".to_string(),
            workspace_id: None,
            session_id: None,
            source: "manual".to_string(),
            operator: None,
            reason: None,
            mode: PermissionMode::Ask,
            created_at: now,
            updated_at: now,
            revoked_at: None,
            revoked_by: None,
            revoked_reason: None,
        };

        assert_eq!(
            sanitize_terminal_text(&permission_rule_label(&i18n, &rule)),
            "读取 · /tmp/demo.txt"
        );
    }

    #[test]
    fn permission_rule_parser_errors_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let error = parse_permission_rule_input(&i18n, "tool demo maybe")
            .expect_err("expected localized parse error");

        assert_eq!(
            error,
            "需要一个结构化规则：以 tool/path 开头，并以 allow|ask|deny 结尾"
        );
    }

    #[test]
    fn runtime_setting_messages_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let mut run_options = RunOptionsState::default();

        assert_eq!(
            run_options.runtime_setting_summary(&i18n, RUNTIME_SETTINGS[0]),
            "默认"
        );

        let updated = run_options
            .apply_runtime_setting_input(&i18n, RUNTIME_SETTINGS[0], "high")
            .expect("expected localized success message");
        assert_eq!(
            updated,
            i18n.text_args(
                "runtime-setting-apply-updated",
                &crate::fl_args!(
                    "field" => runtime_setting_display_label(&i18n, RUNTIME_SETTINGS[0])
                ),
            )
        );

        let error = run_options
            .apply_runtime_setting_input(&i18n, RUNTIME_SETTINGS[3], "maybe")
            .expect_err("expected localized validation error");
        assert_eq!(
            error,
            i18n.text_args(
                "runtime-setting-error-bool",
                &crate::fl_args!(
                    "field" => runtime_setting_display_label(&i18n, RUNTIME_SETTINGS[3])
                ),
            )
        );
    }

    #[test]
    fn provider_studio_messages_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let auth_kind = ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab));
        let rule = auth_kind
            .adapter_rule("openai")
            .expect("expected localized adapter rule");

        assert_eq!(
            provider_studio_field_label(&i18n, ProviderStudioField::DefaultAdapter),
            "默认 Adapter"
        );
        assert_eq!(
            provider_studio_field_label(&i18n, ProviderStudioField::AuthLoginMethod),
            "登录方式"
        );
        assert_eq!(
            provider_studio_field_prompt(&i18n, ProviderStudioField::AuthMode),
            "更新 auth mode（none | api | credential）"
        );
        assert_eq!(
            provider_studio_field_prompt(&i18n, ProviderStudioField::AuthSubtype),
            "更新 auth subtype（api：custom | cline_api | gitlab_api | bedrock_sigv4；credential：openai_chatgpt | github_copilot | gitlab | google_adc | sap_ai_core）"
        );
        assert_eq!(
            provider_studio_field_prompt(&i18n, ProviderStudioField::AuthLoginMethod),
            "更新登录方式（device | browser）"
        );
        assert_eq!(
            provider_studio_adapter_rule_detail(&i18n, rule),
            "通过 openai adapter 路由的 GitLab OAuth 凭证。"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_live_listing_unavailable_message(
                &i18n,
                &ProviderDraftAuthKind::Credential(None)
            )),
            "当前 auth credential 不支持 live model listing"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_listing_auth_required_message(
                &i18n,
                &ProviderDraftAuthKind::Credential(None)
            )),
            "列出 adapter models 需要当前 auth/adapter 组合支持 live model discovery，或需要一个已保存的 provider；当前 auth 是 credential"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_model_count_label(&i18n, 3)),
            "3 个模型"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_catalog_match_label(&i18n, Some("gpt-4o"))),
            "catalog gpt-4o"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_draft_auth_action_message(
                &i18n,
                &crate::backend::ProviderDraftAuthMessage::OpenaiDeviceStarted {
                    user_code: "WXYZ-1234".to_string(),
                }
            )),
            "已开始 OpenAI 设备登录。打开对话框里显示的验证 URL，输入代码 WXYZ-1234，然后按 p。"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_draft_auth_action_message(
                &i18n,
                &crate::backend::ProviderDraftAuthMessage::CopilotDeviceStarted {
                    user_code: "ABCD-EFGH".to_string(),
                }
            )),
            "已开始 Copilot 设备登录。打开显示的验证 URL，输入代码 ABCD-EFGH，然后按 p。"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_draft_auth_action_message(
                &i18n,
                &crate::backend::ProviderDraftAuthMessage::OpenaiPending,
            )),
            "OpenAI 设备登录仍在等待中。先完成验证步骤，再按一次 p。"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_save_result_message(
                &i18n,
                &crate::backend::ProviderStudioSaveResult::AdapterMatchesSaved {
                    provider_id: "demo".to_string(),
                    adapter_id: "openai".to_string(),
                    listed_model_count: 4,
                    matched_model_count: 3,
                }
            )),
            "已保存 demo/openai，共 4 个列出模型，其中 3 个匹配 catalog。"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_draft_auth_error_message(
                &i18n,
                &crate::backend::ProviderDraftAuthError::RequiredField(
                    crate::backend::ProviderDraftAuthField::CallbackUrl,
                ),
            )),
            "Callback URL 为必填项"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_draft_auth_error_message(
                &i18n,
                &crate::backend::ProviderDraftAuthError::StartBrowserAuthFirst,
            )),
            "请先用“开始认证”或 o 启动浏览器认证"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_save_error_message(
                &i18n,
                &crate::backend::ProviderStudioSaveError::Validation(
                    crate::backend::ProviderStudioSaveValidationError::FieldRequired(
                        crate::backend::ProviderStudioSaveField::DefaultAdapter,
                    ),
                ),
            )),
            "默认 Adapter 为必填项"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_save_error_message(
                &i18n,
                &crate::backend::ProviderStudioSaveError::ProviderModelConfigMustBeObject,
            )),
            "provider model config 必须是一个 JSON object"
        );
    }

    #[test]
    fn provider_studio_auth_actions_focus_missing_browser_fields() {
        let i18n = I18n::resolve(Some("en-US"), None);
        let mut draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "demo".to_string(),
            auth_kind: ProviderDraftAuthKind::Credential(Some(CredentialIssuer::Gitlab)),
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        };
        draft.normalize_shape();
        draft.auth.instance_url.clear();
        draft.credential_drafts.gitlab.redirect_uri.clear();
        let dialog = provider_studio_test_overlay(draft);

        assert_eq!(
            sanitize_terminal_text(&provider_studio_start_auth_summary(&i18n, &dialog)),
            "set Instance URL"
        );
        assert_eq!(
            provider_studio_preferred_detail_field_index(&dialog),
            provider_studio_detail_fields(&dialog)
                .iter()
                .position(|field| *field == ProviderStudioField::InstanceUrl)
                .expect("instance field")
        );
    }

    #[test]
    fn provider_studio_pending_browser_auth_guides_callback_completion() {
        let i18n = I18n::resolve(Some("en-US"), None);
        let mut draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "demo".to_string(),
            auth_kind: ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)),
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        };
        draft.normalize_shape();
        draft.credential_drafts.openai_chatgpt.login_kind =
            crate::backend::ProviderDraftInteractiveLoginKind::Browser;
        draft.credential_drafts.openai_chatgpt.redirect_uri =
            "http://localhost:1455/auth/callback".to_string();
        draft.credential_drafts.openai_chatgpt.browser =
            Some(crate::backend::ProviderBrowserAuthSessionDraft {
                authorize_url: "https://example.com/authorize".to_string(),
                display_url: Some("https://example.com/authorize?...".to_string()),
                state: "state-123456".to_string(),
                pkce_verifier: "verifier".to_string(),
            });
        let dialog = provider_studio_test_overlay(draft);

        assert_eq!(
            sanitize_terminal_text(&provider_studio_start_auth_summary(&i18n, &dialog)),
            "open authorize URL · https://example.com/authorize?..."
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_continue_auth_summary(&i18n, &dialog)),
            "paste Callback URL · state state-123456"
        );
        assert_eq!(
            provider_studio_preferred_detail_field_index(&dialog),
            provider_studio_detail_fields(&dialog)
                .iter()
                .position(|field| *field == ProviderStudioField::CallbackUrl)
                .expect("callback field")
        );
    }

    #[test]
    fn provider_studio_openai_device_login_is_default_and_hides_browser_fields() {
        let i18n = I18n::resolve(Some("en-US"), None);
        let mut draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "demo".to_string(),
            auth_kind: ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)),
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        };
        draft.normalize_shape();
        let dialog = provider_studio_test_overlay(draft);

        assert_eq!(
            sanitize_terminal_text(&provider_studio_start_auth_summary(&i18n, &dialog)),
            "start device login"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_continue_auth_summary(&i18n, &dialog)),
            "start device login"
        );
        assert!(
            !provider_studio_detail_fields(&dialog).contains(&ProviderStudioField::RedirectUri)
        );
        assert!(
            !provider_studio_detail_fields(&dialog).contains(&ProviderStudioField::CallbackUrl)
        );
    }

    #[test]
    fn provider_studio_pending_openai_device_auth_guides_polling() {
        let i18n = I18n::resolve(Some("en-US"), None);
        let mut draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "demo".to_string(),
            auth_kind: ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)),
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        };
        draft.normalize_shape();
        draft.credential_drafts.openai_chatgpt.device =
            Some(crate::backend::ProviderDeviceAuthSessionDraft {
                verification_url: "https://chatgpt.com/auth/device".to_string(),
                display_url: Some("https://tinyurl.com/oai-device".to_string()),
                user_code: "ABCD-EFGH".to_string(),
                device_code: "device-code".to_string(),
                interval_seconds: 5,
            });
        let dialog = provider_studio_test_overlay(draft);

        assert_eq!(
            sanitize_terminal_text(&provider_studio_start_auth_summary(&i18n, &dialog)),
            "open verification URL · https://tinyurl.com/oai-device · code ABCD-EFGH"
        );
        assert_eq!(
            sanitize_terminal_text(&provider_studio_continue_auth_summary(&i18n, &dialog)),
            "poll now · poll every 5s · code ABCD-EFGH"
        );
        assert_eq!(
            provider_studio_auth_poll_interval(&dialog),
            Some(Duration::from_secs(5))
        );
    }

    #[test]
    fn provider_studio_pending_browser_auth_does_not_auto_poll() {
        let mut draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "demo".to_string(),
            auth_kind: ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)),
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        };
        draft.normalize_shape();
        draft.credential_drafts.openai_chatgpt.login_kind =
            crate::backend::ProviderDraftInteractiveLoginKind::Browser;
        draft.credential_drafts.openai_chatgpt.browser =
            Some(crate::backend::ProviderBrowserAuthSessionDraft {
                authorize_url: "https://example.com/authorize".to_string(),
                display_url: Some("https://example.com/authorize?...".to_string()),
                state: "state-123456".to_string(),
                pkce_verifier: "verifier".to_string(),
            });
        let dialog = provider_studio_test_overlay(draft);

        assert_eq!(provider_studio_auth_poll_interval(&dialog), None);
    }

    #[test]
    fn provider_studio_visible_fields_hide_auth_status_row() {
        let mut draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "demo".to_string(),
            auth_kind: ProviderDraftAuthKind::Credential(None),
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        };
        draft.normalize_shape();
        let dialog = provider_studio_test_overlay(draft);
        let fields = provider_studio_visible_fields(&dialog);

        assert_eq!(
            fields,
            vec![
                ProviderStudioField::ProviderId,
                ProviderStudioField::AuthMode,
                ProviderStudioField::AuthSubtype,
            ]
        );
    }

    #[test]
    fn provider_studio_cline_api_hides_base_url_and_stays_api_key_only() {
        let mut draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "cline".to_string(),
            auth_kind: ProviderDraftAuthKind::ClineApi,
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        };
        draft.normalize_shape();
        draft.auth.secret_source_kind = ProviderDraftSecretSourceKind::Inline;
        draft.auth.secret_source_value = "sk-test".to_string();
        let dialog = provider_studio_test_overlay(draft);

        assert_eq!(
            provider_studio_visible_fields(&dialog),
            vec![
                ProviderStudioField::ProviderId,
                ProviderStudioField::AuthMode,
                ProviderStudioField::AuthSubtype,
                ProviderStudioField::EditAuthDetailsAction,
                ProviderStudioField::DefaultAdapter,
                ProviderStudioField::DefaultModel,
            ]
        );
        assert_eq!(
            provider_studio_detail_fields(&dialog),
            vec![
                ProviderStudioField::ApiKeySource,
                ProviderStudioField::ApiKeyValue
            ]
        );
        assert!(provider_studio_auth_is_configured(&dialog));
        assert!(!provider_studio_field_editable(
            &dialog,
            ProviderStudioField::BaseUrl
        ));
    }

    #[test]
    fn provider_studio_single_adapter_does_not_set_defaults_without_user_choice() {
        let mut draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "demo".to_string(),
            auth_kind: ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)),
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        };
        draft.normalize_shape();
        let mut dialog = provider_studio_test_overlay(draft);
        dialog.adapter_candidate_ids =
            provider_studio_candidate_adapter_ids(&dialog.draft, BTreeSet::new());

        provider_studio_ensure_default_selection(&mut dialog);

        assert_eq!(
            dialog.selected_adapter_ids,
            BTreeSet::from(["openai".to_string()])
        );
        assert!(dialog.draft.default_adapter.is_empty());
        assert!(dialog.draft.default_model.is_empty());
    }

    #[test]
    fn provider_model_config_draft_writes_native_tools_on_model() {
        let mut draft = provider_model_config_draft_from_overlay(
            "gpt-5",
            agena::config::ProviderModelOverlay::default(),
        );
        draft.native_tools_preset = ProviderNativeToolsPreset::OpenAiHostedDefaults;

        let (model_id, value) =
            provider_model_config_draft_to_model_value(&draft).expect("model value");

        assert_eq!(model_id, "gpt-5");
        assert_eq!(
            value,
            json!({
                "native_tools": {
                    "enabled": true,
                    "routes": {
                        "web_search": "provider_hosted",
                        "image_generation": "provider_hosted"
                    }
                }
            })
        );
    }

    #[test]
    fn provider_model_editor_suggests_chatgpt_native_tools_when_missing() {
        let mut provider_draft = ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "oai".to_string(),
            auth_kind: ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt)),
            auth: Default::default(),
            credential_drafts: Default::default(),
            default_adapter: "openai".to_string(),
            default_model: String::new(),
        };
        provider_draft.normalize_shape();
        let mut draft = provider_model_config_draft_from_overlay(
            "gpt-5",
            agena::config::ProviderModelOverlay::default(),
        );
        apply_provider_model_config_native_tools_suggestion(
            &provider_draft,
            "openai",
            false,
            &mut draft,
        );

        assert_eq!(
            draft.native_tools_preset,
            ProviderNativeToolsPreset::OpenAiHostedDefaults
        );
    }

    #[test]
    fn provider_model_config_draft_preserves_custom_native_tools() {
        let draft = provider_model_config_draft_from_value(
            "claude",
            json!({
                "native_tools": {
                    "enabled": true,
                    "routes": {
                        "web_search": "plugin"
                    }
                }
            }),
        )
        .expect("draft");

        assert_eq!(draft.native_tools_preset, ProviderNativeToolsPreset::Custom);

        let (_, value) = provider_model_config_draft_to_model_value(&draft).expect("model value");
        assert_eq!(
            value
                .pointer("/native_tools/routes/web_search")
                .and_then(JsonValue::as_str),
            Some("plugin")
        );
    }

    #[test]
    fn timeline_items_follow_locale_and_preserve_raw_kind_search() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let record = DomainEvent {
            meta: agena::event::EventMeta {
                id: uuid::Uuid::nil(),
                seq_global: 7,
                seq_session: Some(7),
                session_id: Some(1),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: agena::event::envelope::ENVELOPE_SCHEMA_VERSION,
            },
            kind: AgenaSessionEvent::ExecutionStarted(agena::event::ExecutionStartedEvent {
                session_id: 1,
                ts_ms: 1_717_000_000_000,
            }),
        };

        let item = build_timeline_item(&i18n, &record);

        assert!(item.summary.contains("执行开始"));
        assert!(item.copy_text.contains("会话 ID"));
        assert!(item.copy_text.contains("事件类型"));
        assert!(item.search_text.contains("execution_started"));
    }

    #[test]
    fn session_workflow_state_labels_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);

        assert_eq!(
            ui_text::session_workflow_state_label(
                &i18n,
                &session_execution_resource(SessionRunState::AwaitingModel, false),
            ),
            "等待模型"
        );
        assert_eq!(
            ui_text::session_workflow_state_label(
                &i18n,
                &session_execution_resource(SessionRunState::Idle, true),
            ),
            "已阻塞"
        );
        assert_eq!(
            ui_text::session_workflow_state_label(
                &i18n,
                &session_execution_resource(SessionRunState::Idle, false),
            ),
            "空闲"
        );

        let mut execution = session_execution_resource(SessionRunState::Idle, false);
        execution.pending_interactive_requests = vec![PendingInteractiveRequest::Permission {
            request: PermissionRequest {
                action: PermissionAction::Tool {
                    tool_name: "read".to_string(),
                    qualifier: None,
                },
                ..permission_request("perm-plan")
            },
        }];
        assert_eq!(
            ui_text::session_workflow_state_label(&i18n, &execution),
            "等待权限审批"
        );
    }

    #[test]
    fn agent_editor_messages_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);

        assert_eq!(editor_save_footer(&i18n, false), "输入以编辑");
        assert_eq!(editor_save_footer(&i18n, true), "Ctrl+S 保存");
        assert_eq!(
            agent_studio_field_label(&i18n, AgentStudioField::DefaultModel),
            "默认 Model"
        );
    }

    #[test]
    fn permission_studio_atomic_mode_edits_preserve_sibling_sections() {
        let i18n = I18n::english();
        let mut permission = PermissionConfig {
            path: Some(PathPermissionConfig {
                workspace: Some(PathAccessModes {
                    read: Some(PermissionMode::Ask),
                    write: Some(PermissionMode::Deny),
                }),
                external: Some(PathAccessModes {
                    read: Some(PermissionMode::Deny),
                    write: Some(PermissionMode::Deny),
                }),
                rules: IndexMap::from([(
                    "tmp/**".to_string(),
                    PathAccessRuleConfig::Shorthand("allow".to_string()),
                )]),
            }),
            ..PermissionConfig::default()
        };

        apply_permission_studio_mode_input(
            &i18n,
            &mut permission,
            &PermissionStudioModeTarget::PathWorkspaceRead,
            "allow",
        )
        .expect("expected atomic workspace update");

        let path = permission.path.expect("path section should remain present");
        assert_eq!(
            path.workspace,
            Some(PathAccessModes {
                read: Some(PermissionMode::Allow),
                write: Some(PermissionMode::Deny),
            })
        );
        assert_eq!(
            path.external,
            Some(PathAccessModes {
                read: Some(PermissionMode::Deny),
                write: Some(PermissionMode::Deny),
            })
        );
        assert_eq!(
            path.rules.get("tmp/**"),
            Some(&PathAccessRuleConfig::Shorthand("allow".to_string()))
        );
    }

    #[test]
    fn permission_studio_tool_default_mode_edits_are_structured() {
        let i18n = I18n::english();
        let mut permission = PermissionConfig::default();

        apply_permission_studio_mode_input(
            &i18n,
            &mut permission,
            &PermissionStudioModeTarget::ToolDefault,
            "ask",
        )
        .expect("expected tool default update");

        assert_eq!(
            permission.tools.and_then(|tools| tools.default),
            Some(PermissionMode::Ask)
        );
    }

    #[test]
    fn permission_studio_tool_qualifier_edits_preserve_fallback() {
        let mut permission = PermissionConfig {
            tools: Some(ToolPermissionConfig {
                rules: BTreeMap::from([(
                    "bash".to_string(),
                    ToolPermissionRules::Mode(PermissionMode::Ask),
                )]),
                ..ToolPermissionConfig::default()
            }),
            ..PermissionConfig::default()
        };

        add_tool_qualifier_rule(&mut permission, "bash", "npm test", PermissionMode::Deny);

        assert_eq!(
            tool_rule_fallback_mode(&permission, "bash"),
            Some(PermissionMode::Ask)
        );
        assert_eq!(
            tool_qualifier_mode(&permission, "bash", "npm test"),
            Some(PermissionMode::Deny)
        );

        remove_tool_qualifier_rule(&mut permission, "bash", "npm test");

        assert_eq!(
            tool_rule_fallback_mode(&permission, "bash"),
            Some(PermissionMode::Ask)
        );
        assert!(tool_qualifier_rules(&permission, "bash").is_empty());
    }

    #[test]
    fn settings_field_messages_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);

        assert_eq!(
            settings_field_display_description(&i18n, SETTINGS_FIELDS[0]),
            "没有会话覆盖时使用的 provider、adapter、model、thinking 和 speed 默认值"
        );
        assert_eq!(
            settings_field_display_label(&i18n, SETTINGS_FIELDS[0]),
            "默认"
        );
        assert_eq!(
            sanitize_terminal_text(&format_setting_field_summary(&i18n, &json!(1), &json!(2))),
            "文件：1 / 生效：2"
        );

        let bool_field = SETTINGS_FIELDS
            .iter()
            .copied()
            .find(|field| matches!(field.kind, SettingsFieldKind::Bool))
            .expect("settings should expose at least one bool field");
        let error = parse_settings_field_input(&i18n, bool_field, "maybe")
            .expect_err("expected localized settings parse error");
        assert_eq!(
            error,
            i18n.text_args(
                "settings-field-parse-bool",
                &crate::fl_args!("field" => bool_field.path),
            )
        );
    }

    #[test]
    fn settings_plugins_section_uses_only_navigation_entries() {
        let i18n = I18n::english();
        let sources = ConfigJsonSources {
            config_path: PathBuf::from("/tmp/agena-config.json"),
            config_found: true,
            applied_layers: Vec::new(),
            file: json!({}),
            effective: json!({
                "runtime": {
                    "reload": { "enabled": true },
                    "providers": {
                        "stream_replay": {
                            "max_retries_after_output": 2,
                            "max_tracked_events": 2048
                        }
                    }
                }
            }),
        };

        let all_paths = SETTINGS_FIELDS
            .iter()
            .map(|field| field.path)
            .collect::<Vec<_>>();
        assert!(
            !all_paths.iter().any(|path| path.starts_with("memory.")),
            "settings fields must not expose removed top-level memory paths: {all_paths:?}"
        );
        assert!(
            !all_paths
                .iter()
                .any(|path| path.starts_with("plugins.list.")),
            "per-plugin config must be edited through the plugin workbench, not global settings fields: {all_paths:?}"
        );

        let runtime_items =
            settings_studio_field_items(&i18n, &sources, SettingsStudioSectionId::ConfigRuntime);
        assert!(runtime_items.iter().all(|item| matches!(
            &item.action,
            SettingsPickerAction::EditField(field) if field.path.starts_with("runtime.")
        )));
        assert!(runtime_items
            .iter()
            .any(|item| item.label == "Replay Retries After Output"
                && matches!(
                    &item.action,
                    SettingsPickerAction::EditField(field)
                        if field.path == "runtime.providers.stream_replay.max_retries_after_output"
                )));

        let plugin_items =
            settings_studio_field_items(&i18n, &sources, SettingsStudioSectionId::ConfigPlugins);
        assert!(
            plugin_items.is_empty(),
            "plugin-specific config fields should not be hard-coded into the global settings section: {plugin_items:?}"
        );
    }

    #[test]
    fn settings_field_items_expose_config_source_rows() {
        let i18n = I18n::english();
        let sources = ConfigJsonSources {
            config_path: PathBuf::from("/tmp/agena-config.json"),
            config_found: true,
            applied_layers: vec![
                "built-in defaults".to_string(),
                "file:/tmp/agena-config.json".to_string(),
                "process environment".to_string(),
            ],
            file: json!({
                "runtime": {
                    "reload": {
                        "enabled": false
                    }
                }
            }),
            effective: json!({
                "runtime": {
                    "reload": {
                        "enabled": true
                    }
                }
            }),
        };

        let items =
            settings_studio_field_items(&i18n, &sources, SettingsStudioSectionId::ConfigRuntime);
        let reload = items
            .iter()
            .find(|item| item.path.as_deref() == Some("runtime.reload.enabled"))
            .expect("runtime reload setting should be listed");

        assert_eq!(reload.current_value.as_deref(), Some("false"));
        assert_eq!(reload.effective_value.as_deref(), Some("true"));
        let rendered_rows = sanitize_terminal_text(
            reload
                .source_rows
                .iter()
                .map(|row| format!("{}={}", row.label, row.value))
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        );
        assert!(rendered_rows.contains("Config file=/tmp/agena-config.json (found)"));
        assert!(rendered_rows.contains("File value=false"));
        assert!(rendered_rows.contains("Effective value=true"));
        assert!(
            rendered_rows.contains("Writes to=runtime.reload.enabled -> /tmp/agena-config.json")
        );
        assert!(rendered_rows.contains(
            "Active layers=built-in defaults -> file:/tmp/agena-config.json -> process environment"
        ));
    }

    #[test]
    fn settings_plugin_policy_items_render_as_navigation_entries() {
        let i18n = I18n::english();
        let sources = ConfigJsonSources {
            config_path: PathBuf::from("/tmp/agena-config.json"),
            config_found: true,
            applied_layers: Vec::new(),
            file: json!({}),
            effective: json!({
                "plugins": {
                    "policy": {
                        "tool_presentation": {
                            "default_mode": "brief",
                            "plugins": { "agena.web": "detailed" },
                            "tools": { "agena.web/fetch": "brief" }
                        },
                        "ui_presentation": {
                            "default_mode": "summary",
                            "plugins": { "agena.web": "detailed" },
                            "tools": { "agena.web/open": "summary" }
                        }
                    }
                }
            }),
        };

        let items = settings_studio_plugin_items(&i18n, &sources);
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();

        assert!(labels.contains(&"Plugin Policy Studio"));
        assert!(labels.contains(&"Plugin Config Workbench"));
        assert!(!labels.contains(&"Tool Description Mode"));
        assert!(!labels.contains(&"Plugin UI Display Mode"));
        let expected_value = ui_text::t(&i18n, "value-open");
        for item in &items {
            assert_eq!(item.value, expected_value);
            assert!(item.current_value.is_none());
            assert!(item.effective_value.is_none());
            assert!(item.source_rows.is_empty());
            assert!(item.path.is_none());
        }
    }

    #[test]
    fn current_settings_write_paths_validate_against_config_schema() {
        let temp = tempdir().expect("temp config dir");
        let config_path = temp.path().join("config.json");
        fs::write(&config_path, "{}\n").expect("write empty config");

        for (path, value) in [
            ("agents.default", json!("build")),
            (PLUGIN_TOOL_PRESENTATION_DEFAULT_MODE_PATH, json!("brief")),
            (PLUGIN_UI_PRESENTATION_DEFAULT_MODE_PATH, json!("summary")),
            ("session.compaction.auto", json!(true)),
            ("tracing.database", json!("error")),
            (
                "runtime.providers.stream_replay.max_tracked_events",
                json!(2048),
            ),
        ] {
            agena::config::set_file_setting(
                config_path.clone(),
                agena::config::ConfigSettingsSetInput {
                    path: path.to_owned(),
                    value,
                    options: agena::config::ConfigSettingsEditOptions {
                        dry_run: false,
                        validate: true,
                        reload: false,
                    },
                },
            )
            .unwrap_or_else(|error| panic!("{path} should validate: {error}"));
        }
    }

    #[test]
    fn settings_complex_collections_are_navigation_entries() {
        let i18n = I18n::english();
        let sources = ConfigJsonSources {
            config_path: PathBuf::from("/tmp/agena-config.json"),
            config_found: true,
            applied_layers: Vec::new(),
            file: json!({}),
            effective: json!({
                "agents": {
                    "default": "build"
                },
                "providers": {
                    "default": "openai"
                }
            }),
        };

        let mut agent_items =
            settings_studio_field_items(&i18n, &sources, SettingsStudioSectionId::ConfigAgents);
        agent_items.push(settings_studio_agent_browser_item(&i18n, 8, Some("build")));
        assert_eq!(
            agent_items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Default Agent", "Agent List"]
        );
        assert!(matches!(
            &agent_items[1].action,
            SettingsPickerAction::OpenAgentList
        ));

        let provider = ProviderSummaryResource {
            provider_id: "openai".to_string(),
            defaults: agena_api::resource::ProviderDefaultsResource {
                adapter: Some("responses".to_string()),
                model: "gpt-5".to_string(),
                thinking_mode: Some("thinking-high".to_string()),
                speed_mode: Some("speed-fast".to_string()),
                verbosity: None,
                parallel_tool_calls: None,
            },
            adapters: Vec::new(),
            native_tools: None,
        };
        let provider_items = settings_studio_provider_items(&i18n, &sources, &[provider]);
        assert_eq!(
            provider_items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Default", "Provider List"]
        );
        assert!(matches!(
            &provider_items[0].action,
            SettingsPickerAction::OpenProviderDefaultWizard
        ));
        assert_eq!(
            sanitize_terminal_text(provider_items[0].value.as_str()),
            "openai / responses / gpt-5 · think high · speed fast"
        );
        assert!(matches!(
            &provider_items[1].action,
            SettingsPickerAction::OpenProviderList
        ));
    }

    #[test]
    fn permission_settings_items_expose_effective_session_agent_and_global_layers() {
        let i18n = I18n::english();
        let sources = ConfigJsonSources {
            config_path: PathBuf::from("/tmp/agena-config.json"),
            config_found: true,
            applied_layers: vec!["built-in defaults".to_string()],
            file: json!({}),
            effective: json!({}),
        };
        let global = PermissionConfig {
            network: Some(NetworkPermissionConfig {
                internet: Some(PermissionMode::Ask),
                ..Default::default()
            }),
            ..Default::default()
        };
        let agent = PermissionConfig {
            network: Some(NetworkPermissionConfig {
                private: Some(PermissionMode::Allow),
                ..Default::default()
            }),
            ..Default::default()
        };
        let session_override = PermissionConfig {
            network: Some(NetworkPermissionConfig {
                internet: Some(PermissionMode::Deny),
                ..Default::default()
            }),
            ..Default::default()
        };
        let effective = global.merged_with(&agent).merged_with(&session_override);
        let session = SessionPermissionStudioState {
            session_id: 42,
            session_title: "work".to_string(),
            agent_name: Some("build".to_string()),
            agent_permission: Some(agent),
            permission: session_override,
            effective_permission: effective,
        };

        let items = settings_studio_permission_items(
            &i18n,
            &sources,
            &PermissionConfig::default(),
            &global,
            Some(&session),
        );

        assert_eq!(
            items
                .iter()
                .map(|item| sanitize_terminal_text(item.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                "Effective Permission".to_string(),
                "Current Session Permission".to_string(),
                "Agent Permission · build".to_string(),
                "Global Permission".to_string()
            ]
        );
        assert!(matches!(
            &items[0].action,
            SettingsPickerAction::OpenSessionEffectivePermissionView(42)
        ));
        assert!(matches!(
            &items[1].action,
            SettingsPickerAction::OpenCurrentSessionPermissionWorkbench
        ));
        assert!(matches!(
            &items[2].action,
            SettingsPickerAction::OpenAgentPermissionWorkbench(agent) if agent == "build"
        ));
        assert!(matches!(
            &items[3].action,
            SettingsPickerAction::OpenGlobalPermissionWorkbench
        ));
        let effective_rows = sanitize_terminal_text(
            items[0]
                .source_rows
                .iter()
                .map(|row| format!("{}={}", row.label, row.value))
                .collect::<Vec<_>>()
                .join("\n")
                .as_str(),
        );
        assert!(effective_rows.contains("Global="));
        assert!(effective_rows.contains("Agent build="));
        assert!(effective_rows.contains("Session="));
        assert!(effective_rows.contains("Effective="));
    }

    #[test]
    fn agent_and_provider_lists_expose_create_rows() {
        let i18n = I18n::english();
        let agent_items = agent_list_items(
            &i18n,
            vec![AgentDescriptor {
                name: "build".to_string(),
                description: "Primary coding agent".to_string(),
                permission: Default::default(),
                defaults: Default::default(),
                scope: agena::agents::AgentScope::Project,
                source_path: None,
            }],
            Some("build"),
            &HashSet::from(["build".to_string()]),
        );
        assert_eq!(agent_items[0].label, "+ New Agent");
        assert!(matches!(agent_items[0].value, PickerValue::AgentCreate));
        assert_eq!(agent_items[1].label, "build");

        let provider = ProviderSummaryResource {
            provider_id: "github".to_string(),
            defaults: agena_api::resource::ProviderDefaultsResource {
                adapter: Some("copilot".to_string()),
                model: "gpt-5".to_string(),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                parallel_tool_calls: None,
            },
            adapters: vec![agena_api::resource::ProviderAdapterSummaryResource {
                adapter_id: "copilot".to_string(),
                enabled: true,
                configured_model_count: 1,
            }],
            native_tools: None,
        };
        let provider_create = provider_list_create_item(&i18n);
        assert_eq!(provider_create.label, "+ New Provider");
        assert!(matches!(provider_create.value, PickerValue::ProviderCreate));
        assert_eq!(
            sanitize_terminal_text(&i18n_provider_list_detail(&i18n, &provider)),
            "copilot / gpt-5 · 1 adapters"
        );
    }

    #[test]
    fn agent_studio_items_keep_default_agent_as_settings_field() {
        let i18n = I18n::english();
        let profile = AgentProfile {
            name: "build".to_string(),
            frontmatter: AgentFrontmatter {
                description: "Primary coding agent".to_string(),
                ..Default::default()
            },
            prompt: "Implement the request.".to_string(),
            source_path: None,
            scope: AgentScope::Default,
        };

        let items = agent_studio_items(&i18n, &profile, AgentProfileStorage::BuiltIn);
        let labels = items
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();

        assert!(labels.contains(&"Permission Policy"));
        assert!(labels.contains(&"Source"));
        assert!(
            !labels.contains(&"agents.default"),
            "agents.default belongs to Settings, not the Agent Studio field list"
        );
    }

    #[test]
    fn agent_permission_detail_lines_render_structured_summary() {
        let i18n = I18n::english();
        let permission = PermissionConfig {
            path: Some(PathPermissionConfig {
                workspace: Some(PathAccessModes {
                    read: Some(PermissionMode::Allow),
                    write: Some(PermissionMode::Ask),
                }),
                ..Default::default()
            }),
            tools: Some(ToolPermissionConfig {
                names: BTreeMap::from([
                    ("agent".to_string(), PermissionMode::Allow),
                    ("workflow".to_string(), PermissionMode::Ask),
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let lines = agent_permission_document_detail_lines(&i18n, &permission);
        let rendered = agena_tui_components::build_detail_text_plain(
            &lines,
            &DetailTextSpec::with_label_width(14),
        );

        assert!(rendered.contains("Path Section"));
        assert!(rendered.contains("workspace"));
        assert!(rendered.contains("read allow"));
        assert!(rendered.contains("write ask"));
        assert!(rendered.contains("Tool Section"));
        assert!(rendered.contains("Tool Names"));
        assert!(rendered.contains("agent"));
        assert!(
            !rendered.contains('{') && !rendered.contains("\"agent\""),
            "permission detail should be TUI rows, not raw JSON: {rendered}"
        );
    }

    #[test]
    fn agent_storage_distinguishes_builtin_config_markdown_and_runtime() {
        let i18n = I18n::english();
        let mut profile = AgentProfile {
            name: "build".to_string(),
            frontmatter: Default::default(),
            prompt: String::new(),
            source_path: None,
            scope: AgentScope::Default,
        };
        assert_eq!(
            agent_profile_storage(&profile, false),
            AgentProfileStorage::BuiltIn
        );
        assert_eq!(
            agent_profile_source_label_localized(&i18n, &profile, AgentProfileStorage::BuiltIn),
            "built-in defaults"
        );

        profile.scope = AgentScope::Project;
        assert_eq!(
            agent_profile_storage(&profile, true),
            AgentProfileStorage::Config
        );

        profile.source_path = Some(PathBuf::from("/tmp/custom.md"));
        assert_eq!(
            agent_profile_storage(&profile, true),
            AgentProfileStorage::Markdown
        );
        assert!(AgentProfileStorage::Markdown.editable());

        profile.source_path = None;
        assert_eq!(
            agent_profile_storage(&profile, false),
            AgentProfileStorage::Runtime
        );
        assert!(!AgentProfileStorage::Runtime.editable());
    }

    #[test]
    fn agent_markdown_document_round_trips_frontmatter_and_prompt() {
        let mut frontmatter = AgentFrontmatter {
            description: "Custom agent".to_string(),
            ..Default::default()
        };
        frontmatter.defaults.provider = Some("github".to_string());
        frontmatter.defaults.model = Some("gpt-5".to_string());

        let text = agent_markdown_document(&frontmatter, "Use the repo context.\n").unwrap();
        let parsed = AgentProfile::from_raw(&text, "custom", AgentScope::User).unwrap();
        assert_eq!(parsed.frontmatter.description, "Custom agent");
        assert_eq!(
            parsed.frontmatter.defaults.provider.as_deref(),
            Some("github")
        );
        assert_eq!(parsed.frontmatter.defaults.model.as_deref(), Some("gpt-5"));
        assert_eq!(parsed.prompt, "Use the repo context.\n");

        assert_eq!(
            agent_markdown_document(&AgentFrontmatter::default(), "Prompt only").unwrap(),
            "Prompt only\n"
        );
    }

    #[test]
    fn transcript_node_flash_messages_follow_locale() {
        let i18n = I18n::resolve(Some("zh-CN"), None);
        let kind = transcript_node_kind_label(&i18n, TranscriptNodeKind::Tool);

        assert_eq!(kind, "tool 输出");
        assert_eq!(
            sanitize_terminal_text(&i18n.text_args(
                "flash-transcript-node-copied",
                &crate::fl_args!("kind" => kind.clone()),
            )),
            "已复制当前tool 输出"
        );
        assert_eq!(
            sanitize_terminal_text(&i18n.text_args(
                "flash-transcript-node-collapsed",
                &crate::fl_args!("kind" => kind),
            )),
            "已收起tool 输出"
        );
    }
}
