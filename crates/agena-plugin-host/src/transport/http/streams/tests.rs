use std::time::Duration;

use super::*;

fn context(call: i64) -> HostCallbackContext {
    HostCallbackContext {
        plugin_id: Some("test.http-streams".into()),
        session_id: Some(11),
        call_id: Some(call),
        workspace_root: Some("/test/workspace".into()),
        tool_name: Some("stream".into()),
        authority_token: Some(format!("test-authority-{call}")),
    }
}

fn chunk(stream_id: &str, text: &str) -> StreamEvent {
    StreamEvent::Chunk(ToolStreamChunk {
        stream_id: stream_id.into(),
        text_delta: Some(text.into()),
        metadata: Default::default(),
    })
}

fn end(stream_id: &str) -> StreamEvent {
    StreamEvent::End(ToolStreamEnd::text(stream_id, "complete"))
}

#[tokio::test]
async fn slow_stream_consumer_fails_instead_of_blocking_event_ingress() {
    let streams = Arc::new(StreamRegistry::default());
    let handle = streams
        .begin(context(1))
        .unwrap()
        .register("stream".into())
        .unwrap();
    for index in 0..=64 {
        streams
            .ingest(context(1), chunk("stream", &format!("chunk-{index}")))
            .unwrap();
    }
    let result = tokio::time::timeout(Duration::from_secs(1), handle.end)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert!(streams.lock().active.is_empty());
}

