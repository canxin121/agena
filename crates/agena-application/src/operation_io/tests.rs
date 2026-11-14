use super::*;

#[tokio::test]
async fn polling_stream_retries_replay_events_and_close_stops_the_source() {
    let io = Arc::new(OperationIo::default());
    let (tx, rx) = mpsc::channel(4);
    let handle = io.register_stream(rx);
    tx.send(("change".into(), serde_json::json!({"revision":1})))
        .await
        .unwrap();
    let request = ReadStreamParams {
        stream_id: handle.stream_id,
        after_seq: 0,
        limit: None,
        wait_ms: Some(1000),
    };
    let first = io.read_stream(request.clone()).await.unwrap();
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.next_seq, 1);
    let retry = io.read_stream(request).await.unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(retry).unwrap()
    );
    assert!(
        io.read_stream(ReadStreamParams {
            stream_id: handle.stream_id,
            after_seq: 20,
            limit: None,
            wait_ms: None
        })
        .await
        .is_err()
    );
    io.close_stream(handle.stream_id);
    tokio::time::timeout(Duration::from_secs(1), tx.closed())
        .await
        .unwrap();
    assert!(
        io.read_stream(ReadStreamParams {
            stream_id: handle.stream_id,
            after_seq: 0,
            limit: None,
            wait_ms: None
        })
        .await
        .is_err()
    );
    io.close_stream(handle.stream_id);
}

#[tokio::test]
async fn slow_reader_receives_an_explicit_gap_and_the_remaining_tail() {
    let io = Arc::new(OperationIo::default());
    let (tx, rx) = mpsc::channel(512);
    let handle = io.register_stream(rx);
    for value in 1..=300 {
        tx.send(("change".into(), serde_json::json!(value)))
            .await
            .unwrap();
    }
    drop(tx);
    let entry = io
        .streams
        .lock()
        .unwrap()
        .get(&handle.stream_id)
        .unwrap()
        .clone();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let changed = entry.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if entry.state.lock().unwrap().closed {
                break;
            }
            changed.await;
        }
    })
    .await
    .unwrap();
    let first = io
        .read_stream(ReadStreamParams {
            stream_id: handle.stream_id,
            after_seq: 0,
            limit: Some(128),
            wait_ms: None,
        })
        .await
        .unwrap();
    assert!(first.lagged);
    assert_eq!(first.events.first().unwrap().sequence, 45);
    assert!(!first.closed);
    let tail = io
        .read_stream(ReadStreamParams {
            stream_id: handle.stream_id,
            after_seq: first.next_seq,
            limit: Some(128),
            wait_ms: None,
        })
        .await
        .unwrap();
    assert!(!tail.lagged);
    assert!(tail.closed);
    assert_eq!(tail.next_seq, 300);
}

#[tokio::test]
async fn transfer_offsets_are_seekable_and_retried_uploads_cannot_corrupt_data() {
    let io = Arc::new(OperationIo::default());
    let transfer = io
        .register_transfer(
            tempfile::NamedTempFile::new().unwrap(),
            Some(UploadPurpose::Import {
                request_id: agena_domain::CommandId::new(),
            }),
            None,
        )
        .await
        .unwrap();
    let encode = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    assert_eq!(
        io.write_transfer(transfer.transfer_id, 0, encode(b"abc"))
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        io.write_transfer(transfer.transfer_id, 0, encode(b"abc"))
            .await
            .unwrap(),
        3
    );
    assert!(
        io.write_transfer(transfer.transfer_id, 0, encode(b"xyz"))
            .await
            .is_err()
    );
    assert!(
        io.write_transfer(transfer.transfer_id, 9, encode(b"end"))
            .await
            .is_err()
    );
    assert_eq!(
        io.write_transfer(transfer.transfer_id, 3, encode(b"def"))
            .await
            .unwrap(),
        6
    );
    let temp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(temp.path(), b"abcdef").unwrap();
    let download = io
        .register_transfer(temp, None, Some("sample".into()))
        .await
        .unwrap();
    let request = ReadTransferParams {
        transfer_id: download.transfer_id,
        offset: 2,
        max_bytes: Some(3),
    };
    let chunk = io.read_transfer(request.clone()).await.unwrap();
    assert_eq!(chunk.data_base64, encode(b"cde"));
    assert_eq!(chunk.next_offset, 5);
    assert!(!chunk.eof);
    assert_eq!(
        chunk.data_base64,
        io.read_transfer(request).await.unwrap().data_base64
    );
    let end = io
        .read_transfer(ReadTransferParams {
            transfer_id: download.transfer_id,
            offset: 5,
            max_bytes: None,
        })
        .await
        .unwrap();
    assert_eq!(end.data_base64, encode(b"f"));
    assert!(end.eof);
    io.close_transfer(download.transfer_id);
    assert!(
        io.read_transfer(ReadTransferParams {
            transfer_id: download.transfer_id,
            offset: 0,
            max_bytes: None
        })
        .await
        .is_err()
    );
}
