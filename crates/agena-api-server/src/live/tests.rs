use super::*;
use agena_domain::KernelEpoch;
use agena_storage::backend::kernel::ChangeFeed;
use agena_storage::kernel::{CommitPosition, CommitSequence, SessionId};

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
        assert!(futures_util::poll!(recovery.as_mut()).is_pending());

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
