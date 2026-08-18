use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Instant,
};

use agena_api::resource::ProviderAdapterModelsResource;
use agena_domain::{ModelRef, PermissionConfig, UserInputRequest};
use agena_domain::{PermissionMode, PermissionReplyKind, PermissionRequest, PermissionScope};

use agena_application::dto::ModelCatalogResponse;
use agena_application::provider_studio::ProviderConfigDraft;
use agena_provider::AgenaToolMode;
use agena_tui::model_catalog::ModelCatalogPresentation;
use agena_tui::permission_prompt::PermissionPromptAutoApproveStatus;
use agena_tui::permission_prompt::PermissionPromptPresentation;
use agena_tui_components::{
    ConfirmDialogState, DashboardSelectionState, EditorDialogState, InputDialogState,
    SectionedListState, SelectableListState, SelectionCursor,
};
pub(crate) use agena_tui_permission_studio::permission_rule_studio::PermissionRuleStudioItem;
use agena_tui_permission_studio::permission_rule_studio::PermissionRuleStudioPresentation;
use agena_tui_settings::{SettingsStudioPresentation, SettingsStudioSectionId};

use super::LineInputOverlay;
use agena_tui::user_input::UserInputPresentation;

#[derive(Debug, Clone)]
pub(crate) struct SettingsStudioOverlay {
    pub(crate) title: String,
    pub(crate) footer: String,
    pub(crate) state: SettingsStudioPresentation<SettingsPickerAction>,
}

