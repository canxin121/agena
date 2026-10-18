use super::{
    event_tests::tool as running_tool,
    tests::{fixture, sql},
    *,
};

#[tokio::test]
async fn discarded_pipe_packets_have_immutable_durable_windows_without_fake_content_cursors() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = running_tool(&ports).await;
        let text = tool.begin_text(RequestId::new()).await.unwrap().value;
        let first = text.try_append("A界\r\n").unwrap();
        text.record_loss(7, &"丢失".repeat(100)).unwrap();
        text.record_loss(0, "screen update was discarded").unwrap();
        let second = text.try_append("🙂B").unwrap();
        assert_eq!(second.offset, first.offset + "🙂B".len() as u64);
        text.record_loss(9, "a separate discarded packet").unwrap();
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"exit_code":0}),
            },
        )
        .await
        .unwrap();
        let page = ports
            .reader
            .capture_losses(session, text.id(), None, 1)
            .await
            .unwrap();
        assert_eq!(
            page.stream.capture_loss,
            CaptureLossCounters {
                bytes: 16,
                packets: 3,
                committed_bytes: 16,
                committed_packets: 3
            }
        );
        assert_eq!(page.stream.state, CaptureState::Interrupted);
        assert_eq!(page.losses.len(), 1);
        assert_eq!(
            (
                page.losses[0].first_packet,
                page.losses[0].last_packet,
                page.losses[0].bytes
            ),
            (1, 2, 7)
        );
        assert_eq!(page.losses[0].after_position, first.offset);
        assert!(page.losses[0].diagnostic.len() <= 256);
        assert!(page.has_more);
        let tail = ports
            .reader
            .capture_losses(session, text.id(), Some(page.next), 256)
            .await
            .unwrap();
        assert_eq!(tail.losses[0].after_position, second.offset);
        assert_eq!(tail.next.packet, 3);
        assert!(!tail.has_more);
        let invalid_cursor = CaptureLossCursor {
            packet: 1,
            ..page.next
        };
        assert!(matches!(
            ports
                .reader
                .capture_losses(session, text.id(), Some(invalid_cursor), 1)
                .await,
            Err(KernelError::Invalid(_))
        ));
        let accepted = ports
            .reader
            .text_range(session, text.id(), None, 1024)
            .await
            .unwrap();
        assert_eq!(accepted.bytes, "A界\r\n🙂B".as_bytes());
        assert_eq!(accepted.stream.accepted, second.offset);
        assert_eq!(
            ports.reader.tool(session, tool.id()).await.unwrap().phase,
            ToolPhase::Succeeded
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn losses_roll_back_with_the_required_tail_and_retry_without_double_accounting() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _round, tool) = running_tool(&ports).await;
        let log = tool.begin_log(RequestId::new()).await.unwrap().value;
        log.try_append(agena_domain::CommandOutputStream::Stderr, "accepted\r\n")
            .unwrap();
        log.record_loss(19, "reader capacity exhausted").unwrap();
        sql(&admin,"CREATE TRIGGER reject_loss_commit BEFORE UPDATE OF lost_packets ON agena_content_streams WHEN NEW.lost_packets>0 BEGIN SELECT RAISE(ABORT,'loss commit fault'); END").await;
        let request = RequestId::new();
        let result = ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"observed":true}),
        };
        assert!(tool.complete(request, result.clone()).await.is_err());
        let durable = ports
            .reader
            .capture_losses(session, log.id(), None, 10)
            .await
            .unwrap();
        assert_eq!(durable.stream.capture_loss.committed_packets, 0);
        assert!(durable.losses.is_empty());
        sql(&admin, "DROP TRIGGER reject_loss_commit").await;
        tool.complete(request, result).await.unwrap();
        let durable = ports
            .reader
            .capture_losses(session, log.id(), None, 10)
            .await
            .unwrap();
        assert_eq!(durable.stream.capture_loss.committed_packets, 1);
        assert_eq!(durable.losses[0].bytes, 19);
        assert_eq!(durable.stream.committed, 1);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn capture_diagnostics_keep_original_positions_and_branch_read_authorization() {
    let (ports, _admin, _root, _url) = fixture(false).await;
    let (session, round, tool) = running_tool(&ports).await;
    let log = tool.begin_log(RequestId::new()).await.unwrap().value;
    log.try_append(agena_domain::CommandOutputStream::Stdout, "first")
        .unwrap();
    log.record_loss(5, "first packet lost").unwrap();
    log.try_append(agena_domain::CommandOutputStream::Stdout, "last")
        .unwrap();
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::Value::Null,
        },
    )
    .await
    .unwrap();
    let entry = ports
        .reader
        .history(session, None, 20)
        .await
        .unwrap()
        .entries
        .iter()
        .find(|entry| matches!(entry.payload, EntryPayload::Tool { .. }))
        .unwrap()
        .id;
    let child = ports
        .sessions
        .fork(RequestId::new(), session, entry)
        .await
        .unwrap()
        .value
        .id;
    let page = ports
        .reader
        .capture_losses(child, log.id(), None, 10)
        .await
        .unwrap();
    assert_eq!(page.losses[0].after_position, 1);
    let other = ports
        .sessions
        .create(RequestId::new(), 1, "unrelated".into())
        .await
        .unwrap()
        .value
        .id;
    assert!(matches!(
        ports.reader.capture_losses(other, log.id(), None, 10).await,
        Err(KernelError::Ownership)
    ));
    round.finish(RequestId::new(), None, true).await.unwrap();
    ports.shutdown().await.unwrap();
}
