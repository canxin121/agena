//! Central application-level TUI keymap.
//!
//! Page handlers own state transitions, but physical key assignments live in
//! this module. Reusable editor/list mechanics remain in
//! `agena-tui-components`; configurable composer actions are re-exported here
//! so this directory is the discovery point for all Agena TUI input policy.

mod activities;
mod core;
mod plugin;
mod studio;
mod usage;

use crossterm::event::{KeyEvent, KeyModifiers};

fn command_modifiers(key: KeyEvent) -> KeyModifiers {
    key.modifiers
        & (KeyModifiers::CONTROL
            | KeyModifiers::ALT
            | KeyModifiers::SHIFT
            | KeyModifiers::SUPER
            | KeyModifiers::HYPER
            | KeyModifiers::META)
}

pub(super) fn unmodified(key: KeyEvent) -> bool {
    command_modifiers(key).is_empty()
}

pub(super) fn only_ctrl(key: KeyEvent) -> bool {
    command_modifiers(key) == KeyModifiers::CONTROL
}

pub(super) fn only_ctrl_shift(key: KeyEvent) -> bool {
    command_modifiers(key) == (KeyModifiers::CONTROL | KeyModifiers::SHIFT)
}

pub(super) fn only_shift(key: KeyEvent) -> bool {
    command_modifiers(key) == KeyModifiers::SHIFT
}

