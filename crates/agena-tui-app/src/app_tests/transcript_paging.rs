use super::parts_fixtures;
use crate::app_backend::{SessionStateWithTranscriptPage, SessionTranscriptPage};
use crate::{App, I18n, LaunchOptions, TranscriptNodeKey, TranscriptState, TuiBackend};
use agena_api::live::SessionTranscriptFoldResource;
use agena_api::resource::{SessionExecutionResource, SessionTranscriptPart};
use agena_tui_transcript::{TranscriptContentId, TranscriptEntryId};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

const SESSION_ID: i64 = 7;
const WIDTH: u16 = 120;
const HEIGHT: u16 = 24;

fn execution(parts: Vec<SessionTranscriptPart>, version: i64) -> SessionExecutionResource {
    serde_json::from_value(json!({
        "session": {
            "id": SESSION_ID, "depth": 0, "root_id": SESSION_ID,
            "workspace_id": 1, "title": "Paging test", "version": version,
            "relation_kind": "root", "lifecycle_state": "ready",
            "created_at": "2026-10-03T00:00:00Z", "updated_at": "2026-10-03T00:00:00Z",
            "message_count": 1, "child_session_count": 0
        },
        "parts": parts, "latest_event_seq": version,
        "execution": {"agent_id": "test"}, "usage": {"current_tokens": 0}
    }))
    .expect("session execution fixture")
}

fn activities(range: std::ops::Range<i64>) -> Vec<SessionTranscriptPart> {
    range
        .map(|part_id| {
            parts_fixtures::hook(
                3,
                part_id,
                "assistant",
                &format!("Part {part_id}"),
                &format!("detail {part_id}"),
            )
        })
        .collect()
}

fn snapshot(anchor: i64, end: i64, version: i64) -> SessionStateWithTranscriptPage {
    let parts = std::iter::once(parts_fixtures::run(3, "assistant", "completed"))
        .chain(activities(anchor..end))
        .collect::<Vec<_>>();
    SessionStateWithTranscriptPage {
        execution: execution(parts.clone(), version),
        page: SessionTranscriptPage {
            parts,
            folds: vec![SessionTranscriptFoldResource {
                run_id: 3,
                run_ids: vec![3],
                anchor_part_id: anchor,
                hidden_count: (anchor - 4) as u64,
                next_cursor: Some(format!("before-{anchor}")),
            }],
            next_cursor: Some("older-messages".into()),
            has_more: true,
        },
    }
}

fn app(backend: TuiBackend) -> App {
    let mut app = App::new_with_backend(backend, LaunchOptions::default(), I18n::english());
    app.transcript.session_id = Some(SESSION_ID);
    app.layout.transcript_body = Rect::new(0, 0, WIDTH, HEIGHT);
    app
}

fn fold_key(anchor: i64) -> TranscriptNodeKey {
    TranscriptNodeKey::Activity {
        entry_id: TranscriptEntryId::StoredMessage(3),
        content_id: TranscriptContentId::TranscriptFold {
            run_id: 3,
            anchor_part_id: anchor,
        },
    }
}

fn assert_fold_visible(transcript: &mut TranscriptState, anchor: i64, hidden: u64) {
    let key = fold_key(anchor);
    let fold = transcript
        .transcript_fold_for_node(&key)
        .expect("load-more cursor");
    assert_eq!(fold.hidden_count, hidden);
    let rendered = transcript.rendered(WIDTH);
    let node = rendered
        .nodes
        .iter()
        .find(|node| node.key == key)
        .expect("load-more control must render");
    assert!(node.toggleable);
    assert!(
        rendered.lines[node.start_line]
            .text
            .contains("Enter: show 5")
    );
}

fn assert_parts_visible(transcript: &mut TranscriptState, range: std::ops::Range<i64>) {
    let rendered = transcript.rendered(WIDTH);
    for part_id in range {
        assert!(rendered.nodes.iter().any(|node| matches!(node.key,
            TranscriptNodeKey::Activity { content_id: TranscriptContentId::StoredPart(id), .. } if id == part_id
        )), "previously loaded part {part_id} must stay visible");
    }
}

