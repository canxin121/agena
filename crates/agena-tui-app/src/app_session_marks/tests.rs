use super::*;

#[test]
fn snapshots_older_than_a_commit_cannot_revert_its_marker() {
    let mut writes = SessionMarkWrites::default();
    writes.committed.insert((1, SessionMark::Pin), (10, true));
    writes
        .committed
        .insert((1, SessionMark::Favorite), (12, false));
    assert!(writes.visible_value(1, SessionMark::Pin, 9, false));
    assert!(!writes.visible_value(1, SessionMark::Favorite, 11, true));
    // A newer change from another client remains authoritative.
    assert!(!writes.visible_value(1, SessionMark::Pin, 11, false));
    assert!(writes.visible_value(1, SessionMark::Favorite, 13, true));
    assert!(!writes.visible_value(2, SessionMark::Pin, 1, false));
    // Pending input wins even over newer snapshots of the in-flight write.
    let (_, write) = writes.toggle(1, SessionMark::Pin, true);
    assert!(!writes.visible_value(1, SessionMark::Pin, 20, true));
    writes.toggle(1, SessionMark::Pin, false);
    assert!(writes.visible_value(1, SessionMark::Pin, 20, false));
    writes.complete(write.unwrap(), Some(false));
    assert!(writes.visible_value(1, SessionMark::Pin, 20, false));
}

#[test]
fn fast_clicks_toggle_latest_intent_and_coalesce_to_one_followup_write() {
    for count in 1..100 {
        let mut writes = SessionMarkWrites::default();
        let (_, first) = writes.toggle(1, SessionMark::Pin, false);
        let first = first.unwrap();
        for _ in 1..count {
            // Even a stale rendered value must not discard the next click.
            assert!(writes.toggle(1, SessionMark::Pin, false).1.is_none());
        }
        let (visible, next) = writes.complete(first, Some(true)).unwrap();
        assert_eq!(visible, count % 2 == 1);
        if let Some(next) = next {
            assert!(!next.value);
            assert_eq!(writes.complete(next, Some(false)), Some((false, None)));
        }
        assert!(writes.is_idle());
    }
}

#[test]
fn intermediate_acknowledgements_never_replace_a_newer_intent() {
    let mut writes = SessionMarkWrites::default();
    let (_, first) = writes.toggle(1, SessionMark::Favorite, false);
    let first = first.unwrap();
    assert_eq!(writes.toggle(1, SessionMark::Favorite, true), (false, None));
    let (visible, second) = writes.complete(first, Some(true)).unwrap();
    assert!(!visible);
    let second = second.unwrap();
    assert_eq!(writes.toggle(1, SessionMark::Favorite, false), (true, None));
    assert_eq!(writes.complete(first, Some(true)), None);
    let (visible, third) = writes.complete(second, Some(false)).unwrap();
    assert!(visible);
    assert_eq!(
        writes.complete(third.unwrap(), Some(true)),
        Some((true, None))
    );
    assert!(writes.is_idle());
}

#[test]
fn pins_favorites_and_different_sessions_have_independent_queues() {
    let mut writes = SessionMarkWrites::default();
    let (_, pin) = writes.toggle(1, SessionMark::Pin, false);
    let (_, favorite) = writes.toggle(1, SessionMark::Favorite, false);
    let (_, other) = writes.toggle(2, SessionMark::Pin, true);
    assert_eq!(
        writes.complete(favorite.unwrap(), Some(true)),
        Some((true, None))
    );
    assert!(!writes.is_idle());
    assert_eq!(
        writes.complete(other.unwrap(), Some(false)),
        Some((false, None))
    );
    assert_eq!(
        writes.complete(pin.unwrap(), Some(true)),
        Some((true, None))
    );
    assert!(writes.is_idle());
}

#[test]
fn failed_writes_roll_back_only_their_marker_and_allow_retry() {
    let mut writes = SessionMarkWrites::default();
    let (_, pin) = writes.toggle(1, SessionMark::Pin, false);
    let (_, favorite) = writes.toggle(1, SessionMark::Favorite, false);
    assert_eq!(writes.complete(pin.unwrap(), None), Some((false, None)));
    assert!(!writes.is_idle());
    assert_eq!(
        writes.complete(favorite.unwrap(), Some(true)),
        Some((true, None))
    );
    let (_, retry) = writes.toggle(1, SessionMark::Pin, false);
    assert_eq!(
        writes.complete(retry.unwrap(), Some(true)),
        Some((true, None))
    );
    assert!(writes.is_idle());
}

#[test]
fn a_failed_compensating_write_reconciles_with_the_last_confirmed_value() {
    let mut writes = SessionMarkWrites::default();
    let (_, first) = writes.toggle(1, SessionMark::Pin, false);
    writes.toggle(1, SessionMark::Pin, true);
    let (_, second) = writes.complete(first.unwrap(), Some(true)).unwrap();
    assert_eq!(writes.complete(second.unwrap(), None), Some((true, None)));
    assert!(writes.is_idle());
}