pub(super) fn unmodified_or_shift(key: KeyEvent) -> bool {
    let modifiers = command_modifiers(key);
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

/// Shared forward/backward navigation for focusable panes and tab strips.
///
/// `BackTab` is what most terminals report for Shift+Tab. Both representations
/// are accepted so backward pane navigation does not depend on Alt/Option.
pub(super) fn tab_navigation_action(key: KeyEvent) -> Option<KeyAction> {
    match key.code {
        crossterm::event::KeyCode::Tab if unmodified(key) => Some(KeyAction::NextTab),
        crossterm::event::KeyCode::Tab if only_shift(key) => Some(KeyAction::PreviousTab),
        crossterm::event::KeyCode::BackTab if unmodified_or_shift(key) => {
            Some(KeyAction::PreviousTab)
        }
        _ => None,
    }
}

/// Shared previous/next-page navigation for secondary paginated surfaces.
///
/// Plain horizontal arrows are reserved for pagination when the surface has
/// neither horizontally editable cells nor side-by-side pane navigation.
pub(super) fn horizontal_pagination_action(key: KeyEvent) -> Option<KeyAction> {
    match key.code {
        crossterm::event::KeyCode::Left if unmodified(key) => Some(KeyAction::PageUp),
        crossterm::event::KeyCode::Right if unmodified(key) => Some(KeyAction::PageDown),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Context of a key binding.
pub enum KeyContext {
    Global,
    Main,
    Sessions,
    Transcript,
    Composer,
    ComposerItem,
    PromptHistory,
    Suggestion,
    Help,
    Choice,
    FileAttach,
    SessionSearch,
    Picker,
    SessionModel,
    Timeline,
    Confirm,
    PermissionPrompt,
    UserInputQuestion,
    UserInputReview,
    Usage,
    Activities,
    PlanViewer,
    Hub,
    SettingsStudio,
    PermissionStudio,
    PermissionRuleStudio,
    PathBrowser,
    ProviderStudio,
    ProviderDetail,
    ProviderModel,
    ModelCatalog,
    PluginList,
    PluginDetail,
    PluginConfig,
    PluginConfigActions,
    PluginConfigSelection,
    PluginDrilldown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Action bound to a key.
pub enum KeyAction {
    Interrupt,
    Help,
    Close,
    Confirm,
    Activate,
    Back,
    EnterInsert,
    EnterView,
    ToggleVisualCharacter,
    ToggleVisualLine,
    ToggleVisualBlock,
    CancelSelection,
    SearchForward,
    SearchBackward,
    SearchNext,
    SearchPrevious,
    New,
    Continue,
    OpenUsage,
    HubCreateSession,
    HubPreviousPage,
    HubNextPage,
    HubTogglePinned,
    OpenPlan,
    ModeAll,
    ModeRoots,
    ModeSubtree,
    ModeCycle,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    LineStart,
    LineFirstNonBlank,
    LineEnd,
    WordForward,
    WordBackward,
    WordEnd,
    WordEndBackward,
    BigWordForward,
    BigWordBackward,
    BigWordEnd,
    BigWordEndBackward,
    FindForward,
    FindBackward,
    FindTillForward,
    FindTillBackward,
    RepeatFind,
    RepeatFindReverse,
    PreviousMessage,
    NextMessage,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    ScrollLineUp,
    ScrollLineDown,
    JumpBackward,
    JumpForward,
    Home,
    End,
    GotoPrefix,
    ViewportPrefix,
    ViewportTop,
    ViewportCenter,
    ViewportBottom,
    ViewTop,
    ViewMiddle,
    ViewBottom,
    SwapVisualEndpoint,
    SwapVisualBlockCorner,
    AroundTextObject,
    InnerTextObject,
    TextObjectMarkdown,
    TextObjectMessage,
    TextObjectParagraph,
    YankLine,
    Toggle,
    ToggleFavorite,
    ToggleSessionWork,
    ExpandAll,
    Copy,
    CopyVisible,
    CopyAll,
    CopyLast,
    CountDigit(u8),
    Fill,
    Previous,
    Next,
    Delete,
    Open,
    Accept,
    Older,
    Newer,
    NewerKeepOpen,
    Edit,
    CancelRequest,
    NextTab,
    PreviousTab,
    Clear,
    Refresh,
    SettingsSearch,
    SettingsDetails,
    PlanToggleAutorun,
    ProviderRefreshModels,
    ProviderAddModel,
    ProviderSaveAdapter,
    ProviderSave,
    UsageCyclePeriod,
    UsageCycleView,
    UsageCycleProvider,
    UsageCycleModel,
    UsageToggleSubagents,
    UsageCycleSort,
    ActivitiesToggleFinished,
    ActivitiesCycleKind,
    ActivitiesCycleStatus,
    ActivitiesStop,
    ActivitiesDismiss,
    ActivitiesClearFinished,
    ModelCatalogSearch,
    PluginCycleTransport,
    PluginCycleConfig,
    PluginValidate,
    PluginReset,
    PluginDiff,
    PluginSave,
    PluginRestart,
    PermissionAdd,
    PermissionRename,
    PermissionBrowse,
    PermissionSave,
}

pub fn resolve(context: KeyContext, key: KeyEvent) -> Option<KeyAction> {
    match context {
        KeyContext::Global
        | KeyContext::Main
        | KeyContext::Sessions
        | KeyContext::Transcript
        | KeyContext::Composer
        | KeyContext::ComposerItem
        | KeyContext::PromptHistory
        | KeyContext::Suggestion
        | KeyContext::Help
        | KeyContext::Choice
        | KeyContext::FileAttach
        | KeyContext::SessionSearch
        | KeyContext::Picker
        | KeyContext::SessionModel
        | KeyContext::Timeline
        | KeyContext::Confirm
        | KeyContext::PermissionPrompt
        | KeyContext::UserInputQuestion
        | KeyContext::UserInputReview
        | KeyContext::PlanViewer
        | KeyContext::Hub => core::resolve(context, key),
        KeyContext::Usage => usage::resolve(key),
        KeyContext::Activities => activities::resolve(key),
        KeyContext::SettingsStudio
        | KeyContext::PermissionStudio
        | KeyContext::PermissionRuleStudio
        | KeyContext::PathBrowser
        | KeyContext::ProviderStudio
        | KeyContext::ProviderDetail
        | KeyContext::ProviderModel
        | KeyContext::ModelCatalog => studio::resolve(context, key),
        KeyContext::PluginList
        | KeyContext::PluginDetail
        | KeyContext::PluginConfig
        | KeyContext::PluginConfigActions
        | KeyContext::PluginConfigSelection
        | KeyContext::PluginDrilldown => plugin::resolve(context, key),
    }
}
