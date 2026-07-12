use std::{collections::BTreeMap, time::Instant};

use agena_api::resource::{PermissionRuleResource, ProviderSummaryResource, SessionResource};
use agena_tui_components::{Editor, SearchListNoCustom, SearchListOverlay, SelectableListState};

use crate::commands::CommandSpec;

use super::{
    AgentDescriptor, ComposerDraft, I18n, MessageResource, ModelRef, RenderedTranscript,
    SessionExecutionResource, SessionLoadScope, SessionViewMode, TranscriptBlockCursor,
    TranscriptDetailDefaults, TranscriptNodeKey,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchItem {
    pub(crate) session: SessionResource,
    pub(crate) label: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchOverlayMeta {
    pub(crate) all_items: Vec<SessionSearchItem>,
    pub(crate) mode: SessionViewMode,
    pub(crate) scope_session_id: Option<i64>,
    pub(crate) page_limit: usize,
    pub(crate) page_index: usize,
    pub(crate) offset: usize,
    pub(crate) cursors: Vec<Option<String>>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) has_more: bool,
}

pub(crate) type SessionSearchOverlay =
    SearchListOverlay<SessionSearchItem, SearchListNoCustom, SessionSearchOverlayMeta, Editor>;

#[derive(Debug, Clone)]
pub(crate) struct PickerOverlayMeta {
    pub(crate) all_items: Vec<PickerItem>,
    pub(crate) kind: PickerKind,
}

pub(crate) type PickerOverlay =
    SearchListOverlay<PickerItem, SearchListNoCustom, PickerOverlayMeta, Editor>;

#[derive(Debug, Clone)]
pub(crate) struct SessionModelChooserOverlayMeta {
    pub(crate) all_items: Vec<SessionModelChoiceItem>,
    pub(crate) page_size: usize,
    pub(crate) current_model_label: Option<String>,
}

pub(crate) type SessionModelChooserOverlay = SearchListOverlay<
    SessionModelChoiceItem,
    SearchListNoCustom,
    SessionModelChooserOverlayMeta,
    Editor,
>;

#[derive(Debug, Clone)]
pub(crate) struct SessionModelChoiceItem {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) search_text: String,
    pub(crate) model: ModelRef,
}

#[derive(Debug, Clone)]
pub(crate) struct PickerItem {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) value: PickerValue,
}

#[derive(Debug, Clone)]
pub(crate) enum PickerValue {
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
pub(crate) enum ProviderPickerPurpose {
    SetProvider,
    Configure,
}

#[derive(Debug, Clone)]
pub(crate) enum PickerKind {
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
pub(crate) enum LineageRelation {
    Ancestor,
    Current,
    Sibling,
    Child,
}

#[derive(Debug, Clone)]
pub(crate) struct LineageSessionItem {
    pub(crate) session: SessionResource,
    pub(crate) relation: LineageRelation,
    pub(crate) depth: usize,
    pub(crate) is_leaf: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionLineageSummary {
    pub(crate) root_id: i64,
    pub(crate) depth: usize,
    pub(crate) side_branch_count: usize,
    pub(crate) descendant_count: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct CurrentLineageState {
    pub(crate) session_id: i64,
    pub(crate) summary: SessionLineageSummary,
}

#[derive(Debug, Clone)]
pub(crate) struct FlashMessage {
    pub(crate) text: String,
    pub(crate) level: FlashLevel,
    pub(crate) expires_at: Instant,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum FlashLevel {
    Success,
    Warning,
    Error,
    Info,
}

#[derive(Default)]
pub(crate) struct SessionListState {
    pub(crate) source_items: Vec<SessionResource>,
    pub(crate) list: SelectableListState<SessionResource>,
    pub(crate) search_query: String,
    pub(crate) view_mode: SessionViewMode,
    pub(crate) subtree_root_id: Option<i64>,
    pub(crate) pending_scope: Option<SessionLoadScope>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) has_more: bool,
    pub(crate) loading: bool,
    pub(crate) loading_more: bool,
    pub(crate) initialized: bool,
}

pub(crate) struct TranscriptState {
    pub(crate) i18n: I18n,
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
    pub(crate) pending_restore_draft: Option<ComposerDraft>,
    pub(crate) follow_tail: bool,
    pub(crate) scroll: usize,
    pub(crate) cursor_line: usize,
    pub(crate) block_cursor: Option<TranscriptBlockCursor>,
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
    pub(crate) system: Option<String>,
    pub(crate) temperature: Option<f32>,
    pub(crate) max_output_tokens: Option<u32>,
}
