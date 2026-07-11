use crossterm::event::{KeyCode as K, KeyEvent};

use super::{KeyAction as A, unmodified, unmodified_or_shift};

pub(super) fn resolve(key: KeyEvent) -> Option<A> {
    match key.code {
        K::Esc if unmodified(key) => Some(A::Close),
        K::BackTab if unmodified_or_shift(key) => Some(A::PreviousTab),
        K::Tab if unmodified(key) => Some(A::NextTab),
        K::Up if unmodified(key) => Some(A::MoveUp),
        K::Down if unmodified(key) => Some(A::MoveDown),
        K::Home if unmodified(key) => Some(A::Home),
        K::End if unmodified(key) => Some(A::End),
        K::Enter if unmodified(key) => Some(A::Open),
        _ => None,
    }
}
