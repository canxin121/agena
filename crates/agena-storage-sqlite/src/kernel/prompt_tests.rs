use agena_domain::{CommandOutputStream, DocumentMutation, ViewBlock};

use super::{
    tests::{document, fixture, fixture_with_diagnostics},
    *,
};

async fn committed(reader: &KernelReader, session: SessionId, content: ContentId, through: u64) {
    let mut watch = reader.watch_content(session, content).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if watch.head().unwrap().committed >= through {
                break;
            }
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn provider_input_keeps_its_metadata_outcomes_replay_and_source_heads_after_later_writes() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, admin, _root, url) =
            fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let (session, origin, tool) = event_tests::tool(&ports).await;
        ports
            .sessions
            .update_selection(
                RequestId::new(),
                session,
                SelectionChange::ThinkingMode(Some("original-mode".into())),
            )
            .await
            .unwrap();
        let text = tool.begin_text(RequestId::new()).await.unwrap().value;
        let log = tool.begin_log(RequestId::new()).await.unwrap().value;
        let doc = tool.begin_document(RequestId::new()).await.unwrap().value;
        let terminal = tool.begin_terminal(RequestId::new()).await.unwrap().value;
        let prefix = "  界🙂\r\n\0".repeat(8192);
        let text_cursor = text.append(&prefix).await.unwrap();
        let log_cursor = log
            .append(CommandOutputStream::Stdout, "first stdout\r\n")
            .await
            .unwrap();
        let doc_cursor = doc
            .apply(DocumentMutation::Insert {
                after: None,
                block: ViewBlock::Text {
                    id: Some("body".into()),
                    text: "before".into(),
                },
            })
            .await
            .unwrap();
        let screen = event_tests::screen("before");
        let terminal_cursor = terminal.snapshot(screen.clone()).await.unwrap();
        tokio::join!(
            committed(&ports.reader, session, text.id(), text_cursor.offset),
            committed(&ports.reader, session, log.id(), log_cursor.sequence),
            committed(&ports.reader, session, doc.id(), doc_cursor.sequence),
            committed(
                &ports.reader,
                session,
                terminal.id(),
                terminal_cursor.sequence
            ),
        );
        let interaction = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "prompt-fixture".into(),
                    description: "not yet decided".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let opaque = serde_json::json!({"opaque":"  世界\n\0\"\\", "nested":[null,1,{"preserve":[true,false]}]});
        origin
            .finish(RequestId::new(), Some(opaque.clone()), true)
            .await
            .unwrap();
        let request = ports
            .sessions
            .send(RequestId::new(), session, document("next provider request"))
            .await
            .unwrap()
            .value;
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, request.id)
            .await
            .unwrap()
            .value;
        let history = ports.reader.history(session, None, 200).await.unwrap();
        let inputs = history
            .entries
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        let round = attempt
            .begin_round(RequestId::new(), inputs.clone())
            .await
            .unwrap()
            .value;
        let frozen = round.prompt().await.unwrap();
        let expected = serde_json::to_value(&frozen).unwrap();
        assert_eq!(
            frozen
                .entries
                .iter()
                .map(|entry| entry.entry.id)
                .collect::<Vec<_>>(),
            inputs
        );
        assert_eq!(frozen.sources.len(), 4);
        assert_eq!(frozen.replays.len(), 1);
        assert_eq!(frozen.replays[0].provider_state, Some(opaque));
        assert_eq!(frozen.replays[0].phase, ProviderRoundPhase::Succeeded);
        assert!(
            frozen
                .entries
                .iter()
                .find_map(|entry| entry.tool.as_ref())
                .is_some_and(|tool| tool.phase == ToolPhase::Running && tool.result.is_none())
        );
        assert!(
            frozen
                .entries
                .iter()
                .find_map(|entry| entry.interaction.as_ref())
                .is_some_and(|interaction| interaction.decision.is_none())
        );
        let start = diagnostics.snapshot();
        assert!(frozen.ptr_eq(&round.prompt().await.unwrap()));
        assert_eq!(diagnostics.snapshot().prompt_queries, start.prompt_queries);
        assert_eq!(
            diagnostics.snapshot().prompt_body_bytes,
            start.prompt_body_bytes
        );

        text.append("later text which cannot enter the frozen request")
            .await
            .unwrap();
        log.append(CommandOutputStream::Stderr, "later stderr")
            .await
            .unwrap();
        doc.apply(DocumentMutation::AppendText {
            block_id: "body".into(),
            text: " later".into(),
        })
        .await
        .unwrap();
        terminal
            .snapshot(event_tests::screen("after"))
            .await
            .unwrap();
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                interaction,
                super::tests::permission_decision(interaction, true, None),
                None,
            )
            .await
            .unwrap();
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"later_result":true}),
            },
        )
        .await
        .unwrap();
        ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    title: Some("later title".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        ports
            .sessions
            .send(RequestId::new(), session, document("later user message"))
            .await
            .unwrap();
        ports
            .sessions
            .update_selection(
                RequestId::new(),
                session,
                SelectionChange::ThinkingMode(Some("later-mode".into())),
            )
            .await
            .unwrap();
        let refilled = round.prompt().await.unwrap();
        assert_eq!(serde_json::to_value(&refilled).unwrap(), expected);
        let text_head = frozen
            .sources
            .iter()
            .find(|head| head.id == text.id())
            .unwrap();
        let mut after = None;
        let mut bytes = Vec::new();
        loop {
            let page = ports
                .reader
                .text_range_at(session, text_head, after, 777)
                .await
                .unwrap();
            bytes.extend_from_slice(&page.bytes);
            after = Some(page.next);
            if !page.has_more {
                break;
            }
        }
        assert_eq!(bytes, prefix.as_bytes());
        let log_head = frozen
            .sources
            .iter()
            .find(|head| head.id == log.id())
            .unwrap();
        let page = ports
            .reader
            .event_page_at(session, log_head, None, 256)
            .await
            .unwrap();
        assert_eq!(page.records.len(), 1);
        assert!(!page.has_more);
        assert!(
            matches!(&page.records[0].payload, EventPayload::Log { channel: CommandOutputStream::Stdout, text } if text == "first stdout\r\n")
        );
        let doc_head = frozen
            .sources
            .iter()
            .find(|head| head.id == doc.id())
            .unwrap();
        let snapshot = ports
            .reader
            .event_snapshot_at(session, doc_head)
            .await
            .unwrap();
        assert!(
            matches!(&snapshot.document.as_ref().unwrap().blocks[0], ViewBlock::Text {text,..} if text == "before")
        );
        let terminal_head = frozen
            .sources
            .iter()
            .find(|head| head.id == terminal.id())
            .unwrap();
        assert_eq!(
            ports
                .reader
                .event_snapshot_at(session, terminal_head)
                .await
                .unwrap()
                .terminal
                .as_deref(),
            Some(&screen)
        );
        let other = ports
            .sessions
            .create(RequestId::new(), 1, "foreign".into())
            .await
            .unwrap()
            .value
            .id;
        assert!(matches!(
            ports.reader.prompt(other, round.id()).await,
            Err(KernelError::NotFound(_))
        ));
        assert!(matches!(
            ports
                .reader
                .text_range_at(other, text_head, None, 100)
                .await,
            Err(KernelError::Ownership)
        ));
        let round_id = round.id();
        drop((
            admin, origin, tool, text, log, doc, terminal, round, attempt,
        ));
        ports.shutdown().await.unwrap();
        if !memory {
            let reopened = open_session_kernel(&url).await.unwrap();
            assert_eq!(
                serde_json::to_value(reopened.reader.prompt(session, round_id).await.unwrap())
                    .unwrap(),
                expected
            );
            reopened.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn provider_input_byte_preflight_rejects_oversized_windows_without_partial_rounds() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let session = ports
            .sessions
            .create(RequestId::new(), 1, "bounded prompt".into())
            .await
            .unwrap()
            .value
            .id;
        let mut inputs = Vec::new();
        let mut latest = None;
        let text = "界".repeat(80_000);
        for _ in 0..36 {
            let sent = ports
                .sessions
                .send(RequestId::new(), session, document(&text))
                .await
                .unwrap()
                .value;
            inputs.push(sent.message_id);
            latest = Some(sent.id);
        }
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, latest.unwrap())
            .await
            .unwrap()
            .value;
        let request = RequestId::new();
        assert!(matches!(
            attempt.begin_round(request, inputs.clone()).await,
            Err(KernelError::Invalid(_))
        ));
        for table in [
            "agena_rounds",
            "agena_round_inputs",
            "agena_round_input_sources",
            "agena_round_input_replays",
        ] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(admin.get_sqlite_connection_pool())
                .await
                .unwrap();
            assert_eq!(count, 0, "{table} survived a failed input savepoint");
        }
        let round = attempt
            .begin_round(request, inputs[34..].to_vec())
            .await
            .unwrap()
            .value;
        let prompt = round.prompt().await.unwrap();
        assert_eq!(prompt.entries.len(), 2);
        assert!(prompt.entries.iter().all(|entry| matches!(&entry.entry.payload, EntryPayload::User {document} if document.text() == text)));
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn input_preflight_does_not_read_entry_tool_or_interaction_json_tables() {
    let (ports, admin, _root, _url) = fixture(true).await;
    let roots = sqlx::query("SELECT rootpage FROM sqlite_schema WHERE name IN ('agena_entries','agena_tool_calls','agena_interactions')")
        .fetch_all(admin.get_sqlite_connection_pool()).await.unwrap()
        .into_iter().map(|row| row.get::<i64,_>("rootpage")).collect::<std::collections::BTreeSet<_>>();
    let rows = sqlx::query(&format!("EXPLAIN {}", prompt::INPUT_PREFLIGHT_SQL))
        .bind(1i64)
        .fetch_all(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
    let cursors = rows
        .iter()
        .filter(|row| {
            row.get::<String, _>("opcode") == "OpenRead" && roots.contains(&row.get::<i64, _>("p2"))
        })
        .map(|row| row.get::<i64, _>("p1"))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        !rows
            .iter()
            .any(|row| row.get::<String, _>("opcode") == "Column"
                && cursors.contains(&row.get::<i64, _>("p1")))
    );
    ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn frozen_heads_exclude_uncommitted_events_and_report_retention_and_wrong_epochs() {
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        root.path().join("fixed-heads.db").display()
    );
    let kernel = open_kernel_runtime(&url).await.unwrap();
    kernel
        .workspaces
        .create(root.path().to_string_lossy().into_owned())
        .await
        .unwrap();
    let ports = kernel.ports;
    let (session, _round, tool) = event_tests::tool(&ports).await;
    let text = tool.begin_text(RequestId::new()).await.unwrap().value;
    let terminal = tool.begin_terminal(RequestId::new()).await.unwrap().value;
    let before_text = ports
        .reader
        .watch_content(session, text.id())
        .await
        .unwrap()
        .head()
        .unwrap();
    let before_terminal = ports
        .reader
        .watch_content(session, terminal.id())
        .await
        .unwrap()
        .head()
        .unwrap();
    assert_eq!(before_text.committed, 0);
    assert_eq!(before_terminal.committed, 0);
    text.append("old prefix").await.unwrap();
    terminal
        .snapshot(event_tests::screen("new live frame"))
        .await
        .unwrap();
    assert!(
        ports
            .reader
            .text_range_at(session, &before_text, None, 100)
            .await
            .unwrap()
            .bytes
            .is_empty()
    );
    assert!(matches!(
        ports
            .reader
            .event_snapshot_at(session, &before_terminal)
            .await,
        Err(KernelError::NotFound(_))
    ));
    let complete = text.finish(RequestId::new()).await.unwrap().value;
    kernel
        .maintenance
        .prune_content(RequestId::new(), session, text.id(), complete.committed)
        .await
        .unwrap();
    assert!(matches!(
        ports
            .reader
            .text_range_at(session, &complete, None, 100)
            .await,
        Err(KernelError::ContentGap { .. })
    ));
    let terminal_id = terminal.id();
    drop((
        tool,
        text,
        terminal,
        _round,
        kernel.workspaces,
        kernel.permissions,
        kernel.model_catalog,
        kernel.maintenance,
        kernel.database,
    ));
    ports.shutdown().await.unwrap();
    let reopened = open_session_kernel(&url).await.unwrap();
    // Graceful shutdown seals this capture without changing the meaning of
    // its old committed cursor. A kill/reopen reset is tested in the real
    // process fixture, where the capture is still open when killed.
    assert!(
        reopened
            .reader
            .event_page_at(session, &before_terminal, None, 10)
            .await
            .unwrap()
            .records
            .is_empty()
    );
    assert_eq!(
        reopened
            .reader
            .event_snapshot(session, terminal_id)
            .await
            .unwrap()
            .stream
            .epoch,
        before_terminal.epoch
    );
    let mut wrong_epoch = (*before_terminal).clone();
    wrong_epoch.epoch = ContentEpoch::new();
    assert!(matches!(
        reopened
            .reader
            .event_page_at(session, &wrong_epoch, None, 10)
            .await,
        Err(KernelError::ContentReset)
    ));
    reopened.shutdown().await.unwrap();
}
