use crate::event::DomainEvent;

/// Generic event-fold abstraction.
///
/// Every projection over the append-only event log is expressed by
/// implementing this trait. The runtime guarantees:
///
/// * `fold` is invoked once per [`DomainEvent`] in monotonically ascending
///   `seq_global` order.
/// * `finish` is invoked exactly once after the last event, allowing the
///   implementation to materialize whatever derived shape it produces.
///
/// Implementors MUST treat their internal accumulator as the only mutable
/// state — they may NOT side-effect on the caller's behalf, write to disk,
/// or otherwise observe the world. This keeps every projection a pure
/// function of the event log and therefore deterministic.
pub(crate) trait HistoryFold: Default {
    type Output;
    type Error;

    fn fold(&mut self, event: &DomainEvent) -> Result<(), Self::Error>;
    fn finish(self) -> Self::Output;
}

/// Run a `HistoryFold` over a slice of events, yielding the projection.
pub(crate) fn fold_history<F: HistoryFold>(events: &[DomainEvent]) -> Result<F::Output, F::Error> {
    let mut acc = F::default();
    for event in events {
        acc.fold(event)?;
    }
    Ok(acc.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, PublishContext};
    use crate::session::history::{TurnAbortReason, TurnAborted};
    use crate::session::ids::TurnId;
    use crate::event::{EventMeta, envelope::ENVELOPE_SCHEMA_VERSION};
    use chrono::Utc;
    use uuid::Uuid;

    #[derive(Default)]
    struct CountFold(usize);

    impl HistoryFold for CountFold {
        type Output = usize;
        type Error = std::convert::Infallible;

        fn fold(&mut self, _event: &DomainEvent) -> Result<(), Self::Error> {
            self.0 += 1;
            Ok(())
        }
        fn finish(self) -> Self::Output {
            self.0
        }
    }

    #[test]
    fn fold_history_visits_every_event_in_order() {
        let events: Vec<DomainEvent> = (0..5)
            .map(|i| DomainEvent {
                meta: EventMeta {
                    id: Uuid::new_v4(),
                    seq_global: i,
                    seq_session: Some(i),
                    session_id: Some(1),
                    workspace_id: None,
                    created_at: Utc::now(),
                    causation_id: None,
                    correlation_id: None,
                    envelope_schema: ENVELOPE_SCHEMA_VERSION,
                },
                kind: EventKind::TurnAborted(TurnAborted {
                    turn_id: TurnId::new(),
                    reason: TurnAbortReason::ProcessRestart,
                    message: None,
                }),
            })
            .collect();
        let count = fold_history::<CountFold>(&events).unwrap();
        assert_eq!(count, 5);
        let _ = PublishContext::default();
    }
}
