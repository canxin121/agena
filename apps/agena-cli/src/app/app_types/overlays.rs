use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Instant,
};

use agena::{
    agent::PermissionConfig,
    agents::AgentProfile,
    message::UserInputRequest,
    permission::{PermissionMode, PermissionReplyKind, PermissionRequest, PermissionScope},
};
use agena_api::resource::ProviderAdapterModelsResource;
use ratatui::text::Text;

use crate::backend::{ProviderConfigDraft, ProviderToolsPreset};
use crate::i18n::I18n;
use agena_api_server::local_api::{CatalogModelResource, ModelCatalogResponse};
use agena_tui_components::{
    ConfirmDialogState, DashboardSelectionState, Editor, EditorDialogState, InputDialogState,
    ListWorkbenchState, QuestionFlowState, SearchPicker, SectionedListFocus, SectionedListState,
    SelectableListState, SelectionCursor,
};

use super::{LineInputOverlay, UserInputAnswerDraft};

#[derive(Debug, Clone)]
pub(crate) struct SettingsStudioOverlay {
    pub(crate) title: String,
    pub(crate) footer: String,
    pub(crate) state: SectionedListState<SettingsStudioSection>,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentStudioOverlay {
    pub(crate) agent_name: String,
    pub(crate) profile: AgentProfile,
    pub(crate) storage: AgentProfileStorage,
    pub(crate) editable: bool,
    pub(crate) default_agent_name: Option<String>,
    pub(crate) workbench: ListWorkbenchState<AgentStudioItem, AgentStudioEditor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentProfileStorage {
    BuiltIn,
    Config,
    Markdown,
    Runtime,
}

impl AgentProfileStorage {
    pub(crate) fn editable(self) -> bool {
        matches!(self, Self::Config | Self::Markdown)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AgentStudioItem {
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) detail: String,
    pub(crate) action: AgentStudioAction,
}

#[derive(Debug, Clone)]
pub(crate) enum AgentStudioAction {
    Edit(AgentStudioField),
    OpenPermissionWorkbench,
    OpenSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentStudioField {
    Description,
    Prompt,
    DefaultProvider,
    DefaultAdapter,
    DefaultModel,
}

pub(crate) type AgentStudioEditor = EditorDialogState<AgentStudioEditorAction>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AgentStudioEditorAction {
    Field(AgentStudioField),
}

#[derive(Debug, Clone)]
pub(crate) enum PermissionStudioSource {
    GlobalConfig,
    WorkspaceConfig,
    Agent { agent_name: String },
    Session { session_id: i64 },
    EffectiveSession { session_id: i64 },
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionStudioOverlay {
    pub(crate) title: String,
    pub(crate) footer: String,
    pub(crate) source: PermissionStudioSource,
    pub(crate) title_context: String,
    pub(crate) source_label: String,
    pub(crate) scope_label: String,
    pub(crate) editable: bool,
    pub(crate) permission: PermissionConfig,
    pub(crate) nav: SelectableListState<PermissionStudioNavItem>,
    pub(crate) pane_focus: PermissionStudioPaneFocus,
    pub(crate) page: PermissionStudioPage,
    pub(crate) state: SectionedListState<PermissionStudioSection>,
    pub(crate) editor: Option<PermissionStudioEditor>,
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionStudioNavItem {
    pub(crate) label: String,
    pub(crate) level: usize,
    pub(crate) page: PermissionStudioPage,
    pub(crate) section: Option<PermissionStudioSectionId>,
    pub(crate) selectable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionStudioPaneFocus {
    Navigation,
    Content,
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionStudioItem {
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) action: PermissionStudioAction,
}

#[derive(Debug, Clone)]
pub(crate) enum PermissionStudioAction {
    Noop,
    CreateRule,
    EditMode(PermissionStudioModeTarget),
    AddToolCommandPattern { tool_name: String },
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionStudioSection {
    pub(crate) id: PermissionStudioSectionId,
    pub(crate) label: String,
    pub(crate) items: Vec<PermissionStudioItem>,
}

pub(crate) type PermissionStudioFocus = SectionedListFocus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionStudioSectionId {
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
pub(crate) enum PermissionStudioPage {
    Overview,
    PathDefaults,
    PathRules,
    NetworkZones,
    NetworkRules,
    ToolTags,
    ToolNames,
    ToolCommandRules,
}

pub(crate) type PermissionStudioEditor = EditorDialogState<PermissionStudioEditorAction>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PermissionStudioEditorAction {
    Text(PermissionStudioTextTarget),
    AddPathRule,
    AddNetworkRule,
    AddToolTag,
    AddToolName,
    AddToolRule,
    AddToolCommandPattern { tool_name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PermissionStudioModeTarget {
    PathWorkspaceRead,
    PathWorkspaceWrite,
    PathExternalRead,
    PathExternalWrite,
    NetworkInternet,
    NetworkPrivate,
    NetworkLoopback,
    ToolDefault,
    PathRuleRead { pattern: String },
    PathRuleWrite { pattern: String },
    NetworkRule { target: String },
    ToolTag { key: String },
    ToolName { key: String },
    ToolRule { tool_name: String },
    ToolCommandPattern { tool_name: String, pattern: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PermissionStudioTextTarget {
    PathRulePattern { pattern: String },
    NetworkRuleTarget { target: String },
    ToolTagKey { key: String },
    ToolNameKey { key: String },
    ToolRuleName { tool_name: String },
}

#[derive(Debug, Clone)]
pub(crate) struct SettingsStudioSection {
    pub(crate) id: SettingsStudioSectionId,
    pub(crate) label: String,
    pub(crate) summary: String,
    pub(crate) description: String,
    pub(crate) items: Vec<SettingsStudioItem>,
}

#[derive(Debug, Clone)]
pub(crate) struct SettingsStudioItem {
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) detail: String,
    pub(crate) path: Option<String>,
    pub(crate) current_value: Option<String>,
    pub(crate) effective_value: Option<String>,
    pub(crate) source_rows: Vec<SettingsSourceRow>,
    pub(crate) action: SettingsPickerAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsSourceRow {
    pub(crate) label: String,
    pub(crate) value: String,
}

pub(crate) type SettingsStudioFocus = SectionedListFocus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsStudioSectionId {
    ModelsProviders,
    Agents,
    Permissions,
    PluginsTools,
    RuntimeSession,
    Interface,
    Diagnostics,
}

pub(crate) type SettingsValueEditOverlay = InputDialogState<SettingsFieldSpec>;

#[derive(Debug, Clone)]
pub(crate) struct ChoiceOverlayMeta {
    pub(crate) i18n: I18n,
    pub(crate) action: ChoiceOverlayAction,
    /// The committed value when the picker opened. This is deliberately kept
    /// separate from the transient search query in `ChoiceOverlay::input`.
    pub(crate) current_value: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ChoiceItem {
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) value: String,
    pub(crate) search_text: String,
    pub(crate) current: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ChoiceCustomValue {
    pub(crate) raw: String,
}

pub(crate) type ChoiceOverlay =
    SearchPicker<ChoiceItem, ChoiceCustomValue, ChoiceOverlayMeta, Editor>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChoiceOverlayStyle {
    Searchable,
    SearchableSelect,
    SelectOnly,
}

#[derive(Debug, Clone)]
pub(crate) enum ChoiceOverlayAction {
    SettingsField(SettingsFieldSpec),
    SessionModelVariant(SessionModelVariantStep),
    ProviderStudioField(ProviderStudioField),
    ProviderStudioModelField(ProviderModelConfigField),
    PermissionRuleStudio(PermissionRuleStudioChoiceField),
    PermissionStudioMode(PermissionStudioModeTarget),
    PermissionStudioAddEntries(PermissionStudioCatalogKind),
    PermissionStudioAddEntriesMode {
        kind: PermissionStudioCatalogKind,
        entries: Vec<String>,
        add_custom_after: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionStudioCatalogKind {
    ToolTags,
    ToolNames,
}

pub(in crate::app) const PERMISSION_STUDIO_CUSTOM_ENTRY: &str = "__agena_permission_custom_entry__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionModelVariantStep {
    ThinkingMode,
    SpeedMode,
    Verbosity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionRuleStudioChoiceField {
    SubjectKind,
    PathAccessKind,
    Scope,
    Mode,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SettingsFieldSpec {
    pub(crate) section: SettingsStudioSectionId,
    pub(crate) path: &'static str,
    pub(crate) label_key: &'static str,
    pub(crate) description_key: &'static str,
    pub(crate) kind: SettingsFieldKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum SettingsFieldKind {
    String,
    Bool,
    Integer,
}

#[derive(Debug, Clone)]
pub(crate) enum SettingsPickerAction {
    EditField(SettingsFieldSpec),
    OpenProviderDefaultModelChooser,
    OpenAgentList,
    OpenProviderList,
    OpenModelCatalogWorkbench,
    OpenGlobalPermissionWorkbench,
    OpenWorkspacePermissionWorkbench,
    OpenCurrentSessionPermissionWorkbench,
    OpenSessionEffectivePermissionView(i64),
    OpenPluginWorkbench,
    RefreshProviderClientVersions,
    OpenConfigFile,
    OpenTerminalDiagnostics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PermissionRuleDraft {
    pub(crate) subject_kind: PermissionRuleSubjectKind,
    pub(crate) tool_name: String,
    pub(crate) qualifier: String,
    pub(crate) path_access_kind: String,
    pub(crate) workspace_root: String,
    pub(crate) target_path: String,
    pub(crate) network_target: String,
    pub(crate) network_host: String,
    pub(crate) network_port: String,
    pub(crate) scope: String,
    pub(crate) session_id: String,
    pub(crate) mode: PermissionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionRuleSubjectKind {
    Tool,
    PathAccess,
    NetworkAccess,
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionRuleStudioOverlay {
    pub(crate) rule_id: Option<i64>,
    pub(crate) draft: PermissionRuleDraft,
    /// Permission prompt suspended while the user edits a pre-filled rule.
    ///
    /// Keeping this on the route instead of in `App::overlay` prevents an
    /// invisible modal from intercepting all input while the full-screen rule
    /// editor is active.
    pub(crate) return_permission: Option<PermissionOverlay>,
    pub(crate) workbench: ListWorkbenchState<PermissionRuleStudioItem, PermissionRuleStudioEditor>,
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionRuleStudioItem {
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) detail: String,
    pub(crate) action: PermissionRuleStudioAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionRuleStudioAction {
    SubjectKind,
    ToolName,
    Qualifier,
    PathAccessKind,
    WorkspaceRoot,
    TargetPath,
    NetworkTarget,
    Scope,
    SessionId,
    Mode,
}

pub(crate) type PermissionRuleStudioEditor = EditorDialogState<PermissionRuleStudioEditField>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionRuleStudioEditField {
    ToolName,
    Qualifier,
    WorkspaceRoot,
    TargetPath,
    NetworkTarget,
    SessionId,
}

#[derive(Debug, Clone)]
pub(crate) struct UserInputOverlay {
    pub(crate) session_id: i64,
    pub(crate) request: UserInputRequest,
    pub(crate) answers: BTreeMap<String, UserInputAnswerDraft>,
    pub(crate) state: QuestionFlowState,
    pub(crate) editing_custom: bool,
    pub(crate) custom_input: Editor,
    pub(crate) review_option: usize,
    pub(crate) review_scroll: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionOverlay {
    pub(crate) session_id: i64,
    pub(crate) request: PermissionRequest,
    pub(crate) page: PermissionOverlayPage,
    pub(crate) selection: SelectionCursor,
}

#[derive(Debug, Clone)]
pub(crate) enum PendingInteractiveOverlayTarget {
    Permission {
        session_id: i64,
        request: Box<PermissionRequest>,
    },
    UserInput {
        session_id: i64,
        request: Box<UserInputRequest>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingInteractiveKind {
    Permission,
    UserInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionOverlayPage {
    Action,
    Scope(PermissionOverlayDecision),
    Details(PermissionOverlayDetailsReturn),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionOverlayDetailsReturn {
    Action,
    Scope(PermissionOverlayDecision),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionOverlayDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionOverlayChoice {
    OpenScope(PermissionOverlayDecision),
    Reply {
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
    },
    EditRule,
    Details,
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionReplayState {
    pub(crate) session_id: i64,
    pub(crate) fingerprint: String,
    pub(crate) last_request_id: String,
    pub(crate) kind: PermissionReplyKind,
    pub(crate) scope: Option<PermissionScope>,
    pub(crate) label: String,
}

pub(crate) type ConfirmOverlay = ConfirmDialogState<ConfirmAction>;

#[derive(Debug, Clone)]
pub(crate) enum ConfirmAction {
    Rewind {
        session_id: i64,
        message_id: i64,
        message_text: String,
        target: String,
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
    PermissionStudioDeleteToolCommandPattern {
        tool_name: String,
        pattern: String,
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
pub(crate) struct FileAttachOverlayMeta {
    pub(crate) i18n: I18n,
}

#[derive(Debug, Clone)]
pub(crate) struct TypedPathValue {
    pub(crate) raw: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PathBrowserOverlayMeta {
    pub(crate) i18n: I18n,
    pub(crate) mode: PathBrowserMode,
    pub(crate) target: PathBrowserTarget,
}

pub(crate) type FileAttachOverlay =
    SearchPicker<PathBuf, TypedPathValue, FileAttachOverlayMeta, Editor>;
pub(crate) type PathBrowserOverlay =
    SearchPicker<PathBrowserItem, TypedPathValue, PathBrowserOverlayMeta, Editor>;

#[derive(Debug, Clone)]
pub(crate) struct PathBrowserItem {
    pub(crate) path: PathBuf,
    pub(crate) label: String,
    pub(crate) detail: String,
    pub(crate) is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathBrowserMode {
    AnyPath,
    DirectoryOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathBrowserTarget {
    PermissionRuleStudio(PermissionRuleStudioPathField),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionRuleStudioPathField {
    WorkspaceRoot,
    TargetPath,
}

#[derive(Debug, Clone)]
pub(crate) struct TimelineOverlayMeta {
    pub(crate) session_id: i64,
}

pub(crate) type TimelineOverlay = SearchPicker<
    TimelineItem,
    agena_tui_components::SearchPickerNoCustom,
    TimelineOverlayMeta,
    Editor,
>;

#[derive(Debug, Clone)]
pub(crate) struct TimelineItem {
    pub(crate) summary: String,
    pub(crate) detail_body: Text<'static>,
    pub(crate) search_text: String,
    pub(crate) linked_message_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderStudioOverlay {
    pub(crate) title: String,
    pub(crate) footer: String,
    pub(crate) show_provider_list: bool,
    pub(crate) providers: SelectableListState<ProviderStudioProviderRow>,
    pub(crate) selection: DashboardSelectionState<ProviderStudioFocus>,
    pub(crate) draft: ProviderConfigDraft,
    pub(crate) adapter_models: Vec<ProviderAdapterModelsResource>,
    pub(crate) configured_adapter_ids: BTreeSet<String>,
    pub(crate) adapter_candidate_ids: Vec<String>,
    pub(crate) selected_adapter_ids: BTreeSet<String>,
    pub(crate) selected_model_keys: BTreeSet<String>,
    pub(crate) catalog_matches: BTreeMap<String, CatalogModelResource>,
    pub(crate) listing_adapter_models: bool,
    pub(crate) saving: bool,
    pub(crate) pending_adapter_models_key: Option<String>,
    pub(crate) pending_auth_key: Option<String>,
    pub(crate) next_auth_poll_at: Option<Instant>,
    pub(crate) detail_page: Option<ProviderStudioDetailPage>,
    pub(crate) model_page: Option<ProviderStudioModelPage>,
    pub(crate) editor: Option<ProviderStudioEditor>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderStudioProviderRow {
    pub(crate) provider_id: Option<String>,
    pub(crate) label: String,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderStudioFocus {
    Fields,
    Adapters,
    Models,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderStudioField {
    ProviderId,
    AuthMode,
    AuthSubtype,
    AuthLoginMethod,
    StartAuthAction,
    ContinueAuthAction,
    EditAuthDetailsAction,
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
    RequestTimeoutSecs,
    ConnectTimeoutSecs,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderStudioDetailPage {
    pub(crate) title: String,
    pub(crate) footer: String,
    pub(crate) selection: SelectionCursor,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderStudioModelPage {
    pub(crate) title: String,
    pub(crate) footer: String,
    pub(crate) adapter_id: String,
    pub(crate) original_model_id: String,
    pub(crate) draft: ProviderModelConfigDraft,
    pub(crate) selection: SelectionCursor,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderModelConfigDraft {
    pub(crate) model_id: String,
    pub(crate) enabled: bool,
    pub(crate) agena_tool_mode: agena::config::AgenaToolMode,
    pub(crate) display_name: String,
    pub(crate) lifecycle: String,
    pub(crate) context_window_tokens: String,
    pub(crate) max_input_tokens: String,
    pub(crate) max_output_tokens: String,
    pub(crate) input_modalities: BTreeSet<String>,
    pub(crate) features: BTreeSet<String>,
    pub(crate) output_modalities: String,
    pub(crate) thinking_mode_variants: BTreeSet<String>,
    pub(crate) speed_mode_variants: BTreeSet<String>,
    pub(crate) description: String,
    pub(crate) provider_tools_preset: ProviderToolsPreset,
    pub(crate) provider_tools_custom: agena::config::ProviderToolsConfig,
    pub(crate) definition: agena::provider::ConfiguredModelDefinition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderModelConfigField {
    ModelId,
    Enabled,
    AgenaToolMode,
    DisplayName,
    Lifecycle,
    ContextWindowTokens,
    MaxInputTokens,
    MaxOutputTokens,
    Features,
    InputModalities,
    OutputModalities,
    ThinkingModeVariants,
    SpeedModeVariants,
    Description,
    ProviderTools,
}

pub(crate) type ProviderStudioEditor = EditorDialogState<ProviderStudioEditorAction>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderStudioEditorAction {
    Field(ProviderStudioField),
    NewModel { adapter_id: String },
    ModelField(ProviderModelConfigField),
}

#[derive(Debug, Clone)]
pub(crate) struct ModelCatalogStudioOverlay {
    pub(crate) query: String,
    pub(crate) summary: ModelCatalogResponse,
    pub(crate) total: usize,
    pub(crate) offset: usize,
    pub(crate) limit: usize,
    pub(crate) loading: bool,
    pub(crate) workbench: ListWorkbenchState<CatalogModelResource, LineInputOverlay>,
}
