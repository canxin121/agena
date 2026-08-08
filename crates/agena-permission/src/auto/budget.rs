//! Per-session classifier denial budget. After repeated classifier denials
//! the pipeline stops burning model calls and falls back to interactive ask.

use std::collections::VecDeque;

/// After this many consecutive classifier denials the runtime falls back to
/// interactive confirmation.
pub const AUTO_APPROVAL_CONSECUTIVE_DENIAL_LIMIT: usize = 3;
/// After this many total classifier denials the runtime falls back to
/// interactive confirmation.
pub const AUTO_APPROVAL_TOTAL_DENIAL_LIMIT: usize = 20;
/// Maximum number of recent classifier decisions fed into the next prompt.
pub const AUTO_APPROVAL_MAX_RECENT_DECISIONS: usize = 8;

#[derive(Debug, Clone, Default)]
/// Budget tracking consecutive permission denials.
pub struct DenialBudget {
    consecutive_denials: usize,
    total_denials: usize,
    recent_decisions: VecDeque<&'static str>,
}

impl DenialBudget {
    pub fn record_decision(&mut self, allowed: bool) {
        if allowed {
            self.consecutive_denials = 0;
            self.recent_decisions.push_back("ALLOW");
        } else {
            self.consecutive_denials += 1;
            self.total_denials += 1;
            self.recent_decisions.push_back("DENY");
        }
        while self.recent_decisions.len() > AUTO_APPROVAL_MAX_RECENT_DECISIONS {
            self.recent_decisions.pop_front();
        }
    }

    pub fn exceeded(&self) -> bool {
        self.consecutive_denials >= AUTO_APPROVAL_CONSECUTIVE_DENIAL_LIMIT
            || self.total_denials >= AUTO_APPROVAL_TOTAL_DENIAL_LIMIT
    }

    pub fn recent_decision_labels(&self) -> Vec<&'static str> {
        self.recent_decisions.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_after_repeated_denials_and_resets_on_allow() {
        let mut budget = DenialBudget::default();
        assert!(!budget.exceeded());
        budget.record_decision(false);
        budget.record_decision(false);
        assert!(!budget.exceeded());
        budget.record_decision(false);
        assert!(budget.exceeded());
        budget.record_decision(true);
        assert!(!budget.exceeded(), "an allow resets the consecutive streak");
    }

    #[test]
    fn has_a_total_ceiling() {
        let mut budget = DenialBudget::default();
        for _ in 0..19 {
            budget.record_decision(false);
            budget.record_decision(true);
        }
        assert!(!budget.exceeded());
        budget.record_decision(false);
        assert!(budget.exceeded());
    }

    #[test]
    fn recent_decisions_are_bounded() {
        let mut budget = DenialBudget::default();
        for index in 0..12 {
            budget.record_decision(index % 2 == 0);
        }
        assert_eq!(
            budget.recent_decision_labels().len(),
            AUTO_APPROVAL_MAX_RECENT_DECISIONS
        );
    }
}