#[tokio::test]
async fn dropping_stream_receivers_reclaims_registration() {
    let streams = Arc::new(StreamRegistry::default());
    let handle = streams
        .begin(context(1))
        .unwrap()
        .register("stream".into())
        .unwrap();
    drop(handle);
    tokio::time::timeout(Duration::from_secs(1), async {
        while !streams.lock().active.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(streams.ingest(context(1), chunk("stream", "late")).is_err());
    assert!(streams.lock().pending.is_empty());
}

#[tokio::test]
async fn cancelling_pending_registration_discards_early_events() {
    let streams = Arc::new(StreamRegistry::default());
    let pending = streams.begin(context(1)).unwrap();
    streams.ingest(context(1), chunk("stream", "old")).unwrap();
    drop(pending);
    assert!(streams.lock().pending.is_empty());
    assert!(streams.ingest(context(1), end("stream")).is_err());
    let mut handle = streams
        .begin(context(2))
        .unwrap()
        .register("stream".into())
        .unwrap();
    streams.ingest(context(2), chunk("stream", "new")).unwrap();
    streams.ingest(context(2), end("stream")).unwrap();
    assert_eq!(
        handle.chunks.recv().await.unwrap().text_delta.as_deref(),
        Some("new")
    );
    assert!(handle.chunks.recv().await.is_none());
    assert!(handle.end.await.unwrap().is_ok());
}

#[tokio::test]
async fn pending_callbacks_cannot_claim_another_invocations_stream_id() {
    let streams = Arc::new(StreamRegistry::default());
    let first = streams.begin(context(1)).unwrap();
    let second = streams.begin(context(2)).unwrap();
    streams
        .ingest(context(1), chunk("first", "first-early"))
        .unwrap();
    assert!(
        streams
            .ingest(context(2), chunk("first", "injected"))
            .is_err()
    );
    assert!(
        streams
            .ingest(context(1), chunk("different", "injected"))
            .is_err()
    );
    let mut changed = context(1);
    changed.workspace_root = Some("/other/workspace".into());
    assert!(streams.ingest(changed, chunk("first", "altered")).is_err());
    streams
        .ingest(context(2), chunk("second", "second-early"))
        .unwrap();
    let mut first = first.register("first".into()).unwrap();
    let mut second = second.register("second".into()).unwrap();
    streams.ingest(context(1), end("first")).unwrap();
    streams.ingest(context(2), end("second")).unwrap();
    for (handle, expected) in [(&mut first, "first-early"), (&mut second, "second-early")] {
        assert_eq!(
            handle.chunks.recv().await.unwrap().text_delta.as_deref(),
            Some(expected)
        );
        assert!(handle.chunks.recv().await.is_none());
    }
    assert!(first.end.await.unwrap().is_ok());
    assert!(second.end.await.unwrap().is_ok());
}

#[tokio::test]
async fn duplicate_stream_response_preserves_the_original_consumer() {
    let streams = Arc::new(StreamRegistry::default());
    let mut first = streams
        .begin(context(1))
        .unwrap()
        .register("shared".into())
        .unwrap();
    let duplicate = streams.begin(context(2)).unwrap().register("shared".into());
    assert!(matches!(duplicate, Err(TransportError::Rpc(_))));
    assert!(streams.lock().pending.is_empty());
    streams
        .ingest(context(1), chunk("shared", "original"))
        .unwrap();
    streams.ingest(context(1), end("shared")).unwrap();
    assert_eq!(
        first.chunks.recv().await.unwrap().text_delta.as_deref(),
        Some("original")
    );
    assert!(first.end.await.unwrap().is_ok());
}

#[tokio::test]
async fn response_must_match_the_id_of_its_early_callbacks() {
    let streams = Arc::new(StreamRegistry::default());
    let pending = streams.begin(context(1)).unwrap();
    streams.ingest(context(1), chunk("early", "early")).unwrap();
    assert!(pending.register("different".into()).is_err());
    let state = streams.lock();
    assert!(state.pending.is_empty());
    assert!(state.active.is_empty());
}

#[tokio::test]
async fn transport_close_clears_pending_and_fails_active_streams() {
    let streams = Arc::new(StreamRegistry::default());
    let active = streams
        .begin(context(1))
        .unwrap()
        .register("active".into())
        .unwrap();
    let pending = streams.begin(context(2)).unwrap();
    streams
        .ingest(context(2), chunk("pending", "early"))
        .unwrap();
    streams.close(PluginError::internal("test close"));
    assert!(active.end.await.unwrap().is_err());
    assert!(pending.register("pending".into()).is_err());
    assert!(streams.begin(context(3)).is_err());
    assert!(streams.ingest(context(2), end("pending")).is_err());
    let state = streams.lock();
    assert!(state.pending.is_empty());
    assert!(state.active.is_empty());
}

#[tokio::test]
async fn delayed_abandonment_monitor_preserves_a_reused_id() {
    let streams = Arc::new(StreamRegistry::default());
    let first = streams
        .begin(context(1))
        .unwrap()
        .register("reused".into())
        .unwrap();
    streams.ingest(context(1), end("reused")).unwrap();
    drop(first);
    let mut second = streams
        .begin(context(2))
        .unwrap()
        .register("reused".into())
        .unwrap();
    tokio::task::yield_now().await;
    assert!(streams.ingest(context(1), chunk("reused", "old")).is_err());
    streams.ingest(context(2), chunk("reused", "new")).unwrap();
    streams.ingest(context(2), end("reused")).unwrap();
    assert_eq!(
        second.chunks.recv().await.unwrap().text_delta.as_deref(),
        Some("new")
    );
    assert!(second.end.await.unwrap().is_ok());
}

#[tokio::test]
async fn pending_limit_and_duplicate_authority_preserve_existing_calls() {
    let streams = Arc::new(StreamRegistry::default());
    let mut pending = (0..128)
        .map(|call| streams.begin(context(call)).unwrap())
        .collect::<Vec<_>>();
    assert!(streams.begin(context(0)).is_err());
    assert!(streams.begin(context(128)).is_err());
    assert_eq!(streams.lock().pending.len(), 128);
    drop(pending.pop());
    let replacement = streams.begin(context(128)).unwrap();
    let active = replacement.register("replacement".into()).unwrap();
    assert!(streams.begin(context(128)).is_err());
    streams.ingest(context(128), end("replacement")).unwrap();
    assert!(active.end.await.unwrap().is_ok());
    drop(pending);
    assert!(streams.lock().pending.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_keeps_early_chunks_before_concurrent_later_chunks() {
    for call in 0..64 {
        let streams = Arc::new(StreamRegistry::default());
        let pending = streams.begin(context(call)).unwrap();
        streams
            .ingest(context(call), chunk("stream", "early"))
            .unwrap();
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let registration = tokio::spawn({
            let gate = gate.clone();
            async move {
                gate.wait().await;
                pending.register("stream".into()).unwrap()
            }
        });
        gate.wait().await;
        streams
            .ingest(context(call), chunk("stream", "later"))
            .unwrap();
        let mut handle = registration.await.unwrap();
        streams.ingest(context(call), end("stream")).unwrap();
        assert_eq!(
            handle.chunks.recv().await.unwrap().text_delta.as_deref(),
            Some("early")
        );
        assert_eq!(
            handle.chunks.recv().await.unwrap().text_delta.as_deref(),
            Some("later")
        );
        assert!(handle.chunks.recv().await.is_none());
        assert!(handle.end.await.unwrap().is_ok());
    }
}
