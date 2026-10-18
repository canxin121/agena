use agena_domain::{CommandOutputStream, DocumentMutation, ViewBlock};
use serde_json::json;

use super::{
    event_tests::screen,
    tests::{document as input, fixture, sql},
    *,
};

async fn tool(ports: &KernelPorts) -> (SessionId, AttemptHandle, RoundHandle, ToolHandle) {
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "background".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = ports
        .sessions
        .send(RequestId::new(), session, input("background"))
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
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "launch".into(),
                name: "fixture".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    (session, attempt, round, tool)
}

fn launch() -> BackgroundActivityStart {
    BackgroundActivityStart {
        kind: BackgroundActivityKind::Process,
        label: "scripted background process".into(),
    }
}

fn success() -> ToolResult {
    ToolResult {
        phase: ToolPhase::Succeeded,
        value: json!({"launched":true}),
    }
}

#[tokio::test]
async fn activity_outputs_survive_the_launch_tool_and_attempt_then_seal_with_their_own_outcome() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, attempt, round, tool) = tool(&ports).await;
        let launch_request = RequestId::new();
        let launched = tool.begin_activity(launch_request, launch()).await.unwrap();
        let activity = &launched.value;
        let replay = tool.begin_activity(launch_request, launch()).await.unwrap();
        assert_eq!(launched.position, replay.position);
        assert_eq!(activity.id(), replay.value.id());
        let text = activity.begin_text(RequestId::new()).await.unwrap().value;
        let log = activity.begin_log(RequestId::new()).await.unwrap().value;
        let terminal = activity
            .begin_terminal(RequestId::new())
            .await
            .unwrap()
            .value;
        let document = activity
            .begin_document(RequestId::new())
            .await
            .unwrap()
            .value;
        let foreground = tool.begin_text(RequestId::new()).await.unwrap().value;
        foreground.append("launch output").await.unwrap();
        text.append("before ").await.unwrap();
        let mut watch = ports
            .reader
            .watch_content(session, text.id())
            .await
            .unwrap();
        tool.complete(RequestId::new(), success()).await.unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        // Successful launch attempt has no running tools/rounds. Its activity
        // remains a separately owned item and continues at the original epoch.
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        assert!(foreground.append("late foreground").await.is_err());
        text.append("after 界🙂\r\n\0").await.unwrap();
        log.append_at(123, CommandOutputStream::Stderr, "err\u{1b}[31m\r\n")
            .await
            .unwrap();
        let base = terminal.snapshot(screen("before")).await.unwrap();
        terminal
            .patch(base, screen("after"), vec![0, 1])
            .await
            .unwrap();
        document
            .apply(DocumentMutation::Insert {
                after: None,
                block: ViewBlock::Text {
                    id: Some("block".into()),
                    text: "kept".into(),
                },
            })
            .await
            .unwrap();
        // Opening more output is activity-scoped too; it requires no live
        // attempt or launch-tool execution capability.
        let late = activity.begin_text(RequestId::new()).await.unwrap().value;
        late.append("opened after parent finish").await.unwrap();
        assert_eq!(
            ports
                .reader
                .active_executions(session, None, 10)
                .await
                .unwrap()
                .executions
                .len(),
            0
        );
        assert_eq!(
            ports
                .reader
                .active_activities(session, None, 10)
                .await
                .unwrap()
                .activities
                .len(),
            1
        );
        let view = ports.reader.tool(session, tool.id()).await.unwrap();
        assert_eq!(view.phase, ToolPhase::Succeeded);
        assert_eq!(view.activities, vec![activity.id()]);
        assert_eq!(
            view.outputs
                .iter()
                .filter(|out| out.activity_id == Some(activity.id()))
                .count(),
            5
        );
        let outcome = BackgroundActivityOutcome::Succeeded {
            value: json!({"exit_code":0,"opaque":[null,"世界"]}),
        };
        let end_request = RequestId::new();
        let ended = activity.end(end_request, outcome.clone()).await.unwrap();
        assert_eq!(
            ended,
            activity.end(end_request, outcome.clone()).await.unwrap()
        );
        // A new request for the same observed outcome is harmless; a
        // contradictory observed outcome cannot overwrite it.
        activity
            .end(RequestId::new(), outcome.clone())
            .await
            .unwrap();
        assert!(
            activity
                .end(RequestId::new(), BackgroundActivityOutcome::Stopped)
                .await
                .is_err()
        );
        let head = watch.head().unwrap();
        assert_eq!(head.state, CaptureState::Complete);
        assert_eq!(head.accepted, head.committed);
        assert_eq!(head.activity_id, Some(activity.id()));
        assert_eq!(head.epoch, text.epoch());
        assert!(text.append("late").await.is_err());
        assert!(activity.begin_log(RequestId::new()).await.is_err());
        assert!(
            ports
                .reader
                .active_activities(session, None, 10)
                .await
                .unwrap()
                .activities
                .is_empty()
        );
        let stored = ports.reader.activity(session, activity.id()).await.unwrap();
        assert_eq!(stored.outcome, Some(outcome));
        assert_eq!(stored.launch_tool, tool.id());
        assert_eq!(
            ports
                .reader
                .text_range(session, text.id(), None, 1024)
                .await
                .unwrap()
                .bytes,
            b"before after \xe7\x95\x8c\xf0\x9f\x99\x82\r\n\0"
        );
        assert_eq!(
            ports
                .reader
                .event_snapshot(session, terminal.id())
                .await
                .unwrap()
                .terminal
                .as_deref(),
            Some(&screen("after"))
        );
        let mut archive = Vec::new();
        ports
            .reader
            .export_jsonl(session, &mut archive)
            .await
            .unwrap();
        let entries: Vec<serde_json::Value> = std::str::from_utf8(&archive)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|line: &serde_json::Value| line["kind"] == "entry")
            .collect();
        let fact = entries
            .iter()
            .find(|entry| {
                entry["data"]["facts"]["activities"]
                    .as_array()
                    .is_some_and(|rows| !rows.is_empty())
            })
            .unwrap();
        assert_eq!(fact["data"]["facts"]["activities"][0]["phase"], "succeeded");
        assert_eq!(
            fact["data"]["facts"]["activities"][0]["id"],
            activity.id().to_string()
        );
        let foreign: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        assert_eq!(foreign, 0);
        watch.changed().await.unwrap();
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn parent_cancellation_and_retry_fence_attempt_output_without_fencing_independent_activity() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = fixture(memory).await;
        let (session, _attempt, round, tool) = tool(&ports).await;
        let activity = tool
            .begin_activity(RequestId::new(), launch())
            .await
            .unwrap()
            .value;
        let output = activity.begin_log(RequestId::new()).await.unwrap().value;
        let ordinary = tool.begin_text(RequestId::new()).await.unwrap().value;
        ordinary.append("before cancellation").await.unwrap();
        ports
            .sessions
            .cancel(RequestId::new(), session, round.execution_id())
            .await
            .unwrap();
        output
            .append(CommandOutputStream::Stdout, "after cancellation")
            .await
            .unwrap();
        ports
            .sessions
            .retry(RequestId::new(), session, round.execution_id())
            .await
            .unwrap();
        ports
            .executor
            .claim(RequestId::new(), session, round.execution_id())
            .await
            .unwrap();
        assert!(ordinary.append("superseded").await.is_err());
        output
            .append(CommandOutputStream::Stderr, "after retry")
            .await
            .unwrap();
        assert!(
            tool.begin_activity(RequestId::new(), launch())
                .await
                .is_err()
        );
        activity
            .end(RequestId::new(), BackgroundActivityOutcome::Stopped)
            .await
            .unwrap();
        let log = ports
            .reader
            .event_page(session, output.id(), None, 100)
            .await
            .unwrap();
        assert_eq!(log.records.len(), 2);
        assert_eq!(log.stream.state, CaptureState::Complete);
        assert_eq!(log.stream.committed, 2);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn activity_launch_requires_local_authorization_and_read_membership() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _attempt, round, tool) = tool(&ports).await;
        let permission = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "scripted".into(),
                    description: "launch".into(),
                },
            )
            .await
            .unwrap()
            .value;
        let request = RequestId::new();
        assert!(matches!(
            tool.begin_activity(request, launch()).await,
            Err(KernelError::Conflict(_))
        ));
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                permission,
                super::tests::permission_decision(permission, false, None),
                None,
            )
            .await
            .unwrap();
        assert!(tool.begin_activity(request, launch()).await.is_err());
        let hosted = round
            .begin_provider_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "hosted".into(),
                    name: "hosted".into(),
                    input: json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        assert!(
            hosted
                .begin_activity(RequestId::new(), launch())
                .await
                .is_err()
        );
        let allowed = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "allowed".into(),
                    name: "allowed".into(),
                    input: json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let permission = allowed
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: "allowed".into(),
                    description: "allowed".into(),
                },
            )
            .await
            .unwrap()
            .value;
        ports
            .sessions
            .reply(
                RequestId::new(),
                session,
                permission,
                super::tests::permission_decision(permission, true, None),
                None,
            )
            .await
            .unwrap();
        let activity = allowed
            .begin_activity(request, launch())
            .await
            .unwrap()
            .value;
        let unrelated = ports
            .sessions
            .create(RequestId::new(), 1, "unrelated".into())
            .await
            .unwrap()
            .value
            .id;
        assert!(
            ports
                .reader
                .activity(unrelated, activity.id())
                .await
                .is_err()
        );
        assert!(
            ports
                .reader
                .active_activities(
                    unrelated,
                    Some(ActiveActivityCursor {
                        session,
                        after: activity.id()
                    }),
                    10
                )
                .await
                .is_err()
        );
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM agena_background_activities")
            .fetch_one(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
        let pages = ports
            .reader
            .active_activities(session, None, 1)
            .await
            .unwrap();
        assert_eq!(pages.activities.len(), 1);
        assert!(!pages.has_more);
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn rejected_activity_commit_keeps_the_exact_outcome_and_accepted_tail_retryable() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = fixture(memory).await;
        let (session, _attempt, _round, tool) = tool(&ports).await;
        let activity = tool
            .begin_activity(RequestId::new(), launch())
            .await
            .unwrap()
            .value;
        let output = activity.begin_text(RequestId::new()).await.unwrap().value;
        output
            .append("one external effect; retained tail 界\n")
            .await
            .unwrap();
        let watch = ports
            .reader
            .watch_content(session, output.id())
            .await
            .unwrap();
        sql(&admin,"CREATE TABLE reject_activity_commit (owner INTEGER REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED)").await;
        sql(&admin,"CREATE TRIGGER reject_activity AFTER UPDATE OF phase ON agena_background_activities WHEN NEW.phase!='running' BEGIN INSERT INTO reject_activity_commit VALUES (-1); END").await;
        let request = RequestId::new();
        let outcome = BackgroundActivityOutcome::Failed {
            diagnostic: "observed child exit 7".into(),
        };
        assert!(matches!(
            activity.end(request, outcome.clone()).await,
            Err(KernelError::Storage(_))
        ));
        assert!(
            ports
                .reader
                .activity(session, activity.id())
                .await
                .unwrap()
                .outcome
                .is_none()
        );
        assert_eq!(watch.head().unwrap().state, CaptureState::Open);
        assert_eq!(
            ports
                .reader
                .text_range(session, output.id(), None, 1024)
                .await
                .unwrap()
                .bytes,
            "one external effect; retained tail 界\n".as_bytes()
        );
        sql(&admin, "DROP TRIGGER reject_activity").await;
        let ended = activity.end(request, outcome.clone()).await.unwrap();
        assert_eq!(ended, activity.end(request, outcome.clone()).await.unwrap());
        assert_eq!(
            ports
                .reader
                .activity(session, activity.id())
                .await
                .unwrap()
                .outcome,
            Some(outcome)
        );
        let head = watch.head().unwrap();
        assert_eq!(head.accepted, head.committed);
        // Process failure does not imply capture failure.
        assert_eq!(head.state, CaptureState::Complete);
        ports.shutdown().await.unwrap();
    }
}