fn wire_page(
    parts: &[SessionTranscriptPart],
    folds: &[SessionTranscriptFoldResource],
    has_more: bool,
    cursor: Option<&str>,
) -> serde_json::Value {
    let wire_parts = parts
        .iter()
        .map(|part| {
            let mut value = serde_json::to_value(part).unwrap();
            value.as_object_mut().unwrap().extend(
                json!({
                    "visibility": "user", "origin_session_id": SESSION_ID, "revision": 0,
                    "started_at_ms": part.created_at_ms, "updated_at_ms": part.created_at_ms
                })
                .as_object()
                .unwrap()
                .clone(),
            );
            value
        })
        .collect::<Vec<_>>();
    json!({
        "session_id": SESSION_ID, "version": 2, "parts": wire_parts, "folds": folds,
        "page": {"has_more": has_more, "next_cursor": cursor, "returned": wire_parts.len()}
    })
}

#[tokio::test]
async fn refresh_preserves_the_remote_part_loading_control_through_http() {
    let load = snapshot(64, 69, 2);
    let mut shell = load.execution.clone();
    shell.parts.clear();
    let transcript = wire_page(
        &load.page.parts,
        &load.page.folds,
        true,
        Some("older-messages"),
    );
    let first_page = wire_page(&activities(59..64), &[], true, Some("before-59"));
    let all_page = wire_page(&activities(9..59), &[], true, Some("before-9"));
    let last_page = wire_page(&activities(4..9), &[], false, None);
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let backend = TuiBackend::remote_mock_at(&format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        let mut paths = Vec::new();
        for _ in 0..7 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&buffer[..read]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            let body = if path.ends_with("/state") {
                serde_json::to_string(&shell).unwrap()
            } else if path.contains("/transcript/folds?") {
                match path {
                    "/api/v1/sessions/7/transcript/folds?limit=5&cursor=before-64" => {
                        first_page.to_string()
                    }
                    "/api/v1/sessions/7/transcript/folds?limit=50&cursor=before-59" => {
                        all_page.to_string()
                    }
                    "/api/v1/sessions/7/transcript/folds?limit=50&cursor=before-9" => {
                        last_page.to_string()
                    }
                    _ => panic!("unexpected fold request: {path}"),
                }
            } else {
                assert_eq!(path, "/api/v1/sessions/7/transcript?limit=3");
                transcript.to_string()
            };
            paths.push(path.to_owned());
            stream.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
            ).as_bytes()).await.unwrap();
        }
        assert_eq!(
            paths.iter().filter(|path| path.ends_with("/state")).count(),
            2
        );
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.contains("/transcript/folds?"))
                .count(),
            3
        );
    });
    let refresh = backend
        .refresh_session(SESSION_ID, Some(1), true)
        .await
        .unwrap();
    let mut app = app(backend);
    app.handle_session_refreshed(SESSION_ID, Ok(refresh));
    assert_fold_visible(&mut app.transcript, 64, 60);

    app.transcript.set_cursor_line(WIDTH, HEIGHT, 1);
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, 40)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(
        screen.contains("60 older parts hidden"),
        "load control must appear on the terminal"
    );

    let width = app.layout.transcript_body.width;
    let height = app.layout.transcript_body.height;
    let marker_line = app
        .transcript
        .rendered(width)
        .nodes
        .iter()
        .find(|node| node.key == fold_key(64))
        .unwrap()
        .start_line;
    app.transcript.set_cursor_line(width, height, marker_line);
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    app.handle_transcript_key(enter);
    app.handle_transcript_key(enter);
    assert_eq!(
        app.transcript.transcript_fold_loads.len(),
        1,
        "repeated Enter shares one fetch"
    );
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), app.rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        message,
        crate::AppMessage::TranscriptFoldPartsLoaded { .. }
    ));
    app.handle_message(message);
    assert_parts_visible(&mut app.transcript, 59..69);
    assert_fold_visible(&mut app.transcript, 59, 55);
    assert!(app.transcript.transcript_fold_loads.is_empty());

    app.handle_transcript_key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    for _ in 0..2 {
        let message = tokio::time::timeout(std::time::Duration::from_secs(3), app.rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            &message,
            crate::AppMessage::TranscriptFoldPartsLoaded {
                expand_all: true,
                result: Ok(_),
                ..
            }
        ));
        app.handle_message(message);
    }
    assert_parts_visible(&mut app.transcript, 4..69);
    assert!(app.transcript.transcript_folds.is_empty());
    assert!(app.transcript.transcript_fold_loads.is_empty());

    let refresh = app
        .application
        .refresh_session(SESSION_ID, Some(2), true)
        .await
        .unwrap();
    app.handle_session_refreshed(SESSION_ID, Ok(refresh));
    assert_parts_visible(&mut app.transcript, 4..69);
    assert!(
        app.transcript.transcript_folds.is_empty(),
        "refresh must not refold a fully loaded reply"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn multi_round_reply_expansion_keeps_its_cursor_across_run_boundaries() {
    let mut app = app(TuiBackend::remote_mock());
    let parts_for = |range: std::ops::Range<i64>| {
        range
            .filter(|id| ![8, 13].contains(id))
            .map(|id| {
                let run_id = if id < 8 {
                    3
                } else if id < 13 {
                    8
                } else {
                    13
                };
                if id == 23 {
                    parts_fixtures::text(run_id, id, "assistant", "final answer")
                } else {
                    parts_fixtures::hook(run_id, id, "assistant", &format!("Part {id}"), "detail")
                }
            })
            .collect::<Vec<_>>()
    };
    let parts = [3, 8, 13]
        .map(|id| parts_fixtures::run(id, "assistant", "completed"))
        .into_iter()
        .chain(parts_for(19..24))
        .collect::<Vec<_>>();
    let load = |version| SessionStateWithTranscriptPage {
        execution: execution(parts.clone(), version),
        page: SessionTranscriptPage {
            parts: parts.clone(),
            folds: vec![SessionTranscriptFoldResource {
                run_id: 13,
                run_ids: vec![3, 8, 13],
                anchor_part_id: 19,
                hidden_count: 13,
                next_cursor: Some("rounds-before-19".into()),
            }],
            next_cursor: None,
            has_more: false,
        },
    };
    app.handle_session_state_loaded(SESSION_ID, Ok(load(1)));
    assert!(app.transcript.merge_fold_parts(
        13,
        19,
        parts_for(16..19),
        Some("rounds-before-16".into()),
        true
    ));
    assert!(app.transcript.merge_fold_parts(
        13,
        16,
        parts_for(12..16),
        Some("rounds-before-12".into()),
        true
    ));
    app.handle_session_state_loaded(SESSION_ID, Ok(load(2)));
    let fold = &app.transcript.transcript_folds[0];
    assert_eq!(fold.run_id, 13);
    assert_eq!(fold.run_ids, vec![3, 8, 13]);
    assert_eq!(fold.anchor_part_id, 12);
    assert_eq!(fold.hidden_count, 7);
    assert_eq!(fold.next_cursor.as_deref(), Some("rounds-before-12"));
    let key = TranscriptNodeKey::Activity {
        entry_id: TranscriptEntryId::StoredMessage(3),
        content_id: TranscriptContentId::TranscriptFold {
            run_id: 13,
            anchor_part_id: 12,
        },
    };
    assert!(
        app.transcript
            .rendered(WIDTH)
            .nodes
            .iter()
            .any(|node| node.key == key && node.toggleable)
    );
    assert!(
        app.transcript
            .merge_fold_parts(13, 12, parts_for(4..12), None, false)
    );
    app.handle_session_state_loaded(SESSION_ID, Ok(load(3)));
    assert!(app.transcript.transcript_folds.is_empty());
    for part in parts_for(4..24) {
        assert_parts_visible(&mut app.transcript, part.part_id..part.part_id + 1);
    }
    assert!(
        app.transcript
            .rendered(WIDTH)
            .lines
            .iter()
            .any(|line| line.text.contains("final answer"))
    );
}

#[tokio::test]
async fn newest_reply_folds_update_after_older_history_was_loaded() {
    let mut app = app(TuiBackend::remote_mock());
    app.transcript.transcript_older_pages_loaded = true;
    app.transcript
        .set_transcript_page(Some("oldest-loaded-boundary".into()), true);
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(9, 14, 2)));
    assert_fold_visible(&mut app.transcript, 9, 5);
    assert_eq!(
        app.transcript.transcript_next_cursor.as_deref(),
        Some("oldest-loaded-boundary")
    );
}

