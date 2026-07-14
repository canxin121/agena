use std::{collections::BTreeMap, time::Instant};

use agena_api::resource::{PermissionRuleResource, ProviderSummaryResource, SessionResource};
use agena_tui_components::{Editor, SearchPicker, SearchPickerNoCustom, SelectableListState};

use crate::commands::CommandSpec;

use super::{
    AgentDescriptor, ComposerDraft, I18n, MathRenderContext, MessageResource, ModelRef,
    RenderedTranscript, SessionExecutionResource, SessionLoadScope, SessionViewMode,
    TranscriptBlockCursor, TranscriptDetailDefaults, TranscriptNodeKey,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchItem {
    pub(crate) session: SessionResource,
    pub(crate) label: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SessionSearchOverlayMeta {
    /// The complete subtree catalog. Visual pagination is always owned by the
    /// shared `SearchPicker`; remote modes append backend result batches.
    pub(crate) all_items: Vec<SessionSearchItem>,
    pub(crate) mode: SessionViewMode,
    pub(crate) scope_session_id: Option<i64>,
    /// Index of the latest backend batch, not a user-visible page number.
    pub(crate) page_index: usize,
    pub(crate) next_cursor: Option<String>,
    pub(crate) has_more: bool,
}

pub(crate) type SessionSearchOverlay =
    SearchPicker<SessionSearchItem, SearchPickerNoCustom, SessionSearchOverlayMeta, Editor>;

#[derive(Debug, Clone)]
pub(crate) struct PickerOverlayMeta {
    pub(crate) kind: PickerKind,
}

pub(crate) type PickerOverlay =
    SearchPicker<PickerItem, SearchPickerNoCustom, PickerOverlayMeta, Editor>;

#[derive(Debug, Clone)]
pub(crate) struct SessionModelChooserOverlayMeta {
    pub(crate) purpose: SessionModelChooserPurpose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionModelChooserPurpose {
    RuntimeOverride,
    ProviderDefault,
}

pub(crate) type SessionModelChooserOverlay = SearchPicker<
    SessionModelChoiceItem,
    SearchPickerNoCustom,
    SessionModelChooserOverlayMeta,
    Editor,
>;

#[derive(Debug, Clone)]
pub(crate) struct SessionModelChoiceItem {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) search_text: String,
    pub(crate) model: ModelRef,
    pub(crate) current: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PickerItem {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) value: PickerValue,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum PickerValue {
    Command(&'static CommandSpec),
    PluginCommand(Box<agena::plugin::PluginCommandCatalogItem>),
    ProviderCreate,
    Provider(ProviderSummaryResource),
    AgentCreate,
    Agent(Box<AgentDescriptor>),
    Session(i64),
    Message(Box<MessageResource>),
    PermissionRuleCreate,
    PermissionRule(Box<PermissionRuleResource>),
    Inspector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderPickerPurpose {
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
}
