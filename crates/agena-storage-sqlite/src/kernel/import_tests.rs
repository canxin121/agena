use super::{snapshot_tests::fixture, tests::document, *};
use agena_domain::{CommandOutputStream, ContentKind};
use serde_json::{Value, json};

async fn write_archive(ports: &KernelPorts, session: SessionId, path: &std::path::Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    ports
        .reader
        .export_jsonl(session, &mut bytes)
        .await
        .unwrap();
    tokio::fs::write(path, &bytes).await.unwrap();
    bytes
}
fn rebuild(records: &[Value]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut count = 0u64;
    let mut entries = 0u64;
    for record in records.iter().filter(|r| r["kind"] != "footer") {
        serde_json::to_writer(&mut bytes, record).unwrap();
        bytes.push(b'\n');
        count += 1;
        if record["kind"] == "entry" {
            entries += 1;
        }
    }
    let report = ExportReport {
        entries,
        records: count,
        bytes: bytes.len() as u64,
        sha256: Sha256::digest(&bytes).into(),
    };
    serde_json::to_writer(&mut bytes, &json!({"kind":"footer","data":report})).unwrap();
    bytes.push(b'\n');
    bytes
}

#[tokio::test]
async fn import_remaps_content_blobs_and_preserves_opaque_state_with_idempotent_atomic_receipt() {
    for memory in [true, false] {
        let (ports, _maintenance, admin, _diagnostics, root) = fixture(memory).await;
        let (session, round, tool) = event_tests::tool(&ports).await;
        let output = tool.begin_log(RequestId::new()).await.unwrap().value;
        output
            .append_at(123, CommandOutputStream::Stdout, "out 界\r\n\0")
            .await
            .unwrap();
        output
            .append_at(124, CommandOutputStream::Stderr, "err")
            .await
            .unwrap();
        output.record_loss(7, "capture overflow").unwrap();
        output.finish(RequestId::new()).await.unwrap();
        let terminal = tool.begin_terminal(RequestId::new()).await.unwrap().value;
        let base = terminal
            .snapshot(event_tests::screen("before"))
            .await
            .unwrap();
        let mut next = event_tests::screen("after 界");
        next.cells.truncate(1);
        terminal.patch(base, next, vec![0]).await.unwrap();
        terminal.finish(RequestId::new()).await.unwrap();
        let mixed = tool.begin_mixed(RequestId::new()).await.unwrap().value;
        mixed
            .append(agena_domain::ContentInput::Text {
                text: "first".into(),
            })
            .await
            .unwrap();
        mixed
            .append(agena_domain::ContentInput::Log {
                stream: CommandOutputStream::Stderr,
                text: "second".into(),
            })
            .await
            .unwrap();
        mixed.finish(RequestId::new()).await.unwrap();
        let permission = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "import-permission".into(),
                    description: "preserve the complete decision".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let permission_reply = agena_domain::PermissionReply {
            request_id: permission.get().to_string(),
            kind: agena_domain::PermissionReplyKind::DenyAlways,
            reason: Some("preserved reason 中文 🧪".into()),
            scope: Some(agena_domain::PermissionScope::Session),
        };
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                permission,
                InteractionDecision::Permission {
                    reply: permission_reply.clone(),
                    allowed: false,
                },
                Some(RememberPermission {
                    mode: agena_domain::PermissionMode::Deny,
                    scope: agena_domain::PermissionScope::Session,
                }),
            )
            .await
            .unwrap();
        let original_permission = ports.reader.interaction(session, permission).await.unwrap();
        let permission_decided_at = original_permission.decided_at_ms;
        assert!(permission_decided_at.is_some());
        drop(original_permission);
        let input = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Input {
                    request: agena_domain::UserInputRequest {
                        request_id: "external-input-correlation".into(),
                        session_id: Some(session.get()),
                        title: "preserve input metadata".into(),
                        body_markdown: String::new(),
                        kind: agena_domain::UserInputKind::AskUser,
                        source: agena_domain::UserInputSource::Host,
                        auto_resolution_ms: None,
                        presented_at: None,
                        questions: Vec::new(),
                        created_at: chrono::Utc::now(),
                    },
                },
            )
            .await
            .unwrap()
            .value;
        let input_reply = agena_domain::UserInputReply {
            request_id: input.get().to_string(),
            kind: agena_domain::UserInputReplyKind::Submit,
            answers: Default::default(),
            reason: Some("typed input reply reason".into()),
        };
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                input,
                InteractionDecision::Input(encode(&input_reply).unwrap()),
                None,
            )
            .await
            .unwrap();
        let binary = (0..300_019).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        let prepared = ports
            .sessions
            .prepare_blob(
                RequestId::new(),
                session,
                binary.len() as u64,
                &mut binary.as_slice(),
            )
            .await
            .unwrap();
        let old_blob = prepared.descriptor().id;
        tool.complete_with_blobs(RequestId::new(),ToolResult{phase:ToolPhase::Succeeded,value:json!({"blob_id":old_blob,"source":{"resource_id":output.id(),"kind":"log"}})},std::slice::from_ref(&prepared)).await.unwrap();
        round
            .finish(
                RequestId::new(),
                Some(json!({"opaque":[null,1,{"unknown":" 界 "}]})),
                true,
            )
            .await
            .unwrap();
        let original = ports.reader.history(session, None, 200).await.unwrap();
        let original_ids = original.entries.iter().map(|e| e.id).collect::<Vec<_>>();
        drop(original);
        let state: String =
            sqlx::query_scalar("SELECT provider_state_json FROM agena_rounds WHERE session_id=?")
                .bind(session.get())
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        let path = root.path().join("session.jsonl");
        let archive = write_archive(&ports, session, &path).await;
        let mut corrupted = archive
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        let mut changed = false;
        for record in &mut corrupted {
            if let Some(encoded) = record["data"]["facts"]["interaction"]["decision_json"].as_str()
            {
                let mut decision: Value = serde_json::from_str(encoded).unwrap();
                if decision["kind"] == "permission" {
                    decision["value"]["reply"]["request_id"] = json!("foreign-interaction");
                    record["data"]["facts"]["interaction"]["decision_json"] =
                        json!(encode(&decision).unwrap());
                    changed = true;
                }
            }
        }
        assert!(changed);
        let corrupted_path = root.path().join("foreign-permission.jsonl");
        tokio::fs::write(&corrupted_path, rebuild(&corrupted))
            .await
            .unwrap();
        assert!(
            ports
                .sessions
                .import_archive(RequestId::new(), 1, &corrupted_path)
                .await
                .is_err()
        );
        let mut feed = ports.reader.watch_all(None).unwrap();
        let request = RequestId::new();
        let imported = ports
            .sessions
            .import_archive(request, 1, &path)
            .await
            .unwrap();
        assert_ne!(imported.value.id, session);
        let change = feed
            .try_recv()
            .unwrap()
            .expect("import must publish one scoped change");
        assert_eq!(change.session, imported.value.id);
        assert_eq!(change.cursor.position, imported.position);
        assert_eq!(change.kind, ChangeKind::Membership);
        assert!(change.requires_snapshot);
        let list = change
            .list_change
            .as_ref()
            .expect("import must capture its committed list scope");
        assert!(list.before.is_none());
        assert!(list.parents.is_empty());
        let after = list.after.as_ref().unwrap();
        assert_eq!(after.session, imported.value.id);
        assert_eq!(after.workspace_id, 1);
        assert_eq!(after.parent_session_id, None);
        assert_eq!(after.favorite, imported.value.favorite);
        assert_eq!(after.pinned, imported.value.pinned);
        assert_eq!(
            after.metadata_version,
            imported.value.metadata_version.get()
        );
        assert_eq!(after.queued_executions, 0);
        assert_eq!(after.running_executions, 0);
        assert_eq!(after.include_in_lists, change.include_in_lists);
        drop(change);
        assert_eq!(
            ports
                .sessions
                .import_archive(request, 1, &path)
                .await
                .unwrap(),
            imported
        );
        assert!(
            feed.try_recv().unwrap().is_none(),
            "idempotent import replay must not publish again"
        );
        drop(feed);
        let history = ports
            .reader
            .history(imported.value.id, None, 200)
            .await
            .unwrap();
        assert_eq!(history.entries.len(), original_ids.len());
        assert!(
            history
                .entries
                .iter()
                .all(|e| !original_ids.contains(&e.id))
        );
        assert!(
            history
                .entries
                .iter()
                .all(|e| e.execution_phase == ExecutionPhase::Interrupted)
        );
        let imported_tool = history
            .entries
            .iter()
            .find(|e| matches!(e.payload, EntryPayload::Tool { .. }))
            .unwrap();
        let EntryPayload::Tool { tool_id } = imported_tool.payload else {
            unreachable!()
        };
        let tool_view = ports.reader.tool(imported.value.id, tool_id).await.unwrap();
        assert_eq!(tool_view.interactions.len(), 2);
        for id in &tool_view.interactions {
            assert_ne!(*id, permission);
            assert_ne!(*id, input);
            let view = ports
                .reader
                .interaction(imported.value.id, *id)
                .await
                .unwrap();
            match &view.request {
                InteractionRequest::Permission { .. } => {
                    let mut expected = permission_reply.clone();
                    expected.request_id = id.get().to_string();
                    assert_eq!(
                        view.decision,
                        Some(InteractionDecision::Permission {
                            reply: expected,
                            allowed: false
                        })
                    );
                    assert_eq!(view.decided_at_ms, permission_decided_at);
                    assert_eq!(
                        view.remembered.as_ref().unwrap().scope,
                        agena_domain::PermissionScope::Session
                    );
                }
                InteractionRequest::Input { request } => {
                    assert_eq!(request.request_id, id.get().to_string());
                    assert_eq!(request.session_id, Some(imported.value.id.get()));
                    let Some(InteractionDecision::Input(encoded)) = &view.decision else {
                        panic!("import lost typed input reply")
                    };
                    let mut expected = input_reply.clone();
                    expected.request_id = id.get().to_string();
                    assert_eq!(
                        serde_json::from_str::<agena_domain::UserInputReply>(encoded).unwrap(),
                        expected
                    );
                }
            }
        }
        let sources = &tool_view.outputs;
        assert_eq!(sources.len(), 3);
        assert!(
            sources
                .iter()
                .all(|s| s.id != output.id() && s.id != terminal.id() && s.id != mixed.id())
        );
        let log = sources.iter().find(|s| s.kind == ContentKind::Log).unwrap();
        let log = ports
            .reader
            .watch_content(imported.value.id, log.id)
            .await
            .unwrap()
            .head()
            .unwrap();
        assert_eq!(log.capture_loss.committed_bytes, 7);
        assert_eq!(log.state, CaptureState::Interrupted);
        let events = ports
            .reader
            .event_page(imported.value.id, log.id, None, 20)
            .await
            .unwrap();
        assert_eq!(events.records.len(), 2);
        assert_eq!(events.records[1].timestamp_ms, 124);
        let blob = ports
            .reader
            .entry_blobs(imported.value.id, imported_tool.id)
            .await
            .unwrap()[0]
            .clone();
        assert_ne!(blob.id, old_blob);
        let reader = ports
            .reader
            .open_blob(imported.value.id, blob.id)
            .await
            .unwrap();
        let mut actual = Vec::new();
        reader.copy_to(&mut actual).await.unwrap();
        assert_eq!(actual, binary);
        let imported_state: String =
            sqlx::query_scalar("SELECT provider_state_json FROM agena_rounds WHERE session_id=?")
                .bind(imported.value.id.get())
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
        assert_eq!(imported_state, state);
        assert!(
            ports
                .reader
                .active_executions(imported.value.id, None, 20)
                .await
                .unwrap()
                .executions
                .is_empty()
        );
        drop(history);
        drop(tool_view);
        drop(events);
        drop(log);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn corrupt_or_incomplete_archive_never_publishes_a_partial_session() {
    let (ports, _maintenance, admin, _diagnostics, root) = fixture(true).await;
    let (session, _round, tool) = event_tests::tool(&ports).await;
    let output = tool.begin_log(RequestId::new()).await.unwrap().value;
    output
        .append(CommandOutputStream::Stderr, "bytes")
        .await
        .unwrap();
    output.finish(RequestId::new()).await.unwrap();
    let path = root.path().join("archive.jsonl");
    let bytes = write_archive(&ports, session, &path).await;
    let records = bytes
        .split(|b| *b == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agena_sessions")
        .fetch_one(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
    let mut variants = Vec::new();
    variants.push(bytes[..bytes.len() - 12].to_vec());
    let no_piece = records
        .iter()
        .filter(|r| r["kind"] != "content_piece")
        .cloned()
        .collect::<Vec<_>>();
    variants.push(rebuild(&no_piece));
    let no_source = records
        .iter()
        .filter(|r| {
            r["kind"] != "content" && r["kind"] != "content_block" && r["kind"] != "content_piece"
        })
        .cloned()
        .collect::<Vec<_>>();
    variants.push(rebuild(&no_source));
    for variant in variants {
        tokio::fs::write(&path, variant).await.unwrap();
        assert!(
            ports
                .sessions
                .import_archive(RequestId::new(), 1, &path)
                .await
                .is_err()
        );
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agena_sessions")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        assert_eq!(after, before);
    }
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn archive_fork_exports_dependency_closure_without_promoting_support_to_membership() {
    let (ports, _maintenance, admin, _diagnostics, root) = fixture(false).await;
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "origin".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, document("request"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await
        .unwrap()
        .value;
    let text = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    text.append("support response").await.unwrap();
    text.finish(RequestId::new()).await.unwrap();
    let branch = ports
        .sessions
        .fork(RequestId::new(), session, sent.message_id)
        .await
        .unwrap()
        .value
        .id;
    let path = root.path().join("fork.jsonl");
    let bytes = write_archive(&ports, branch, &path).await;
    let records = bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice::<Value>(l).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.iter().filter(|r| r["kind"] == "entry").count(), 1);
    assert!(records.iter().any(|r| r["kind"] == "support_entry"));
    let mut feed = ports.reader.watch_all(None).unwrap();
    let imported = ports
        .sessions
        .import_archive(RequestId::new(), 1, &path)
        .await
        .unwrap()
        .value
        .id;
    let change = feed.try_recv().unwrap().unwrap();
    let list = change.list_change.as_ref().unwrap();
    assert!(list.before.is_none());
    assert!(list.parents.is_empty());
    assert_eq!(list.after.as_ref().unwrap().parent_session_id, None);
    drop(change);
    drop(feed);
    let visible = ports.reader.history(imported, None, 20).await.unwrap();
    assert_eq!(visible.entries.len(), 1);
    assert!(matches!(
        visible.entries[0].payload,
        EntryPayload::User { .. }
    ));
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agena_entries WHERE origin_session_id=?")
            .bind(imported.get())
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
    assert!(total > 1);
    let mut missing = records.clone();
    missing.retain(|r| r["kind"] != "support_entry");
    tokio::fs::write(&path, rebuild(&missing)).await.unwrap();
    assert!(
        ports
            .sessions
            .import_archive(RequestId::new(), 1, &path)
            .await
            .is_err()
    );
    drop(visible);
    ports.shutdown().await.unwrap();
}
