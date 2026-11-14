use super::*;
use agena_domain::KernelEpoch;
use agena_storage::backend::kernel::ChangeFeed;
use agena_storage::kernel::{ChangeKind, CommitPosition, CommitSequence, SessionId};

#[test]
fn session_scope_receives_global_changes_without_exposing_them_to_project_lists() {
    let global = Scope::Global;
    let project = Scope::Workspace { workspace_id: 2 };
    let owner = Scope::Session { session_id: 7 };
    let other = Scope::Session { session_id: 8 };
    for listed in [false, true] {
        assert!(includes_session_change(&owner, 7, None, listed));
        assert!(!includes_session_change(&other, 7, None, listed));
        assert!(!includes_session_change(&project, 7, None, listed));
        assert!(!includes_session_change(&global, 7, None, listed));
    }
    assert!(includes_session_change(&project, 7, Some(2), true));
    assert!(!includes_session_change(&project, 7, Some(3), true));
    assert!(!includes_session_change(&project, 7, Some(2), false));
    assert!(!belongs_to_workspace(2, None));
    assert!(!belongs_to_workspace(2, Some(3)));
    assert!(belongs_to_workspace(2, Some(2)));
}

#[tokio::test]
async fn reset_recovery_keeps_commits_while_its_notification_is_backpressured() {
    let feed = ChangeFeed::new(KernelEpoch::new(), 4);
    let session = SessionId::try_from(1).unwrap();
    let mut watch = feed.watch(session, None);
    feed.invalidate();
    assert!(matches!(watch.try_recv(), Err(WatchError::Reset { .. })));

    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(LiveItem::RuntimeSignal(RuntimeSignalResource {
        kind: "earlier-event".into(),
        session_id: Some(session.get()),
        payload: serde_json::Value::Null,
    }))
    .unwrap();
    let position = CommitPosition {
        sequence: CommitSequence::INITIAL,
        ordinal: 0,
    };
    {
        let recovery = recover_watch(&mut watch, || Ok(feed.watch(session, None)), &tx);
        tokio::pin!(recovery);
        std::future::poll_fn(|context| {
            assert!(std::future::Future::poll(recovery.as_mut(), context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;

        // The transport has not drained its preceding event. A new commit
        // during this delivery window must belong to the replacement watch.
        feed.publish(position, session, ChangeKind::Metadata);
        assert!(matches!(rx.recv().await, Some(LiveItem::RuntimeSignal(_))));
        assert!(recovery.await);
    }
    assert!(matches!(rx.recv().await, Some(LiveItem::Lagged(1))));
    let change = watch
        .try_recv()
        .unwrap()
        .expect("the recovery watch must retain a commit made before reset delivery");
    assert_eq!(change.cursor.position, position);
    assert_eq!(change.session, session);
}

#[test]
fn scoped_conversations_keep_runtime_wide_invalidations_without_other_session_content() {
    for (kind, payload) in [
        ("tool_registry_changed", serde_json::Value::Null),
        ("plugin", serde_json::json!({"kind": "attention.changed"})),
    ] {
        let mut signal = RuntimeSignalResource {
            kind: kind.into(),
            session_id: None,
            payload,
        };
        assert!(is_global_invalidation(&signal));
        signal.session_id = Some(99);
        assert!(!is_global_invalidation(&signal));
    }
    assert!(!is_global_invalidation(&RuntimeSignalResource {
        kind: "plugin".into(),
        session_id: None,
        payload: serde_json::json!({"kind": "plan.changed"}),
    }));
}
