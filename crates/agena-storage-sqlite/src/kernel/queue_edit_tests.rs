use super::{snapshot_tests::fixture, tests::document, *};

#[tokio::test]
async fn queued_edits_preserve_identity_order_and_blobs_and_reject_stale_or_started_inputs() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "queue edits".into())
            .await
            .unwrap()
            .value
            .id;
        let mut inputs = Vec::new();
        for text in ["first", "middle", "last"] {
            inputs.push(
                ports
                    .sessions
                    .send(RequestId::new(), session, document(text))
                    .await
                    .unwrap()
                    .value,
            );
        }
        let initial = ports
            .reader
            .entry(session, inputs[1].message_id)
            .await
            .unwrap();
        let blob = ports
            .sessions
            .prepare_blob(RequestId::new(), session, 5, &mut &b"image"[..])
            .await
            .unwrap();
        let request = RequestId::new();
        let edited = ports
            .sessions
            .edit_queued_with_blobs(
                request,
                session,
                inputs[1].id,
                initial.revision,
                document("edited middle"),
                std::slice::from_ref(&blob),
            )
            .await
            .unwrap();
        assert_eq!(edited.value, inputs[1]);
        let replay = ports
            .sessions
            .edit_queued_with_blobs(
                request,
                session,
                inputs[1].id,
                initial.revision,
                document("edited middle"),
                std::slice::from_ref(&blob),
            )
            .await
            .unwrap();
        assert_eq!(replay.position, edited.position);
        let changed = ports
            .reader
            .entry(session, inputs[1].message_id)
            .await
            .unwrap();
        assert_eq!(changed.revision.get(), initial.revision.get() + 1);
        assert!(
            matches!(&changed.payload, EntryPayload::User { document } if document.text()=="edited middle")
        );
        let blobs = ports
            .reader
            .entry_blobs(session, inputs[1].message_id)
            .await
            .unwrap();
        assert_eq!(blobs.as_slice(), std::slice::from_ref(blob.descriptor()));
        assert!(matches!(
            ports
                .sessions
                .edit_queued_with_blobs(
                    RequestId::new(),
                    session,
                    inputs[1].id,
                    initial.revision,
                    document("stale"),
                    &[]
                )
                .await,
            Err(KernelError::Conflict(_))
        ));
        assert_eq!(
            ports
                .reader
                .entry_blobs(session, inputs[1].message_id)
                .await
                .unwrap()
                .len(),
            1,
            "failed edit must not drop attachments"
        );
        let retained = ports
            .reader
            .open_blob(session, blob.descriptor().id)
            .await
            .unwrap()
            .into_prepared();
        ports
            .sessions
            .edit_queued_with_blobs(
                RequestId::new(),
                session,
                inputs[1].id,
                changed.revision,
                document("retained attachment"),
                &[retained],
            )
            .await
            .unwrap();
        let changed = ports
            .reader
            .entry(session, inputs[1].message_id)
            .await
            .unwrap();
        ports
            .sessions
            .edit_queued_with_blobs(
                RequestId::new(),
                session,
                inputs[1].id,
                changed.revision,
                document("text only"),
                &[],
            )
            .await
            .unwrap();
        assert!(
            ports
                .reader
                .entry_blobs(session, inputs[1].message_id)
                .await
                .unwrap()
                .is_empty()
        );
        let snapshot = ports.reader.control_snapshot(session, 128).await.unwrap();
        let mut ordered = snapshot.active_executions.executions.clone();
        ordered.sort_by_key(|execution| execution.message_id);
        assert_eq!(
            ordered.iter().map(|e| e.id).collect::<Vec<_>>(),
            inputs.iter().map(|e| e.id).collect::<Vec<_>>()
        );
        for (index, input) in inputs.iter().enumerate() {
            let attempt = ports
                .executor
                .claim_next(RequestId::new(), session, input.id)
                .await
                .unwrap()
                .value;
            let before = ports.reader.entry(session, input.message_id).await.unwrap();
            assert!(matches!(
                ports
                    .sessions
                    .edit_queued_with_blobs(
                        RequestId::new(),
                        session,
                        input.id,
                        before.revision,
                        document("too late"),
                        &[]
                    )
                    .await,
                Err(KernelError::Conflict(_))
            ));
            attempt
                .finish(RequestId::new(), ExecutionPhase::Failed)
                .await
                .unwrap();
            if index == 1 {
                ports
                    .sessions
                    .retry(RequestId::new(), session, input.id)
                    .await
                    .unwrap();
                assert!(matches!(
                    ports
                        .sessions
                        .edit_queued_with_blobs(
                            RequestId::new(),
                            session,
                            input.id,
                            before.revision,
                            document("rewrite history"),
                            &[]
                        )
                        .await,
                    Err(KernelError::Conflict(_))
                ));
                ports
                    .sessions
                    .cancel_queued(RequestId::new(), session, input.id)
                    .await
                    .unwrap();
            }
        }
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn editing_a_shared_queued_input_cannot_rewrite_another_conversation() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "original".into())
            .await
            .unwrap()
            .value
            .id;
        let input = ports
            .sessions
            .send(RequestId::new(), session, document("shared queued input"))
            .await
            .unwrap()
            .value;
        let branch = ports
            .sessions
            .fork(RequestId::new(), session, input.message_id)
            .await
            .unwrap()
            .value
            .id;
        let entry = ports.reader.entry(session, input.message_id).await.unwrap();
        assert!(matches!(
            ports
                .sessions
                .edit_queued_with_blobs(
                    RequestId::new(),
                    session,
                    input.id,
                    entry.revision,
                    document("rewritten"),
                    &[]
                )
                .await,
            Err(KernelError::Conflict(_))
        ));
        let shared = ports.reader.entry(branch, input.message_id).await.unwrap();
        assert!(
            matches!(&shared.payload, EntryPayload::User { document } if document.text()=="shared queued input")
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn accessible_inherited_blobs_can_be_retained_without_granting_other_sessions_access() {
    for memory in [true, false] {
        let (ports, _maintenance, _admin, _diagnostics, _root) = fixture(memory).await;
        let parent = ports
            .sessions
            .create(RequestId::new(), 1, "parent".into())
            .await
            .unwrap()
            .value
            .id;
        let blob = ports
            .sessions
            .prepare_blob(RequestId::new(), parent, 4, &mut &b"data"[..])
            .await
            .unwrap();
        let sent = ports
            .sessions
            .send_with_blobs(
                RequestId::new(),
                parent,
                document("shared attachment"),
                std::slice::from_ref(&blob),
            )
            .await
            .unwrap()
            .value;
        let branch = ports
            .sessions
            .fork(RequestId::new(), parent, sent.message_id)
            .await
            .unwrap()
            .value
            .id;
        let other = ports
            .sessions
            .create(RequestId::new(), 1, "unrelated".into())
            .await
            .unwrap()
            .value
            .id;
        assert!(matches!(
            ports.reader.open_blob(other, blob.descriptor().id).await,
            Err(KernelError::Ownership)
        ));
        let inherited = ports
            .reader
            .open_blob(branch, blob.descriptor().id)
            .await
            .unwrap()
            .into_prepared();
        let queued = ports
            .sessions
            .send_with_blobs(
                RequestId::new(),
                branch,
                document("branch input"),
                std::slice::from_ref(&inherited),
            )
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .edit_queued_with_blobs(
                RequestId::new(),
                branch,
                queued.id,
                EntryRevision::INITIAL,
                document("edited branch input"),
                &[inherited],
            )
            .await
            .unwrap();
        assert_eq!(
            ports
                .reader
                .entry_blobs(branch, queued.message_id)
                .await
                .unwrap()[0]
                .id,
            blob.descriptor().id
        );
        ports.shutdown().await.unwrap();
    }
}
