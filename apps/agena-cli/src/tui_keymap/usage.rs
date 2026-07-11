use crossterm::event::{KeyCode as K, KeyEvent, KeyModifiers};

use super::KeyAction as A;

pub(super) fn resolve(key: KeyEvent) -> Option<A> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if ctrl || alt {
        return None;
    }

    match key.code {
        K::Esc | K::Char('q') => Some(A::Close),
        K::Left | K::Char('h') => Some(A::PreviousPeriod),
        K::Right | K::Char('l') => Some(A::NextPeriod),
        K::BackTab => Some(A::PreviousTab),
        K::Tab => Some(A::NextTab),
        K::Char(view @ '1'..='5') => Some(A::SelectView(view as u8 - b'0')),
        K::Up | K::Char('k') => Some(A::MoveUp),
        K::Down | K::Char('j') => Some(A::MoveDown),
        K::Home | K::Char('g') => Some(A::Home),
        K::End | K::Char('G') => Some(A::End),
        K::Char('s') => Some(A::CycleSort),
        K::Char('a') => Some(A::ToggleSubagents),
        K::Char('p') => Some(A::NextProvider),
        K::Char('P') => Some(A::PreviousProvider),
        K::Char('m') => Some(A::NextModel),
        K::Char('M') => Some(A::PreviousModel),
        K::Char('r') => Some(A::Refresh),
        K::Enter => Some(A::Open),
        _ => None,
    }
}