#[tokio::test]
async fn expanded_parts_and_the_remaining_cursor_survive_a_snapshot_refresh() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    app.handle_transcript_fold_parts_loaded(
        SESSION_ID,
        3,
        12,
        false,
        Ok(SessionTranscriptPage {
            parts: activities(9..12),
            folds: vec![],
            next_cursor: Some("before-9".into()),
            has_more: true,
        }),
    );
    assert_fold_visible(&mut app.transcript, 9, 5);
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 2)));
    assert_fold_visible(&mut app.transcript, 9, 5);
    assert_parts_visible(&mut app.transcript, 9..17);
}

#[tokio::test]
async fn reconnect_keeps_the_fold_and_rejects_stale_fold_metadata() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_event_arrived(
        SESSION_ID,
        crate::LiveEvent {
            snapshot: Some(snapshot(12, 17, 3)),
            force_refresh: false,
        },
    );
    assert_fold_visible(&mut app.transcript, 12, 8);
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(9, 14, 2)));
    assert_fold_visible(&mut app.transcript, 12, 8);
    app.handle_session_event_arrived(
        99,
        crate::LiveEvent {
            snapshot: Some(snapshot(9, 14, 4)),
            force_refresh: false,
        },
    );
    assert_fold_visible(&mut app.transcript, 12, 8);
}

