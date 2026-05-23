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
    agents::{AgentDescriptor, AgentProfile},
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
        RunOptions, SessionExecutionResource, SessionResource, SessionRunState,
        SessionUsageResource,
    },
};
use anyhow::Result;
use chrono::{DateTime, Local, Utc};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::{
    Frame, Terminal,
    backend::Backend as RatatuiBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::interval,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::backend::{
    Backend, ConfigJsonSources, InspectorRow, LiveEvent, ProviderConfigDraft,
    ProviderDraftAdapterRule, ProviderDraftAuthKind, SessionRefresh,
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
    ModelCatalogEntryResource, ModelCatalogListResponse, ModelCatalogResponse,
};

mod provider_studio;
mod transcript_view;
mod view;

use self::provider_studio::*;

use self::transcript_view::{
    render_message, render_message_detailed, render_transcript_export_markdown,
    rewind_message_preview, sanitize_terminal_text,
};

const MESSAGE_PAGE_SIZE: u64 = 40;
const TIMELINE_EVENT_LIMIT: u64 = 200;
const PLUGIN_INSPECTOR_LOG_LIMIT: usize = 20;
const UI_TICK_MS: u64 = 32;
const REFRESH_INTERVAL_MS: u64 = 250;
const DRAFT_PERSIST_INTERVAL_MS: u64 = 250;
const WORD_SEPARATORS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";
const PASTE_BURST_MIN_CHARS: u16 = 3;
const PASTE_BURST_CHAR_INTERVAL_MS: u64 = 8;
const PASTE_ENTER_SUPPRESS_WINDOW_MS: u64 = 120;
const LARGE_PASTE_CHAR_THRESHOLD: usize = 1000;
const TOOL_CARD_PREVIEW_LINES: usize = 8;
const TOOL_CARD_PREVIEW_CHARS: usize = 2_500;
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

const SETTINGS_FIELDS: [SettingsFieldSpec; 21] = [
    SettingsFieldSpec {
        path: "default.provider",
        description: "default provider id",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        path: "default.adapter",
        description: "default adapter for the default provider",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        path: "default.model",
        description: "default configured model for the default provider adapter",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        path: "default.agent",
        description: "default agent name",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        path: "ui.locale",
        description: "ui locale",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        path: "telemetry.enabled",
        description: "telemetry exporter enabled",
        kind: SettingsFieldKind::Bool,
    },
    SettingsFieldSpec {
        path: "telemetry.service_name",
        description: "telemetry service name",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        path: "telemetry.otlp_endpoint",
        description: "telemetry OTLP endpoint",
        kind: SettingsFieldKind::String,
    },
    SettingsFieldSpec {
        path: "runtime.provider_http.timeout_secs",
        description: "provider HTTP timeout seconds",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.provider_http.connect_timeout_secs",
        description: "provider HTTP connect timeout seconds",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.request_retry.max_retries",
        description: "provider request retry count",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.request_retry.base_delay_ms",
        description: "provider retry base delay ms",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.request_retry.max_delay_ms",
        description: "provider retry max delay ms",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.reload.enabled",
        description: "runtime config reloader enabled",
        kind: SettingsFieldKind::Bool,
    },
    SettingsFieldSpec {
        path: "runtime.reload.poll_interval_secs",
        description: "runtime reload poll interval seconds",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.model_catalog.cache_max_age_secs",
        description: "model catalog cache age seconds",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.session_cache.max_sessions",
        description: "session cache max entries",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.session_cache.ttl_secs",
        description: "session cache ttl seconds",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "runtime.session_cache.max_bytes",
        description: "session cache max bytes",
        kind: SettingsFieldKind::Integer,
    },
    SettingsFieldSpec {
        path: "plugins.list.\"agena.memory\".options.project_instructions.enabled",
        description: "project instructions memory enabled",
        kind: SettingsFieldKind::Bool,
    },
    SettingsFieldSpec {
        path: "plugins.list.\"agena.memory\".options.project_instructions.include_global",
        description: "project instructions include global files",
        kind: SettingsFieldKind::Bool,
    },
];

const RUNTIME_SETTINGS: [RuntimeSettingSpec; 7] = [
    RuntimeSettingSpec {
        id: RuntimeSettingId::ThinkingMode,
        label: "Thinking Mode",
        description: "current-session thinking mode override",
        kind: SettingsFieldKind::String,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::SpeedMode,
        label: "Speed Mode",
        description: "current-session speed mode override",
        kind: SettingsFieldKind::String,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::Verbosity,
        label: "Verbosity",
        description: "current-session verbosity override",
        kind: SettingsFieldKind::String,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::ParallelToolCalls,
        label: "Parallel Tool Calls",
        description: "current-session parallel tool calls override",
        kind: SettingsFieldKind::Bool,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::Temperature,
        label: "Temperature",
        description: "current-session temperature override",
        kind: SettingsFieldKind::Float,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::MaxOutput,
        label: "Max Output Tokens",
        description: "current-session max output token override",
        kind: SettingsFieldKind::Integer,
    },
    RuntimeSettingSpec {
        id: RuntimeSettingId::System,
        label: "System Prompt",
        description: "current-session system prompt override",
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
    entries: Vec<String>,
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
        result: UiResult<crate::backend::ProviderDraftAuthActionResult>,
    },
    SessionModelChooserLoaded {
        result: UiResult<Vec<SessionModelChoiceItem>>,
    },
    ProviderStudioSaved {
        provider_id: String,
        result: UiResult<String>,
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
    /// Replaces the legacy 250ms polling tick: callers receive each domain
    /// event in real time, with a hint about whether a refresh is needed.
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
    SettingsValueEdit(SettingsValueEditOverlay),
    RuntimeSettingEdit(RuntimeSettingEditOverlay),
    Choice(ChoiceOverlay),
    PermissionRuleEdit(PermissionRuleEditOverlay),
    FileAttach(FileAttachOverlay),
    Permission(PermissionOverlay),
    UserInputReply(UserInputOverlay),
    Confirm(ConfirmOverlay),
    SessionSearch(SessionSearchOverlay),
    Picker(PickerOverlay),
    SessionModelChooser(SessionModelChooserOverlay),
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
    AgentPermissionStudio(AgentPermissionStudioOverlay),
    SessionSearch(SessionSearchOverlay),
    Picker(PickerOverlay),
    SessionModelChooser(SessionModelChooserOverlay),
    Timeline(TimelineOverlay),
    PluginInspector(PluginInspectorOverlay),
    ProviderStudio(Box<ProviderStudioOverlay>),
    ModelCatalogStudio(ModelCatalogStudioOverlay),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogHost {
    Route,
    Overlay,
}

#[derive(Debug, Clone)]
struct LineInputOverlay {
    title: String,
    prompt: String,
    input: Editor,
}

#[derive(Debug, Clone, Default)]
struct HelpOverlay {
    scroll: u16,
}

#[derive(Debug, Clone)]
struct SettingsStudioOverlay {
    title: String,
    footer: String,
    sections: Vec<SettingsStudioSection>,
    selected_section: usize,
    selected_item: usize,
    focus: SettingsStudioFocus,
    default_agent_name: Option<String>,
    plugins_enabled: bool,
    plugins_default_mode: String,
}

#[derive(Debug, Clone)]
struct AgentStudioOverlay {
    title: String,
    footer: String,
    agent_name: String,
    profile: AgentProfile,
    editable: bool,
    default_agent_name: Option<String>,
    items: Vec<AgentStudioItem>,
    selected: usize,
    editor: Option<AgentStudioEditor>,
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
    ToggleHidden,
    SetDefault,
    OpenPermissionWorkbench,
    OpenSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentStudioField {
    Description,
    Prompt,
    Mode,
    Aliases,
    AllowedEntries,
    Temperature,
    MaxOutputTokens,
    Steps,
    DefaultProvider,
    DefaultAdapter,
    DefaultModel,
}

#[derive(Debug, Clone)]
struct AgentStudioEditor {
    title: String,
    prompt: String,
    footer: String,
    multiline: bool,
    input: Editor,
    action: AgentStudioEditorAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentStudioEditorAction {
    Field(AgentStudioField),
}

#[derive(Debug, Clone)]
struct AgentPermissionStudioOverlay {
    title: String,
    footer: String,
    agent_name: String,
    profile: AgentProfile,
    editable: bool,
    items: Vec<AgentPermissionStudioItem>,
    selected: usize,
    editor: Option<AgentPermissionStudioEditor>,
}

#[derive(Debug, Clone)]
struct AgentPermissionStudioItem {
    label: String,
    value: String,
    detail: String,
    action: AgentPermissionStudioAction,
}

#[derive(Debug, Clone)]
enum AgentPermissionStudioAction {
    Edit(AgentPermissionField),
    OpenSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentPermissionField {
    InheritPath,
    InheritNetwork,
    InheritEntries,
    PathConfig,
    NetworkConfig,
    EntryConfig,
    FullConfig,
}

#[derive(Debug, Clone)]
struct AgentPermissionStudioEditor {
    title: String,
    prompt: String,
    footer: String,
    multiline: bool,
    input: Editor,
    action: AgentPermissionStudioEditorAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentPermissionStudioEditorAction {
    Field(AgentPermissionField),
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
    action: SettingsPickerAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsStudioFocus {
    Navigation,
    Items,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsStudioSectionId {
    General,
    Runtime,
    Plugins,
    PluginEntries,
    Agents,
    Providers,
    ModelCatalog,
    Permissions,
    Files,
}

#[derive(Debug, Clone)]
struct SettingsValueEditOverlay {
    title: String,
    prompt: String,
    input: Editor,
    field: SettingsFieldSpec,
}

#[derive(Debug, Clone)]
struct RuntimeSettingEditOverlay {
    title: String,
    prompt: String,
    input: Editor,
    field: RuntimeSettingSpec,
}

#[derive(Debug, Clone)]
struct ChoiceOverlay {
    title: String,
    prompt: String,
    footer: String,
    empty_message: String,
    input: Editor,
    filter_query: String,
    all_items: Vec<ChoiceItem>,
    items: Vec<ChoiceItem>,
    selected: usize,
    allow_custom: bool,
    allow_clear: bool,
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
enum ChoiceOverlayAction {
    SettingsField(SettingsFieldSpec),
    RuntimeSetting(RuntimeSettingSpec),
    ProviderStudioField(ProviderStudioField),
}

#[derive(Debug, Clone)]
enum ChoiceRow {
    Clear,
    Custom(String),
    Item(ChoiceItem),
}

#[derive(Debug, Clone, Copy)]
struct SettingsFieldSpec {
    path: &'static str,
    description: &'static str,
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
    TogglePluginsEnabled,
    ToggleToolDescriptionMode,
    TogglePluginEntryDisabled {
        plugin_id: String,
        entry: JsonValue,
        disabled: bool,
    },
    OpenAgent(Box<AgentDescriptor>),
    OpenProviderWorkbench,
    OpenProviderWorkbenchFor(String),
    OpenModelCatalogWorkbench,
    OpenRuntimeProviderOverride,
    OpenRuntimeModelOverride,
    ClearRuntimeModelStack,
    OpenPermissionRules,
    OpenConfigFile,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeSettingSpec {
    id: RuntimeSettingId,
    label: &'static str,
    description: &'static str,
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
    title: String,
    prompt: String,
    input: Editor,
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
struct UserInputOverlay {
    session_id: i64,
    request: UserInputRequest,
    answers: BTreeMap<String, UserInputAnswerDraft>,
    selected_question: usize,
    selected_option: usize,
    screen: UserInputOverlayScreen,
    editing_custom: bool,
    custom_input: Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserInputOverlayScreen {
    Question,
    Review,
}

#[derive(Debug, Clone)]
struct PermissionOverlay {
    session_id: i64,
    request: PermissionRequest,
    selected: usize,
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
struct ConfirmOverlay {
    title: String,
    body_lines: Vec<String>,
    footer: String,
    action: ConfirmAction,
}

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
    ExitWorktree {
        session_id: i64,
        discard_changes: bool,
    },
}

#[derive(Debug, Clone)]
struct FileAttachOverlay {
    input: Editor,
    results: Vec<PathBuf>,
    selected: usize,
}

#[derive(Debug, Clone)]
struct TimelineOverlay {
    session_id: i64,
    title: String,
    prompt: String,
    empty_message: String,
    footer: String,
    input: Editor,
    all_items: Vec<TimelineItem>,
    items: Vec<TimelineItem>,
    selected: usize,
    loading: bool,
}

#[derive(Debug, Clone)]
struct TimelineItem {
    summary: String,
    detail: String,
    search_text: String,
    copy_text: String,
    linked_message_id: Option<i64>,
}

#[derive(Debug, Clone)]
struct PluginInspectorOverlay {
    title: String,
    prompt: String,
    empty_message: String,
    footer: String,
    input: Editor,
    all_items: Vec<PluginInspectorItem>,
    items: Vec<PluginInspectorItem>,
    selected: usize,
}

#[derive(Debug, Clone)]
struct PluginInspectorItem {
    plugin_id: String,
    summary: String,
    detail: String,
    logs: String,
    search_text: String,
    copy_text: String,
    state: agena::plugin::status::PluginRunState,
}

#[derive(Debug, Clone)]
struct ProviderStudioOverlay {
    title: String,
    footer: String,
    show_provider_list: bool,
    providers: Vec<ProviderStudioProviderRow>,
    selected_provider: usize,
    focus: ProviderStudioFocus,
    selected_field: usize,
    draft: ProviderConfigDraft,
    adapter_models: Vec<ProviderAdapterModelsResource>,
    configured_adapter_ids: BTreeSet<String>,
    adapter_candidate_ids: Vec<String>,
    selected_adapter: usize,
    selected_model: usize,
    adapter_selection_touched: bool,
    selected_adapter_ids: BTreeSet<String>,
    selected_model_keys: BTreeSet<String>,
    catalog_matches: BTreeMap<String, ModelCatalogEntryResource>,
    listing_adapter_models: bool,
    saving: bool,
    pending_adapter_models_key: Option<String>,
    pending_auth_key: Option<String>,
    detail_page: Option<ProviderStudioDetailPage>,
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
    CredentialIssuer,
    AuthStatus,
    StartAuthAction,
    ContinueAuthAction,
    EditAuthDetailsAction,
    BaseUrl,
    InstanceUrl,
    ApiKeyEnv,
    ApiKey,
    RedirectUri,
    CallbackUrl,
    RefreshToken,
    AccessToken,
    ExpiresAtMs,
    AccountId,
    EnterpriseDomain,
    Username,
    DisplayName,
    Email,
    AvatarUrl,
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
    selected_field: usize,
}

#[derive(Debug, Clone)]
struct ProviderStudioEditor {
    title: String,
    prompt: String,
    footer: String,
    multiline: bool,
    input: Editor,
    action: ProviderStudioEditorAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderStudioEditorAction {
    Field(ProviderStudioField),
    ModelJson {
        adapter_id: String,
        model_id: String,
    },
}

#[derive(Debug, Clone)]
struct ModelCatalogStudioOverlay {
    title: String,
    footer: String,
    query: String,
    items: Vec<ModelCatalogEntryResource>,
    summary: ModelCatalogResponse,
    total: usize,
    offset: usize,
    limit: usize,
    loading: bool,
    selected: usize,
    editor: Option<LineInputOverlay>,
}

#[derive(Debug, Clone)]
struct SessionSearchOverlay {
    title: String,
    prompt: String,
    empty_message: String,
    footer: String,
    input: Editor,
    items: Vec<SessionResource>,
    all_items: Vec<SessionResource>,
    selected: usize,
    loading: bool,
    mode: SessionViewMode,
    scope_session_id: Option<i64>,
    page_limit: usize,
    page_index: usize,
    offset: usize,
    cursors: Vec<Option<String>>,
    next_cursor: Option<String>,
    has_more: bool,
}

#[derive(Debug, Clone)]
struct PickerOverlay {
    title: String,
    prompt: String,
    empty_message: String,
    footer: String,
    input: Editor,
    all_items: Vec<PickerItem>,
    items: Vec<PickerItem>,
    selected: usize,
    loading: bool,
    kind: PickerKind,
}

#[derive(Debug, Clone)]
struct SessionModelChooserOverlay {
    title: String,
    prompt: String,
    footer: String,
    empty_message: String,
    input: Editor,
    loading: bool,
    all_items: Vec<SessionModelChoiceItem>,
    items: Vec<SessionModelChoiceItem>,
    selected: usize,
    page_size: usize,
}

#[derive(Debug, Clone)]
struct SessionModelChoiceItem {
    label: String,
    detail: String,
    search_text: String,
    model: ModelRef,
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
    RuntimeEntry(String),
    Provider(ProviderSummaryResource),
    Session(i64),
    Message(i64),
    PermissionRuleCreate,
    PermissionRule(Box<PermissionRuleResource>),
    Inspector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderPickerPurpose {
    SetProvider,
}

#[derive(Debug, Clone)]
enum PickerKind {
    Commands,
    Lineage { session_id: i64 },
    RewindMessages { session_id: i64 },
    Providers(ProviderPickerPurpose),
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
    items: Vec<SessionResource>,
    selected: usize,
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

#[derive(Debug, Clone)]
struct SlashCommandSuggestionState {
    query: String,
    fingerprint: String,
    items: Vec<SlashCommandSuggestionItem>,
    selected: usize,
}

#[derive(Debug, Clone)]
struct SlashCommandSuggestionItem {
    label: String,
    detail: String,
    value: SlashCommandSuggestionValue,
}

#[derive(Debug, Clone)]
enum SlashCommandSuggestionValue {
    Command(&'static CommandSpec),
    RuntimeEntry(String),
}

#[derive(Debug, Clone)]
struct SlashCommandSuggestionContext {
    query: String,
    fingerprint: String,
    name_range: Range<usize>,
}

#[derive(Debug, Clone)]
struct FileMentionSuggestionState {
    query: String,
    fingerprint: String,
    mention_range: Range<usize>,
    items: Vec<FileMentionSuggestionItem>,
    selected: usize,
}

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
struct PromptHistorySearchState {
    query: Editor,
    results: Vec<PromptHistorySearchResult>,
    selected: usize,
    original: ComposerDraft,
}

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
struct PersistentPromptHistoryEntry {
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

impl TranscriptNodeKind {
    fn label(self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Reasoning => "thinking block",
            Self::Tool => "tool output",
        }
    }
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

#[derive(Debug, Clone, Default)]
struct Editor {
    text: String,
    cursor: usize,
    preferred_column: Option<usize>,
    kill_buffer: String,
    elements: Vec<EditorElement>,
    paste_burst: PasteBurst,
}

#[derive(Debug, Clone)]
struct EditorView {
    lines: Vec<Line<'static>>,
    cursor_x: u16,
    cursor_y: u16,
}

#[derive(Debug, Clone)]
struct EditorElement {
    range: Range<usize>,
}

#[derive(Debug, Clone, Default)]
struct PasteBurst {
    last_plain_char_time: Option<Instant>,
    consecutive_plain_char_burst: u16,
    burst_window_until: Option<Instant>,
    buffer: String,
    active: bool,
    pending_first_char: Option<(char, Instant)>,
}

#[derive(Debug, Clone, Copy)]
enum PasteCharDecision {
    BeginBuffer { retro_chars: u16 },
    BufferAppend,
    RetainFirstChar,
    BeginBufferFromPending,
}

#[derive(Debug, Clone)]
enum PasteFlushResult {
    Paste(String),
    Typed(char),
    None,
}

#[derive(Debug, Clone)]
struct RetroGrab {
    start_byte: usize,
    grabbed: String,
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
                Some(format!("failed to load composer drafts: {error}")),
            ),
        };
        let prompt_history_path = default_prompt_history_path();
        let (prompt_history, pending_prompt_history_error) =
            match PromptHistory::load(&prompt_history_path) {
                Ok(history) => (history, None),
                Err(error) => (
                    PromptHistory::default(),
                    Some(format!("failed to load prompt history: {error}")),
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

        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('s') | KeyCode::Char('S'))
        {
            self.open_resume_session_picker();
            return;
        }

        if !self.current_route_is_main() {
            return;
        }

        // ESC while a run is in flight has global priority. Cancel the
        // active run before falling through to focus-specific Esc.
        if matches!(key.code, KeyCode::Esc)
            && key.modifiers.is_empty()
            && self.transcript.submitting
            && let Some(session_id) = self.transcript.session_id
        {
            self.transcript.submitting = false;
            self.submitting_session_ids.remove(&session_id);
            self.request_cancel_run(session_id);
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
                    self.overlay = Some(Overlay::TranscriptSearch(LineInputOverlay {
                        title: ui_text::t(&self.i18n, "overlay-transcript-search-title"),
                        prompt: ui_text::t(&self.i18n, "overlay-transcript-search-prompt"),
                        input: Editor::from_text(self.transcript.search_query.clone()),
                    }));
                }
                Focus::Composer => unreachable!("composer focus is excluded above"),
            }
            return;
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('f')) {
            self.overlay = Some(Overlay::TranscriptSearch(LineInputOverlay {
                title: ui_text::t(&self.i18n, "overlay-transcript-search-title"),
                prompt: ui_text::t(&self.i18n, "overlay-transcript-search-prompt"),
                input: Editor::from_text(self.transcript.search_query.clone()),
            }));
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
            self.open_plugin_inspector_overlay("");
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

    fn handle_overlay_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut overlay) = self.overlay.take() else {
            return false;
        };

        let close = match &mut overlay {
            Overlay::TranscriptSearch(dialog) => {
                self.handle_line_overlay_key(key, dialog, OverlayCommit::TranscriptSearch)
            }
            Overlay::SessionRename(dialog) => self.handle_session_rename_overlay_key(key, dialog),
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
            Overlay::Permission(dialog) => self.handle_permission_overlay_key(key, dialog),
            Overlay::UserInputReply(dialog) => self.handle_user_input_overlay_key(key, dialog),
            Overlay::Confirm(dialog) => self.handle_confirm_overlay_key(key, dialog),
            Overlay::SessionSearch(dialog) => self.handle_session_search_overlay_key(key, dialog),
            Overlay::Picker(dialog) => self.handle_picker_overlay_key(key, dialog),
            Overlay::SessionModelChooser(dialog) => {
                self.handle_session_model_chooser_overlay_key(key, dialog)
            }
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
            Route::AgentPermissionStudio(dialog) => {
                self.handle_agent_permission_studio_overlay_key(key, dialog)
            }
            Route::SessionSearch(dialog) => self.handle_session_search_overlay_key(key, dialog),
            Route::Picker(dialog) => self.handle_picker_overlay_key(key, dialog),
            Route::SessionModelChooser(dialog) => {
                self.handle_session_model_chooser_overlay_key(key, dialog)
            }
            Route::Timeline(dialog) => self.handle_timeline_overlay_key(key, dialog),
            Route::PluginInspector(dialog) => self.handle_plugin_inspector_overlay_key(key, dialog),
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
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => {
                dialog.input.flush_all_pending_input();
                let value = dialog.input.text().trim().to_string();
                match commit {
                    OverlayCommit::TranscriptSearch => {
                        self.transcript.set_search_query(value);
                        self.jump_search_match(true);
                    }
                }
                true
            }
            _ => {
                dialog.input.handle_line_input_key(key);
                false
            }
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
                        .get(dialog.selected_question)
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

        match dialog.screen {
            UserInputOverlayScreen::Question => self.handle_user_input_question_key(key, dialog),
            UserInputOverlayScreen::Review => self.handle_user_input_review_key(key, dialog),
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
                dialog.selected_option = 0;
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
                Self::move_user_input_question(dialog, -5);
                false
            }
            KeyCode::PageDown => {
                Self::move_user_input_question(dialog, 5);
                false
            }
            KeyCode::Home => {
                dialog.selected_question = 0;
                false
            }
            KeyCode::End => {
                dialog.selected_question = dialog.request.questions.len().saturating_sub(1);
                false
            }
            KeyCode::Char('e') | KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                Self::focus_user_input_question(dialog, dialog.selected_question);
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
        match Self::build_structured_user_input_reply(dialog) {
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
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
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
            dialog.selected_question = 0;
            dialog.selected_option = 0;
            dialog.screen = UserInputOverlayScreen::Question;
            return;
        }
        let len = dialog.request.questions.len() as isize;
        let next = (dialog.selected_question as isize + delta).clamp(0, len - 1) as usize;
        if dialog.screen == UserInputOverlayScreen::Review {
            dialog.selected_question = next;
            return;
        }
        Self::focus_user_input_question(dialog, next);
    }

    fn focus_user_input_question(dialog: &mut UserInputOverlay, index: usize) {
        if dialog.request.questions.is_empty() {
            dialog.selected_question = 0;
            dialog.selected_option = 0;
            dialog.screen = UserInputOverlayScreen::Question;
            return;
        }
        dialog.screen = UserInputOverlayScreen::Question;
        dialog.selected_question = min(index, dialog.request.questions.len().saturating_sub(1));
        Self::sync_user_input_option_selection(dialog);
    }

    fn sync_user_input_option_selection(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
            dialog.selected_option = 0;
            return;
        };
        let row_count = Self::user_input_option_row_count(question);
        if row_count == 0 {
            dialog.selected_option = 0;
            return;
        }
        let preferred = dialog
            .answers
            .get(&question.id)
            .map(|draft| Self::preferred_user_input_option_row(question, draft))
            .unwrap_or(0);
        dialog.selected_option = min(preferred, row_count.saturating_sub(1));
    }

    fn move_user_input_option(dialog: &mut UserInputOverlay, delta: isize) {
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
            return;
        };
        let row_count = Self::user_input_option_row_count(question);
        if row_count == 0 {
            return;
        }
        let len = row_count as isize;
        dialog.selected_option =
            (dialog.selected_option as isize + delta).clamp(0, len - 1) as usize;
    }

    fn move_user_input_option_to_end(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
            return;
        };
        let row_count = Self::user_input_option_row_count(question);
        if row_count == 0 {
            dialog.selected_option = 0;
            return;
        }
        dialog.selected_option = row_count.saturating_sub(1);
    }

    fn move_user_input_tab(dialog: &mut UserInputOverlay, delta: isize) {
        if dialog.request.questions.is_empty() {
            dialog.screen = UserInputOverlayScreen::Question;
            return;
        }
        if dialog.screen == UserInputOverlayScreen::Review {
            if delta < 0 {
                Self::focus_user_input_question(dialog, dialog.selected_question);
            }
            return;
        }
        let last_index = dialog.request.questions.len().saturating_sub(1);
        if delta < 0 {
            if dialog.selected_question > 0 {
                Self::focus_user_input_question(dialog, dialog.selected_question - 1);
            }
            return;
        }
        if dialog.selected_question < last_index {
            Self::focus_user_input_question(dialog, dialog.selected_question + 1);
            return;
        }
        if !Self::user_input_review_hidden(dialog) {
            dialog.screen = UserInputOverlayScreen::Review;
        }
    }

    fn toggle_user_input_option(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
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
            if !draft.option_indexes.insert(dialog.selected_option) {
                draft.option_indexes.remove(&dialog.selected_option);
            }
        } else {
            draft.option_indexes.clear();
            draft.option_indexes.insert(dialog.selected_option);
            draft.custom_values.clear();
        }
    }

    fn select_user_input_option(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
            return;
        };
        if Self::selected_user_input_row_is_custom(dialog, question) {
            return;
        }
        let question_id = question.id.clone();
        let draft = dialog.answers.entry(question_id).or_default();
        draft.option_indexes.clear();
        draft.option_indexes.insert(dialog.selected_option);
        draft.custom_values.clear();
    }

    fn begin_user_input_custom_edit(dialog: &mut UserInputOverlay) -> bool {
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
            return false;
        };
        let allow_custom = question.allow_custom;
        let selected_option = question.options.len();
        let question_id = question.id.clone();
        if !allow_custom {
            return false;
        }
        dialog.screen = UserInputOverlayScreen::Question;
        dialog.selected_option = selected_option;
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
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
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
        dialog.selected_option = custom_row;
        dialog.editing_custom = false;
        !draft.custom_values.is_empty()
    }

    fn clear_user_input_answer(dialog: &mut UserInputOverlay) {
        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
            return;
        };
        dialog.answers.remove(&question.id);
        dialog.custom_input.clear();
        dialog.editing_custom = false;
    }

    fn build_structured_user_input_reply(
        dialog: &mut UserInputOverlay,
    ) -> std::result::Result<UserInputReply, String> {
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
                return Err(format!("missing answer for {label}"));
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
        question.allow_custom && dialog.selected_option >= question.options.len()
    }

    fn handle_permission_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.selected = min(
                    dialog.selected + 1,
                    permission_overlay_choices(&self.i18n)
                        .len()
                        .saturating_sub(1),
                );
                false
            }
            KeyCode::Enter => {
                let choice = permission_overlay_choice(dialog.selected);
                self.request_permission_reply(
                    dialog.session_id,
                    dialog.request.request_id.clone(),
                    choice.kind,
                    choice.scope,
                    permission_overlay_choice_label(&self.i18n, choice),
                );
                true
            }
            KeyCode::Char('a') => {
                self.request_permission_reply(
                    dialog.session_id,
                    dialog.request.request_id.clone(),
                    PermissionReplyKind::AllowOnce,
                    None,
                    ui_text::permission_reply_label(&self.i18n, PermissionReplyKind::AllowOnce),
                );
                true
            }
            KeyCode::Char('s') | KeyCode::Char('A') => {
                self.request_permission_reply(
                    dialog.session_id,
                    dialog.request.request_id.clone(),
                    PermissionReplyKind::AllowAlways,
                    Some(PermissionScope::Session),
                    ui_text::permission_reply_label(&self.i18n, PermissionReplyKind::AllowAlways),
                );
                true
            }
            KeyCode::Char('d') => {
                self.request_permission_reply(
                    dialog.session_id,
                    dialog.request.request_id.clone(),
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
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => {
                dialog.input.flush_all_pending_input();
                self.submit_session_rename(dialog.input.text())
            }
            _ => {
                dialog.input.handle_line_input_key(key);
                false
            }
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
            KeyCode::Char('d')
                if dialog.focus == SettingsStudioFocus::Items
                    && matches!(
                        dialog.sections.get(dialog.selected_section),
                        Some(section) if section.id == SettingsStudioSectionId::Agents
                    ) =>
            {
                let Some(item) = dialog
                    .sections
                    .get(dialog.selected_section)
                    .and_then(|section| section.items.get(dialog.selected_item))
                    .cloned()
                else {
                    return false;
                };
                let SettingsPickerAction::OpenAgent(agent) = item.action else {
                    return false;
                };
                self.set_default_agent(agent.name.as_str(), dialog);
                false
            }
            KeyCode::Char('t')
                if dialog.focus == SettingsStudioFocus::Items
                    && matches!(
                        dialog.sections.get(dialog.selected_section),
                        Some(section) if section.id == SettingsStudioSectionId::Agents
                    ) =>
            {
                let Some(item) = dialog
                    .sections
                    .get(dialog.selected_section)
                    .and_then(|section| section.items.get(dialog.selected_item))
                    .cloned()
                else {
                    return false;
                };
                let SettingsPickerAction::OpenAgent(agent) = item.action else {
                    return false;
                };
                if agent.source_path.is_some()
                    || !matches!(agent.scope, agena::agents::AgentScope::Project)
                {
                    self.flash_warning(format!(
                        "agent {} is not stored in the current config file; open the source file to edit it",
                        agent.name
                    ));
                    return false;
                }
                self.toggle_agent_hidden(agent.name.as_str(), agent.hidden, dialog);
                false
            }
            KeyCode::Char('t') | KeyCode::Char('d')
                if dialog.focus == SettingsStudioFocus::Items
                    && matches!(
                        dialog.sections.get(dialog.selected_section),
                        Some(section) if section.id == SettingsStudioSectionId::PluginEntries
                    ) =>
            {
                let Some(item) = dialog
                    .sections
                    .get(dialog.selected_section)
                    .and_then(|section| section.items.get(dialog.selected_item))
                    .cloned()
                else {
                    return false;
                };
                let SettingsPickerAction::TogglePluginEntryDisabled {
                    plugin_id,
                    entry,
                    disabled,
                } = item.action
                else {
                    return false;
                };
                self.toggle_plugin_entry_disabled(plugin_id.as_str(), entry, disabled, dialog);
                false
            }
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l')
                if dialog.focus == SettingsStudioFocus::Navigation =>
            {
                dialog.focus = SettingsStudioFocus::Items;
                false
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h')
                if dialog.focus == SettingsStudioFocus::Items =>
            {
                dialog.focus = SettingsStudioFocus::Navigation;
                false
            }
            KeyCode::PageUp => {
                self.move_settings_studio_selection(dialog, -10);
                false
            }
            KeyCode::PageDown => {
                self.move_settings_studio_selection(dialog, 10);
                false
            }
            KeyCode::Home => {
                self.set_settings_studio_selection(dialog, 0);
                false
            }
            KeyCode::End => {
                self.set_settings_studio_selection(dialog, usize::MAX);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_settings_studio_selection(dialog, -1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_settings_studio_selection(dialog, 1);
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
        if dialog.editor.is_some() {
            if matches!(key.code, KeyCode::Esc) {
                dialog.editor = None;
                return false;
            }
            let commit = if let Some(editor) = dialog.editor.as_mut() {
                if editor.multiline {
                    if matches!(key.code, KeyCode::Char('s'))
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        editor.input.flush_all_pending_input();
                        Some((editor.action, editor.input.text().to_string()))
                    } else {
                        editor.input.handle_multiline_input_key(key);
                        None
                    }
                } else {
                    match key.code {
                        KeyCode::Enter => {
                            editor.input.flush_all_pending_input();
                            Some((editor.action, editor.input.text().to_string()))
                        }
                        _ => {
                            editor.input.handle_line_input_key(key);
                            None
                        }
                    }
                }
            } else {
                None
            };
            if let Some((action, input)) = commit {
                if let Err(error) = self.commit_agent_studio_editor(dialog, action, input) {
                    self.flash_error(error);
                } else {
                    dialog.editor = None;
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
            KeyCode::Char('d') => {
                match self.set_default_agent_value(dialog.agent_name.as_str()) {
                    Ok(()) => {
                        self.flash_success(format!("set default.agent to {}", dialog.agent_name));
                        self.refresh_agent_studio_overlay(dialog);
                    }
                    Err(error) => self.flash_error(error),
                }
                false
            }
            KeyCode::Char('t') => {
                if !dialog.editable {
                    self.flash_warning(
                        "this agent is file-backed; open the source file to edit hidden/visible"
                            .to_string(),
                    );
                    return false;
                }
                let hidden = dialog.profile.frontmatter.hidden;
                match self.set_agent_hidden_value(dialog.agent_name.as_str(), !hidden) {
                    Ok(_) => {
                        self.flash_success(if hidden {
                            format!("unhid agent {}", dialog.agent_name)
                        } else {
                            format!("hid agent {}", dialog.agent_name)
                        });
                        self.refresh_agent_studio_overlay(dialog);
                    }
                    Err(error) => self.flash_error(error),
                }
                false
            }
            KeyCode::PageUp => {
                dialog.selected = dialog.selected.saturating_sub(10);
                false
            }
            KeyCode::PageDown => {
                dialog.selected = min(
                    dialog.selected.saturating_add(10),
                    dialog.items.len().saturating_sub(1),
                );
                false
            }
            KeyCode::Home => {
                dialog.selected = 0;
                false
            }
            KeyCode::End => {
                dialog.selected = dialog.items.len().saturating_sub(1);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.selected = min(
                    dialog.selected.saturating_add(1),
                    dialog.items.len().saturating_sub(1),
                );
                false
            }
            KeyCode::Enter => self.activate_agent_studio_selection(dialog),
            _ => false,
        }
    }

    fn handle_agent_permission_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut AgentPermissionStudioOverlay,
    ) -> bool {
        if dialog.editor.is_some() {
            if matches!(key.code, KeyCode::Esc) {
                dialog.editor = None;
                return false;
            }
            let commit = if let Some(editor) = dialog.editor.as_mut() {
                if editor.multiline {
                    if matches!(key.code, KeyCode::Char('s'))
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        editor.input.flush_all_pending_input();
                        Some((editor.action, editor.input.text().to_string()))
                    } else {
                        editor.input.handle_multiline_input_key(key);
                        None
                    }
                } else {
                    match key.code {
                        KeyCode::Enter => {
                            editor.input.flush_all_pending_input();
                            Some((editor.action, editor.input.text().to_string()))
                        }
                        _ => {
                            editor.input.handle_line_input_key(key);
                            None
                        }
                    }
                }
            } else {
                None
            };
            if let Some((action, input)) = commit {
                if let Err(error) = self.commit_agent_permission_editor(dialog, action, input) {
                    self.flash_error(error);
                } else {
                    dialog.editor = None;
                }
            }
            return false;
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('r') => {
                self.refresh_agent_permission_studio_overlay(dialog);
                false
            }
            KeyCode::Char('o') => {
                self.open_agent_profile_source(&dialog.profile);
                false
            }
            KeyCode::PageUp => {
                dialog.selected = dialog.selected.saturating_sub(10);
                false
            }
            KeyCode::PageDown => {
                dialog.selected = min(
                    dialog.selected.saturating_add(10),
                    dialog.items.len().saturating_sub(1),
                );
                false
            }
            KeyCode::Home => {
                dialog.selected = 0;
                false
            }
            KeyCode::End => {
                dialog.selected = dialog.items.len().saturating_sub(1);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.selected = min(
                    dialog.selected.saturating_add(1),
                    dialog.items.len().saturating_sub(1),
                );
                false
            }
            KeyCode::Enter => self.activate_agent_permission_studio_selection(dialog),
            _ => false,
        }
    }

    fn activate_agent_studio_selection(&mut self, dialog: &mut AgentStudioOverlay) -> bool {
        let Some(item) = dialog.items.get(dialog.selected).cloned() else {
            return false;
        };
        match item.action {
            AgentStudioAction::Edit(field) => {
                if !dialog.editable {
                    self.flash_warning(
                        "this agent is file-backed; open the source file to edit it".to_string(),
                    );
                    return false;
                }
                self.open_agent_studio_editor(dialog, field);
            }
            AgentStudioAction::ToggleHidden => {
                if !dialog.editable {
                    self.flash_warning(
                        "this agent is file-backed; open the source file to edit it".to_string(),
                    );
                    return false;
                }
                let hidden = dialog.profile.frontmatter.hidden;
                match self.set_agent_hidden_value(dialog.agent_name.as_str(), !hidden) {
                    Ok(()) => {
                        self.flash_success(if hidden {
                            format!("unhid agent {}", dialog.agent_name)
                        } else {
                            format!("hid agent {}", dialog.agent_name)
                        });
                        self.refresh_agent_studio_overlay(dialog);
                    }
                    Err(error) => self.flash_error(error),
                }
            }
            AgentStudioAction::SetDefault => {
                match self.set_default_agent_value(dialog.agent_name.as_str()) {
                    Ok(()) => {
                        self.flash_success(format!("set default.agent to {}", dialog.agent_name));
                        self.refresh_agent_studio_overlay(dialog);
                    }
                    Err(error) => self.flash_error(error),
                }
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
            agent_studio_editor_config(&dialog.profile, field);
        dialog.editor = Some(AgentStudioEditor {
            title,
            prompt,
            footer,
            multiline,
            input,
            action: AgentStudioEditorAction::Field(field),
        });
    }

    fn commit_agent_studio_editor(
        &mut self,
        dialog: &mut AgentStudioOverlay,
        action: AgentStudioEditorAction,
        input: String,
    ) -> UiResult<()> {
        match action {
            AgentStudioEditorAction::Field(field) => {
                let (path, value) = agent_studio_field_setting_value(
                    dialog.agent_name.as_str(),
                    field,
                    input.as_str(),
                )?;
                if let Some(value) = value {
                    self.block_on_async(self.backend.set_config_setting(path.as_str(), value))
                        .map_err(|error| error.to_string())?;
                    self.flash_success(format!("updated {path}"));
                } else {
                    self.block_on_async(self.backend.delete_config_setting(path.as_str()))
                        .map_err(|error| error.to_string())?;
                    self.flash_success(format!("cleared {path}"));
                }
                self.refresh_agent_studio_overlay(dialog);
            }
        }
        Ok(())
    }

    fn activate_agent_permission_studio_selection(
        &mut self,
        dialog: &mut AgentPermissionStudioOverlay,
    ) -> bool {
        let Some(item) = dialog.items.get(dialog.selected).cloned() else {
            return false;
        };
        match item.action {
            AgentPermissionStudioAction::Edit(field) => {
                if !dialog.editable {
                    self.flash_warning(
                        "this agent is file-backed; open the source file to edit permissions"
                            .to_string(),
                    );
                    return false;
                }
                self.open_agent_permission_studio_editor(dialog, field);
            }
            AgentPermissionStudioAction::OpenSource => {
                self.open_agent_profile_source(&dialog.profile)
            }
        }
        false
    }

    fn open_agent_permission_studio_editor(
        &mut self,
        dialog: &mut AgentPermissionStudioOverlay,
        field: AgentPermissionField,
    ) {
        let (title, prompt, footer, multiline, input) =
            agent_permission_editor_config(&dialog.profile, field);
        dialog.editor = Some(AgentPermissionStudioEditor {
            title,
            prompt,
            footer,
            multiline,
            input,
            action: AgentPermissionStudioEditorAction::Field(field),
        });
    }

    fn commit_agent_permission_editor(
        &mut self,
        dialog: &mut AgentPermissionStudioOverlay,
        action: AgentPermissionStudioEditorAction,
        input: String,
    ) -> UiResult<()> {
        match action {
            AgentPermissionStudioEditorAction::Field(field) => {
                let (path, value) = agent_permission_field_setting_value(
                    dialog.agent_name.as_str(),
                    field,
                    input.as_str(),
                )?;
                if let Some(value) = value {
                    self.block_on_async(self.backend.set_config_setting(path.as_str(), value))
                        .map_err(|error| error.to_string())?;
                    self.flash_success(format!("updated {path}"));
                } else {
                    self.block_on_async(self.backend.delete_config_setting(path.as_str()))
                        .map_err(|error| error.to_string())?;
                    self.flash_success(format!("cleared {path}"));
                }
                self.refresh_agent_permission_studio_overlay(dialog);
            }
        }
        Ok(())
    }

    fn handle_settings_value_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SettingsValueEditOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => {
                dialog.input.flush_all_pending_input();
                let input = dialog.input.text().to_string();
                match parse_settings_field_input(dialog.field, input.as_str()) {
                    Ok(Some(value)) => match self
                        .block_on_async(self.backend.set_config_setting(dialog.field.path, value))
                    {
                        Ok(_) => {
                            self.flash_success(format!("updated {}", dialog.field.path));
                            self.refresh_current_route_after_local_edit();
                            true
                        }
                        Err(error) => {
                            self.flash_error(error);
                            false
                        }
                    },
                    Ok(None) => match self
                        .block_on_async(self.backend.delete_config_setting(dialog.field.path))
                    {
                        Ok(_) => {
                            self.flash_success(format!("cleared {}", dialog.field.path));
                            self.refresh_current_route_after_local_edit();
                            true
                        }
                        Err(error) => {
                            self.flash_error(error);
                            false
                        }
                    },
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            _ => {
                dialog.input.handle_line_input_key(key);
                false
            }
        }
    }

    fn handle_runtime_setting_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut RuntimeSettingEditOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => {
                dialog.input.flush_all_pending_input();
                let input = dialog.input.text().to_string();
                match self
                    .run_options
                    .apply_runtime_setting_input(dialog.field, input.as_str())
                {
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
            _ => {
                dialog.input.handle_line_input_key(key);
                false
            }
        }
    }

    fn handle_choice_overlay_key(&mut self, key: KeyEvent, dialog: &mut ChoiceOverlay) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let row_count = Self::choice_overlay_rows(dialog).len();
                if row_count > 0 {
                    dialog.selected = min(dialog.selected + 1, row_count.saturating_sub(1));
                }
                false
            }
            KeyCode::PageUp => {
                dialog.selected = dialog.selected.saturating_sub(10);
                false
            }
            KeyCode::PageDown => {
                let row_count = Self::choice_overlay_rows(dialog).len();
                if row_count > 0 {
                    dialog.selected = min(dialog.selected + 10, row_count.saturating_sub(1));
                }
                false
            }
            KeyCode::Home => {
                dialog.selected = 0;
                false
            }
            KeyCode::End => {
                let row_count = Self::choice_overlay_rows(dialog).len();
                if row_count > 0 {
                    dialog.selected = row_count.saturating_sub(1);
                }
                false
            }
            KeyCode::Tab => {
                if let Some(ChoiceRow::Item(item)) =
                    Self::choice_overlay_rows(dialog).get(dialog.selected)
                {
                    dialog.input.set_text(item.value.clone());
                    Self::sync_choice_overlay_query(dialog, true);
                }
                false
            }
            KeyCode::Enter => self.commit_choice_overlay(dialog),
            _ => {
                dialog.input.handle_line_input_key(key);
                Self::sync_choice_overlay_query(dialog, true);
                false
            }
        }
    }

    fn handle_permission_rule_edit_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PermissionRuleEditOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => {
                dialog.input.flush_all_pending_input();
                let draft = match parse_permission_rule_input(dialog.input.text()) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        self.flash_warning(error);
                        return false;
                    }
                };
                let label = permission_rule_draft_label(&draft);
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
                            &crate::fl_args!("name" => permission_rule_label(&rule)),
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
            _ => {
                dialog.input.handle_line_input_key(key);
                false
            }
        }
    }

    fn handle_file_attach_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut FileAttachOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !dialog.results.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 1, dialog.results.len().saturating_sub(1));
                }
                false
            }
            KeyCode::Tab => {
                if let Some(path) = dialog.results.get(dialog.selected) {
                    dialog.input.set_text(path.to_string_lossy().to_string());
                }
                false
            }
            KeyCode::Enter => {
                let path = dialog
                    .results
                    .get(dialog.selected)
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from(dialog.input.text().trim()));
                match self.stage_attachment_from_path(path.as_path(), false) {
                    Ok(()) => true,
                    Err(error) => {
                        self.flash_error(error);
                        false
                    }
                }
            }
            _ => {
                dialog.input.handle_line_input_key(key);
                self.refresh_file_attach_overlay(dialog);
                false
            }
        }
    }

    fn handle_help_overlay_key(&mut self, key: KeyEvent, dialog: &mut HelpOverlay) -> bool {
        let max_scroll = ui_text::help_lines(&self.i18n)
            .len()
            .saturating_sub(1)
            .min(u16::MAX as usize) as u16;
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.scroll = dialog.scroll.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.scroll = min(dialog.scroll.saturating_add(1), max_scroll);
                false
            }
            KeyCode::PageUp => {
                dialog.scroll = dialog.scroll.saturating_sub(8);
                false
            }
            KeyCode::PageDown => {
                dialog.scroll = min(dialog.scroll.saturating_add(8), max_scroll);
                false
            }
            KeyCode::Home => {
                dialog.scroll = 0;
                false
            }
            KeyCode::End => {
                dialog.scroll = max_scroll;
                false
            }
            _ => false,
        }
    }

    fn handle_session_search_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionSearchOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !dialog.items.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 1, dialog.items.len().saturating_sub(1));
                }
                false
            }
            KeyCode::PageUp => {
                dialog.selected = dialog.selected.saturating_sub(10);
                false
            }
            KeyCode::PageDown => {
                if !dialog.items.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 10, dialog.items.len().saturating_sub(1));
                }
                false
            }
            KeyCode::Home => {
                dialog.selected = 0;
                false
            }
            KeyCode::End => {
                if !dialog.items.is_empty() {
                    dialog.selected = dialog.items.len().saturating_sub(1);
                }
                false
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if dialog.loading || dialog.page_index == 0 {
                    return false;
                }
                dialog.page_index = dialog.page_index.saturating_sub(1);
                dialog.selected = 0;
                match dialog.mode {
                    SessionViewMode::Subtree => {
                        self.refresh_session_search_overlay_local(dialog);
                    }
                    SessionViewMode::All | SessionViewMode::Roots => {
                        let cursor = dialog.cursors.get(dialog.page_index).cloned().flatten();
                        dialog.loading = true;
                        dialog.footer = self.session_search_footer(dialog);
                        self.request_session_search_page(
                            dialog.mode,
                            dialog.input.text().trim().to_string(),
                            dialog.page_index,
                            cursor,
                        );
                    }
                }
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if dialog.loading || !dialog.has_more {
                    return false;
                }
                match dialog.mode {
                    SessionViewMode::Subtree => {
                        dialog.page_index = dialog.page_index.saturating_add(1);
                        dialog.selected = 0;
                        self.refresh_session_search_overlay_local(dialog);
                    }
                    SessionViewMode::All | SessionViewMode::Roots => {
                        let Some(cursor) = dialog.next_cursor.clone() else {
                            return false;
                        };
                        dialog.page_index = dialog.page_index.saturating_add(1);
                        if dialog.cursors.len() <= dialog.page_index {
                            dialog.cursors.resize(dialog.page_index + 1, None);
                        }
                        dialog.cursors[dialog.page_index] = Some(cursor.clone());
                        dialog.selected = 0;
                        dialog.loading = true;
                        dialog.footer = self.session_search_footer(dialog);
                        self.request_session_search_page(
                            dialog.mode,
                            dialog.input.text().trim().to_string(),
                            dialog.page_index,
                            Some(cursor),
                        );
                    }
                }
                false
            }
            KeyCode::Tab => {
                if let Some(session) = dialog.items.get(dialog.selected) {
                    let title = session.title.clone();
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
                self.open_session(session.id, session.title);
                self.focus = Focus::Composer;
                true
            }
            _ => {
                let before = dialog.input.text().trim().to_string();
                dialog.input.handle_line_input_key(key);
                let after = dialog.input.text().trim().to_string();
                if before != after {
                    self.reset_session_search_query(dialog, after);
                }
                false
            }
        }
    }

    fn reset_session_search_query(&mut self, dialog: &mut SessionSearchOverlay, query: String) {
        dialog.page_index = 0;
        dialog.selected = 0;
        dialog.offset = 0;
        dialog.cursors.clear();
        dialog.cursors.push(None);
        dialog.next_cursor = None;
        dialog.has_more = false;
        dialog.loading = true;
        dialog.footer = self.session_search_footer(dialog);
        match dialog.mode {
            SessionViewMode::Subtree => {
                if let Some(session_id) = dialog.scope_session_id {
                    self.request_session_search_subtree(session_id, query);
                }
            }
            SessionViewMode::All | SessionViewMode::Roots => {
                self.request_session_search_page(dialog.mode, query, 0, None);
            }
        }
    }

    fn refresh_session_search_overlay_local(&self, dialog: &mut SessionSearchOverlay) {
        let query = dialog.input.text().trim().to_ascii_lowercase();
        let filtered = dialog
            .all_items
            .iter()
            .filter(|session| session_matches_query(session, query.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let total = filtered.len();
        let page_limit = dialog.page_limit.max(1);
        let max_page_index = total.saturating_sub(1) / page_limit;
        dialog.page_index = min(dialog.page_index, max_page_index);
        dialog.offset = dialog.page_index.saturating_mul(page_limit);
        dialog.items = filtered
            .into_iter()
            .skip(dialog.offset)
            .take(page_limit)
            .collect();
        dialog.has_more = dialog.offset + dialog.items.len() < total;
        dialog.next_cursor = None;
        dialog.selected = min(dialog.selected, dialog.items.len().saturating_sub(1));
        dialog.loading = false;
        dialog.footer = self.session_search_footer(dialog);
    }

    fn session_search_footer(&self, dialog: &SessionSearchOverlay) -> String {
        let scope = match dialog.mode {
            SessionViewMode::All => ui_text::t(&self.i18n, "overlay-session-search-scope-all"),
            SessionViewMode::Roots => ui_text::t(&self.i18n, "overlay-session-search-scope-roots"),
            SessionViewMode::Subtree => {
                ui_text::t(&self.i18n, "overlay-session-search-scope-subtree")
            }
        };
        let start = if dialog.items.is_empty() {
            0
        } else {
            dialog.offset.saturating_add(1)
        };
        let end = dialog.offset.saturating_add(dialog.items.len());
        if dialog.mode == SessionViewMode::Subtree {
            let total = dialog
                .all_items
                .iter()
                .filter(|session| {
                    session_matches_query(
                        session,
                        dialog.input.text().trim().to_ascii_lowercase().as_str(),
                    )
                })
                .count();
            let page_total = if total == 0 {
                0
            } else {
                (total + dialog.page_limit.saturating_sub(1)) / dialog.page_limit.max(1)
            };
            return self.i18n.text_args(
                "overlay-session-search-footer-local",
                &crate::fl_args!(
                    "scope" => scope,
                    "start" => start as i64,
                    "end" => end as i64,
                    "total" => total as i64,
                    "page" => dialog.page_index.saturating_add(1) as i64,
                    "pages" => page_total.max(1) as i64,
                ),
            );
        }

        let end_state = if dialog.has_more {
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
                "page" => dialog.page_index.saturating_add(1) as i64,
                "tail" => end_state,
            ),
        )
    }

    fn handle_picker_overlay_key(&mut self, key: KeyEvent, dialog: &mut PickerOverlay) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !dialog.items.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 1, dialog.items.len().saturating_sub(1));
                }
                false
            }
            KeyCode::Tab => {
                if let Some(item) = dialog.items.get(dialog.selected) {
                    dialog.input.set_text(item.label.clone());
                    Self::refresh_picker_overlay(dialog);
                }
                false
            }
            KeyCode::Char('n') if matches!(dialog.kind, PickerKind::PermissionRules) => {
                self.open_permission_rule_editor(None, dialog.input.text(), None);
                false
            }
            KeyCode::Char('d') if matches!(dialog.kind, PickerKind::PermissionRules) => {
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
                if matches!(dialog.kind, PickerKind::PermissionRules) {
                    match item.value {
                        PickerValue::PermissionRuleCreate => {
                            self.open_permission_rule_editor(None, dialog.input.text(), None);
                            return false;
                        }
                        PickerValue::PermissionRule(rule) => {
                            self.open_permission_rule_editor(
                                Some(&rule),
                                dialog.input.text(),
                                None,
                            );
                            return false;
                        }
                        _ => {}
                    }
                }
                self.handle_picker_selection(dialog.kind.clone(), item);
                true
            }
            _ => {
                dialog.input.handle_line_input_key(key);
                Self::refresh_picker_overlay(dialog);
                false
            }
        }
    }

    fn handle_session_model_chooser_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut SessionModelChooserOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !dialog.items.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 1, dialog.items.len().saturating_sub(1));
                }
                false
            }
            KeyCode::Left | KeyCode::PageUp => {
                dialog.selected = dialog.selected.saturating_sub(dialog.page_size.max(1));
                false
            }
            KeyCode::Right | KeyCode::PageDown => {
                if !dialog.items.is_empty() {
                    dialog.selected = min(
                        dialog.selected + dialog.page_size.max(1),
                        dialog.items.len().saturating_sub(1),
                    );
                }
                false
            }
            KeyCode::Home => {
                dialog.selected = 0;
                false
            }
            KeyCode::End => {
                dialog.selected = dialog.items.len().saturating_sub(1);
                false
            }
            KeyCode::Enter => {
                let Some(item) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                self.apply_model_override(item.model);
                true
            }
            _ => {
                dialog.input.handle_line_input_key(key);
                Self::refresh_session_model_chooser_overlay(dialog, false, None);
                false
            }
        }
    }

    fn handle_timeline_overlay_key(&mut self, key: KeyEvent, dialog: &mut TimelineOverlay) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !dialog.items.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 1, dialog.items.len().saturating_sub(1));
                }
                false
            }
            KeyCode::PageUp => {
                dialog.selected = dialog.selected.saturating_sub(10);
                false
            }
            KeyCode::PageDown => {
                if !dialog.items.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 10, dialog.items.len().saturating_sub(1));
                }
                false
            }
            KeyCode::Home => {
                dialog.selected = 0;
                false
            }
            KeyCode::End => {
                if !dialog.items.is_empty() {
                    dialog.selected = dialog.items.len().saturating_sub(1);
                }
                false
            }
            KeyCode::Enter => {
                if let Some(item) = dialog.items.get(dialog.selected)
                    && let Some(message_id) = item.linked_message_id
                {
                    self.jump_to_message(message_id);
                    return true;
                }
                false
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(item) = dialog.items.get(dialog.selected) {
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
            _ => {
                dialog.input.handle_line_input_key(key);
                Self::refresh_timeline_overlay(dialog);
                false
            }
        }
    }

    fn handle_plugin_inspector_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut PluginInspectorOverlay,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !dialog.items.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 1, dialog.items.len().saturating_sub(1));
                }
                false
            }
            KeyCode::PageUp => {
                dialog.selected = dialog.selected.saturating_sub(10);
                false
            }
            KeyCode::PageDown => {
                if !dialog.items.is_empty() {
                    dialog.selected =
                        min(dialog.selected + 10, dialog.items.len().saturating_sub(1));
                }
                false
            }
            KeyCode::Home => {
                dialog.selected = 0;
                false
            }
            KeyCode::End => {
                if !dialog.items.is_empty() {
                    dialog.selected = dialog.items.len().saturating_sub(1);
                }
                false
            }
            KeyCode::Enter => false,
            KeyCode::Tab => {
                if let Some(item) = dialog.items.get(dialog.selected) {
                    dialog.input.set_text(item.plugin_id.clone());
                    Self::refresh_plugin_inspector_overlay(dialog);
                }
                false
            }
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.reload_plugin_inspector_overlay(dialog);
                false
            }
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(item) = dialog.items.get(dialog.selected) {
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
            _ => {
                dialog.input.handle_line_input_key(key);
                Self::refresh_plugin_inspector_overlay(dialog);
                false
            }
        }
    }

    fn handle_provider_studio_overlay_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut ProviderStudioOverlay,
    ) -> bool {
        if let Some(editor) = dialog.editor.as_mut() {
            if matches!(key.code, KeyCode::Esc) {
                dialog.editor = None;
                return false;
            }

            if editor.multiline {
                if matches!(key.code, KeyCode::Char('s'))
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    editor.input.flush_all_pending_input();
                    let value = editor.input.text().trim();
                    let parsed = if value.is_empty() {
                        Ok(JsonValue::Object(Default::default()))
                    } else {
                        serde_json::from_str::<JsonValue>(value).map_err(|error| error.to_string())
                    };
                    match (&editor.action, parsed) {
                        (
                            ProviderStudioEditorAction::ModelJson {
                                adapter_id,
                                model_id,
                            },
                            Ok(model_value),
                        ) => {
                            dialog.saving = true;
                            self.request_provider_studio_save_model_value(
                                dialog.draft.clone(),
                                adapter_id.clone(),
                                model_id.clone(),
                                model_value,
                            );
                            dialog.editor = None;
                        }
                        (_, Err(error)) => self.flash_error(format!("invalid model json: {error}")),
                        _ => {}
                    }
                    return false;
                }

                editor.input.handle_multiline_input_key(key);
                return false;
            }

            return match key.code {
                KeyCode::Enter => {
                    editor.input.flush_all_pending_input();
                    let value = editor.input.text().trim().to_string();
                    if let ProviderStudioEditorAction::Field(field) = editor.action
                        && let Err(error) = self.commit_provider_studio_field(dialog, field, value)
                    {
                        self.flash_error(error);
                        return false;
                    }
                    dialog.editor = None;
                    false
                }
                _ => {
                    editor.input.handle_line_input_key(key);
                    false
                }
            };
        }

        if dialog.detail_page.is_some() {
            return self.handle_provider_studio_detail_page_key(key, dialog);
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Tab => {
                dialog.focus = dialog.focus.next(dialog.show_provider_list);
                false
            }
            KeyCode::BackTab => {
                dialog.focus = dialog.focus.prev(dialog.show_provider_list);
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
            KeyCode::Char('A') if dialog.focus == ProviderStudioFocus::Adapters => {
                Self::select_all_provider_studio_adapters(dialog);
                false
            }
            KeyCode::Char('A') if dialog.focus == ProviderStudioFocus::Models => {
                Self::select_all_provider_studio_models(dialog);
                false
            }
            KeyCode::Char('c') | KeyCode::Char('C')
                if dialog.focus == ProviderStudioFocus::Adapters =>
            {
                Self::clear_provider_studio_selected_adapters(dialog);
                false
            }
            KeyCode::Char('c') | KeyCode::Char('C')
                if dialog.focus == ProviderStudioFocus::Models =>
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
            KeyCode::Char(' ') if dialog.focus == ProviderStudioFocus::Adapters => {
                self.toggle_provider_studio_selected_adapter(dialog);
                false
            }
            KeyCode::Char(' ') if dialog.focus == ProviderStudioFocus::Models => {
                self.toggle_provider_studio_selected_model(dialog);
                false
            }
            KeyCode::PageUp => {
                self.move_provider_studio_selection(dialog, -10);
                false
            }
            KeyCode::PageDown => {
                self.move_provider_studio_selection(dialog, 10);
                false
            }
            KeyCode::Home => {
                self.set_provider_studio_selection(dialog, 0);
                false
            }
            KeyCode::End => {
                self.set_provider_studio_selection(dialog, usize::MAX);
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
        if let Some(editor) = dialog.editor.as_mut() {
            return match key.code {
                KeyCode::Esc => {
                    dialog.editor = None;
                    false
                }
                KeyCode::Enter => {
                    editor.input.flush_all_pending_input();
                    dialog.query = editor.input.text().trim().to_string();
                    dialog.offset = 0;
                    dialog.selected = 0;
                    dialog.loading = true;
                    dialog.editor = None;
                    self.request_model_catalog_page(dialog.query.clone(), 0);
                    false
                }
                _ => {
                    editor.input.handle_line_input_key(key);
                    false
                }
            };
        }

        match key.code {
            KeyCode::Esc => true,
            KeyCode::Char('/') => {
                dialog.editor = Some(LineInputOverlay {
                    title: ui_text::t(&self.i18n, "overlay-model-catalog-search-title"),
                    prompt: ui_text::t(&self.i18n, "overlay-model-catalog-search-prompt"),
                    input: Editor::from_text(dialog.query.clone()),
                });
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
                dialog.selected = 0;
                dialog.loading = true;
                self.request_model_catalog_page(dialog.query.clone(), offset);
                false
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if dialog.offset + dialog.items.len() >= dialog.total {
                    return false;
                }
                dialog.offset += dialog.limit.max(1);
                dialog.selected = 0;
                dialog.loading = true;
                self.request_model_catalog_page(dialog.query.clone(), dialog.offset);
                false
            }
            KeyCode::PageUp => {
                dialog.selected = dialog.selected.saturating_sub(10);
                false
            }
            KeyCode::PageDown => {
                dialog.selected = min(
                    dialog.selected.saturating_add(10),
                    dialog.items.len().saturating_sub(1),
                );
                false
            }
            KeyCode::Home => {
                dialog.selected = 0;
                false
            }
            KeyCode::End => {
                dialog.selected = dialog.items.len().saturating_sub(1);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                dialog.selected = dialog.selected.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                dialog.selected = min(
                    dialog.selected.saturating_add(1),
                    dialog.items.len().saturating_sub(1),
                );
                false
            }
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
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                        handled_route = true;
                    }
                }
                Route::AgentPermissionStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
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
                        dialog.page_index = 0;
                        dialog.selected = 0;
                        dialog.offset = 0;
                        dialog.cursors.clear();
                        dialog.cursors.push(None);
                        dialog.next_cursor = None;
                        dialog.has_more = false;
                        dialog.loading = true;
                        pending_session_search_request =
                            Some((dialog.mode, dialog.scope_session_id, after));
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
                Route::PluginInspector(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_plugin_inspector_overlay(dialog);
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
                    if let Some(editor) = dialog.editor.as_mut() {
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
                Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
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
                    Self::sync_choice_overlay_query(dialog, true);
                }
                Overlay::PermissionRuleEdit(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::FileAttach(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    dialog.results = backend
                        .search_workspace_files(dialog.input.text(), 24)
                        .unwrap_or_default();
                    dialog.selected = min(dialog.selected, dialog.results.len().saturating_sub(1));
                }
                Overlay::UserInputReply(dialog) => {
                    if dialog.screen == UserInputOverlayScreen::Review {
                        Self::focus_user_input_question(dialog, dialog.selected_question);
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
                        dialog.page_index = 0;
                        dialog.selected = 0;
                        dialog.offset = 0;
                        dialog.cursors.clear();
                        dialog.cursors.push(None);
                        dialog.next_cursor = None;
                        dialog.has_more = false;
                        dialog.loading = true;
                        pending_session_search_request =
                            Some((dialog.mode, dialog.scope_session_id, after));
                    }
                }
                Overlay::Picker(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_picker_overlay(dialog);
                }
                Overlay::SessionModelChooser(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_session_model_chooser_overlay(dialog, false, None);
                }
                Overlay::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_all_pending_input();
                        editor.input.insert_str(text.as_str());
                    }
                }
                Overlay::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
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
            KeyCode::Home => self.sessions.selected = 0,
            KeyCode::End => {
                if !self.sessions.items.is_empty() {
                    self.sessions.selected = self.sessions.items.len().saturating_sub(1);
                    self.maybe_request_more_sessions();
                }
            }
            _ => {}
        }
    }

    fn handle_transcript_key(&mut self, key: KeyEvent) {
        let width = self.layout.transcript_body.width;
        let height = self.layout.transcript_body.height;
        if matches!(key.code, KeyCode::Char('i')) {
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
        } else if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
            self.transcript.scroll_by_lines(width, height, -1);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Down | KeyCode::Char('j')) {
            self.transcript.scroll_by_lines(width, height, 1);
        } else if matches!(key.code, KeyCode::PageUp)
            || matches!(key.code, KeyCode::Char('b'))
                && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript.scroll_by_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::PageDown)
            || matches!(key.code, KeyCode::Char('f'))
                && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript.scroll_by_page(width, height, true);
        } else if matches!(key.code, KeyCode::Char(' '))
            && key.modifiers.contains(KeyModifiers::SHIFT)
        {
            self.transcript.scroll_by_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Char(' ')) {
            self.transcript.scroll_by_page(width, height, true);
        } else if matches!(key.code, KeyCode::Char('u'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript.scroll_by_half_page(width, height, false);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::Char('d'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            self.transcript.scroll_by_half_page(width, height, true);
        } else if matches!(key.code, KeyCode::Home | KeyCode::Char('g')) {
            self.transcript.scroll_to_top(width, height);
            self.maybe_request_older_messages();
        } else if matches!(key.code, KeyCode::End | KeyCode::Char('G')) {
            self.transcript.scroll_to_bottom(width, height);
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
            self.flash_info(format!(
                "{} {}",
                if node.expanded {
                    "collapsed"
                } else {
                    "expanded"
                },
                node.kind.label()
            ));
        }
    }

    fn copy_transcript_cursor_node(&mut self) {
        let width = self.layout.transcript_body.width;
        let Some(node) = self.transcript.current_cursor_node_cloned(width) else {
            return;
        };
        match set_clipboard_text(node.copy_text.as_str()) {
            Ok(()) => self.flash_success(format!("copied current {}", node.kind.label())),
            Err(error) => self.flash_error(format!("clipboard copy failed: {error}")),
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
        // Configurable bindings take precedence over the legacy hardcoded
        // map. The defaults preserve the user's stated preference:
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
                self.flash_info("large paste snippets can be removed, but do not have a file view");
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
                self.replace_composer_draft(search.original.clone());
                true
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                if let Some(result) = search.results.get(search.selected).cloned() {
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
                if !search.results.is_empty() {
                    search.selected = min(
                        search.selected.saturating_add(1),
                        search.results.len().saturating_sub(1),
                    );
                }
                false
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                search.selected = search.selected.saturating_sub(1);
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
        let mut search = PromptHistorySearchState {
            query: Editor::default(),
            results: Vec::new(),
            selected: 0,
            original: self.current_composer_draft(),
        };
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
        search.results = prompt_history
            .entries
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
        search.selected = min(search.selected, search.results.len().saturating_sub(1));
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
        if state.items.is_empty() {
            state.selected = 0;
            return;
        }
        let len = state.items.len() as isize;
        state.selected = (state.selected as isize + delta).rem_euclid(len) as usize;
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
            .remove_range(state.mention_range.start, state.mention_range.end);
        if let Err(error) = self.stage_attachment_from_path(item.path.as_path(), false) {
            self.flash_error(error);
            return;
        }
        let after_cursor_is_space = self.composer.text()[self.composer.cursor..]
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
        self.file_mention_suggestions = Some(FileMentionSuggestionState {
            query: context.query,
            fingerprint: context.fingerprint,
            mention_range: context.mention_range,
            items,
            selected,
        });
    }

    fn file_mention_suggestion_context(&self) -> Option<FileMentionSuggestionContext> {
        if self.focus != Focus::Composer || self.overlay.is_some() || !self.current_route_is_main()
        {
            return None;
        }
        if self.prompt_history_search.is_some() {
            return None;
        }
        file_mention_suggestion_context_for_text(self.composer.text(), self.composer.cursor)
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
        if state.items.is_empty() {
            state.selected = 0;
            return;
        }
        let len = state.items.len() as isize;
        state.selected = (state.selected as isize + delta).rem_euclid(len) as usize;
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
        state.items.get(state.selected)
    }

    fn apply_slash_command_completion(&mut self, item: &SlashCommandSuggestionItem) {
        let Some(context) = self.slash_command_suggestion_context() else {
            return;
        };

        let name = match &item.value {
            SlashCommandSuggestionValue::Command(spec) => spec.name,
            SlashCommandSuggestionValue::RuntimeEntry(name) => name.as_str(),
        };
        let replacement = format!("/{name}");
        self.slash_command_suggestions = None;
        self.dismissed_slash_command_suggestions_for = None;

        self.composer
            .remove_range(context.name_range.start, context.name_range.end);
        self.composer
            .insert_str_at(context.name_range.start, replacement.as_str());

        let after_cursor_is_space = self.composer.text()[self.composer.cursor..]
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
        self.slash_command_suggestions = Some(SlashCommandSuggestionState {
            query: context.query,
            fingerprint: context.fingerprint,
            items,
            selected,
        });
    }

    fn slash_command_suggestion_context(&self) -> Option<SlashCommandSuggestionContext> {
        if self.focus != Focus::Composer || self.overlay.is_some() || !self.current_route_is_main()
        {
            return None;
        }

        let context =
            slash_command_suggestion_context_for_text(self.composer.text(), self.composer.cursor)?;
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
            self.runtime_entry_command_rows()
                .into_iter()
                .filter(|entry| runtime_entry_matches_slash_query(entry.label.as_str(), &query))
                .map(|entry| {
                    let label = entry.label;
                    SlashCommandSuggestionItem {
                        label: format!("/{label}"),
                        detail: entry.detail,
                        value: SlashCommandSuggestionValue::RuntimeEntry(label),
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
            AppMessage::SessionModelChooserLoaded { result } => {
                self.handle_session_model_chooser_loaded(result)
            }
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
                self.apply_transcript_execution(execution);
                self.maybe_auto_open_pending_interactive_overlay();
                self.sessions.select_by_id(session_id);
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
                if let Some(execution) = refresh.execution {
                    self.apply_transcript_execution(execution);
                    self.maybe_auto_open_pending_interactive_overlay();
                }
                if let Some(page) = refresh.latest_messages {
                    self.transcript.merge_latest_messages(
                        page,
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
                if refresh.event_count > 0 {
                    self.sessions.select_by_id(session_id);
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
                self.apply_transcript_execution(execution);
                self.maybe_auto_open_pending_interactive_overlay();
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
            self.apply_transcript_execution(execution);
            self.maybe_auto_open_pending_interactive_overlay();
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
                self.handle_session_execution_updated(session_id, execution, true);
                self.flash_success(self.i18n.text_args(
                    "flash-permission-reply-sent",
                    &crate::fl_args!("label" => label),
                ));
            }
            Err(error) => {
                self.transcript.submitting = false;
                self.submitting_session_ids.remove(&session_id);
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
        let PickerKind::Providers(current_purpose) = &dialog.kind else {
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
                dialog.all_items = providers
                    .into_iter()
                    .map(|provider| PickerItem {
                        label: provider.provider_id.clone(),
                        detail: format!(
                            "default {} / {}",
                            provider.default_adapter.as_deref().unwrap_or("adapter"),
                            provider.default_model
                        ),
                        value: PickerValue::Provider(provider),
                    })
                    .collect();
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
        if dialog.mode != mode
            || dialog.page_index != page_index
            || dialog.input.text().trim() != query
        {
            self.restore_session_search_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-resume-empty");
        match result {
            Ok(page) => {
                dialog.items = page.items;
                dialog.offset = dialog.page_index.saturating_mul(dialog.page_limit);
                dialog.next_cursor = page.page.next_cursor;
                dialog.has_more = page.page.has_more;
                dialog.selected = min(dialog.selected, dialog.items.len().saturating_sub(1));
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
        if dialog.mode != SessionViewMode::Subtree
            || dialog.scope_session_id != Some(session_id)
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
                dialog.all_items = sessions;
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
                } = &dialog.kind
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
                dialog.all_items = items
                    .into_iter()
                    .map(|item| self.lineage_session_picker_item(item))
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
                self.restore_picker_dialog(host, dialog);
            }
            Err(error) => {
                if let Some((host, mut dialog)) = self.take_picker_dialog() {
                    if matches!(dialog.kind, PickerKind::Lineage { session_id: current_session_id } if current_session_id == session_id)
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
        } = &dialog.kind
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
                dialog.all_items = messages
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

    fn handle_session_model_chooser_loaded(
        &mut self,
        result: UiResult<Vec<SessionModelChoiceItem>>,
    ) {
        let Some((host, mut dialog)) = self.take_session_model_chooser_dialog() else {
            return;
        };

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match result {
            Ok(items) => {
                dialog.all_items = items;
                let current_model = self.current_session_model_ref();
                Self::refresh_session_model_chooser_overlay(
                    &mut dialog,
                    true,
                    current_model.as_ref(),
                );
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_session_model_chooser_dialog(host, dialog);
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
                dialog.items = response.items;
                dialog.summary = response.summary;
                dialog.total = response.total;
                dialog.offset = response.offset;
                dialog.limit = response.limit;
                dialog.selected = min(dialog.selected, dialog.items.len().saturating_sub(1));
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
                dialog.selected_adapter = min(
                    dialog.selected_adapter,
                    dialog.adapter_candidate_ids.len().saturating_sub(1),
                );
                dialog.selected_model = 0;
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
        result: UiResult<crate::backend::ProviderDraftAuthActionResult>,
    ) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            match result {
                Ok(action) => self.flash_success(action.message),
                Err(error) => self.flash_error(error),
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
                if let Some(text) = action.clipboard_text
                    && let Err(error) = set_clipboard_text(text.as_str())
                {
                    self.flash_error(format!("clipboard copy failed: {error}"));
                }
                self.flash_success(action.message);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_provider_studio_dialog(host, dialog);
    }

    fn handle_provider_studio_saved(&mut self, provider_id: String, result: UiResult<String>) {
        let Some((host, mut dialog)) = self.take_provider_studio_dialog() else {
            match result {
                Ok(message) => self.flash_success(message),
                Err(error) => self.flash_error(error),
            }
            return;
        };
        dialog.saving = false;
        match result {
            Ok(message) => {
                let preserved_selected_adapter_ids = dialog.selected_adapter_ids.clone();
                let preserved_selected_adapter_id = provider_studio_selected_adapter_id(&dialog);
                let preserved_selected_model_keys = dialog.selected_model_keys.clone();
                self.flash_success(message);
                let providers = self.backend.list_configured_providers();
                dialog.providers = provider_studio_provider_rows(providers.as_slice());
                dialog.selected_provider = dialog
                    .providers
                    .iter()
                    .position(|row| row.provider_id.as_deref() == Some(provider_id.as_str()))
                    .unwrap_or(0);
                self.load_provider_studio_draft(&mut dialog, Some(provider_id.as_str()), None);
                restore_provider_studio_adapter_selection(
                    &mut dialog,
                    &preserved_selected_adapter_ids,
                    preserved_selected_adapter_id.as_deref(),
                );
                dialog.selected_model_keys = preserved_selected_model_keys;
                provider_studio_ensure_default_selection(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.restore_provider_studio_dialog(host, dialog);
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
                dialog.selected = 0;
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
        } = &dialog.kind
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
                dialog.all_items = sessions
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
        if dialog.session_id != session_id {
            self.restore_timeline_dialog(host, dialog);
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-timeline-empty");
        match result {
            Ok(events) => {
                dialog.all_items = events.iter().map(build_timeline_item).collect();
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
                self.apply_transcript_execution(execution);
                self.maybe_auto_open_pending_interactive_overlay();
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
            let result = Ok(backend.list_providers());
            let _ = tx.send(AppMessage::ProvidersLoaded { purpose, result });
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
    /// bus into [`AppMessage::SessionEventArrived`]. Replaces the legacy
    /// 250ms refresh polling with push-based notifications. Aborts any
    /// previous subscription so we never accumulate stale receivers.
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

    fn apply_transcript_execution(&mut self, execution: SessionExecutionResource) {
        self.transcript.apply_execution(execution);
        self.sync_seen_pending_request_ids();
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
            .map(|text| derive_session_title(text.as_str()))
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
        self.backend.runtime_entry_exists(name)
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
        // Preserve the legacy paste-burst behavior: if a multi-character
        // paste burst is active, an Enter inside it should be treated as
        // a literal newline rather than a submit/queue.
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
            if self.backend.runtime_entry_exists(name) {
                self.execute_runtime_entry_prompt(name, args);
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
        self.request_permission_reply(
            session_id,
            request.request_id,
            kind,
            None,
            ui_text::permission_reply_label(&self.i18n, kind),
        );
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
            selected_question: 0,
            selected_option: 0,
            screen: UserInputOverlayScreen::Question,
            editing_custom: false,
            custom_input: Editor::default(),
        };
        Self::sync_user_input_option_selection(&mut overlay);
        overlay
    }

    fn build_permission_overlay(session_id: i64, request: PermissionRequest) -> PermissionOverlay {
        PermissionOverlay {
            session_id,
            request,
            selected: 0,
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
        match pending_interactive_kind_for_execution(execution) {
            Some(PendingInteractiveKind::Permission) => {
                Some(ui_text::t(&self.i18n, "session-awaiting-approval"))
            }
            Some(PendingInteractiveKind::UserInput) => {
                Some(ui_text::t(&self.i18n, "session-awaiting-user-input"))
            }
            None if execution.blocked => Some(ui_text::t(&self.i18n, "session-blocked")),
            None => None,
        }
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
        let key = match kind {
            PendingInteractiveKind::Permission => "flash-session-awaiting-approval",
            PendingInteractiveKind::UserInput => "flash-session-awaiting-user-input",
        };
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

    fn open_file_attach_overlay(&mut self) {
        let mut overlay = FileAttachOverlay {
            input: Editor::default(),
            results: Vec::new(),
            selected: 0,
        };
        self.refresh_file_attach_overlay(&mut overlay);
        self.overlay = Some(Overlay::FileAttach(overlay));
    }

    fn open_rename_session_overlay(&mut self) {
        let Some(title) = self.current_or_selected_session_title() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.overlay = Some(Overlay::SessionRename(LineInputOverlay {
            title: ui_text::t(&self.i18n, "overlay-rename-title"),
            prompt: ui_text::t(&self.i18n, "overlay-rename-prompt"),
            input: Editor::from_text(title),
        }));
    }

    fn open_timeline_overlay(&mut self, limit: u64) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.current_route = Route::Timeline(TimelineOverlay {
            session_id,
            title: self.i18n.text_args(
                "overlay-timeline-title",
                &crate::fl_args!("session" => session_id),
            ),
            prompt: ui_text::t(&self.i18n, "overlay-timeline-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            footer: ui_text::t(&self.i18n, "overlay-timeline-footer"),
            input: Editor::default(),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            loading: true,
        });
        self.request_timeline(session_id, limit);
    }

    fn open_plugin_inspector_overlay(&mut self, query: &str) {
        let mut dialog = PluginInspectorOverlay {
            title: ui_text::t(&self.i18n, "overlay-plugins-title"),
            prompt: ui_text::t(&self.i18n, "overlay-plugins-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-plugins-empty"),
            footer: ui_text::t(&self.i18n, "overlay-plugins-footer"),
            input: Editor::from_text(query.to_string()),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
        };
        self.reload_plugin_inspector_overlay(&mut dialog);
        self.current_route = Route::PluginInspector(dialog);
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
        let plugins_enabled = settings_studio_plugins_enabled(&sources);
        let plugins_default_mode = settings_studio_plugins_default_mode(&sources);
        let plugin_entry_items =
            settings_studio_plugin_entry_items(&sources, &self.backend.plugin_statuses());
        let configured_providers = self.backend.list_configured_providers();
        let permission_rule_count = self
            .block_on_async(self.backend.list_permission_rules())
            .map(|rules| rules.len())
            .unwrap_or_default();
        let model_catalog = self
            .backend
            .list_model_catalog_entries("", 0, 1)
            .map_err(|error| error.to_string())?;

        let general_items = settings_studio_general_items(&sources);
        let runtime_items = settings_studio_runtime_items(&self.run_options);
        let plugin_items = settings_studio_plugin_items(&sources);
        let agent_items = settings_studio_agent_items(&agents, default_agent.as_deref());
        let provider_items = settings_studio_provider_items(&configured_providers);
        let model_catalog_items = settings_studio_model_catalog_items(&model_catalog);
        let file_items = settings_studio_file_items(&sources);
        let agent_count = agents.len();
        let agent_primary_count = agents
            .iter()
            .filter(|agent| agent.mode.allows_root())
            .count();
        let agent_subagent_count = agents
            .iter()
            .filter(|agent| agent.mode.allows_subagent())
            .count();
        let agent_hidden_count = agents.iter().filter(|agent| agent.hidden).count();
        let mut sections = vec![
            SettingsStudioSection {
                id: SettingsStudioSectionId::General,
                label: "General".to_string(),
                summary: format!("{} agena.json fields", general_items.len()),
                description:
                    "Persistent agena.json settings. Enter edits the selected field and writes the file override."
                        .to_string(),
                items: general_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Runtime,
                label: "Runtime".to_string(),
                summary: "Session-scoped provider, model, and generation overrides".to_string(),
                description:
                    "These settings affect the current session only. Provider and model actions open the existing pickers."
                        .to_string(),
                items: runtime_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Plugins,
                label: "Plugins".to_string(),
                summary: format!(
                    "{} · default {}",
                    if plugins_enabled { "enabled" } else { "disabled" },
                    plugins_default_mode
                ),
                description:
                    "Control global plugin loading and the model-visible tool description mode."
                        .to_string(),
                items: plugin_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::PluginEntries,
                label: "Plugin Entries".to_string(),
                summary: format!(
                    "{} entries · {} disabled",
                    plugin_entry_items.len(),
                    settings_studio_plugin_entry_disabled_count(&plugin_entry_items)
                ),
                description:
                    "Toggle individual plugins.list entries. Disabled entries stay in config and are skipped on reload."
                        .to_string(),
                items: plugin_entry_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Agents,
                label: "Agents".to_string(),
                summary: match default_agent.as_deref() {
                    Some(default) => format!(
                        "{} agent profiles · default {} · {} hidden",
                        agent_count, default, agent_hidden_count
                    ),
                    None => format!(
                        "{} agent profiles · {} primary · {} subagent · {} hidden",
                        agent_count, agent_primary_count, agent_subagent_count, agent_hidden_count
                    ),
                },
                description:
                    "Browse discovered agent profiles. Enter opens the dedicated workbench, d makes the selected agent default, and t toggles hidden for config-owned profiles."
                        .to_string(),
                items: agent_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Providers,
                label: "Providers".to_string(),
                summary: format!(
                    "{} configured provider{}",
                    configured_providers.len(),
                    if configured_providers.len() == 1 { "" } else { "s" }
                ),
                description:
                    "Saved providers, auth modes, adapter discovery, and per-model configuration. Enter opens the provider studio."
                        .to_string(),
                items: provider_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::ModelCatalog,
                label: "Model Catalog".to_string(),
                summary: format!("{} entries", model_catalog.summary.entry_count),
                description:
                    "Browse the resolved model catalog, inspect entry metadata, and refresh the local cache."
                        .to_string(),
                items: model_catalog_items,
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Permissions,
                label: "Permissions".to_string(),
                summary: format!("{permission_rule_count} persisted permission rule(s)"),
                description:
                    "Permission policies are managed in the existing rules workbench. Enter opens it and Esc returns here."
                        .to_string(),
                items: vec![SettingsStudioItem {
                    label: "Manage Permission Rules".to_string(),
                    value: permission_rule_count.to_string(),
                    detail:
                        "Create, inspect, edit, or revoke persisted permission rules."
                            .to_string(),
                    action: SettingsPickerAction::OpenPermissionRules,
                }],
            },
            SettingsStudioSection {
                id: SettingsStudioSectionId::Files,
                label: "Files".to_string(),
                summary: "Open raw configuration files".to_string(),
                description:
                    "Direct file access when visual editing is not enough."
                        .to_string(),
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
            title: "Configuration".to_string(),
            footer:
                "Left/Right switch pane | Up/Down move | Enter edit/open | r refresh | Esc close"
                    .to_string(),
            sections: std::mem::take(&mut sections),
            selected_section,
            selected_item,
            focus,
            default_agent_name: default_agent,
            plugins_enabled,
            plugins_default_mode,
        })
    }

    fn refresh_settings_studio_overlay(&mut self, dialog: &mut SettingsStudioOverlay) {
        let preferred_section = dialog
            .sections
            .get(dialog.selected_section)
            .map(|section| section.id);
        let preferred_item = dialog
            .sections
            .get(dialog.selected_section)
            .and_then(|section| section.items.get(dialog.selected_item))
            .map(|item| item.label.as_str());
        match self.build_settings_studio_overlay(preferred_section, preferred_item, dialog.focus) {
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

    fn build_agent_studio_overlay(
        &self,
        agent_name: &str,
        preferred_item_label: Option<&str>,
    ) -> UiResult<AgentStudioOverlay> {
        let profile = self
            .backend
            .get_agent_profile(agent_name)
            .ok_or_else(|| format!("agent not found: {agent_name}"))?;
        let editable = agent_profile_editable(&profile);
        let default_agent_name = self.backend.default_agent_name();
        let mut items = agent_studio_items(&profile, editable, default_agent_name.as_deref());
        let item_len = items.len();
        let selected = preferred_item_label
            .and_then(|label| items.iter().position(|item| item.label == label))
            .unwrap_or(0);
        Ok(AgentStudioOverlay {
            title: format!("Agent · {}", profile.name),
            footer: "Up/Down move | Enter edit/open | p permissions | d default | t hidden | o source | r refresh | Esc back".to_string(),
            agent_name: profile.name.clone(),
            profile,
            editable,
            default_agent_name,
            items: std::mem::take(&mut items),
            selected: min(selected, item_len.saturating_sub(1)),
            editor: None,
        })
    }

    fn refresh_agent_studio_overlay(&mut self, dialog: &mut AgentStudioOverlay) {
        let preferred_item = dialog
            .items
            .get(dialog.selected)
            .map(|item| item.label.as_str());
        match self.build_agent_studio_overlay(dialog.agent_name.as_str(), preferred_item) {
            Ok(updated) => *dialog = updated,
            Err(error) => self.flash_error(error),
        }
    }

    fn open_agent_permission_studio(&mut self, agent_name: &str) {
        match self.build_agent_permission_studio_overlay(agent_name, None) {
            Ok(dialog) => self.current_route = Route::AgentPermissionStudio(dialog),
            Err(error) => self.flash_error(error),
        }
    }

    fn build_agent_permission_studio_overlay(
        &self,
        agent_name: &str,
        preferred_item_label: Option<&str>,
    ) -> UiResult<AgentPermissionStudioOverlay> {
        let profile = self
            .backend
            .get_agent_profile(agent_name)
            .ok_or_else(|| format!("agent not found: {agent_name}"))?;
        let editable = agent_profile_editable(&profile);
        let mut items = agent_permission_studio_items(&profile, editable);
        let item_len = items.len();
        let selected = preferred_item_label
            .and_then(|label| items.iter().position(|item| item.label == label))
            .unwrap_or(0);
        Ok(AgentPermissionStudioOverlay {
            title: format!("Permission · {}", profile.name),
            footer: "Up/Down move | Enter edit/open | o source | r refresh | Esc back".to_string(),
            agent_name: profile.name.clone(),
            profile,
            editable,
            items: std::mem::take(&mut items),
            selected: min(selected, item_len.saturating_sub(1)),
            editor: None,
        })
    }

    fn refresh_agent_permission_studio_overlay(
        &mut self,
        dialog: &mut AgentPermissionStudioOverlay,
    ) {
        let preferred_item = dialog
            .items
            .get(dialog.selected)
            .map(|item| item.label.as_str());
        match self.build_agent_permission_studio_overlay(dialog.agent_name.as_str(), preferred_item)
        {
            Ok(updated) => *dialog = updated,
            Err(error) => self.flash_error(error),
        }
    }

    fn select_settings_studio_query(&self, dialog: &mut SettingsStudioOverlay, query: &str) {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() {
            return;
        }
        for (section_index, section) in dialog.sections.iter().enumerate() {
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
                    dialog.selected_section = section_index;
                    dialog.selected_item = item_index;
                    dialog.focus = SettingsStudioFocus::Items;
                    return;
                }
            }
        }
    }

    fn set_settings_studio_selection(&mut self, dialog: &mut SettingsStudioOverlay, target: usize) {
        match dialog.focus {
            SettingsStudioFocus::Navigation => {
                dialog.selected_section = min(target, dialog.sections.len().saturating_sub(1));
                dialog.selected_item = min(
                    dialog.selected_item,
                    dialog
                        .sections
                        .get(dialog.selected_section)
                        .map(|section| section.items.len().saturating_sub(1))
                        .unwrap_or_default(),
                );
            }
            SettingsStudioFocus::Items => {
                let max_index = dialog
                    .sections
                    .get(dialog.selected_section)
                    .map(|section| section.items.len().saturating_sub(1))
                    .unwrap_or_default();
                dialog.selected_item = min(target, max_index);
            }
        }
    }

    fn move_settings_studio_selection(&mut self, dialog: &mut SettingsStudioOverlay, delta: isize) {
        let current = match dialog.focus {
            SettingsStudioFocus::Navigation => dialog.selected_section,
            SettingsStudioFocus::Items => dialog.selected_item,
        };
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize)
        };
        self.set_settings_studio_selection(dialog, next);
    }

    fn activate_settings_studio_selection(&mut self, dialog: &mut SettingsStudioOverlay) -> bool {
        if dialog.focus == SettingsStudioFocus::Navigation {
            dialog.focus = SettingsStudioFocus::Items;
            return false;
        }
        let Some(item) = dialog
            .sections
            .get(dialog.selected_section)
            .and_then(|section| section.items.get(dialog.selected_item))
            .cloned()
        else {
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
            SettingsPickerAction::TogglePluginsEnabled => {
                self.toggle_plugins_enabled(dialog);
                false
            }
            SettingsPickerAction::ToggleToolDescriptionMode => {
                self.toggle_tool_description_mode(dialog);
                false
            }
            SettingsPickerAction::TogglePluginEntryDisabled {
                plugin_id,
                entry,
                disabled,
            } => {
                self.toggle_plugin_entry_disabled(plugin_id.as_str(), entry, disabled, dialog);
                false
            }
            SettingsPickerAction::OpenAgent(agent) => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_agent_studio(agent.name.as_str());
                false
            }
            SettingsPickerAction::OpenProviderWorkbench => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_provider_studio(None);
                false
            }
            SettingsPickerAction::OpenProviderWorkbenchFor(provider_id) => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_provider_studio(Some(provider_id.as_str()));
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
                self.flash_success("cleared provider/model runtime override stack");
                self.refresh_settings_studio_overlay(dialog);
                false
            }
            SettingsPickerAction::OpenPermissionRules => {
                self.route_stack.push(Route::SettingsStudio(dialog.clone()));
                self.open_permission_rule_picker("");
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
                    dialog
                        .sections
                        .get(dialog.selected_section)
                        .map(|section| section.id),
                    dialog
                        .sections
                        .get(dialog.selected_section)
                        .and_then(|section| section.items.get(dialog.selected_item))
                        .map(|item| item.label.as_str()),
                    dialog.focus,
                )
                .map(Route::SettingsStudio)
                .unwrap_or(Route::SettingsStudio(dialog)),
            Route::AgentStudio(dialog) => self
                .build_agent_studio_overlay(
                    dialog.agent_name.as_str(),
                    dialog
                        .items
                        .get(dialog.selected)
                        .map(|item| item.label.as_str()),
                )
                .map(Route::AgentStudio)
                .unwrap_or(Route::AgentStudio(dialog)),
            Route::AgentPermissionStudio(dialog) => self
                .build_agent_permission_studio_overlay(
                    dialog.agent_name.as_str(),
                    dialog
                        .items
                        .get(dialog.selected)
                        .map(|item| item.label.as_str()),
                )
                .map(Route::AgentPermissionStudio)
                .unwrap_or(Route::AgentPermissionStudio(dialog)),
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
            Route::Picker(dialog) if matches!(dialog.kind, PickerKind::PermissionRules)
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

    fn take_session_model_chooser_dialog(
        &mut self,
    ) -> Option<(DialogHost, SessionModelChooserOverlay)> {
        match std::mem::replace(&mut self.current_route, Route::Main) {
            Route::SessionModelChooser(dialog) => Some((DialogHost::Route, dialog)),
            route => {
                self.current_route = route;
                match self.overlay.take() {
                    Some(Overlay::SessionModelChooser(dialog)) => {
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

    fn restore_session_model_chooser_dialog(
        &mut self,
        host: DialogHost,
        dialog: SessionModelChooserOverlay,
    ) {
        match host {
            DialogHost::Route => self.current_route = Route::SessionModelChooser(dialog),
            DialogHost::Overlay => self.overlay = Some(Overlay::SessionModelChooser(dialog)),
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
        let search = dialog.filter_query.trim().to_lowercase();
        dialog.items = dialog
            .all_items
            .iter()
            .filter(|item| search.is_empty() || item.search_text.contains(search.as_str()))
            .cloned()
            .collect();
        let row_count = Self::choice_overlay_rows(dialog).len();
        if row_count == 0 {
            dialog.selected = 0;
        } else {
            dialog.selected = min(dialog.selected, row_count.saturating_sub(1));
        }
    }

    fn sync_choice_overlay_query(dialog: &mut ChoiceOverlay, prefer_input_value: bool) {
        let next_query = dialog.input.text().to_string();
        let changed = dialog.filter_query != next_query;
        dialog.filter_query = next_query;
        Self::refresh_choice_overlay(dialog);
        if prefer_input_value && changed {
            dialog.selected = Self::preferred_choice_overlay_selection(dialog);
        }
    }

    fn choice_overlay_rows(dialog: &ChoiceOverlay) -> Vec<ChoiceRow> {
        let mut rows = Vec::new();
        if dialog.allow_clear {
            rows.push(ChoiceRow::Clear);
        }
        let trimmed = dialog.input.text().trim();
        if dialog.allow_custom && !trimmed.is_empty() {
            rows.push(ChoiceRow::Custom(trimmed.to_string()));
        }
        rows.extend(dialog.items.iter().cloned().map(ChoiceRow::Item));
        rows
    }

    fn preferred_choice_overlay_selection(dialog: &ChoiceOverlay) -> usize {
        let trimmed = dialog.input.text().trim();
        if trimmed.is_empty() {
            return 0;
        }

        let clear_offset = usize::from(dialog.allow_clear);
        if dialog.allow_custom {
            if let Some(index) = dialog.items.iter().position(|item| {
                item.value.eq_ignore_ascii_case(trimmed) || item.label.eq_ignore_ascii_case(trimmed)
            }) {
                return clear_offset + usize::from(dialog.allow_custom) + index;
            }
            return clear_offset;
        }

        if let Some(index) = dialog.items.iter().position(|item| {
            item.value.eq_ignore_ascii_case(trimmed) || item.label.eq_ignore_ascii_case(trimmed)
        }) {
            return clear_offset + index;
        }

        0
    }

    fn commit_choice_overlay(&mut self, dialog: &mut ChoiceOverlay) -> bool {
        let rows = Self::choice_overlay_rows(dialog);
        let Some(selection) = rows.get(dialog.selected).cloned() else {
            return false;
        };
        match dialog.action.clone() {
            ChoiceOverlayAction::SettingsField(field) => {
                let input = match selection {
                    ChoiceRow::Clear => String::new(),
                    ChoiceRow::Custom(value) => value,
                    ChoiceRow::Item(item) => item.value,
                };
                match parse_settings_field_input(field, input.as_str()) {
                    Ok(Some(value)) => match self
                        .block_on_async(self.backend.set_config_setting(field.path, value))
                    {
                        Ok(_) => {
                            self.flash_success(format!("updated {}", field.path));
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
                                self.flash_success(format!("cleared {}", field.path));
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
                    ChoiceRow::Clear => String::new(),
                    ChoiceRow::Custom(value) => value,
                    ChoiceRow::Item(item) => item.value,
                };
                match self
                    .run_options
                    .apply_runtime_setting_input(field, input.as_str())
                {
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
            ChoiceOverlayAction::ProviderStudioField(field) => {
                let value = match selection {
                    ChoiceRow::Clear => String::new(),
                    ChoiceRow::Custom(value) => value,
                    ChoiceRow::Item(item) => item.value,
                };
                let Some((host, mut parent)) = self.take_provider_studio_dialog() else {
                    self.flash_error("provider studio context was lost");
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
            self.open_choice_overlay(ChoiceOverlay {
                title: format!("Edit {}", field.path),
                prompt: settings_value_edit_prompt(field, &file_value, &effective_value),
                footer: ui_text::t(&self.i18n, "overlay-choice-footer"),
                empty_message: ui_text::t(&self.i18n, "overlay-picker-empty"),
                input: Editor::from_text(setting_value_input_text(&prefill)),
                filter_query: String::new(),
                all_items,
                items: Vec::new(),
                selected: 0,
                allow_custom: true,
                allow_clear: true,
                action: ChoiceOverlayAction::SettingsField(field),
            });
            return;
        }
        self.overlay = Some(Overlay::SettingsValueEdit(SettingsValueEditOverlay {
            title: format!("Edit {}", field.path),
            prompt: settings_value_edit_prompt(field, &file_value, &effective_value),
            input: Editor::from_text(setting_value_input_text(&prefill)),
            field,
        }));
    }

    fn open_runtime_setting_editor(&mut self, field: RuntimeSettingSpec, _return_query: &str) {
        let current_summary = self.run_options.runtime_setting_summary(field);
        if let Some(all_items) = self.runtime_setting_choice_items(field) {
            self.open_choice_overlay(ChoiceOverlay {
                title: format!("Edit {}", field.label),
                prompt: runtime_setting_edit_prompt(field, current_summary.as_str()),
                footer: ui_text::t(&self.i18n, "overlay-choice-footer"),
                empty_message: ui_text::t(&self.i18n, "overlay-picker-empty"),
                input: Editor::from_text(self.run_options.runtime_setting_input_text(field)),
                filter_query: String::new(),
                all_items,
                items: Vec::new(),
                selected: 0,
                allow_custom: true,
                allow_clear: true,
                action: ChoiceOverlayAction::RuntimeSetting(field),
            });
            return;
        }
        self.overlay = Some(Overlay::RuntimeSettingEdit(RuntimeSettingEditOverlay {
            title: format!("Edit {}", field.label),
            prompt: runtime_setting_edit_prompt(field, current_summary.as_str()),
            input: Editor::from_text(self.run_options.runtime_setting_input_text(field)),
            field,
        }));
    }

    fn settings_field_choice_items(&self, field: SettingsFieldSpec) -> Option<Vec<ChoiceItem>> {
        match field.path {
            "default.provider" => Some(
                self.backend
                    .list_providers()
                    .into_iter()
                    .map(|provider| {
                        choice_item(
                            provider.provider_id,
                            format!(
                                "default {}/{}",
                                provider.default_adapter.as_deref().unwrap_or("adapter"),
                                provider.default_model
                            ),
                        )
                    })
                    .collect(),
            ),
            "default.adapter" => Some(self.default_adapter_choice_items()),
            "default.model" => Some(self.default_model_choice_items()),
            "default.agent" => Some(
                self.backend
                    .list_agent_names()
                    .into_iter()
                    .map(|agent| choice_item(agent, "registered agent profile"))
                    .collect(),
            ),
            "ui.locale" => Some(
                SUPPORTED_LOCALES
                    .iter()
                    .map(|(code, detail)| choice_item(*code, *detail))
                    .collect(),
            ),
            _ if matches!(field.kind, SettingsFieldKind::Bool) => {
                Some(boolean_choice_items("write a boolean override"))
            }
            _ => None,
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
                Ok(rows) => Some(inspector_rows_to_choice_items(rows)),
                Err(error) => {
                    self.flash_warning(error.to_string());
                    Some(Vec::new())
                }
            },
            RuntimeSettingId::SpeedMode => match self
                .backend
                .runtime_speed_mode_rows(&self.run_options.to_request())
            {
                Ok(rows) => Some(inspector_rows_to_choice_items(rows)),
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
                        .map(|value| choice_item(value, "supported by the current model"))
                        .collect(),
                ),
                Err(error) => {
                    self.flash_warning(error.to_string());
                    Some(Vec::new())
                }
            },
            RuntimeSettingId::ParallelToolCalls => {
                Some(boolean_choice_items("apply a parallel-tool-calls override"))
            }
            RuntimeSettingId::Temperature
            | RuntimeSettingId::MaxOutput
            | RuntimeSettingId::System => None,
        }
    }

    fn default_model_choice_items(&self) -> Vec<ChoiceItem> {
        dedupe_choice_items(
            self.backend
                .default_model_options()
                .into_iter()
                .map(|model| {
                    let adapter_id = model
                        .adapter_id
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "adapter".to_owned());
                    let mut detail_parts = vec![format!(
                        "configured on {}/{}",
                        model.provider_id, adapter_id
                    )];
                    if let Some(display_name) = model
                        .display_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        detail_parts.push(display_name.to_owned());
                    }
                    choice_item(model.id.to_string(), detail_parts.join(" · "))
                })
                .collect(),
        )
    }

    fn default_adapter_choice_items(&self) -> Vec<ChoiceItem> {
        self.backend
            .default_adapter_options()
            .into_iter()
            .map(|adapter| {
                let model_label = if adapter.configured_model_count == 1 {
                    "configured model"
                } else {
                    "configured models"
                };
                choice_item(
                    adapter.adapter_id,
                    format!(
                        "enabled on default provider · {} {}",
                        adapter.configured_model_count, model_label
                    ),
                )
            })
            .collect()
    }

    fn provider_studio_field_choice_items(
        &self,
        dialog: &ProviderStudioOverlay,
        field: ProviderStudioField,
    ) -> Option<Vec<ChoiceItem>> {
        match field {
            ProviderStudioField::AuthMode => Some(vec![
                choice_item("none", "disable provider auth metadata"),
                choice_item("api", "API key or endpoint-based provider auth"),
                choice_item(
                    "gitlab_api",
                    "GitLab token auth routed through openai or anthropic",
                ),
                choice_item(
                    "credential",
                    "credential-backed auth resolved from a local issuer",
                ),
                choice_item("bedrock_sigv4", "AWS Bedrock SigV4 signing"),
            ]),
            ProviderStudioField::CredentialIssuer => Some(vec![
                choice_item("openai_chatgpt", "OpenAI ChatGPT credentials"),
                choice_item("github_copilot", "GitHub Copilot credentials"),
                choice_item("gitlab", "GitLab OAuth credentials"),
                choice_item("google_adc", "Google Application Default Credentials"),
                choice_item("sap_ai_core", "SAP AI Core service key auth"),
                choice_item("atomgit", "AtomGit credentials"),
            ]),
            ProviderStudioField::InstanceUrl => Some(vec![choice_item(
                "https://gitlab.com",
                "GitLab.com browser OAuth endpoint",
            )]),
            ProviderStudioField::RedirectUri => Some(vec![
                choice_item(
                    "http://127.0.0.1:1455/callback",
                    "localhost callback URL for copy/paste OAuth redirects",
                ),
                choice_item(
                    "http://127.0.0.1:1455/auth/callback",
                    "localhost callback URL that matches the Studio web route",
                ),
            ]),
            ProviderStudioField::Region => Some(
                AWS_REGION_CHOICES
                    .iter()
                    .map(|region| choice_item(*region, "AWS region"))
                    .collect(),
            ),
            ProviderStudioField::Profile => {
                Some(provider_studio_profile_choice_items(&self.backend))
            }
            ProviderStudioField::ApiKeyEnv => Some(provider_studio_api_key_env_choice_items()),
            ProviderStudioField::ServiceKeyEnv => Some(vec![choice_item(
                "AICORE_SERVICE_KEY",
                "default SAP AI Core service key env var",
            )]),
            ProviderStudioField::DefaultAdapter => Some(
                dialog
                    .adapter_candidate_ids
                    .iter()
                    .map(|adapter_id| {
                        let detail = provider_studio_adapter_rule(dialog, adapter_id.as_str())
                            .map(|rule| {
                                let mut parts = vec![rule.detail.to_owned()];
                                if dialog.configured_adapter_ids.contains(adapter_id) {
                                    parts.push("configured".to_owned());
                                }
                                parts.join(" · ")
                            })
                            .unwrap_or_else(|| {
                                if dialog.configured_adapter_ids.contains(adapter_id) {
                                    "configured on disk; not part of the current auth contract"
                                        .to_owned()
                                } else {
                                    "not supported by the current auth contract".to_owned()
                                }
                            });
                        choice_item(adapter_id.clone(), detail)
                    })
                    .collect(),
            ),
            ProviderStudioField::DefaultModel => {
                Some(provider_studio_default_model_choice_items(dialog))
            }
            _ => None,
        }
    }

    fn open_runtime_config_in_editor(&mut self) {
        let path = self.backend.config_path();
        if !path.exists() {
            if let Some(parent) = path.parent()
                && let Err(error) = fs::create_dir_all(parent)
            {
                self.flash_error(format!(
                    "failed to prepare config directory {}: {error}",
                    parent.display()
                ));
                return;
            }
            if let Err(error) = fs::write(&path, "") {
                self.flash_error(format!(
                    "failed to create config file {}: {error}",
                    path.display()
                ));
                return;
            }
        }
        self.pending_ui_action = Some(UiAction::OpenPath { path });
    }

    fn open_agent_profile_source(&mut self, profile: &AgentProfile) {
        if let Some(path) = profile.source_path.clone() {
            self.pending_ui_action = Some(UiAction::OpenPath { path });
        } else {
            self.open_runtime_config_in_editor();
        }
    }

    fn toggle_plugins_enabled(&mut self, dialog: &mut SettingsStudioOverlay) {
        let next = !dialog.plugins_enabled;
        match self.block_on_async(
            self.backend
                .set_config_setting("plugins.enabled", JsonValue::Bool(next)),
        ) {
            Ok(_) => {
                self.flash_success(if next {
                    "enabled plugins"
                } else {
                    "disabled plugins"
                });
                self.refresh_settings_studio_overlay(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn toggle_tool_description_mode(&mut self, dialog: &mut SettingsStudioOverlay) {
        let next = if dialog.plugins_default_mode == "help" {
            "detailed"
        } else {
            "help"
        };
        match self.block_on_async(self.backend.set_config_setting(
            "plugins.tool_presentation.default_mode",
            JsonValue::String(next.to_string()),
        )) {
            Ok(_) => {
                self.flash_success(format!("set tool description mode to {next}"));
                self.refresh_settings_studio_overlay(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn toggle_plugin_entry_disabled(
        &mut self,
        plugin_id: &str,
        entry: JsonValue,
        disabled: bool,
        dialog: &mut SettingsStudioOverlay,
    ) {
        let Some(mut entry_object) = entry.as_object().cloned() else {
            self.flash_error(format!(
                "plugin entry {plugin_id} is not a JSON object and cannot be rewritten"
            ));
            return;
        };
        entry_object.insert("disabled".to_string(), JsonValue::Bool(!disabled));
        match self.block_on_async(self.backend.set_config_setting(
            &format!("plugins.list.{}", quoted_settings_segment(plugin_id)),
            JsonValue::Object(entry_object),
        )) {
            Ok(_) => {
                self.flash_success(if disabled {
                    format!("enabled plugin {plugin_id}; config kept and runtime reloaded")
                } else {
                    format!("disabled plugin {plugin_id}; config kept and runtime reloaded")
                });
                self.refresh_settings_studio_overlay(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn set_default_agent_value(&mut self, agent_name: &str) -> UiResult<()> {
        self.block_on_async(
            self.backend
                .set_config_setting("default.agent", JsonValue::String(agent_name.to_string())),
        )
        .map(|_| ())
    }

    fn set_default_agent(&mut self, agent_name: &str, dialog: &mut SettingsStudioOverlay) {
        match self.set_default_agent_value(agent_name) {
            Ok(()) => {
                self.flash_success(format!("set default.agent to {agent_name}"));
                self.refresh_settings_studio_overlay(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn set_agent_hidden_value(&mut self, agent_name: &str, hidden: bool) -> UiResult<()> {
        self.block_on_async(self.backend.set_agent_hidden(agent_name, hidden))
            .map(|_| ())
    }

    fn toggle_agent_hidden(
        &mut self,
        agent_name: &str,
        hidden: bool,
        dialog: &mut SettingsStudioOverlay,
    ) {
        match self.set_agent_hidden_value(agent_name, !hidden) {
            Ok(()) => {
                self.flash_success(if hidden {
                    format!("unhid agent {agent_name}")
                } else {
                    format!("hid agent {agent_name}")
                });
                self.refresh_settings_studio_overlay(dialog);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn open_inspector_picker(
        &mut self,
        title: String,
        prompt: String,
        query: &str,
        rows: Vec<crate::backend::InspectorRow>,
    ) {
        let mut overlay = PickerOverlay {
            title,
            prompt,
            empty_message: ui_text::t(&self.i18n, "overlay-picker-empty"),
            footer: ui_text::t(&self.i18n, "overlay-picker-footer"),
            input: Editor::from_text(query.to_string()),
            all_items: rows
                .into_iter()
                .map(|row| PickerItem {
                    label: row.label,
                    detail: row.detail,
                    value: PickerValue::Inspector,
                })
                .collect(),
            items: Vec::new(),
            selected: 0,
            loading: false,
            kind: PickerKind::Inspector,
        };
        Self::refresh_picker_overlay(&mut overlay);
        self.current_route = Route::Picker(overlay);
    }

    fn open_permission_rule_picker(&mut self, query: &str) {
        match self.block_on_async(self.backend.list_permission_rules()) {
            Ok(rules) => {
                let mut all_items = vec![PickerItem {
                    label: ui_text::t(&self.i18n, "permission-rule-create-label"),
                    detail: ui_text::t(&self.i18n, "permission-rule-create-detail"),
                    value: PickerValue::PermissionRuleCreate,
                }];
                all_items.extend(rules.into_iter().map(|rule| PickerItem {
                    label: permission_rule_label(&rule),
                    detail: permission_rule_detail(&rule),
                    value: PickerValue::PermissionRule(Box::new(rule)),
                }));
                let mut overlay = PickerOverlay {
                    title: ui_text::t(&self.i18n, "overlay-permission-rules-title"),
                    prompt: ui_text::t(&self.i18n, "overlay-permission-rules-prompt"),
                    empty_message: ui_text::t(&self.i18n, "overlay-picker-empty"),
                    footer: ui_text::t(&self.i18n, "overlay-permission-rules-footer"),
                    input: Editor::from_text(query.to_string()),
                    all_items,
                    items: Vec::new(),
                    selected: 0,
                    loading: false,
                    kind: PickerKind::PermissionRules,
                };
                Self::refresh_picker_overlay(&mut overlay);
                self.current_route = Route::Picker(overlay);
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn open_permission_rule_editor(
        &mut self,
        rule: Option<&PermissionRuleResource>,
        return_query: &str,
        return_overlay: Option<Overlay>,
    ) {
        let (rule_id, title, input) = match rule {
            Some(rule) => {
                let draft = permission_rule_draft_from_resource(rule);
                let input = Editor::from_text(render_permission_rule_draft(&draft));
                (
                    Some(rule.id),
                    ui_text::t(&self.i18n, "overlay-permission-rule-edit-title"),
                    input,
                )
            }
            None => {
                let draft = PermissionRuleDraft::default();
                let input = Editor::from_text(render_permission_rule_draft(&draft));
                (
                    None,
                    ui_text::t(&self.i18n, "overlay-permission-rule-create-title"),
                    input,
                )
            }
        };
        self.overlay = Some(Overlay::PermissionRuleEdit(PermissionRuleEditOverlay {
            rule_id,
            title,
            prompt: ui_text::t(&self.i18n, "overlay-permission-rule-prompt"),
            input,
            return_query: return_query.to_string(),
            return_overlay: return_overlay.map(Box::new),
        }));
    }

    fn open_permission_rule_editor_from_request(&mut self, request: &PermissionRequest) {
        let draft = permission_rule_draft_from_request(request);
        let input = Editor::from_text(render_permission_rule_draft(&draft));
        self.overlay = Some(Overlay::PermissionRuleEdit(PermissionRuleEditOverlay {
            rule_id: None,
            title: ui_text::t(&self.i18n, "overlay-permission-rule-create-title"),
            prompt: ui_text::t(&self.i18n, "overlay-permission-rule-prompt"),
            input,
            return_query: String::new(),
            return_overlay: None,
        }));
    }

    fn open_revoke_permission_rule_confirm(
        &mut self,
        rule: &PermissionRuleResource,
        return_query: &str,
    ) {
        let label = permission_rule_label(rule);
        self.overlay = Some(Overlay::Confirm(ConfirmOverlay {
            title: ui_text::t(&self.i18n, "overlay-permission-rule-delete-title"),
            body_lines: vec![self.i18n.text_args(
                "overlay-permission-rule-delete-body",
                &crate::fl_args!("name" => label.clone()),
            )],
            footer: ui_text::t(&self.i18n, "overlay-confirm-footer"),
            action: ConfirmAction::RevokePermissionRule {
                rule_id: rule.id,
                label,
                return_query: return_query.to_string(),
            },
        }));
    }

    fn open_worktree_remove_confirm(&mut self, session_id: i64, discard_changes: bool) {
        let mut body_lines = vec![ui_text::t(&self.i18n, "overlay-worktree-remove-body")];
        if discard_changes {
            body_lines.push(ui_text::t(&self.i18n, "overlay-worktree-remove-force"));
        }
        self.overlay = Some(Overlay::Confirm(ConfirmOverlay {
            title: ui_text::t(&self.i18n, "overlay-worktree-remove-title"),
            body_lines,
            footer: ui_text::t(&self.i18n, "overlay-confirm-footer"),
            action: ConfirmAction::ExitWorktree {
                session_id,
                discard_changes,
            },
        }));
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
            self.runtime_entry_command_rows()
                .into_iter()
                .map(|entry| PickerItem {
                    label: format!("/{}", entry.label),
                    detail: entry.detail,
                    value: PickerValue::RuntimeEntry(entry.label),
                }),
        );
        let mut overlay = PickerOverlay {
            title: ui_text::t(&self.i18n, "overlay-commands-title"),
            prompt: ui_text::t(&self.i18n, "overlay-commands-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-empty"),
            footer: ui_text::t(&self.i18n, "overlay-picker-footer"),
            input: Editor::default(),
            all_items,
            items: Vec::new(),
            selected: 0,
            loading: false,
            kind: PickerKind::Commands,
        };
        Self::refresh_picker_overlay(&mut overlay);
        self.current_route = Route::Picker(overlay);
    }

    fn runtime_entry_command_rows(&self) -> Vec<crate::backend::InspectorRow> {
        self.backend
            .runtime_entry_rows()
            .into_iter()
            .filter(|entry| commands::find_command(entry.label.as_str()).is_none())
            .collect()
    }

    fn open_resume_session_picker(&mut self) {
        self.open_resume_session_picker_with_query("");
    }

    fn open_resume_session_picker_with_query(&mut self, query: &str) {
        let mut input = Editor::from_text(query.trim().to_string());
        input.cursor = input.text().len();
        let scope_session_id = (self.sessions.view_mode == SessionViewMode::Subtree)
            .then(|| self.current_or_selected_session_id())
            .flatten();
        let mut dialog = SessionSearchOverlay {
            title: ui_text::t(&self.i18n, "overlay-resume-title"),
            prompt: ui_text::t(&self.i18n, "overlay-resume-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            footer: String::new(),
            input,
            items: Vec::new(),
            all_items: Vec::new(),
            selected: 0,
            loading: true,
            mode: self.sessions.view_mode,
            scope_session_id,
            page_limit: 50,
            page_index: 0,
            offset: 0,
            cursors: vec![None],
            next_cursor: None,
            has_more: false,
        };
        dialog.footer = self.session_search_footer(&dialog);
        match dialog.mode {
            SessionViewMode::Subtree => {
                let Some(session_id) = dialog.scope_session_id else {
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
                    dialog.mode,
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
        self.current_route = Route::Picker(PickerOverlay {
            title: self.i18n.text_args(
                "overlay-lineage-title",
                &crate::fl_args!("session" => session_id),
            ),
            prompt: ui_text::t(&self.i18n, "overlay-lineage-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            footer: ui_text::t(&self.i18n, "overlay-picker-footer"),
            input: Editor::default(),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            loading: true,
            kind: PickerKind::Lineage { session_id },
        });
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
        self.current_route = Route::Picker(PickerOverlay {
            title: self.i18n.text_args(
                "overlay-rewind-title",
                &crate::fl_args!("session" => session_id),
            ),
            prompt: ui_text::t(&self.i18n, "overlay-rewind-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            footer: ui_text::t(&self.i18n, "overlay-picker-footer"),
            input: Editor::default(),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            loading: true,
            kind: PickerKind::RewindMessages { session_id },
        });
        self.request_rewind_messages(session_id);
    }

    fn open_provider_picker(&mut self, purpose: ProviderPickerPurpose) {
        self.current_route = Route::Picker(PickerOverlay {
            title: ui_text::t(&self.i18n, "overlay-providers-title"),
            prompt: ui_text::t(&self.i18n, "overlay-providers-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            footer: ui_text::t(&self.i18n, "overlay-picker-footer"),
            input: Editor::default(),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            loading: true,
            kind: PickerKind::Providers(purpose),
        });
        self.request_providers(purpose);
    }

    fn open_session_model_chooser(&mut self) {
        self.current_route = Route::SessionModelChooser(SessionModelChooserOverlay {
            title: "Session Model".to_string(),
            prompt: "Search provider, adapter, or model".to_string(),
            footer: "Type filter | Up/Down move | Left/Right page | Enter select | Esc close"
                .to_string(),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            input: Editor::default(),
            loading: true,
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            page_size: 18,
        });
        self.request_session_model_chooser_items();
    }

    fn open_provider_studio(&mut self, initial_provider: Option<&str>) {
        let providers = self.backend.list_configured_providers();
        let provider_rows = provider_studio_provider_rows(providers.as_slice());
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
            providers: provider_rows,
            selected_provider,
            focus: ProviderStudioFocus::Fields,
            selected_field: 0,
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
            selected_adapter: 0,
            selected_model: 0,
            adapter_selection_touched: false,
            selected_adapter_ids: BTreeSet::new(),
            selected_model_keys: BTreeSet::new(),
            catalog_matches: BTreeMap::new(),
            listing_adapter_models: false,
            saving: false,
            pending_adapter_models_key: None,
            pending_auth_key: None,
            detail_page: None,
            editor: None,
        };
        let selected_id = overlay
            .providers
            .get(overlay.selected_provider)
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
                }
                dialog.draft = draft;
                dialog.selected_field = 0;
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
                dialog.selected_adapter = 0;
                dialog.selected_model = 0;
                dialog.pending_adapter_models_key = None;
                dialog.pending_auth_key = None;
                dialog.detail_page = None;
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
                    dialog.selected_adapter = index;
                } else if let Some(first_selected) = dialog
                    .adapter_candidate_ids
                    .iter()
                    .position(|candidate| dialog.selected_adapter_ids.contains(candidate.as_str()))
                {
                    dialog.selected_adapter = first_selected;
                }
            }
            Err(error) => self.flash_error(error.to_string()),
        }
    }

    fn open_model_catalog_studio(&mut self) {
        let dialog = ModelCatalogStudioOverlay {
            title: "Model Catalog".to_string(),
            footer: ui_text::t(&self.i18n, "overlay-model-catalog-footer"),
            query: String::new(),
            items: Vec::new(),
            summary: ModelCatalogResponse {
                refreshing: false,
                last_refresh_at: None,
                last_successful_source: None,
                last_error: None,
                entry_count: 0,
            },
            total: 0,
            offset: 0,
            limit: 50,
            loading: true,
            selected: 0,
            editor: None,
        };
        self.request_model_catalog_page(String::new(), 0);
        self.current_route = Route::ModelCatalogStudio(dialog.clone());
    }

    fn request_model_catalog_page(&mut self, query: String, offset: usize) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_model_catalog_entries(query.as_str(), offset, 50)
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
            self.flash_warning(format!(
                "live model listing is unavailable for auth {}",
                dialog.draft.auth_kind.label()
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
                self.flash_error(format!(
                    "Draft model listing only supports live HTTP adapters. Unsupported: {}",
                    unsupported.join(", ")
                ));
                return;
            }
        }

        let request_key = provider_studio_request_key(&dialog.draft, &adapter_ids);
        dialog.pending_adapter_models_key = Some(request_key.clone());
        dialog.listing_adapter_models = true;
        let backend = self.backend.clone();
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
                Err(format!(
                    "listing adapter models requires api/gitlab_api auth or an existing saved provider; current auth is {}",
                    draft.auth_kind.label()
                ))
            };
            let _ = tx.send(AppMessage::ProviderStudioAdapterModelsLoaded {
                request_key,
                result,
            });
        });
    }

    fn request_provider_studio_start_auth(&mut self, dialog: &mut ProviderStudioOverlay) {
        let request_key = provider_studio_auth_request_key(&dialog.draft, "start");
        dialog.pending_auth_key = Some(request_key.clone());
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let draft = dialog.draft.clone();
        tokio::spawn(async move {
            let result = backend
                .start_provider_draft_auth(draft)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ProviderStudioAuthCompleted {
                request_key,
                result,
            });
        });
    }

    fn request_provider_studio_continue_auth(&mut self, dialog: &mut ProviderStudioOverlay) {
        let request_key = provider_studio_auth_request_key(&dialog.draft, "continue");
        dialog.pending_auth_key = Some(request_key.clone());
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let draft = dialog.draft.clone();
        tokio::spawn(async move {
            let result = backend
                .continue_provider_draft_auth(draft)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ProviderStudioAuthCompleted {
                request_key,
                result,
            });
        });
    }

    fn request_session_model_chooser_items(&mut self) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = async {
                let providers = backend.list_providers();
                let mut items = Vec::new();
                for provider in providers {
                    let default_adapter = provider.default_adapter.clone();
                    let provider_id = provider.provider_id.clone();
                    let models = backend
                        .list_provider_models(provider_id.as_str())
                        .await
                        .map_err(|error| error.to_string())?;
                    for model in models {
                        items.push(session_model_choice_item(
                            provider_id.as_str(),
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
            .await;
            let _ = tx.send(AppMessage::SessionModelChooserLoaded { result });
        });
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
                .await
                .map_err(|error| error.to_string());
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
                .await
                .map_err(|error| error.to_string());
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
                .await
                .map_err(|error| error.to_string());
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
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ProviderStudioSaved {
                provider_id: draft.provider_id.clone(),
                result,
            });
        });
    }

    fn move_provider_studio_selection(&mut self, dialog: &mut ProviderStudioOverlay, delta: isize) {
        let current = self.current_provider_studio_selection(dialog);
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize)
        };
        self.set_provider_studio_selection(dialog, next);
    }

    fn set_provider_studio_selection(&mut self, dialog: &mut ProviderStudioOverlay, target: usize) {
        match dialog.focus {
            ProviderStudioFocus::Fields => {
                dialog.selected_field = min(
                    target,
                    provider_studio_visible_fields(dialog)
                        .len()
                        .saturating_sub(1),
                );
            }
            ProviderStudioFocus::Adapters => {
                dialog.selected_adapter =
                    min(target, dialog.adapter_candidate_ids.len().saturating_sub(1));
                dialog.selected_model = min(
                    dialog.selected_model,
                    provider_studio_selected_adapter_models(dialog)
                        .map(|adapter| adapter.models.len().saturating_sub(1))
                        .unwrap_or_default(),
                );
            }
            ProviderStudioFocus::Models => {
                let max_index = provider_studio_selected_adapter_models(dialog)
                    .map(|adapter| adapter.models.len().saturating_sub(1))
                    .unwrap_or_default();
                dialog.selected_model = min(target, max_index);
            }
        }
    }

    fn current_provider_studio_selection(&self, dialog: &ProviderStudioOverlay) -> usize {
        match dialog.focus {
            ProviderStudioFocus::Fields => dialog.selected_field,
            ProviderStudioFocus::Adapters => dialog.selected_adapter,
            ProviderStudioFocus::Models => dialog.selected_model,
        }
    }

    fn open_provider_studio_detail_page(&mut self, dialog: &mut ProviderStudioOverlay) {
        if provider_studio_detail_fields(dialog).is_empty() {
            self.flash_warning("no auth details are available for the current auth mode");
            return;
        }
        dialog.detail_page = Some(ProviderStudioDetailPage {
            title: ui_text::t(&self.i18n, "overlay-provider-studio-detail"),
            footer: ui_text::t(&self.i18n, "overlay-provider-studio-detail-footer"),
            selected_field: 0,
        });
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
            self.open_choice_overlay(ChoiceOverlay {
                title: ui_text::t(&self.i18n, "overlay-provider-studio-edit-title"),
                prompt: provider_studio_field_prompt(&self.i18n, field),
                footer: ui_text::t(&self.i18n, "overlay-choice-footer"),
                empty_message: ui_text::t(&self.i18n, "overlay-picker-empty"),
                input: Editor::from_text(provider_studio_field_value(&dialog.draft, field)),
                filter_query: String::new(),
                all_items,
                items: Vec::new(),
                selected: 0,
                allow_custom: true,
                allow_clear: provider_studio_field_allows_clear(field),
                action: ChoiceOverlayAction::ProviderStudioField(field),
            });
            return;
        }
        dialog.editor = Some(ProviderStudioEditor {
            title: ui_text::t(&self.i18n, "overlay-provider-studio-edit-title"),
            prompt: provider_studio_field_prompt(&self.i18n, field),
            footer: ui_text::t(&self.i18n, "overlay-provider-studio-edit-footer"),
            multiline: false,
            input: Editor::from_text(provider_studio_field_value(&dialog.draft, field)),
            action: ProviderStudioEditorAction::Field(field),
        });
    }

    fn activate_provider_studio_detail_page_selection(
        &mut self,
        dialog: &mut ProviderStudioOverlay,
    ) {
        let Some(selected_field) = dialog.detail_page.as_ref().map(|page| page.selected_field)
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
                self.request_provider_studio_start_auth(dialog);
                false
            }
            KeyCode::Char('p') | KeyCode::Char('P') if dialog.draft.supports_interactive_auth() => {
                self.request_provider_studio_continue_auth(dialog);
                false
            }
            KeyCode::Enter => {
                self.activate_provider_studio_detail_page_selection(dialog);
                false
            }
            KeyCode::PageUp => {
                detail_page.selected_field = detail_page.selected_field.saturating_sub(10);
                false
            }
            KeyCode::PageDown => {
                detail_page.selected_field = min(
                    detail_page.selected_field.saturating_add(10),
                    field_count.saturating_sub(1),
                );
                false
            }
            KeyCode::Home => {
                detail_page.selected_field = 0;
                false
            }
            KeyCode::End => {
                detail_page.selected_field = field_count.saturating_sub(1);
                false
            }
            KeyCode::Up | KeyCode::Char('k') => {
                detail_page.selected_field = detail_page.selected_field.saturating_sub(1);
                false
            }
            KeyCode::Down | KeyCode::Char('j') => {
                detail_page.selected_field = min(
                    detail_page.selected_field.saturating_add(1),
                    field_count.saturating_sub(1),
                );
                false
            }
            _ => false,
        }
    }

    fn activate_provider_studio_focus(&mut self, dialog: &mut ProviderStudioOverlay) {
        match dialog.focus {
            ProviderStudioFocus::Fields => {
                let fields = provider_studio_visible_fields(dialog);
                let Some(field) = fields.get(dialog.selected_field).copied() else {
                    return;
                };
                match field {
                    ProviderStudioField::StartAuthAction => {
                        self.request_provider_studio_start_auth(dialog);
                    }
                    ProviderStudioField::ContinueAuthAction => {
                        self.request_provider_studio_continue_auth(dialog);
                    }
                    ProviderStudioField::EditAuthDetailsAction => {
                        self.open_provider_studio_detail_page(dialog);
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
                if let Some(adapter_models) = provider_studio_selected_adapter_models(dialog)
                    && let Some(model) = adapter_models.models.get(dialog.selected_model).cloned()
                {
                    let adapter_id = adapter_models.adapter_id.clone();
                    match self.backend.provider_model_draft_value(
                        &dialog.draft,
                        adapter_id.as_str(),
                        model.id.as_str(),
                        Some(&model),
                    ) {
                        Ok(model_value) => {
                            let text = serde_json::to_string_pretty(&model_value)
                                .unwrap_or_else(|_| "{}".to_owned());
                            dialog.editor = Some(ProviderStudioEditor {
                                title: format!("Model Config · {adapter_id}/{}", model.id),
                                prompt: "Edit the persisted provider model JSON.".to_string(),
                                footer: ui_text::t(
                                    &self.i18n,
                                    "overlay-provider-studio-model-edit-footer",
                                ),
                                multiline: true,
                                input: Editor::from_text(text),
                                action: ProviderStudioEditorAction::ModelJson {
                                    adapter_id,
                                    model_id: model.id.to_string(),
                                },
                            });
                        }
                        Err(error) => self.flash_error(error.to_string()),
                    }
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
            }
            ProviderStudioField::AuthStatus
            | ProviderStudioField::StartAuthAction
            | ProviderStudioField::ContinueAuthAction
            | ProviderStudioField::EditAuthDetailsAction => {}
            ProviderStudioField::AuthMode => {
                match ProviderDraftAuthKind::parse_mode(
                    value.as_str(),
                    dialog.draft.auth_kind.credential_issuer(),
                ) {
                    Ok(auth_kind) => {
                        dialog.draft.auth_kind = auth_kind;
                        dialog.draft.normalize_shape();
                        self.refresh_provider_studio_adapter_state(dialog);
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            ProviderStudioField::CredentialIssuer => {
                let mode_value = if value.trim().is_empty() {
                    "credential".to_owned()
                } else {
                    format!("credential:{value}")
                };
                match ProviderDraftAuthKind::parse_mode(
                    mode_value.as_str(),
                    dialog.draft.auth_kind.credential_issuer(),
                ) {
                    Ok(auth_kind) => {
                        dialog.draft.auth_kind = auth_kind;
                        dialog.draft.auth.credential_issuer = value.trim().to_owned();
                        dialog.draft.normalize_shape();
                        self.refresh_provider_studio_adapter_state(dialog);
                    }
                    Err(error) => return Err(error.to_string()),
                }
            }
            ProviderStudioField::BaseUrl => {
                dialog.draft.auth.base_url = value;
            }
            ProviderStudioField::InstanceUrl => {
                dialog.draft.auth.instance_url = value;
            }
            ProviderStudioField::ApiKeyEnv => {
                dialog.draft.auth.api_key_env = value;
            }
            ProviderStudioField::ApiKey => {
                dialog.draft.auth.api_key = value;
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
            ProviderStudioField::Username => {
                dialog.draft.credential_drafts.atomgit.username = value;
            }
            ProviderStudioField::DisplayName => {
                dialog.draft.credential_drafts.atomgit.display_name = value;
            }
            ProviderStudioField::Email => {
                dialog.draft.credential_drafts.atomgit.email = value;
            }
            ProviderStudioField::AvatarUrl => {
                dialog.draft.credential_drafts.atomgit.avatar_url = value;
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
        dialog.selected_field = min(
            dialog.selected_field,
            provider_studio_visible_fields(dialog)
                .len()
                .saturating_sub(1),
        );
        let detail_field_count = provider_studio_detail_fields(dialog).len();
        if let Some(detail_page) = dialog.detail_page.as_mut() {
            detail_page.selected_field = min(
                detail_page.selected_field,
                detail_field_count.saturating_sub(1),
            );
        }
        dialog.selected_adapter_ids.retain(|adapter_id| {
            dialog
                .adapter_candidate_ids
                .iter()
                .any(|candidate| candidate == adapter_id)
                && selectable_adapter_ids.contains(adapter_id)
        });
        if !dialog.adapter_selection_touched && dialog.selected_adapter_ids.is_empty() {
            dialog.selected_adapter_ids = selectable_adapter_ids.clone();
        }
        dialog.selected_adapter = min(
            dialog.selected_adapter,
            dialog.adapter_candidate_ids.len().saturating_sub(1),
        );
        dialog.selected_model = min(
            dialog.selected_model,
            provider_studio_selected_adapter_models(dialog)
                .map(|adapter| adapter.models.len().saturating_sub(1))
                .unwrap_or_default(),
        );
        if !dialog.adapter_models.is_empty() {
            provider_studio_restore_model_selection(dialog);
        }
        provider_studio_ensure_default_selection(dialog);
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
        let catalog_entries = self.backend.lookup_model_catalog_entries(&lookup_ids);
        dialog.catalog_matches = dialog
            .adapter_models
            .iter()
            .flat_map(|adapter| {
                adapter.models.iter().filter_map(|model| {
                    provider_studio_catalog_match_entry(model, &catalog_entries).map(|entry| {
                        (
                            provider_studio_model_key(
                                adapter.adapter_id.as_str(),
                                model.id.as_str(),
                            ),
                            entry.clone(),
                        )
                    })
                })
            })
            .collect();
    }

    fn refresh_provider_studio_adapter_state(&mut self, dialog: &mut ProviderStudioOverlay) {
        dialog.adapter_models.clear();
        dialog.selected_model_keys.clear();
        dialog.catalog_matches.clear();
        self.sync_provider_studio_shape(dialog);
        dialog.selected_model = 0;
        dialog.pending_adapter_models_key = None;
        dialog.listing_adapter_models = false;
    }

    fn open_child_sessions_picker(&mut self) {
        let Some(parent_session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.current_route = Route::Picker(PickerOverlay {
            title: self.i18n.text_args(
                "overlay-children-title",
                &crate::fl_args!("session" => parent_session_id),
            ),
            prompt: ui_text::t(&self.i18n, "overlay-children-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            footer: ui_text::t(&self.i18n, "overlay-picker-footer"),
            input: Editor::default(),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            loading: true,
            kind: PickerKind::ChildSessions { parent_session_id },
        });
        self.request_child_sessions(parent_session_id);
    }

    fn open_rewind_confirm_overlay(&mut self, session_id: i64, message_id: i64, target: String) {
        self.overlay = Some(Overlay::Confirm(ConfirmOverlay {
            title: ui_text::t(&self.i18n, "overlay-rewind-confirm-title"),
            body_lines: vec![
                self.i18n.text_args(
                    "overlay-rewind-confirm-keep",
                    &crate::fl_args!("target" => target.clone()),
                ),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-warning"),
                ui_text::t(&self.i18n, "overlay-rewind-confirm-draft"),
            ],
            footer: ui_text::t(&self.i18n, "overlay-confirm-footer"),
            action: ConfirmAction::Rewind {
                session_id,
                message_id,
                target,
            },
        }));
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
        let query = dialog.input.text().trim().to_ascii_lowercase();
        dialog.items = dialog
            .all_items
            .iter()
            .filter(|item| Self::picker_item_matches(&dialog.kind, item, query.as_str()))
            .cloned()
            .collect();
        dialog.selected = min(dialog.selected, dialog.items.len().saturating_sub(1));
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
        let query = dialog.input.text().trim().to_ascii_lowercase();
        dialog.items = dialog
            .all_items
            .iter()
            .filter(|item| query.is_empty() || item.search_text.contains(query.as_str()))
            .cloned()
            .collect();
        dialog.selected = min(dialog.selected, dialog.items.len().saturating_sub(1));
    }

    fn reload_plugin_inspector_overlay(&self, dialog: &mut PluginInspectorOverlay) {
        dialog.all_items = self
            .backend
            .plugin_statuses()
            .into_iter()
            .map(|status| {
                let plugin_id = status.plugin_id.clone();
                let inspect = self.backend.plugin_inspect(plugin_id.as_str());
                let logs =
                    self.backend
                        .plugin_logs(plugin_id.as_str(), None, PLUGIN_INSPECTOR_LOG_LIMIT);
                build_plugin_inspector_item(status, inspect, logs)
            })
            .collect();
        Self::refresh_plugin_inspector_overlay(dialog);
    }

    fn refresh_plugin_inspector_overlay(dialog: &mut PluginInspectorOverlay) {
        let query = dialog.input.text().trim().to_ascii_lowercase();
        dialog.items = dialog
            .all_items
            .iter()
            .filter(|item| query.is_empty() || item.search_text.contains(query.as_str()))
            .cloned()
            .collect();
        dialog.selected = min(dialog.selected, dialog.items.len().saturating_sub(1));
    }

    fn picker_item_matches(kind: &PickerKind, item: &PickerItem, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        match (kind, &item.value) {
            (PickerKind::Commands, PickerValue::Command(spec)) => {
                commands::command_matches_query(spec, query)
                    || item.detail.to_ascii_lowercase().contains(query)
            }
            (PickerKind::Commands, PickerValue::RuntimeEntry(_)) => {
                item.label.to_ascii_lowercase().contains(query)
                    || item.detail.to_ascii_lowercase().contains(query)
            }
            (PickerKind::Lineage { .. }, PickerValue::Session(session_id)) => {
                item.label.to_ascii_lowercase().contains(query)
                    || item.detail.to_ascii_lowercase().contains(query)
                    || format!("#{session_id}").contains(query)
            }
            (PickerKind::RewindMessages { .. }, PickerValue::Message(message_id)) => {
                item.label.to_ascii_lowercase().contains(query)
                    || item.detail.to_ascii_lowercase().contains(query)
                    || format!("#{message_id}").contains(query)
            }
            _ => {
                item.label.to_ascii_lowercase().contains(query)
                    || item.detail.to_ascii_lowercase().contains(query)
            }
        }
    }

    fn handle_picker_selection(&mut self, kind: PickerKind, item: PickerItem) {
        match (kind, item.value) {
            (PickerKind::Commands, PickerValue::Command(spec)) => {
                self.execute_command(spec, "");
            }
            (PickerKind::Commands, PickerValue::RuntimeEntry(entry_name)) => {
                self.composer
                    .set_text(format!("/{entry_name} ").trim_end().to_string());
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
                self.open_permission_rule_editor(None, "", None);
            }
            (PickerKind::PermissionRules, PickerValue::PermissionRule(rule)) => {
                self.open_permission_rule_editor(Some(&rule), "", None);
            }
            (PickerKind::Inspector, PickerValue::Inspector) => {}
            _ => {}
        }
    }

    fn apply_provider_override(&mut self, provider: ProviderSummaryResource) {
        self.run_options.model = Some(match provider.default_adapter.clone() {
            Some(adapter_id) => ModelRef::new_with_adapter(
                provider.provider_id.clone(),
                adapter_id,
                provider.default_model.clone(),
            ),
            None => ModelRef::new(provider.provider_id.clone(), provider.default_model.clone()),
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
                "model" => provider.default_model,
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

    fn session_is_busy(&self, session_id: i64) -> bool {
        self.submitting_session_ids.contains(&session_id)
            || (self.transcript.session_id == Some(session_id) && self.transcript.submitting)
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
            ConfirmAction::ExitWorktree {
                session_id,
                discard_changes,
            } => match self
                .backend
                .exit_worktree(session_id, "remove".to_string(), discard_changes)
            {
                Ok(output) => self.flash_success(format!(
                    "worktree {}: {}",
                    output.action.as_deref().unwrap_or("exited"),
                    output.path
                )),
                Err(error) => self.flash_error(error.to_string()),
            },
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
                    self.overlay = Some(Overlay::TranscriptSearch(LineInputOverlay {
                        title: ui_text::t(&self.i18n, "overlay-transcript-search-title"),
                        prompt: ui_text::t(&self.i18n, "overlay-transcript-search-prompt"),
                        input: Editor::from_text(self.transcript.search_query.clone()),
                    }));
                } else {
                    self.transcript.set_search_query(args.trim().to_string());
                    self.jump_search_match(true);
                }
            }
            CommandId::Rename => self.handle_rename_command(spec, args),
            CommandId::Timeline => self.handle_timeline_command(spec, args),
            CommandId::Plugins => self.handle_plugins_command(spec, args),
            CommandId::Settings => self.handle_settings_command(args),
            CommandId::Model => self.open_session_model_chooser(),
            CommandId::Review => self.handle_review_command(args),
            CommandId::Worktree => self.handle_worktree_command(args),
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

    fn execute_runtime_entry_prompt(&mut self, entry_name: &str, args: &str) {
        let target_session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
            .unwrap_or(-1);
        let prompt = match self
            .backend
            .runtime_entry_prompt(target_session_id, entry_name, args)
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
        let title = format!("btw: {}", derive_session_title(question));
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
        self.open_plugin_inspector_overlay(args.trim());
    }

    fn handle_settings_command(&mut self, args: &str) {
        self.open_settings_studio(args.trim());
    }

    fn handle_review_command(&mut self, args: &str) {
        self.execute_runtime_entry_prompt("review", args);
    }

    fn handle_worktree_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("list") {
            self.open_inspector_picker(
                "Worktree".to_string(),
                "Inspect active and managed worktrees".to_string(),
                "",
                self.backend.worktree_inspector_rows(),
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
                    self.backend.enter_worktree(session_id, None, None)
                } else {
                    self.backend
                        .enter_worktree(session_id, Some(argument.to_string()), None)
                };
                match result {
                    Ok(output) => self.flash_success(format!(
                        "worktree ready: {} ({})",
                        output.path,
                        output.branch.as_deref().unwrap_or("unknown")
                    )),
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "attach" => {
                let path = rest.trim();
                if path.is_empty() {
                    self.flash_warning(self.i18n.text_args(
                        "flash-command-usage",
                        &crate::fl_args!("usage" => "/worktree attach <path>"),
                    ));
                    return;
                }
                match self
                    .backend
                    .enter_worktree(session_id, None, Some(path.to_string()))
                {
                    Ok(output) => self.flash_success(format!(
                        "worktree attached: {} ({})",
                        output.path,
                        output.branch.as_deref().unwrap_or("unknown")
                    )),
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "exit" | "leave" => {
                let exit_args = rest.trim();
                let (mode, extra) = split_command_args_once(exit_args).unwrap_or((exit_args, ""));
                match mode.to_ascii_lowercase().as_str() {
                    "" | "keep" => match self.backend.exit_worktree(session_id, "keep".to_string(), false) {
                        Ok(output) => self.flash_success(format!(
                            "worktree {}: {}",
                            output.action.as_deref().unwrap_or("exited"),
                            output.path
                        )),
                        Err(error) => self.flash_error(error.to_string()),
                    },
                    "remove" => {
                        let discard_changes =
                            matches!(extra.trim().to_ascii_lowercase().as_str(), "force" | "discard");
                        self.open_worktree_remove_confirm(session_id, discard_changes);
                    }
                    _ => {
                        self.flash_warning(self.i18n.text_args(
                            "flash-command-usage",
                            &crate::fl_args!("usage" => "/worktree exit [keep|remove [force]]"),
                        ));
                    }
                }
            }
            _ => self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => "/worktree [list|enter [name]|attach <path>|exit [keep|remove [force]]]"),
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
                self.flash_success(format!(
                    "commit created: {} {}",
                    &commit[..commit.len().min(12)],
                    summary
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
            Ok(url) => self.flash_success(format!("pull request created: {url}")),
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
        self.sessions.items = build_visible_session_items(
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
                .summary()
                .unwrap_or_else(|| ui_text::t(&self.i18n, "runtime-status-default")),
        ];
        parts.extend(
            self.current_execution_context_parts()
                .into_iter()
                .filter(|part| !part.starts_with("cwd=")),
        );
        parts.push(self.workspace_context_label());
        parts.push(format!(
            "keys q={} send={}",
            self.keybindings.queue.len(),
            self.keybindings.submit.len()
        ));
        parts.push(format!(
            "statusline {}",
            if self.backend.plugin_statusline_segments().is_empty() {
                "default"
            } else {
                "plugin"
            }
        ));
        let tui_blocks = self.backend.plugin_tui_content_blocks().len();
        if tui_blocks > 0 {
            parts.push(format!("tui_blocks {tui_blocks}"));
        }
        if let Some(theme) = self.plugin_theme.as_ref() {
            parts.push(format!("theme {}", theme.id));
        }
        self.i18n.text_args(
            "flash-runtime-status",
            &crate::fl_args!("summary" => parts.join(" | ")),
        )
    }

    fn current_diagnostics_summary(&self) -> String {
        let runtime = self
            .run_options
            .summary()
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
        format!("ws {}", self.backend.workspace_name())
    }

    fn current_execution_context_parts(&self) -> Vec<String> {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return Vec::new();
        };

        let mut parts = Vec::new();
        parts.push(format!("state={}", session_workflow_state_label(execution)));
        if let Some(agent_profile) = execution.execution.agent_profile.as_deref()
            && !agent_profile.trim().is_empty()
        {
            parts.push(format!("agent={agent_profile}"));
        }
        if let Some(skill_name) = execution.execution.active_skill_name.as_deref()
            && !skill_name.trim().is_empty()
        {
            parts.push(format!("skill={skill_name}"));
        }
        if let Some(task_id) = execution.execution.task_id.as_deref()
            && !task_id.trim().is_empty()
        {
            parts.push(format!("task={task_id}"));
        }
        if execution.execution.model_provider_id.is_some() || execution.execution.model_id.is_some()
        {
            let provider = execution
                .execution
                .model_provider_id
                .as_deref()
                .unwrap_or("auto");
            let model = execution.execution.model_id.as_deref().unwrap_or("default");
            parts.push(format!("model={provider}/{model}"));
        }
        if let Some(thinking_mode) = execution.execution.model_thinking_mode.as_deref()
            && !thinking_mode.trim().is_empty()
        {
            parts.push(format!("thinking={thinking_mode}"));
        }
        if let Some(speed_mode) = execution.execution.model_speed_mode.as_deref()
            && !speed_mode.trim().is_empty()
        {
            parts.push(format!("speed={speed_mode}"));
        }
        if let Some(verbosity) = execution.execution.model_verbosity.as_deref()
            && !verbosity.trim().is_empty()
        {
            parts.push(format!("verbosity={verbosity}"));
        }
        if let Some(parallel_tool_calls) = execution.execution.model_parallel_tool_calls {
            parts.push(format!(
                "parallel_tools={}",
                if parallel_tool_calls { "on" } else { "off" }
            ));
        }
        if let Some(workspace_root) = execution.execution.effective_workspace_root.as_deref()
            && !workspace_root.trim().is_empty()
        {
            parts.push(format!("cwd={workspace_root}"));
        }
        if !execution.execution.allowed_tools.is_empty() {
            parts.push(format!("tools={}", execution.execution.allowed_tools.len()));
        }
        let (permission_count, user_input_count) =
            pending_interactive_counts_for_execution(execution);
        if permission_count > 0 {
            parts.push(format!("perm={permission_count}"));
        }
        if user_input_count > 0 {
            parts.push(format!("input={user_input_count}"));
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
            let model_part = if let (Some(provider_id), Some(model_id)) = (
                execution.execution.model_provider_id.as_deref(),
                execution.execution.model_id.as_deref(),
            ) {
                Some(
                    execution
                        .execution
                        .model_adapter_id
                        .as_deref()
                        .map(|adapter_id| format!("{provider_id}/{adapter_id}/{model_id}"))
                        .unwrap_or_else(|| format!("{provider_id}/{model_id}")),
                )
            } else {
                fallback_model()
            };
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
                parts.push(format!("thinking {thinking_mode}"));
            }
            if let Some(speed_mode) = execution.execution.model_speed_mode.as_deref()
                && !speed_mode.trim().is_empty()
            {
                parts.push(format!("speed {speed_mode}"));
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
            parts.push(format!("thinking {thinking_mode}"));
        }
        if let Some(speed_mode) = self.run_options.speed_mode.as_deref()
            && !speed_mode.trim().is_empty()
        {
            parts.push(format!("speed {speed_mode}"));
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
                if let Some(editor) = dialog.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::AgentPermissionStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_mut() {
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
            Route::PluginInspector(dialog) => dialog.input.flush_pending_input_if_due(now),
            Route::ProviderStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
            Route::ModelCatalogStudio(dialog) => {
                if let Some(editor) = dialog.editor.as_mut() {
                    editor.input.flush_pending_input_if_due(now);
                }
            }
        }
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
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
                    Self::sync_choice_overlay_query(dialog, true);
                }
                Overlay::PermissionRuleEdit(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::FileAttach(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::UserInputReply(dialog) => {
                    if dialog.editing_custom {
                        dialog.custom_input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::SessionSearch(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::Picker(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::SessionModelChooser(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                    Self::refresh_session_model_chooser_overlay(dialog, false, None);
                }
                Overlay::Timeline(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::ProviderStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::ModelCatalogStudio(dialog) => {
                    if let Some(editor) = dialog.editor.as_mut() {
                        editor.input.flush_pending_input_if_due(now);
                    }
                }
                Overlay::Confirm(_) => {}
                Overlay::Permission(_) => {}
            }
        }
    }

    fn refresh_file_attach_overlay(&self, dialog: &mut FileAttachOverlay) {
        dialog.results = self
            .backend
            .search_workspace_files(dialog.input.text(), 24)
            .unwrap_or_default();
        dialog.selected = min(dialog.selected, dialog.results.len().saturating_sub(1));
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
            format!(
                "failed to inspect attachment {}: {error}",
                resolved.display()
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
            .map_err(|error| format!("failed to save composer drafts: {error}"))?;
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
            self.report_prompt_history_error(format!("failed to save prompt history: {error}"));
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
                .ok_or_else(|| "composer placeholder range is invalid".to_string())?;
            if actual_placeholder != element.placeholder {
                return Err("composer placeholder state is out of sync".to_string());
            }

            let item = items_by_placeholder
                .remove(element.placeholder.as_str())
                .ok_or_else(|| format!("missing staged item for {}", element.placeholder))?;
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
                render_message(message, u16::MAX, &self.i18n)
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
        if let Some(summary) = self.run_options.summary() {
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
                return Err(format!("export path is a directory: {}", path.display()));
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
            return [
                "No session selected.",
                "Press Alt+S to pick a session, or start typing in the composer to create one.",
            ]
            .join("\n");
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

fn settings_studio_general_items(sources: &ConfigJsonSources) -> Vec<SettingsStudioItem> {
    SETTINGS_FIELDS
        .iter()
        .map(|field| {
            let file_value =
                get_json_path(&sources.file, Some(field.path)).unwrap_or(JsonValue::Null);
            let effective_value =
                get_json_path(&sources.effective, Some(field.path)).unwrap_or(JsonValue::Null);
            SettingsStudioItem {
                label: field.path.to_string(),
                value: format_setting_value_inline(&effective_value),
                detail: format!(
                    "{} · {}",
                    field.description,
                    format_setting_field_summary(&file_value, &effective_value)
                ),
                action: SettingsPickerAction::EditField(*field),
            }
        })
        .collect()
}

fn settings_studio_runtime_items(run_options: &RunOptionsState) -> Vec<SettingsStudioItem> {
    let runtime_model = run_options
        .model
        .as_ref()
        .map(|model| format!("{}/{}", model.provider_id, model.model_id))
        .unwrap_or_else(|| "default".to_string());
    let runtime_provider = run_options
        .model
        .as_ref()
        .map(|model| model.provider_id.to_string())
        .unwrap_or_else(|| "default".to_string());
    let mut items = vec![
        SettingsStudioItem {
            label: "Provider Override".to_string(),
            value: runtime_provider,
            detail: "Choose the current session provider override with the provider picker."
                .to_string(),
            action: SettingsPickerAction::OpenRuntimeProviderOverride,
        },
        SettingsStudioItem {
            label: "Model Override".to_string(),
            value: runtime_model,
            detail: "Choose the current session model override with the model picker."
                .to_string(),
            action: SettingsPickerAction::OpenRuntimeModelOverride,
        },
        SettingsStudioItem {
            label: "Clear Runtime Stack".to_string(),
            value: "reset".to_string(),
            detail:
                "Clear provider/model plus thinking, speed, verbosity, and parallel-tool-call overrides."
                    .to_string(),
            action: SettingsPickerAction::ClearRuntimeModelStack,
        },
    ];
    items.extend(RUNTIME_SETTINGS.iter().map(|field| SettingsStudioItem {
        label: field.label.to_string(),
        value: run_options.runtime_setting_summary(*field),
        detail: field.description.to_string(),
        action: SettingsPickerAction::EditRuntimeSetting(*field),
    }));
    items
}

fn settings_studio_tool_description_mode_label(mode: &str) -> &'static str {
    match mode {
        "help" => "help",
        _ => "detailed",
    }
}

fn quoted_settings_segment(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn settings_studio_plugins_enabled(sources: &ConfigJsonSources) -> bool {
    get_json_path(&sources.effective, Some("plugins.enabled"))
        .unwrap_or(JsonValue::Bool(true))
        .as_bool()
        .unwrap_or(true)
}

fn settings_studio_plugins_default_mode(sources: &ConfigJsonSources) -> String {
    get_json_path(
        &sources.effective,
        Some("plugins.tool_presentation.default_mode"),
    )
    .unwrap_or(JsonValue::String("detailed".to_string()))
    .as_str()
    .unwrap_or("detailed")
    .to_string()
}

fn settings_studio_plugins_count(sources: &ConfigJsonSources, path: &str) -> usize {
    get_json_path(&sources.effective, Some(path))
        .unwrap_or(JsonValue::Null)
        .as_object()
        .map(|object| object.len())
        .unwrap_or(0)
}

fn settings_studio_plugin_entry_items(
    sources: &ConfigJsonSources,
    runtime_statuses: &[agena::plugin::status::PluginStatus],
) -> Vec<SettingsStudioItem> {
    let plugin_entries_value =
        get_json_path(&sources.effective, Some("plugins.list")).unwrap_or(JsonValue::Null);
    let plugin_entries = plugin_entries_value
        .as_object()
        .cloned()
        .unwrap_or_default();
    let file_entries_value =
        get_json_path(&sources.file, Some("plugins.list")).unwrap_or(JsonValue::Null);
    let file_entries = file_entries_value.as_object().cloned().unwrap_or_default();

    let mut items = runtime_statuses
        .iter()
        .filter_map(|status| {
            let plugin_id = status.plugin_id.as_str();
            let (entry, source) = if let Some(entry) = plugin_entries.get(plugin_id) {
                (
                    entry.clone(),
                    if file_entries.contains_key(plugin_id) {
                        "file".to_string()
                    } else {
                        "runtime".to_string()
                    },
                )
            } else if status.kind == "static" {
                (
                    json!({
                        "kind": "static",
                        "disabled": false,
                    }),
                    "builtin".to_string(),
                )
            } else {
                return None;
            };
            let entry_object = entry.as_object()?;
            let disabled = entry_object
                .get("disabled")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            let value = [
                status.kind.to_string(),
                source.clone(),
                status.state.as_str().to_string(),
                if disabled {
                    "disabled · skipped on reload".to_string()
                } else {
                    "enabled · loads on reload".to_string()
                },
            ]
            .join(" · ");
            Some(SettingsStudioItem {
                label: status.plugin_id.clone(),
                value,
                detail: plugin_entry_detail_text(status, entry_object, source.as_str(), disabled),
                action: SettingsPickerAction::TogglePluginEntryDisabled {
                    plugin_id: status.plugin_id.clone(),
                    entry,
                    disabled,
                },
            })
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.label.cmp(&right.label));
    items
}

fn plugin_entry_detail_text(
    status: &agena::plugin::status::PluginStatus,
    entry: &JsonMap<String, JsonValue>,
    source: &str,
    disabled: bool,
) -> String {
    let mut parts = vec![
        format!("kind={}", status.kind),
        format!("source={source}"),
        format!("state={}", status.state.as_str()),
    ];
    if let Some(pid) = status.pid {
        parts.push(format!("pid={pid}"));
    }
    if status.restart_count > 0 {
        parts.push(format!("restarts={}", status.restart_count));
    }
    if let Some(error) = status
        .last_error
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("last_error={error}"));
    }
    parts.push(if disabled {
        "disabled entries stay in config and are skipped on reload".to_string()
    } else {
        "enabled entries load on the next runtime reload".to_string()
    });
    if source == "builtin" {
        parts.push(
            "This built-in plugin is registered by the runtime even when the config file omits it."
                .to_string(),
        );
    }
    if entry
        .get("options")
        .and_then(JsonValue::as_object)
        .is_some_and(|options| !options.is_empty())
    {
        parts.push("custom options configured".to_string());
    }
    parts.push("Enter or t toggles the entry".to_string());
    parts.join(" · ")
}

fn settings_studio_plugin_items(sources: &ConfigJsonSources) -> Vec<SettingsStudioItem> {
    let enabled = settings_studio_plugins_enabled(sources);
    let default_mode = settings_studio_plugins_default_mode(sources);
    let plugin_override_count =
        settings_studio_plugins_count(sources, "plugins.tool_presentation.plugins");
    let tool_override_count =
        settings_studio_plugins_count(sources, "plugins.tool_presentation.tools");
    vec![
        SettingsStudioItem {
            label: "plugins.enabled".to_string(),
            value: if enabled { "on".to_string() } else { "off".to_string() },
            detail:
                "Turns all plugin hooks and tools on or off. The runtime reloads after the change."
                    .to_string(),
            action: SettingsPickerAction::TogglePluginsEnabled,
        },
        SettingsStudioItem {
            label: "plugins.tool_presentation.default_mode".to_string(),
            value: settings_studio_tool_description_mode_label(default_mode.as_str()).to_string(),
            detail:
                "Sets the default tool description mode. Help stays short and moves details into help output."
                    .to_string(),
            action: SettingsPickerAction::ToggleToolDescriptionMode,
        },
        SettingsStudioItem {
            label: "plugins.tool_presentation.plugins".to_string(),
            value: format!("{plugin_override_count} override(s)"),
            detail:
                "Per-plugin description mode overrides. Edit the raw config file for individual entries."
                    .to_string(),
            action: SettingsPickerAction::OpenConfigFile,
        },
        SettingsStudioItem {
            label: "plugins.tool_presentation.tools".to_string(),
            value: format!("{tool_override_count} override(s)"),
            detail:
                "Per-tool description mode overrides. Edit the raw config file for individual entries."
                    .to_string(),
            action: SettingsPickerAction::OpenConfigFile,
        },
    ]
}

fn settings_studio_plugin_entry_disabled_count(items: &[SettingsStudioItem]) -> usize {
    items
        .iter()
        .filter(|item| {
            matches!(
                &item.action,
                SettingsPickerAction::TogglePluginEntryDisabled { disabled: true, .. }
            )
        })
        .count()
}

fn agent_mode_label(mode: agena::agent::AgentMode) -> &'static str {
    match mode {
        agena::agent::AgentMode::Primary => "primary",
        agena::agent::AgentMode::Subagent => "subagent",
        agena::agent::AgentMode::All => "all",
    }
}

fn agent_default_summary(default: &agena::agents::AgentDefaultModelConfig) -> String {
    let mut parts = Vec::new();
    if let Some(provider) = default
        .provider
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("provider={provider}"));
    }
    if let Some(adapter) = default
        .adapter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("adapter={adapter}"));
    }
    if let Some(model) = default
        .model
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("model={model}"));
    }
    if parts.is_empty() {
        "inherits runtime model defaults".to_string()
    } else {
        parts.join(" · ")
    }
}

fn agent_permission_summary(permission: &agena::agent::AgentPermissionConfig) -> String {
    if permission.is_empty() {
        return "inherits runtime defaults".to_string();
    }

    let mut parts = Vec::new();
    let inherit_off = [
        (!permission.inherit.path(), "path"),
        (!permission.inherit.tools(), "tools"),
        (!permission.inherit.network(), "network"),
        (!permission.inherit.plugin_tools(), "plugin-tools"),
    ]
    .into_iter()
    .filter_map(|(disabled, label)| disabled.then_some(label))
    .collect::<Vec<_>>();
    if !inherit_off.is_empty() {
        parts.push(format!("inherit off: {}", inherit_off.join(", ")));
    }

    if let Some(path) = permission.path.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if path.workspace.is_some() {
            detail.push("workspace".to_string());
        }
        if path.external.is_some() {
            detail.push("external".to_string());
        }
        if !path.rules.is_empty() {
            detail.push(format!("{} rule(s)", path.rules.len()));
        }
        parts.push(format!(
            "path={}",
            if detail.is_empty() {
                "custom".to_string()
            } else {
                detail.join(" · ")
            }
        ));
    }
    if let Some(network) = permission.network.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if !network.rules.is_empty() {
            detail.push(format!("{} rule(s)", network.rules.len()));
        }
        parts.push(format!(
            "network={}",
            if detail.is_empty() {
                "custom".to_string()
            } else {
                detail.join(" · ")
            }
        ));
    }
    if let Some(tools) = permission.tools.as_ref() {
        let mut detail: Vec<String> = Vec::new();
        if !tools.tags.is_empty() {
            detail.push(format!("{} tag(s)", tools.tags.len()));
        }
        if !tools.names.is_empty() {
            detail.push(format!("{} name(s)", tools.names.len()));
        }
        if !tools.plugin.is_empty() {
            detail.push(format!("{} plugin override(s)", tools.plugin.len()));
        }
        if !tools.rules.is_empty() {
            detail.push(format!("{} rule set(s)", tools.rules.len()));
        }
        parts.push(format!(
            "tools={}",
            if detail.is_empty() {
                "custom".to_string()
            } else {
                detail.join(" · ")
            }
        ));
    }

    if parts.is_empty() {
        "inherits runtime defaults".to_string()
    } else {
        parts.join(" · ")
    }
}

fn settings_studio_agent_items(
    agents: &[AgentDescriptor],
    default_agent: Option<&str>,
) -> Vec<SettingsStudioItem> {
    let mut items = agents
        .iter()
        .map(|agent| {
            let source = agent
                .source_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "runtime config".to_string());
            let mut badges = vec![
                agent.scope.as_str().to_string(),
                agent_mode_label(agent.mode).to_string(),
            ];
            if agent.hidden {
                badges.push("hidden".to_string());
            }
            if default_agent.is_some_and(|name| name == agent.name.as_str()) {
                badges.push("default".to_string());
            }
            let value = badges.join(" · ");
            let detail = if agent.description.trim().is_empty() {
                format!("No description provided. source={source}")
            } else {
                format!("{} · source={source}", agent.description)
            };
            SettingsStudioItem {
                label: agent.name.clone(),
                value,
                detail,
                action: SettingsPickerAction::OpenAgent(Box::new(agent.clone())),
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| left.label.cmp(&right.label));
    items
}

fn settings_studio_agent_detail_text(
    agent: &AgentDescriptor,
    default_agent: Option<&str>,
) -> String {
    let mut lines = vec![
        format!("scope: {}", agent.scope.as_str()),
        format!("mode: {}", agent_mode_label(agent.mode)),
        format!(
            "visibility: {}",
            if agent.hidden { "hidden" } else { "visible" }
        ),
        format!(
            "default: {}",
            if default_agent.is_some_and(|name| name == agent.name.as_str()) {
                "yes"
            } else {
                "no"
            }
        ),
        format!(
            "source: {}",
            agent
                .source_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "runtime config".to_string())
        ),
        format!(
            "permission: {}",
            agent_permission_summary(&agent.permission)
        ),
        format!("model: {}", agent_default_summary(&agent.default)),
    ];
    if !agent.aliases.is_empty() {
        lines.push(format!("aliases: {}", agent.aliases.join(", ")));
    }
    if !agent.allowed_tools.is_empty() {
        lines.push(format!("tools: {}", agent.allowed_tools.join(", ")));
    }
    if !agent.description.trim().is_empty() {
        lines.push(String::new());
        lines.push(agent.description.clone());
    }
    lines.push(String::new());
    lines.push(
        "Enter opens the dedicated agent workbench. d makes this the default agent. t toggles hidden when the profile is owned by the current config file.".to_string(),
    );
    lines.join("\n")
}

fn agent_profile_editable(profile: &AgentProfile) -> bool {
    profile.source_path.is_none() && matches!(profile.scope, agena::agents::AgentScope::Project)
}

fn agent_profile_source_label(profile: &AgentProfile) -> String {
    profile
        .source_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "runtime config (agena.json)".to_string())
}

fn agent_prompt_summary(prompt: &str) -> String {
    if prompt.trim().is_empty() {
        "unset".to_string()
    } else {
        format!("{} chars", prompt.chars().count())
    }
}

fn agent_list_summary(values: &[String], empty: &str) -> String {
    if values.is_empty() {
        empty.to_string()
    } else if values.len() <= 3 {
        values.join(", ")
    } else {
        format!("{} items", values.len())
    }
}

fn agent_optional_string_summary(value: Option<&str>, empty: &str) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| empty.to_string())
}

fn agent_optional_number_summary<T: ToString>(value: Option<T>, empty: &str) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| empty.to_string())
}

fn agent_studio_items(
    profile: &AgentProfile,
    editable: bool,
    default_agent_name: Option<&str>,
) -> Vec<AgentStudioItem> {
    let is_default = default_agent_name.is_some_and(|name| name == profile.name.as_str());
    vec![
        AgentStudioItem {
            label: "Description".to_string(),
            value: agent_optional_string_summary(
                (!profile.frontmatter.description.trim().is_empty())
                    .then_some(profile.frontmatter.description.as_str()),
                "unset",
            ),
            detail: "Short listing summary for the agent profile.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::Description),
        },
        AgentStudioItem {
            label: "Prompt".to_string(),
            value: agent_prompt_summary(profile.prompt.as_str()),
            detail: "Main prompt body. Opens a multiline editor.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::Prompt),
        },
        AgentStudioItem {
            label: "Mode".to_string(),
            value: agent_mode_label(profile.frontmatter.mode).to_string(),
            detail: "Whether this profile is available as a primary agent, a subagent, or both."
                .to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::Mode),
        },
        AgentStudioItem {
            label: "Hidden".to_string(),
            value: if profile.frontmatter.hidden {
                "yes".to_string()
            } else {
                "no".to_string()
            },
            detail: if editable {
                "Toggles whether this agent stays out of default agent pickers.".to_string()
            } else {
                "Read-only here because this profile is backed by a markdown file.".to_string()
            },
            action: AgentStudioAction::ToggleHidden,
        },
        AgentStudioItem {
            label: "Aliases".to_string(),
            value: agent_list_summary(&profile.frontmatter.aliases, "none"),
            detail: "Alternate names that resolve to this profile.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::Aliases),
        },
        AgentStudioItem {
            label: "Allowed Entries".to_string(),
            value: agent_list_summary(&profile.frontmatter.allowed_tools, "unrestricted"),
            detail: "Optional allowlist for runtime entries/tools.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::AllowedEntries),
        },
        AgentStudioItem {
            label: "Temperature".to_string(),
            value: profile
                .frontmatter
                .temperature
                .map(|value| value.0.to_string())
                .unwrap_or_else(|| "inherit".to_string()),
            detail: "Optional model temperature override for this agent.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::Temperature),
        },
        AgentStudioItem {
            label: "Max Output Tokens".to_string(),
            value: agent_optional_number_summary(profile.frontmatter.max_output_tokens, "inherit"),
            detail: "Optional output token budget for this agent.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::MaxOutputTokens),
        },
        AgentStudioItem {
            label: "Steps".to_string(),
            value: agent_optional_number_summary(profile.frontmatter.steps, "inherit"),
            detail: "Optional step cap for this agent.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::Steps),
        },
        AgentStudioItem {
            label: "Default Provider".to_string(),
            value: agent_optional_string_summary(
                profile.frontmatter.default.provider.as_deref(),
                "inherit",
            ),
            detail: "Agent-scoped default provider override.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultProvider),
        },
        AgentStudioItem {
            label: "Default Adapter".to_string(),
            value: agent_optional_string_summary(
                profile.frontmatter.default.adapter.as_deref(),
                "inherit",
            ),
            detail: "Agent-scoped default adapter override.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultAdapter),
        },
        AgentStudioItem {
            label: "Default Model".to_string(),
            value: agent_optional_string_summary(
                profile.frontmatter.default.model.as_deref(),
                "inherit",
            ),
            detail: "Agent-scoped default model override.".to_string(),
            action: AgentStudioAction::Edit(AgentStudioField::DefaultModel),
        },
        AgentStudioItem {
            label: "Permission Policy".to_string(),
            value: agent_permission_summary(&profile.frontmatter.permission),
            detail: "Open the dedicated permission workbench for this agent.".to_string(),
            action: AgentStudioAction::OpenPermissionWorkbench,
        },
        AgentStudioItem {
            label: "Default Agent".to_string(),
            value: if is_default {
                "selected".to_string()
            } else {
                "inactive".to_string()
            },
            detail: "Sets default.agent to this profile when activated.".to_string(),
            action: AgentStudioAction::SetDefault,
        },
        AgentStudioItem {
            label: if profile.source_path.is_some() {
                "Open Source File".to_string()
            } else {
                "Open Config File".to_string()
            },
            value: agent_profile_source_label(profile),
            detail: if editable {
                "Open the raw config file for direct inspection.".to_string()
            } else {
                "Open the markdown profile backing this agent.".to_string()
            },
            action: AgentStudioAction::OpenSource,
        },
    ]
}

fn agent_studio_item_detail_text(
    profile: &AgentProfile,
    item: &AgentStudioItem,
    editable: bool,
    default_agent_name: Option<&str>,
) -> String {
    match &item.action {
        AgentStudioAction::Edit(AgentStudioField::Description) => {
            let mut lines = vec![
                "The description is shown in agent listings and pickers.".to_string(),
                String::new(),
            ];
            if profile.frontmatter.description.trim().is_empty() {
                lines.push("Description is currently unset.".to_string());
            } else {
                lines.push(profile.frontmatter.description.clone());
            }
            lines.push(String::new());
            lines.push(agent_editability_hint(editable));
            lines.join("\n")
        }
        AgentStudioAction::Edit(AgentStudioField::Prompt) => {
            let mut lines = vec![
                format!("Prompt length: {} chars", profile.prompt.chars().count()),
                String::new(),
            ];
            if profile.prompt.trim().is_empty() {
                lines.push("Prompt is currently unset.".to_string());
            } else {
                lines.push(profile.prompt.clone());
            }
            lines.push(String::new());
            lines.push(agent_editability_hint(editable));
            lines.join("\n")
        }
        AgentStudioAction::Edit(AgentStudioField::Aliases) => {
            let mut lines = vec![
                "Alternate names for invoking this agent.".to_string(),
                String::new(),
            ];
            if profile.frontmatter.aliases.is_empty() {
                lines.push("No aliases configured.".to_string());
            } else {
                lines.extend(profile.frontmatter.aliases.iter().cloned());
            }
            lines.push(String::new());
            lines.push(agent_editability_hint(editable));
            lines.join("\n")
        }
        AgentStudioAction::Edit(AgentStudioField::AllowedEntries) => {
            let mut lines = vec![
                "When empty, the agent can use the normal runtime tool surface.".to_string(),
                String::new(),
            ];
            if profile.frontmatter.allowed_tools.is_empty() {
                lines.push("No allowlist configured.".to_string());
            } else {
                lines.extend(profile.frontmatter.allowed_tools.iter().cloned());
            }
            lines.push(String::new());
            lines.push(agent_editability_hint(editable));
            lines.join("\n")
        }
        AgentStudioAction::OpenPermissionWorkbench => {
            let lines = vec![
                format!(
                    "Summary: {}",
                    agent_permission_summary(&profile.frontmatter.permission)
                ),
                String::new(),
                permission_pretty_document(&profile.frontmatter.permission),
                String::new(),
                if editable {
                    "Enter or p opens the permission workbench.".to_string()
                } else {
                    "Enter or p opens a read-only permission view for this file-backed profile."
                        .to_string()
                },
            ];
            lines.join("\n")
        }
        AgentStudioAction::SetDefault => format!(
            "Current default agent: {}\n\nEnter or d sets default.agent to this profile.",
            default_agent_name.unwrap_or("unset")
        ),
        AgentStudioAction::ToggleHidden => {
            let current = if profile.frontmatter.hidden {
                "hidden"
            } else {
                "visible"
            };
            format!(
                "Current visibility: {current}\nScope: {}\nSource: {}\n\n{}",
                profile.scope.as_str(),
                agent_profile_source_label(profile),
                if editable {
                    "Enter or t toggles hidden/visible and persists the config override."
                } else {
                    "This profile is file-backed, so hidden/visible must be edited in the source file."
                }
            )
        }
        AgentStudioAction::OpenSource => format!(
            "Source: {}\nScope: {}\n\nEnter or o opens the raw source/config file.",
            agent_profile_source_label(profile),
            profile.scope.as_str(),
        ),
        AgentStudioAction::Edit(_) => format!(
            "{}\nCurrent value: {}\n\n{}",
            item.detail,
            item.value,
            agent_editability_hint(editable),
        ),
    }
}

fn agent_studio_overview_text(
    profile: &AgentProfile,
    default_agent_name: Option<&str>,
    editable: bool,
) -> String {
    let mut lines = vec![
        format!("name: {}", profile.name),
        format!("scope: {}", profile.scope.as_str()),
        format!("mode: {}", agent_mode_label(profile.frontmatter.mode)),
        format!(
            "visibility: {}",
            if profile.frontmatter.hidden {
                "hidden"
            } else {
                "visible"
            }
        ),
        format!(
            "default agent: {}",
            if default_agent_name.is_some_and(|name| name == profile.name.as_str()) {
                "yes"
            } else {
                "no"
            }
        ),
        format!("source: {}", agent_profile_source_label(profile)),
        format!(
            "permission: {}",
            agent_permission_summary(&profile.frontmatter.permission)
        ),
    ];
    if !profile.frontmatter.default.is_empty() {
        lines.push(format!(
            "model defaults: {}",
            agent_default_summary(&profile.frontmatter.default)
        ));
    }
    if !profile.frontmatter.description.trim().is_empty() {
        lines.push(String::new());
        lines.push(profile.frontmatter.description.clone());
    }
    lines.push(String::new());
    lines.push(if editable {
        "This profile is backed by agena.json and can be edited directly in the TUI.".to_string()
    } else {
        "This profile is backed by a markdown source file and is read-only in the TUI.".to_string()
    });
    lines.join("\n")
}

fn agent_editability_hint(editable: bool) -> String {
    if editable {
        "Enter edits this field in agena.json.".to_string()
    } else {
        "This profile is read-only in the TUI because it is backed by a markdown file.".to_string()
    }
}

fn agent_studio_editor_config(
    profile: &AgentProfile,
    field: AgentStudioField,
) -> (String, String, String, bool, Editor) {
    let multiline = matches!(
        field,
        AgentStudioField::Description
            | AgentStudioField::Prompt
            | AgentStudioField::Aliases
            | AgentStudioField::AllowedEntries
    );
    let title = format!("Edit {}", agent_studio_field_label(field));
    let prompt = agent_studio_field_prompt(field).to_string();
    let footer = if multiline {
        "Ctrl+S save | Esc cancel".to_string()
    } else {
        "Enter save | Esc cancel".to_string()
    };
    let input = Editor::from_text(agent_studio_field_input_text(profile, field));
    (title, prompt, footer, multiline, input)
}

fn agent_studio_field_label(field: AgentStudioField) -> &'static str {
    match field {
        AgentStudioField::Description => "Description",
        AgentStudioField::Prompt => "Prompt",
        AgentStudioField::Mode => "Mode",
        AgentStudioField::Aliases => "Aliases",
        AgentStudioField::AllowedEntries => "Allowed Entries",
        AgentStudioField::Temperature => "Temperature",
        AgentStudioField::MaxOutputTokens => "Max Output Tokens",
        AgentStudioField::Steps => "Steps",
        AgentStudioField::DefaultProvider => "Default Provider",
        AgentStudioField::DefaultAdapter => "Default Adapter",
        AgentStudioField::DefaultModel => "Default Model",
    }
}

fn agent_studio_field_prompt(field: AgentStudioField) -> &'static str {
    match field {
        AgentStudioField::Description => "Multiline description. Leave blank to clear.",
        AgentStudioField::Prompt => "Multiline prompt body. Leave blank to clear.",
        AgentStudioField::Mode => "Enter primary, subagent, or all. Leave blank to clear.",
        AgentStudioField::Aliases => {
            "One alias per line. Commas are also accepted. Leave blank to clear."
        }
        AgentStudioField::AllowedEntries => {
            "One runtime entry/tool name per line. Commas are also accepted. Leave blank to clear."
        }
        AgentStudioField::Temperature => "Enter a floating-point value. Leave blank to clear.",
        AgentStudioField::MaxOutputTokens => {
            "Enter a positive integer token limit. Leave blank to clear."
        }
        AgentStudioField::Steps => "Enter a positive integer step cap. Leave blank to clear.",
        AgentStudioField::DefaultProvider => "Provider id override. Leave blank to clear.",
        AgentStudioField::DefaultAdapter => "Adapter id override. Leave blank to clear.",
        AgentStudioField::DefaultModel => "Model id override. Leave blank to clear.",
    }
}

fn agent_studio_field_input_text(profile: &AgentProfile, field: AgentStudioField) -> String {
    match field {
        AgentStudioField::Description => profile.frontmatter.description.clone(),
        AgentStudioField::Prompt => profile.prompt.clone(),
        AgentStudioField::Mode => agent_mode_label(profile.frontmatter.mode).to_string(),
        AgentStudioField::Aliases => profile.frontmatter.aliases.join("\n"),
        AgentStudioField::AllowedEntries => profile.frontmatter.allowed_tools.join("\n"),
        AgentStudioField::Temperature => profile
            .frontmatter
            .temperature
            .map(|value| value.0.to_string())
            .unwrap_or_default(),
        AgentStudioField::MaxOutputTokens => profile
            .frontmatter
            .max_output_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        AgentStudioField::Steps => profile
            .frontmatter
            .steps
            .map(|value| value.to_string())
            .unwrap_or_default(),
        AgentStudioField::DefaultProvider => profile
            .frontmatter
            .default
            .provider
            .clone()
            .unwrap_or_default(),
        AgentStudioField::DefaultAdapter => profile
            .frontmatter
            .default
            .adapter
            .clone()
            .unwrap_or_default(),
        AgentStudioField::DefaultModel => profile
            .frontmatter
            .default
            .model
            .clone()
            .unwrap_or_default(),
    }
}

fn agent_studio_field_setting_value(
    agent_name: &str,
    field: AgentStudioField,
    input: &str,
) -> UiResult<(String, Option<JsonValue>)> {
    let trimmed = input.trim();
    let path = match field {
        AgentStudioField::Description => agent_config_path(agent_name, "description"),
        AgentStudioField::Prompt => agent_config_path(agent_name, "prompt"),
        AgentStudioField::Mode => agent_config_path(agent_name, "mode"),
        AgentStudioField::Aliases => agent_config_path(agent_name, "aliases"),
        AgentStudioField::AllowedEntries => agent_config_path(agent_name, "allowed_entries"),
        AgentStudioField::Temperature => agent_config_path(agent_name, "temperature"),
        AgentStudioField::MaxOutputTokens => agent_config_path(agent_name, "max_output_tokens"),
        AgentStudioField::Steps => agent_config_path(agent_name, "steps"),
        AgentStudioField::DefaultProvider => agent_config_path(agent_name, "default.provider"),
        AgentStudioField::DefaultAdapter => agent_config_path(agent_name, "default.adapter"),
        AgentStudioField::DefaultModel => agent_config_path(agent_name, "default.model"),
    };
    let value = match field {
        AgentStudioField::Description | AgentStudioField::Prompt => {
            (!trimmed.is_empty()).then_some(JsonValue::String(input.to_string()))
        }
        AgentStudioField::Mode => {
            if trimmed.is_empty() {
                None
            } else {
                let normalized = trimmed.to_ascii_lowercase();
                match normalized.as_str() {
                    "primary" | "subagent" | "all" => Some(JsonValue::String(normalized)),
                    other => {
                        return Err(format!(
                            "invalid mode `{other}`; expected primary, subagent, or all"
                        ));
                    }
                }
            }
        }
        AgentStudioField::Aliases | AgentStudioField::AllowedEntries => {
            let values = parse_string_list_input(input);
            (!values.is_empty()).then_some(JsonValue::Array(
                values.into_iter().map(JsonValue::String).collect(),
            ))
        }
        AgentStudioField::Temperature => {
            if trimmed.is_empty() {
                None
            } else {
                let parsed = trimmed
                    .parse::<f32>()
                    .map_err(|error| format!("invalid temperature: {error}"))?;
                if !parsed.is_finite() {
                    return Err("temperature must be finite".to_string());
                }
                Some(json!(parsed))
            }
        }
        AgentStudioField::MaxOutputTokens => {
            if trimmed.is_empty() {
                None
            } else {
                Some(json!(trimmed.parse::<u32>().map_err(|error| format!(
                    "invalid max_output_tokens: {error}"
                ))?))
            }
        }
        AgentStudioField::Steps => {
            if trimmed.is_empty() {
                None
            } else {
                Some(json!(
                    trimmed
                        .parse::<usize>()
                        .map_err(|error| format!("invalid steps: {error}"))?
                ))
            }
        }
        AgentStudioField::DefaultProvider
        | AgentStudioField::DefaultAdapter
        | AgentStudioField::DefaultModel => {
            (!trimmed.is_empty()).then_some(JsonValue::String(trimmed.to_string()))
        }
    };
    Ok((path, value))
}

fn agent_permission_studio_items(
    profile: &AgentProfile,
    editable: bool,
) -> Vec<AgentPermissionStudioItem> {
    let permission = &profile.frontmatter.permission;
    vec![
        AgentPermissionStudioItem {
            label: "Inherit Path".to_string(),
            value: permission_inherit_value_label(permission, AgentPermissionField::InheritPath),
            detail: "Controls whether path defaults flow in from runtime permissions.".to_string(),
            action: AgentPermissionStudioAction::Edit(AgentPermissionField::InheritPath),
        },
        AgentPermissionStudioItem {
            label: "Inherit Network".to_string(),
            value: permission_inherit_value_label(permission, AgentPermissionField::InheritNetwork),
            detail: "Controls whether network defaults flow in from runtime permissions."
                .to_string(),
            action: AgentPermissionStudioAction::Edit(AgentPermissionField::InheritNetwork),
        },
        AgentPermissionStudioItem {
            label: "Inherit Entries".to_string(),
            value: permission_inherit_value_label(permission, AgentPermissionField::InheritEntries),
            detail: "Controls whether tool/entry defaults flow in from runtime permissions."
                .to_string(),
            action: AgentPermissionStudioAction::Edit(AgentPermissionField::InheritEntries),
        },
        AgentPermissionStudioItem {
            label: "Path Section".to_string(),
            value: agent_path_permission_summary(permission.path.as_ref()),
            detail: "Structured path policy override as JSON.".to_string(),
            action: AgentPermissionStudioAction::Edit(AgentPermissionField::PathConfig),
        },
        AgentPermissionStudioItem {
            label: "Network Section".to_string(),
            value: agent_network_permission_summary(permission.network.as_ref()),
            detail: "Structured network policy override as JSON.".to_string(),
            action: AgentPermissionStudioAction::Edit(AgentPermissionField::NetworkConfig),
        },
        AgentPermissionStudioItem {
            label: "Entry Section".to_string(),
            value: agent_tool_permission_summary(permission.tools.as_ref()),
            detail: "Structured runtime entry/tool policy override as JSON.".to_string(),
            action: AgentPermissionStudioAction::Edit(AgentPermissionField::EntryConfig),
        },
        AgentPermissionStudioItem {
            label: "Full Permission Document".to_string(),
            value: agent_permission_summary(permission),
            detail: "Edit the whole agent permission document as JSON.".to_string(),
            action: AgentPermissionStudioAction::Edit(AgentPermissionField::FullConfig),
        },
        AgentPermissionStudioItem {
            label: if profile.source_path.is_some() {
                "Open Source File".to_string()
            } else {
                "Open Config File".to_string()
            },
            value: agent_profile_source_label(profile),
            detail: if editable {
                "Open agena.json for direct inspection.".to_string()
            } else {
                "Open the markdown profile backing this agent.".to_string()
            },
            action: AgentPermissionStudioAction::OpenSource,
        },
    ]
}

fn agent_permission_studio_item_detail_text(
    profile: &AgentProfile,
    item: &AgentPermissionStudioItem,
    editable: bool,
) -> String {
    match item.action {
        AgentPermissionStudioAction::Edit(AgentPermissionField::InheritPath)
        | AgentPermissionStudioAction::Edit(AgentPermissionField::InheritNetwork)
        | AgentPermissionStudioAction::Edit(AgentPermissionField::InheritEntries) => {
            format!(
                "{}\nCurrent value: {}\n\n{}",
                item.detail,
                item.value,
                if editable {
                    "Enter edits this inheritance override. Leave the input blank to clear it."
                } else {
                    "This profile is read-only here because it is backed by a markdown file."
                }
            )
        }
        AgentPermissionStudioAction::Edit(AgentPermissionField::PathConfig) => {
            format!(
                "{}\n\n{}\n\n{}",
                item.detail,
                pretty_json_optional(profile.frontmatter.permission.path.as_ref())
                    .unwrap_or_else(|| "Section is currently unset.".to_string()),
                if editable {
                    "Enter opens a multiline JSON editor for this section."
                } else {
                    "This section is read-only here because the profile is file-backed."
                }
            )
        }
        AgentPermissionStudioAction::Edit(AgentPermissionField::NetworkConfig) => {
            format!(
                "{}\n\n{}\n\n{}",
                item.detail,
                pretty_json_optional(profile.frontmatter.permission.network.as_ref())
                    .unwrap_or_else(|| "Section is currently unset.".to_string()),
                if editable {
                    "Enter opens a multiline JSON editor for this section."
                } else {
                    "This section is read-only here because the profile is file-backed."
                }
            )
        }
        AgentPermissionStudioAction::Edit(AgentPermissionField::EntryConfig) => {
            format!(
                "{}\n\n{}\n\n{}",
                item.detail,
                pretty_json_optional(profile.frontmatter.permission.tools.as_ref())
                    .unwrap_or_else(|| "Section is currently unset.".to_string()),
                if editable {
                    "Enter opens a multiline JSON editor for this section."
                } else {
                    "This section is read-only here because the profile is file-backed."
                }
            )
        }
        AgentPermissionStudioAction::Edit(AgentPermissionField::FullConfig) => {
            format!(
                "{}\n\n{}\n\n{}",
                item.detail,
                permission_pretty_document(&profile.frontmatter.permission),
                if editable {
                    "Enter opens a multiline JSON editor for the full permission document."
                } else {
                    "This permission document is read-only here because the profile is file-backed."
                }
            )
        }
        AgentPermissionStudioAction::OpenSource => format!(
            "Source: {}\nScope: {}\n\nEnter or o opens the raw source/config file.",
            agent_profile_source_label(profile),
            profile.scope.as_str(),
        ),
    }
}

fn agent_permission_editor_config(
    profile: &AgentProfile,
    field: AgentPermissionField,
) -> (String, String, String, bool, Editor) {
    let multiline = matches!(
        field,
        AgentPermissionField::PathConfig
            | AgentPermissionField::NetworkConfig
            | AgentPermissionField::EntryConfig
            | AgentPermissionField::FullConfig
    );
    let title = format!("Edit {}", agent_permission_field_label(field));
    let prompt = agent_permission_field_prompt(field).to_string();
    let footer = if multiline {
        "Ctrl+S save | Esc cancel".to_string()
    } else {
        "Enter save | Esc cancel".to_string()
    };
    let input = Editor::from_text(agent_permission_field_input_text(profile, field));
    (title, prompt, footer, multiline, input)
}

fn agent_permission_field_label(field: AgentPermissionField) -> &'static str {
    match field {
        AgentPermissionField::InheritPath => "Inherit Path",
        AgentPermissionField::InheritNetwork => "Inherit Network",
        AgentPermissionField::InheritEntries => "Inherit Entries",
        AgentPermissionField::PathConfig => "Path Section",
        AgentPermissionField::NetworkConfig => "Network Section",
        AgentPermissionField::EntryConfig => "Entry Section",
        AgentPermissionField::FullConfig => "Full Permission Document",
    }
}

fn agent_permission_field_prompt(field: AgentPermissionField) -> &'static str {
    match field {
        AgentPermissionField::InheritPath
        | AgentPermissionField::InheritNetwork
        | AgentPermissionField::InheritEntries => {
            "Enter true or false. Leave blank to clear the override."
        }
        AgentPermissionField::PathConfig => {
            "Enter a JSON object matching PathPermissionConfig. Leave blank to clear the section."
        }
        AgentPermissionField::NetworkConfig => {
            "Enter a JSON object matching NetworkPermissionConfig. Leave blank to clear the section."
        }
        AgentPermissionField::EntryConfig => {
            "Enter a JSON object matching ToolPermissionConfig. Leave blank to clear the section."
        }
        AgentPermissionField::FullConfig => {
            "Enter a JSON object matching AgentPermissionConfig. Leave blank to clear the whole permission override."
        }
    }
}

fn agent_permission_field_input_text(
    profile: &AgentProfile,
    field: AgentPermissionField,
) -> String {
    match field {
        AgentPermissionField::InheritPath => permission_inherit_setting_value(
            &profile.frontmatter.permission,
            AgentPermissionField::InheritPath,
        )
        .map(|value| value.to_string())
        .unwrap_or_default(),
        AgentPermissionField::InheritNetwork => permission_inherit_setting_value(
            &profile.frontmatter.permission,
            AgentPermissionField::InheritNetwork,
        )
        .map(|value| value.to_string())
        .unwrap_or_default(),
        AgentPermissionField::InheritEntries => permission_inherit_setting_value(
            &profile.frontmatter.permission,
            AgentPermissionField::InheritEntries,
        )
        .map(|value| value.to_string())
        .unwrap_or_default(),
        AgentPermissionField::PathConfig => {
            pretty_json_optional(profile.frontmatter.permission.path.as_ref()).unwrap_or_default()
        }
        AgentPermissionField::NetworkConfig => {
            pretty_json_optional(profile.frontmatter.permission.network.as_ref())
                .unwrap_or_default()
        }
        AgentPermissionField::EntryConfig => {
            pretty_json_optional(profile.frontmatter.permission.tools.as_ref()).unwrap_or_default()
        }
        AgentPermissionField::FullConfig => {
            if profile.frontmatter.permission.is_empty() {
                String::new()
            } else {
                permission_pretty_document(&profile.frontmatter.permission)
            }
        }
    }
}

fn agent_permission_field_setting_value(
    agent_name: &str,
    field: AgentPermissionField,
    input: &str,
) -> UiResult<(String, Option<JsonValue>)> {
    let path = agent_permission_path(agent_name, field);
    let trimmed = input.trim();
    let value = match field {
        AgentPermissionField::InheritPath
        | AgentPermissionField::InheritNetwork
        | AgentPermissionField::InheritEntries => {
            parse_optional_bool_input(trimmed)?.map(JsonValue::Bool)
        }
        AgentPermissionField::PathConfig => {
            parse_optional_json_config::<agena::agent::PathPermissionConfig>(trimmed)?
        }
        AgentPermissionField::NetworkConfig => {
            parse_optional_json_config::<agena::agent::NetworkPermissionConfig>(trimmed)?
        }
        AgentPermissionField::EntryConfig => {
            parse_optional_json_config::<agena::agent::ToolPermissionConfig>(trimmed)?
        }
        AgentPermissionField::FullConfig => {
            parse_optional_json_config::<agena::agent::AgentPermissionConfig>(trimmed)?
        }
    };
    Ok((path, value))
}

fn permission_pretty_document(permission: &agena::agent::AgentPermissionConfig) -> String {
    if permission.is_empty() {
        "Permission override is currently unset.".to_string()
    } else {
        pretty_json(permission)
    }
}

fn permission_inherit_setting_value(
    permission: &agena::agent::AgentPermissionConfig,
    field: AgentPermissionField,
) -> Option<bool> {
    match &permission.inherit {
        agena::agent::PermissionInheritanceConfig::All(value) => Some(*value),
        agena::agent::PermissionInheritanceConfig::Sections(sections) => match field {
            AgentPermissionField::InheritPath => sections.path,
            AgentPermissionField::InheritNetwork => sections.network,
            AgentPermissionField::InheritEntries => sections.tools,
            _ => None,
        },
    }
}

fn permission_inherit_value_label(
    permission: &agena::agent::AgentPermissionConfig,
    field: AgentPermissionField,
) -> String {
    match permission_inherit_setting_value(permission, field) {
        Some(value) => value.to_string(),
        None => "default(true)".to_string(),
    }
}

fn agent_path_permission_summary(path: Option<&agena::agent::PathPermissionConfig>) -> String {
    let Some(path) = path else {
        return "unset".to_string();
    };
    let mut parts = Vec::new();
    if path.workspace.is_some() {
        parts.push("workspace".to_string());
    }
    if path.external.is_some() {
        parts.push("external".to_string());
    }
    if !path.rules.is_empty() {
        parts.push(format!("{} rule(s)", path.rules.len()));
    }
    if parts.is_empty() {
        "custom".to_string()
    } else {
        parts.join(" · ")
    }
}

fn agent_network_permission_summary(
    network: Option<&agena::agent::NetworkPermissionConfig>,
) -> String {
    let Some(network) = network else {
        return "unset".to_string();
    };
    let mut parts = Vec::new();
    if network.internet.is_some() {
        parts.push("internet".to_string());
    }
    if network.private.is_some() {
        parts.push("private".to_string());
    }
    if network.loopback.is_some() {
        parts.push("loopback".to_string());
    }
    if !network.rules.is_empty() {
        parts.push(format!("{} rule(s)", network.rules.len()));
    }
    if parts.is_empty() {
        "custom".to_string()
    } else {
        parts.join(" · ")
    }
}

fn agent_tool_permission_summary(tools: Option<&agena::agent::ToolPermissionConfig>) -> String {
    let Some(tools) = tools else {
        return "unset".to_string();
    };
    let mut parts = Vec::new();
    if !tools.tags.is_empty() {
        parts.push(format!("{} tag(s)", tools.tags.len()));
    }
    if !tools.names.is_empty() {
        parts.push(format!("{} name(s)", tools.names.len()));
    }
    if !tools.rules.is_empty() {
        parts.push(format!("{} rule set(s)", tools.rules.len()));
    }
    if parts.is_empty() {
        "custom".to_string()
    } else {
        parts.join(" · ")
    }
}

fn pretty_json_optional<T: Serialize>(value: Option<&T>) -> Option<String> {
    value.map(pretty_json)
}

fn pretty_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
}

fn parse_optional_json_config<T>(input: &str) -> UiResult<Option<JsonValue>>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    if input.trim().is_empty() {
        return Ok(None);
    }
    let parsed =
        serde_json::from_str::<T>(input).map_err(|error| format!("invalid json: {error}"))?;
    serde_json::to_value(parsed)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn parse_optional_bool_input(input: &str) -> UiResult<Option<bool>> {
    if input.trim().is_empty() {
        return Ok(None);
    }
    match input.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" => Ok(Some(true)),
        "false" | "no" | "off" => Ok(Some(false)),
        other => Err(format!(
            "invalid boolean `{other}`; expected true/false, yes/no, or on/off"
        )),
    }
}

fn parse_string_list_input(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in input
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let value = token.to_string();
        if !out.contains(&value) {
            out.push(value);
        }
    }
    out
}

fn agent_config_path(agent_name: &str, suffix: &str) -> String {
    format!("agents.{}.{}", quoted_settings_segment(agent_name), suffix)
}

fn agent_permission_path(agent_name: &str, field: AgentPermissionField) -> String {
    match field {
        AgentPermissionField::InheritPath => {
            agent_config_path(agent_name, "permission.inherit.path")
        }
        AgentPermissionField::InheritNetwork => {
            agent_config_path(agent_name, "permission.inherit.network")
        }
        AgentPermissionField::InheritEntries => {
            agent_config_path(agent_name, "permission.inherit.entries")
        }
        AgentPermissionField::PathConfig => agent_config_path(agent_name, "permission.path"),
        AgentPermissionField::NetworkConfig => agent_config_path(agent_name, "permission.network"),
        AgentPermissionField::EntryConfig => agent_config_path(agent_name, "permission.entries"),
        AgentPermissionField::FullConfig => agent_config_path(agent_name, "permission"),
    }
}

fn settings_studio_provider_items(
    providers: &[ProviderSummaryResource],
) -> Vec<SettingsStudioItem> {
    let mut items = vec![SettingsStudioItem {
        label: "+ New provider".to_string(),
        value: String::new(),
        detail:
            "Create a new provider, list live adapter models, and edit the persisted default route."
                .to_string(),
        action: SettingsPickerAction::OpenProviderWorkbench,
    }];
    items.extend(providers.iter().map(|provider| SettingsStudioItem {
        label: provider.provider_id.clone(),
        value: format!(
            "{}/{}",
            provider.default_adapter.as_deref().unwrap_or("adapter"),
            provider.default_model
        ),
        detail: format!(
            "{} adapter{} configured",
            provider.adapters.len(),
            if provider.adapters.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
        action: SettingsPickerAction::OpenProviderWorkbenchFor(provider.provider_id.clone()),
    }));
    items
}

fn settings_studio_model_catalog_items(
    response: &ModelCatalogListResponse,
) -> Vec<SettingsStudioItem> {
    vec![SettingsStudioItem {
        label: "Open Model Catalog".to_string(),
        value: response.summary.entry_count.to_string(),
        detail: "Inspect resolved catalog entries and refresh the local model catalog cache."
            .to_string(),
        action: SettingsPickerAction::OpenModelCatalogWorkbench,
    }]
}

fn settings_studio_file_items(sources: &ConfigJsonSources) -> Vec<SettingsStudioItem> {
    vec![SettingsStudioItem {
        label: "Open agena.json".to_string(),
        value: if sources.config_found {
            "present".to_string()
        } else {
            "create on open".to_string()
        },
        detail: sources.config_path.display().to_string(),
        action: SettingsPickerAction::OpenConfigFile,
    }]
}

fn format_setting_field_summary(file_value: &JsonValue, effective_value: &JsonValue) -> String {
    if !file_value.is_null() {
        if file_value == effective_value {
            format!("configured {}", format_setting_value_inline(file_value))
        } else {
            format!(
                "file {} · effective {}",
                format_setting_value_inline(file_value),
                format_setting_value_inline(effective_value)
            )
        }
    } else if !effective_value.is_null() {
        format!("effective {}", format_setting_value_inline(effective_value))
    } else {
        "unset".to_string()
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
    field: SettingsFieldSpec,
    file_value: &JsonValue,
    effective_value: &JsonValue,
) -> String {
    let mut lines = vec![field.description.to_string()];
    if !file_value.is_null() {
        lines.push(format!(
            "File override: {}",
            format_setting_value_inline(file_value)
        ));
        if file_value != effective_value {
            lines.push(format!(
                "Effective value: {}",
                format_setting_value_inline(effective_value)
            ));
        }
    } else {
        lines.push(format!(
            "Effective value: {}",
            format_setting_value_inline(effective_value)
        ));
    }
    lines.push(settings_field_help_suffix(field.kind).to_string());
    lines.join("\n")
}

fn runtime_setting_edit_prompt(field: RuntimeSettingSpec, current_summary: &str) -> String {
    format!(
        "{}\nCurrent override: {}\n{}",
        field.description,
        current_summary,
        settings_field_help_suffix(field.kind)
    )
}

fn settings_field_help_suffix(kind: SettingsFieldKind) -> &'static str {
    match kind {
        SettingsFieldKind::String => {
            "Enter text. Leave empty or type `clear` to remove the file override."
        }
        SettingsFieldKind::Bool => {
            "Enter true/false, on/off, yes/no, or 1/0. Leave empty or type `clear` to remove the file override."
        }
        SettingsFieldKind::Integer => {
            "Enter a whole number. Leave empty or type `clear` to remove the file override."
        }
        SettingsFieldKind::Float => {
            "Enter a number. Leave empty or type `clear` to remove the override."
        }
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

fn boolean_choice_items(detail: &str) -> Vec<ChoiceItem> {
    vec![choice_item("true", detail), choice_item("false", detail)]
}

fn provider_studio_default_model_choice_items(dialog: &ProviderStudioOverlay) -> Vec<ChoiceItem> {
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
            if let Some(entry) = dialog.catalog_matches.get(key.as_str()) {
                detail_parts.push(format!("catalog {}", entry.model_id));
            } else {
                detail_parts.push("no catalog match".to_owned());
            }
            items.push(choice_item(model.id.to_string(), detail_parts.join(" · ")));
        }
    }
    dedupe_choice_items(items)
}

fn provider_studio_profile_choice_items(backend: &Backend) -> Vec<ChoiceItem> {
    let mut items = backend
        .list_aws_profile_names()
        .into_iter()
        .map(|profile| choice_item(profile, "AWS shared config profile"))
        .collect::<Vec<_>>();
    if !items.iter().any(|item| item.value == "default") {
        items.insert(0, choice_item("default", "AWS default profile"));
    }
    dedupe_choice_items(items)
}

fn provider_studio_api_key_env_choice_items() -> Vec<ChoiceItem> {
    let items = vec![
        choice_item("OPENAI_API_KEY", "OpenAI-compatible API key env var"),
        choice_item("ANTHROPIC_API_KEY", "Anthropic API key env var"),
        choice_item("GEMINI_API_KEY", "Gemini API key env var"),
        choice_item("GITLAB_TOKEN", "GitLab token env var"),
        choice_item(
            "GOOGLE_VERTEX_ACCESS_TOKEN",
            "Google Vertex access token env var",
        ),
        choice_item("SHARED_GATEWAY_API_KEY", "shared gateway API key env var"),
        choice_item("OPENCODE_API_KEY", "Opencode API key env var"),
    ];
    dedupe_choice_items(items)
}

fn provider_studio_field_allows_clear(field: ProviderStudioField) -> bool {
    matches!(
        field,
        ProviderStudioField::AuthMode
            | ProviderStudioField::CredentialIssuer
            | ProviderStudioField::BaseUrl
            | ProviderStudioField::InstanceUrl
            | ProviderStudioField::ApiKeyEnv
            | ProviderStudioField::ApiKey
            | ProviderStudioField::RedirectUri
            | ProviderStudioField::CallbackUrl
            | ProviderStudioField::RefreshToken
            | ProviderStudioField::AccessToken
            | ProviderStudioField::ExpiresAtMs
            | ProviderStudioField::AccountId
            | ProviderStudioField::EnterpriseDomain
            | ProviderStudioField::Username
            | ProviderStudioField::DisplayName
            | ProviderStudioField::Email
            | ProviderStudioField::AvatarUrl
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

fn choice_overlay_clear_detail(action: &ChoiceOverlayAction) -> String {
    match action {
        ChoiceOverlayAction::SettingsField(field) => {
            format!("Remove the file override for {}.", field.path)
        }
        ChoiceOverlayAction::RuntimeSetting(field) => {
            format!("Remove the current-session override for {}.", field.label)
        }
        ChoiceOverlayAction::ProviderStudioField(field) => format!(
            "Set {} to an empty draft value.",
            provider_studio_field_label(*field)
        ),
    }
}

fn parse_settings_field_input(
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
                    return Err(format!(
                        "{} expects a boolean like true/false or on/off",
                        field.path
                    ));
                }
            };
            Ok(Some(JsonValue::Bool(value)))
        }
        SettingsFieldKind::Integer => {
            let value = trimmed
                .parse::<u64>()
                .map_err(|_| format!("{} expects an unsigned integer value", field.path))?;
            Ok(Some(JsonValue::from(value)))
        }
        SettingsFieldKind::Float => {
            let value = trimmed
                .parse::<f64>()
                .map_err(|_| format!("{} expects a numeric value", field.path))?;
            Ok(Some(JsonValue::from(value)))
        }
    }
}

impl ProviderStudioFocus {
    fn next(self, show_provider_list: bool) -> Self {
        match (show_provider_list, self) {
            (true, Self::Fields) => Self::Adapters,
            (true, Self::Adapters) => Self::Models,
            (true, Self::Models) => Self::Fields,
            (false, Self::Fields) => Self::Adapters,
            (false, Self::Adapters) => Self::Models,
            (false, Self::Models) => Self::Fields,
        }
    }

    fn prev(self, show_provider_list: bool) -> Self {
        match (show_provider_list, self) {
            (true, Self::Fields) => Self::Models,
            (true, Self::Adapters) => Self::Fields,
            (true, Self::Models) => Self::Adapters,
            (false, Self::Fields) => Self::Models,
            (false, Self::Adapters) => Self::Fields,
            (false, Self::Models) => Self::Adapters,
        }
    }
}

fn provider_studio_provider_rows(
    providers: &[ProviderSummaryResource],
) -> Vec<ProviderStudioProviderRow> {
    let mut rows = vec![ProviderStudioProviderRow {
        provider_id: None,
        label: "+ New provider".to_owned(),
        detail: "Empty provider draft".to_owned(),
    }];
    rows.extend(providers.iter().map(|provider| ProviderStudioProviderRow {
        provider_id: Some(provider.provider_id.clone()),
        label: provider.provider_id.clone(),
        detail: format!(
            "{} / {} · {} adapter{}",
            provider.default_adapter.as_deref().unwrap_or("adapter"),
            provider.default_model,
            provider.adapters.len(),
            if provider.adapters.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
    }));
    rows
}

fn session_model_choice_item(
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
        .unwrap_or_else(|| default_adapter.unwrap_or("default").to_owned());
    let context_window = model
        .metadata
        .limits
        .context_window_tokens
        .map(|value| format!("ctx {value}"))
        .unwrap_or_else(|| "ctx ?".to_string());
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
                .unwrap_or_else(|| "default".to_owned()),
            model.id
        ),
        detail: detail_parts.join(" · "),
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

fn provider_studio_catalog_match_entry<'a>(
    model: &ProviderModel,
    entries: &'a [ModelCatalogEntryResource],
) -> Option<&'a ModelCatalogEntryResource> {
    let lookup_id = provider_model_catalog_lookup_id(model);
    entries
        .iter()
        .filter(|entry| entry.model_id == model.id.as_str() || entry.model_id == lookup_id)
        .min_by_key(|entry| entry.model_id.as_str())
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
        self.items.get(self.selected)
    }

    fn current_selected_id(&self) -> Option<i64> {
        self.current_selected().map(|item| item.id)
    }

    fn clamp_selection(&mut self) {
        if self.items.is_empty() {
            self.selected = 0;
        } else {
            self.selected = min(self.selected, self.items.len().saturating_sub(1));
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }

        let last = self.items.len().saturating_sub(1) as isize;
        let next = (self.selected as isize + delta).clamp(0, last);
        self.selected = next as usize;
    }

    fn should_load_more(&self) -> bool {
        false
    }

    fn select_by_id(&mut self, session_id: i64) -> bool {
        if let Some(index) = self.items.iter().position(|item| item.id == session_id) {
            self.selected = index;
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
            let entry =
                serde_json::from_str::<PersistentPromptHistoryEntry>(line).map_err(|error| {
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
        if self.entries.is_empty() {
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
        for text in &self.entries {
            let line = serde_json::to_string(&PersistentPromptHistoryEntry { text: text.clone() })
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
        if self.entries.last().is_some_and(|entry| entry == &text) {
            return false;
        }
        self.entries.retain(|entry| entry != &text);
        self.entries.push(text);
        if self.entries.len() > MAX_PROMPT_HISTORY_ENTRIES {
            let excess = self.entries.len() - MAX_PROMPT_HISTORY_ENTRIES;
            self.entries.drain(0..excess);
        }
        true
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn get(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(String::as_str)
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
    }

    fn scroll_to_bottom(&mut self, width: u16, height: u16) {
        self.scroll = self.max_scroll(width, height);
        self.follow_tail = true;
        self.cursor_line = self.rendered(width).lines.len().saturating_sub(1);
    }

    fn scroll_to_top(&mut self, width: u16, height: u16) {
        self.scroll = 0;
        self.cursor_line = 0;
        self.follow_tail = self.is_at_bottom(width, height);
    }

    fn scroll_by_lines(&mut self, width: u16, height: u16, delta: isize) {
        self.follow_tail = false;
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
        let cursor_line = self.cursor_line;
        let rendered = self.rendered(width);
        let node_index = rendered
            .line_nodes
            .get(cursor_line)
            .and_then(|value| *value)?;
        rendered.nodes.get(node_index)
    }

    fn current_cursor_node_cloned(&mut self, width: u16) -> Option<RenderedTranscriptNode> {
        self.current_cursor_node(width).cloned()
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

impl Editor {
    fn from_text(text: String) -> Self {
        let mut editor = Self::default();
        editor.set_text(text);
        editor
    }

    fn text(&self) -> &str {
        &self.text
    }

    fn set_text(&mut self, text: String) {
        self.text = text;
        self.cursor = self.text.len();
        self.preferred_column = None;
        self.elements.clear();
        self.paste_burst = PasteBurst::default();
    }

    fn set_elements(&mut self, elements: Vec<Range<usize>>) {
        let mut normalized = elements
            .into_iter()
            .filter_map(|range| {
                let start = min(range.start, self.text.len());
                let end = min(range.end, self.text.len());
                (start < end).then_some(EditorElement { range: start..end })
            })
            .collect::<Vec<_>>();
        normalized.sort_by_key(|element| element.range.start);
        normalized.dedup_by(|a, b| a.range == b.range);
        self.elements = normalized;
        self.cursor = self.clamp_pos_to_nearest_boundary(self.cursor);
    }

    fn draft_elements(&self) -> Vec<Range<usize>> {
        self.elements
            .iter()
            .map(|element| element.range.clone())
            .collect()
    }

    fn element_texts(&self) -> Vec<String> {
        self.elements
            .iter()
            .filter_map(|element| self.text.get(element.range.clone()).map(str::to_owned))
            .collect()
    }

    fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.preferred_column = None;
        self.elements.clear();
        self.paste_burst = PasteBurst::default();
    }

    fn insert_char(&mut self, ch: char) {
        let mut buffer = [0_u8; 4];
        self.insert_str(ch.encode_utf8(&mut buffer));
    }

    fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let at = self.clamp_pos_for_insertion(self.cursor);
        self.insert_str_at(at, text);
    }

    fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    fn insert_element(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let start = self.clamp_pos_for_insertion(self.cursor);
        self.insert_str_at(start, text);
        let end = start + text.len();
        self.elements.push(EditorElement { range: start..end });
        self.elements.sort_by_key(|element| element.range.start);
        self.cursor = end;
        self.preferred_column = None;
    }

    fn logical_line_count(&self) -> usize {
        split_editor_lines_with_offsets(self.text.as_str()).len()
    }

    fn handle_line_input_key(&mut self, key: KeyEvent) {
        self.handle_input_key(key, false);
    }

    fn handle_multiline_input_key(&mut self, key: KeyEvent) {
        self.handle_input_key(key, true);
    }

    fn render_view(&self, width: u16, height: u16) -> EditorView {
        let width = max(width as usize, 1);
        let height = max(height as usize, 1);
        let lines = split_editor_lines_with_offsets(self.text.as_str());
        let current_line_index = self.current_line_index();
        let current_col = self.current_display_column();
        let hscroll = current_col.saturating_sub(width.saturating_sub(1));
        let vscroll = current_line_index.saturating_sub(height.saturating_sub(1));
        let visible_lines = lines
            .iter()
            .skip(vscroll)
            .take(height)
            .map(|range| {
                slice_display_window_styled(
                    self.text.as_str(),
                    range.clone(),
                    hscroll,
                    width,
                    self.elements.as_slice(),
                )
            })
            .collect::<Vec<_>>();

        EditorView {
            lines: visible_lines,
            cursor_x: min(current_col.saturating_sub(hscroll), u16::MAX as usize) as u16,
            cursor_y: min(
                current_line_index.saturating_sub(vscroll),
                u16::MAX as usize,
            ) as u16,
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent, multiline: bool) {
        let now = Instant::now();
        self.flush_pending_input_if_due(now);

        match key {
            KeyEvent {
                code: KeyCode::Char('\u{0001}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.prepare_for_command();
                self.move_home(true);
            }
            KeyEvent {
                code: KeyCode::Char('\u{0002}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.prepare_for_command();
                self.move_left();
            }
            KeyEvent {
                code: KeyCode::Left,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                || (modifiers.contains(KeyModifiers::ALT) && !is_altgr(modifiers)) =>
            {
                self.prepare_for_command();
                self.move_word_left();
            }
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.move_word_left();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0005}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('e'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.prepare_for_command();
                self.move_end(true);
            }
            KeyEvent {
                code: KeyCode::Char('\u{0006}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Right,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.prepare_for_command();
                self.move_right();
            }
            KeyEvent {
                code: KeyCode::Right,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL)
                || (modifiers.contains(KeyModifiers::ALT) && !is_altgr(modifiers)) =>
            {
                self.prepare_for_command();
                self.move_word_right();
            }
            KeyEvent {
                code: KeyCode::Char('f'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.move_word_right();
            }
            KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                ..
            } if multiline => {
                self.prepare_for_command();
                self.move_up();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0010}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('p'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } if multiline => {
                self.prepare_for_command();
                self.move_up();
            }
            KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                ..
            } if multiline => {
                self.prepare_for_command();
                self.move_down();
            }
            KeyEvent {
                code: KeyCode::Char('\u{000e}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('n'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } if multiline => {
                self.prepare_for_command();
                self.move_down();
            }
            KeyEvent {
                code: KeyCode::Home,
                ..
            } => {
                self.prepare_for_command();
                self.move_home(false);
            }
            KeyEvent {
                code: KeyCode::End, ..
            } => {
                self.prepare_for_command();
                self.move_end(false);
            }
            KeyEvent {
                code: KeyCode::Char('h'),
                modifiers,
                ..
            } if modifiers == (KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Backspace,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0008}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.prepare_for_command();
                self.backspace();
            }
            KeyEvent {
                code: KeyCode::Delete,
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::ALT) => {
                self.prepare_for_command();
                self.delete_forward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0004}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
            | KeyEvent {
                code: KeyCode::Delete,
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.prepare_for_command();
                self.delete();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0017}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.prepare_for_command();
                self.delete_backward_word();
            }
            KeyEvent {
                code: KeyCode::Char('\u{0015}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('u'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.prepare_for_command();
                self.kill_to_start_of_line(multiline);
            }
            KeyEvent {
                code: KeyCode::Char('\u{000b}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.prepare_for_command();
                self.kill_to_end_of_line(multiline);
            }
            KeyEvent {
                code: KeyCode::Char('\u{0019}'),
                modifiers: KeyModifiers::NONE,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('y'),
                modifiers: KeyModifiers::CONTROL,
                ..
            } => {
                self.prepare_for_command();
                self.yank();
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            } if is_altgr(modifiers) => {
                self.handle_plain_char(c, now);
            }
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT,
                ..
            } => {
                self.handle_plain_char(c, now);
            }
            _ => {}
        }
    }

    fn should_insert_newline_on_enter(&mut self) -> bool {
        let now = Instant::now();
        self.flush_pending_input_if_due(now);
        self.paste_burst
            .newline_should_insert_instead_of_submit(now)
    }

    fn insert_explicit_newline(&mut self) {
        self.flush_all_pending_input();
        self.insert_newline();
        self.paste_burst.clear_window_after_non_char();
    }

    fn insert_newline_from_enter(&mut self) {
        let now = Instant::now();
        self.flush_pending_input_if_due(now);
        if self.paste_burst.append_newline_if_active(now) {
            return;
        }
        self.flush_all_pending_input();
        self.insert_newline();
        self.paste_burst.clear_window_after_non_char();
    }

    fn flush_pending_input_if_due(&mut self, now: Instant) {
        match self.paste_burst.flush_if_due(now) {
            PasteFlushResult::Paste(text) => self.insert_str(text.as_str()),
            PasteFlushResult::Typed(ch) => self.insert_char(ch),
            PasteFlushResult::None => {}
        }
    }

    fn flush_all_pending_input(&mut self) {
        match self.paste_burst.flush_now() {
            PasteFlushResult::Paste(text) => self.insert_str(text.as_str()),
            PasteFlushResult::Typed(ch) => self.insert_char(ch),
            PasteFlushResult::None => {}
        }
    }

    fn prepare_for_command(&mut self) {
        self.flush_all_pending_input();
        self.paste_burst.clear_window_after_non_char();
    }

    fn handle_plain_char(&mut self, ch: char, now: Instant) {
        match self.paste_burst.on_plain_char(ch, now) {
            PasteCharDecision::RetainFirstChar => {}
            PasteCharDecision::BufferAppend | PasteCharDecision::BeginBufferFromPending => {
                self.paste_burst.append_char_to_buffer(ch, now);
            }
            PasteCharDecision::BeginBuffer { retro_chars } => {
                if let Some(retro) = self.decide_retro_grab(retro_chars as usize) {
                    self.remove_range(retro.start_byte, self.cursor);
                    self.paste_burst
                        .begin_with_retro_grabbed(retro.grabbed, now);
                    self.paste_burst.append_char_to_buffer(ch, now);
                } else {
                    self.flush_all_pending_input();
                    self.insert_char(ch);
                }
            }
        }
    }

    fn move_left(&mut self) {
        self.cursor = self.prev_atomic_boundary(self.cursor);
        self.preferred_column = None;
    }

    fn move_right(&mut self) {
        self.cursor = self.next_atomic_boundary(self.cursor);
        self.preferred_column = None;
    }

    fn move_word_left(&mut self) {
        self.cursor = self.beginning_of_previous_word();
        self.preferred_column = None;
    }

    fn move_word_right(&mut self) {
        self.cursor = self.end_of_next_word();
        self.preferred_column = None;
    }

    fn move_home(&mut self, move_up_at_bol: bool) {
        let bol = self.current_line_start();
        if move_up_at_bol && self.cursor == bol && bol > 0 {
            self.cursor = self.clamp_pos_to_nearest_boundary(self.beginning_of_line(bol - 1));
        } else {
            self.cursor = self.clamp_pos_to_nearest_boundary(bol);
        }
        self.preferred_column = None;
    }

    fn move_end(&mut self, move_down_at_eol: bool) {
        let eol = self.current_line_end();
        if move_down_at_eol && self.cursor == eol && eol < self.text.len() {
            self.cursor = self.clamp_pos_to_nearest_boundary(self.end_of_line(eol + 1));
        } else {
            self.cursor = self.clamp_pos_to_nearest_boundary(eol);
        }
        self.preferred_column = None;
    }

    fn move_up(&mut self) {
        let current_start = self.current_line_start();
        if current_start == 0 {
            return;
        }
        let target_end = current_start.saturating_sub(1);
        let target_start = self.text[..target_end]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let target_col = self
            .preferred_column
            .unwrap_or_else(|| self.current_display_column());
        self.cursor = byte_index_at_display_column(
            &self.text[target_start..target_end],
            target_start,
            target_col,
        );
        self.cursor = self.clamp_pos_to_nearest_boundary(self.cursor);
        self.preferred_column = Some(target_col);
    }

    fn move_down(&mut self) {
        let current_end = self.current_line_end();
        if current_end >= self.text.len() {
            return;
        }
        let target_start = current_end + 1;
        let target_end = self.text[target_start..]
            .find('\n')
            .map(|index| target_start + index)
            .unwrap_or(self.text.len());
        let target_col = self
            .preferred_column
            .unwrap_or_else(|| self.current_display_column());
        self.cursor = byte_index_at_display_column(
            &self.text[target_start..target_end],
            target_start,
            target_col,
        );
        self.cursor = self.clamp_pos_to_nearest_boundary(self.cursor);
        self.preferred_column = Some(target_col);
    }

    fn backspace(&mut self) {
        let previous = self.prev_atomic_boundary(self.cursor);
        if previous < self.cursor {
            self.remove_range(previous, self.cursor);
        }
    }

    fn delete(&mut self) {
        let next = self.next_atomic_boundary(self.cursor);
        if next > self.cursor {
            self.remove_range(self.cursor, next);
        }
    }

    fn delete_backward_word(&mut self) {
        let start = self.beginning_of_previous_word();
        self.kill_buffer = self.remove_range(start, self.cursor);
    }

    fn delete_forward_word(&mut self) {
        let end = self.end_of_next_word();
        self.kill_buffer = self.remove_range(self.cursor, end);
    }

    fn kill_to_start_of_line(&mut self, multiline: bool) {
        let start = self.current_line_start();
        if self.cursor == start && multiline && start > 0 {
            self.kill_buffer = self.remove_range(start - 1, start);
        } else {
            self.kill_buffer = self.remove_range(start, self.cursor);
        }
    }

    fn kill_to_end_of_line(&mut self, multiline: bool) {
        let end = self.current_line_end();
        if self.cursor == end && multiline && end < self.text.len() {
            self.kill_buffer = self.remove_range(self.cursor, end + 1);
        } else {
            self.kill_buffer = self.remove_range(self.cursor, end);
        }
    }

    fn yank(&mut self) {
        if !self.kill_buffer.is_empty() {
            let text = self.kill_buffer.clone();
            self.insert_str(text.as_str());
        }
    }

    fn remove_range(&mut self, start: usize, end: usize) -> String {
        let range = self.expand_range_to_element_boundaries(
            min(start, self.text.len())..min(end, self.text.len()),
        );
        if range.start >= range.end {
            return String::new();
        }
        let removed = self.text[range.clone()].to_string();
        self.text.replace_range(range.clone(), "");
        self.update_elements_after_replace(range.start, range.end, 0);
        self.cursor = self.clamp_pos_to_nearest_boundary(range.start);
        self.preferred_column = None;
        removed
    }

    fn current_line_index(&self) -> usize {
        self.text[..self.cursor]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
    }

    /// True when there's no preceding newline before the cursor — used by
    /// the queue-edit shortcut so UP only steals the keystroke when the
    /// user is on the editor's first line.
    fn cursor_on_first_line(&self) -> bool {
        self.current_line_index() == 0
    }

    fn current_line_start(&self) -> usize {
        self.text[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0)
    }

    fn current_line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map(|index| self.cursor + index)
            .unwrap_or(self.text.len())
    }

    fn beginning_of_line(&self, pos: usize) -> usize {
        let pos = min(pos, self.text.len());
        self.text[..pos]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0)
    }

    fn end_of_line(&self, pos: usize) -> usize {
        let pos = min(pos, self.text.len());
        self.text[pos..]
            .find('\n')
            .map(|index| pos + index)
            .unwrap_or(self.text.len())
    }

    fn current_display_column(&self) -> usize {
        UnicodeWidthStr::width(&self.text[self.current_line_start()..self.cursor])
    }

    fn beginning_of_previous_word(&self) -> usize {
        let mut pos = self.cursor;
        while pos > 0 {
            let start = self.prev_atomic_boundary(pos);
            if is_word_grapheme(&self.text[start..pos]) {
                break;
            }
            pos = start;
        }
        while pos > 0 {
            let start = self.prev_atomic_boundary(pos);
            if !is_word_grapheme(&self.text[start..pos]) {
                break;
            }
            pos = start;
        }
        self.adjust_pos_out_of_elements(pos, true)
    }

    fn end_of_next_word(&self) -> usize {
        let mut pos = self.cursor;
        while pos < self.text.len() {
            let end = self.next_atomic_boundary(pos);
            if is_word_grapheme(&self.text[pos..end]) {
                break;
            }
            pos = end;
        }
        while pos < self.text.len() {
            let end = self.next_atomic_boundary(pos);
            if !is_word_grapheme(&self.text[pos..end]) {
                break;
            }
            pos = end;
        }
        self.adjust_pos_out_of_elements(pos, false)
    }

    fn decide_retro_grab(&self, retro_chars: usize) -> Option<RetroGrab> {
        let before = &self.text[..self.cursor];
        let start_byte = retro_start_index(before, retro_chars);
        if self.range_intersects_element(start_byte..self.cursor) {
            return None;
        }
        let grabbed = before[start_byte..].to_string();
        let looks_pastey = grabbed.chars().any(char::is_whitespace)
            || grabbed
                .chars()
                .any(|ch| matches!(ch, '/' | '\\' | ':' | '=' | ',' | '.'))
            || grabbed.chars().count() >= 16;
        looks_pastey.then_some(RetroGrab {
            start_byte,
            grabbed,
        })
    }

    fn insert_str_at(&mut self, at: usize, text: &str) {
        let at = self.clamp_pos_for_insertion(at);
        self.text.insert_str(at, text);
        self.update_elements_after_replace(at, at, text.len());
        self.cursor = at + text.len();
        self.preferred_column = None;
    }

    fn find_element_containing(&self, pos: usize) -> Option<usize> {
        self.elements
            .iter()
            .position(|element| pos > element.range.start && pos < element.range.end)
    }

    fn range_intersects_element(&self, range: Range<usize>) -> bool {
        self.elements
            .iter()
            .any(|element| element.range.start < range.end && element.range.end > range.start)
    }

    fn clamp_pos_to_nearest_boundary(&self, mut pos: usize) -> usize {
        pos = min(pos, self.text.len());
        if let Some(index) = self.find_element_containing(pos) {
            let element = &self.elements[index];
            let dist_start = pos.saturating_sub(element.range.start);
            let dist_end = element.range.end.saturating_sub(pos);
            if dist_start <= dist_end {
                element.range.start
            } else {
                element.range.end
            }
        } else {
            pos
        }
    }

    fn clamp_pos_for_insertion(&self, pos: usize) -> usize {
        self.clamp_pos_to_nearest_boundary(pos)
    }

    fn expand_range_to_element_boundaries(&self, mut range: Range<usize>) -> Range<usize> {
        loop {
            let mut changed = false;
            for element in &self.elements {
                if element.range.start < range.end && element.range.end > range.start {
                    let next_start = min(range.start, element.range.start);
                    let next_end = max(range.end, element.range.end);
                    if next_start != range.start || next_end != range.end {
                        range.start = next_start;
                        range.end = next_end;
                        changed = true;
                    }
                }
            }
            if !changed {
                return range;
            }
        }
    }

    fn shift_elements(&mut self, at: usize, removed: usize, inserted: usize) {
        let end = at.saturating_add(removed);
        let delta = inserted as isize - removed as isize;
        self.elements
            .retain(|element| !(element.range.start >= at && element.range.end <= end));

        for element in &mut self.elements {
            if element.range.end <= at {
                continue;
            }
            if element.range.start >= end {
                element.range.start = ((element.range.start as isize) + delta) as usize;
                element.range.end = ((element.range.end as isize) + delta) as usize;
                continue;
            }

            let new_start = min(at, element.range.start);
            let tail = element.range.end.saturating_sub(end);
            element.range.start = new_start;
            element.range.end = at.saturating_add(inserted).saturating_add(tail);
        }
    }

    fn update_elements_after_replace(&mut self, start: usize, end: usize, inserted_len: usize) {
        self.shift_elements(start, end.saturating_sub(start), inserted_len);
    }

    fn prev_atomic_boundary(&self, pos: usize) -> usize {
        if pos == 0 {
            return 0;
        }
        if let Some(index) = self
            .elements
            .iter()
            .position(|element| pos > element.range.start && pos <= element.range.end)
        {
            return self.elements[index].range.start;
        }
        let boundary = previous_grapheme_boundary(self.text.as_str(), pos);
        if let Some(index) = self.find_element_containing(boundary) {
            self.elements[index].range.start
        } else {
            boundary
        }
    }

    fn next_atomic_boundary(&self, pos: usize) -> usize {
        if pos >= self.text.len() {
            return self.text.len();
        }
        if let Some(index) = self
            .elements
            .iter()
            .position(|element| pos >= element.range.start && pos < element.range.end)
        {
            return self.elements[index].range.end;
        }
        let boundary = next_grapheme_boundary(self.text.as_str(), pos);
        if let Some(index) = self.find_element_containing(boundary) {
            self.elements[index].range.end
        } else {
            boundary
        }
    }

    fn adjust_pos_out_of_elements(&self, pos: usize, prefer_start: bool) -> usize {
        if let Some(index) = self.find_element_containing(pos) {
            let element = &self.elements[index];
            if prefer_start {
                element.range.start
            } else {
                element.range.end
            }
        } else {
            pos
        }
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
            "Allow always (session)".to_string()
        }
        (PermissionReplyKind::AllowAlways, Some(PermissionScope::Workspace)) => {
            "Allow always (workspace)".to_string()
        }
        (PermissionReplyKind::AllowAlways, Some(PermissionScope::Global)) => {
            "Allow always (global)".to_string()
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Session)) => {
            "Deny always (session)".to_string()
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Workspace)) => {
            "Deny always (workspace)".to_string()
        }
        (PermissionReplyKind::DenyAlways, Some(PermissionScope::Global)) => {
            "Deny always (global)".to_string()
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
        PermissionAction::Tool { tool_name, .. } => i18n.text_args(
            "overlay-permission-action-tool",
            &crate::fl_args!("tool" => tool_name.clone()),
        ),
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
        PermissionAction::NetworkAccess { host, port, .. } => match port {
            Some(port) => format!("network {host}:{port}"),
            None => format!("network {host}"),
        },
    }
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

fn format_timestamp_ms(timestamp_ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(format_timestamp)
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn build_plugin_inspector_item(
    status: agena::plugin::status::PluginStatus,
    inspect: Option<agena::plugin::PluginInspect>,
    logs: Vec<agena::plugin::PluginLogEntry>,
) -> PluginInspectorItem {
    let manifest = inspect.as_ref().and_then(|item| item.manifest.as_ref());
    let summary = manifest.map_or_else(
        || {
            format!(
                "{} [{}] {}",
                status.plugin_id,
                status.state.as_str(),
                status.kind
            )
        },
        |manifest| {
            format!(
                "{} [{}] {} — {}@{}",
                status.plugin_id,
                status.state.as_str(),
                status.kind,
                manifest.name,
                manifest.version
            )
        },
    );
    let authority = inspect.as_ref().and_then(|item| item.authority.as_ref());
    let detail = format_plugin_inspector_detail(&status, manifest, authority);
    let logs_text = format_plugin_inspector_logs(logs.as_slice());
    let copy_text = format!("{summary}\n\n{detail}\n\nRecent logs\n-----------\n{logs_text}");
    let search_text = format!(
        "{} {} {}",
        summary.to_ascii_lowercase(),
        detail.to_ascii_lowercase(),
        logs_text.to_ascii_lowercase()
    );

    PluginInspectorItem {
        plugin_id: status.plugin_id.clone(),
        summary,
        detail,
        logs: logs_text,
        search_text,
        copy_text,
        state: status.state,
    }
}

fn format_plugin_inspector_detail(
    status: &agena::plugin::status::PluginStatus,
    manifest: Option<&agena::plugin::PluginManifest>,
    authority: Option<&agena::plugin::host::PluginAuthoritySummary>,
) -> String {
    let mut lines = vec![
        format!("plugin_id: {}", status.plugin_id),
        format!("kind: {}", status.kind),
        format!("state: {}", status.state.as_str()),
        format!(
            "pid: {}",
            status
                .pid
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string())
        ),
        format!("restart_count: {}", status.restart_count),
        format!(
            "last_exit_code: {}",
            status
                .last_exit_code
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string())
        ),
        format!(
            "last_restart: {}",
            status
                .last_restart_at_ms
                .map(format_timestamp_ms)
                .unwrap_or_else(|| "-".to_string())
        ),
    ];
    if let Some(error) = status.last_error.as_deref() {
        lines.push(format!("last_error: {error}"));
    }

    lines.push(String::new());
    match manifest {
        Some(manifest) => {
            lines.push(format!("manifest: {}@{}", manifest.name, manifest.version));
            if let Some(description) = manifest.description.as_deref() {
                lines.push(format!("description: {description}"));
            }
            if !manifest.authors.is_empty() {
                lines.push(format!("authors: {}", manifest.authors.join(", ")));
            }
            if !manifest.transports.is_empty() {
                lines.push(format!(
                    "transports: {}",
                    manifest
                        .transports
                        .iter()
                        .map(|transport| format!("{transport:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            lines.push(format!("hooks: {:?}", manifest.hooks));
            lines.push(format!("entries: {}", manifest.entries.len()));
            for entry in manifest.entries.iter().take(5) {
                lines.extend(
                    format_plugin_entry_summary_lines(entry)
                        .into_iter()
                        .map(|line| format!("  {line}")),
                );
            }
            if manifest.entries.len() > 5 {
                lines.push(format!("  - ... +{} more", manifest.entries.len() - 5));
            }
            let capabilities = manifest
                .entries
                .iter()
                .flat_map(|entry| entry.host_capabilities.iter())
                .map(|capability| format!("{capability:?}"))
                .collect::<BTreeSet<_>>();
            if !capabilities.is_empty() {
                lines.push(format!(
                    "capabilities: {}",
                    capabilities.into_iter().collect::<Vec<_>>().join(", ")
                ));
            }
        }
        None => lines.push("manifest: unavailable".to_string()),
    }

    if let Some(authority) = authority {
        lines.push(String::new());
        lines.push(format!("trust_level: {}", authority.trust_level));
        if !authority.provenance.is_empty() {
            lines.push(format!("provenance: {}", authority.provenance.join(" | ")));
        }
        if !authority.plugin_capabilities.is_empty() {
            lines.push(format!(
                "effective_capabilities: {}",
                authority.plugin_capabilities.join(", ")
            ));
        }
        if !authority.entry_capabilities.is_empty() {
            lines.push("entry_capabilities:".to_string());
            for (entry, capabilities) in &authority.entry_capabilities {
                lines.push(format!("  - {}: {}", entry, capabilities.join(", ")));
            }
        }
    }

    lines.join("\n")
}

fn format_plugin_entry_summary_lines(entry: &agena::plugin::PluginToolDecl) -> Vec<String> {
    let mut lines = vec![format!("{}", entry.name)];
    if let Some(description) = entry.description_text().trim().split('\n').next()
        && !description.trim().is_empty()
    {
        lines.push(format!("description: {}", description.trim()));
    }
    if let Some(summary) = entry.summary_text() {
        lines.push(format!("summary: {summary}"));
    }
    if let Some(help) = entry.help_text() {
        lines.push(format!("help: {help}"));
    }
    let mut facts = Vec::new();
    if let Some(mode) = entry.description_mode {
        let mode_label = match mode {
            agena::plugin::ToolDescriptionMode::Detailed => "detailed",
            agena::plugin::ToolDescriptionMode::Help => "help",
        };
        facts.push(format!("mode={mode_label}"));
    }
    if entry.strict {
        facts.push("strict".to_string());
    }
    if matches!(
        entry.streaming,
        agena::plugin::sdk::ToolStreamingMode::Streaming
    ) {
        facts.push("streaming".to_string());
    }
    if entry.concurrency_safe {
        facts.push("concurrency-safe".to_string());
    }
    if !entry.tags.is_empty() {
        facts.push(format!(
            "tags={}",
            entry
                .tags
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !facts.is_empty() {
        lines.push(format!("facts: {}", facts.join(" · ")));
    }
    if !entry.host_capabilities.is_empty() {
        lines.push(format!(
            "host_caps: {}",
            entry
                .host_capabilities
                .iter()
                .map(|capability| format!("{capability:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    lines
}

fn format_plugin_inspector_logs(entries: &[agena::plugin::PluginLogEntry]) -> String {
    if entries.is_empty() {
        return "No retained logs".to_string();
    }
    entries
        .iter()
        .map(|entry| {
            let mut line = format!(
                "[{}] #{} {} {} {}",
                format_timestamp_ms(entry.timestamp_ms),
                entry.seq,
                entry.level,
                entry.source,
                entry.message
            );
            if !entry.fields.is_null() {
                line.push(' ');
                line.push_str(entry.fields.to_string().as_str());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_timeline_item(record: &DomainEvent) -> TimelineItem {
    let event_type = timeline_event_type_name(record);
    let summary_suffix = timeline_event_summary(record);
    let summary = if summary_suffix.is_empty() {
        format!("#{}  {}", record.meta.seq_global, event_type)
    } else {
        format!(
            "#{}  {}  {}",
            record.meta.seq_global, event_type, summary_suffix
        )
    };

    let mut detail_lines = vec![
        format!("seq: {}", record.meta.seq_global),
        format!("created: {}", format_timestamp(record.meta.created_at)),
        format!("type: {event_type}"),
        format!("event_id: {}", record.meta.id),
    ];
    if let Some(causation_id) = record.meta.causation_id {
        detail_lines.push(format!("causation_id: {causation_id}"));
    }
    if let Some(correlation_id) = record.meta.correlation_id {
        detail_lines.push(format!("correlation_id: {correlation_id}"));
    }
    detail_lines.push(String::new());
    detail_lines.extend(timeline_event_detail_lines(record));

    let detail = detail_lines.join("\n");
    let copy_text = format!("{summary}\n\n{detail}");
    let search_text = format!(
        "{} {}",
        summary.to_ascii_lowercase(),
        detail.to_ascii_lowercase()
    );
    let linked_message_id = timeline_event_message_id(record);

    TimelineItem {
        summary,
        detail,
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
        | AgenaSessionEvent::SessionGoalUpdated(_)
        | AgenaSessionEvent::RunStarted(_)
        | AgenaSessionEvent::RunCompleted(_)
        | AgenaSessionEvent::RunAborted(_)
        | AgenaSessionEvent::ToolCallIssued(_)
        | AgenaSessionEvent::ToolCallCompleted(_)
        | AgenaSessionEvent::PluginEvent(_) => None,
    }
}

fn timeline_event_type_name(record: &DomainEvent) -> &'static str {
    record.kind.tag_str()
}

fn session_workflow_state_label(execution: &SessionExecutionResource) -> &'static str {
    match pending_interactive_kind_for_execution(execution) {
        Some(PendingInteractiveKind::Permission) => return "awaiting_permission",
        Some(PendingInteractiveKind::UserInput) => return "awaiting_user_input",
        None => {}
    }
    if execution.blocked {
        return "blocked";
    }
    match execution.run_state {
        SessionRunState::AwaitingModel => "awaiting_model",
        SessionRunState::Idle => "idle",
    }
}

fn timeline_event_summary(record: &DomainEvent) -> String {
    match &record.kind {
        AgenaSessionEvent::ExecutionStarted(event) => format!("session #{}", event.session_id),
        AgenaSessionEvent::ExecutionFailed(event) => {
            format!(
                "{}: {}",
                event.error.code,
                detail_excerpt(event.error.message.as_str(), 72)
            )
        }
        AgenaSessionEvent::MessagePartUpdated(event) => format!(
            "message #{} part #{} updated ({:?})",
            event.message_id, event.part.id, event.part.kind
        ),
        AgenaSessionEvent::MessagePartDelta(event) => format!(
            "message #{} part #{} {:?} (+{} chars)",
            event.message_id,
            event.part_id,
            event.field,
            event.delta.chars().count(),
        ),
        AgenaSessionEvent::CommandBegin(event) => detail_excerpt(event.command.as_str(), 72),
        AgenaSessionEvent::CommandOutputDelta(event) => {
            let preview = if event.preview_text.trim().is_empty() {
                format!("{} bytes", event.chunk.len())
            } else {
                detail_excerpt(event.preview_text.as_str(), 56)
            };
            format!("{:?} {}", event.stream, preview)
        }
        AgenaSessionEvent::CommandEnd(event) => {
            format!(
                "{:?} exit={} ({} ms)",
                event.status, event.exit_code, event.duration_ms
            )
        }
        AgenaSessionEvent::StreamError(event) => {
            format!(
                "{}: {}",
                event.error.code,
                detail_excerpt(event.error.message.as_str(), 72)
            )
        }
        AgenaSessionEvent::PermissionRequested(event) => {
            format!(
                "permission requested [{}]: {}",
                permission_risk_label(event.risk),
                detail_excerpt(event.reason.as_str(), 72)
            )
        }
        AgenaSessionEvent::PermissionReplied(event) => {
            format!("permission replied: {:?}", event.kind)
        }
        AgenaSessionEvent::PermissionRuleCreated(event) => {
            format!("permission rule #{} created", event.rule_id)
        }
        AgenaSessionEvent::PermissionRuleUpdated(event) => {
            format!("permission rule #{} updated", event.rule_id)
        }
        AgenaSessionEvent::PermissionRuleRevoked(event) => {
            format!("permission rule #{} revoked", event.rule_id)
        }
        AgenaSessionEvent::SessionGoalUpdated(event) => {
            let objective = event
                .objective
                .as_deref()
                .map(|value| detail_excerpt(value, 56))
                .unwrap_or_else(|| "<none>".to_string());
            let status = event.status.as_deref().unwrap_or("unknown");
            format!("goal updated: {status} · {objective}")
        }
        AgenaSessionEvent::RunStarted(p) => format!("run {}", p.run_id),
        AgenaSessionEvent::RunCompleted(p) => {
            format!("run {} ({:?})", p.run_id, p.finish_reason)
        }
        AgenaSessionEvent::RunAborted(p) => {
            format!("run {} aborted ({:?})", p.run_id, p.reason)
        }
        AgenaSessionEvent::UserMessageAppended(p) => {
            format!("user #{}", p.message_id)
        }
        AgenaSessionEvent::AssistantMessageCompleted(p) => {
            format!("assistant #{} ({:?})", p.message_id, p.finish_reason)
        }
        AgenaSessionEvent::ToolCallIssued(p) => {
            format!("tool {} call={}", p.name, p.call_id)
        }
        AgenaSessionEvent::ToolCallCompleted(p) => format!("tool call {} done", p.call_id),
        AgenaSessionEvent::SystemNoticeAppended(p) => {
            format!("system #{}: {:?}", p.message_id, p.kind)
        }
        AgenaSessionEvent::PluginEvent(p) => {
            format!("plugin {}/{}", p.plugin_id, p.kind_label)
        }
    }
}

fn timeline_event_detail_lines(record: &DomainEvent) -> Vec<String> {
    match &record.kind {
        AgenaSessionEvent::ExecutionStarted(event) => {
            vec![format!("session_id: {}", event.session_id)]
        }
        AgenaSessionEvent::ExecutionFailed(event) => vec![
            format!("session_id: {}", event.session_id),
            format!("error_code: {}", event.error.code),
            format!("error_message: {}", event.error.message),
        ],
        AgenaSessionEvent::MessagePartUpdated(event) => vec![
            format!("message_id: {}", event.message_id),
            format!("part_id: {}", event.part.id),
            format!("part_kind: {:?}", event.part.kind),
            format!("status: {:?}", event.part.status),
            format!(
                "summary: {}",
                event
                    .part
                    .summary
                    .clone()
                    .unwrap_or_else(|| "<none>".to_string())
            ),
        ],
        AgenaSessionEvent::MessagePartDelta(event) => vec![
            format!("message_id: {}", event.message_id),
            format!("part_id: {}", event.part_id),
            format!("field: {:?}", event.field),
            format!("seq: {}", event.seq),
            format!("delta: {}", detail_excerpt(event.delta.as_str(), 200)),
        ],
        AgenaSessionEvent::CommandBegin(event) => vec![
            format!("session_id: {}", event.context.session_id),
            format!("call_id: {}", event.context.call_id),
            format!("command: {}", event.command),
            format!("cwd: {}", event.cwd),
        ],
        AgenaSessionEvent::CommandOutputDelta(event) => vec![
            format!("session_id: {}", event.context.session_id),
            format!("call_id: {}", event.context.call_id),
            format!("stream: {:?}", event.stream),
            format!("seq: {}", event.seq),
            format!("bytes: {}", event.chunk.len()),
            format!(
                "preview: {}",
                detail_excerpt(event.preview_text.as_str(), 200)
            ),
        ],
        AgenaSessionEvent::CommandEnd(event) => vec![
            format!("session_id: {}", event.context.session_id),
            format!("call_id: {}", event.context.call_id),
            format!("status: {:?}", event.status),
            format!("exit_code: {}", event.exit_code),
            format!("duration_ms: {}", event.duration_ms),
        ],
        AgenaSessionEvent::StreamError(event) => vec![
            format!("session_id: {}", event.session_id),
            format!("error_code: {}", event.error.code),
            format!("error_message: {}", event.error.message),
        ],
        AgenaSessionEvent::PermissionRequested(event) => {
            let mut lines = vec![
                format!("session_id: {}", event.session_id),
                format!("request_id: {}", event.request_id),
                format!("reason: {}", event.reason),
                format!("risk: {}", permission_risk_label(event.risk)),
                format!(
                    "explanation: {}",
                    detail_excerpt(event.explanation.as_str(), 200)
                ),
            ];
            if let Some(source) = event.source.as_deref() {
                lines.push(format!("source: {source}"));
            }
            if let Some(scope) = event.scope.as_deref() {
                lines.push(format!("scope: {scope}"));
            }
            if let Some(operator) = event.operator.as_deref() {
                lines.push(format!("operator: {operator}"));
            }
            append_permission_trace_strings(&mut lines, &event.trace);
            lines
        }
        AgenaSessionEvent::PermissionReplied(event) => vec![
            format!("session_id: {}", event.session_id),
            format!("request_id: {}", event.request_id),
            format!("kind: {:?}", event.kind),
            format!(
                "reason: {}",
                event.reason.clone().unwrap_or_else(|| "<none>".to_string())
            ),
        ],
        AgenaSessionEvent::PermissionRuleCreated(event)
        | AgenaSessionEvent::PermissionRuleUpdated(event)
        | AgenaSessionEvent::PermissionRuleRevoked(event) => vec![
            format!("rule_id: {}", event.rule_id),
            format!("action_key: {}", event.action_key),
            format!("mode: {}", event.mode),
            format!("scope: {}", event.scope),
            format!("source: {}", event.source),
        ],
        AgenaSessionEvent::SessionGoalUpdated(event) => vec![
            format!("session_id: {}", event.session_id),
            format!(
                "goal_id: {}",
                event
                    .goal_id
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<none>".to_string())
            ),
            format!(
                "objective: {}",
                event
                    .objective
                    .clone()
                    .unwrap_or_else(|| "<none>".to_string())
            ),
            format!(
                "status: {}",
                event.status.clone().unwrap_or_else(|| "<none>".to_string())
            ),
            format!(
                "completed_at_ms: {}",
                event
                    .completed_at_ms
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<none>".to_string())
            ),
        ],
        AgenaSessionEvent::RunStarted(p) => vec![
            format!("run_id: {}", p.run_id),
            format!("model: {} / {}", p.provider_id, p.model_id),
        ],
        AgenaSessionEvent::RunCompleted(p) => vec![
            format!("run_id: {}", p.run_id),
            format!("finish: {:?}", p.finish_reason),
        ],
        AgenaSessionEvent::RunAborted(p) => vec![
            format!("run_id: {}", p.run_id),
            format!("reason: {:?}", p.reason),
            format!(
                "message: {}",
                p.message.clone().unwrap_or_else(|| "<none>".to_string())
            ),
        ],
        AgenaSessionEvent::UserMessageAppended(p) => vec![
            format!("message_id: {}", p.message_id),
            format!("run_id: {}", p.run_id),
        ],
        AgenaSessionEvent::AssistantMessageCompleted(p) => vec![
            format!("message_id: {}", p.message_id),
            format!("run_id: {}", p.run_id),
            format!("finish: {:?}", p.finish_reason),
        ],
        AgenaSessionEvent::ToolCallIssued(p) => vec![
            format!("call_id: {}", p.call_id),
            format!("name: {}", p.name),
            format!("run_id: {}", p.run_id),
        ],
        AgenaSessionEvent::ToolCallCompleted(p) => vec![
            format!("call_id: {}", p.call_id),
            format!("run_id: {}", p.run_id),
        ],
        AgenaSessionEvent::SystemNoticeAppended(p) => vec![
            format!("message_id: {}", p.message_id),
            format!("kind: {:?}", p.kind),
            format!("text: {}", detail_excerpt(p.text.as_str(), 200)),
        ],
        AgenaSessionEvent::PluginEvent(p) => vec![
            format!("plugin_id: {}", p.plugin_id),
            format!("kind_label: {}", p.kind_label),
            format!("payload: {}", detail_excerpt(&p.payload.to_string(), 200)),
        ],
    }
}

fn permission_risk_label(risk: PermissionRiskLevel) -> &'static str {
    match risk {
        PermissionRiskLevel::Low => "low",
        PermissionRiskLevel::Medium => "medium",
        PermissionRiskLevel::High => "high",
        PermissionRiskLevel::Critical => "critical",
    }
}

fn permission_trace_step_label(step: &DecisionTraceStep) -> String {
    let source_kind = match step.source_kind {
        PolicySourceKind::StaticPolicy => "static_policy",
        PolicySourceKind::PersistedRule => "persisted_rule",
        PolicySourceKind::PluginAdvice => "plugin_advice",
        PolicySourceKind::ManagedPolicy => "managed_policy",
    };
    let mut facts = vec![source_kind.to_string()];
    if let Some(source) = step.source.as_deref() {
        facts.push(format!("source={source}"));
    }
    if let Some(scope) = step.scope {
        facts.push(format!("scope={scope}"));
    }
    if let Some(operator) = step.operator.as_deref() {
        facts.push(format!("operator={operator}"));
    }
    format!("- {} — {}", facts.join(" · "), step.summary)
}

fn append_permission_trace_lines(lines: &mut Vec<Line<'static>>, trace: &[DecisionTraceStep]) {
    if trace.is_empty() {
        return;
    }
    lines.push(Line::from("Trace:"));
    lines.extend(
        trace
            .iter()
            .map(|step| Line::from(permission_trace_step_label(step))),
    );
}

fn append_permission_trace_strings(lines: &mut Vec<String>, trace: &[DecisionTraceStep]) {
    if trace.is_empty() {
        return;
    }
    lines.push("trace:".to_string());
    lines.extend(trace.iter().map(permission_trace_step_label));
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

fn derive_session_title(text: &str) -> String {
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("New session");
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

fn split_editor_lines_with_offsets(text: &str) -> Vec<Range<usize>> {
    let mut start = 0;
    let mut lines = Vec::new();
    for (index, ch) in text.char_indices() {
        if ch == '\n' {
            lines.push(start..index);
            start = index + 1;
        }
    }
    lines.push(start..text.len());
    lines
}

fn previous_grapheme_boundary(text: &str, index: usize) -> usize {
    text[..index]
        .grapheme_indices(true)
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let grapheme = text[index..].graphemes(true).next().unwrap_or_default();
    index + grapheme.len()
}

fn byte_index_at_display_column(line: &str, offset: usize, target_column: usize) -> usize {
    let mut width = 0_usize;
    for (index, grapheme) in line.grapheme_indices(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > target_column {
            return offset + index;
        }
        width = width.saturating_add(grapheme_width);
    }
    offset + line.len()
}

fn slice_display_window_styled(
    text: &str,
    range: Range<usize>,
    start_column: usize,
    width: usize,
    elements: &[EditorElement],
) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }

    let end_column = start_column.saturating_add(width);
    let line_text = &text[range.clone()];
    let mut current_column = 0_usize;
    let mut current_style = Style::default();
    let mut current_segment = String::new();
    let mut spans = Vec::new();

    for (offset, grapheme) in line_text.grapheme_indices(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        let next_column = current_column.saturating_add(grapheme_width);
        if next_column <= start_column {
            current_column = next_column;
            continue;
        }
        if current_column >= end_column {
            break;
        }

        let absolute_start = range.start + offset;
        let absolute_end = absolute_start + grapheme.len();
        let style = if elements
            .iter()
            .any(|element| element.range.start < absolute_end && element.range.end > absolute_start)
        {
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        if !current_segment.is_empty() && style != current_style {
            spans.push(Span::styled(
                std::mem::take(&mut current_segment),
                current_style,
            ));
        }
        current_style = style;
        current_segment.push_str(grapheme);
        current_column = next_column;
    }

    if !current_segment.is_empty() {
        spans.push(Span::styled(current_segment, current_style));
    }

    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
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

fn is_altgr(modifiers: KeyModifiers) -> bool {
    cfg!(windows)
        && modifiers.contains(KeyModifiers::CONTROL)
        && modifiers.contains(KeyModifiers::ALT)
}

fn is_word_separator(ch: char) -> bool {
    WORD_SEPARATORS.contains(ch)
}

fn is_word_grapheme(grapheme: &str) -> bool {
    grapheme
        .chars()
        .any(|ch| !ch.is_whitespace() && !is_word_separator(ch))
}

fn retro_start_index(before: &str, retro_chars: usize) -> usize {
    let mut index = before.len();
    for _ in 0..retro_chars {
        let previous = previous_grapheme_boundary(before, index);
        if previous == index {
            break;
        }
        index = previous;
    }
    index
}

impl PasteBurst {
    fn on_plain_char(&mut self, ch: char, now: Instant) -> PasteCharDecision {
        let interval = Duration::from_millis(PASTE_BURST_CHAR_INTERVAL_MS);
        match self.last_plain_char_time {
            Some(previous) if now.duration_since(previous) <= interval => {
                self.consecutive_plain_char_burst =
                    self.consecutive_plain_char_burst.saturating_add(1);
            }
            _ => self.consecutive_plain_char_burst = 1,
        }
        self.last_plain_char_time = Some(now);

        if self.active {
            self.extend_window(now);
            return PasteCharDecision::BufferAppend;
        }

        if let Some((held, held_at)) = self.pending_first_char
            && now.duration_since(held_at) <= interval
        {
            self.active = true;
            let _ = self.pending_first_char.take();
            self.buffer.push(held);
            self.extend_window(now);
            return PasteCharDecision::BeginBufferFromPending;
        }

        if self.consecutive_plain_char_burst >= PASTE_BURST_MIN_CHARS {
            return PasteCharDecision::BeginBuffer {
                retro_chars: self.consecutive_plain_char_burst.saturating_sub(1),
            };
        }

        self.pending_first_char = Some((ch, now));
        PasteCharDecision::RetainFirstChar
    }

    fn flush_if_due(&mut self, now: Instant) -> PasteFlushResult {
        let timed_out = self.last_plain_char_time.is_some_and(|previous| {
            now.duration_since(previous) > Duration::from_millis(PASTE_BURST_CHAR_INTERVAL_MS)
        });

        if !timed_out {
            return PasteFlushResult::None;
        }

        self.flush_now()
    }

    fn flush_now(&mut self) -> PasteFlushResult {
        self.last_plain_char_time = None;
        self.consecutive_plain_char_burst = 0;

        if self.active || !self.buffer.is_empty() {
            self.active = false;
            self.burst_window_until = None;
            let text = std::mem::take(&mut self.buffer);
            return PasteFlushResult::Paste(text);
        }

        if let Some((ch, _)) = self.pending_first_char.take() {
            self.burst_window_until = None;
            return PasteFlushResult::Typed(ch);
        }

        PasteFlushResult::None
    }

    fn append_char_to_buffer(&mut self, ch: char, now: Instant) {
        self.buffer.push(ch);
        self.extend_window(now);
    }

    fn begin_with_retro_grabbed(&mut self, grabbed: String, now: Instant) {
        if !grabbed.is_empty() {
            self.buffer.push_str(grabbed.as_str());
        }
        self.active = true;
        self.extend_window(now);
    }

    fn newline_should_insert_instead_of_submit(&self, now: Instant) -> bool {
        self.active
            || self.burst_window_until.is_some_and(|until| now <= until)
            || self.pending_first_char.is_some()
    }

    fn append_newline_if_active(&mut self, now: Instant) -> bool {
        if self.active {
            self.buffer.push('\n');
            self.extend_window(now);
            true
        } else {
            false
        }
    }

    fn extend_window(&mut self, now: Instant) {
        self.burst_window_until = Some(now + Duration::from_millis(PASTE_ENTER_SUPPRESS_WINDOW_MS));
    }

    fn clear_window_after_non_char(&mut self) {
        self.last_plain_char_time = None;
        self.consecutive_plain_char_burst = 0;
        self.burst_window_until = None;
        self.active = false;
        self.pending_first_char = None;
        self.buffer.clear();
    }
}

impl RunOptionsState {
    fn clear_model_stack(&mut self) {
        self.model = None;
        self.thinking_mode = None;
        self.speed_mode = None;
        self.verbosity = None;
        self.parallel_tool_calls = None;
    }

    fn runtime_setting_summary(&self, field: RuntimeSettingSpec) -> String {
        match field.id {
            RuntimeSettingId::ThinkingMode => self
                .thinking_mode
                .as_deref()
                .map(|value| format!("override \"{value}\""))
                .unwrap_or_else(|| "default".to_string()),
            RuntimeSettingId::SpeedMode => self
                .speed_mode
                .as_deref()
                .map(|value| format!("override \"{value}\""))
                .unwrap_or_else(|| "default".to_string()),
            RuntimeSettingId::Verbosity => self
                .verbosity
                .as_deref()
                .map(|value| format!("override \"{value}\""))
                .unwrap_or_else(|| "default".to_string()),
            RuntimeSettingId::ParallelToolCalls => self
                .parallel_tool_calls
                .map(|value| format!("override {}", if value { "on" } else { "off" }))
                .unwrap_or_else(|| "default".to_string()),
            RuntimeSettingId::Temperature => self
                .temperature
                .map(|value| format!("override {value:.2}"))
                .unwrap_or_else(|| "default".to_string()),
            RuntimeSettingId::MaxOutput => self
                .max_output_tokens
                .map(|value| format!("override {}", value))
                .unwrap_or_else(|| "default".to_string()),
            RuntimeSettingId::System => self
                .system
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| {
                    format!(
                        "override {}",
                        format_setting_value_inline(&JsonValue::String(value.clone()))
                    )
                })
                .unwrap_or_else(|| "default".to_string()),
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
        field: RuntimeSettingSpec,
        input: &str,
    ) -> std::result::Result<String, String> {
        let trimmed = input.trim();
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
            return Ok(format!("cleared {}", field.label.to_ascii_lowercase()));
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
                        return Err(format!(
                            "{} expects true/false, on/off, yes/no, or 1/0",
                            field.label
                        ));
                    }
                };
                self.parallel_tool_calls = Some(value);
            }
            RuntimeSettingId::Temperature => {
                let value = trimmed
                    .parse::<f32>()
                    .map_err(|_| format!("{} expects a numeric value", field.label))?;
                if !value.is_finite() {
                    return Err(format!("{} expects a finite number", field.label));
                }
                self.temperature = Some(value);
            }
            RuntimeSettingId::MaxOutput => {
                let value = trimmed
                    .parse::<u32>()
                    .map_err(|_| format!("{} expects a positive integer", field.label))?;
                if value == 0 {
                    return Err(format!("{} expects a positive integer", field.label));
                }
                self.max_output_tokens = Some(value);
            }
            RuntimeSettingId::System => self.system = Some(trimmed.to_string()),
        }

        Ok(format!("updated {}", field.label.to_ascii_lowercase()))
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

    fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(model) = self.model.as_ref() {
            parts.push(format!("{}/{}", model.provider_id, model.model_id));
        }
        if let Some(thinking_mode) = self.thinking_mode.as_ref() {
            parts.push(format!("thinking {}", thinking_mode));
        }
        if let Some(speed_mode) = self.speed_mode.as_ref() {
            parts.push(format!("speed {}", speed_mode));
        }
        if let Some(verbosity) = self.verbosity.as_ref() {
            parts.push(format!("verbosity {}", verbosity));
        }
        if let Some(parallel_tool_calls) = self.parallel_tool_calls {
            parts.push(format!(
                "parallel-tools {}",
                if parallel_tool_calls { "on" } else { "off" }
            ));
        }
        if let Some(temperature) = self.temperature {
            parts.push(format!("temp {:.2}", temperature));
        }
        if let Some(max_output_tokens) = self.max_output_tokens {
            parts.push(format!("max {}", max_output_tokens));
        }
        if self
            .system
            .as_ref()
            .is_some_and(|system| !system.trim().is_empty())
        {
            parts.push("system".to_string());
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
    base.push(".agena");
    base.push("tui-drafts.json");
    base
}

fn default_prompt_history_path() -> PathBuf {
    let mut base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.push(".agena");
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

fn permission_rule_label(rule: &PermissionRuleResource) -> String {
    match rule.subject_kind.as_str() {
        "tool" => match (rule.tool_name.as_deref(), rule.qualifier.as_deref()) {
            (Some(tool_name), Some(qualifier)) if !qualifier.trim().is_empty() => {
                format!("{tool_name} · {qualifier}")
            }
            (Some(tool_name), _) => tool_name.to_string(),
            _ => rule.action_key.clone(),
        },
        "path_access" => format!(
            "{} · {}",
            rule.path_access_kind.as_deref().unwrap_or("path"),
            rule.target_path
                .as_deref()
                .unwrap_or(rule.action_key.as_str())
        ),
        "network_access" => {
            let host = rule
                .network_host
                .as_deref()
                .or(rule.network_target.as_deref())
                .unwrap_or(rule.action_key.as_str());
            match rule.network_port {
                Some(port) => format!("network · {host}:{port}"),
                None => format!("network · {host}"),
            }
        }
        _ => rule.action_key.clone(),
    }
}

fn permission_rule_scope_label(rule: &PermissionRuleResource) -> String {
    match rule.scope.as_str() {
        "session" => rule
            .session_id
            .map(|id| format!("session #{id}"))
            .unwrap_or_else(|| "session".to_string()),
        "workspace" => rule
            .workspace_id
            .map(|id| format!("workspace #{id}"))
            .unwrap_or_else(|| "workspace".to_string()),
        other => other.to_string(),
    }
}

fn permission_rule_detail(rule: &PermissionRuleResource) -> String {
    let mut facts = vec![
        format!("mode={}", permission_mode_name(rule.mode)),
        format!("scope={}", permission_rule_scope_label(rule)),
        format!("source={}", rule.source),
    ];
    if let Some(operator) = rule
        .operator
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        facts.push(format!("operator={operator}"));
    }
    if let Some(reason) = rule
        .reason
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        facts.push(format!("reason={reason}"));
    }
    facts.push(format!("updated={}", rule.updated_at));
    facts.join(" · ")
}

fn permission_rule_draft_label(draft: &PermissionRuleDraft) -> String {
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
        PermissionRuleSubjectKind::PathAccess => {
            format!(
                "{} · {}",
                draft.path_access_kind.trim(),
                draft.target_path.trim()
            )
        }
        PermissionRuleSubjectKind::NetworkAccess => {
            let target = draft.network_target.trim();
            if target.is_empty() {
                "network".to_string()
            } else {
                format!("network · {target}")
            }
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

fn render_permission_rule_preview(input: &str) -> String {
    match parse_permission_rule_input(input) {
        Ok(draft) => {
            let mut lines = vec![format!("label: {}", permission_rule_draft_label(&draft))];
            lines.push(format!("mode: {}", permission_mode_name(draft.mode)));
            lines.push(format!("scope: {}", draft.scope));
            match draft.subject_kind {
                PermissionRuleSubjectKind::Tool => {
                    lines.push(format!("subject: tool ({})", draft.tool_name.trim()));
                    if !draft.qualifier.trim().is_empty() {
                        lines.push(format!("qualifier: {}", draft.qualifier.trim()));
                    }
                }
                PermissionRuleSubjectKind::PathAccess => {
                    lines.push(format!(
                        "subject: path_access ({})",
                        draft.path_access_kind.trim()
                    ));
                    lines.push(format!("target: {}", draft.target_path.trim()));
                    if !draft.workspace_root.trim().is_empty() {
                        lines.push(format!("workspace_root: {}", draft.workspace_root.trim()));
                    }
                }
                PermissionRuleSubjectKind::NetworkAccess => {
                    lines.push("subject: network_access".to_string());
                    lines.push(format!("target: {}", draft.network_target.trim()));
                }
            }
            if draft.scope == "session" && !draft.session_id.trim().is_empty() {
                lines.push(format!("session_id: {}", draft.session_id.trim()));
            }
            lines.join("\n")
        }
        Err(error) => format!("invalid rule: {error}"),
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

fn parse_permission_rule_input(input: &str) -> std::result::Result<PermissionRuleDraft, String> {
    let tokens = shlex::split(input).ok_or_else(|| "invalid shell-style arguments".to_string())?;
    if tokens.len() < 4 {
        return Err(
            "expected a structured rule starting with tool/path and ending with allow|ask|deny"
                .to_string(),
        );
    }
    let subject = tokens[0].to_ascii_lowercase();
    let mut draft = PermissionRuleDraft::default();
    match subject.as_str() {
        "tool" => {
            draft.subject_kind = PermissionRuleSubjectKind::Tool;
            draft.tool_name = tokens[1].clone();
            draft.mode = parse_permission_mode_token(tokens[2].as_str())?;
            for token in &tokens[3..] {
                let (key, value) = split_permission_rule_option(token)?;
                match key {
                    "qualifier" => draft.qualifier = value.to_string(),
                    "scope" => draft.scope = parse_permission_scope_token(value)?.to_string(),
                    "session" => draft.session_id = value.to_string(),
                    _ => return Err(format!("unknown permission rule option: {key}")),
                }
            }
            if draft.tool_name.trim().is_empty() {
                return Err("tool_name is required".to_string());
            }
        }
        "path" => {
            draft.subject_kind = PermissionRuleSubjectKind::PathAccess;
            draft.path_access_kind = tokens[1].clone();
            draft.target_path = tokens[2].clone();
            draft.mode = parse_permission_mode_token(tokens[3].as_str())?;
            for token in &tokens[4..] {
                let (key, value) = split_permission_rule_option(token)?;
                match key {
                    "scope" => draft.scope = parse_permission_scope_token(value)?.to_string(),
                    "session" => draft.session_id = value.to_string(),
                    "workspace_root" => draft.workspace_root = value.to_string(),
                    _ => return Err(format!("unknown permission rule option: {key}")),
                }
            }
            if draft.path_access_kind.trim().is_empty() {
                return Err("path_access_kind is required".to_string());
            }
            if draft.target_path.trim().is_empty() {
                return Err("target_path is required".to_string());
            }
        }
        "network" => {
            draft.subject_kind = PermissionRuleSubjectKind::NetworkAccess;
            draft.network_target = tokens[1].clone();
            draft.mode = parse_permission_mode_token(tokens[2].as_str())?;
            for token in &tokens[3..] {
                let (key, value) = split_permission_rule_option(token)?;
                match key {
                    "scope" => draft.scope = parse_permission_scope_token(value)?.to_string(),
                    "session" => draft.session_id = value.to_string(),
                    _ => return Err(format!("unknown permission rule option: {key}")),
                }
            }
            if draft.network_target.trim().is_empty() {
                return Err("network target is required".to_string());
            }
        }
        _ => {
            return Err("rule subject must start with `tool`, `path`, or `network`".to_string());
        }
    }
    if draft.scope == "session" && draft.session_id.trim().is_empty() {
        return Err("session scope requires session=<id>".to_string());
    }
    Ok(draft)
}

fn parse_permission_mode_token(token: &str) -> std::result::Result<PermissionMode, String> {
    match token.to_ascii_lowercase().as_str() {
        "allow" => Ok(PermissionMode::Allow),
        "ask" => Ok(PermissionMode::Ask),
        "deny" => Ok(PermissionMode::Deny),
        _ => Err("permission mode must be allow, ask, or deny".to_string()),
    }
}

fn parse_permission_scope_token(token: &str) -> std::result::Result<&'static str, String> {
    match token.to_ascii_lowercase().as_str() {
        "session" => Ok("session"),
        "workspace" => Ok("workspace"),
        "global" => Ok("global"),
        _ => Err("scope must be session, workspace, or global".to_string()),
    }
}

fn split_permission_rule_option(token: &str) -> std::result::Result<(&str, &str), String> {
    token
        .split_once('=')
        .ok_or_else(|| format!("expected key=value option, got `{token}`"))
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

fn runtime_entry_matches_slash_query(label: &str, query: &str) -> bool {
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
    use agena::message::{ExecutionStatus, MessagePart, PartContent};
    use agena::plugin::status::{PluginRunState, PluginStatus};
    use chrono::Utc;

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
            questions: Vec::new(),
            created_at: Utc::now(),
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
    fn composer_input_is_active_only_when_the_composer_is_engaged() {
        assert!(composer_input_is_active(Focus::Composer, true, false));
        assert!(composer_input_is_active(Focus::Composer, false, true));
        assert!(!composer_input_is_active(Focus::Composer, false, false));
        assert!(!composer_input_is_active(Focus::Transcript, true, true));
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
    fn settings_plugin_entries_include_runtime_builtin_plugins() {
        let sources = ConfigJsonSources {
            config_path: PathBuf::from("/tmp/agena-config.json"),
            config_found: true,
            file: json!({}),
            effective: json!({ "plugins": { "list": {} } }),
        };
        let items = settings_studio_plugin_entry_items(
            &sources,
            &[PluginStatus {
                plugin_id: "agena.fs".to_string(),
                kind: "static",
                state: PluginRunState::Running,
                pid: None,
                restart_count: 0,
                last_exit_code: None,
                last_restart_at_ms: None,
                last_error: None,
            }],
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "agena.fs");
        assert!(items[0].value.contains("builtin"));
        assert!(items[0].detail.contains("built-in plugin"));
    }
}
