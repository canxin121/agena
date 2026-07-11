use crossterm::event::{KeyCode as K, KeyEvent};

use super::{KeyAction as A, unmodified, unmodified_or_shift};

pub(super) fn resolve(key: KeyEvent) -> Option<A> {
    match key.code {
        K::Esc | K::Char('q') if unmodified(key) => Some(A::Close),
        K::Left | K::Char('h') if unmodified(key) => Some(A::PreviousPeriod),
        K::Right | K::Char('l') if unmodified(key) => Some(A::NextPeriod),
        K::BackTab if unmodified_or_shift(key) => Some(A::PreviousTab),
        K::Tab if unmodified(key) => Some(A::NextTab),
        K::Char(view @ '1'..='5') if unmodified(key) => Some(A::SelectView(view as u8 - b'0')),
        K::Up | K::Char('k') if unmodified(key) => Some(A::MoveUp),
        K::Down | K::Char('j') if unmodified(key) => Some(A::MoveDown),
        K::Home | K::Char('g') if unmodified(key) => Some(A::Home),
        K::End if unmodified(key) => Some(A::End),
        K::Char('G') if unmodified_or_shift(key) => Some(A::End),
        K::Char('s') if unmodified(key) => Some(A::CycleSort),
        K::Char('a') if unmodified(key) => Some(A::ToggleSubagents),
        K::Char('p') if unmodified(key) => Some(A::NextProvider),
        K::Char('P') if unmodified_or_shift(key) => Some(A::PreviousProvider),
        K::Char('m') if unmodified(key) => Some(A::NextModel),
        K::Char('M') if unmodified_or_shift(key) => Some(A::PreviousModel),
        K::Char('r') if unmodified(key) => Some(A::Refresh),
        K::Enter if unmodified(key) => Some(A::Open),
        _ => None,
    }
}