#[tokio::test]
async fn cached_fold_expansion_survives_reopening_the_session() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    assert!(app.transcript.merge_fold_parts(
        3,
        12,
        activities(9..12),
        Some("before-9".into()),
        true
    ));
    let cache = app.transcript.cache_snapshot();
    app.transcript.reset(99, "other session".into());
    app.transcript
        .restore_cache(cache, SESSION_ID, "Paging test".into());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 2)));
    assert_fold_visible(&mut app.transcript, 9, 5);
    assert_parts_visible(&mut app.transcript, 9..17);
    assert!(
        app.transcript
            .merge_fold_parts(3, 9, activities(4..9), None, false)
    );
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 3)));
    assert_parts_visible(&mut app.transcript, 4..17);
    assert!(app.transcript.transcript_folds.is_empty());
    assert!(
        app.transcript
            .rendered(WIDTH)
            .nodes
            .iter()
            .all(|node| !matches!(node.key, TranscriptNodeKey::ActivitySummary { .. }))
    );
}

#[tokio::test]
async fn a_small_streaming_append_preserves_the_advanced_fold_cursor() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    assert!(app.transcript.merge_fold_parts(
        3,
        12,
        activities(9..12),
        Some("before-9".into()),
        true
    ));
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(14, 19, 2)));
    assert_fold_visible(&mut app.transcript, 9, 5);
    assert_eq!(
        app.transcript.transcript_folds[0].next_cursor.as_deref(),
        Some("before-9")
    );
    assert_parts_visible(&mut app.transcript, 9..19);
}

#[tokio::test]
async fn streaming_gaps_are_loaded_before_older_parts_without_double_counting() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    assert!(app.transcript.merge_fold_parts(
        3,
        12,
        activities(9..12),
        Some("before-9".into()),
        true
    ));
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(22, 27, 2)));
    assert_fold_visible(&mut app.transcript, 22, 10);
    assert_eq!(
        app.transcript.transcript_folds[0].next_cursor.as_deref(),
        Some("before-22")
    );
    assert_parts_visible(&mut app.transcript, 9..17);
    assert_parts_visible(&mut app.transcript, 22..27);
    assert!(app.transcript.merge_fold_parts(
        3,
        22,
        activities(17..22),
        Some("before-17".into()),
        true
    ));
    assert_fold_visible(&mut app.transcript, 17, 5);
    assert!(app.transcript.merge_fold_parts(
        3,
        17,
        activities(12..17),
        Some("before-12".into()),
        true
    ));
    assert_fold_visible(&mut app.transcript, 12, 5);
    assert!(!app.transcript.merge_fold_parts(
        3,
        17,
        activities(12..17),
        Some("before-12".into()),
        true
    ));
    assert_fold_visible(&mut app.transcript, 12, 5);
    assert!(app.transcript.merge_fold_parts(
        3,
        12,
        activities(7..12),
        Some("before-7".into()),
        true
    ));
    assert_fold_visible(&mut app.transcript, 7, 3);
    assert!(
        app.transcript
            .merge_fold_parts(3, 7, activities(4..7), None, false)
    );
    assert_parts_visible(&mut app.transcript, 4..27);
    assert!(app.transcript.transcript_folds.is_empty());
}

#[tokio::test]
async fn failed_and_timed_out_fold_requests_can_be_retried() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    let fold = app.transcript.transcript_folds[0].clone();
    app.request_transcript_fold_parts(fold.clone(), false, 5);
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), app.rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        &message,
        crate::AppMessage::TranscriptFoldPartsLoaded { result: Err(_), .. }
    ));
    app.handle_message(message);
    assert!(app.transcript.transcript_fold_loads.is_empty());
    assert_fold_visible(&mut app.transcript, 12, 8);
    app.request_transcript_fold_parts(fold.clone(), false, 5);
    assert_eq!(app.transcript.transcript_fold_loads.len(), 1);
    assert!(
        app.transcript
            .recover_stalled_requests(std::time::Duration::ZERO)
    );
    assert!(app.transcript.transcript_fold_loads.is_empty());
    app.request_transcript_fold_parts(fold, false, 5);
    assert_eq!(app.transcript.transcript_fold_loads.len(), 1);
}
