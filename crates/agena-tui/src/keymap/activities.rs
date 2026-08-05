use crossterm::event::{KeyCode as K, KeyEvent};

use super::{KeyAction as A, unmodified};

pub(super) fn resolve(key: KeyEvent) -> Option<A> {
    match key.code {
        K::Esc if unmodified(key) => Some(A::Close),
        K::Up if unmodified(key) => Some(A::MoveUp),
        K::Down if unmodified(key) => Some(A::MoveDown),
        K::Enter if unmodified(key) => Some(A::Open),
        K::Char('r') if unmodified(key) => Some(A::Refresh),
        K::Char('s') if unmodified(key) => Some(A::ActivitiesStop),
        K::Char('d') if unmodified(key) => Some(A::ActivitiesDismiss),
        K::Char('x') if unmodified(key) => Some(A::ActivitiesClearFinished),
        K::Char('f') if unmodified(key) => Some(A::ActivitiesToggleFinished),
        K::Char('k') if unmodified(key) => Some(A::ActivitiesCycleKind),
        K::Char('t') if unmodified(key) => Some(A::ActivitiesCycleStatus),
        _ => None,
    }
}
