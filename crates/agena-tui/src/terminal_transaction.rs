//! Typed terminal response transactions owned by the TUI runtime.

use agena_tui_components::TerminalRgb;
use crossterm::event::{BackgroundColorQuery, TerminalResponse};

use crate::terminal_color::{TerminalColorDetection, TerminalColorSource};

#[derive(Debug)]
pub enum ProtocolTransactionState {
    /// Distinct CPR barrier sent after every synchronous startup probe. Its
    /// arrival proves that earlier query replies can no longer be in flight.
    StartupBarrier,
    /// A live background query followed by DSR. The candidate is committed
    /// only when the ordered DSR barrier arrives.
    ColorRefresh {
        query: BackgroundColorQuery,
        source: TerminalColorSource,
        candidate: Option<TerminalRgb>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolProgress {
    Pending,
    Complete(Option<TerminalColorDetection>),
}

impl ProtocolTransactionState {
    pub fn observe(&mut self, response: TerminalResponse) -> ProtocolProgress {
        match self {
            Self::StartupBarrier => {
                if matches!(response, TerminalResponse::CursorPosition { .. }) {
                    ProtocolProgress::Complete(None)
                } else {
                    ProtocolProgress::Pending
                }
            }
            Self::ColorRefresh {
                query,
                source,
                candidate,
            } => match response {
                TerminalResponse::BackgroundColor {
                    query: response_query,
                    red,
                    green,
                    blue,
                } if response_query == *query => {
                    *candidate = Some(TerminalRgb::new(red, green, blue));
                    ProtocolProgress::Pending
                }
                // A success or failure DSR is still the ordered completion
                // marker. Commit a color only when one was received before it.
                TerminalResponse::DeviceStatus(_) => {
                    ProtocolProgress::Complete(candidate.map(|background| TerminalColorDetection {
                        background: Some(background),
                        source: *source,
                    }))
                }
                _ => ProtocolProgress::Pending,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_transaction_completes_only_at_its_distinct_cpr_barrier() {
        let mut transaction = ProtocolTransactionState::StartupBarrier;
        assert_eq!(
            transaction.observe(TerminalResponse::DeviceStatus(0)),
            ProtocolProgress::Pending
        );
        assert_eq!(
            transaction.observe(TerminalResponse::BackgroundColor {
                query: BackgroundColorQuery::Iterm2Osc4,
                red: 250,
                green: 250,
                blue: 250,
            }),
            ProtocolProgress::Pending
        );
        assert_eq!(
            transaction.observe(TerminalResponse::CursorPosition { column: 0, row: 0 }),
            ProtocolProgress::Complete(None)
        );
    }

    #[test]
    fn color_transaction_correlates_selector_and_commits_at_dsr_barrier() {
        let mut transaction = ProtocolTransactionState::ColorRefresh {
            query: BackgroundColorQuery::Iterm2Osc4,
            source: TerminalColorSource::Iterm2Osc4,
            candidate: None,
        };
        assert_eq!(
            transaction.observe(TerminalResponse::BackgroundColor {
                query: BackgroundColorQuery::Osc11,
                red: 1,
                green: 2,
                blue: 3,
            }),
            ProtocolProgress::Pending
        );
        assert_eq!(
            transaction.observe(TerminalResponse::BackgroundColor {
                query: BackgroundColorQuery::Iterm2Osc4,
                red: 250,
                green: 240,
                blue: 230,
            }),
            ProtocolProgress::Pending
        );
        assert_eq!(
            transaction.observe(TerminalResponse::DeviceStatus(3)),
            ProtocolProgress::Complete(Some(TerminalColorDetection {
                background: Some(TerminalRgb::new(250, 240, 230)),
                source: TerminalColorSource::Iterm2Osc4,
            }))
        );
    }

    #[test]
    fn color_transaction_without_a_matching_reply_preserves_current_color() {
        let mut transaction = ProtocolTransactionState::ColorRefresh {
            query: BackgroundColorQuery::Osc11,
            source: TerminalColorSource::Osc11,
            candidate: None,
        };
        assert_eq!(
            transaction.observe(TerminalResponse::DeviceStatus(0)),
            ProtocolProgress::Complete(None)
        );
    }
}
