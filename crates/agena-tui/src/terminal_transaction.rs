//! Typed terminal response transactions owned by the TUI runtime.

use agena_tui_components::TerminalRgb;
use crossterm::event::{BackgroundColorQuery, TerminalResponse};

use crate::terminal_color::{TerminalColorDetection, TerminalColorSource};

#[derive(Debug)]
/// State of a terminal protocol transaction.
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
/// Progress of a terminal protocol transaction.
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
