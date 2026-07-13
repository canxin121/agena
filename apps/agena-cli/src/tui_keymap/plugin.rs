use crossterm::event::{KeyCode as K, KeyEvent};

use super::{KeyAction as A, KeyContext, unmodified};

pub(super) fn resolve(context: KeyContext, key: KeyEvent) -> Option<A> {
    match context {
        KeyContext::PluginPolicy => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Tab if unmodified(key) => Some(A::Toggle),
            K::Enter if unmodified(key) => Some(A::Activate),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Left if unmodified(key) => Some(A::MoveLeft),
            K::Right if unmodified(key) => Some(A::MoveRight),
            K::Delete if unmodified(key) => Some(A::Delete),
            _ => None,
        },
        KeyContext::PluginList => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Tab if unmodified(key) => Some(A::NextTab),
            K::Enter if unmodified(key) => Some(A::Open),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            _ => None,
        },
        KeyContext::PluginDetail => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Tab if unmodified(key) => Some(A::NextTab),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            _ => None,
        },
        KeyContext::PluginConfig => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Tab if unmodified(key) => Some(A::NextTab),
            K::Delete if unmodified(key) => Some(A::Delete),
            K::Enter if unmodified(key) => Some(A::Edit),
            K::Left if unmodified(key) => Some(A::MoveLeft),
            K::Right if unmodified(key) => Some(A::MoveRight),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            _ => None,
        },
        KeyContext::PluginConfigActions => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Left if unmodified(key) => Some(A::PageUp),
            K::Right if unmodified(key) => Some(A::PageDown),
            K::Enter if unmodified(key) => Some(A::Accept),
            _ => None,
        },
        KeyContext::PluginConfigSelection => match key.code {
            K::Esc if unmodified(key) => Some(A::Close),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Left if unmodified(key) => Some(A::PageUp),
            K::Right if unmodified(key) => Some(A::PageDown),
            K::Char(' ') if unmodified(key) => Some(A::Toggle),
            K::Enter if unmodified(key) => Some(A::Accept),
            _ => None,
        },
        KeyContext::PluginDrilldown => match key.code {
            K::Esc if unmodified(key) => Some(A::Back),
            K::Delete if unmodified(key) => Some(A::Delete),
            K::Up if unmodified(key) => Some(A::MoveUp),
            K::Down if unmodified(key) => Some(A::MoveDown),
            K::Left if unmodified(key) => Some(A::MoveLeft),
            K::Right if unmodified(key) => Some(A::MoveRight),
            K::Enter if unmodified(key) => Some(A::Edit),
            _ => None,
        },
        _ => None,
    }
}
