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
    event::{DomainEvent, EventKind as AgenaSessionEvent},
    message::{
        AttachmentKind, FileChangeKind, FirstPartyToolInput, FirstPartyToolOutput, MessagePart,
        PartContent, ToolExecutionPart, ToolInvocation, UserInputReply, UserInputReplyKind,
        UserInputRequest,
    },
    model::ModelRef,
    permission::{
        PermissionAction, PermissionMode, PermissionReplyKind, PermissionRequest, PermissionScope,
    },
    provider::ProviderModel,
    role::Role,
};
use agena_api::{
    commands::UpsertPermissionRuleParams,
    pagination::PaginatedResponse,
    resource::{
        MessageResource, PermissionRuleResource, ProviderSummaryResource, RunOptions,
        SessionExecutionResource, SessionResource, SessionRunState,
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
use textwrap::{Options as WrapOptions, WordSplitter, wrap};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::interval,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::backend::{Backend, SessionRefresh};
use crate::clipboard::{
    normalize_pasted_path, paste_image_to_temp_png, pasted_image_format, set_clipboard_text,
};
use crate::commands::{self, CommandId, CommandSpec};
use crate::composer_queue::{ComposerQueue, QueuePriority, QueuedMessage};
use crate::external_editor::{edit_text, open_path};
use crate::external_pager::page_text;
use crate::i18n::I18n;
use crate::keybindings::{ComposerAction, ComposerKeyBindings};
use crate::terminal;
use crate::tui_config::{TuiConfig, TuiStatusLineConfig};
use crate::ui_text;

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
const TOOL_CARD_PREVIEW_LINES: usize = 18;
const TOOL_CARD_PREVIEW_CHARS: usize = 6_000;
const PROMPT_SUMMARY_TAG: &str = "prompt_summary";

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
    flash: Option<FlashMessage>,
    sessions: SessionListState,
    transcript: TranscriptState,
    run_options: RunOptionsState,
    composer: Editor,
    composer_items: Vec<ComposerItem>,
    draft_store: DraftStore,
    draft_store_path: PathBuf,
    draft_store_dirty: bool,
    draft_store_last_persist_at: Instant,
    draft_store_reported_error: Option<String>,
    pending_draft_store_error: Option<String>,
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
    WorkspaceSessionsLoaded {
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
    ModelsLoaded {
        purpose: ModelLoadPurpose,
        provider_id: String,
        result: UiResult<Vec<ProviderModel>>,
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
        triggers_refresh: bool,
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
    Help,
    SessionSearch(LineInputOverlay),
    TranscriptSearch(LineInputOverlay),
    SessionRename(LineInputOverlay),
    PermissionRuleEdit(PermissionRuleEditOverlay),
    FileAttach(FileAttachOverlay),
    Permission(PermissionOverlay),
    UserInputReply(UserInputOverlay),
    Confirm(ConfirmOverlay),
    Picker(PickerOverlay),
    Timeline(TimelineOverlay),
    PluginInspector(PluginInspectorOverlay),
}

#[derive(Debug, Clone)]
struct LineInputOverlay {
    title: String,
    prompt: String,
    input: Editor,
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
    scope: String,
    session_id: String,
    mode: PermissionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionRuleSubjectKind {
    BuiltinTool,
    PathAccess,
}

#[derive(Debug, Clone)]
struct UserInputOverlay {
    session_id: i64,
    request: UserInputRequest,
    input: Editor,
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
    Model(ModelRef),
    Session(i64),
    Message(i64),
    PermissionRuleCreate,
    PermissionRule(PermissionRuleResource),
    Inspector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderPickerPurpose {
    SetProvider,
    ChooseModelProvider,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelLoadPurpose {
    SetModel,
}

#[derive(Debug, Clone)]
enum PickerKind {
    Commands,
    WorkspaceSessions,
    Lineage {
        session_id: i64,
    },
    RewindMessages {
        session_id: i64,
    },
    Providers(ProviderPickerPurpose),
    Models {
        provider_id: String,
        purpose: ModelLoadPurpose,
    },
    ChildSessions {
        parent_session_id: i64,
    },
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
struct CurrentLineageState {
    session_id: i64,
    items: Vec<LineageSessionItem>,
    summary: SessionLineageSummary,
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
            flash: None,
            sessions: SessionListState {
                search_query: launch.initial_session_search.unwrap_or_default(),
                ..SessionListState::default()
            },
            transcript: TranscriptState::new(i18n),
            run_options: RunOptionsState::default(),
            composer: Editor::default(),
            composer_items: Vec::new(),
            draft_store,
            draft_store_path,
            draft_store_dirty: false,
            draft_store_last_persist_at: Instant::now()
                .checked_sub(Duration::from_millis(DRAFT_PERSIST_INTERVAL_MS))
                .unwrap_or_else(Instant::now),
            draft_store_reported_error: None,
            pending_draft_store_error,
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
            self.should_quit = true;
            return;
        }

        if self.handle_overlay_key(key) {
            return;
        }

        // ESC while a turn is in flight — global priority. Cancels the
        // active turn before falling through to focus-specific Esc.
        // Mirrors Claude Code's `useCancelRequest` priority order.
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
                    self.overlay = Some(Overlay::Help);
                    return;
                }
                _ => {}
            }
        }

        if matches!(key.code, KeyCode::Tab) {
            self.focus = match self.focus {
                Focus::Sessions => Focus::Transcript,
                Focus::Transcript => Focus::Composer,
                Focus::Composer => Focus::Sessions,
            };
            return;
        }

        if matches!(key.code, KeyCode::BackTab) {
            self.focus = match self.focus {
                Focus::Sessions => Focus::Composer,
                Focus::Transcript => Focus::Sessions,
                Focus::Composer => Focus::Transcript,
            };
            return;
        }

        if matches!(key.code, KeyCode::Char('/')) && self.focus != Focus::Composer {
            self.overlay = Some(match self.focus {
                Focus::Sessions => Overlay::SessionSearch(LineInputOverlay {
                    title: ui_text::t(&self.i18n, "overlay-session-search-title"),
                    prompt: ui_text::t(&self.i18n, "overlay-session-search-prompt"),
                    input: Editor::from_text(self.sessions.search_query.clone()),
                }),
                Focus::Transcript => Overlay::TranscriptSearch(LineInputOverlay {
                    title: ui_text::t(&self.i18n, "overlay-transcript-search-title"),
                    prompt: ui_text::t(&self.i18n, "overlay-transcript-search-prompt"),
                    input: Editor::from_text(self.transcript.search_query.clone()),
                }),
                Focus::Composer => unreachable!("composer focus is excluded above"),
            });
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
            Overlay::Help => matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
            ),
            Overlay::SessionSearch(dialog) => {
                self.handle_line_overlay_key(key, dialog, OverlayCommit::SessionSearch)
            }
            Overlay::TranscriptSearch(dialog) => {
                self.handle_line_overlay_key(key, dialog, OverlayCommit::TranscriptSearch)
            }
            Overlay::SessionRename(dialog) => self.handle_session_rename_overlay_key(key, dialog),
            Overlay::PermissionRuleEdit(dialog) => {
                self.handle_permission_rule_edit_overlay_key(key, dialog)
            }
            Overlay::FileAttach(dialog) => self.handle_file_attach_overlay_key(key, dialog),
            Overlay::Permission(dialog) => self.handle_permission_overlay_key(key, dialog),
            Overlay::UserInputReply(dialog) => self.handle_user_input_overlay_key(key, dialog),
            Overlay::Confirm(dialog) => self.handle_confirm_overlay_key(key, dialog),
            Overlay::Picker(dialog) => self.handle_picker_overlay_key(key, dialog),
            Overlay::Timeline(dialog) => self.handle_timeline_overlay_key(key, dialog),
            Overlay::PluginInspector(dialog) => {
                self.handle_plugin_inspector_overlay_key(key, dialog)
            }
        };

        if !close {
            self.overlay = Some(overlay);
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
                    OverlayCommit::SessionSearch => {
                        self.sessions.search_query = value;
                        self.refresh_sessions_after_query_change();
                    }
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
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Enter => {
                dialog.input.flush_all_pending_input();
                match parse_user_input_answers(&self.i18n, dialog.input.text(), &dialog.request) {
                    Ok(answers) => {
                        let reply = UserInputReply {
                            request_id: dialog.request.request_id.clone(),
                            kind: UserInputReplyKind::Submit,
                            answers,
                            reason: None,
                        };
                        self.request_user_input_reply(dialog.session_id, reply);
                        true
                    }
                    Err(error) => {
                        self.flash_warning(error);
                        false
                    }
                }
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let reply = UserInputReply {
                    request_id: dialog.request.request_id.clone(),
                    kind: UserInputReplyKind::Cancel,
                    answers: BTreeMap::new(),
                    reason: None,
                };
                self.request_user_input_reply(dialog.session_id, reply);
                true
            }
            _ => {
                dialog.input.handle_line_input_key(key);
                false
            }
        }
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

                match tokio::runtime::Handle::try_current() {
                    Ok(handle) => {
                        let result = match dialog.rule_id {
                            Some(rule_id) => handle
                                .block_on(self.backend.replace_permission_rule(rule_id, params)),
                            None => handle.block_on(self.backend.create_permission_rule(params)),
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
                    Err(error) => {
                        self.flash_error(error.to_string());
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

    fn handle_paste(&mut self, text: String) {
        let backend = self.backend.clone();
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::SessionSearch(dialog)
                | Overlay::TranscriptSearch(dialog)
                | Overlay::SessionRename(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
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
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                }
                Overlay::Picker(dialog) => {
                    dialog.input.flush_all_pending_input();
                    dialog.input.insert_str(text.as_str());
                    Self::refresh_picker_overlay(dialog);
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
                Overlay::Help => {}
            }
            return;
        }

        if self.focus == Focus::Composer {
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
        // Esc handling is special — double-tap clears the input. We track
        // it before consulting the configurable bindings.
        if matches!(key.code, KeyCode::Esc) && key.modifiers.is_empty() {
            self.handle_composer_esc();
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
                    self.composer.insert_explicit_newline();
                    return;
                }
                ComposerAction::EditQueue => {
                    if self.try_pop_queue_into_editor() {
                        return;
                    }
                    // Fall through to normal cursor-up behavior when queue
                    // is empty.
                }
            }
        }
        match key.code {
            KeyCode::F(3) => {
                self.open_file_attach_overlay();
            }
            KeyCode::F(4) => {
                self.composer.flush_all_pending_input();
                self.pending_ui_action = Some(UiAction::EditComposerExternally);
            }
            KeyCode::F(6) => {
                self.pending_ui_action = Some(UiAction::AttachClipboardImage);
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_file_attach_overlay();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.composer.flush_all_pending_input();
                self.pending_ui_action = Some(UiAction::EditComposerExternally);
            }
            KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.open_file_attach_overlay();
            }
            KeyCode::Char('i') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.pending_ui_action = Some(UiAction::AttachClipboardImage);
            }
            _ => {
                self.composer.handle_multiline_input_key(key);
                self.sync_composer_items_with_editor();
            }
        }
    }

    /// Single-Esc → leave composer focus. Double-Esc within the configured
    /// window → clear the input. Mirrors Claude Code's "double tap esc to
    /// clear input" affordance.
    fn handle_composer_esc(&mut self) {
        let now = Instant::now();
        let double = self
            .last_esc_at
            .map(|prev| now.duration_since(prev) <= self.double_esc_window)
            .unwrap_or(false);
        if double {
            self.composer = Editor::default();
            self.composer_items.clear();
            self.last_esc_at = None;
            return;
        }
        self.last_esc_at = Some(now);
        self.focus = Focus::Transcript;
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
            AppMessage::WorkspaceSessionsLoaded { result } => {
                self.handle_workspace_sessions_loaded(result)
            }
            AppMessage::LineageLoaded { session_id, result } => {
                self.handle_lineage_loaded(session_id, result)
            }
            AppMessage::RewindMessagesLoaded { session_id, result } => {
                self.handle_rewind_messages_loaded(session_id, result)
            }
            AppMessage::ProvidersLoaded { purpose, result } => {
                self.handle_providers_loaded(purpose, result)
            }
            AppMessage::ModelsLoaded {
                purpose,
                provider_id,
                result,
            } => self.handle_models_loaded(purpose, provider_id, result),
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
            AppMessage::SessionEventArrived {
                session_id,
                triggers_refresh,
            } => self.handle_session_event_arrived(session_id, triggers_refresh),
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
                self.request_refresh(session_id, true);
                self.request_sessions(false);
                // Pop the next pending message and submit it. Mirrors
                // Codex's `maybe_send_next_queued_input` post-turn.
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
        self.transcript.submitting = false;
        self.submitting_session_ids.remove(&session_id);
        if self.transcript.session_id == Some(session_id) {
            self.transcript.apply_execution(execution);
        }
        if refresh {
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
                        detail: format!("default {}", provider.default_model_ref),
                        value: PickerValue::Provider(provider),
                    })
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.overlay = Some(Overlay::Picker(dialog));
    }

    fn handle_workspace_sessions_loaded(&mut self, result: UiResult<Vec<SessionResource>>) {
        let Some(Overlay::Picker(mut dialog)) = self.overlay.take() else {
            return;
        };
        if !matches!(dialog.kind, PickerKind::WorkspaceSessions) {
            self.overlay = Some(Overlay::Picker(dialog));
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
                dialog.all_items = sessions
                    .into_iter()
                    .map(|session| self.workspace_session_picker_item(session))
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.overlay = Some(Overlay::Picker(dialog));
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
                        items: items.clone(),
                        summary,
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
                    .filter(|message| !message.metadata.has_tag(PROMPT_SUMMARY_TAG))
                    .rev()
                    .map(|message| self.rewind_message_picker_item(message))
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.overlay = Some(Overlay::Picker(dialog));
    }

    fn handle_models_loaded(
        &mut self,
        purpose: ModelLoadPurpose,
        provider_id: String,
        result: UiResult<Vec<ProviderModel>>,
    ) {
        let Some(Overlay::Picker(mut dialog)) = self.overlay.take() else {
            return;
        };
        let PickerKind::Models {
            provider_id: current_provider_id,
            purpose: current_purpose,
        } = &dialog.kind
        else {
            self.overlay = Some(Overlay::Picker(dialog));
            return;
        };
        if *current_provider_id != provider_id || *current_purpose != purpose {
            self.overlay = Some(Overlay::Picker(dialog));
            return;
        }

        dialog.loading = false;
        dialog.empty_message = ui_text::t(&self.i18n, "overlay-picker-empty");
        match result {
            Ok(models) => {
                dialog.all_items = models
                    .into_iter()
                    .map(|model| {
                        let context_window = model
                            .metadata
                            .limits
                            .context_window_tokens
                            .map(|value| format!("ctx {value}"))
                            .unwrap_or_else(|| "ctx ?".to_string());
                        let display_name = model
                            .display_name
                            .clone()
                            .unwrap_or_else(|| model.id.to_string());
                        PickerItem {
                            label: model.id.to_string(),
                            detail: format!("{display_name} | {context_window}"),
                            value: PickerValue::Model(model.reference()),
                        }
                    })
                    .collect();
                Self::refresh_picker_overlay(&mut dialog);
            }
            Err(error) => self.flash_error(error),
        }
        self.overlay = Some(Overlay::Picker(dialog));
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
                self.handle_session_execution_updated(session_id, execution, false);
                self.request_session_state(session_id);
                self.request_messages(session_id, MessageLoadMode::Replace);
                self.request_lineage(session_id);
                self.focus = Focus::Transcript;
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

    fn request_workspace_sessions_picker(&mut self) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_workspace_sessions(false)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::WorkspaceSessionsLoaded { result });
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

    fn request_models(&mut self, provider_id: String, purpose: ModelLoadPurpose) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_provider_models(provider_id.as_str())
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ModelsLoaded {
                purpose,
                provider_id,
                result,
            });
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
                    .send(AppMessage::SessionEventArrived {
                        session_id,
                        triggers_refresh: live.triggers_refresh,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.active_subscription = Some(handle);
    }

    fn handle_session_event_arrived(&mut self, session_id: i64, triggers_refresh: bool) {
        // Ignore events for sessions the user has already navigated away
        // from. The forwarder is normally aborted in that case but a few
        // in-flight messages may still land.
        if self.transcript.session_id != Some(session_id) {
            return;
        }
        if triggers_refresh {
            self.request_refresh(session_id, false);
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

    /// Secondary submit action (bare Enter by default). When the AI is
    /// idle, sends immediately. When the AI is mid-turn, the message is
    /// appended to the local pending queue and drained on turn
    /// completion. Mirrors Claude Code's default behavior.
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
        let Some(execution) = self.transcript.execution.as_ref() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-user-input-request"));
            return;
        };
        let Some(request) = execution.pending_user_input_requests.first().cloned() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-user-input-request"));
            return;
        };
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        self.overlay = Some(Overlay::UserInputReply(UserInputOverlay {
            session_id,
            request,
            input: Editor::default(),
        }));
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
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => match handle.block_on(self.backend.list_permission_rules()) {
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
                Err(error) => self.flash_error(error.to_string()),
            },
            Err(error) => self.flash_error(error.to_string()),
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
            self.backend
                .runtime_entry_rows()
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

    fn open_resume_session_picker(&mut self) {
        self.overlay = Some(Overlay::Picker(PickerOverlay {
            title: ui_text::t(&self.i18n, "overlay-resume-title"),
            prompt: ui_text::t(&self.i18n, "overlay-resume-prompt"),
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            footer: ui_text::t(&self.i18n, "overlay-picker-footer"),
            input: Editor::default(),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            loading: true,
            kind: PickerKind::WorkspaceSessions,
        }));
        self.request_workspace_sessions_picker();
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
        let title_key = match purpose {
            ProviderPickerPurpose::SetProvider => "overlay-providers-title",
            ProviderPickerPurpose::ChooseModelProvider => "overlay-model-providers-title",
        };
        let prompt_key = match purpose {
            ProviderPickerPurpose::SetProvider => "overlay-providers-prompt",
            ProviderPickerPurpose::ChooseModelProvider => "overlay-model-providers-prompt",
        };
        self.overlay = Some(Overlay::Picker(PickerOverlay {
            title: ui_text::t(&self.i18n, title_key),
            prompt: ui_text::t(&self.i18n, prompt_key),
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

    fn open_model_picker(&mut self, provider_id: String) {
        let title = self.i18n.text_args(
            "overlay-models-title",
            &crate::fl_args!("provider" => provider_id.clone()),
        );
        let prompt = self.i18n.text_args(
            "overlay-models-prompt",
            &crate::fl_args!("provider" => provider_id.clone()),
        );
        self.overlay = Some(Overlay::Picker(PickerOverlay {
            title,
            prompt,
            empty_message: ui_text::t(&self.i18n, "overlay-picker-loading"),
            footer: ui_text::t(&self.i18n, "overlay-picker-footer"),
            input: Editor::default(),
            all_items: Vec::new(),
            items: Vec::new(),
            selected: 0,
            loading: true,
            kind: PickerKind::Models {
                provider_id: provider_id.clone(),
                purpose: ModelLoadPurpose::SetModel,
            },
        }));
        self.request_models(provider_id, ModelLoadPurpose::SetModel);
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

    fn workspace_session_picker_item(&self, session: SessionResource) -> PickerItem {
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

        PickerItem {
            label: session.title.clone(),
            detail: detail_parts.join(" | "),
            value: PickerValue::Session(session.id),
        }
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
            (PickerKind::WorkspaceSessions, PickerValue::Session(session_id))
            | (PickerKind::Lineage { .. }, PickerValue::Session(session_id)) => {
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
                if matches!(
                    spec.id,
                    CommandId::Temperature | CommandId::MaxOutput | CommandId::System
                ) {
                    self.prefill_command(spec);
                } else {
                    self.execute_command(spec, "");
                }
            }
            (PickerKind::Commands, PickerValue::RuntimeEntry(entry_name)) => {
                self.composer
                    .set_text(format!("/{entry_name} ").trim_end().to_string());
                self.focus = Focus::Composer;
            }
            (PickerKind::WorkspaceSessions, PickerValue::Session(session_id)) => {
                self.open_session(
                    session_id,
                    ui_text::session_fallback_title(&self.i18n, session_id),
                );
                self.focus = Focus::Transcript;
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
            (
                PickerKind::Providers(ProviderPickerPurpose::ChooseModelProvider),
                PickerValue::Provider(provider),
            ) => {
                self.open_model_picker(provider.provider_id.clone());
            }
            (PickerKind::Models { .. }, PickerValue::Model(model)) => {
                self.apply_model_override(model);
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

    fn prefill_command(&mut self, spec: &'static CommandSpec) {
        if !self.composer.text().trim().is_empty() || !self.composer_items.is_empty() {
            self.flash_warning(ui_text::t(
                &self.i18n,
                "flash-command-prefill-requires-empty-composer",
            ));
            self.focus = Focus::Composer;
            return;
        }
        self.composer.set_text(format!("/{} ", spec.name));
        self.composer.cursor = self.composer.text().len();
        self.focus = Focus::Composer;
    }

    fn apply_provider_override(&mut self, provider: ProviderSummaryResource) {
        self.run_options.model = Some(ModelRef::new(
            provider.provider_id.clone(),
            provider.default_model.clone(),
        ));
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

    fn current_or_selected_session_summary(&self) -> Option<SessionResource> {
        if let Some(execution) = self.transcript.execution.as_ref() {
            return Some(execution.session.clone());
        }
        self.sessions.current_selected().cloned()
    }

    fn current_lineage_item(&self, session_id: i64) -> Option<&LineageSessionItem> {
        let lineage = self.current_lineage.as_ref()?;
        (lineage.session_id == self.transcript.session_id?).then_some(())?;
        lineage
            .items
            .iter()
            .find(|item| item.session.id == session_id)
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

    fn status_context_summary(&self) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(session_id) = self.current_or_selected_session_id() {
            parts.push(format!("#{session_id}"));
        }

        if let Some(theme) = self.plugin_theme.as_ref() {
            parts.push(format!("theme={}", theme.id));
        }

        let plugin_segments = self.backend.plugin_statusline_segments();
        if !plugin_segments.is_empty() {
            parts.push(format!("statusline+{}", plugin_segments.len()));
        }

        if let Some(session) = self.current_or_selected_session_summary() {
            if let Some(parent_id) = session.parent_id {
                parts.push(self.i18n.text_args(
                    "session-summary-parent",
                    &crate::fl_args!("id" => parent_id),
                ));
            }
            if session.child_session_count > 0 {
                parts.push(self.i18n.text_args(
                    "session-summary-children",
                    &crate::fl_args!("count" => session.child_session_count as i64),
                ));
            }
        }
        parts.extend(self.current_lineage_context_parts());
        parts.extend(self.current_execution_context_parts());

        parts.push(self.current_session_view_summary());

        if let Some(summary) = self.run_options.summary() {
            parts.push(summary);
        }

        (!parts.is_empty()).then(|| parts.join(" | "))
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

    fn resolve_model_provider_for_picker(&self) -> Option<String> {
        if let Some(model) = self.run_options.model.as_ref() {
            return Some(model.provider_id.to_string());
        }
        let providers = self.backend.list_providers();
        (providers.len() == 1).then(|| providers[0].provider_id.clone())
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
            } => match tokio::runtime::Handle::try_current() {
                Ok(handle) => match handle.block_on(self.backend.revoke_permission_rule(rule_id)) {
                    Ok(_) => {
                        self.flash_success(self.i18n.text_args(
                            "flash-permission-rule-revoked",
                            &crate::fl_args!("name" => label),
                        ));
                        self.open_permission_rule_picker(return_query.as_str());
                    }
                    Err(error) => self.flash_error(error.to_string()),
                },
                Err(error) => self.flash_error(error.to_string()),
            },
            ConfirmAction::ExitWorktree {
                session_id,
                discard_changes,
            } => match self
                .backend
                .exit_worktree(session_id, "remove".to_string(), discard_changes)
            {
                Ok(agena::message::FirstPartyToolOutput::ExitWorktree { action, path }) => {
                    self.flash_success(format!("worktree {action}: {path}"));
                }
                Ok(_) => self.flash_success("worktree exited".to_string()),
                Err(error) => self.flash_error(error.to_string()),
            },
        }
    }

    fn execute_command(&mut self, spec: &'static CommandSpec, args: &str) {
        match spec.id {
            CommandId::Help => self.overlay = Some(Overlay::Help),
            CommandId::Commands => self.open_command_palette(),
            CommandId::New => self.create_session(None),
            CommandId::Sessions => self.handle_sessions_command(spec, args),
            CommandId::Resume => self.open_resume_session_picker(),
            CommandId::Lineage => self.open_lineage_picker(),
            CommandId::Rewind => self.open_rewind_messages_picker(),
            CommandId::Search => {
                self.focus = Focus::Sessions;
                if args.trim().is_empty() {
                    self.overlay = Some(Overlay::SessionSearch(LineInputOverlay {
                        title: ui_text::t(&self.i18n, "overlay-session-search-title"),
                        prompt: ui_text::t(&self.i18n, "overlay-session-search-prompt"),
                        input: Editor::from_text(self.sessions.search_query.clone()),
                    }));
                } else {
                    self.sessions.search_query = args.trim().to_string();
                    self.refresh_sessions_after_query_change();
                }
            }
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
            CommandId::Mcp => self.handle_mcp_command(args),
            CommandId::Lsp => self.handle_lsp_command(args),
            CommandId::Skills => self.handle_skills_command(args),
            CommandId::Runtime => self.handle_runtime_command(args),
            CommandId::Cost => self.handle_cost_command(),
            CommandId::Review => self.handle_review_command(args),
            CommandId::Permissions => self.handle_permissions_command(args),
            CommandId::Config => self.handle_config_command(args),
            CommandId::Worktree => self.handle_worktree_command(args),
            CommandId::Git => self.handle_git_command(args),
            CommandId::Commit => self.handle_commit_command(args),
            CommandId::Pr => self.handle_pr_command(args),
            CommandId::Export => self.handle_export_command(args),
            CommandId::Memory => self.handle_memory_command(spec, args),
            CommandId::Pager => self.pending_ui_action = Some(UiAction::PageTranscript),
            CommandId::Continue => self.continue_current_session(),
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
            CommandId::CopyVisible => self.copy_visible_transcript(),
            CommandId::Providers => self.open_provider_picker(ProviderPickerPurpose::SetProvider),
            CommandId::Provider => self.handle_provider_command(args),
            CommandId::Models => self.handle_models_command(args),
            CommandId::Model => self.handle_model_command(args),
            CommandId::Temperature => self.handle_temperature_command(spec, args),
            CommandId::MaxOutput => self.handle_max_output_command(spec, args),
            CommandId::System => self.handle_system_command(spec, args),
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

    /// `/btw <question>` — fork a child session and submit the question
    /// there *without* touching the parent transcript. Mirrors Claude
    /// Code's "side question" affordance. The parent turn keeps running
    /// (or stays idle) untouched; the user can switch to the new session
    /// via the sessions pane to read the answer.
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

    fn handle_mcp_command(&mut self, args: &str) {
        self.open_inspector_picker(
            "MCP".to_string(),
            "Inspect configured MCP servers".to_string(),
            args.trim(),
            self.backend.mcp_inspector_rows(),
        );
    }

    fn handle_lsp_command(&mut self, args: &str) {
        self.open_inspector_picker(
            "LSP".to_string(),
            "Inspect configured LSP servers".to_string(),
            args.trim(),
            self.backend.lsp_inspector_rows(),
        );
    }

    fn handle_skills_command(&mut self, args: &str) {
        self.open_inspector_picker(
            "Skills".to_string(),
            "Inspect discovered skills".to_string(),
            args.trim(),
            self.backend.skills_inspector_rows(),
        );
    }

    fn handle_runtime_command(&mut self, args: &str) {
        self.open_inspector_picker(
            "Runtime".to_string(),
            "Inspect runtime summary".to_string(),
            args.trim(),
            self.backend.runtime_inspector_rows(),
        );
    }

    fn handle_cost_command(&mut self) {
        let Some(session_id) = self.current_or_selected_session_id() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                match handle.block_on(self.backend.session_cost_inspector_rows(session_id)) {
                    Ok(rows) => self.open_inspector_picker(
                        format!("Cost [#{}]", session_id),
                        "Inspect session usage and cost".to_string(),
                        "",
                        rows,
                    ),
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            Err(error) => self.flash_error(error.to_string()),
        }
    }

    fn handle_review_command(&mut self, args: &str) {
        self.execute_runtime_entry_prompt("review", args);
    }

    fn handle_permissions_command(&mut self, args: &str) {
        self.open_permission_rule_picker(args.trim());
    }

    fn handle_config_command(&mut self, args: &str) {
        self.open_inspector_picker(
            "Config".to_string(),
            "Inspect resolved config".to_string(),
            args.trim(),
            self.backend.config_inspector_rows(),
        );
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
                    Ok(agena::message::FirstPartyToolOutput::EnterWorktree { path, branch }) => {
                        self.flash_success(format!("worktree ready: {path} ({branch})"));
                    }
                    Ok(_) => self.flash_success("worktree entered".to_string()),
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
                    Ok(agena::message::FirstPartyToolOutput::EnterWorktree { path, branch }) => {
                        self.flash_success(format!("worktree attached: {path} ({branch})"));
                    }
                    Ok(_) => self.flash_success("worktree attached".to_string()),
                    Err(error) => self.flash_error(error.to_string()),
                }
            }
            "exit" | "leave" => {
                let exit_args = rest.trim();
                let (mode, extra) = split_command_args_once(exit_args).unwrap_or((exit_args, ""));
                match mode.to_ascii_lowercase().as_str() {
                    "" | "keep" => match self.backend.exit_worktree(session_id, "keep".to_string(), false) {
                        Ok(agena::message::FirstPartyToolOutput::ExitWorktree { action, path }) => {
                            self.flash_success(format!("worktree {action}: {path}"));
                        }
                        Ok(_) => self.flash_success("worktree exited".to_string()),
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

    fn handle_git_command(&mut self, args: &str) {
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => match handle.block_on(self.backend.git_inspector_rows()) {
                Ok(rows) => self.open_inspector_picker(
                    "Git".to_string(),
                    "Inspect git branch, diff, and worktree status".to_string(),
                    args.trim(),
                    rows,
                ),
                Err(error) => self.flash_error(error.to_string()),
            },
            Err(error) => self.flash_error(error.to_string()),
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

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => match handle.block_on(self.backend.create_commit(message.to_string())) {
                Ok((commit, summary)) => {
                    self.flash_success(format!(
                        "commit created: {} {}",
                        &commit[..commit.len().min(12)],
                        summary
                    ));
                }
                Err(error) => self.flash_error(error.to_string()),
            },
            Err(error) => self.flash_error(error.to_string()),
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

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => match handle.block_on(self.backend.create_pr(title, body, base, head)) {
                Ok(url) => self.flash_success(format!("pull request created: {url}")),
                Err(error) => self.flash_error(error.to_string()),
            },
            Err(error) => self.flash_error(error.to_string()),
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

    fn handle_sessions_command(&mut self, spec: &'static CommandSpec, args: &str) {
        self.focus = Focus::Sessions;
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.request_sessions(false);
            return;
        }

        let next_mode = match trimmed.to_ascii_lowercase().as_str() {
            "all" | "recent" => SessionViewMode::All,
            "roots" | "root" => SessionViewMode::Roots,
            "subtree" | "tree" | "branch" => SessionViewMode::Subtree,
            _ => {
                self.flash_warning(self.i18n.text_args(
                    "flash-command-usage",
                    &crate::fl_args!("usage" => spec.invocation()),
                ));
                return;
            }
        };
        self.set_session_view_mode(next_mode);
    }

    fn set_session_view_mode(&mut self, mode: SessionViewMode) {
        if mode == SessionViewMode::Subtree && self.current_or_selected_session_id().is_none() {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }
        self.sessions.view_mode = mode;
        self.focus = Focus::Sessions;
        self.flash_success(self.i18n.text_args(
            "flash-session-view-mode",
            &crate::fl_args!("mode" => self.current_session_view_summary()),
        ));
        self.request_sessions(false);
    }

    fn cycle_session_view_mode(&mut self) {
        self.set_session_view_mode(self.sessions.view_mode.next());
    }

    fn refresh_sessions_after_query_change(&mut self) {
        let preferred_id = self
            .sessions
            .current_selected_id()
            .or(self.transcript.session_id)
            .or(self.launch.initial_session_id);
        if self.sessions.initialized && !self.sessions.source_items.is_empty() {
            self.rebuild_visible_sessions(preferred_id);
        } else {
            self.request_sessions(false);
        }
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

    fn handle_provider_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.open_provider_picker(ProviderPickerPurpose::SetProvider);
            return;
        }
        if trimmed.eq_ignore_ascii_case("clear") {
            self.run_options.model = None;
            self.flash_success(ui_text::t(&self.i18n, "flash-provider-cleared"));
            return;
        }
        let Some(provider) = self
            .backend
            .list_providers()
            .into_iter()
            .find(|provider| provider.provider_id == trimmed)
        else {
            self.flash_error(self.i18n.text_args(
                "flash-provider-not-found",
                &crate::fl_args!("provider" => trimmed),
            ));
            return;
        };
        self.apply_provider_override(provider);
    }

    fn handle_models_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            if let Some(provider_id) = self.resolve_model_provider_for_picker() {
                self.open_model_picker(provider_id);
            } else {
                self.open_provider_picker(ProviderPickerPurpose::ChooseModelProvider);
            }
            return;
        }
        self.open_model_picker(trimmed.to_string());
    }

    fn handle_model_command(&mut self, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.handle_models_command("");
            return;
        }
        if trimmed.eq_ignore_ascii_case("clear") {
            self.run_options.model = None;
            self.flash_success(ui_text::t(&self.i18n, "flash-model-cleared"));
            return;
        }
        match self.resolve_model_argument(trimmed) {
            Ok(model) => self.apply_model_override(model),
            Err(error) => self.flash_error(error),
        }
    }

    fn handle_temperature_command(&mut self, spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => spec.invocation()),
            ));
            return;
        }
        if trimmed.eq_ignore_ascii_case("clear") {
            self.run_options.temperature = None;
            self.flash_success(ui_text::t(&self.i18n, "flash-temperature-cleared"));
            return;
        }
        match trimmed.parse::<f32>() {
            Ok(value) if value.is_finite() => {
                self.run_options.temperature = Some(value);
                self.flash_success(self.i18n.text_args(
                    "flash-temperature-set",
                    &crate::fl_args!("value" => format!("{value:.2}")),
                ));
            }
            _ => self.flash_error(ui_text::t(&self.i18n, "flash-temperature-invalid")),
        }
    }

    fn handle_max_output_command(&mut self, spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => spec.invocation()),
            ));
            return;
        }
        if trimmed.eq_ignore_ascii_case("clear") {
            self.run_options.max_output_tokens = None;
            self.flash_success(ui_text::t(&self.i18n, "flash-max-output-cleared"));
            return;
        }
        match trimmed.parse::<u32>() {
            Ok(value) if value > 0 => {
                self.run_options.max_output_tokens = Some(value);
                self.flash_success(self.i18n.text_args(
                    "flash-max-output-set",
                    &crate::fl_args!("value" => value as i64),
                ));
            }
            _ => self.flash_error(ui_text::t(&self.i18n, "flash-max-output-invalid")),
        }
    }

    fn handle_system_command(&mut self, spec: &'static CommandSpec, args: &str) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.flash_warning(self.i18n.text_args(
                "flash-command-usage",
                &crate::fl_args!("usage" => spec.invocation()),
            ));
            return;
        }
        if trimmed.eq_ignore_ascii_case("clear") {
            self.run_options.system = None;
            self.flash_success(ui_text::t(&self.i18n, "flash-system-cleared"));
            return;
        }
        self.run_options.system = Some(trimmed.to_string());
        self.flash_success(ui_text::t(&self.i18n, "flash-system-set"));
    }

    fn resolve_model_argument(&self, arg: &str) -> UiResult<ModelRef> {
        let trimmed = arg.trim();
        if trimmed.contains('/') {
            return self
                .backend
                .resolve_model_target(trimmed, None)
                .map_err(|error| error.to_string());
        }
        if let Some((target, model)) = split_command_args_once(trimmed) {
            return self
                .backend
                .resolve_model_target(target, Some(model))
                .map_err(|error| error.to_string());
        }
        let Some(provider_id) = self.resolve_model_provider_for_picker() else {
            return Err(ui_text::t(&self.i18n, "flash-model-provider-required"));
        };
        self.backend
            .resolve_model_target(provider_id.as_str(), Some(trimmed))
            .map_err(|error| error.to_string())
    }

    fn current_runtime_status_summary(&self) -> String {
        let mut parts = vec![
            self.run_options
                .summary()
                .unwrap_or_else(|| ui_text::t(&self.i18n, "runtime-status-default")),
        ];
        parts.extend(self.current_execution_context_parts());
        parts.push(format!(
            "queue_key={} submit_key={}",
            self.keybindings.queue.len(),
            self.keybindings.submit.len()
        ));
        parts.push(format!(
            "statusline={}",
            if self.backend.plugin_statusline_segments().is_empty() {
                "default"
            } else {
                "plugin"
            }
        ));
        if let Some(theme) = self.plugin_theme.as_ref() {
            parts.push(format!("theme={}", theme.id));
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
        let cwd = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "<unavailable>".to_owned());
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
        if let Some(overlay) = &mut self.overlay {
            match overlay {
                Overlay::SessionSearch(dialog)
                | Overlay::TranscriptSearch(dialog)
                | Overlay::SessionRename(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::PermissionRuleEdit(dialog) => {
                    dialog.input.flush_pending_input_if_due(now);
                }
                Overlay::FileAttach(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::UserInputReply(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::Picker(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::Timeline(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::PluginInspector(dialog) => dialog.input.flush_pending_input_if_due(now),
                Overlay::Confirm(_) => {}
                Overlay::Permission(_) => {}
                Overlay::Help => {}
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
                "Use the session pane or start typing in the composer to create one.",
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

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let composer_height = self.composer_height();
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(composer_height),
                Constraint::Length(1),
            ])
            .split(area);

        let main = vertical[0];
        let composer = vertical[1];
        let status = vertical[2];

        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Min(32)])
            .split(main);

        let transcript = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(4)])
            .split(horizontal[1]);

        self.layout = LayoutCache {
            transcript_body: inner_rect(transcript[1]),
        };

        self.transcript.clamp_scroll(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
        );

        self.render_sessions(frame, horizontal[0]);
        self.render_transcript_header(frame, transcript[0]);
        self.render_transcript(frame, transcript[1]);
        self.render_composer(frame, composer);
        self.render_status(frame, status);
        self.render_overlay(frame, area);
    }

    fn render_sessions(&mut self, frame: &mut Frame, area: Rect) {
        let title = ui_text::sessions_title(
            &self.i18n,
            self.current_session_view_summary().as_str(),
            self.sessions.search_query.as_str(),
        );
        let current_session_id = self.transcript.session_id;
        let current_parent_id = self.current_parent_session_id();
        let session_depths = session_depth_map(self.sessions.items.as_slice());

        if self.sessions.items.is_empty() && self.sessions.initialized {
            let empty = Paragraph::new(ui_text::t(&self.i18n, "sessions-empty"))
                .block(Block::default().title(title).borders(Borders::ALL))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }

        let mut items = self
            .sessions
            .items
            .iter()
            .map(|session| {
                let is_open = self.transcript.session_id == Some(session.id);
                let lineage_relation = self
                    .current_lineage_item(session.id)
                    .map(|item| item.relation);
                let is_current_child =
                    current_session_id.is_some_and(|id| session.parent_id == Some(id));
                let is_current_parent = current_parent_id == Some(session.id);
                let depth = session_depths.get(&session.id).copied().unwrap_or_default();
                let mut title_style = Style::default();
                if is_open {
                    title_style = title_style.fg(Color::Cyan).add_modifier(Modifier::BOLD);
                }

                let mut title_spans = vec![Span::styled(
                    format!(
                        "{}{}",
                        "  ".repeat(depth),
                        if depth == 0 { "◆ " } else { "↳ " }
                    ),
                    Style::default().fg(Color::DarkGray),
                )];
                title_spans.push(Span::styled(session.title.clone(), title_style));
                if is_open {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-current")),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                if is_current_parent {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-parent")),
                        Style::default().fg(Color::Yellow),
                    ));
                } else if matches!(lineage_relation, Some(LineageRelation::Ancestor)) {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-ancestor")),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                if is_current_child || matches!(lineage_relation, Some(LineageRelation::Child)) {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-child")),
                        Style::default().fg(Color::Green),
                    ));
                } else if matches!(lineage_relation, Some(LineageRelation::Sibling)) {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-sibling")),
                        Style::default().fg(Color::Blue),
                    ));
                }
                if session.child_session_count > 0 {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!(
                            "[{}]",
                            self.i18n.text_args(
                                "session-summary-children",
                                &crate::fl_args!("count" => session.child_session_count as i64),
                            )
                        ),
                        Style::default().fg(Color::DarkGray),
                    ));
                }

                let meta = ui_text::session_meta(
                    &self.i18n,
                    session.id,
                    session.message_count,
                    session.updated_at,
                );
                let mut meta_parts = vec![meta];
                if let Some(parent_id) = session.parent_id {
                    meta_parts.push(self.i18n.text_args(
                        "session-summary-parent",
                        &crate::fl_args!("id" => parent_id),
                    ));
                }
                ListItem::new(vec![
                    Line::from(title_spans),
                    Line::from(Span::styled(
                        meta_parts.join(" | "),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
            })
            .collect::<Vec<_>>();

        if self.sessions.loading_more {
            items.push(ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "sessions-loading-more"),
                Style::default().fg(Color::DarkGray),
            ))));
        } else if self.sessions.has_more {
            items.push(ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "sessions-more"),
                Style::default().fg(Color::DarkGray),
            ))));
        }

        let list = List::new(items)
            .block(Block::default().title(title).borders(Borders::ALL))
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(32, 46, 64))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(">> ");

        let mut state = ListState::default();
        state.select(self.sessions.selection_for_render());
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn render_transcript_header(&mut self, frame: &mut Frame, area: Rect) {
        let is_running = self.transcript.execution.as_ref().is_some_and(|execution| {
            execution.run_state != SessionRunState::Idle || execution.blocked
        });
        let title = ui_text::transcript_header_title(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
            is_running,
        );
        let mut top_right = Vec::new();
        if self.transcript.submitting {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-busy"));
        }
        if self.transcript.loading_initial {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-loading"));
        } else if self.transcript.loading_older {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-loading-older"));
        }

        let mut bottom_left = Vec::new();
        if let Some(execution) = self.transcript.execution.as_ref() {
            bottom_left.push(ui_text::session_meta(
                &self.i18n,
                execution.session.id,
                execution.session.message_count,
                execution.session.updated_at,
            ));
            if let Some(parent_id) = execution.session.parent_id {
                bottom_left.push(self.i18n.text_args(
                    "session-summary-parent",
                    &crate::fl_args!("id" => parent_id),
                ));
            }
            if execution.session.child_session_count > 0 {
                bottom_left.push(self.i18n.text_args(
                    "session-summary-children",
                    &crate::fl_args!("count" => execution.session.child_session_count as i64),
                ));
            }
        }
        bottom_left.extend(self.current_lineage_context_parts());
        bottom_left.extend(self.current_execution_context_parts());
        bottom_left.push(self.current_session_view_summary());
        if let Some(summary) = self.run_options.summary() {
            bottom_left.push(summary);
        }

        let mut bottom_right = Vec::new();
        let total_lines = self
            .transcript
            .rendered(self.layout.transcript_body.width.max(1))
            .lines
            .len();
        if total_lines > 0 {
            let first_line = min(self.transcript.scroll.saturating_add(1), total_lines);
            let last_line = min(
                self.transcript
                    .scroll
                    .saturating_add(self.layout.transcript_body.height.max(1) as usize),
                total_lines,
            );
            let percent = ((last_line as f64 / total_lines as f64) * 100.0).round() as u16;
            bottom_right.push(ui_text::transcript_lines_summary(
                &self.i18n,
                first_line,
                last_line,
                total_lines,
                percent,
            ));
        }
        if self.transcript.follow_tail {
            bottom_right.push(ui_text::t(&self.i18n, "transcript-header-tail"));
        }
        if !self.transcript.search_query.trim().is_empty() {
            bottom_right.push(ui_text::transcript_search_summary(
                &self.i18n,
                self.transcript.search_query.as_str(),
                self.transcript.current_search_match_number(),
                self.transcript.current_search_match_count(),
            ));
        }

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(inner);
        self.render_header_row(
            frame,
            rows[0],
            title,
            top_right.join(" | "),
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        );
        self.render_header_row(
            frame,
            rows[1],
            bottom_left.join(" | "),
            bottom_right.join(" | "),
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::DarkGray),
        );
    }

    fn render_transcript(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(ui_text::transcript_panel_title(&self.i18n))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let lines = if self.transcript.session_id.is_none() {
            vec![
                Line::from(ui_text::t(&self.i18n, "no-session-selected")),
                Line::from(ui_text::t(&self.i18n, "no-session-selected-hint")),
            ]
        } else {
            let rendered = self.transcript.rendered(inner.width).clone();
            let matches = rendered.search_matches.clone();
            let active_match = self
                .transcript
                .search_match_index
                .and_then(|index| matches.get(index).copied());
            rendered
                .lines
                .iter()
                .enumerate()
                .map(|(idx, line)| {
                    let line_is_active = active_match == Some(idx);
                    let line_has_match = matches.contains(&idx);
                    highlight_search_line(
                        line.text.as_str(),
                        line.style,
                        self.transcript.search_query.as_str(),
                        line_is_active,
                        line_has_match,
                    )
                })
                .collect::<Vec<_>>()
        };

        let paragraph = Paragraph::new(Text::from(lines))
            .scroll((min(self.transcript.scroll, u16::MAX as usize) as u16, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    }

    fn render_composer(&self, frame: &mut Frame, area: Rect) {
        let mut title = ui_text::composer_title(&self.i18n, self.transcript.session_id);
        if let Some(summary) = self.run_options.summary() {
            title = format!("{title}[{summary}] ");
        }
        // Queue indicator in the composer title — `· N queued · preview…`
        // (mirrors Claude Code's `· N queued`). Only shown when non-empty.
        if !self.queue.is_empty() {
            let preview = self.queue.first_preview(40).unwrap_or_default();
            if preview.is_empty() {
                title = format!("{title}· {} queued ", self.queue.len());
            } else {
                title = format!("{title}· {} queued · {preview} ", self.queue.len());
            }
        }
        if self.transcript.submitting {
            title = format!("{title}· esc to interrupt ");
        }
        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let view = self.composer.render_view(inner.width, inner.height);
        let content = if self.composer.text().is_empty() {
            Text::from(Line::from(Span::styled(
                ui_text::t(&self.i18n, "composer-placeholder"),
                Style::default().fg(Color::DarkGray),
            )))
        } else {
            Text::from(view.lines.clone())
        };

        frame.render_widget(Paragraph::new(content), inner);

        if self.overlay.is_none() && self.focus == Focus::Composer {
            frame.set_cursor_position((
                inner.x.saturating_add(view.cursor_x),
                inner.y.saturating_add(view.cursor_y),
            ));
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        if let Some(flash) = &self.flash {
            let style = match flash.level {
                FlashLevel::Success => {
                    Style::default().fg(self.theme_color("flash_success", Color::Green))
                }
                FlashLevel::Warning => {
                    Style::default().fg(self.theme_color("flash_warning", Color::Yellow))
                }
                FlashLevel::Error => {
                    Style::default().fg(self.theme_color("flash_error", Color::Red))
                }
                FlashLevel::Info => {
                    Style::default().fg(self.theme_color("flash_info", Color::Cyan))
                }
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(flash.text.clone(), style))),
                area,
            );
            return;
        }

        let base_style = Style::default().fg(self.theme_color("status", Color::DarkGray));
        let mut spans = Vec::new();
        let text = if let Some(text) = self
            .status_line
            .as_ref()
            .and_then(|status_line| status_line.text.clone())
        {
            text
        } else {
            let default_hint = match self.focus {
                Focus::Sessions => ui_text::t(&self.i18n, "status-sessions"),
                Focus::Transcript => ui_text::t(&self.i18n, "status-transcript"),
                Focus::Composer => ui_text::t(&self.i18n, "status-composer"),
            };
            self.status_context_summary()
                .map(|context| format!("{context}  |  {default_hint}"))
                .unwrap_or(default_hint)
        };
        spans.push(Span::styled(text, base_style));

        for segment in self.backend.plugin_statusline_segments() {
            if segment.content.trim().is_empty() {
                continue;
            }
            spans.push(Span::styled("  |  ", base_style));
            let style = segment
                .color
                .as_deref()
                .and_then(parse_tui_color)
                .map(|color| Style::default().fg(color))
                .unwrap_or(base_style);
            spans.push(Span::styled(segment.content, style));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn theme_color(&self, key: &str, fallback: Color) -> Color {
        self.plugin_theme
            .as_ref()
            .and_then(|theme| theme.colors.get(key))
            .and_then(|value| parse_tui_color(value))
            .unwrap_or(fallback)
    }

    fn render_header_row(
        &self,
        frame: &mut Frame,
        area: Rect,
        left: String,
        right: String,
        left_style: Style,
        right_style: Style,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        if right.trim().is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(left, left_style))),
                area,
            );
            return;
        }

        let right_width = UnicodeWidthStr::width(right.as_str()).saturating_add(1) as u16;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(min(area.width, right_width)),
            ])
            .split(area);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(left, left_style))),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(right, right_style)))
                .alignment(Alignment::Right),
            columns[1],
        );
    }

    fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        let Some(overlay) = &self.overlay else {
            return;
        };

        match overlay {
            Overlay::Help => {
                let area = centered_rect(area, 92, 36);
                frame.render_widget(Clear, area);
                let help_lines = ui_text::help_lines(&self.i18n);
                let text = help_lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if index == 0 {
                            Line::from(Span::styled(
                                value,
                                Style::default().add_modifier(Modifier::BOLD),
                            ))
                        } else {
                            Line::from(value)
                        }
                    })
                    .collect::<Vec<_>>();
                let widget = Paragraph::new(Text::from(text))
                    .block(
                        Block::default()
                            .title(format!(" {} ", ui_text::t(&self.i18n, "help-title")))
                            .borders(Borders::ALL),
                    )
                    .wrap(Wrap { trim: false });
                frame.render_widget(widget, area);
            }
            Overlay::SessionSearch(dialog)
            | Overlay::TranscriptSearch(dialog)
            | Overlay::SessionRename(dialog) => {
                self.render_line_overlay(frame, area, dialog);
            }
            Overlay::PermissionRuleEdit(dialog) => {
                self.render_permission_rule_edit_overlay(frame, area, dialog);
            }
            Overlay::FileAttach(dialog) => {
                self.render_file_attach_overlay(frame, area, dialog);
            }
            Overlay::Permission(dialog) => {
                self.render_permission_overlay(frame, area, dialog);
            }
            Overlay::UserInputReply(dialog) => {
                self.render_user_input_overlay(frame, area, dialog);
            }
            Overlay::Confirm(dialog) => {
                self.render_confirm_overlay(frame, area, dialog);
            }
            Overlay::Picker(dialog) => {
                self.render_picker_overlay(frame, area, dialog);
            }
            Overlay::Timeline(dialog) => {
                self.render_timeline_overlay(frame, area, dialog);
            }
            Overlay::PluginInspector(dialog) => {
                self.render_plugin_inspector_overlay(frame, area, dialog);
            }
        }
    }

    fn render_line_overlay(&self, frame: &mut Frame, area: Rect, dialog: &LineInputOverlay) {
        let area = centered_rect(area, 70, 7);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);
        let view = dialog.input.render_view(rows[1].width, 1);
        frame.render_widget(
            Paragraph::new(Text::from(view.lines.clone()))
                .block(Block::default().borders(Borders::BOTTOM)),
            rows[1],
        );
        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-line-footer")),
            rows[2],
        );
        frame.set_cursor_position((
            rows[1].x.saturating_add(view.cursor_x),
            rows[1].y.saturating_add(view.cursor_y),
        ));
    }

    fn render_permission_rule_edit_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionRuleEditOverlay,
    ) {
        let area = centered_rect(area, 82, 11);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(2),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);
        frame.render_widget(
            Paragraph::new(permission_rule_edit_help())
                .block(Block::default().borders(Borders::BOTTOM))
                .wrap(Wrap { trim: false }),
            rows[1],
        );
        let input_view = dialog.input.render_view(rows[2].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::BOTTOM)),
            rows[2],
        );
        frame.render_widget(
            Paragraph::new(render_permission_rule_preview(dialog.input.text()))
                .wrap(Wrap { trim: false }),
            rows[3],
        );
        frame.set_cursor_position((
            rows[2].x.saturating_add(input_view.cursor_x),
            rows[2].y.saturating_add(input_view.cursor_y),
        ));
    }

    fn render_file_attach_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &FileAttachOverlay,
    ) {
        let area = centered_rect(area, 88, 18);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-attach-title")
            ))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-attach-prompt")),
            rows[0],
        );

        let input_view = dialog.input.render_view(rows[1].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );

        let result_items = if dialog.results.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-attach-no-match"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .results
                .iter()
                .map(|path| ListItem::new(path.to_string_lossy().to_string()))
                .collect::<Vec<_>>()
        };
        let list = List::new(result_items)
            .block(Block::default().borders(Borders::ALL).title(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-attach-matches")
            )))
            .highlight_style(Style::default().bg(Color::Rgb(32, 46, 64)))
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.results.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, rows[2], &mut state);

        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-attach-footer")),
            rows[3],
        );

        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[1]
                .y
                .saturating_add(1)
                .saturating_add(input_view.cursor_y),
        ));
    }

    fn render_permission_overlay(&self, frame: &mut Frame, area: Rect, dialog: &PermissionOverlay) {
        let area = centered_rect(area, 84, 15);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-permission-title")
            ))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(7),
                Constraint::Length(4),
                Constraint::Length(1),
            ])
            .split(inner);

        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            self.i18n.text_args(
                "overlay-permission-request-id",
                &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
            ),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(permission_action_label(
            &self.i18n,
            &dialog.request.action,
        )));
        lines.push(Line::from(self.i18n.text_args(
            "overlay-permission-reason",
            &crate::fl_args!("reason" => dialog.request.reason.clone()),
        )));
        if !dialog.request.explanation.trim().is_empty() {
            lines.push(Line::from(format!(
                "Explanation: {}",
                dialog.request.explanation
            )));
        }
        let mut facts = Vec::new();
        if let Some(source) = dialog.request.source.as_deref() {
            facts.push(format!("source={source}"));
        }
        if let Some(scope) = dialog.request.scope {
            facts.push(format!("scope={scope}"));
        }
        if let Some(operator) = dialog.request.operator.as_deref() {
            facts.push(format!("operator={operator}"));
        }
        if !facts.is_empty() {
            lines.push(Line::from(facts.join(" · ")));
        }
        if let Some(session_id) = dialog.request.session_id {
            lines.push(Line::from(self.i18n.text_args(
                "overlay-permission-session",
                &crate::fl_args!("session" => session_id),
            )));
        }

        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            rows[0],
        );

        let choices = permission_overlay_choices(&self.i18n);
        let items = choices
            .iter()
            .map(|label| ListItem::new(label.clone()))
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::Rgb(32, 46, 64)))
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select(Some(dialog.selected));
        frame.render_stateful_widget(list, rows[1], &mut state);

        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-permission-footer")),
            rows[2],
        );
    }

    fn render_user_input_overlay(&self, frame: &mut Frame, area: Rect, dialog: &UserInputOverlay) {
        let height = min(18, area.height.saturating_sub(4));
        let area = centered_rect(area, 84, height);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-user-input-title")
            ))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(inner);

        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            self.i18n.text_args(
                "overlay-user-input-request-id",
                &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
            ),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        for question in &dialog.request.questions {
            lines.push(Line::from(Span::styled(
                format!("{} ({})", question.question, question.id),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for option in &question.options {
                let mut text = format!("  - {}", option.label);
                if !option.description.trim().is_empty() {
                    text.push_str(format!(" | {}", option.description).as_str());
                }
                lines.push(Line::from(text));
            }
            if question.allow_custom {
                lines.push(Line::from(format!(
                    "  - {}",
                    ui_text::t(&self.i18n, "overlay-user-input-custom-allowed")
                )));
            }
            lines.push(Line::from(""));
        }
        lines.push(Line::from(ui_text::t(
            &self.i18n,
            "overlay-user-input-reply-format",
        )));
        lines.push(Line::from(ui_text::t(
            &self.i18n,
            "overlay-user-input-cancel-hint",
        )));

        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            rows[0],
        );

        let view = dialog.input.render_view(rows[1].width, rows[1].height);
        frame.render_widget(
            Paragraph::new(Text::from(view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );
        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-user-input-footer")),
            rows[2],
        );
        frame.set_cursor_position((
            rows[1].x.saturating_add(1).saturating_add(view.cursor_x),
            rows[1].y.saturating_add(1).saturating_add(view.cursor_y),
        ));
    }

    fn render_confirm_overlay(&self, frame: &mut Frame, area: Rect, dialog: &ConfirmOverlay) {
        let body_height = dialog.body_lines.len() as u16;
        let area = centered_rect(area, 76, max(8, body_height.saturating_add(4)));
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(body_height), Constraint::Length(1)])
            .split(inner);

        let body = dialog
            .body_lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(line.clone())
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Text::from(body)).wrap(Wrap { trim: false }),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(dialog.footer.clone()).alignment(Alignment::Right),
            rows[1],
        );
    }

    fn render_picker_overlay(&self, frame: &mut Frame, area: Rect, dialog: &PickerOverlay) {
        let area = centered_rect(area, 88, 18);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);

        let input_view = dialog.input.render_view(rows[1].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );

        let result_items = if dialog.loading {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-picker-loading"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else if dialog.items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                dialog.empty_message.clone(),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|item| {
                    ListItem::new(vec![
                        Line::from(item.label.clone()),
                        Line::from(Span::styled(
                            item.detail.clone(),
                            Style::default().fg(Color::DarkGray),
                        )),
                    ])
                })
                .collect::<Vec<_>>()
        };

        let list = List::new(result_items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(Style::default().bg(Color::Rgb(32, 46, 64)))
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.loading && !dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, rows[2], &mut state);

        frame.render_widget(Paragraph::new(dialog.footer.clone()), rows[3]);
        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[1]
                .y
                .saturating_add(1)
                .saturating_add(input_view.cursor_y),
        ));
    }

    fn render_timeline_overlay(&self, frame: &mut Frame, area: Rect, dialog: &TimelineOverlay) {
        let area = centered_rect(area, 94, 24);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);

        let input_view = dialog.input.render_view(rows[1].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(rows[2]);

        let list_items = if dialog.loading {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-picker-loading"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else if dialog.items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                dialog.empty_message.clone(),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|item| ListItem::new(item.summary.clone()))
                .collect::<Vec<_>>()
        };
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-timeline-events")
                    ))
                    .borders(Borders::ALL),
            )
            .highlight_style(Style::default().bg(Color::Rgb(32, 46, 64)))
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.loading && !dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, content[0], &mut state);

        let detail = if dialog.loading {
            ui_text::t(&self.i18n, "overlay-picker-loading")
        } else {
            dialog
                .items
                .get(dialog.selected)
                .map(|item| item.detail.clone())
                .unwrap_or_else(|| dialog.empty_message.clone())
        };
        frame.render_widget(
            Paragraph::new(detail)
                .block(
                    Block::default()
                        .title(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-timeline-detail")
                        ))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            content[1],
        );

        frame.render_widget(Paragraph::new(dialog.footer.clone()), rows[3]);
        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[1]
                .y
                .saturating_add(1)
                .saturating_add(input_view.cursor_y),
        ));
    }

    fn render_plugin_inspector_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PluginInspectorOverlay,
    ) {
        let area = centered_rect(area, 96, 28);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(12),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);

        let input_view = dialog.input.render_view(rows[1].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(rows[2]);
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(content[1]);

        let list_items = if dialog.items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                dialog.empty_message.clone(),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|item| {
                    let style = match item.state {
                        agena::plugin::status::PluginRunState::Running => {
                            Style::default().fg(Color::Green)
                        }
                        agena::plugin::status::PluginRunState::Restarting => {
                            Style::default().fg(Color::Yellow)
                        }
                        agena::plugin::status::PluginRunState::Failed => {
                            Style::default().fg(Color::Red)
                        }
                        agena::plugin::status::PluginRunState::Stopped => {
                            Style::default().fg(Color::DarkGray)
                        }
                    };
                    ListItem::new(Line::from(Span::styled(item.summary.clone(), style)))
                })
                .collect::<Vec<_>>()
        };
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-plugins-list")
                    ))
                    .borders(Borders::ALL),
            )
            .highlight_style(Style::default().bg(Color::Rgb(32, 46, 64)))
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, content[0], &mut state);

        let detail = dialog
            .items
            .get(dialog.selected)
            .map(|item| item.detail.clone())
            .unwrap_or_else(|| dialog.empty_message.clone());
        frame.render_widget(
            Paragraph::new(detail)
                .block(
                    Block::default()
                        .title(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-plugins-detail")
                        ))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right[0],
        );

        let logs = dialog
            .items
            .get(dialog.selected)
            .map(|item| item.logs.clone())
            .unwrap_or_else(|| dialog.empty_message.clone());
        frame.render_widget(
            Paragraph::new(logs)
                .block(
                    Block::default()
                        .title(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-plugins-logs")
                        ))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right[1],
        );

        frame.render_widget(Paragraph::new(dialog.footer.clone()), rows[3]);
        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[1]
                .y
                .saturating_add(1)
                .saturating_add(input_view.cursor_y),
        ));
    }

    fn composer_height(&self) -> u16 {
        let line_count = max(1, self.composer.logical_line_count());
        min(12, line_count as u16 + 2)
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
    SessionSearch,
    TranscriptSearch,
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

    fn selection_for_render(&self) -> Option<usize> {
        (!self.items.is_empty()).then_some(self.selected)
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
        merged.extend(page.items);
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

fn render_message(message: &MessageResource, width: u16, i18n: &I18n) -> Vec<RenderedLine> {
    let mut lines = Vec::new();
    let role_style = style_for_role(message.role);
    lines.push(RenderedLine {
        text: format!(
            "[{} | {} | {}]",
            ui_text::role_label(i18n, message.role),
            format_timestamp(message.created_at),
            ui_text::message_state_label(i18n, message.state)
        ),
        style: role_style.add_modifier(Modifier::BOLD),
    });

    if let Some(parts) = &message.parts {
        for part in parts {
            render_part(part, width, &mut lines, i18n);
        }
    } else {
        lines.push(RenderedLine::dim(ui_text::message_parts_not_loaded(
            i18n,
            message.part_count as usize,
        )));
    }

    if let Some(usage) = &message.usage {
        lines.push(RenderedLine {
            text: ui_text::message_usage(
                i18n,
                usage.input_tokens,
                usage.output_tokens,
                usage.reasoning_tokens,
            ),
            style: Style::default().fg(Color::DarkGray),
        });
    }

    if let Some(finish) = &message.finish
        && !finish.trim().is_empty()
    {
        lines.push(RenderedLine {
            text: ui_text::message_finish(i18n, finish),
            style: Style::default().fg(Color::DarkGray),
        });
    }

    if message.parts.as_ref().is_none_or(Vec::is_empty) {
        lines.push(RenderedLine::dim(format!(
            "  {}",
            ui_text::t(i18n, "message-empty")
        )));
    }

    lines
        .into_iter()
        .flat_map(|line| wrap_rendered_line(line, width))
        .collect::<Vec<_>>()
}

fn render_part(part: &MessagePart, width: u16, out: &mut Vec<RenderedLine>, i18n: &I18n) {
    let prefix = "  ";
    match part.content.as_ref() {
        Some(PartContent::Text(text)) => {
            push_multiline(out, prefix, text.text.as_str(), Style::default(), width)
        }
        Some(PartContent::Reasoning(reasoning)) => {
            let summary = if !reasoning.summary.is_empty() {
                reasoning.summary.join(" ")
            } else {
                reasoning.raw_content.join(" ")
            };
            push_multiline(
                out,
                prefix,
                i18n.text_args("message-thinking", &crate::fl_args!("summary" => summary))
                    .as_str(),
                Style::default().fg(Color::DarkGray),
                width,
            );
        }
        Some(PartContent::ToolExecution(tool)) => render_tool_execution(tool, out, width, i18n),
        Some(PartContent::CommandExecution(command)) => {
            out.push(RenderedLine {
                text: format!("{prefix}$ {}", command.command),
                style: Style::default().fg(Color::Yellow),
            });
            if let Some(output) = &command.output
                && !output.trim().is_empty()
            {
                push_multiline(out, "    ", output, Style::default().fg(Color::Gray), width);
            }
            out.push(RenderedLine::dim(format!(
                "    {}",
                i18n.text_args(
                    "message-command-status",
                    &crate::fl_args!(
                        "status" => ui_text::execution_status_label(i18n, command.status),
                        "exit" => command.exit_code.unwrap_or(-1),
                    ),
                )
            )));
        }
        Some(PartContent::FileChange(change)) => {
            out.push(RenderedLine {
                text: format!("{prefix}{}", ui_text::t(i18n, "message-file-changes")),
                style: Style::default().fg(Color::Magenta),
            });
            for entry in &change.changes {
                let path = if entry.kind == FileChangeKind::Moved {
                    entry
                        .from_path
                        .as_ref()
                        .map(|from_path| format!("{from_path} -> {}", entry.path))
                        .unwrap_or_else(|| entry.path.clone())
                } else {
                    entry.path.clone()
                };
                out.push(RenderedLine::plain(format!(
                    "    - {} ({})",
                    path,
                    ui_text::file_change_kind_label(i18n, entry.kind)
                )));
            }
        }
        Some(PartContent::WebSearch(search)) => {
            out.push(RenderedLine {
                text: format!(
                    "{prefix}{}",
                    i18n.text_args(
                        "message-search",
                        &crate::fl_args!("query" => search.query.as_str())
                    )
                ),
                style: Style::default().fg(Color::Cyan),
            });
            for result in &search.results {
                out.push(RenderedLine::plain(format!("    - {}", result.title)));
                out.push(RenderedLine::dim(format!("      {}", result.url)));
                if let Some(snippet) = &result.snippet
                    && !snippet.trim().is_empty()
                {
                    push_multiline(out, "      ", snippet, Style::default(), width);
                }
            }
        }
        Some(PartContent::TodoList(todo)) => {
            out.push(RenderedLine {
                text: format!("{prefix}{}", ui_text::t(i18n, "message-todo-list")),
                style: Style::default().fg(Color::Blue),
            });
            for item in &todo.items {
                out.push(RenderedLine::plain(format!(
                    "    - [{}|{}] {}",
                    ui_text::todo_status_label(i18n, item.status),
                    ui_text::todo_priority_label(i18n, item.priority),
                    item.content
                )));
            }
        }
        Some(PartContent::Error(error)) => {
            out.push(RenderedLine {
                text: format!(
                    "{prefix}{}",
                    i18n.text_args(
                        "message-error",
                        &crate::fl_args!(
                            "code" => error.code.as_str(),
                            "message" => error.message.as_str(),
                        ),
                    )
                ),
                style: Style::default().fg(Color::Red),
            });
        }
        Some(PartContent::Attachment(attachment)) => {
            out.push(RenderedLine {
                text: format!("{prefix}{}", ui_text::t(i18n, "message-attachments")),
                style: Style::default().fg(Color::Magenta),
            });
            for item in &attachment.attachments {
                let label = item
                    .title
                    .as_ref()
                    .or(item.filename.as_ref())
                    .cloned()
                    .unwrap_or_else(|| item.mime.clone());
                out.push(RenderedLine::plain(format!("    - {label}")));
            }
        }
        Some(PartContent::PermissionRequest(permission)) => {
            push_multiline(
                out,
                prefix,
                ui_text::permission_summary(i18n, permission).as_str(),
                Style::default().fg(Color::Yellow),
                width,
            );
        }
        Some(PartContent::UserInputRequest(request)) => {
            out.push(RenderedLine {
                text: format!(
                    "{prefix}{}",
                    i18n.text_args(
                        "message-awaiting-user-input",
                        &crate::fl_args!("request_id" => request.request.request_id.as_str()),
                    )
                ),
                style: Style::default().fg(Color::Yellow),
            });
            for question in &request.request.questions {
                out.push(RenderedLine::plain(ui_text::message_question_line(
                    i18n,
                    question.question.as_str(),
                    question.id.as_str(),
                )));
            }
        }
        None => {
            let fallback = part
                .summary
                .clone()
                .unwrap_or_else(|| ui_text::t(i18n, "message-part-detail-unavailable"));
            push_multiline(
                out,
                prefix,
                fallback.as_str(),
                Style::default().fg(Color::DarkGray),
                width,
            );
        }
    }
}

fn render_tool_execution(
    tool: &ToolExecutionPart,
    out: &mut Vec<RenderedLine>,
    width: u16,
    i18n: &I18n,
) {
    match tool {
        ToolExecutionPart::Pending {
            invocation, title, ..
        } => {
            let label = if title.trim().is_empty() {
                tool_invocation_label(invocation)
            } else {
                title.clone()
            };
            out.push(RenderedLine {
                text: format!(
                    "  {}",
                    i18n.text_args("message-tool-pending", &crate::fl_args!("label" => label))
                ),
                style: Style::default().fg(Color::Yellow),
            });
        }
        ToolExecutionPart::InProgress {
            invocation,
            title,
            output_text,
            ..
        } => {
            let label = if title.trim().is_empty() {
                tool_invocation_label(invocation)
            } else {
                title.clone()
            };
            out.push(RenderedLine {
                text: format!(
                    "  {}",
                    i18n.text_args("message-tool-running", &crate::fl_args!("label" => label))
                ),
                style: Style::default().fg(Color::Yellow),
            });
            if !output_text.trim().is_empty() {
                push_tool_output_preview(out, "    ", output_text, Style::default(), width, i18n);
            }
        }
        ToolExecutionPart::Completed {
            invocation,
            output_text,
            blocks,
            details,
            ..
        } => {
            out.push(RenderedLine {
                text: format!(
                    "  {}",
                    i18n.text_args(
                        "message-tool-done",
                        &crate::fl_args!("label" => tool_invocation_label(invocation)),
                    )
                ),
                style: Style::default().fg(Color::Green),
            });
            if !output_text.trim().is_empty() {
                push_tool_output_preview(out, "    ", output_text, Style::default(), width, i18n);
            }
            if let Some(diff) = apply_patch_diff(details) {
                out.push(RenderedLine::dim(format!(
                    "    diff ({} lines)",
                    diff.lines().count()
                )));
                push_tool_output_preview(
                    out,
                    "    ",
                    diff.as_str(),
                    Style::default().fg(Color::DarkGray),
                    width,
                    i18n,
                );
            }
            if !blocks.is_empty() {
                out.push(RenderedLine::dim(ui_text::message_tool_result_blocks(
                    i18n,
                    blocks.len(),
                )));
            }
        }
        ToolExecutionPart::Failed {
            invocation,
            error_message,
            output_text,
            ..
        } => {
            out.push(RenderedLine {
                text: format!(
                    "  {}",
                    i18n.text_args(
                        "message-tool-failed",
                        &crate::fl_args!("label" => tool_invocation_label(invocation)),
                    )
                ),
                style: Style::default().fg(Color::Red),
            });
            if !error_message.trim().is_empty() {
                push_multiline(
                    out,
                    "    ",
                    error_message,
                    Style::default().fg(Color::Red),
                    width,
                );
            }
            if !output_text.trim().is_empty() {
                push_tool_output_preview(out, "    ", output_text, Style::default(), width, i18n);
            }
        }
    }
}

fn apply_patch_diff(details: &agena::message::ToolOutput) -> Option<String> {
    match details.as_first_party()? {
        FirstPartyToolOutput::ApplyPatch { diff, .. } if !diff.trim().is_empty() => Some(diff),
        _ => None,
    }
}

fn tool_invocation_label(invocation: &ToolInvocation) -> String {
    if let Some(input) = invocation.as_first_party() {
        return match input {
            FirstPartyToolInput::Bash(input) => format!("bash {}", input.command),
            FirstPartyToolInput::Read(input) => format!("read {}", input.file_path),
            FirstPartyToolInput::ViewFile(input) => format!("view_file {}", input.path),
            FirstPartyToolInput::ApplyPatch(_) => "apply_patch".to_string(),
            FirstPartyToolInput::Glob(input) => format!("glob {}", input.pattern),
            FirstPartyToolInput::Grep(input) => format!("grep {}", input.pattern),
            FirstPartyToolInput::Task(input) => format!("task {}", input.description),
            FirstPartyToolInput::ToolSearch(input) => format!("tool_search {}", input.query),
            FirstPartyToolInput::TodoWrite(_) => "todo_write".to_string(),
            FirstPartyToolInput::AskUser(_) => "ask_user".to_string(),
            FirstPartyToolInput::Monitor(input) => match input {
                agena::message::MonitorToolInput::Start { command, .. } => {
                    format!("monitor start {command}")
                }
                agena::message::MonitorToolInput::List {} => "monitor list".to_string(),
                agena::message::MonitorToolInput::Read { monitor_id, .. } => {
                    format!("monitor read {monitor_id}")
                }
                agena::message::MonitorToolInput::Stop { monitor_id } => {
                    format!("monitor stop {monitor_id}")
                }
            },
            FirstPartyToolInput::WebFetch(input) => format!("web_fetch {}", input.url),
            FirstPartyToolInput::WebSearch(input) => format!("web_search {}", input.query),
            FirstPartyToolInput::EnterPlanMode(_) => "enter_plan_mode".to_string(),
            FirstPartyToolInput::ExitPlanMode(_) => "exit_plan_mode".to_string(),
            FirstPartyToolInput::EnterWorktree(input) => match (&input.name, &input.path) {
                (Some(n), _) => format!("enter_worktree name={n}"),
                (_, Some(p)) => format!("enter_worktree path={p}"),
                _ => "enter_worktree".to_string(),
            },
            FirstPartyToolInput::ExitWorktree(input) => format!("exit_worktree {}", input.action),
            FirstPartyToolInput::CronCreate(input) => {
                format!("cron_create {}", input.expression)
            }
            FirstPartyToolInput::CronList(_) => "cron_list".to_string(),
            FirstPartyToolInput::CronDelete(input) => format!("cron_delete {}", input.id),
            FirstPartyToolInput::ScheduleWakeup(input) => {
                format!("schedule_wakeup +{}s", input.delay_seconds)
            }
            FirstPartyToolInput::LspDefinition(input) => {
                format!(
                    "lsp_definition {}:{}:{}",
                    input.file_path, input.line, input.character
                )
            }
            FirstPartyToolInput::LspReferences(input) => {
                format!(
                    "lsp_references {}:{}:{}",
                    input.file_path, input.line, input.character
                )
            }
            FirstPartyToolInput::LspHover(input) => {
                format!(
                    "lsp_hover {}:{}:{}",
                    input.file_path, input.line, input.character
                )
            }
            FirstPartyToolInput::LspDiagnostics(input) => {
                format!("lsp_diagnostics {}", input.file_path)
            }
            FirstPartyToolInput::NotebookEdit(input) => {
                format!("notebook_edit {}", input.notebook_path)
            }
            FirstPartyToolInput::PowerShell(input) => format!("powershell {}", input.command),
        };
    }
    let ToolInvocation { name, .. } = invocation;
    name.clone()
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
        PermissionAction::BuiltinTool { tool_name, .. } => i18n.text_args(
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
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ToolOutputPreview {
    text: String,
    omitted_lines: usize,
}

fn push_tool_output_preview(
    out: &mut Vec<RenderedLine>,
    prefix: &str,
    text: &str,
    style: Style,
    width: u16,
    i18n: &I18n,
) {
    let preview = tool_output_preview(text);
    push_multiline(out, prefix, preview.text.as_str(), style, width);
    if preview.omitted_lines > 0 {
        out.push(RenderedLine::dim(i18n.text_args(
            "message-tool-output-collapsed",
            &crate::fl_args!("lines" => preview.omitted_lines as i64),
        )));
    }
}

fn tool_output_preview(text: &str) -> ToolOutputPreview {
    let total_lines = text.split('\n').count();
    let mut preview = String::new();
    let mut used_chars = 0_usize;
    let mut included_lines = 0_usize;
    let mut truncated = false;

    for (index, line) in text.split('\n').enumerate() {
        if index >= TOOL_CARD_PREVIEW_LINES {
            truncated = true;
            break;
        }

        let separator_chars = usize::from(index > 0);
        let line_chars = line.chars().count();
        if used_chars
            .saturating_add(separator_chars)
            .saturating_add(line_chars)
            > TOOL_CARD_PREVIEW_CHARS
        {
            if index > 0 {
                preview.push('\n');
            }
            let remaining = TOOL_CARD_PREVIEW_CHARS
                .saturating_sub(used_chars)
                .saturating_sub(separator_chars);
            preview.extend(line.chars().take(remaining));
            included_lines = index + 1;
            truncated = true;
            break;
        }

        if index > 0 {
            preview.push('\n');
            used_chars += 1;
        }
        preview.push_str(line);
        used_chars += line_chars;
        included_lines = index + 1;
    }

    let mut omitted_lines = if truncated {
        total_lines.saturating_sub(included_lines)
    } else {
        0
    };
    if truncated && omitted_lines == 0 {
        omitted_lines = 1;
    }

    ToolOutputPreview {
        text: preview,
        omitted_lines,
    }
}

fn push_multiline(out: &mut Vec<RenderedLine>, prefix: &str, text: &str, style: Style, width: u16) {
    for raw_line in text.split('\n') {
        out.extend(wrap_rendered_line(
            RenderedLine {
                text: format!("{prefix}{raw_line}"),
                style,
            },
            width,
        ));
    }
}

fn wrap_rendered_line(line: RenderedLine, width: u16) -> Vec<RenderedLine> {
    if width <= 1 {
        return vec![line];
    }
    if UnicodeWidthStr::width(line.text.as_str()) <= width as usize {
        return vec![line];
    }

    let options = WrapOptions::new(width as usize)
        .break_words(false)
        .word_splitter(WordSplitter::NoHyphenation);
    wrap(line.text.as_str(), options)
        .into_iter()
        .map(|segment| RenderedLine {
            text: segment.into_owned(),
            style: line.style,
        })
        .collect()
}

#[allow(dead_code)]
fn role_label(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
    }
}

fn style_for_role(role: Role) -> Style {
    match role {
        Role::User => Style::default().fg(Color::Green),
        Role::Assistant => Style::default().fg(Color::Cyan),
        Role::System => Style::default().fg(Color::Magenta),
        Role::Tool => Style::default().fg(Color::Yellow),
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
    let detail = format_plugin_inspector_detail(&status, manifest);
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
                lines.push(format!("  - {} ({:?})", entry.name, entry.behavior));
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

#[allow(dead_code)]
fn format_relative_time(timestamp: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(timestamp);
    if delta.num_seconds() < 60 {
        "just now".to_string()
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else {
        format!("{}d ago", delta.num_days())
    }
}

fn rewind_message_preview(message: &MessageResource, i18n: &I18n) -> String {
    let preview = render_message(message, 72, i18n)
        .into_iter()
        .skip(1)
        .map(|line| line.text.trim().to_string())
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| ui_text::t(i18n, "message-empty"));
    truncate_display_width(preview.as_str(), 64)
}

fn render_transcript_export_markdown(
    i18n: &I18n,
    session_id: Option<i64>,
    session_title: &str,
    execution: Option<&SessionExecutionResource>,
    messages: &[MessageResource],
    has_more_older: bool,
) -> String {
    if session_id.is_none() && messages.is_empty() {
        return String::new();
    }

    let title = if !session_title.trim().is_empty() {
        session_title.trim().to_string()
    } else if let Some(session_id) = session_id {
        ui_text::session_fallback_title(i18n, session_id)
    } else {
        "Agena Transcript Export".to_string()
    };

    let mut out = vec![format!("# {title}"), String::new()];
    if let Some(session_id) = session_id {
        out.push(format!("- Session ID: {session_id}"));
    }
    out.push(format!(
        "- Exported At: {}",
        Local::now().format("%Y-%m-%d %H:%M:%S %z")
    ));
    out.push(format!("- Messages Loaded: {}", messages.len()));
    out.push(format!(
        "- Older Messages Omitted: {}",
        if has_more_older { "yes" } else { "no" }
    ));
    if let Some(execution) = execution {
        if let Some(parent_id) = execution.session.parent_id {
            out.push(format!("- Parent Session: #{parent_id}"));
        }
        out.push(format!(
            "- Child Sessions: {}",
            execution.session.child_session_count
        ));
    }
    out.push(String::new());

    if messages.is_empty() {
        out.push("_No messages loaded in this session._".to_string());
        return out.join("\n");
    }

    for message in messages {
        let timestamp = format_timestamp(message.created_at);
        out.push(format!(
            "## {} · {} · {}",
            ui_text::role_label(i18n, message.role),
            ui_text::message_state_label(i18n, message.state),
            timestamp,
        ));
        out.push(String::new());
        out.push("~~~~text".to_string());
        out.extend(
            render_message(message, u16::MAX, i18n)
                .into_iter()
                .map(|line| line.text),
        );
        out.push("~~~~".to_string());
        out.push(String::new());
    }

    out.join("\n")
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
        AgenaSessionEvent::MessageRevised(event) => Some(event.target_message_id),
        AgenaSessionEvent::RunStarted(_)
        | AgenaSessionEvent::RunFailed(_)
        | AgenaSessionEvent::StreamError(_)
        | AgenaSessionEvent::PermissionRequested(_)
        | AgenaSessionEvent::PermissionReplied(_)
        | AgenaSessionEvent::PermissionRuleCreated(_)
        | AgenaSessionEvent::PermissionRuleUpdated(_)
        | AgenaSessionEvent::PermissionRuleRevoked(_)
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
                "permission requested: {}",
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
        AgenaSessionEvent::MessageRevised(p) => {
            format!("message #{} revised", p.target_message_id)
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
        AgenaSessionEvent::PermissionRequested(event) => vec![
            format!("session_id: {}", event.session_id),
            format!("request_id: {}", event.request_id),
            format!("reason: {}", event.reason),
            format!(
                "explanation: {}",
                detail_excerpt(event.explanation.as_str(), 200)
            ),
        ],
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
        AgenaSessionEvent::MessageRevised(p) => vec![
            format!("target_message_id: {}", p.target_message_id),
            format!("kind: {:?}", p.kind),
        ],
        AgenaSessionEvent::PluginEvent(p) => vec![
            format!("plugin_id: {}", p.plugin_id),
            format!("kind_label: {}", p.kind_label),
            format!("payload: {}", detail_excerpt(&p.payload.to_string(), 200)),
        ],
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

fn session_depth_map(items: &[SessionResource]) -> BTreeMap<i64, usize> {
    let by_id = items
        .iter()
        .map(|session| (session.id, session.parent_id))
        .collect::<BTreeMap<_, _>>();
    let mut depths = BTreeMap::new();
    for session in items {
        let mut depth = 0_usize;
        let mut current = session.parent_id;
        let mut seen = HashSet::new();
        while let Some(parent_id) = current {
            if !seen.insert(parent_id) || !by_id.contains_key(&parent_id) {
                break;
            }
            depth = depth.saturating_add(1);
            current = by_id.get(&parent_id).copied().flatten();
        }
        depths.insert(session.id, depth);
    }
    depths
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

#[allow(dead_code)]
fn default_session_title() -> String {
    format!("New session {}", Local::now().format("%H:%M"))
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

fn contains_case_insensitive(text: &str, query: &str) -> bool {
    let trimmed = query.trim();
    !trimmed.is_empty()
        && text
            .to_lowercase()
            .contains(trimmed.to_lowercase().as_str())
}

fn highlight_search_line(
    text: &str,
    base_style: Style,
    query: &str,
    active_match: bool,
    has_match: bool,
) -> Line<'static> {
    let line_style = if active_match {
        base_style.bg(Color::Rgb(60, 43, 8))
    } else if has_match {
        base_style.bg(Color::Rgb(36, 28, 7))
    } else {
        base_style
    };

    if !has_match || query.trim().is_empty() {
        return Line::from(Span::styled(text.to_string(), line_style));
    }

    let ranges = find_search_ranges(text, query);
    if ranges.is_empty() {
        return Line::from(Span::styled(text.to_string(), line_style));
    }

    let mut spans = Vec::new();
    let mut cursor = 0;
    for range in ranges {
        if cursor < range.start {
            spans.push(Span::styled(
                text[cursor..range.start].to_string(),
                line_style,
            ));
        }
        let match_style = if active_match {
            line_style
                .bg(Color::Rgb(121, 86, 10))
                .add_modifier(Modifier::BOLD)
        } else {
            line_style.bg(Color::Rgb(84, 61, 10))
        };
        spans.push(Span::styled(text[range.clone()].to_string(), match_style));
        cursor = range.end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), line_style));
    }

    Line::from(spans)
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

fn parse_tui_color(value: &str) -> Option<Color> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "dark_gray" | "dark-grey" | "darkgrey" => Some(Color::DarkGray),
        "lightred" | "light_red" | "light-red" => Some(Color::LightRed),
        "lightgreen" | "light_green" | "light-green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" | "light-yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" | "light-blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" | "light-magenta" => Some(Color::LightMagenta),
        "lightcyan" | "light_cyan" | "light-cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => parse_hex_color(value),
    }
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(red, green, blue))
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

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = min(width, area.width.saturating_sub(2));
    let height = min(height, area.height.saturating_sub(2));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn inner_rect(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
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

#[allow(dead_code)]
fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let bytes_f = bytes as f64;
    if bytes_f >= GB {
        format!("{:.1} GB", bytes_f / GB)
    } else if bytes_f >= MB {
        format!("{:.1} MB", bytes_f / MB)
    } else if bytes_f >= KB {
        format!("{:.1} KB", bytes_f / KB)
    } else {
        format!("{bytes} B")
    }
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
    fn to_request(&self) -> RunOptions {
        RunOptions {
            model: self.model.clone(),
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
            subject_kind: PermissionRuleSubjectKind::BuiltinTool,
            tool_name: String::new(),
            qualifier: String::new(),
            path_access_kind: "read".to_string(),
            workspace_root: String::new(),
            target_path: String::new(),
            scope: "workspace".to_string(),
            session_id: String::new(),
            mode: PermissionMode::Ask,
        }
    }
}

fn permission_rule_draft_from_resource(rule: &PermissionRuleResource) -> PermissionRuleDraft {
    PermissionRuleDraft {
        subject_kind: if rule.subject_kind == "path_access" {
            PermissionRuleSubjectKind::PathAccess
        } else {
            PermissionRuleSubjectKind::BuiltinTool
        },
        tool_name: rule.tool_name.clone().unwrap_or_default(),
        qualifier: rule.qualifier.clone().unwrap_or_default(),
        path_access_kind: rule
            .path_access_kind
            .clone()
            .unwrap_or_else(|| "read".to_string()),
        workspace_root: rule.workspace_root.clone().unwrap_or_default(),
        target_path: rule.target_path.clone().unwrap_or_default(),
        scope: rule.scope.clone(),
        session_id: rule.session_id.map(|id| id.to_string()).unwrap_or_default(),
        mode: rule.mode,
    }
}

fn permission_rule_label(rule: &PermissionRuleResource) -> String {
    match rule.subject_kind.as_str() {
        "builtin_tool" => match (rule.tool_name.as_deref(), rule.qualifier.as_deref()) {
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
        PermissionRuleSubjectKind::BuiltinTool => {
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
    }
}

fn render_permission_rule_draft(draft: &PermissionRuleDraft) -> String {
    match draft.subject_kind {
        PermissionRuleSubjectKind::BuiltinTool => {
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
    }
}

fn render_permission_rule_preview(input: &str) -> String {
    match parse_permission_rule_input(input) {
        Ok(draft) => {
            let mut lines = vec![format!("label: {}", permission_rule_draft_label(&draft))];
            lines.push(format!("mode: {}", permission_mode_name(draft.mode)));
            lines.push(format!("scope: {}", draft.scope));
            match draft.subject_kind {
                PermissionRuleSubjectKind::BuiltinTool => {
                    lines.push(format!(
                        "subject: builtin_tool ({})",
                        draft.tool_name.trim()
                    ));
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
    ]
    .join("\n")
}

fn permission_rule_params_from_draft(draft: &PermissionRuleDraft) -> UpsertPermissionRuleParams {
    match draft.subject_kind {
        PermissionRuleSubjectKind::BuiltinTool => UpsertPermissionRuleParams {
            action_key: None,
            subject_kind: Some("builtin_tool".to_string()),
            tool_name: Some(draft.tool_name.trim().to_string()),
            qualifier: non_empty_owned(draft.qualifier.clone()),
            path_access_kind: None,
            workspace_root: None,
            target_path: None,
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
            draft.subject_kind = PermissionRuleSubjectKind::BuiltinTool;
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
        _ => {
            return Err("rule subject must start with `tool` or `path`".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use agena::{
        event::{CommandContext, CommandEndEvent},
        message::{
            ExecutionStatus, MessageMetadata, MessageStatus, PartContent, UserInputOption,
            UserInputQuestion,
        },
    };
    use chrono::Utc;
    use serde_json::json;

    #[test]
    fn derive_title_uses_first_non_empty_line() {
        assert_eq!(
            derive_session_title("\n\n  hello world  \nnext"),
            "hello world"
        );
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
    fn parse_permission_rule_input_supports_builtin_tool_rules() {
        let draft = parse_permission_rule_input(
            "tool bash allow qualifier='npm test' scope=session session=42",
        )
        .expect("permission rule input should parse");
        assert_eq!(draft.subject_kind, PermissionRuleSubjectKind::BuiltinTool);
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
    fn parse_permission_rule_input_supports_global_scope() {
        let draft = parse_permission_rule_input("tool bash allow scope=global")
            .expect("global permission rule input should parse");
        assert_eq!(draft.subject_kind, PermissionRuleSubjectKind::BuiltinTool);
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
            .entry(
                agena::plugin::PluginEntryDecl::new("inspect", json!({"type": "object"}))
                    .behavior(agena::plugin::EntryBehavior::Task)
                    .host_capabilities([
                        agena::plugin::sdk::HostCapability::PluginStatus,
                        agena::plugin::sdk::HostCapability::ReadConfig,
                    ]),
            )
            .entry(
                agena::plugin::PluginEntryDecl::new("logs", json!({"type": "object"}))
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
        let line =
            highlight_search_line("alpha hello omega", Style::default(), "hello", false, true);
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
                role: Role::User,
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
        }
    }
}
