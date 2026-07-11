//! Central application-level TUI keymap.
//!
//! Page handlers own state transitions, but physical key assignments live in
//! this module. Reusable editor/list mechanics remain in
//! `agena-tui-components`; configurable composer actions are re-exported here
//! so this directory is the discovery point for all Agena TUI input policy.

mod composer;
mod core;
mod plugin;
mod studio;

use crossterm::event::KeyEvent;

pub use self::composer::{ComposerAction, ComposerKeyBindings};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    SettingsStudio,
    AgentStudio,
    PermissionStudio,
    PermissionRuleStudio,
    PathBrowser,
    ProviderStudio,
    ProviderDetail,
    ProviderModel,
    ModelCatalog,
    PluginPolicy,
    PluginList,
    PluginDetail,
    PluginConfig,
    PluginConfigActions,
    PluginConfigSelection,
    PluginDrilldown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Interrupt,
    Close,
    Confirm,
    Activate,
    Back,
    EnterInsert,
    EnterView,
    SearchForward,
    SearchBackward,
    SearchNext,
    SearchPrevious,
    New,
    Continue,
    ModeAll,
    ModeRoots,
    ModeSubtree,
    ModeCycle,
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    PageUp,
    PageDown,
    HalfPageUp,
    HalfPageDown,
    Home,
    End,
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
    Refresh,
    Add,
    Edit,
    Rename,
    Duplicate,
    Browse,
    Details,
    CancelRequest,
    PreviousQuestion,
    NextQuestion,
    PreviousTab,
    NextTab,
    Clear,
    Save,
    SaveAdapter,
    SaveModel,
    SelectAll,
    AuthStart,
    AuthContinue,
    LoadModels,
    DeleteProvider,
    CatalogSearch,
    CatalogRefresh,
    CopyEvent,
    TransportFilter,
    ConfigFilter,
    ShowDiff,
    CloseDiff,
    Validate,
    InsertDefaults,
    Actions,
    Restart,
    Reset,
    SelectType,
    Brief,
    Detailed,
    Summary,
    ClearOverride,
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
        | KeyContext::UserInputReview => core::resolve(context, key),
        KeyContext::SettingsStudio
        | KeyContext::AgentStudio
        | KeyContext::PermissionStudio
        | KeyContext::PermissionRuleStudio
        | KeyContext::PathBrowser
        | KeyContext::ProviderStudio
        | KeyContext::ProviderDetail
        | KeyContext::ProviderModel
        | KeyContext::ModelCatalog => studio::resolve(context, key),
        KeyContext::PluginPolicy
        | KeyContext::PluginList
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn removed_main_shortcuts_are_not_registered() {
        for code in [
            KeyCode::Char('q'),
            KeyCode::Tab,
            KeyCode::BackTab,
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
    fn vim_mode_and_search_keys_are_registered_by_context() {
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
    fn visible_shortcut_hints_track_the_central_keymap() {
        let english = crate::i18n::I18n::english();
        let transcript = crate::ui_text::t(&english, "status-transcript");
        let composer = crate::ui_text::t(&english, "status-composer");
        let global = crate::ui_text::t(&english, "status-global");

        assert!(transcript.contains("i insert"));
        assert!(composer.contains("Esc view"));
        for removed in ["Alt+S", "Alt+P", "q quit"] {
            assert!(!global.contains(removed));
        }
    }
}
