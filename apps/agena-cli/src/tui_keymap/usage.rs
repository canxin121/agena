use crossterm::event::{KeyCode as K, KeyEvent};

use super::{KeyAction as A, unmodified};

pub(super) fn resolve(key: KeyEvent) -> Option<A> {
    match key.code {
        K::Esc if unmodified(key) => Some(A::Close),
        K::Tab if unmodified(key) => Some(A::NextTab),
        K::Up if unmodified(key) => Some(A::MoveUp),
        K::Down if unmodified(key) => Some(A::MoveDown),
        K::Enter if unmodified(key) => Some(A::Open),
        _ => None,
    }
}
