use crossterm::event::{KeyCode as K, KeyEvent};

use super::{
    KeyAction as A, KeyContext, horizontal_pagination_action, only_ctrl, tab_navigation_action,
    unmodified,
};

pub(super) fn resolve(context: KeyContext, key: KeyEvent) -> Option<A> {
    match context {
        KeyContext::SettingsStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Char('r') if only_ctrl(key) => Some(A::Refresh),
            K::Left if unmodified(key) => Some(A::MoveLeft),
            K::Right if unmodified(key) => Some(A::MoveRight),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => tab_navigation_action(key),
        },
        KeyContext::PermissionStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Char('n') if only_ctrl(key) => Some(A::PermissionAdd),
            K::Char('e') if only_ctrl(key) => Some(A::PermissionRename),
            K::Char('d') if only_ctrl(key) => Some(A::Delete),
            K::Left if unmodified(key) => Some(A::MoveLeft),
            K::Right if unmodified(key) => Some(A::MoveRight),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => tab_navigation_action(key),
        },
        KeyContext::PermissionRuleStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Char('o') if only_ctrl(key) => Some(A::PermissionBrowse),
            K::Char('s') if only_ctrl(key) => Some(A::PermissionSave),
            K::Char('d') if only_ctrl(key) => Some(A::Delete),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::PathBrowser => match key.code {
            K::Enter if unmodified(key) => Some(A::Accept),
            _ => None,
        },
        KeyContext::ProviderStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Char('d') if only_ctrl(key) => Some(A::Delete),
            K::Char('r') if only_ctrl(key) => Some(A::ProviderRefreshModels),
            K::Char('n') if only_ctrl(key) => Some(A::ProviderAddModel),
            K::Char('a') if only_ctrl(key) => Some(A::ProviderSaveAdapter),
            K::Char('s') if only_ctrl(key) => Some(A::ProviderSave),
            K::Char(' ') if unmodified(key) => Some(A::Toggle),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => tab_navigation_action(key),
        },
        KeyContext::ProviderDetail => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::ProviderModel => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Char('d') if only_ctrl(key) => Some(A::Delete),
            K::Char('s') if only_ctrl(key) => Some(A::ProviderSave),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::ModelCatalog => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Char('f') if only_ctrl(key) => Some(A::ModelCatalogSearch),
            K::Char('r') if only_ctrl(key) => Some(A::Refresh),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            _ => horizontal_pagination_action(key),
        },
        _ => None,
    }
}
