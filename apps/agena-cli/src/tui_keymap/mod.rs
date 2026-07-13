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
mod usage;

use crossterm::event::{KeyEvent, KeyModifiers};

pub use self::composer::{ComposerAction, ComposerKeyBindings};

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

pub(super) fn only_alt(key: KeyEvent) -> bool {
    command_modifiers(key) == KeyModifiers::ALT
}

pub(super) fn only_shift(key: KeyEvent) -> bool {
    command_modifiers(key) == KeyModifiers::SHIFT
}

pub(super) fn unmodified_or_shift(key: KeyEvent) -> bool {
    let modifiers = command_modifiers(key);
    modifiers.is_empty() || modifiers == KeyModifiers::SHIFT
}

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
    Usage,
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
    Help,
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
    OpenUsage,
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
    Edit,
    CancelRequest,
    NextTab,
    Clear,
    ClearOverride,
    ProviderDelete,
    ProviderRefreshModels,
    ProviderAddModel,
    ProviderDeleteAdapter,
    ProviderSaveAdapter,
    ProviderDeleteModel,
    ProviderSave,
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
        KeyContext::Usage => usage::resolve(key),
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
    use super::{ComposerAction, ComposerKeyBindings, KeyAction, KeyContext, resolve};
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
            (KeyContext::Transcript, KeyCode::Home, KeyAction::Home),
            (KeyContext::Transcript, KeyCode::End, KeyAction::End),
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
            Some(KeyAction::Delete)
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
            Some(KeyAction::NextTab)
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
    fn settings_studio_uses_directional_pane_navigation() {
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
            None
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
                key(KeyCode::Char('d'), KeyModifiers::CONTROL)
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
            None
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
            bindings.match_action(&key(KeyCode::Up, KeyModifiers::CONTROL)),
            Some(ComposerAction::EditQueue)
        );
        assert_eq!(
            bindings.match_action(&key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            None
        );
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
    fn provider_actions_require_explicit_control_shortcuts() {
        for (character, action) in [
            ('k', KeyAction::ProviderDelete),
            ('r', KeyAction::ProviderRefreshModels),
            ('n', KeyAction::ProviderAddModel),
            ('d', KeyAction::ProviderDeleteAdapter),
            ('a', KeyAction::ProviderSaveAdapter),
            ('x', KeyAction::ProviderDeleteModel),
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
    }

    #[test]
    fn secondary_pages_have_no_plain_character_commands() {
        for context in [
            KeyContext::Help,
            KeyContext::Usage,
            KeyContext::SettingsStudio,
            KeyContext::AgentStudio,
            KeyContext::PermissionStudio,
            KeyContext::PermissionRuleStudio,
            KeyContext::ProviderStudio,
            KeyContext::ProviderDetail,
            KeyContext::ProviderModel,
            KeyContext::ModelCatalog,
            KeyContext::PluginPolicy,
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
    fn secondary_pages_reject_redundant_navigation_keys() {
        let contexts = [
            KeyContext::Help,
            KeyContext::Usage,
            KeyContext::SettingsStudio,
            KeyContext::AgentStudio,
            KeyContext::PermissionStudio,
            KeyContext::PermissionRuleStudio,
            KeyContext::PathBrowser,
            KeyContext::ProviderStudio,
            KeyContext::ProviderDetail,
            KeyContext::ProviderModel,
            KeyContext::ModelCatalog,
            KeyContext::PluginPolicy,
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
                KeyCode::BackTab,
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
                KeyCode::BackTab,
                KeyCode::Backspace,
            ] {
                assert_eq!(
                    resolve(context, key(code, KeyModifiers::NONE)),
                    None,
                    "{context:?} still binds redundant key {code:?}"
                );
            }
        }
    }

    #[test]
    fn user_input_cancellation_no_longer_uses_editor_delete() {
        for context in [KeyContext::UserInputQuestion, KeyContext::UserInputReview] {
            assert_eq!(
                resolve(context, key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
                None
            );
            assert_eq!(
                resolve(context, key(KeyCode::Char('x'), KeyModifiers::CONTROL)),
                Some(KeyAction::CancelRequest)
            );
        }
    }

    #[test]
    fn visible_shortcut_hints_track_the_central_keymap() {
        let english = crate::i18n::I18n::english();
        let transcript = crate::ui_text::t(&english, "status-transcript");
        let composer = crate::ui_text::t(&english, "status-composer");
        let global = crate::ui_text::t(&english, "status-global");
        let help_hint = crate::ui_text::t(&english, "context-help-global-hint");
        let provider_footer = crate::ui_text::t(&english, "overlay-provider-studio-footer");

        assert!(transcript.contains("i insert"));
        assert!(composer.contains("Esc view"));
        assert!(composer.contains("Ctrl+Up recover queued"));
        assert!(composer.contains("Up at start history"));
        assert!(!composer.contains("Ctrl+R/Alt+Up history"));
        assert!(help_hint.contains("Ctrl+H"));
        for removed in ["Alt+S", "Alt+P", "q quit"] {
            assert!(!global.contains(removed));
        }
        for shortcut in [
            "Ctrl+R", "Ctrl+N", "Ctrl+A", "Ctrl+D", "Ctrl+X", "Ctrl+S", "Ctrl+K",
        ] {
            assert!(provider_footer.contains(shortcut));
        }
        assert!(!provider_footer.contains("Enter edits or activates"));
    }
}
