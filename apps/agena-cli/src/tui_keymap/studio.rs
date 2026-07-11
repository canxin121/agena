use crossterm::event::{KeyCode as K, KeyEvent};

use super::{KeyAction as A, KeyContext, unmodified, unmodified_or_shift};

pub(super) fn resolve(context: KeyContext, key: KeyEvent) -> Option<A> {
    match context {
        KeyContext::SettingsStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Tab if unmodified(key) => Some(A::NextTab),
            K::BackTab if unmodified_or_shift(key) => Some(A::PreviousTab),
            K::PageUp if unmodified(key) => Some(A::PageUp),
            K::PageDown if unmodified(key) => Some(A::PageDown),
            K::Home if unmodified(key) => Some(A::Home),
            K::End if unmodified(key) => Some(A::End),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::AgentStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::PermissionStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Tab if unmodified(key) => Some(A::NextTab),
            K::BackTab if unmodified_or_shift(key) => Some(A::PreviousTab),
            K::PageUp if unmodified(key) => Some(A::PageUp),
            K::PageDown if unmodified(key) => Some(A::PageDown),
            K::Home if unmodified(key) => Some(A::Home),
            K::End if unmodified(key) => Some(A::End),
            K::Left if unmodified(key) => Some(A::MoveLeft),
            K::Right if unmodified(key) => Some(A::MoveRight),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::PermissionRuleStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::PathBrowser => match key.code {
            K::Enter if unmodified(key) => Some(A::Accept),
            _ => None,
        },
        KeyContext::ProviderStudio => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Tab if unmodified(key) => Some(A::NextTab),
            K::BackTab if unmodified_or_shift(key) => Some(A::PreviousTab),
            K::Char(' ') if unmodified(key) => Some(A::Toggle),
            K::PageUp if unmodified(key) => Some(A::PageUp),
            K::PageDown if unmodified(key) => Some(A::PageDown),
            K::Home if unmodified(key) => Some(A::Home),
            K::End if unmodified(key) => Some(A::End),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::ProviderDetail => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::ProviderModel => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        KeyContext::ModelCatalog => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Tab if unmodified(key) => Some(A::NextTab),
            K::BackTab if unmodified_or_shift(key) => Some(A::PreviousTab),
            K::Enter if unmodified(key) => Some(A::Activate),
            _ => None,
        },
        _ => None,
    }
}