#[derive(Debug, Clone)]
pub(crate) enum PermissionStudioSource {
    GlobalConfig,
    WorkspaceConfig,
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
pub(crate) struct PermissionStudioItem {
    pub(crate) label: String,
    pub(crate) value: String,
    pub(crate) action: PermissionStudioAction,
}

#[derive(Debug, Clone)]
pub(crate) enum PermissionStudioAction {
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

pub(crate) use agena_tui_permission_studio::permission_studio::{
    PermissionStudioFocus, PermissionStudioNavItem, PermissionStudioPage,
    PermissionStudioPaneFocus, PermissionStudioSectionId,
};

pub(crate) type PermissionStudioEditor = EditorDialogState<PermissionStudioEditorAction>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PermissionStudioEditorAction {
    Text(PermissionStudioTextTarget),
    AddPathRule,
    AddNetworkRule,
    AddToolName,
    AddToolRule,
    AddToolCommandPattern { tool_name: String },
}

pub(crate) use agena_tui_permission_studio::PermissionStudioModeTarget;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PermissionStudioTextTarget {
    PathRulePattern { pattern: String },
    NetworkRuleTarget { target: String },
    ToolNameKey { key: String },
    ToolRuleName { tool_name: String },
}

pub(crate) type SettingsValueEditOverlay = InputDialogState<SettingsFieldSpec>;

pub(crate) use agena_tui::choice::ChoicePickerItem as ChoiceItem;

#[derive(Debug, Clone)]
pub(crate) struct ChoiceOverlay {
    pub(crate) presentation: agena_tui::choice::ChoicePresentation,
    pub(crate) action: ChoiceOverlayAction,
}

#[derive(Debug, Clone)]
pub(crate) enum ChoiceOverlayAction {
    InsertContent,
    SettingsField(SettingsFieldSpec),
    SessionModelMode(SessionModelModeStep),
    ModelSelectionMode {
        purpose: agena_tui::model_chooser::SessionModelChooserPurpose,
        model: ModelRef,
        step: SessionModelModeStep,
        thinking_mode: Option<String>,
        speed_mode: Option<String>,
        verbosity: Option<String>,
    },
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
    ToolNames,
}

pub(crate) const PERMISSION_STUDIO_CUSTOM_ENTRY: &str = "__agena_permission_custom_entry__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionModelModeStep {
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

#[derive(Debug, Clone)]
pub(crate) struct SettingsFieldSpec {
    pub(crate) section: SettingsStudioSectionId,
    pub(crate) path: String,
    pub(crate) label_key: &'static str,
    pub(crate) description_key: &'static str,
    pub(crate) kind: SettingsFieldKind,
    /// Dynamic label override (e.g. a plugin-contributed activity kind).
    pub(crate) label_override: Option<String>,
    /// Dynamic description override (e.g. a plugin-contributed activity kind).
    pub(crate) description_override: Option<String>,
}

/// Synthetic settings paths used only to route the existing single-line
/// editor to the server-owned MCP control API. They are never written to
/// `agena.json`.
pub(crate) const MCP_PUBLIC_URL_SETTINGS_PATH: &str = "__agena.mcp_server.public_url";
pub(crate) const MCP_OAUTH_ISSUER_URL_SETTINGS_PATH: &str = "__agena.mcp_server.oauth_issuer_url";
pub(crate) const MCP_OAUTH_PASSWORD_SETTINGS_PATH: &str = "__agena.mcp_server.oauth_password";

#[derive(Debug, Clone, Copy)]
pub(crate) enum SettingsFieldKind {
    String,
    Bool,
    Integer,
}

#[derive(Debug, Clone)]
pub(crate) enum SettingsPickerAction {
    EditField(SettingsFieldSpec),
    ToggleMcpServer,
    ToggleMcpAuth,
    ToggleMcpAnonymousAccess,
    ToggleMcpClientRegistration,
    EditMcpPublicUrl,
    EditMcpOAuthIssuerUrl,
    EditMcpOAuthPassword,
    ClearMcpOAuthPassword,
    OpenProviderDefaultModelChooser,
    OpenPermissionApprovalModelChooser,
    OpenProviderList,
    OpenModelCatalogWorkbench,
    OpenProviderClientVersions,
    OpenGlobalPermissionWorkbench,
    OpenWorkspacePermissionWorkbench,
    OpenCurrentSessionPermissionWorkbench,
    OpenSessionEffectivePermissionView(i64),
    OpenPluginWorkbench,
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
    pub(crate) presentation: PermissionRuleStudioPresentation<PermissionRuleStudioAction>,
    pub(crate) editor: Option<PermissionRuleStudioEditor>,
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
    pub(crate) presentation: UserInputPresentation,
}

#[derive(Debug, Clone)]
pub(crate) struct PermissionOverlay {
    pub(crate) session_id: i64,
    pub(crate) request: PermissionRequest,
    pub(crate) presentation: PermissionPromptPresentation,
    /// Live async status of the "auto-approve" choice; `None` when idle.
    pub(crate) auto_approve: Option<PermissionPromptAutoApproveStatus>,
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
pub(crate) enum PermissionOverlayChoice {
    OpenScope(agena_tui::permission_prompt::PermissionPromptDecision),
    Reply {
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
    },
    AutoApprove,
    EditRule,
    Details,
}

pub(crate) type ConfirmOverlay = ConfirmDialogState<ConfirmAction>;

#[derive(Debug, Clone)]
pub(crate) enum ConfirmAction {
    Rewind {
        session_id: i64,
        turn_id: agena_domain::TurnId,
        message_text: String,
        target: String,
    },
    PermissionStudioDeletePathRule {
        pattern: String,
    },
    PermissionStudioDeleteNetworkRule {
        target: String,
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
    SkillStudioDelete {
        name: String,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PathBrowserOverlay {
    pub(crate) presentation: agena_tui::path_browser::PathBrowserPresentation,
    pub(crate) target: PathBrowserTarget,
    pub(crate) path_actions: BTreeMap<String, PathBuf>,
    /// The directory currently being browsed. It is also reflected in the
    /// editable path input whenever directory navigation completes.
    pub(crate) current_directory: PathBuf,
}

/// Display a directory as an editable path prefix. Keeping the trailing path
/// separator means a user can immediately type a child filename or folder
/// name and have the browser resolve it beneath the visible directory.
pub(crate) fn path_browser_directory_input(path: &Path) -> String {
    let mut input = path.display().to_string();
    if !input.ends_with(std::path::MAIN_SEPARATOR) {
        input.push(std::path::MAIN_SEPARATOR);
    }
    input
}

pub(crate) use agena_tui::path_browser::PathBrowserMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathBrowserTarget {
    PermissionRuleStudio(PermissionRuleStudioPathField),
    FileAttachment { images_only: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionRuleStudioPathField {
    WorkspaceRoot,
    TargetPath,
}

pub(crate) use agena_tui::timeline::{TimelineItem, TimelineOverlay, TimelinePresentation};

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
    pub(crate) native_compaction: bool,
    pub(crate) agena_tool_mode: AgenaToolMode,
    pub(crate) display_name: String,
    pub(crate) lifecycle: String,
    pub(crate) context_window_tokens: String,
    pub(crate) max_input_tokens: String,
    pub(crate) max_output_tokens: String,
    pub(crate) input_modalities: BTreeSet<String>,
    pub(crate) features: BTreeSet<String>,
    pub(crate) output_modalities: String,
    pub(crate) supported_thinking_modes: BTreeSet<String>,
    pub(crate) supported_speed_modes: BTreeSet<String>,
    pub(crate) description: String,
    pub(crate) definition: agena_provider::ConfiguredModelDefinition,
}

pub(crate) use agena_tui_provider_studio::ProviderModelConfigField;

pub(crate) type ProviderStudioEditor = EditorDialogState<ProviderStudioEditorAction>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderStudioEditorAction {
    Field(ProviderStudioField),
    NewModel { adapter_id: String },
    ModelField(ProviderModelConfigField),
}

#[derive(Debug, Clone)]
pub(crate) struct ModelCatalogStudioOverlay {
    pub(crate) summary: ModelCatalogResponse,
    pub(crate) presentation: ModelCatalogPresentation,
    pub(crate) editor: Option<LineInputOverlay>,
}
