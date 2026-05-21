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
    config::get_json_path,
    event::{DomainEvent, EventKind as AgenaSessionEvent},
    message::{
        AttachmentKind, ExecutionStatus, MessagePart, MessageStatus, OperationPart, PartContent,
        ToolInvocation, UserInputQuestion, UserInputReply, UserInputReplyKind, UserInputRequest,
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
        MessageResource, MessageRole, PermissionRuleResource, ProviderAdapterModelsResource,
        ProviderAdapterModelsResponse, ProviderSummaryResource, RunOptions,
        SessionExecutionResource, SessionResource, SessionRunState, SessionUsageResource,
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
use serde_json::Value as JsonValue;
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
use crate::tui_config::{TuiComposerMode, TuiConfig, TuiStatusLineConfig};
use crate::ui_text;
use agena_api_server::local_api::{
    ModelCatalogEntryResource, ModelCatalogListResponse, ModelCatalogResponse,
};

mod provider_studio;
mod transcript_view;
mod view;

use self::provider_studio::*;

#[cfg(test)]
use self::transcript_view::tool_output_preview;
use self::transcript_view::{
    render_message, render_transcript_export_markdown, rewind_message_preview,
    sanitize_terminal_text,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComposerMode {
    Emacs,
    VimInsert,
    VimNormal,
}

impl ComposerMode {
    fn from_config(mode: TuiComposerMode) -> Self {
        match mode {
            TuiComposerMode::Emacs => Self::Emacs,
            TuiComposerMode::Vim => Self::VimInsert,
        }
    }

    fn is_vim(self) -> bool {
        matches!(self, Self::VimInsert | Self::VimNormal)
    }

    fn status_label(self) -> &'static str {
        match self {
            Self::Emacs => "EMACS",
            Self::VimInsert => "INSERT",
            Self::VimNormal => "NORMAL",
        }
    }
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
    overlay: Option<Overlay>,
    overlay_stack: Vec<Overlay>,
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
    composer_mode: ComposerMode,
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
    /// FIFO once the active turn finishes. See `composer_queue.rs`.
    queue: ComposerQueue,
    status_line: Option<StatusLineState>,
    plugin_theme: Option<agena::plugin::HostThemePalette>,
    keybindings: ComposerKeyBindings,
    /// Last time the user pressed Esc inside the composer; used to detect
    /// a double-tap that clears the input.
    last_esc_at: Option<Instant>,
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
    SessionTurnSubmitted {
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
    /// backend accepted the steer; `Err` when the turn was no longer
    /// steerable (e.g. terminal phase). On error we re-enqueue the draft
    /// so the user's intent isn't dropped.
    SteerSubmitted {
        session_id: i64,
        draft: ComposerDraft,
        result: UiResult<()>,
    },
    /// Result of a `request_cancel_turn` call. We always treat the in-flight
    /// turn as gone when this lands, regardless of success — the user has
    /// already signalled cancel intent.
    TurnCancelled {
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
    Help(HelpOverlay),
    TranscriptSearch(LineInputOverlay),
    SessionRename(LineInputOverlay),
    SettingsStudio(SettingsStudioOverlay),
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
    PluginInspector(PluginInspectorOverlay),
    ProviderStudio(ProviderStudioOverlay),
    ModelCatalogStudio(ModelCatalogStudioOverlay),
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
    PermissionRule(PermissionRuleResource),
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
    search_query: String,
    search_match_index: Option<usize>,
    execution: Option<SessionExecutionResource>,
    last_event_seq: Option<i64>,
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
}

#[derive(Debug, Clone)]
struct RenderedLine {
    text: String,
    style: Style,
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
            focus: Focus::Composer,
            overlay: None,
            overlay_stack: Vec::new(),
            seen_user_input_request_ids: BTreeSet::new(),
            flash: None,
            sessions: SessionListState {
                search_query: launch.initial_session_search.unwrap_or_default(),
                ..SessionListState::default()
            },
            transcript: TranscriptState::new(i18n),
            run_options: RunOptionsState::default(),
            composer: Editor::default(),
            composer_items: Vec::new(),
            slash_command_suggestions: None,
            dismissed_slash_command_suggestions_for: None,
            file_mention_suggestions: None,
            dismissed_file_mention_suggestions_for: None,
            prompt_history_search: None,
            selected_composer_item: None,
            composer_mode: ComposerMode::from_config(launch.tui_config.composer_mode),
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
            last_esc_at: None,
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
            Event::FocusGained | Event::FocusLost | Event::Mouse(_) => {}
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

        if key.modifiers.contains(KeyModifiers::ALT)
            && matches!(key.code, KeyCode::Char('s') | KeyCode::Char('S'))
        {
            self.open_resume_session_picker();
            return;
        }

        // ESC while a turn is in flight has global priority. Cancel the
        // active turn before falling through to focus-specific Esc.
        if matches!(key.code, KeyCode::Esc)
            && key.modifiers.is_empty()
            && self.transcript.submitting
            && let Some(session_id) = self.transcript.session_id
        {
            self.transcript.submitting = false;
            self.submitting_session_ids.remove(&session_id);
            self.request_cancel_turn(session_id);
            return;
        }

        if self.focus != Focus::Composer {
            match key.code {
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('?') => {
                    self.overlay = Some(Overlay::Help(HelpOverlay::default()));
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
    }

    fn handle_overlay_key(&mut self, key: KeyEvent) -> bool {
        let Some(mut overlay) = self.overlay.take() else {
            return false;
        };

        let close = match &mut overlay {
            Overlay::Help(dialog) => self.handle_help_overlay_key(key, dialog),
            Overlay::TranscriptSearch(dialog) => {
                self.handle_line_overlay_key(key, dialog, OverlayCommit::TranscriptSearch)
            }
            Overlay::SessionRename(dialog) => self.handle_session_rename_overlay_key(key, dialog),
            Overlay::SettingsStudio(dialog) => self.handle_settings_studio_overlay_key(key, dialog),
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
            Overlay::PluginInspector(dialog) => {
                self.handle_plugin_inspector_overlay_key(key, dialog)
            }
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
                        self.open_permission_rule_picker(dialog.return_query.as_str());
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
                self.open_permission_rule_editor(None, dialog.input.text());
                true
            }
            KeyCode::Char('d') if matches!(dialog.kind, PickerKind::PermissionRules) => {
                let Some(item) = dialog.items.get(dialog.selected).cloned() else {
                    return false;
                };
                if let PickerValue::PermissionRule(rule) = item.value {
                    self.open_revoke_permission_rule_confirm(&rule, dialog.input.text());
                    true
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
                            self.open_permission_rule_editor(None, dialog.input.text());
                            return true;
                        }
                        PickerValue::PermissionRule(rule) => {
                            self.open_permission_rule_editor(Some(&rule), dialog.input.text());
                            return true;
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
                    if let ProviderStudioEditorAction::Field(field) = editor.action {
                        if let Err(error) = self.commit_provider_studio_field(dialog, field, value)
                        {
                            self.flash_error(error);
                            return false;
                        }
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
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::SettingsStudio(_) => {}
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
                Overlay::PluginInspector(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_plugin_inspector_overlay(dialog);
                }
                Overlay::Confirm(_) => {}
                Overlay::Permission(_) => {}
                Overlay::Help(_) => {}
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
        if matches!(key.code, KeyCode::Char('y')) {
            self.copy_loaded_transcript();
        } else if matches!(key.code, KeyCode::Char('Y')) {
            self.copy_visible_transcript();
        } else if matches!(key.code, KeyCode::Char('c')) {
            self.copy_last_assistant_message();
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
            if self.composer_mode == ComposerMode::VimInsert {
                self.composer_mode = ComposerMode::VimNormal;
                self.sync_composer_suggestions();
                return;
            }
            self.handle_composer_esc();
            return;
        }
        if self.composer_mode == ComposerMode::VimNormal && self.handle_vim_normal_key(key) {
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
                    self.last_esc_at = None;
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

    fn handle_vim_normal_key(&mut self, key: KeyEvent) -> bool {
        match key {
            KeyEvent {
                code: KeyCode::Char('h'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_left();
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('j'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_down();
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('k'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_up();
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('l'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_right();
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('w'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_word_right();
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('b'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_word_left();
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('0'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_home(false);
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('$'),
                modifiers: KeyModifiers::SHIFT,
                ..
            } => {
                self.composer.move_end(false);
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('x'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.delete();
                self.after_composer_text_mutated();
                true
            }
            KeyEvent {
                code: KeyCode::Char('i'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer_mode = ComposerMode::VimInsert;
                true
            }
            KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_right();
                self.composer_mode = ComposerMode::VimInsert;
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('I'),
                modifiers: KeyModifiers::SHIFT,
                ..
            } => {
                self.composer.move_home(false);
                self.composer_mode = ComposerMode::VimInsert;
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('A'),
                modifiers: KeyModifiers::SHIFT,
                ..
            } => {
                self.composer.move_end(false);
                self.composer_mode = ComposerMode::VimInsert;
                self.sync_composer_suggestions();
                true
            }
            KeyEvent {
                code: KeyCode::Char('o'),
                modifiers: KeyModifiers::NONE,
                ..
            } => {
                self.composer.move_end(false);
                self.composer.insert_explicit_newline();
                self.composer_mode = ComposerMode::VimInsert;
                self.after_composer_text_mutated();
                true
            }
            KeyEvent {
                code: KeyCode::Char('O'),
                modifiers: KeyModifiers::SHIFT,
                ..
            } => {
                let line_start = self.composer.current_line_start();
                self.composer.insert_str_at(line_start, "\n");
                self.composer.cursor = line_start;
                self.composer_mode = ComposerMode::VimInsert;
                self.after_composer_text_mutated();
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
        if self.focus != Focus::Composer || self.overlay.is_some() {
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
        if self.focus != Focus::Composer || self.overlay.is_some() {
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

    /// Single-Esc dismisses transient composer UI. Double-Esc within the
    /// configured window clears the input without leaving the composer.
    fn handle_composer_esc(&mut self) {
        let now = Instant::now();
        let double = self
            .last_esc_at
            .map(|prev| now.duration_since(prev) <= self.double_esc_window)
            .unwrap_or(false);
        if double {
            self.reset_prompt_history_recall();
            self.clear_composer_state();
            self.last_esc_at = None;
            return;
        }
        self.last_esc_at = Some(now);
        self.slash_command_suggestions = None;
        self.file_mention_suggestions = None;
        self.prompt_history_search = None;
        self.selected_composer_item = None;
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
            AppMessage::SessionTurnSubmitted {
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
            AppMessage::TurnCancelled { session_id, result } => {
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
                    self.request_submit_turn(session.id, draft);
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
                self.transcript.apply_execution(execution);
                self.maybe_auto_open_user_input_overlay();
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
                    self.transcript.apply_execution(execution);
                    self.maybe_auto_open_user_input_overlay();
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
                self.transcript.apply_execution(execution);
                self.maybe_auto_open_user_input_overlay();
                self.request_refresh(session_id, true);
                self.request_sessions(false);
                // Pop the next pending message and submit it after the turn.
                self.try_drain_queue_one();
            }
            Err(error) => {
                self.transcript.pending_restore_draft = None;
                if self.transcript.session_id == Some(session_id) {
                    self.restore_composer_draft(draft);
                }
                self.flash_error(error);
                // Pause draining: a failed turn typically means the user
                // wants to inspect the error rather than fire the next
                // queued message blindly. They can press Up to recover
                // the queue contents.
            }
        }
    }

    /// Pop one editable message from the queue and submit it. Called
    /// whenever an in-flight turn completes successfully so the user sees
    /// their pending messages run automatically.
    fn try_drain_queue_one(&mut self) {
        if self.transcript.submitting {
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
                // Backend rejected the steer (turn no longer steerable).
                // Don't drop the user's message — push it onto the front
                // of the queue so it goes out at the next turn boundary.
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
        match result {
            Ok(()) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-turn-cancelled"));
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
            self.transcript.apply_execution(execution);
            self.maybe_auto_open_user_input_overlay();
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
        let Some(Overlay::Picker(mut dialog)) = self.overlay.take() else {
            return;
        };
        let PickerKind::Providers(current_purpose) = &dialog.kind else {
            self.overlay = Some(Overlay::Picker(dialog));
            return;
        };
        if *current_purpose != purpose {
            self.overlay = Some(Overlay::Picker(dialog));
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
        self.overlay = Some(Overlay::Picker(dialog));
    }

    fn handle_session_search_page_loaded(
        &mut self,
        mode: SessionViewMode,
        query: String,
        page_index: usize,
        result: UiResult<PaginatedResponse<SessionResource>>,
    ) {
        let Some(Overlay::SessionSearch(mut dialog)) = self.overlay.take() else {
            return;
        };
        if dialog.mode != mode
            || dialog.page_index != page_index
            || dialog.input.text().trim() != query
        {
            self.overlay = Some(Overlay::SessionSearch(dialog));
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
        self.overlay = Some(Overlay::SessionSearch(dialog));
    }

    fn handle_session_search_subtree_loaded(
        &mut self,
        session_id: i64,
        query: String,
        result: UiResult<Vec<SessionResource>>,
    ) {
        let Some(Overlay::SessionSearch(mut dialog)) = self.overlay.take() else {
            return;
        };
        if dialog.mode != SessionViewMode::Subtree
            || dialog.scope_session_id != Some(session_id)
            || dialog.input.text().trim() != query
        {
            self.overlay = Some(Overlay::SessionSearch(dialog));
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
        self.overlay = Some(Overlay::SessionSearch(dialog));
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

                let Some(Overlay::Picker(mut dialog)) = self.overlay.take() else {
                    return;
                };
                let PickerKind::Lineage {
                    session_id: current_session_id,
                } = &dialog.kind
                else {
                    self.overlay = Some(Overlay::Picker(dialog));
                    return;
                };
                if *current_session_id != session_id {
                    self.overlay = Some(Overlay::Picker(dialog));
                    return;
                }

                dialog.loading = false;
                dialog.empty_message = ui_text::t(&self.i18n, "overlay-lineage-empty");
                dialog.all_items = items
                    .into_iter()
                    .map(|item| self.lineage_session_picker_item(item))
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
                self.overlay = Some(Overlay::Picker(dialog));
            }
            Err(error) => {
                if let Some(Overlay::Picker(mut dialog)) = self.overlay.take() {
                    if matches!(dialog.kind, PickerKind::Lineage { session_id: current_session_id } if current_session_id == session_id)
                    {
                        dialog.loading = false;
                        dialog.empty_message = ui_text::t(&self.i18n, "overlay-lineage-empty");
                    }
                    self.overlay = Some(Overlay::Picker(dialog));
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
        let Some(Overlay::Picker(mut dialog)) = self.overlay.take() else {
            return;
        };
        let PickerKind::RewindMessages {
            session_id: current_session_id,
        } = &dialog.kind
        else {
            self.overlay = Some(Overlay::Picker(dialog));
            return;
        };
        if *current_session_id != session_id {
            self.overlay = Some(Overlay::Picker(dialog));
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
        self.overlay = Some(Overlay::Picker(dialog));
    }

    fn handle_session_model_chooser_loaded(
        &mut self,
        result: UiResult<Vec<SessionModelChoiceItem>>,
    ) {
        let Some(Overlay::SessionModelChooser(mut dialog)) = self.overlay.take() else {
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
        self.overlay = Some(Overlay::SessionModelChooser(dialog));
    }

    fn handle_model_catalog_loaded(
        &mut self,
        query: String,
        offset: usize,
        result: UiResult<ModelCatalogListResponse>,
    ) {
        let Some(Overlay::ModelCatalogStudio(mut dialog)) = self.overlay.take() else {
            return;
        };
        if dialog.query != query || dialog.offset != offset {
            self.overlay = Some(Overlay::ModelCatalogStudio(dialog));
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
        self.overlay = Some(Overlay::ModelCatalogStudio(dialog));
    }

    fn handle_provider_studio_adapter_models_loaded(
        &mut self,
        request_key: String,
        result: UiResult<ProviderAdapterModelsResponse>,
    ) {
        let Some(Overlay::ProviderStudio(mut dialog)) = self.overlay.take() else {
            return;
        };
        if dialog.pending_adapter_models_key.as_deref() != Some(request_key.as_str()) {
            self.overlay = Some(Overlay::ProviderStudio(dialog));
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
        self.overlay = Some(Overlay::ProviderStudio(dialog));
    }

    fn handle_provider_studio_auth_completed(
        &mut self,
        request_key: String,
        result: UiResult<crate::backend::ProviderDraftAuthActionResult>,
    ) {
        let Some(Overlay::ProviderStudio(mut dialog)) = self.overlay.take() else {
            match result {
                Ok(action) => self.flash_success(action.message),
                Err(error) => self.flash_error(error),
            }
            return;
        };
        if dialog.pending_auth_key.as_deref() != Some(request_key.as_str()) {
            self.overlay = Some(Overlay::ProviderStudio(dialog));
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
        self.overlay = Some(Overlay::ProviderStudio(dialog));
    }

    fn handle_provider_studio_saved(&mut self, provider_id: String, result: UiResult<String>) {
        let Some(Overlay::ProviderStudio(mut dialog)) = self.overlay.take() else {
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
        self.overlay = Some(Overlay::ProviderStudio(dialog));
    }

    fn handle_model_catalog_refreshed(&mut self, result: UiResult<()>) {
        let Some(Overlay::ModelCatalogStudio(mut dialog)) = self.overlay.take() else {
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
        self.overlay = Some(Overlay::ModelCatalogStudio(dialog));
    }

    fn handle_child_sessions_loaded(
        &mut self,
        parent_session_id: i64,
        result: UiResult<Vec<SessionResource>>,
    ) {
        let Some(Overlay::Picker(mut dialog)) = self.overlay.take() else {
            return;
        };
        let PickerKind::ChildSessions {
            parent_session_id: current_parent_id,
        } = &dialog.kind
        else {
            self.overlay = Some(Overlay::Picker(dialog));
            return;
        };
        if *current_parent_id != parent_session_id {
            self.overlay = Some(Overlay::Picker(dialog));
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
        self.overlay = Some(Overlay::Picker(dialog));
    }

    fn handle_timeline_loaded(&mut self, session_id: i64, result: UiResult<Vec<DomainEvent>>) {
        let Some(Overlay::Timeline(mut dialog)) = self.overlay.take() else {
            return;
        };
        if dialog.session_id != session_id {
            self.overlay = Some(Overlay::Timeline(dialog));
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
        self.overlay = Some(Overlay::Timeline(dialog));
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
                self.transcript.apply_execution(execution);
                self.maybe_auto_open_user_input_overlay();
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

    fn request_submit_turn(&mut self, session_id: i64, draft: ComposerDraft) {
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
                .submit_parts_turn_with_options(session_id, parts, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionTurnSubmitted {
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

    /// Steer the in-flight turn by injecting `parts` as a new user message
    /// the model will see on its next step. If the backend reports the
    /// turn is no longer steerable, fall back to enqueueing the original
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

    /// Ask the backend to cancel the in-flight turn for `session_id`.
    /// Best-effort: even if the backend hasn't fully wired cancellation,
    /// we clear the local `submitting` flag so the user regains control.
    fn request_cancel_turn(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .cancel_turn(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::TurnCancelled { session_id, result });
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
        self.transcript.reset(session_id, title);
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
    /// idle, sends a normal turn. When the AI is mid-turn, attempts to
    /// `steer_input` (Phase 3) — i.e. inject the message into the live
    /// turn so the model sees it on its next step. If the backend rejects
    /// the steer (e.g. the turn is in a non-steerable phase), we fall
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
    /// sends immediately. When the AI is mid-turn, the message is appended to
    /// the local pending queue and drained on turn completion.
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
            Some(session_id) => self.request_submit_turn(session_id, draft),
            None => self.create_session(Some(draft)),
        }
    }

    fn continue_current_session(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
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
        if self.session_is_busy(session_id) {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
            return;
        }
        self.request_compact(session_id);
    }

    fn reply_permission(&mut self, kind: PermissionReplyKind) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        let Some(request) = execution.pending_permission_requests.first() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.request_permission_reply(
            session_id,
            request.request_id.clone(),
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
        let request = execution.pending_user_input_requests.first()?.clone();
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

    fn maybe_auto_open_user_input_overlay(&mut self) {
        if self.overlay.is_some() {
            return;
        }
        let Some((session_id, request)) = self.pending_user_input_overlay_target() else {
            return;
        };
        if !self
            .seen_user_input_request_ids
            .insert(request.request_id.clone())
        {
            return;
        }
        self.overlay = Some(Overlay::UserInputReply(Self::build_user_input_overlay(
            session_id, request,
        )));
    }

    fn open_permission_overlay(&mut self) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        let Some(request) = execution.pending_permission_requests.first().cloned() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.overlay = Some(Overlay::Permission(PermissionOverlay {
            session_id,
            request,
            selected: 0,
        }));
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
        self.overlay = Some(Overlay::Timeline(TimelineOverlay {
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
        }));
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
        self.overlay = Some(Overlay::PluginInspector(dialog));
    }

    fn open_settings_studio(&mut self, query: &str) {
        match self.build_settings_studio_overlay(None, None, SettingsStudioFocus::Navigation) {
            Ok(mut dialog) => {
                self.select_settings_studio_query(&mut dialog, query);
                self.overlay_stack.clear();
                self.overlay = Some(Overlay::SettingsStudio(dialog));
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
        let provider_items = settings_studio_provider_items(&configured_providers);
        let model_catalog_items = settings_studio_model_catalog_items(&model_catalog);
        let file_items = settings_studio_file_items(&sources);
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
                summary: format!(
                    "{} entries · {} official · {} custom",
                    model_catalog.summary.entry_count,
                    model_catalog.summary.official_entry_count,
                    model_catalog.summary.custom_entry_count
                ),
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
                self.overlay_stack
                    .push(Overlay::SettingsStudio(dialog.clone()));
                self.open_settings_field_editor(field, "");
                true
            }
            SettingsPickerAction::EditRuntimeSetting(field) => {
                self.overlay_stack
                    .push(Overlay::SettingsStudio(dialog.clone()));
                self.open_runtime_setting_editor(field, "");
                true
            }
            SettingsPickerAction::OpenProviderWorkbench => {
                self.overlay_stack
                    .push(Overlay::SettingsStudio(dialog.clone()));
                self.open_provider_studio(None);
                true
            }
            SettingsPickerAction::OpenProviderWorkbenchFor(provider_id) => {
                self.overlay_stack
                    .push(Overlay::SettingsStudio(dialog.clone()));
                self.open_provider_studio(Some(provider_id.as_str()));
                true
            }
            SettingsPickerAction::OpenModelCatalogWorkbench => {
                self.overlay_stack
                    .push(Overlay::SettingsStudio(dialog.clone()));
                self.open_model_catalog_studio();
                true
            }
            SettingsPickerAction::OpenRuntimeProviderOverride => {
                self.overlay_stack
                    .push(Overlay::SettingsStudio(dialog.clone()));
                self.open_provider_picker(ProviderPickerPurpose::SetProvider);
                true
            }
            SettingsPickerAction::OpenRuntimeModelOverride => {
                self.overlay_stack
                    .push(Overlay::SettingsStudio(dialog.clone()));
                self.open_session_model_chooser();
                true
            }
            SettingsPickerAction::ClearRuntimeModelStack => {
                self.clear_provider_model_overrides();
                self.flash_success("cleared provider/model runtime override stack");
                self.refresh_settings_studio_overlay(dialog);
                false
            }
            SettingsPickerAction::OpenPermissionRules => {
                self.overlay_stack
                    .push(Overlay::SettingsStudio(dialog.clone()));
                self.open_permission_rule_picker("");
                true
            }
            SettingsPickerAction::OpenConfigFile => {
                self.open_runtime_config_in_editor();
                false
            }
        }
    }

    fn refresh_restored_overlay(&self, overlay: Overlay) -> Overlay {
        match overlay {
            Overlay::SettingsStudio(dialog) => self
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
                .map(Overlay::SettingsStudio)
                .unwrap_or(Overlay::SettingsStudio(dialog)),
            other => other,
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
                let Some(Overlay::ProviderStudio(mut parent)) = self.overlay_stack.pop() else {
                    self.flash_error("provider studio context was lost");
                    return true;
                };
                match self.commit_provider_studio_field(&mut parent, field, value) {
                    Ok(()) => {
                        self.overlay = Some(Overlay::ProviderStudio(parent));
                        true
                    }
                    Err(error) => {
                        self.flash_error(error);
                        self.overlay_stack.push(Overlay::ProviderStudio(parent));
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
            ProviderStudioField::ApiKeyEnv => {
                Some(provider_studio_api_key_env_choice_items(dialog))
            }
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
        self.overlay = Some(Overlay::Picker(overlay));
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
                    value: PickerValue::PermissionRule(rule),
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
                self.overlay = Some(Overlay::Picker(overlay));
            }
            Err(error) => self.flash_error(error),
        }
    }

    fn open_permission_rule_editor(
        &mut self,
        rule: Option<&PermissionRuleResource>,
        return_query: &str,
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
        self.overlay = Some(Overlay::Picker(overlay));
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
        self.overlay = Some(Overlay::SessionSearch(dialog));
    }

    fn open_lineage_picker(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.overlay = Some(Overlay::Picker(PickerOverlay {
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
        }));
        self.request_lineage(session_id);
    }

    fn open_rewind_messages_picker(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        if self.session_is_busy(session_id) {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
            return;
        }
        self.overlay = Some(Overlay::Picker(PickerOverlay {
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
        }));
        self.request_rewind_messages(session_id);
    }

    fn open_provider_picker(&mut self, purpose: ProviderPickerPurpose) {
        self.overlay = Some(Overlay::Picker(PickerOverlay {
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
        }));
        self.request_providers(purpose);
    }

    fn open_session_model_chooser(&mut self) {
        self.overlay = Some(Overlay::SessionModelChooser(SessionModelChooserOverlay {
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
        }));
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
        self.overlay = Some(Overlay::ProviderStudio(overlay));
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
                last_refresh_at: None,
                last_successful_source: None,
                last_error: None,
                entry_count: 0,
                official_entry_count: 0,
                custom_entry_count: 0,
            },
            total: 0,
            offset: 0,
            limit: 50,
            loading: true,
            selected: 0,
            editor: None,
        };
        self.request_model_catalog_page(String::new(), 0);
        self.overlay = Some(Overlay::ModelCatalogStudio(dialog.clone()));
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
            self.overlay_stack
                .push(Overlay::ProviderStudio(dialog.clone()));
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
        self.overlay = Some(Overlay::Picker(PickerOverlay {
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
        }));
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
                self.open_permission_rule_editor(None, "");
            }
            (PickerKind::PermissionRules, PickerValue::PermissionRule(rule)) => {
                self.open_permission_rule_editor(Some(&rule), "");
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
                    self.open_permission_rule_picker(return_query.as_str());
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
            CommandId::Help => self.overlay = Some(Overlay::Help(HelpOverlay::default())),
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
            Some(session_id) => self.request_submit_turn(session_id, draft),
            None => self.create_session(Some(draft)),
        }
    }

    /// `/btw <question>` forks a child session and submits the question
    /// there without touching the parent transcript. The parent turn keeps
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
                        .submit_parts_turn_with_options(session_id, parts, options)
                        .await
                        .map_err(|error| error.to_string());
                    // Reuse the existing turn-submitted message — the
                    // handler will route the new session into the UI if
                    // appropriate, otherwise just refresh the list.
                    let _ = tx.send(AppMessage::SessionTurnSubmitted {
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
        if !execution.pending_permission_requests.is_empty() {
            parts.push(format!(
                "perm={}",
                execution.pending_permission_requests.len()
            ));
        }
        if !execution.pending_user_input_requests.is_empty() {
            parts.push(format!(
                "input={}",
                execution.pending_user_input_requests.len()
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
            let mut parts = session_summary_status_parts(model_part, agent, token_usage);
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
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::SettingsStudio(_) => {}
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
                Overlay::PluginInspector(dialog) => dialog.input.flush_pending_input_if_due(now),
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
                Overlay::Help(_) => {}
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
        if self.composer_mode.is_vim() {
            self.composer_mode = ComposerMode::VimInsert;
        }
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

fn provider_studio_api_key_env_choice_items(_dialog: &ProviderStudioOverlay) -> Vec<ChoiceItem> {
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
        .min_by_key(|entry| match entry.kind {
            agena_api_server::local_api::ModelCatalogEntryKind::Custom => 0,
            agena_api_server::local_api::ModelCatalogEntryKind::Official => 1,
        })
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
        Self::new(I18n::english())
    }
}

impl TranscriptState {
    fn new(i18n: I18n) -> Self {
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
            search_query: String::new(),
            search_match_index: None,
            execution: None,
            last_event_seq: None,
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
        self.execution = None;
        self.last_event_seq = None;
        self.search_query.clear();
        self.search_match_index = None;
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
            AgenaSessionEvent::MessagePartUpdated(update) => {
                self.apply_message_part_updated(update);
                false
            }
            AgenaSessionEvent::MessagePartDelta(delta) => {
                self.apply_message_part_delta(delta).is_err()
            }
            AgenaSessionEvent::AssistantMessageCompleted(completed) => {
                let message = MessageResource {
                    id: completed.message_id.raw(),
                    session_id: self.session_id.unwrap_or_default(),
                    role: MessageRole::Assistant,
                    state: MessageStatus::Completed,
                    created_at: completed.created_at,
                    updated_at: completed.created_at,
                    metadata: completed.metadata.clone(),
                    usage: completed.usage.clone(),
                    finish: Some(completed.finish_reason.to_string()),
                    part_count: completed.parts.len() as u64,
                    parts: Some(completed.parts.clone()),
                };
                self.upsert_message(message);
                self.invalidate_render();
                false
            }
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

    fn apply_message_part_updated(&mut self, update: &agena::event::MessagePartUpdatedEvent) {
        let shell = MessageResource {
            id: update.message_id,
            session_id: update.session_id,
            role: api_message_role(update.message_role),
            state: update.message_state,
            created_at: update.message_created_at,
            updated_at: timestamp_ms_or(update.ts_ms, update.message_created_at),
            metadata: Default::default(),
            usage: None,
            finish: None,
            part_count: 1,
            parts: Some(vec![update.part.clone()]),
        };

        let Some(index) = self.upsert_message(shell) else {
            return;
        };
        let message = &mut self.messages[index];
        message.state = update.message_state;
        message.updated_at = timestamp_ms_or(update.ts_ms, message.updated_at);
        let parts = message.parts.get_or_insert_with(Vec::new);
        if let Some(existing) = parts.iter_mut().find(|part| part.id == update.part.id) {
            *existing = update.part.clone();
        } else {
            parts.push(update.part.clone());
        }
        parts.sort_by_key(|part| part.part_index);
        message.part_count = parts.len() as u64;
        self.invalidate_render();
    }

    fn apply_message_part_delta(
        &mut self,
        delta: &agena::event::MessagePartDeltaEvent,
    ) -> Result<(), ()> {
        let Some(message) = self
            .messages
            .iter_mut()
            .find(|message| message.id == delta.message_id)
        else {
            return Err(());
        };
        let Some(parts) = message.parts.as_mut() else {
            return Err(());
        };
        let Some(part) = parts.iter_mut().find(|part| part.id == delta.part_id) else {
            return Err(());
        };

        if part.status == ExecutionStatus::Pending {
            let _ = part.transition_status(ExecutionStatus::InProgress);
        }
        message.state = MessageStatus::InProgress;
        message.updated_at = timestamp_ms_or(delta.ts_ms, message.updated_at);

        let updated = match &delta.field {
            agena::event::PartDeltaField::Text => part.append_text_delta(delta.delta.as_str()),
            agena::event::PartDeltaField::ReasoningSummary => {
                part.append_reasoning_summary_delta(delta.delta.clone())
            }
            agena::event::PartDeltaField::ReasoningRawContent => {
                part.append_reasoning_raw_delta(delta.delta.clone())
            }
            agena::event::PartDeltaField::CommandStdout
            | agena::event::PartDeltaField::CommandStderr => {
                part.append_command_output_delta(delta.delta.as_str())
            }
            agena::event::PartDeltaField::ToolOutputText => {
                part.append_tool_output_delta(delta.delta.as_str())
            }
            agena::event::PartDeltaField::Custom { .. } => false,
        };
        if !updated {
            return Err(());
        }
        self.invalidate_render();
        Ok(())
    }

    fn upsert_message(&mut self, incoming: MessageResource) -> Option<usize> {
        let message_id = incoming.id;
        if let Some(existing) = self
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
        {
            let merged = merge_message_resources(existing, &incoming);
            *existing = merged;
        } else {
            self.messages.push(incoming);
        }
        self.messages.sort_by_key(message_sort_key);
        self.messages
            .iter()
            .position(|message| message.id == message_id)
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
        let padding = height.saturating_div(3) as usize;
        self.scroll = line.saturating_sub(padding);
        self.follow_tail = false;
        self.clamp_scroll(width, height);
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
        let padding = height.saturating_div(3) as usize;
        self.scroll = line.saturating_sub(padding);
        self.follow_tail = false;
        self.clamp_scroll(width, height);
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
        if self.session_id.is_some() {
            if self.loading_older {
                lines.push(RenderedLine::dim(ui_text::t(
                    &self.i18n,
                    "transcript-loading-older",
                )));
            } else if self.has_more_older {
                lines.push(RenderedLine::dim(ui_text::t(
                    &self.i18n,
                    "transcript-more-older",
                )));
            }
        }

        if self.messages.is_empty() && self.session_id.is_some() && !self.loading_initial {
            lines.push(RenderedLine::dim(ui_text::t(
                &self.i18n,
                "transcript-empty-session",
            )));
        }

        for (index, message) in self.messages.iter().enumerate() {
            if index > 0 {
                lines.push(RenderedLine::plain(""));
            }
            message_line_starts.push((message.id, lines.len()));
            lines.extend(render_message(message, width, &self.i18n));
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
        });
        self.rendered.as_ref().expect("render cache should exist")
    }

    fn invalidate_render(&mut self) {
        self.rendered = None;
    }

    fn clamp_scroll(&mut self, width: u16, height: u16) {
        let max_scroll = self.max_scroll(width, height);
        self.scroll = min(self.scroll, max_scroll);
    }

    fn scroll_to_bottom(&mut self, width: u16, height: u16) {
        self.scroll = self.max_scroll(width, height);
        self.follow_tail = true;
    }

    fn scroll_to_top(&mut self, width: u16, height: u16) {
        self.scroll = 0;
        self.follow_tail = self.is_at_bottom(width, height);
    }

    fn scroll_by_lines(&mut self, width: u16, height: u16, delta: isize) {
        self.follow_tail = false;
        if delta.is_negative() {
            self.scroll = self.scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.scroll = self.scroll.saturating_add(delta as usize);
        }
        self.clamp_scroll(width, height);
        self.follow_tail = self.is_at_bottom(width, height);
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
}

impl RenderedLine {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: Style::default(),
        }
    }

    fn dim(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: Style::default().fg(Color::DarkGray),
        }
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
    merged.parts = if incoming.parts.is_none() && current.parts.is_some() {
        current.parts.clone()
    } else if current_parts_score > incoming_parts_score {
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
    if merged.finish.is_none() {
        merged.finish = current.finish.clone();
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

fn api_message_role(role: agena::role::Role) -> MessageRole {
    match role {
        agena::role::Role::User => MessageRole::User,
        agena::role::Role::Assistant => MessageRole::Assistant,
        agena::role::Role::System => MessageRole::System,
    }
}

fn timestamp_ms_or(timestamp_ms: i64, fallback: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms).unwrap_or(fallback)
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
                let tags = if entry.tags.is_empty() {
                    "untagged".to_string()
                } else {
                    entry
                        .tags
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                lines.push(format!("  - {} [{}]", entry.name, tags));
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
        AgenaSessionEvent::RunStarted(_)
        | AgenaSessionEvent::RunFailed(_)
        | AgenaSessionEvent::StreamError(_)
        | AgenaSessionEvent::PermissionRequested(_)
        | AgenaSessionEvent::PermissionReplied(_)
        | AgenaSessionEvent::PermissionRuleCreated(_)
        | AgenaSessionEvent::PermissionRuleUpdated(_)
        | AgenaSessionEvent::PermissionRuleRevoked(_)
        | AgenaSessionEvent::SessionGoalUpdated(_)
        | AgenaSessionEvent::TurnStarted(_)
        | AgenaSessionEvent::TurnCompleted(_)
        | AgenaSessionEvent::TurnAborted(_)
        | AgenaSessionEvent::ToolCallIssued(_)
        | AgenaSessionEvent::ToolCallCompleted(_)
        | AgenaSessionEvent::PluginEvent(_) => None,
    }
}

fn timeline_event_type_name(record: &DomainEvent) -> &'static str {
    record.kind.tag_str()
}

fn session_workflow_state_label(execution: &SessionExecutionResource) -> &'static str {
    if !execution.pending_permission_requests.is_empty() {
        return "awaiting_permission";
    }
    if !execution.pending_user_input_requests.is_empty() {
        return "awaiting_user_input";
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
        AgenaSessionEvent::RunStarted(event) => format!("session #{}", event.session_id),
        AgenaSessionEvent::RunFailed(event) => {
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
        AgenaSessionEvent::TurnStarted(p) => format!("turn {}", p.turn_id),
        AgenaSessionEvent::TurnCompleted(p) => {
            format!("turn {} ({:?})", p.turn_id, p.finish_reason)
        }
        AgenaSessionEvent::TurnAborted(p) => {
            format!("turn {} aborted ({:?})", p.turn_id, p.reason)
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
        AgenaSessionEvent::RunStarted(event) => vec![format!("session_id: {}", event.session_id)],
        AgenaSessionEvent::RunFailed(event) => vec![
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
        AgenaSessionEvent::TurnStarted(p) => vec![
            format!("turn_id: {}", p.turn_id),
            format!("model: {} / {}", p.provider_id, p.model_id),
        ],
        AgenaSessionEvent::TurnCompleted(p) => vec![
            format!("turn_id: {}", p.turn_id),
            format!("finish: {:?}", p.finish_reason),
        ],
        AgenaSessionEvent::TurnAborted(p) => vec![
            format!("turn_id: {}", p.turn_id),
            format!("reason: {:?}", p.reason),
            format!(
                "message: {}",
                p.message.clone().unwrap_or_else(|| "<none>".to_string())
            ),
        ],
        AgenaSessionEvent::UserMessageAppended(p) => vec![
            format!("message_id: {}", p.message_id),
            format!("turn_id: {}", p.turn_id),
        ],
        AgenaSessionEvent::AssistantMessageCompleted(p) => vec![
            format!("message_id: {}", p.message_id),
            format!("turn_id: {}", p.turn_id),
            format!("finish: {:?}", p.finish_reason),
        ],
        AgenaSessionEvent::ToolCallIssued(p) => vec![
            format!("call_id: {}", p.call_id),
            format!("name: {}", p.name),
            format!("turn_id: {}", p.turn_id),
        ],
        AgenaSessionEvent::ToolCallCompleted(p) => vec![
            format!("call_id: {}", p.call_id),
            format!("turn_id: {}", p.turn_id),
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
    token_usage: Option<(u64, Option<u64>)>,
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
    if let Some(token_progress) = token_usage.and_then(|(current_tokens, limit_tokens)| {
        token_progress_status_part(current_tokens, limit_tokens)
    }) {
        parts.push(token_progress);
    }
    parts
}

fn status_line_token_usage(usage: &SessionUsageResource) -> Option<(u64, Option<u64>)> {
    usage
        .limit_tokens
        .map(|limit_tokens| (usage.current_tokens, Some(limit_tokens)))
}

fn token_progress_status_part(current_tokens: u64, limit_tokens: Option<u64>) -> Option<String> {
    limit_tokens
        .filter(|value| *value > 0)
        .map(|limit_tokens| format_token_progress_label(current_tokens, limit_tokens))
}

fn format_token_progress_label(current_tokens: u64, limit_tokens: u64) -> String {
    if limit_tokens == 0 {
        return "0%".to_string();
    }

    let percent = ((current_tokens as f64 / limit_tokens as f64) * 100.0)
        .clamp(0.0, 100.0)
        .round() as u64;

    format!("{percent}%")
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

#[cfg(test)]
fn parse_user_input_answers(
    i18n: &I18n,
    raw: &str,
    request: &UserInputRequest,
) -> std::result::Result<BTreeMap<String, Vec<String>>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ui_text::user_input_error_empty(i18n));
    }

    let valid_ids = request
        .questions
        .iter()
        .map(|question| question.id.as_str())
        .collect::<HashSet<_>>();
    let mut answers = BTreeMap::new();

    for part in trimmed.split(';') {
        let chunk = part.trim();
        if chunk.is_empty() {
            continue;
        }
        let Some((question_id, values)) = chunk.split_once('=') else {
            return Err(ui_text::user_input_error_invalid_segment(i18n, chunk));
        };
        let question_id = question_id.trim();
        if !valid_ids.contains(question_id) {
            return Err(ui_text::user_input_error_unknown_question(
                i18n,
                question_id,
            ));
        }
        let parsed = values
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if parsed.is_empty() {
            return Err(ui_text::user_input_error_missing_answer(i18n, question_id));
        }
        answers.insert(question_id.to_string(), parsed);
    }

    if answers.is_empty() {
        return Err(ui_text::user_input_error_no_answers(i18n));
    }

    Ok(answers)
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
            max_turn_loops: None,
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
    use crate::backend::{
        ProviderBrowserAuthSessionDraft, ProviderCredentialDraftBundle,
        ProviderDeviceAuthSessionDraft, ProviderDraftAuthDetails,
    };
    use agena::{
        event::{
            CommandContext, CommandEndEvent, MessagePartDeltaEvent, MessagePartUpdatedEvent,
            PartDeltaField,
        },
        message::{
            ExecutionStatus, MessageMetadata, MessageStatus, PartContent, ReasoningPart,
            UserInputOption, UserInputQuestion,
        },
        provider::auth::CredentialIssuer,
    };
    use chrono::Utc;
    use serde_json::json;

    fn provider_studio_test_draft() -> ProviderConfigDraft {
        ProviderConfigDraft {
            source_provider_id: None,
            provider_id: "test".to_string(),
            auth_kind: ProviderDraftAuthKind::Unset,
            auth: ProviderDraftAuthDetails {
                base_url: String::new(),
                instance_url: String::new(),
                api_key_env: String::new(),
                api_key: String::new(),
                credential_issuer: String::new(),
                region: String::new(),
                profile: String::new(),
                access_key_id: String::new(),
                secret_access_key: String::new(),
                session_token: String::new(),
                service_key_env: String::new(),
            },
            credential_drafts: ProviderCredentialDraftBundle::default(),
            default_adapter: String::new(),
            default_model: String::new(),
        }
    }

    fn provider_studio_test_dialog(draft: ProviderConfigDraft) -> ProviderStudioOverlay {
        ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Fields,
            selected_field: 0,
            draft,
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
        }
    }

    #[test]
    fn derive_title_uses_first_non_empty_line() {
        assert_eq!(
            derive_session_title("\n\n  hello world  \nnext"),
            "hello world"
        );
    }

    #[test]
    fn slash_command_context_accepts_bare_slash_and_prefixes() {
        let bare = slash_command_suggestion_context_for_text("/", 1)
            .expect("bare slash should show suggestions");
        assert_eq!(bare.query, "");
        assert_eq!(bare.name_range, 0..1);

        let prefix = slash_command_suggestion_context_for_text("/rew", 4)
            .expect("prefix should show suggestions");
        assert_eq!(prefix.query, "rew");
        assert_eq!(prefix.name_range, 0..4);
    }

    #[test]
    fn slash_command_context_rejects_history_like_and_literal_inputs() {
        assert!(slash_command_suggestion_context_for_text("/ test", 1).is_none());
        assert!(slash_command_suggestion_context_for_text("//literal", 2).is_none());
        assert!(slash_command_suggestion_context_for_text(" /rew", 5).is_none());
    }

    #[test]
    fn format_token_progress_label_uses_percent_for_normal_ranges() {
        assert_eq!(format_token_progress_label(8_500, 10_000), "85%");
        assert_eq!(format_token_progress_label(10_800, 10_000), "100%");
    }

    #[test]
    fn format_token_progress_label_clamps_extreme_overages() {
        assert_eq!(format_token_progress_label(216_383, 19_965), "100%");
    }

    #[test]
    fn session_summary_status_parts_drop_model_and_agent_prefixes() {
        assert_eq!(
            session_summary_status_parts(
                Some("atom/openai/deepseek-v4-flash".to_string()),
                Some("build".to_string()),
                Some((3_889, Some(19_965))),
            ),
            vec![
                "atom/openai/deepseek-v4-flash".to_string(),
                "build".to_string(),
                "19%".to_string(),
            ]
        );
    }

    #[test]
    fn status_line_token_usage_falls_back_to_prompt_threshold() {
        assert_eq!(
            status_line_token_usage(&SessionUsageResource {
                measured_prompt_tokens: Some(1_100),
                current_tokens: 1_200,
                projected_tokens: Some(1_350),
                limit_tokens: Some(2_400),
                limit_basis: Some(agena_api::resource::SessionUsageLimitBasis::PromptThreshold),
                reserved_tokens: None,
                model_context_window_tokens: Some(8_192),
                model_max_input_tokens: None,
                model_max_output_tokens: Some(512),
            }),
            Some((1_200, Some(2_400)))
        );
    }

    #[test]
    fn slash_command_context_only_applies_while_cursor_is_on_name() {
        assert!(slash_command_suggestion_context_for_text("/review focus", 4).is_some());
        assert!(slash_command_suggestion_context_for_text("/review focus", 8).is_none());
        assert!(slash_command_suggestion_context_for_text("/review\nnext", 8).is_none());
    }

    #[test]
    fn file_mention_context_detects_inline_tokens() {
        let text = "open @apps/agena-tui/src/app.rs";
        let context = file_mention_suggestion_context_for_text(text, text.len())
            .expect("mention should parse");
        assert_eq!(context.query, "apps/agena-tui/src/app.rs");
        assert_eq!(context.mention_range, 5..text.len());
    }

    #[test]
    fn file_mention_context_rejects_double_at_and_embedded_at() {
        assert!(file_mention_suggestion_context_for_text("@@literal", 9).is_none());
        assert!(file_mention_suggestion_context_for_text("@a@b", 4).is_none());
    }

    #[test]
    fn parse_user_input_reply_pairs() {
        let request = UserInputRequest {
            request_id: "req".to_string(),
            session_id: Some(1),
            questions: vec![
                UserInputQuestion {
                    id: "lang".to_string(),
                    header: String::new(),
                    question: "language?".to_string(),
                    options: vec![UserInputOption {
                        label: "Rust".to_string(),
                        description: String::new(),
                    }],
                    multiple: false,
                    allow_custom: true,
                },
                UserInputQuestion {
                    id: "libs".to_string(),
                    header: String::new(),
                    question: "libs?".to_string(),
                    options: Vec::new(),
                    multiple: true,
                    allow_custom: true,
                },
            ],
            created_at: Utc::now(),
        };

        let answers = parse_user_input_answers(
            &I18n::english(),
            "lang=Rust; libs=ratatui,crossterm",
            &request,
        )
        .expect("reply should parse");
        assert_eq!(answers["lang"], vec!["Rust"]);
        assert_eq!(answers["libs"], vec!["ratatui", "crossterm"]);
    }

    #[test]
    fn user_input_answer_values_merge_selected_options_and_custom_values() {
        let question = UserInputQuestion {
            id: "libs".to_string(),
            header: String::new(),
            question: "libs?".to_string(),
            options: vec![
                UserInputOption {
                    label: "ratatui".to_string(),
                    description: String::new(),
                },
                UserInputOption {
                    label: "crossterm".to_string(),
                    description: String::new(),
                },
            ],
            multiple: true,
            allow_custom: true,
        };
        let draft = UserInputAnswerDraft {
            option_indexes: [0_usize].into_iter().collect(),
            custom_values: vec!["serde".to_string()],
        };

        assert_eq!(
            user_input_answer_values(&question, &draft),
            vec!["ratatui".to_string(), "serde".to_string()]
        );
    }

    #[test]
    fn user_input_focus_prefers_custom_row_when_custom_answer_exists() {
        let request = UserInputRequest {
            request_id: "req".to_string(),
            session_id: Some(1),
            questions: vec![UserInputQuestion {
                id: "lang".to_string(),
                header: "Lang".to_string(),
                question: "language?".to_string(),
                options: vec![
                    UserInputOption {
                        label: "Rust".to_string(),
                        description: String::new(),
                    },
                    UserInputOption {
                        label: "Go".to_string(),
                        description: String::new(),
                    },
                ],
                multiple: false,
                allow_custom: true,
            }],
            created_at: Utc::now(),
        };
        let mut dialog = App::build_user_input_overlay(1, request);
        dialog.answers.insert(
            "lang".to_string(),
            UserInputAnswerDraft {
                option_indexes: BTreeSet::new(),
                custom_values: vec!["Zig".to_string()],
            },
        );

        App::focus_user_input_question(&mut dialog, 0);
        assert_eq!(dialog.selected_option, 2);
        assert_eq!(dialog.screen, UserInputOverlayScreen::Question);
    }

    #[test]
    fn user_input_tab_moves_to_review_after_last_question() {
        let request = UserInputRequest {
            request_id: "req".to_string(),
            session_id: Some(1),
            questions: vec![
                UserInputQuestion {
                    id: "lang".to_string(),
                    header: "Lang".to_string(),
                    question: "language?".to_string(),
                    options: vec![UserInputOption {
                        label: "Rust".to_string(),
                        description: String::new(),
                    }],
                    multiple: false,
                    allow_custom: false,
                },
                UserInputQuestion {
                    id: "editor".to_string(),
                    header: "Editor".to_string(),
                    question: "editor?".to_string(),
                    options: vec![UserInputOption {
                        label: "Vim".to_string(),
                        description: String::new(),
                    }],
                    multiple: false,
                    allow_custom: false,
                },
            ],
            created_at: Utc::now(),
        };
        let mut dialog = App::build_user_input_overlay(1, request);

        App::move_user_input_tab(&mut dialog, 1);
        assert_eq!(dialog.selected_question, 1);
        assert_eq!(dialog.screen, UserInputOverlayScreen::Question);

        App::move_user_input_tab(&mut dialog, 1);
        assert_eq!(dialog.screen, UserInputOverlayScreen::Review);
    }

    #[test]
    fn parse_pr_command_args_supports_title_and_options() {
        let (title, body, base, head) = parse_pr_command_args(
            "ship feature --body 'details here' --base main --head feature/branch",
        )
        .expect("/pr args should parse");
        assert_eq!(title, "ship feature");
        assert_eq!(body.as_deref(), Some("details here"));
        assert_eq!(base.as_deref(), Some("main"));
        assert_eq!(head.as_deref(), Some("feature/branch"));
    }

    #[test]
    fn parse_pr_command_args_requires_title_before_options() {
        let error = parse_pr_command_args("--base main").expect_err("title should be required");
        assert!(error.to_string().contains("title"));
    }

    #[test]
    fn choice_overlay_rows_include_clear_and_custom_entries() {
        let dialog = ChoiceOverlay {
            title: "Edit test".to_string(),
            prompt: "prompt".to_string(),
            footer: "footer".to_string(),
            empty_message: "empty".to_string(),
            input: Editor::from_text("custom-value".to_string()),
            filter_query: "custom".to_string(),
            all_items: vec![choice_item("preset", "preset option")],
            items: vec![choice_item("preset", "preset option")],
            selected: 0,
            allow_custom: true,
            allow_clear: true,
            action: ChoiceOverlayAction::SettingsField(SETTINGS_FIELDS[0]),
        };

        let rows = App::choice_overlay_rows(&dialog);
        assert!(matches!(rows[0], ChoiceRow::Clear));
        assert!(matches!(rows[1], ChoiceRow::Custom(ref value) if value == "custom-value"));
        assert!(matches!(rows[2], ChoiceRow::Item(_)));
    }

    #[test]
    fn preferred_choice_overlay_selection_prefers_exact_option_then_custom() {
        let option = choice_item("build", "agent");
        let default_agent_field = SETTINGS_FIELDS
            .iter()
            .find(|field| field.path == "default.agent")
            .copied()
            .expect("default.agent field should exist");
        let exact = ChoiceOverlay {
            title: "Edit default.agent".to_string(),
            prompt: "prompt".to_string(),
            footer: "footer".to_string(),
            empty_message: "empty".to_string(),
            input: Editor::from_text("build".to_string()),
            filter_query: String::new(),
            all_items: vec![option.clone()],
            items: vec![option.clone()],
            selected: 0,
            allow_custom: true,
            allow_clear: true,
            action: ChoiceOverlayAction::SettingsField(default_agent_field),
        };
        assert_eq!(App::preferred_choice_overlay_selection(&exact), 2);

        let custom = ChoiceOverlay {
            input: Editor::from_text("bespoke".to_string()),
            ..exact
        };
        assert_eq!(App::preferred_choice_overlay_selection(&custom), 1);
    }

    #[test]
    fn sync_choice_overlay_query_does_not_reset_selection_when_query_is_unchanged() {
        let default_agent_field = SETTINGS_FIELDS
            .iter()
            .find(|field| field.path == "default.agent")
            .copied()
            .expect("default.agent field should exist");
        let mut dialog = ChoiceOverlay {
            title: "Edit".to_string(),
            prompt: "prompt".to_string(),
            footer: "footer".to_string(),
            empty_message: "empty".to_string(),
            input: Editor::from_text("a".to_string()),
            filter_query: "a".to_string(),
            all_items: vec![
                choice_item("default", "default agent"),
                choice_item("plan", "plan agent"),
                choice_item("agent", "generic agent"),
            ],
            items: vec![
                choice_item("default", "default agent"),
                choice_item("plan", "plan agent"),
                choice_item("agent", "generic agent"),
            ],
            selected: 3,
            allow_custom: true,
            allow_clear: true,
            action: ChoiceOverlayAction::SettingsField(default_agent_field),
        };

        App::sync_choice_overlay_query(&mut dialog, true);

        assert_eq!(dialog.selected, 3);
    }

    #[test]
    fn provider_studio_default_model_choice_items_dedupes_and_marks_catalog_matches() {
        let mut dialog = ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Fields,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: Some("openai".to_string()),
                provider_id: "openai".to_string(),
                auth_kind: ProviderDraftAuthKind::Api,
                auth: ProviderDraftAuthDetails {
                    base_url: String::new(),
                    instance_url: String::new(),
                    api_key_env: String::new(),
                    api_key: String::new(),
                    credential_issuer: "openai_chatgpt".to_string(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: "openai".to_string(),
                default_model: String::new(),
            },
            adapter_models: vec![
                ProviderAdapterModelsResource {
                    adapter_id: "openai".to_string(),
                    enabled: true,
                    resolved_base_url: None,
                    models: vec![
                        ProviderModel::new("openai", "gpt-5.4"),
                        ProviderModel::new("openai", "gpt-5.4"),
                    ],
                    error: None,
                },
                ProviderAdapterModelsResource {
                    adapter_id: "responses".to_string(),
                    enabled: true,
                    resolved_base_url: None,
                    models: vec![ProviderModel::new("openai", "gpt-5.5")],
                    error: None,
                },
            ],
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: vec!["openai".to_string(), "responses".to_string()],
            selected_adapter: 0,
            selected_model: 0,
            adapter_selection_touched: false,
            selected_adapter_ids: BTreeSet::new(),
            selected_model_keys: BTreeSet::new(),
            catalog_matches: BTreeMap::from([(
                provider_studio_model_key("openai", "gpt-5.4"),
                ModelCatalogEntryResource {
                    model_id: "gpt-5.4".to_string(),
                    kind: agena_api_server::local_api::ModelCatalogEntryKind::Official,
                    source: agena_api_server::local_api::ModelCatalogSourceKind::Generated,
                    source_label: None,
                    has_local_override: false,
                    display_name: Some("GPT-5.4".to_string()),
                    origin: Some("openai".to_string()),
                    lifecycle: None,
                    context_window_tokens: None,
                    max_input_tokens: None,
                    max_output_tokens: None,
                    description: None,
                    knowledge_cutoff: None,
                    release_date: None,
                    last_updated: None,
                    open_weights: None,
                    default_thinking_mode: None,
                    supports_parallel_tool_calls: None,
                    supports_verbosity: None,
                    default_verbosity: None,
                    default_temperature: None,
                    default_top_p: None,
                    default_top_k: None,
                    assistant_reasoning_interleaved: None,
                    assistant_reasoning_field: None,
                    output_modalities: Vec::new(),
                    pricing: None,
                    thinking_modes: BTreeMap::new(),
                    speed_modes: BTreeMap::new(),
                    capabilities: Default::default(),
                },
            )]),
            listing_adapter_models: false,
            saving: false,
            pending_adapter_models_key: None,
            pending_auth_key: None,
            detail_page: None,
            editor: None,
        };
        dialog.selected_adapter = 0;

        let items = provider_studio_default_model_choice_items(&dialog);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].value, "gpt-5.4");
        assert!(items[0].detail.contains("catalog gpt-5.4"));
        assert_eq!(items[1].value, "gpt-5.5");
    }

    #[test]
    fn provider_draft_auth_mode_legacy_aliases_map_to_credential_issuers() {
        assert_eq!(
            ProviderDraftAuthKind::parse_mode("google_adc", None)
                .expect("google adc alias should parse"),
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc))
        );
        assert_eq!(
            ProviderDraftAuthKind::parse_mode("sap_ai_core", None)
                .expect("sap ai core alias should parse"),
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::SapAiCore))
        );
    }

    #[test]
    fn provider_studio_visible_fields_follow_endpoint_credential_issuer_shape() {
        let base_dialog = || ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Fields,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: None,
                provider_id: "test".to_string(),
                auth_kind: ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc)),
                auth: ProviderDraftAuthDetails {
                    base_url: String::new(),
                    instance_url: String::new(),
                    api_key_env: String::new(),
                    api_key: String::new(),
                    credential_issuer: "google_adc".to_string(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: "openai".to_string(),
                default_model: String::new(),
            },
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

        let google_fields = provider_studio_visible_fields(&base_dialog());
        let google_detail_fields = provider_studio_detail_fields(&base_dialog());
        assert!(google_fields.contains(&ProviderStudioField::CredentialIssuer));
        assert!(google_fields.contains(&ProviderStudioField::EditAuthDetailsAction));
        assert!(!google_fields.contains(&ProviderStudioField::BaseUrl));
        assert!(google_detail_fields.contains(&ProviderStudioField::BaseUrl));
        assert!(!google_detail_fields.contains(&ProviderStudioField::ServiceKeyEnv));

        let mut sap_dialog = base_dialog();
        sap_dialog.draft.auth_kind =
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::SapAiCore));
        sap_dialog.draft.auth.credential_issuer = "sap_ai_core".to_string();
        let sap_fields = provider_studio_visible_fields(&sap_dialog);
        let sap_detail_fields = provider_studio_detail_fields(&sap_dialog);
        assert!(sap_fields.contains(&ProviderStudioField::EditAuthDetailsAction));
        assert!(!sap_fields.contains(&ProviderStudioField::BaseUrl));
        assert!(sap_detail_fields.contains(&ProviderStudioField::BaseUrl));
        assert!(sap_detail_fields.contains(&ProviderStudioField::ServiceKeyEnv));
        assert!(!sap_detail_fields.contains(&ProviderStudioField::ApiKey));
        assert!(!sap_detail_fields.contains(&ProviderStudioField::ApiKeyEnv));
    }

    #[test]
    fn provider_studio_visible_fields_follow_oauth_credential_issuer_shape() {
        let dialog_for = |issuer: CredentialIssuer, issuer_label: &str| {
            let mut draft = provider_studio_test_draft();
            draft.provider_id = "oauth".to_string();
            draft.auth_kind = ProviderDraftAuthKind::Credential(Some(issuer));
            draft.auth.credential_issuer = issuer_label.to_string();
            draft.normalize_shape();
            provider_studio_test_dialog(draft)
        };

        let openai_fields = provider_studio_visible_fields(&dialog_for(
            CredentialIssuer::OpenaiChatgpt,
            "openai_chatgpt",
        ));
        let openai_detail_fields = provider_studio_detail_fields(&dialog_for(
            CredentialIssuer::OpenaiChatgpt,
            "openai_chatgpt",
        ));
        assert!(openai_fields.contains(&ProviderStudioField::StartAuthAction));
        assert!(openai_fields.contains(&ProviderStudioField::ContinueAuthAction));
        assert!(openai_fields.contains(&ProviderStudioField::EditAuthDetailsAction));
        assert!(!openai_fields.contains(&ProviderStudioField::RedirectUri));
        assert!(openai_detail_fields.contains(&ProviderStudioField::RedirectUri));
        assert!(openai_detail_fields.contains(&ProviderStudioField::CallbackUrl));
        assert!(openai_detail_fields.contains(&ProviderStudioField::AccountId));
        assert!(!openai_detail_fields.contains(&ProviderStudioField::EnterpriseDomain));

        let copilot_fields = provider_studio_visible_fields(&dialog_for(
            CredentialIssuer::GithubCopilot,
            "github_copilot",
        ));
        let copilot_detail_fields = provider_studio_detail_fields(&dialog_for(
            CredentialIssuer::GithubCopilot,
            "github_copilot",
        ));
        assert!(copilot_fields.contains(&ProviderStudioField::StartAuthAction));
        assert!(copilot_fields.contains(&ProviderStudioField::ContinueAuthAction));
        assert!(!copilot_fields.contains(&ProviderStudioField::EnterpriseDomain));
        assert!(copilot_detail_fields.contains(&ProviderStudioField::EnterpriseDomain));
        assert!(copilot_detail_fields.contains(&ProviderStudioField::RefreshToken));
        assert!(!copilot_detail_fields.contains(&ProviderStudioField::CallbackUrl));

        let gitlab_dialog = dialog_for(CredentialIssuer::Gitlab, "gitlab");
        let gitlab_fields = provider_studio_visible_fields(&gitlab_dialog);
        let gitlab_detail_fields = provider_studio_detail_fields(&gitlab_dialog);
        assert!(gitlab_fields.contains(&ProviderStudioField::StartAuthAction));
        assert!(!gitlab_fields.contains(&ProviderStudioField::InstanceUrl));
        assert!(gitlab_detail_fields.contains(&ProviderStudioField::InstanceUrl));
        assert!(gitlab_detail_fields.contains(&ProviderStudioField::RedirectUri));
        assert!(gitlab_detail_fields.contains(&ProviderStudioField::CallbackUrl));

        let atomgit_dialog = dialog_for(CredentialIssuer::AtomGit, "atomgit");
        let atomgit_fields = provider_studio_visible_fields(&atomgit_dialog);
        let atomgit_detail_fields = provider_studio_detail_fields(&atomgit_dialog);
        assert!(atomgit_fields.contains(&ProviderStudioField::StartAuthAction));
        assert!(!atomgit_fields.contains(&ProviderStudioField::Username));
        assert!(atomgit_detail_fields.contains(&ProviderStudioField::Username));
        assert!(atomgit_detail_fields.contains(&ProviderStudioField::DisplayName));
        assert!(atomgit_detail_fields.contains(&ProviderStudioField::Email));
        assert!(atomgit_detail_fields.contains(&ProviderStudioField::AvatarUrl));
    }

    #[test]
    fn provider_studio_main_action_rows_show_contextual_auth_summaries() {
        let mut openai_draft = provider_studio_test_draft();
        openai_draft.provider_id = "oauth".to_string();
        openai_draft.auth_kind =
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt));
        openai_draft.auth.credential_issuer = "openai_chatgpt".to_string();
        openai_draft.normalize_shape();

        let empty_dialog = provider_studio_test_dialog(openai_draft.clone());
        let empty_status = provider_studio_auth_status_summary(&empty_dialog);
        assert_eq!(
            provider_studio_main_field_value(&empty_dialog, ProviderStudioField::StartAuthAction),
            empty_status
        );
        assert_eq!(
            provider_studio_main_field_value(
                &empty_dialog,
                ProviderStudioField::ContinueAuthAction
            ),
            empty_status
        );
        let empty_details = provider_studio_main_field_value(
            &empty_dialog,
            ProviderStudioField::EditAuthDetailsAction,
        );
        assert!(empty_details.contains(empty_status));
        assert!(!empty_details.contains('['));

        let mut pending_openai = openai_draft.clone();
        pending_openai.credential_drafts.openai_chatgpt.redirect_uri =
            "http://localhost:1455/callback".to_string();
        pending_openai.credential_drafts.openai_chatgpt.browser =
            Some(ProviderBrowserAuthSessionDraft {
                authorize_url: "https://auth.example.com/authorize?client_id=abc".to_string(),
                state: "state-1234567890".to_string(),
                pkce_verifier: "pkce".to_string(),
            });
        let pending_openai_dialog = provider_studio_test_dialog(pending_openai);
        let start_value = provider_studio_main_field_value(
            &pending_openai_dialog,
            ProviderStudioField::StartAuthAction,
        );
        let continue_value = provider_studio_main_field_value(
            &pending_openai_dialog,
            ProviderStudioField::ContinueAuthAction,
        );
        let details_value = provider_studio_main_field_value(
            &pending_openai_dialog,
            ProviderStudioField::EditAuthDetailsAction,
        );
        assert!(start_value.contains("https://auth.example.com/authorize"));
        assert!(continue_value.contains("paste callback_url"));
        assert!(continue_value.contains("state-1234567890"));
        assert!(details_value.contains("pending"));
        assert!(!start_value.contains('['));
        assert!(!continue_value.contains('['));
        assert!(!details_value.contains('['));

        let mut copilot_draft = provider_studio_test_draft();
        copilot_draft.provider_id = "copilot".to_string();
        copilot_draft.auth_kind =
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GithubCopilot));
        copilot_draft.auth.credential_issuer = "github_copilot".to_string();
        copilot_draft.normalize_shape();
        copilot_draft.credential_drafts.github_copilot.device =
            Some(ProviderDeviceAuthSessionDraft {
                verification_url: "https://github.com/login/device".to_string(),
                user_code: "ABCD-EFGH".to_string(),
                device_code: "device-code".to_string(),
                interval_seconds: 5,
            });
        let copilot_dialog = provider_studio_test_dialog(copilot_draft);
        assert!(
            provider_studio_main_field_value(&copilot_dialog, ProviderStudioField::StartAuthAction)
                .contains("https://github.com/login/device")
        );
        let copilot_continue = provider_studio_main_field_value(
            &copilot_dialog,
            ProviderStudioField::ContinueAuthAction,
        );
        assert!(copilot_continue.contains("poll every 5s"));
        assert!(copilot_continue.contains("ABCD-EFGH"));
    }

    #[test]
    fn provider_studio_api_fields_show_base_url_before_adapter_selection() {
        let dialog = ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Fields,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: None,
                provider_id: "shared_gateway".to_string(),
                auth_kind: ProviderDraftAuthKind::Api,
                auth: ProviderDraftAuthDetails {
                    base_url: String::new(),
                    instance_url: String::new(),
                    api_key_env: "OPENAI_API_KEY".to_string(),
                    api_key: String::new(),
                    credential_issuer: "openai_chatgpt".to_string(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: String::new(),
                default_model: String::new(),
            },
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: vec![
                "anthropic".to_string(),
                "gemini".to_string(),
                "openai".to_string(),
            ],
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
        let fields = provider_studio_visible_fields(&dialog);
        let detail_fields = provider_studio_detail_fields(&dialog);
        assert!(fields.contains(&ProviderStudioField::EditAuthDetailsAction));
        assert!(!fields.contains(&ProviderStudioField::BaseUrl));
        assert!(detail_fields.contains(&ProviderStudioField::BaseUrl));
        assert!(detail_fields.contains(&ProviderStudioField::ApiKeyEnv));
    }

    #[test]
    fn provider_studio_gitlab_api_fields_hide_base_url() {
        let mut dialog = ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Fields,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: None,
                provider_id: "gitlab_token".to_string(),
                auth_kind: ProviderDraftAuthKind::Gitlab,
                auth: ProviderDraftAuthDetails {
                    base_url: String::new(),
                    instance_url: String::new(),
                    api_key_env: "GITLAB_TOKEN".to_string(),
                    api_key: String::new(),
                    credential_issuer: "openai_chatgpt".to_string(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: "openai".to_string(),
                default_model: "duo-chat-gpt-5-2".to_string(),
            },
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: vec!["anthropic".to_string(), "openai".to_string()],
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
        dialog.draft.normalize_shape();

        let fields = provider_studio_visible_fields(&dialog);
        let detail_fields = provider_studio_detail_fields(&dialog);
        assert!(!fields.contains(&ProviderStudioField::BaseUrl));
        assert!(!fields.contains(&ProviderStudioField::ApiKeyEnv));
        assert!(!detail_fields.contains(&ProviderStudioField::BaseUrl));
        assert!(detail_fields.contains(&ProviderStudioField::InstanceUrl));
        assert!(detail_fields.contains(&ProviderStudioField::ApiKeyEnv));
    }

    #[test]
    fn provider_studio_request_adapter_ids_require_explicit_selection() {
        let make_dialog = || ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Fields,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: None,
                provider_id: "shared_gateway".to_string(),
                auth_kind: ProviderDraftAuthKind::Api,
                auth: ProviderDraftAuthDetails {
                    base_url: "https://opencode.ai/zen".to_string(),
                    instance_url: String::new(),
                    api_key_env: "OPENCODE_API_KEY".to_string(),
                    api_key: String::new(),
                    credential_issuer: String::new(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: "anthropic".to_string(),
                default_model: "claude-sonnet-4-5".to_string(),
            },
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: vec!["openai".to_string(), "anthropic".to_string()],
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

        let mut default_dialog = make_dialog();
        assert!(provider_studio_request_adapter_ids(&default_dialog).is_empty());

        default_dialog.focus = ProviderStudioFocus::Adapters;
        assert!(provider_studio_request_adapter_ids(&default_dialog).is_empty());

        default_dialog.selected_adapter_ids = BTreeSet::from(["anthropic".to_string()]);
        assert_eq!(
            provider_studio_request_adapter_ids(&default_dialog),
            vec!["anthropic".to_string()]
        );

        default_dialog.selected_adapter_ids =
            BTreeSet::from(["anthropic".to_string(), "gitlab".to_string()]);
        assert_eq!(
            provider_studio_request_adapter_ids(&default_dialog),
            vec!["anthropic".to_string()]
        );
    }

    #[test]
    fn provider_studio_adapter_multiselect_supports_select_all_and_clear() {
        let mut dialog = ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Adapters,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: None,
                provider_id: "shared_gateway".to_string(),
                auth_kind: ProviderDraftAuthKind::Api,
                auth: ProviderDraftAuthDetails {
                    base_url: "https://opencode.ai/zen".to_string(),
                    instance_url: String::new(),
                    api_key_env: String::new(),
                    api_key: String::new(),
                    credential_issuer: String::new(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: "openai".to_string(),
                default_model: "gpt-5.5".to_string(),
            },
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: vec![
                "openai".to_string(),
                "anthropic".to_string(),
                "gemini".to_string(),
            ],
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

        App::select_all_provider_studio_adapters(&mut dialog);
        assert_eq!(
            dialog.selected_adapter_ids,
            BTreeSet::from([
                "anthropic".to_string(),
                "gemini".to_string(),
                "openai".to_string(),
            ])
        );

        App::clear_provider_studio_selected_adapters(&mut dialog);
        assert!(dialog.selected_adapter_ids.is_empty());
    }

    #[test]
    fn provider_studio_model_selection_defaults_to_all_loaded_models() {
        let mut dialog = ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Models,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: None,
                provider_id: "shared_gateway".to_string(),
                auth_kind: ProviderDraftAuthKind::Api,
                auth: ProviderDraftAuthDetails {
                    base_url: "https://opencode.ai/zen".to_string(),
                    instance_url: String::new(),
                    api_key_env: String::new(),
                    api_key: String::new(),
                    credential_issuer: String::new(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: String::new(),
                default_model: String::new(),
            },
            adapter_models: vec![ProviderAdapterModelsResource {
                adapter_id: "openai".to_string(),
                enabled: true,
                resolved_base_url: Some("https://opencode.ai/zen/v1".to_string()),
                models: vec![
                    ProviderModel::new("openai", "gpt-5.5"),
                    ProviderModel::new("openai", "gpt-5.4-mini"),
                ],
                error: None,
            }],
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: vec!["openai".to_string()],
            selected_adapter: 0,
            selected_model: 0,
            adapter_selection_touched: false,
            selected_adapter_ids: BTreeSet::from(["openai".to_string()]),
            selected_model_keys: BTreeSet::new(),
            catalog_matches: BTreeMap::new(),
            listing_adapter_models: false,
            saving: false,
            pending_adapter_models_key: None,
            pending_auth_key: None,
            detail_page: None,
            editor: None,
        };

        provider_studio_restore_model_selection(&mut dialog);
        provider_studio_ensure_default_selection(&mut dialog);

        assert_eq!(
            dialog.selected_model_keys,
            BTreeSet::from([
                provider_studio_model_key("openai", "gpt-5.4-mini"),
                provider_studio_model_key("openai", "gpt-5.5"),
            ])
        );
        assert_eq!(dialog.draft.default_adapter, "openai");
        assert_eq!(dialog.draft.default_model, "gpt-5.5");
    }

    #[test]
    fn provider_studio_selected_adapter_models_for_save_filters_to_checked_models() {
        let dialog = ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Models,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: None,
                provider_id: "shared_gateway".to_string(),
                auth_kind: ProviderDraftAuthKind::Api,
                auth: ProviderDraftAuthDetails {
                    base_url: "https://opencode.ai/zen".to_string(),
                    instance_url: String::new(),
                    api_key_env: String::new(),
                    api_key: String::new(),
                    credential_issuer: String::new(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: "openai".to_string(),
                default_model: "gpt-5.5".to_string(),
            },
            adapter_models: vec![ProviderAdapterModelsResource {
                adapter_id: "openai".to_string(),
                enabled: true,
                resolved_base_url: Some("https://opencode.ai/zen/v1".to_string()),
                models: vec![
                    ProviderModel::new("openai", "gpt-5.5"),
                    ProviderModel::new("openai", "gpt-5.4-mini"),
                ],
                error: None,
            }],
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: vec!["openai".to_string()],
            selected_adapter: 0,
            selected_model: 0,
            adapter_selection_touched: false,
            selected_adapter_ids: BTreeSet::from(["openai".to_string()]),
            selected_model_keys: BTreeSet::from([provider_studio_model_key(
                "openai",
                "gpt-5.4-mini",
            )]),
            catalog_matches: BTreeMap::new(),
            listing_adapter_models: false,
            saving: false,
            pending_adapter_models_key: None,
            pending_auth_key: None,
            detail_page: None,
            editor: None,
        };

        let selected =
            provider_studio_selected_adapter_models_for_save(&dialog).expect("selected adapter");
        assert_eq!(selected.adapter_id, "openai");
        assert_eq!(
            selected
                .models
                .into_iter()
                .map(|model| model.id.to_string())
                .collect::<Vec<_>>(),
            vec!["gpt-5.4-mini".to_string()]
        );
    }

    #[test]
    fn provider_studio_select_all_skips_incompatible_legacy_adapters() {
        let mut dialog = ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Adapters,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: Some("legacy".to_string()),
                provider_id: "legacy".to_string(),
                auth_kind: ProviderDraftAuthKind::Api,
                auth: ProviderDraftAuthDetails {
                    base_url: "https://opencode.ai/zen".to_string(),
                    instance_url: String::new(),
                    api_key_env: String::new(),
                    api_key: String::new(),
                    credential_issuer: String::new(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: "openai".to_string(),
                default_model: "gpt-5.5".to_string(),
            },
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::from(["gitlab".to_string()]),
            adapter_candidate_ids: vec![
                "openai".to_string(),
                "anthropic".to_string(),
                "gemini".to_string(),
                "gitlab".to_string(),
            ],
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

        App::select_all_provider_studio_adapters(&mut dialog);
        assert_eq!(
            dialog.selected_adapter_ids,
            BTreeSet::from([
                "anthropic".to_string(),
                "gemini".to_string(),
                "openai".to_string(),
            ])
        );
    }

    #[test]
    fn provider_studio_restore_adapter_selection_preserves_explicit_checks() {
        let mut dialog = ProviderStudioOverlay {
            title: "Providers".to_string(),
            footer: "footer".to_string(),
            show_provider_list: false,
            providers: Vec::new(),
            selected_provider: 0,
            focus: ProviderStudioFocus::Adapters,
            selected_field: 0,
            draft: ProviderConfigDraft {
                source_provider_id: Some("oc".to_string()),
                provider_id: "oc".to_string(),
                auth_kind: ProviderDraftAuthKind::Api,
                auth: ProviderDraftAuthDetails {
                    base_url: "https://opencode.ai/zen".to_string(),
                    instance_url: String::new(),
                    api_key_env: "OPENCODE_API_KEY".to_string(),
                    api_key: String::new(),
                    credential_issuer: String::new(),
                    region: String::new(),
                    profile: String::new(),
                    access_key_id: String::new(),
                    secret_access_key: String::new(),
                    session_token: String::new(),
                    service_key_env: String::new(),
                },
                credential_drafts: ProviderCredentialDraftBundle::default(),
                default_adapter: "openai".to_string(),
                default_model: "gpt-5.5".to_string(),
            },
            adapter_models: Vec::new(),
            configured_adapter_ids: BTreeSet::new(),
            adapter_candidate_ids: vec![
                "openai".to_string(),
                "anthropic".to_string(),
                "gemini".to_string(),
            ],
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

        restore_provider_studio_adapter_selection(
            &mut dialog,
            &BTreeSet::from([
                "anthropic".to_string(),
                "openai".to_string(),
                "responses".to_string(),
            ]),
            Some("anthropic"),
        );

        assert_eq!(
            dialog.selected_adapter_ids,
            BTreeSet::from(["anthropic".to_string(), "openai".to_string()])
        );
        assert_eq!(dialog.selected_adapter, 1);
    }

    #[test]
    fn provider_studio_request_key_changes_when_auth_inputs_change() {
        let mut draft = provider_studio_test_draft();
        draft.provider_id = "oc".to_string();
        draft.auth_kind = ProviderDraftAuthKind::Credential(Some(CredentialIssuer::OpenaiChatgpt));
        draft.auth.credential_issuer = "openai_chatgpt".to_string();
        draft.default_adapter = "openai".to_string();
        draft.default_model = "gpt-5.5".to_string();
        draft.credential_drafts.openai_chatgpt.callback_url =
            "http://127.0.0.1:1455/callback?code=alpha".to_string();
        draft.normalize_shape();
        let adapter_ids = vec!["anthropic".to_string(), "openai".to_string()];
        let baseline = provider_studio_request_key(&draft, &adapter_ids);

        draft.credential_drafts.openai_chatgpt.callback_url =
            "http://127.0.0.1:1455/callback?code=beta".to_string();
        let callback_changed = provider_studio_request_key(&draft, &adapter_ids);
        assert_ne!(baseline, callback_changed);

        draft.credential_drafts.openai_chatgpt.callback_url =
            "http://127.0.0.1:1455/callback?code=alpha".to_string();
        draft.credential_drafts.openai_chatgpt.tokens.access_token = "token".to_string();
        let token_changed = provider_studio_request_key(&draft, &adapter_ids);
        assert_ne!(baseline, token_changed);
    }

    #[test]
    fn provider_studio_candidate_adapter_ids_follow_auth_contract() {
        let mut draft = provider_studio_test_draft();
        draft.provider_id = "vertex".to_string();
        draft.auth_kind = ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc));
        draft.auth.credential_issuer = "google_adc".to_string();
        draft.default_adapter = "openai".to_string();
        draft.normalize_shape();

        assert_eq!(
            provider_studio_candidate_adapter_ids(&draft, BTreeSet::new()),
            vec!["openai".to_string()]
        );

        let legacy =
            provider_studio_candidate_adapter_ids(&draft, BTreeSet::from(["gitlab".to_string()]));
        assert_eq!(legacy, vec!["openai".to_string(), "gitlab".to_string()]);
    }

    #[test]
    fn provider_studio_live_model_listing_uses_saved_atomgit_gateway() {
        let mut atomgit_draft = provider_studio_test_draft();
        atomgit_draft.provider_id = "atomgit".to_string();
        atomgit_draft.auth_kind =
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::AtomGit));
        atomgit_draft.auth.credential_issuer = "atomgit".to_string();
        atomgit_draft.default_adapter = "openai".to_string();
        atomgit_draft.normalize_shape();

        let mut atomgit_dialog = provider_studio_test_dialog(atomgit_draft);
        atomgit_dialog.selected_adapter_ids = BTreeSet::from(["openai".to_string()]);
        assert!(provider_studio_can_request_adapter_models(&atomgit_dialog));

        atomgit_dialog.draft.source_provider_id = Some("atomgit".to_string());
        assert!(provider_studio_can_request_adapter_models(&atomgit_dialog));

        let mut vertex_draft = provider_studio_test_draft();
        vertex_draft.source_provider_id = Some("vertex".to_string());
        vertex_draft.provider_id = "vertex".to_string();
        vertex_draft.auth_kind =
            ProviderDraftAuthKind::Credential(Some(CredentialIssuer::GoogleAdc));
        vertex_draft.auth.credential_issuer = "google_adc".to_string();
        vertex_draft.auth.base_url = "https://example.com".to_string();
        vertex_draft.default_adapter = "openai".to_string();
        vertex_draft.normalize_shape();

        let mut vertex_dialog = provider_studio_test_dialog(vertex_draft);
        vertex_dialog.selected_adapter_ids = BTreeSet::from(["openai".to_string()]);
        assert!(provider_studio_can_request_adapter_models(&vertex_dialog));
    }

    #[test]
    fn settings_studio_provider_items_only_include_new_and_real_providers() {
        let items = settings_studio_provider_items(&[
            ProviderSummaryResource {
                provider_id: "oc".to_string(),
                default_adapter: Some("openai".to_string()),
                default_model: "gpt-5.5".to_string(),
                adapters: Vec::new(),
            },
            ProviderSummaryResource {
                provider_id: "copilot".to_string(),
                default_adapter: Some("anthropic".to_string()),
                default_model: "claude-sonnet-4-5".to_string(),
                adapters: Vec::new(),
            },
        ]);

        assert_eq!(items[0].label, "+ New provider");
        assert!(!items.iter().any(|item| item.label == "Manage Providers"));
        assert_eq!(items[1].label, "oc");
        assert_eq!(items[2].label, "copilot");
    }

    #[test]
    fn parse_permission_rule_input_supports_tool_rules() {
        let draft = parse_permission_rule_input(
            "tool bash allow qualifier='npm test' scope=session session=42",
        )
        .expect("permission rule input should parse");
        assert_eq!(draft.subject_kind, PermissionRuleSubjectKind::Tool);
        assert_eq!(draft.tool_name, "bash");
        assert_eq!(draft.qualifier, "npm test");
        assert_eq!(draft.scope, "session");
        assert_eq!(draft.session_id, "42");
        assert_eq!(draft.mode, PermissionMode::Allow);
    }

    #[test]
    fn parse_permission_rule_input_supports_path_rules() {
        let draft = parse_permission_rule_input(
            "path read_write src allow scope=workspace workspace_root=/tmp/ws",
        )
        .expect("path permission rule input should parse");
        assert_eq!(draft.subject_kind, PermissionRuleSubjectKind::PathAccess);
        assert_eq!(draft.path_access_kind, "read_write");
        assert_eq!(draft.target_path, "src");
        assert_eq!(draft.workspace_root, "/tmp/ws");
        assert_eq!(draft.mode, PermissionMode::Allow);
    }

    #[test]
    fn parse_permission_rule_input_supports_network_rules() {
        let draft = parse_permission_rule_input("network api.example.com:443 ask scope=global")
            .expect("network permission rule input should parse");
        assert_eq!(draft.subject_kind, PermissionRuleSubjectKind::NetworkAccess);
        assert_eq!(draft.network_target, "api.example.com:443");
        assert_eq!(draft.scope, "global");
        assert_eq!(draft.mode, PermissionMode::Ask);

        let params = permission_rule_params_from_draft(&draft);
        assert_eq!(params.subject_kind.as_deref(), Some("network_access"));
        assert_eq!(
            params.network_target.as_deref(),
            Some("api.example.com:443")
        );
    }

    #[test]
    fn parse_permission_rule_input_supports_global_scope() {
        let draft = parse_permission_rule_input("tool bash allow scope=global")
            .expect("global permission rule input should parse");
        assert_eq!(draft.subject_kind, PermissionRuleSubjectKind::Tool);
        assert_eq!(draft.tool_name, "bash");
        assert_eq!(draft.scope, "global");
        assert_eq!(draft.session_id, "");
        assert_eq!(draft.mode, PermissionMode::Allow);
    }

    #[test]
    fn parse_permission_rule_input_rejects_invalid_mode() {
        let error = parse_permission_rule_input("tool git maybe scope=workspace")
            .expect_err("invalid mode should fail");
        assert!(error.contains("allow, ask, or deny"));
    }

    #[test]
    fn permission_overlay_maps_allow_session_and_deny_choices() {
        assert_eq!(
            permission_overlay_choice(0).kind,
            PermissionReplyKind::AllowOnce
        );
        assert_eq!(
            permission_overlay_choice(1).kind,
            PermissionReplyKind::AllowAlways
        );
        assert_eq!(
            permission_overlay_choice(1).scope,
            Some(PermissionScope::Session)
        );
        assert_eq!(
            permission_overlay_choice(2).scope,
            Some(PermissionScope::Workspace)
        );
        assert_eq!(
            permission_overlay_choice(3).scope,
            Some(PermissionScope::Global)
        );
        assert_eq!(
            permission_overlay_choice(4).kind,
            PermissionReplyKind::DenyOnce
        );
        assert_eq!(
            permission_overlay_choice(5).kind,
            PermissionReplyKind::DenyAlways
        );
        assert_eq!(
            permission_overlay_choice(6).scope,
            Some(PermissionScope::Workspace)
        );
        assert_eq!(
            permission_overlay_choice(9).scope,
            Some(PermissionScope::Global)
        );
    }

    #[test]
    fn tool_output_preview_collapses_large_outputs() {
        let text = (0..TOOL_CARD_PREVIEW_LINES + 3)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");

        let preview = tool_output_preview(text.as_str());

        assert_eq!(preview.text.lines().count(), TOOL_CARD_PREVIEW_LINES);
        assert_eq!(preview.omitted_lines, 3);
    }

    #[test]
    fn render_message_uses_structured_card_layout() {
        let now = Utc::now();
        let lines = render_message(
            &MessageResource {
                id: 10,
                session_id: 42,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: MessageMetadata::default(),
                usage: None,
                finish: None,
                part_count: 1,
                parts: Some(vec![MessagePart::with_content(
                    11,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text("alpha beta gamma delta epsilon"),
                )]),
            },
            22,
            &I18n::english(),
        );

        assert_eq!(lines[0].text, "assistant");
        assert!(lines.iter().skip(1).all(|line| line.text.starts_with("  ")));
        assert!(lines.iter().any(|line| line.text.contains("alpha beta")));
    }

    #[test]
    fn assistant_message_text_joins_loaded_text_parts() {
        let now = Utc::now();
        let message = MessageResource {
            id: 10,
            session_id: 42,
            role: MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
            part_count: 3,
            parts: Some(vec![
                MessagePart::with_content(
                    11,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text(" first answer "),
                ),
                MessagePart::with_content(
                    12,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::reasoning_summary("hidden reasoning"),
                ),
                MessagePart::with_content(
                    13,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text("\nsecond answer\n"),
                ),
            ]),
        };

        assert_eq!(
            assistant_message_text(&message).as_deref(),
            Some("first answer\n\nsecond answer")
        );
    }
    #[test]
    fn rewind_message_preview_prefers_first_text_line() {
        let now = Utc::now();
        let preview = rewind_message_preview(
            &MessageResource {
                id: 10,
                session_id: 42,
                role: MessageRole::Assistant,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: MessageMetadata::default(),
                usage: None,
                finish: None,
                part_count: 1,
                parts: Some(vec![MessagePart::with_content(
                    11,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text("\n\n  first line\nsecond line"),
                )]),
            },
            &I18n::english(),
        );

        assert_eq!(preview, "first line");
    }

    #[test]
    fn plugin_inspector_item_includes_manifest_capabilities_and_logs() {
        let status = agena::plugin::status::PluginStatus {
            plugin_id: "ops.plugin".to_string(),
            kind: "stdio",
            state: agena::plugin::status::PluginRunState::Failed,
            pid: None,
            restart_count: 2,
            last_exit_code: Some(7),
            last_restart_at_ms: Some(1_700_000_000_000),
            last_error: Some("permission denied".to_string()),
        };
        let manifest = agena::plugin::PluginManifest::builder("ops-plugin", "1.2.3")
            .description("Operator surface")
            .author("Agena")
            .hooks(agena::plugin::HookSubscription::EVENT)
            .tool(
                agena::plugin::PluginToolDecl::new("inspect", json!({"type": "object"}))
                    .tag(agena::plugin::sdk::ToolTag::Task)
                    .host_capabilities([
                        agena::plugin::sdk::HostCapability::PluginStatus,
                        agena::plugin::sdk::HostCapability::ReadConfig,
                    ]),
            )
            .tool(
                agena::plugin::PluginToolDecl::new("logs", json!({"type": "object"}))
                    .host_capability(agena::plugin::sdk::HostCapability::PluginStatus),
            )
            .build();
        let logs = vec![agena::plugin::PluginLogEntry {
            seq: 9,
            timestamp_ms: 1_700_000_000_123,
            plugin_id: status.plugin_id.clone(),
            level: "warn".to_string(),
            source: "stderr".to_string(),
            message: "request failed".to_string(),
            fields: json!({"attempt": 2}),
        }];

        let item = build_plugin_inspector_item(
            status.clone(),
            Some(agena::plugin::PluginInspect {
                status,
                manifest: Some(manifest),
                authority: None,
            }),
            logs,
        );

        assert!(item.summary.contains("ops-plugin@1.2.3"));
        assert!(item.detail.contains("manifest: ops-plugin@1.2.3"));
        assert!(item.detail.contains("last_error: permission denied"));
        assert!(
            item.detail
                .contains("capabilities: PluginStatus, ReadConfig")
        );
        assert!(item.logs.contains("stderr request failed"));
        assert!(item.logs.contains("{\"attempt\":2}"));
        assert!(item.search_text.contains("permission denied"));
        assert!(item.search_text.contains("request failed"));
        assert!(item.copy_text.contains("Recent logs"));
        assert_eq!(item.state, agena::plugin::status::PluginRunState::Failed);
    }

    #[test]
    fn plugin_inspector_logs_empty_state_is_readable() {
        assert_eq!(format_plugin_inspector_logs(&[]), "No retained logs");
    }

    #[test]
    fn parse_settings_field_input_supports_clear_bool_and_integer_values() {
        let bool_field = SettingsFieldSpec {
            path: "runtime.reload.enabled",
            description: "runtime config reloader enabled",
            kind: SettingsFieldKind::Bool,
        };
        let int_field = SettingsFieldSpec {
            path: "runtime.request_retry.max_retries",
            description: "provider request retry count",
            kind: SettingsFieldKind::Integer,
        };

        assert_eq!(
            parse_settings_field_input(bool_field, "on").expect("bool should parse"),
            Some(JsonValue::Bool(true))
        );
        assert_eq!(
            parse_settings_field_input(int_field, "42").expect("integer should parse"),
            Some(JsonValue::from(42_u64))
        );
        assert_eq!(
            parse_settings_field_input(int_field, "clear").expect("clear should parse"),
            None
        );
    }

    #[test]
    fn settings_studio_general_items_show_configured_and_effective_values() {
        let sources = ConfigJsonSources {
            config_path: PathBuf::from("/tmp/agena.json"),
            config_found: true,
            file: serde_json::json!({
                "default": {
                    "provider": "openai",
                }
            }),
            effective: serde_json::json!({
                "default": {
                    "provider": "openai",
                    "agent": "build",
                }
            }),
        };

        let items = settings_studio_general_items(&sources);
        let provider = items
            .iter()
            .find(|item| item.label == "default.provider")
            .expect("default provider item should exist");
        let agent = items
            .iter()
            .find(|item| item.label == "default.agent")
            .expect("default agent item should exist");

        assert!(provider.detail.contains("configured"));
        assert!(provider.detail.contains("\"openai\""));
        assert!(agent.detail.contains("effective"));
        assert!(agent.detail.contains("\"build\""));
    }

    #[test]
    fn settings_studio_runtime_items_expose_runtime_override_rows() {
        let run_options = RunOptionsState {
            thinking_mode: Some("deep".to_string()),
            parallel_tool_calls: Some(true),
            temperature: Some(0.7),
            ..RunOptionsState::default()
        };

        let items = settings_studio_runtime_items(&run_options);
        let thinking = items
            .iter()
            .find(|item| item.label == "Thinking Mode")
            .expect("thinking mode item should exist");
        let parallel = items
            .iter()
            .find(|item| item.label == "Parallel Tool Calls")
            .expect("parallel tool calls item should exist");

        assert_eq!(thinking.value, "override \"deep\"");
        assert!(thinking.detail.contains("thinking mode"));
        assert_eq!(parallel.value, "override on");
    }
    #[test]
    fn settings_value_edit_prompt_breaks_context_into_multiple_lines() {
        let field = SETTINGS_FIELDS
            .iter()
            .find(|field| field.path == "default.agent")
            .copied()
            .expect("default.agent field should exist");
        let prompt = settings_value_edit_prompt(
            field,
            &JsonValue::Null,
            &JsonValue::String("build".to_string()),
        );

        let lines = prompt.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "default agent name");
        assert!(lines[1].contains("Effective value: \"build\""));
        assert!(lines[2].contains("Enter text"));
    }

    #[test]
    fn runtime_setting_edit_prompt_breaks_override_into_multiple_lines() {
        let field = RUNTIME_SETTINGS
            .iter()
            .find(|field| field.label == "System Prompt")
            .copied()
            .expect("system prompt field should exist");
        let prompt = runtime_setting_edit_prompt(field, "inherit");

        let lines = prompt.lines().collect::<Vec<_>>();
        assert!(lines[0].contains("system prompt"));
        assert_eq!(lines[1], "Current override: inherit");
        assert!(lines[2].contains("Enter text"));
    }

    #[test]
    fn editor_word_delete_and_yank_follow_shell_bindings() {
        let mut editor = Editor::from_text("hello brave new world".to_string());

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT));
        assert_eq!(editor.text(), "hello brave new world");
        assert_eq!(editor.cursor, "hello brave new ".len());

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(editor.text(), "hello brave world");
        assert_eq!(editor.kill_buffer, "new ");

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        assert_eq!(editor.text(), "hello brave new world");
    }

    #[test]
    fn editor_moves_by_grapheme_clusters() {
        let mut editor = Editor::from_text("a👨‍👩‍👧‍👦b".to_string());
        editor.move_left();
        assert_eq!(editor.cursor, "a👨‍👩‍👧‍👦".len());
        editor.move_left();
        assert_eq!(editor.cursor, "a".len());
    }

    #[test]
    fn editor_keeps_inline_elements_atomic() {
        let mut editor = Editor::from_text("hello ".to_string());
        editor.insert_element("[file Cargo.toml]");
        let before_element = "hello ".len();
        let after_element = "hello [file Cargo.toml]".len();

        assert_eq!(
            editor.element_texts(),
            vec!["[file Cargo.toml]".to_string()]
        );
        assert_eq!(editor.cursor, after_element);

        editor.move_left();
        assert_eq!(editor.cursor, before_element);

        editor.move_right();
        assert_eq!(editor.cursor, after_element);

        editor.backspace();
        assert_eq!(editor.text(), "hello ");
        assert!(editor.element_texts().is_empty());
    }

    #[test]
    fn draft_title_uses_item_labels_instead_of_raw_placeholders() {
        let placeholder = "[file Cargo.toml]";
        let draft = ComposerDraft {
            text: format!("inspect {placeholder}"),
            items: vec![ComposerItem::Attachment(StagedAttachment {
                path: PathBuf::from("Cargo.toml"),
                placeholder: placeholder.to_string(),
                label: "file: Cargo.toml (2 KB)".to_string(),
                is_temp: false,
            })],
            elements: vec![ComposerDraftElement {
                placeholder: placeholder.to_string(),
                range: "inspect ".len().."inspect ".len() + placeholder.len(),
            }],
        };

        let title = draft_title_source(&draft).expect("title preview should exist");
        assert_eq!(title, "inspect file: Cargo.toml (2 KB)");
    }

    #[test]
    fn persistent_snapshot_filters_temp_attachments_and_repairs_ranges() {
        let temp = "[image clipboard.png]";
        let stable = "[file Cargo.toml]";
        let draft = ComposerDraft {
            text: format!("inspect {temp} then {stable}"),
            items: vec![
                ComposerItem::Attachment(StagedAttachment {
                    path: PathBuf::from("/tmp/clipboard.png"),
                    placeholder: temp.to_string(),
                    label: "image: clipboard.png".to_string(),
                    is_temp: true,
                }),
                ComposerItem::Attachment(StagedAttachment {
                    path: PathBuf::from("Cargo.toml"),
                    placeholder: stable.to_string(),
                    label: "file: Cargo.toml".to_string(),
                    is_temp: false,
                }),
            ],
            elements: vec![
                ComposerDraftElement {
                    placeholder: temp.to_string(),
                    range: "inspect ".len().."inspect ".len() + temp.len(),
                },
                ComposerDraftElement {
                    placeholder: stable.to_string(),
                    range: "inspect ".len() + temp.len() + " then ".len()
                        .."inspect ".len() + temp.len() + " then ".len() + stable.len(),
                },
            ],
        };

        let persistent = draft
            .persistent_snapshot()
            .expect("persistent draft should still exist");
        assert_eq!(persistent.text, format!("inspect  then {stable}"));
        assert_eq!(persistent.items.len(), 1);
        assert_eq!(persistent.elements.len(), 1);
        assert_eq!(persistent.elements[0].placeholder, stable);
        assert_eq!(
            &persistent.text[persistent.elements[0].start..persistent.elements[0].end],
            stable
        );
    }

    #[test]
    fn draft_store_round_trips_new_and_session_drafts() {
        let dir = tempfile::tempdir().expect("tempdir should exist");
        let path = dir.path().join("tui-drafts.json");
        let mut store = DraftStore::default();

        store.set(
            DraftSlot::NewSession,
            ComposerDraft {
                text: "new draft".to_string(),
                items: Vec::new(),
                elements: Vec::new(),
            },
        );
        store.set(
            DraftSlot::Session(42),
            ComposerDraft {
                text: "session [paste 10 chars]".to_string(),
                items: vec![ComposerItem::LargePaste(StagedPaste {
                    placeholder: "[paste 10 chars]".to_string(),
                    label: "paste: 10 chars".to_string(),
                    text: "0123456789".to_string(),
                })],
                elements: vec![ComposerDraftElement {
                    placeholder: "[paste 10 chars]".to_string(),
                    range: "session ".len().."session ".len() + "[paste 10 chars]".len(),
                }],
            },
        );

        store.persist(&path).expect("draft store should persist");
        let loaded = DraftStore::load(&path).expect("draft store should load");

        assert_eq!(
            loaded.get(DraftSlot::NewSession),
            store.get(DraftSlot::NewSession)
        );
        assert_eq!(
            loaded.get(DraftSlot::Session(42)),
            store.get(DraftSlot::Session(42))
        );
    }

    #[test]
    fn draft_store_does_not_reload_temp_only_entries() {
        let dir = tempfile::tempdir().expect("tempdir should exist");
        let path = dir.path().join("tui-drafts.json");
        let mut store = DraftStore::default();

        store.set(
            DraftSlot::Session(7),
            ComposerDraft {
                text: "[image clipboard.png]".to_string(),
                items: vec![ComposerItem::Attachment(StagedAttachment {
                    path: PathBuf::from("/tmp/clipboard.png"),
                    placeholder: "[image clipboard.png]".to_string(),
                    label: "image: clipboard.png".to_string(),
                    is_temp: true,
                })],
                elements: vec![ComposerDraftElement {
                    placeholder: "[image clipboard.png]".to_string(),
                    range: 0.."[image clipboard.png]".len(),
                }],
            },
        );

        store.persist(&path).expect("draft store should persist");
        let loaded = DraftStore::load(&path).expect("draft store should load");

        assert!(loaded.get(DraftSlot::Session(7)).is_none());
    }

    #[test]
    fn prompt_history_round_trips_dedupes_and_caps_entries() {
        let dir = tempfile::tempdir().expect("tempdir should exist");
        let path = dir.path().join("prompt-history.jsonl");
        let mut history = PromptHistory::default();

        assert!(history.push("alpha".to_string()));
        assert!(history.push("beta".to_string()));
        assert!(history.push("alpha".to_string()));
        assert!(!history.push("alpha".to_string()));
        assert_eq!(
            history.entries,
            vec!["beta".to_string(), "alpha".to_string()]
        );

        history
            .persist(path.as_path())
            .expect("prompt history should persist");
        let loaded = PromptHistory::load(path.as_path()).expect("prompt history should load");
        assert_eq!(loaded.entries, history.entries);

        let mut capped = PromptHistory::default();
        for index in 0..MAX_PROMPT_HISTORY_ENTRIES + 3 {
            assert!(capped.push(format!("prompt {index}")));
        }
        assert_eq!(capped.len(), MAX_PROMPT_HISTORY_ENTRIES);
        assert_eq!(capped.get(0), Some("prompt 3"));
    }
    #[test]
    fn ctrl_a_and_ctrl_e_cross_line_boundaries_like_shell_editors() {
        let mut editor = Editor::from_text("alpha\nbeta".to_string());
        editor.cursor = "alpha\n".len();

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(editor.cursor, 0);

        editor.cursor = "alpha".len();
        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(editor.cursor, "alpha\nbeta".len());
    }

    #[test]
    fn alt_arrow_and_ctrl_alt_h_follow_terminal_word_bindings() {
        let mut editor = Editor::from_text("hello brave new world".to_string());

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT));
        assert_eq!(editor.cursor, "hello brave new ".len());

        editor.handle_multiline_input_key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ));
        assert_eq!(editor.text(), "hello brave world");
        assert_eq!(editor.kill_buffer, "new ");
    }

    #[test]
    fn ctrl_u_at_line_start_removes_previous_newline() {
        let mut editor = Editor::from_text("alpha\nbeta".to_string());
        editor.cursor = "alpha\n".len();

        editor.handle_multiline_input_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));

        assert_eq!(editor.text(), "alphabeta");
        assert_eq!(editor.kill_buffer, "\n");
        assert_eq!(editor.cursor, "alpha".len());
    }

    #[test]
    fn find_search_ranges_matches_ascii_case_insensitively() {
        let ranges = find_search_ranges("Hello hello HELLO", "hello");
        let segments = ranges
            .into_iter()
            .map(|range| "Hello hello HELLO"[range].to_string())
            .collect::<Vec<_>>();
        assert_eq!(segments, vec!["Hello", "hello", "HELLO"]);
    }

    #[test]
    fn highlight_search_line_preserves_unmatched_text() {
        let line = view::highlight_search_line(
            "alpha hello omega",
            Style::default(),
            "hello",
            false,
            true,
        );
        let rendered = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(rendered, "alpha hello omega");
    }

    #[test]
    fn transcript_search_starts_at_first_match() {
        let mut transcript = TranscriptState {
            search_query: "hello".to_string(),
            rendered: Some(RenderedTranscript {
                width: 80,
                lines: vec![
                    RenderedLine::plain("alpha"),
                    RenderedLine::plain("hello one"),
                    RenderedLine::plain("beta"),
                    RenderedLine::plain("hello two"),
                ],
                search_matches: vec![1, 3],
                message_line_starts: Vec::new(),
            }),
            ..TranscriptState::default()
        };

        transcript.jump_search_match(80, 3, true);
        assert_eq!(transcript.search_match_index, Some(0));
        assert_eq!(transcript.scroll, 0);

        transcript.jump_search_match(80, 3, true);
        assert_eq!(transcript.search_match_index, Some(1));
    }

    #[test]
    fn paste_burst_turns_following_enter_into_newline() {
        let mut burst = PasteBurst::default();
        let start = Instant::now();

        assert!(matches!(
            burst.on_plain_char('a', start),
            PasteCharDecision::RetainFirstChar
        ));
        assert!(matches!(
            burst.on_plain_char('b', start + Duration::from_millis(1)),
            PasteCharDecision::BeginBufferFromPending
        ));
        burst.append_char_to_buffer('b', start + Duration::from_millis(1));

        assert!(burst.newline_should_insert_instead_of_submit(start + Duration::from_millis(2)));
        assert!(burst.append_newline_if_active(start + Duration::from_millis(2)));

        match burst.flush_now() {
            PasteFlushResult::Paste(text) => assert_eq!(text, "ab\n"),
            other => panic!("unexpected flush result: {other:?}"),
        }
    }

    #[test]
    fn transcript_export_markdown_includes_session_metadata() {
        let now = Utc::now();
        let markdown = render_transcript_export_markdown(
            &I18n::english(),
            Some(42),
            "Planning Session",
            None,
            &[MessageResource {
                id: 10,
                session_id: 42,
                role: MessageRole::User,
                state: MessageStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: MessageMetadata::default(),
                usage: None,
                finish: None,
                part_count: 1,
                parts: Some(vec![MessagePart::with_content(
                    11,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    PartContent::text("hello export"),
                )]),
            }],
            true,
        );

        assert!(markdown.contains("# Planning Session"));
        assert!(markdown.contains("- Session ID: 42"));
        assert!(markdown.contains("- Older Messages Omitted: yes"));
        assert!(markdown.contains("hello export"));
    }

    #[test]
    fn build_timeline_item_summarizes_command_end_events() {
        let now = Utc::now();
        use agena::event::{EventMeta, envelope::ENVELOPE_SCHEMA_VERSION};
        let event = DomainEvent {
            meta: EventMeta {
                id: uuid::Uuid::new_v4(),
                seq_global: 12,
                seq_session: Some(12),
                session_id: Some(9),
                workspace_id: None,
                created_at: now,
                causation_id: None,
                correlation_id: None,
                envelope_schema: ENVELOPE_SCHEMA_VERSION,
            },
            kind: AgenaSessionEvent::CommandEnd(CommandEndEvent {
                context: CommandContext {
                    session_id: 9,
                    call_id: 77,
                    message_id: Some(5),
                    part_id: Some(6),
                },
                status: ExecutionStatus::Completed,
                exit_code: 0,
                duration_ms: 240,
                stdout: "ok".to_string(),
                stderr: String::new(),
                aggregated_output: "ok".to_string(),
                ts_ms: now.timestamp_millis(),
            }),
        };
        let item = build_timeline_item(&event);

        assert!(item.summary.contains("command_end"));
        assert!(item.summary.contains("exit=0"));
        assert!(item.detail.contains("duration_ms: 240"));
        assert!(item.copy_text.contains("call_id: 77"));
        assert_eq!(item.linked_message_id, Some(5));
    }

    #[test]
    fn transcript_jump_to_message_scrolls_to_linked_message() {
        let mut transcript = TranscriptState {
            rendered: Some(RenderedTranscript {
                width: 80,
                lines: vec![
                    RenderedLine::plain("header"),
                    RenderedLine::plain("msg1"),
                    RenderedLine::plain("msg2"),
                    RenderedLine::plain("msg3"),
                ],
                search_matches: Vec::new(),
                message_line_starts: vec![(10, 1), (20, 2), (30, 3)],
            }),
            ..TranscriptState::default()
        };

        transcript.jump_to_message(80, 3, 30);
        assert_eq!(transcript.scroll, 1);
        assert!(!transcript.follow_tail);
    }

    #[test]
    fn transcript_live_events_fill_assistant_message_parts() {
        let now = Utc::now();
        let mut transcript = TranscriptState::default();
        transcript.reset(42, "Live".to_string());

        let reasoning_part = MessagePart::with_content(
            501,
            77,
            now,
            ExecutionStatus::Pending,
            PartContent::Reasoning(ReasoningPart {
                summary: Vec::new(),
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        let text_part = MessagePart::with_content(
            502,
            77,
            now,
            ExecutionStatus::Pending,
            PartContent::text(String::new()),
        );

        assert!(!transcript.apply_live_event(
            &test_domain_event(
                42,
                1,
                AgenaSessionEvent::MessagePartUpdated(MessagePartUpdatedEvent {
                    session_id: 42,
                    message_id: 77,
                    message_role: agena::role::Role::Assistant,
                    message_state: MessageStatus::InProgress,
                    message_created_at: now,
                    part: reasoning_part.clone(),
                    ts_ms: now.timestamp_millis(),
                }),
            ),
            80,
            20,
        ));
        assert!(!transcript.apply_live_event(
            &test_domain_event(
                42,
                2,
                AgenaSessionEvent::MessagePartDelta(MessagePartDeltaEvent {
                    session_id: 42,
                    message_id: 77,
                    part_id: reasoning_part.id,
                    call_id: None,
                    field: PartDeltaField::ReasoningSummary,
                    delta: "thinking".to_string(),
                    seq: 1,
                    ts_ms: now.timestamp_millis(),
                }),
            ),
            80,
            20,
        ));
        assert!(!transcript.apply_live_event(
            &test_domain_event(
                42,
                3,
                AgenaSessionEvent::MessagePartUpdated(MessagePartUpdatedEvent {
                    session_id: 42,
                    message_id: 77,
                    message_role: agena::role::Role::Assistant,
                    message_state: MessageStatus::InProgress,
                    message_created_at: now,
                    part: text_part.clone(),
                    ts_ms: now.timestamp_millis(),
                }),
            ),
            80,
            20,
        ));
        assert!(!transcript.apply_live_event(
            &test_domain_event(
                42,
                4,
                AgenaSessionEvent::MessagePartDelta(MessagePartDeltaEvent {
                    session_id: 42,
                    message_id: 77,
                    part_id: text_part.id,
                    call_id: None,
                    field: PartDeltaField::Text,
                    delta: "Hi there".to_string(),
                    seq: 1,
                    ts_ms: now.timestamp_millis(),
                }),
            ),
            80,
            20,
        ));
        assert!(
            !transcript.apply_live_event(
                &test_domain_event(
                    42,
                    5,
                    serde_json::from_value::<AgenaSessionEvent>(json!({
                        "kind": "assistant_message_completed",
                        "payload": {
                            "message_id": 77,
                            "turn_id": uuid::Uuid::new_v4(),
                            "created_at": now,
                            "content": { "blocks": [] },
                            "parts": [
                                MessagePart::with_content(
                                    501,
                                    77,
                                    now,
                                    ExecutionStatus::Completed,
                                    PartContent::reasoning_summary("thinking"),
                                ),
                                MessagePart::with_content(
                                    502,
                                    77,
                                    now,
                                    ExecutionStatus::Completed,
                                    PartContent::text("Hi there"),
                                ),
                            ],
                            "usage": null,
                            "finish_reason": "stop",
                            "metadata": MessageMetadata::default(),
                        }
                    }))
                    .expect("assistant completion event should deserialize"),
                ),
                80,
                20,
            )
        );

        let message = transcript
            .messages
            .iter()
            .find(|message| message.id == 77)
            .expect("assistant message should exist");
        assert_eq!(message.state, MessageStatus::Completed);
        assert_eq!(assistant_message_text(message).as_deref(), Some("Hi there"));
    }

    #[test]
    fn merge_latest_messages_keeps_richer_live_message_over_empty_snapshot() {
        let now = Utc::now();
        let mut transcript = TranscriptState::default();
        transcript.session_id = Some(42);
        transcript.messages = vec![MessageResource {
            id: 77,
            session_id: 42,
            role: MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at: now,
            updated_at: now + chrono::Duration::milliseconds(10),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: Some("stop".to_string()),
            part_count: 1,
            parts: Some(vec![MessagePart::with_content(
                502,
                77,
                now,
                ExecutionStatus::Completed,
                PartContent::text("Hi there"),
            )]),
        }];

        transcript.merge_latest_messages(
            PaginatedResponse {
                items: vec![MessageResource {
                    id: 77,
                    session_id: 42,
                    role: MessageRole::Assistant,
                    state: MessageStatus::InProgress,
                    created_at: now,
                    updated_at: now,
                    metadata: MessageMetadata::default(),
                    usage: None,
                    finish: None,
                    part_count: 1,
                    parts: Some(vec![MessagePart::with_content(
                        502,
                        77,
                        now,
                        ExecutionStatus::Pending,
                        PartContent::text(String::new()),
                    )]),
                }],
                page: Default::default(),
            },
            80,
            20,
        );

        let message = transcript
            .messages
            .iter()
            .find(|message| message.id == 77)
            .expect("assistant message should still exist");
        assert_eq!(message.state, MessageStatus::Completed);
        assert_eq!(assistant_message_text(message).as_deref(), Some("Hi there"));
    }

    #[test]
    fn build_visible_session_items_keeps_ancestors_for_tree_search() {
        let now = Utc::now();
        let items = vec![
            test_session(1, None, "Root", now),
            test_session(2, Some(1), "Child", now),
            test_session(3, Some(2), "Target leaf", now),
        ];

        let visible =
            build_visible_session_items(items.as_slice(), SessionViewMode::Subtree, "target");
        let ids = visible
            .into_iter()
            .map(|session| session.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn build_lineage_session_items_orders_path_before_sibling_branches() {
        let now = Utc::now();
        let mut root = test_session(1, None, "Root", now);
        root.child_session_count = 2;

        let mut ancestor = test_session(2, Some(1), "Ancestor", now);
        ancestor.child_session_count = 2;

        let mut current = test_session(3, Some(2), "Current", now);
        current.child_session_count = 1;

        let items = vec![
            root,
            ancestor,
            current,
            test_session(4, Some(3), "Current Child", now),
            test_session(5, Some(1), "Root Sibling", now),
            test_session(6, Some(2), "Ancestor Sibling", now),
        ];

        let lineage = build_lineage_session_items(items.as_slice(), 3);
        let ids = lineage
            .iter()
            .map(|item| item.session.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![1, 2, 3, 4, 6, 5]);

        let relations = lineage
            .iter()
            .map(|item| (item.session.id, item.relation, item.is_leaf))
            .collect::<Vec<_>>();
        assert_eq!(
            relations,
            vec![
                (1, LineageRelation::Ancestor, false),
                (2, LineageRelation::Ancestor, false),
                (3, LineageRelation::Current, false),
                (4, LineageRelation::Child, true),
                (6, LineageRelation::Sibling, true),
                (5, LineageRelation::Sibling, true),
            ]
        );
    }

    #[test]
    fn sanitize_terminal_text_strips_ansi_and_carriage_returns() {
        let text = "\u{1b}[31mred\u{1b}[0m\r\nnext\u{7}\u{2068}rtl\u{2069}";
        assert_eq!(sanitize_terminal_text(text), "red\nnext rtl");
    }

    #[test]
    fn composer_height_reserves_optional_footer_rows() {
        assert_eq!(2_u16 + u16::from(false), 2);
        assert_eq!(2_u16 + u16::from(true), 3);
    }

    #[test]
    fn adaptive_modal_size_expands_to_available_space_on_small_terminals() {
        assert_eq!(view::adaptive_modal_width(60, 92), 58);
        assert_eq!(view::adaptive_modal_height(16, 24), 14);
    }

    #[test]
    fn adaptive_modal_size_preserves_readable_padding_on_mid_sized_terminals() {
        assert_eq!(view::adaptive_modal_width(88, 96), 86);
        assert_eq!(view::adaptive_modal_height(24, 28), 22);
    }

    #[test]
    fn adaptive_detail_split_falls_back_before_panels_collapse() {
        assert_eq!(
            view::adaptive_detail_split(70, 40, 46),
            [Constraint::Percentage(50), Constraint::Percentage(50)]
        );
        assert_eq!(
            view::adaptive_vertical_split(16, 7, 9),
            [Constraint::Percentage(50), Constraint::Percentage(50)]
        );
    }

    #[test]
    fn detail_overlays_stack_before_side_by_side_becomes_unreadable() {
        assert!(view::should_stack_detail_layout(70, 40, 46));
        assert!(view::should_stack_detail_layout(74, 34, 48));
        assert!(!view::should_stack_detail_layout(120, 40, 46));
    }

    #[test]
    fn transcript_surface_header_height_stays_minimal() {
        assert_eq!(view::transcript_surface_header_height(8), 2);
        assert_eq!(view::transcript_surface_header_height(14), 2);
        assert_eq!(view::transcript_surface_header_height(24), 2);
    }

    #[test]
    fn truncate_display_text_adds_ellipsis_when_needed() {
        assert_eq!(view::truncate_display_text("workspace-root", 8), "works...");
        assert_eq!(view::truncate_display_text("agena", 8), "agena");
    }

    #[test]
    fn composer_height_accounts_for_extra_rows() {
        let logical_lines = 1_u16;
        let no_items_height = min(12, logical_lines + 2);
        let with_items_height = min(12, logical_lines + 3);
        assert_eq!(no_items_height, 3);
        assert_eq!(with_items_height, 4);
    }

    #[test]
    fn build_lineage_session_items_marks_descendants_of_current_as_children() {
        let now = Utc::now();
        let mut current = test_session(1, None, "Current Root", now);
        current.child_session_count = 2;

        let items = vec![
            current,
            test_session(2, Some(1), "Child A", now),
            test_session(3, Some(1), "Child B", now),
        ];

        let lineage = build_lineage_session_items(items.as_slice(), 1);
        let relations = lineage
            .iter()
            .map(|item| (item.session.id, item.relation))
            .collect::<Vec<_>>();
        assert_eq!(
            relations,
            vec![
                (1, LineageRelation::Current),
                (3, LineageRelation::Child),
                (2, LineageRelation::Child),
            ]
        );
    }

    #[test]
    fn summarize_lineage_session_items_reports_root_depth_and_branch_counts() {
        let now = Utc::now();
        let mut root = test_session(1, None, "Root", now);
        root.child_session_count = 2;

        let mut ancestor = test_session(2, Some(1), "Ancestor", now);
        ancestor.child_session_count = 2;

        let mut current = test_session(3, Some(2), "Current", now);
        current.child_session_count = 2;

        let items = vec![
            root,
            ancestor,
            current,
            test_session(4, Some(3), "Child A", now),
            test_session(5, Some(3), "Child B", now),
            test_session(6, Some(2), "Side Branch", now),
        ];

        let lineage = build_lineage_session_items(items.as_slice(), 3);
        let summary =
            summarize_lineage_session_items(lineage.as_slice()).expect("summary should exist");

        assert_eq!(
            summary,
            SessionLineageSummary {
                root_id: 1,
                depth: 2,
                side_branch_count: 1,
                descendant_count: 2,
            }
        );
    }

    #[test]
    fn session_view_mode_cycles_in_expected_order() {
        assert_eq!(SessionViewMode::All.next(), SessionViewMode::Roots);
        assert_eq!(SessionViewMode::Roots.next(), SessionViewMode::Subtree);
        assert_eq!(SessionViewMode::Subtree.next(), SessionViewMode::All);
    }

    fn test_domain_event(
        session_id: i64,
        seq_session: i64,
        kind: AgenaSessionEvent,
    ) -> DomainEvent {
        DomainEvent {
            meta: agena::event::EventMeta {
                id: uuid::Uuid::new_v4(),
                seq_global: seq_session,
                seq_session: Some(seq_session),
                session_id: Some(session_id),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: agena::event::envelope::ENVELOPE_SCHEMA_VERSION,
            },
            kind,
        }
    }

    fn test_session(
        id: i64,
        parent_id: Option<i64>,
        title: &str,
        updated_at: DateTime<Utc>,
    ) -> SessionResource {
        SessionResource {
            id,
            parent_id,
            depth: 0,
            root_id: id,
            workspace_id: 1,
            title: title.to_string(),
            version: 1,
            is_subagent: false,
            created_at: updated_at,
            updated_at,
            message_count: 0,
            child_session_count: 0,
            last_message_at: None,
            goal: None,
        }
    }
}
