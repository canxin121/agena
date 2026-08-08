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
        | KeyContext::PlanViewer => core::resolve(context, key),
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

#[cfg(test)]
mod tests {
    use super::{KeyAction, KeyContext, resolve};
    use crate::input::{ComposerAction, ComposerKeyBindings};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn removed_main_shortcuts_are_not_registered() {
        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('s'),
            KeyCode::Char('b'),
            KeyCode::Char('R'),
            KeyCode::Char('t'),
            KeyCode::Char('P'),
            KeyCode::Char('['),
            KeyCode::Char(']'),
            KeyCode::Char('e'),
            KeyCode::Char('v'),
            KeyCode::Char('u'),
        ] {
            assert_eq!(
                resolve(KeyContext::Main, key(code, KeyModifiers::NONE)),
                None
            );
        }
        assert_eq!(
            resolve(KeyContext::Main, key(KeyCode::Char('s'), KeyModifiers::ALT)),
            None
        );
        assert_eq!(
            resolve(KeyContext::Main, key(KeyCode::Char('p'), KeyModifiers::ALT)),
            None
        );
    }

    #[test]
    fn secondary_tab_contexts_share_forward_and_backward_navigation() {
        for context in [
            KeyContext::SettingsStudio,
            KeyContext::PermissionStudio,
            KeyContext::UserInputQuestion,
            KeyContext::UserInputReview,
            KeyContext::ProviderStudio,
            KeyContext::PluginDetail,
            KeyContext::PluginConfig,
        ] {
            assert_eq!(
                resolve(context, key(KeyCode::Tab, KeyModifiers::NONE)),
                Some(KeyAction::NextTab),
                "{context:?} must move forward with Tab",
            );
            for event in [
                key(KeyCode::Tab, KeyModifiers::SHIFT),
                key(KeyCode::BackTab, KeyModifiers::SHIFT),
            ] {
                assert_eq!(
                    resolve(context, event),
                    Some(KeyAction::PreviousTab),
                    "{context:?} must expose backward pane navigation",
                );
            }
            assert_eq!(
                resolve(context, key(KeyCode::Tab, KeyModifiers::ALT)),
                None,
                "{context:?} must not depend on Alt/Option for backward navigation",
            );
            assert_eq!(
                resolve(
                    context,
                    key(KeyCode::Tab, KeyModifiers::ALT | KeyModifiers::SHIFT),
                ),
                None,
                "{context:?} must reject ambiguous Tab modifiers",
            );
        }
    }

    #[test]
    fn vim_mode_and_search_keys_are_registered_by_context() {
        assert_eq!(
            resolve(
                KeyContext::Global,
                key(KeyCode::Char('h'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::Help)
        );
        assert_eq!(
            resolve(
                KeyContext::Global,
                key(KeyCode::Char('\u{0008}'), KeyModifiers::NONE)
            ),
            Some(KeyAction::Help)
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('i'), KeyModifiers::NONE)
            ),
            Some(KeyAction::EnterInsert)
        );
        assert_eq!(
            resolve(KeyContext::Composer, key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(KeyAction::EnterView)
        );
        assert_eq!(
            resolve(
                KeyContext::Main,
                key(KeyCode::Char('/'), KeyModifiers::NONE)
            ),
            Some(KeyAction::SearchForward)
        );
        assert_eq!(
            resolve(
                KeyContext::Main,
                key(KeyCode::Char('?'), KeyModifiers::SHIFT)
            ),
            Some(KeyAction::SearchBackward)
        );
    }

    #[test]
    fn transcript_arrow_and_vim_keys_resolve_to_the_same_actions() {
        for (arrow, vim) in [
            (KeyCode::Left, 'h'),
            (KeyCode::Right, 'l'),
            (KeyCode::Up, 'k'),
            (KeyCode::Down, 'j'),
        ] {
            assert_eq!(
                resolve(KeyContext::Transcript, key(arrow, KeyModifiers::NONE)),
                resolve(
                    KeyContext::Transcript,
                    key(KeyCode::Char(vim), KeyModifiers::NONE)
                )
            );
        }
    }

    #[test]
    fn transcript_reserves_ctrl_k_and_ctrl_j_for_message_jumps() {
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('k'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::PreviousMessage)
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('j'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::NextMessage)
        );
        // Ctrl+H/L are no longer bound to message jumps in the transcript.
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('h'), KeyModifiers::CONTROL)
            ),
            None
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('l'), KeyModifiers::CONTROL)
            ),
            None
        );
    }

    #[test]
    fn transcript_registers_vim_paging_scroll_and_jump_history_keys() {
        // Ctrl+F joins PageDown as the Vim forward-page chord.
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('f'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::PageDown)
        );
        // Ctrl+E/Y scroll the viewport one line without moving the cursor.
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('e'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::ScrollLineDown)
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('y'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::ScrollLineUp)
        );
        // Ctrl+O/I step through the transcript position history.
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('o'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::JumpBackward)
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('i'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::JumpForward)
        );
        // The same letters keep their plain Vim meanings without Ctrl.
        for (code, action) in [
            (KeyCode::Char('f'), KeyAction::FindForward),
            (KeyCode::Char('e'), KeyAction::WordEnd),
            (KeyCode::Char('y'), KeyAction::Copy),
            (KeyCode::Char('o'), KeyAction::SwapVisualEndpoint),
            (KeyCode::Char('i'), KeyAction::EnterInsert),
        ] {
            assert_eq!(
                resolve(KeyContext::Transcript, key(code, KeyModifiers::NONE)),
                Some(action),
                "{code:?} should keep its plain Transcript Vim mapping",
            );
        }
    }

    #[test]
    fn transcript_registers_vim_visual_mode_keys() {
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('v'), KeyModifiers::NONE)
            ),
            Some(KeyAction::ToggleVisualCharacter)
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('V'), KeyModifiers::SHIFT)
            ),
            Some(KeyAction::ToggleVisualLine)
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Char('v'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::ToggleVisualBlock)
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Esc, KeyModifiers::NONE)
            ),
            Some(KeyAction::CancelSelection)
        );
    }

    #[test]
    fn transcript_registers_vim_motion_prefixes_and_text_objects() {
        for (code, modifiers, action) in [
            (KeyCode::Char('0'), KeyModifiers::NONE, KeyAction::LineStart),
            (
                KeyCode::Char('^'),
                KeyModifiers::SHIFT,
                KeyAction::LineFirstNonBlank,
            ),
            (KeyCode::Char('$'), KeyModifiers::SHIFT, KeyAction::LineEnd),
            (
                KeyCode::Char('w'),
                KeyModifiers::NONE,
                KeyAction::WordForward,
            ),
            (
                KeyCode::Char('b'),
                KeyModifiers::NONE,
                KeyAction::WordBackward,
            ),
            (KeyCode::Char('e'), KeyModifiers::NONE, KeyAction::WordEnd),
            (
                KeyCode::Char('g'),
                KeyModifiers::NONE,
                KeyAction::GotoPrefix,
            ),
            (
                KeyCode::Char('z'),
                KeyModifiers::NONE,
                KeyAction::ViewportPrefix,
            ),
            (
                KeyCode::Char('f'),
                KeyModifiers::NONE,
                KeyAction::FindForward,
            ),
            (
                KeyCode::Char('F'),
                KeyModifiers::SHIFT,
                KeyAction::FindBackward,
            ),
            (
                KeyCode::Char('t'),
                KeyModifiers::NONE,
                KeyAction::FindTillForward,
            ),
            (
                KeyCode::Char('T'),
                KeyModifiers::SHIFT,
                KeyAction::FindTillBackward,
            ),
            (
                KeyCode::Char(';'),
                KeyModifiers::NONE,
                KeyAction::RepeatFind,
            ),
            (
                KeyCode::Char(','),
                KeyModifiers::NONE,
                KeyAction::RepeatFindReverse,
            ),
            (
                KeyCode::Char('o'),
                KeyModifiers::NONE,
                KeyAction::SwapVisualEndpoint,
            ),
            (
                KeyCode::Char('O'),
                KeyModifiers::SHIFT,
                KeyAction::SwapVisualBlockCorner,
            ),
            (
                KeyCode::Char('a'),
                KeyModifiers::NONE,
                KeyAction::AroundTextObject,
            ),
            (
                KeyCode::Char('m'),
                KeyModifiers::NONE,
                KeyAction::TextObjectMarkdown,
            ),
            (
                KeyCode::Char('M'),
                KeyModifiers::SHIFT,
                KeyAction::TextObjectMessage,
            ),
            (
                KeyCode::Char('p'),
                KeyModifiers::NONE,
                KeyAction::TextObjectParagraph,
            ),
            (KeyCode::Char('Y'), KeyModifiers::SHIFT, KeyAction::YankLine),
        ] {
            assert_eq!(
                resolve(KeyContext::Transcript, key(code, modifiers)),
                Some(action),
                "{code:?} should keep its Transcript Vim mapping",
            );
        }
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::Home, KeyModifiers::NONE)
            ),
            Some(KeyAction::LineStart)
        );
        assert_eq!(
            resolve(
                KeyContext::Transcript,
                key(KeyCode::End, KeyModifiers::NONE)
            ),
            Some(KeyAction::LineEnd)
        );
    }

    #[test]
    fn chat_surface_keeps_its_existing_extended_navigation() {
        for (context, code, action) in [
            (KeyContext::Sessions, KeyCode::PageUp, KeyAction::PageUp),
            (KeyContext::Sessions, KeyCode::PageDown, KeyAction::PageDown),
            (KeyContext::Sessions, KeyCode::Home, KeyAction::Home),
            (KeyContext::Sessions, KeyCode::End, KeyAction::End),
            (KeyContext::Transcript, KeyCode::PageUp, KeyAction::PageUp),
            (
                KeyContext::Transcript,
                KeyCode::PageDown,
                KeyAction::PageDown,
            ),
            (KeyContext::Transcript, KeyCode::Home, KeyAction::LineStart),
            (KeyContext::Transcript, KeyCode::End, KeyAction::LineEnd),
        ] {
            assert_eq!(
                resolve(context, key(code, KeyModifiers::NONE)),
                Some(action)
            );
        }
        assert_eq!(
            resolve(
                KeyContext::ComposerItem,
                key(KeyCode::BackTab, KeyModifiers::SHIFT)
            ),
            Some(KeyAction::Previous)
        );
        assert_eq!(
            resolve(
                KeyContext::ComposerItem,
                key(KeyCode::Backspace, KeyModifiers::NONE)
            ),
            None
        );
    }

    #[test]
    fn usage_dashboard_keys_are_registered_by_context() {
        assert_eq!(
            resolve(
                KeyContext::Main,
                key(KeyCode::Char('U'), KeyModifiers::SHIFT)
            ),
            Some(KeyAction::OpenUsage)
        );
        assert_eq!(
            resolve(KeyContext::Usage, key(KeyCode::Up, KeyModifiers::NONE)),
            Some(KeyAction::MoveUp)
        );
        assert_eq!(
            resolve(KeyContext::Usage, key(KeyCode::Tab, KeyModifiers::NONE)),
            None
        );
        for character in ['1', '3', 'a', 'h', 'k', 'm', 'p', 'q', 'r', 's'] {
            assert_eq!(
                resolve(
                    KeyContext::Usage,
                    key(KeyCode::Char(character), KeyModifiers::NONE)
                ),
                None
            );
        }
    }

    #[test]
    fn page_level_actions_use_direct_modified_shortcuts() {
        for (context, code, modifiers, action) in [
            (
                KeyContext::Usage,
                KeyCode::Char('p'),
                KeyModifiers::CONTROL,
                KeyAction::UsageCyclePeriod,
            ),
            (
                KeyContext::Usage,
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
                KeyAction::Refresh,
            ),
            (
                KeyContext::Usage,
                KeyCode::Char('b'),
                KeyModifiers::CONTROL,
                KeyAction::UsageCycleView,
            ),
            (
                KeyContext::Usage,
                KeyCode::Char('o'),
                KeyModifiers::CONTROL,
                KeyAction::UsageCycleProvider,
            ),
            (
                KeyContext::Usage,
                KeyCode::Char('l'),
                KeyModifiers::CONTROL,
                KeyAction::UsageCycleModel,
            ),
            (
                KeyContext::Usage,
                KeyCode::Char('a'),
                KeyModifiers::CONTROL,
                KeyAction::UsageToggleSubagents,
            ),
            (
                KeyContext::Usage,
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
                KeyAction::UsageCycleSort,
            ),
            (
                KeyContext::ModelCatalog,
                KeyCode::Char('f'),
                KeyModifiers::CONTROL,
                KeyAction::ModelCatalogSearch,
            ),
            (
                KeyContext::ModelCatalog,
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
                KeyAction::Refresh,
            ),
            (
                KeyContext::ModelCatalog,
                KeyCode::Left,
                KeyModifiers::NONE,
                KeyAction::PageUp,
            ),
            (
                KeyContext::ModelCatalog,
                KeyCode::Right,
                KeyModifiers::NONE,
                KeyAction::PageDown,
            ),
            (
                KeyContext::PluginList,
                KeyCode::Char('t'),
                KeyModifiers::CONTROL,
                KeyAction::PluginCycleTransport,
            ),
            (
                KeyContext::PluginList,
                KeyCode::Char('g'),
                KeyModifiers::CONTROL,
                KeyAction::PluginCycleConfig,
            ),
            (
                KeyContext::PluginList,
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
                KeyAction::Refresh,
            ),
            (
                KeyContext::PluginConfig,
                KeyCode::Char('k'),
                KeyModifiers::CONTROL,
                KeyAction::PluginValidate,
            ),
            (
                KeyContext::PluginConfig,
                KeyCode::Char('u'),
                KeyModifiers::CONTROL,
                KeyAction::PluginReset,
            ),
            (
                KeyContext::PluginConfig,
                KeyCode::Char('p'),
                KeyModifiers::CONTROL,
                KeyAction::PluginDiff,
            ),
            (
                KeyContext::PluginConfig,
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
                KeyAction::PluginSave,
            ),
            (
                KeyContext::PluginConfig,
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
                KeyAction::PluginRestart,
            ),
            (
                KeyContext::PermissionStudio,
                KeyCode::Char('n'),
                KeyModifiers::CONTROL,
                KeyAction::PermissionAdd,
            ),
            (
                KeyContext::PermissionStudio,
                KeyCode::Char('e'),
                KeyModifiers::CONTROL,
                KeyAction::PermissionRename,
            ),
            (
                KeyContext::PermissionStudio,
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
                KeyAction::Delete,
            ),
            (
                KeyContext::PermissionRuleStudio,
                KeyCode::Char('o'),
                KeyModifiers::CONTROL,
                KeyAction::PermissionBrowse,
            ),
            (
                KeyContext::PermissionRuleStudio,
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
                KeyAction::PermissionSave,
            ),
        ] {
            assert_eq!(resolve(context, key(code, modifiers)), Some(action));
        }
    }

    #[test]
    fn settings_studio_uses_directional_and_tab_pane_navigation() {
        assert_eq!(
            resolve(
                KeyContext::SettingsStudio,
                key(KeyCode::Left, KeyModifiers::NONE)
            ),
            Some(KeyAction::MoveLeft)
        );
        assert_eq!(
            resolve(
                KeyContext::SettingsStudio,
                key(KeyCode::Right, KeyModifiers::NONE)
            ),
            Some(KeyAction::MoveRight)
        );
        assert_eq!(
            resolve(
                KeyContext::SettingsStudio,
                key(KeyCode::Tab, KeyModifiers::NONE)
            ),
            Some(KeyAction::NextTab)
        );
        assert_eq!(
            resolve(
                KeyContext::SettingsStudio,
                key(KeyCode::Char('r'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::Refresh)
        );
    }

    #[test]
    fn unexpected_modifiers_do_not_trigger_plain_page_actions() {
        for (context, code, modifiers) in [
            (
                KeyContext::Global,
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            (
                KeyContext::Sessions,
                KeyCode::Char('k'),
                KeyModifiers::CONTROL,
            ),
            (KeyContext::Transcript, KeyCode::Enter, KeyModifiers::ALT),
            (
                KeyContext::SettingsStudio,
                KeyCode::Char('r'),
                KeyModifiers::ALT,
            ),
            (KeyContext::PluginList, KeyCode::Down, KeyModifiers::SHIFT),
            (KeyContext::Usage, KeyCode::Up, KeyModifiers::SHIFT),
        ] {
            assert_eq!(resolve(context, key(code, modifiers)), None);
        }

        assert_eq!(
            resolve(
                KeyContext::PluginConfig,
                key(KeyCode::Char('d'), KeyModifiers::ALT)
            ),
            None
        );
        assert_eq!(
            resolve(
                KeyContext::PromptHistory,
                key(KeyCode::Char('c'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::Close)
        );
        assert_eq!(
            resolve(
                KeyContext::PromptHistory,
                key(KeyCode::Up, KeyModifiers::NONE)
            ),
            Some(KeyAction::Previous)
        );
        assert_eq!(
            resolve(
                KeyContext::PromptHistory,
                key(KeyCode::Down, KeyModifiers::NONE)
            ),
            Some(KeyAction::Next)
        );
        assert_eq!(
            resolve(
                KeyContext::PromptHistory,
                key(KeyCode::Right, KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            resolve(
                KeyContext::Suggestion,
                key(KeyCode::Left, KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            resolve(
                KeyContext::PromptHistory,
                key(KeyCode::PageDown, KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            ComposerKeyBindings::default().match_action(&key(
                KeyCode::Enter,
                KeyModifiers::CONTROL | KeyModifiers::SUPER,
            )),
            None
        );
    }

    #[test]
    fn searchable_pages_leave_printable_characters_for_the_editor() {
        for context in [
            KeyContext::Choice,
            KeyContext::FileAttach,
            KeyContext::SessionSearch,
            KeyContext::Picker,
            KeyContext::SessionModel,
            KeyContext::Timeline,
            KeyContext::PathBrowser,
            KeyContext::PluginList,
        ] {
            for byte in b' '..=b'~' {
                let character = char::from(byte);
                assert_eq!(
                    resolve(context, key(KeyCode::Char(character), KeyModifiers::NONE)),
                    None,
                    "{context:?} must leave {character:?} for search input"
                );
            }
        }

        assert_eq!(
            resolve(
                KeyContext::SessionSearch,
                key(KeyCode::Left, KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            resolve(
                KeyContext::PathBrowser,
                key(KeyCode::Left, KeyModifiers::NONE)
            ),
            Some(KeyAction::MoveLeft)
        );
        assert_eq!(
            resolve(
                KeyContext::PathBrowser,
                key(KeyCode::Right, KeyModifiers::NONE)
            ),
            Some(KeyAction::MoveRight)
        );
        assert_eq!(
            resolve(
                KeyContext::Timeline,
                key(KeyCode::Char('y'), KeyModifiers::CONTROL)
            ),
            None
        );
    }

    #[test]
    fn overloaded_main_and_composer_keys_have_distinct_chords() {
        assert_eq!(
            resolve(
                KeyContext::Main,
                key(KeyCode::Char('n'), KeyModifiers::NONE)
            ),
            Some(KeyAction::SearchNext)
        );
        assert_eq!(
            resolve(
                KeyContext::Main,
                key(KeyCode::Char('n'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::New)
        );
        assert_eq!(
            resolve(KeyContext::Composer, key(KeyCode::Up, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            resolve(KeyContext::Composer, key(KeyCode::Up, KeyModifiers::ALT)),
            None
        );
        let bindings = ComposerKeyBindings::default();
        assert_eq!(
            bindings.match_action(&key(KeyCode::Up, KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            bindings.match_action(&key(KeyCode::Char('p'), KeyModifiers::CONTROL)),
            Some(ComposerAction::EditQueue)
        );
        assert_eq!(
            bindings.match_action(&key(KeyCode::Up, KeyModifiers::CONTROL)),
            None
        );
        assert_eq!(
            bindings.match_action(&key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(ComposerAction::OpenPendingUserInput)
        );
        assert_eq!(
            bindings.match_action(&key(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            Some(ComposerAction::OpenPendingPermission)
        );
        assert_eq!(
            bindings.match_action(&key(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            Some(ComposerAction::FocusItems)
        );
        assert_eq!(
            bindings.match_action(&key(KeyCode::Char('e'), KeyModifiers::CONTROL)),
            Some(ComposerAction::ExternalEditor)
        );
        assert_eq!(
            bindings.match_action(&key(KeyCode::Char('t'), KeyModifiers::CONTROL)),
            Some(ComposerAction::AttachImage)
        );
        // Composer bindings must not rely on F-keys or Alt/Option.
        for number in 1..=12u8 {
            assert_eq!(
                bindings.match_action(&key(KeyCode::F(number), KeyModifiers::NONE)),
                None,
                "F{number} must not be a composer binding",
            );
        }
        for character in (b'a'..=b'z').map(char::from) {
            assert_eq!(
                bindings.match_action(&key(KeyCode::Char(character), KeyModifiers::ALT)),
                None,
                "Alt+{character} must not be a composer binding",
            );
        }
    }

    #[test]
    fn confirmation_dialog_accepts_the_visible_y_shortcut() {
        for event in [
            key(KeyCode::Char('y'), KeyModifiers::NONE),
            key(KeyCode::Char('Y'), KeyModifiers::SHIFT),
            key(KeyCode::Enter, KeyModifiers::NONE),
        ] {
            assert_eq!(
                resolve(KeyContext::Confirm, event),
                Some(KeyAction::Confirm)
            );
        }
        assert_eq!(
            resolve(
                KeyContext::Confirm,
                key(KeyCode::Char('y'), KeyModifiers::CONTROL)
            ),
            None
        );
    }

    #[test]
    fn plugin_search_picker_overlays_share_page_navigation() {
        for context in [
            KeyContext::PluginConfigActions,
            KeyContext::PluginConfigSelection,
        ] {
            assert_eq!(
                resolve(context, key(KeyCode::Left, KeyModifiers::NONE)),
                Some(KeyAction::PageUp)
            );
            assert_eq!(
                resolve(context, key(KeyCode::Right, KeyModifiers::NONE)),
                Some(KeyAction::PageDown)
            );
            assert_eq!(
                resolve(context, key(KeyCode::PageDown, KeyModifiers::NONE)),
                None
            );
        }
    }

    #[test]
    fn plugin_config_uses_one_unambiguous_context() {
        for (code, modifiers, action) in [
            (KeyCode::Esc, KeyModifiers::NONE, Some(KeyAction::Back)),
            (KeyCode::Tab, KeyModifiers::NONE, Some(KeyAction::NextTab)),
            (KeyCode::Enter, KeyModifiers::NONE, Some(KeyAction::Edit)),
            (
                KeyCode::Char('d'),
                KeyModifiers::CONTROL,
                Some(KeyAction::Delete),
            ),
            (
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
                Some(KeyAction::PluginRestart),
            ),
            (KeyCode::Delete, KeyModifiers::NONE, None),
            (KeyCode::F(5), KeyModifiers::NONE, None),
            (KeyCode::Char('r'), KeyModifiers::NONE, None),
            (KeyCode::Char('R'), KeyModifiers::SHIFT, None),
            (KeyCode::Char('d'), KeyModifiers::NONE, None),
            (KeyCode::Char('D'), KeyModifiers::SHIFT, None),
            (KeyCode::Char('h'), KeyModifiers::CONTROL, None),
        ] {
            assert_eq!(
                resolve(KeyContext::PluginConfig, key(code, modifiers)),
                action
            );
        }
    }

    #[test]
    fn provider_non_delete_actions_require_explicit_control_shortcuts() {
        for (character, action) in [
            ('r', KeyAction::ProviderRefreshModels),
            ('n', KeyAction::ProviderAddModel),
            ('a', KeyAction::ProviderSaveAdapter),
            ('s', KeyAction::ProviderSave),
        ] {
            assert_eq!(
                resolve(
                    KeyContext::ProviderStudio,
                    key(KeyCode::Char(character), KeyModifiers::CONTROL),
                ),
                Some(action),
            );
            assert_eq!(
                resolve(
                    KeyContext::ProviderStudio,
                    key(KeyCode::Char(character), KeyModifiers::NONE),
                ),
                None,
            );
        }
        assert_eq!(
            resolve(
                KeyContext::ProviderModel,
                key(KeyCode::Char('s'), KeyModifiers::CONTROL),
            ),
            Some(KeyAction::ProviderSave),
        );
    }

    #[test]
    fn deletable_selections_share_ctrl_d_without_a_delete_key_dependency() {
        for context in [
            KeyContext::ComposerItem,
            KeyContext::PermissionStudio,
            KeyContext::PermissionRuleStudio,
            KeyContext::ProviderStudio,
            KeyContext::ProviderModel,
            KeyContext::PluginConfig,
            KeyContext::PluginDrilldown,
        ] {
            assert_eq!(
                resolve(context, key(KeyCode::Char('d'), KeyModifiers::CONTROL),),
                Some(KeyAction::Delete),
                "{context:?} must use the shared Ctrl+D delete shortcut",
            );
            assert_eq!(
                resolve(context, key(KeyCode::Delete, KeyModifiers::NONE)),
                None,
                "{context:?} must not require a physical Delete key",
            );
        }

        for character in ['k', 'x'] {
            assert_eq!(
                resolve(
                    KeyContext::ProviderStudio,
                    key(KeyCode::Char(character), KeyModifiers::CONTROL),
                ),
                None,
                "Ctrl+{character} must not retain an entity-specific delete action",
            );
        }
        for code in [KeyCode::Backspace, KeyCode::Char('d')] {
            assert_eq!(
                resolve(KeyContext::ComposerItem, key(code, KeyModifiers::NONE)),
                None,
                "composer items must not retain an alternate delete shortcut",
            );
        }
    }

    #[test]
    fn permission_studios_use_direct_shortcuts_instead_of_an_action_bar() {
        for (code, modifiers, action) in [
            (
                KeyCode::Char('n'),
                KeyModifiers::CONTROL,
                KeyAction::PermissionAdd,
            ),
            (
                KeyCode::Char('e'),
                KeyModifiers::CONTROL,
                KeyAction::PermissionRename,
            ),
        ] {
            assert_eq!(
                resolve(KeyContext::PermissionStudio, key(code, modifiers)),
                Some(action),
            );
        }
        assert_eq!(
            resolve(
                KeyContext::PermissionRuleStudio,
                key(KeyCode::Char('s'), KeyModifiers::CONTROL),
            ),
            Some(KeyAction::PermissionSave),
        );
        assert_eq!(
            resolve(
                KeyContext::PermissionRuleStudio,
                key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            ),
            Some(KeyAction::PermissionBrowse),
        );
    }

    #[test]
    fn secondary_pages_have_no_plain_character_commands() {
        for context in [
            KeyContext::Help,
            KeyContext::Usage,
            KeyContext::SettingsStudio,
            KeyContext::PermissionStudio,
            KeyContext::PermissionRuleStudio,
            KeyContext::ProviderStudio,
            KeyContext::ProviderDetail,
            KeyContext::ProviderModel,
            KeyContext::ModelCatalog,
            KeyContext::PluginList,
            KeyContext::PluginDetail,
            KeyContext::PluginConfig,
            KeyContext::PluginConfigActions,
            KeyContext::PluginConfigSelection,
            KeyContext::PluginDrilldown,
        ] {
            for character in '!'..='~' {
                assert_eq!(
                    resolve(context, key(KeyCode::Char(character), KeyModifiers::NONE)),
                    None,
                    "{context:?} still binds printable character {character:?}"
                );
            }
        }
    }

    #[test]
    fn secondary_surfaces_do_not_require_alt_function_or_delete_keys() {
        for context in [
            KeyContext::ComposerItem,
            KeyContext::UserInputQuestion,
            KeyContext::UserInputReview,
            KeyContext::Usage,
            KeyContext::SettingsStudio,
            KeyContext::PermissionStudio,
            KeyContext::PermissionRuleStudio,
            KeyContext::PathBrowser,
            KeyContext::ProviderStudio,
            KeyContext::ProviderDetail,
            KeyContext::ProviderModel,
            KeyContext::ModelCatalog,
            KeyContext::PluginList,
            KeyContext::PluginDetail,
            KeyContext::PluginConfig,
            KeyContext::PluginConfigActions,
            KeyContext::PluginConfigSelection,
            KeyContext::PluginDrilldown,
        ] {
            assert_eq!(
                resolve(context, key(KeyCode::Delete, KeyModifiers::NONE)),
                None,
                "{context:?} still depends on a physical Delete key",
            );
            for number in 1..=12 {
                assert_eq!(
                    resolve(context, key(KeyCode::F(number), KeyModifiers::NONE)),
                    None,
                    "{context:?} still depends on F{number}",
                );
            }
            for character in (b'a'..=b'z').map(char::from) {
                assert_eq!(
                    resolve(context, key(KeyCode::Char(character), KeyModifiers::ALT),),
                    None,
                    "{context:?} still depends on Alt/Option+{character}",
                );
            }
            assert_eq!(
                resolve(context, key(KeyCode::Tab, KeyModifiers::ALT)),
                None,
                "{context:?} still depends on Alt/Option+Tab",
            );
        }
    }

    #[test]
    fn secondary_pages_reject_redundant_navigation_keys() {
        let contexts = [
            KeyContext::Help,
            KeyContext::Usage,
            KeyContext::SettingsStudio,
            KeyContext::PermissionStudio,
            KeyContext::PermissionRuleStudio,
            KeyContext::PathBrowser,
            KeyContext::ProviderStudio,
            KeyContext::ProviderDetail,
            KeyContext::ProviderModel,
            KeyContext::ModelCatalog,
            KeyContext::PluginList,
            KeyContext::PluginDetail,
            KeyContext::PluginConfig,
            KeyContext::PluginConfigActions,
            KeyContext::PluginConfigSelection,
            KeyContext::PluginDrilldown,
        ];
        for context in contexts {
            for code in [
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::Home,
                KeyCode::End,
                KeyCode::Backspace,
            ] {
                assert_eq!(
                    resolve(context, key(code, KeyModifiers::NONE)),
                    None,
                    "{context:?} still binds redundant key {code:?}"
                );
            }
        }

        for context in [KeyContext::PermissionPrompt, KeyContext::UserInputQuestion] {
            for code in [
                KeyCode::Left,
                KeyCode::Right,
                KeyCode::PageUp,
                KeyCode::PageDown,
                KeyCode::Home,
                KeyCode::End,
                KeyCode::Backspace,
            ] {
                assert_eq!(
                    resolve(context, key(code, KeyModifiers::NONE)),
                    None,
                    "{context:?} still binds redundant key {code:?}"
                );
            }
        }

        for context in [
            KeyContext::Help,
            KeyContext::Usage,
            KeyContext::PermissionPrompt,
            KeyContext::PermissionRuleStudio,
            KeyContext::PathBrowser,
            KeyContext::ProviderDetail,
            KeyContext::ProviderModel,
            KeyContext::ModelCatalog,
            KeyContext::PluginList,
            KeyContext::PluginConfigActions,
            KeyContext::PluginConfigSelection,
            KeyContext::PluginDrilldown,
        ] {
            assert_eq!(
                resolve(context, key(KeyCode::BackTab, KeyModifiers::SHIFT),),
                None,
                "{context:?} has no focus ring for backward Tab navigation",
            );
        }
    }

    #[test]
    fn user_input_uses_ctrl_d_to_clear_and_ctrl_x_to_cancel() {
        for context in [KeyContext::UserInputQuestion, KeyContext::UserInputReview] {
            assert_eq!(
                resolve(context, key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
                Some(KeyAction::Clear)
            );
            assert_eq!(
                resolve(context, key(KeyCode::Char('x'), KeyModifiers::CONTROL)),
                Some(KeyAction::CancelRequest)
            );
        }
    }

    #[test]
    fn plan_viewer_context_registers_navigation_refresh_autorun_and_close() {
        for (code, action) in [
            (KeyCode::Esc, KeyAction::Close),
            (KeyCode::Char('q'), KeyAction::Close),
            (KeyCode::Up, KeyAction::MoveUp),
            (KeyCode::Char('k'), KeyAction::MoveUp),
            (KeyCode::Down, KeyAction::MoveDown),
            (KeyCode::Char('j'), KeyAction::MoveDown),
            (KeyCode::PageUp, KeyAction::PageUp),
            (KeyCode::PageDown, KeyAction::PageDown),
            (KeyCode::Char('r'), KeyAction::Refresh),
            (KeyCode::Char('a'), KeyAction::PlanToggleAutorun),
        ] {
            assert_eq!(
                resolve(KeyContext::PlanViewer, key(code, KeyModifiers::NONE)),
                Some(action),
                "{code:?} must resolve in PlanViewer context",
            );
        }
        for (code, modifiers, action) in [
            (KeyCode::Char('r'), KeyModifiers::SHIFT, KeyAction::Refresh),
            (
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
                KeyAction::Refresh,
            ),
            (
                KeyCode::Char('a'),
                KeyModifiers::CONTROL,
                KeyAction::PlanToggleAutorun,
            ),
        ] {
            assert_eq!(
                resolve(KeyContext::PlanViewer, key(code, modifiers)),
                Some(action),
                "{code:?} with {modifiers:?} must resolve in PlanViewer context",
            );
        }
        for event in [
            key(KeyCode::Char('a'), KeyModifiers::ALT),
            key(KeyCode::Char('p'), KeyModifiers::NONE),
        ] {
            assert_eq!(
                resolve(KeyContext::PlanViewer, event),
                None,
                "{event:?} must not resolve in PlanViewer context",
            );
        }
    }

    #[test]
    fn main_context_ctrl_p_opens_the_plan_viewer() {
        assert_eq!(
            resolve(
                KeyContext::Main,
                key(KeyCode::Char('p'), KeyModifiers::CONTROL)
            ),
            Some(KeyAction::OpenPlan)
        );
        assert_eq!(
            resolve(
                KeyContext::Main,
                key(KeyCode::Char('p'), KeyModifiers::NONE)
            ),
            None
        );
        assert_eq!(
            resolve(KeyContext::Main, key(KeyCode::Char('p'), KeyModifiers::ALT)),
            None
        );
    }
}
