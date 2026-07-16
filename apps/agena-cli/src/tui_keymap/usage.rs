use crossterm::event::{KeyCode as K, KeyEvent};

use super::{KeyAction as A, only_alt, only_ctrl, unmodified};

pub(super) fn resolve(key: KeyEvent) -> Option<A> {
    match key.code {
        K::Esc if unmodified(key) => Some(A::Close),
        K::Char('p') if only_alt(key) => Some(A::UsageCyclePeriod),
        K::Char('v') if only_alt(key) => Some(A::UsageCycleView),
        K::Char('o') if only_alt(key) => Some(A::UsageCycleProvider),
        K::Char('m') if only_alt(key) => Some(A::UsageCycleModel),
        K::Char('a') if only_alt(key) => Some(A::UsageToggleSubagents),
        K::Char('s') if only_alt(key) => Some(A::UsageCycleSort),
        K::Char('r') if only_ctrl(key) => Some(A::Refresh),
        K::Up if unmodified(key) => Some(A::MoveUp),
        K::Down if unmodified(key) => Some(A::MoveDown),
        K::Enter if unmodified(key) => Some(A::Open),
        _ => None,
    }
}
