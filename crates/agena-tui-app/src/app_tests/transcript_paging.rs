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
    let failed = transcript.transcript_fold_errors.contains_key(&fold.run_id);
    let rendered = transcript.rendered(WIDTH);
    let node = rendered
        .nodes
        .iter()
        .find(|node| node.key == key)
        .expect("load-more control must render");
    assert!(node.toggleable);
    assert!(rendered.lines[node.start_line].text.contains(if failed {
        "Enter to retry"
    } else {
        "Enter: show 5"
    }));
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
                assert_eq!(path, "/api/v1/sessions/7/transcript?limit=2");
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
            part_update: None,
            session_deleted: false,
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
            part_update: None,
            session_deleted: false,
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

#[tokio::test]
async fn history_prefetch_requires_upward_intent_and_preserves_retry_cursor() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    app.focus = agena_tui::main_focus::Focus::Transcript;
    app.transcript.scroll_to_top(WIDTH, HEIGHT);
    app.handle_key_event(KeyEvent::new(KeyCode::Char('0'), KeyModifiers::NONE));
    assert!(
        !app.transcript.transcript_older_loading,
        "ordinary keys must not page history"
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert!(app.transcript.transcript_older_loading);
    app.handle_transcript_parts_loaded(
        SESSION_ID,
        Ok(SessionTranscriptPage {
            parts: vec![],
            folds: vec![],
            next_cursor: Some("older-messages".into()),
            has_more: true,
        }),
    );
    assert!(app.transcript.transcript_older_error.is_some());
    assert!(app.transcript.transcript_has_more);
    assert!(!app.transcript.transcript_older_loading);
    app.handle_transcript_parts_loaded(
        SESSION_ID,
        Ok(SessionTranscriptPage {
            parts: vec![],
            folds: vec![],
            next_cursor: Some("older-next".into()),
            has_more: true,
        }),
    );
    assert_eq!(
        app.transcript.transcript_next_cursor.as_deref(),
        Some("older-next")
    );
}

#[tokio::test]
async fn a_streaming_gap_during_a_fold_request_keeps_new_cursor_and_old_parts() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(14, 19, 1)));
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(24, 29, 2)));
    assert!(app.transcript.merge_fold_parts(
        3,
        14,
        activities(9..14),
        Some("before-9".into()),
        true
    ));
    assert_eq!(
        app.transcript.transcript_folds[0].next_cursor.as_deref(),
        Some("before-24")
    );
    assert!(app.transcript.parts.iter().any(|part| part.part_id == 9));
}

#[tokio::test]
async fn normal_live_events_schedule_a_coalesced_refresh() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_event_arrived(
        SESSION_ID,
        crate::LiveEvent {
            part_update: None,
            session_deleted: false,
            snapshot: None,
            force_refresh: false,
        },
    );
    assert!(app.pending_refresh.is_some());
}

#[tokio::test]
async fn completed_refreshes_wait_for_the_tick_gate_and_failures_back_off() {
    use std::time::{Duration, Instant};
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    app.transcript.refreshing = true;
    app.pending_refresh_for(SESSION_ID);
    app.last_refresh_at = Instant::now();
    app.handle_session_refreshed(
        SESSION_ID,
        Ok(crate::app_backend::SessionRefresh {
            reconciled_parts: None,
            snapshot: None,
            latest_event_seq: Some(1),
            event_count: 0,
        }),
    );
    assert!(
        !app.transcript.refreshing,
        "completion must not chain another request"
    );
    assert!(app.pending_refresh.is_some());
    app.on_tick();
    assert!(!app.transcript.refreshing);
    app.last_refresh_at = Instant::now() - Duration::from_millis(300);
    app.on_tick();
    assert!(app.transcript.refreshing);
    assert!(app.pending_refresh.is_none());

    app.handle_session_refreshed(SESSION_ID, Err(crate::UiFailure::internal("offline")));
    app.pending_refresh_for(SESSION_ID);
    app.last_refresh_at = Instant::now() - Duration::from_millis(300);
    app.on_tick();
    assert!(
        !app.transcript.refreshing,
        "failed requests must back off past the streaming interval"
    );
    app.last_refresh_at = Instant::now() - Duration::from_millis(600);
    app.on_tick();
    assert!(app.transcript.refreshing);
}

#[tokio::test]
async fn fold_loading_stops_a_multi_page_cursor_cycle_and_keeps_the_retry() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    app.handle_transcript_fold_parts_loaded(
        SESSION_ID,
        3,
        12,
        true,
        Ok(SessionTranscriptPage {
            parts: activities(10..12),
            folds: vec![],
            next_cursor: Some("before-10".into()),
            has_more: true,
        }),
    );
    assert!(app.transcript.transcript_fold_errors.is_empty());
    let anchor = app.transcript.transcript_folds[0].anchor_part_id;
    app.handle_transcript_fold_parts_loaded(
        SESSION_ID,
        3,
        anchor,
        true,
        Ok(SessionTranscriptPage {
            parts: activities(8..10),
            folds: vec![],
            next_cursor: Some("before-12".into()),
            has_more: true,
        }),
    );
    assert!(app.transcript.transcript_fold_loads.is_empty());
    assert!(app.transcript.transcript_fold_errors.contains_key(&3));
    assert_eq!(
        app.transcript.transcript_folds[0].next_cursor.as_deref(),
        Some("before-10")
    );
    let fold = app.transcript.transcript_folds[0].clone();
    app.request_transcript_fold_parts(fold, false, 5);
    assert_eq!(app.transcript.transcript_fold_loads.len(), 1);
}

#[tokio::test]
async fn old_page_responses_cannot_finish_a_new_request_after_reopening_the_same_session() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_session_state_loaded(SESSION_ID, Ok(snapshot(12, 17, 1)));
    let old = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let current = std::time::Instant::now();
    app.transcript.transcript_older_loading = true;
    app.transcript.transcript_older_in_flight_since = Some(current);
    app.transcript
        .transcript_fold_loads
        .insert((3, 12), current);
    let page = || SessionTranscriptPage {
        parts: activities(4..12),
        folds: vec![],
        next_cursor: None,
        has_more: false,
    };
    app.handle_message(crate::AppMessage::TranscriptPartsLoaded {
        session_id: SESSION_ID,
        requested_at: old,
        result: Ok(page()),
    });
    app.handle_message(crate::AppMessage::TranscriptFoldPartsLoaded {
        session_id: SESSION_ID,
        requested_at: old,
        run_id: 3,
        anchor_part_id: 12,
        expand_all: false,
        result: Ok(page()),
    });
    assert!(app.transcript.transcript_older_loading);
    assert_eq!(
        app.transcript.transcript_fold_loads.get(&(3, 12)),
        Some(&current)
    );
    assert_eq!(app.transcript.transcript_folds[0].hidden_count, 8);
    assert!(!app.transcript.parts.iter().any(|part| part.part_id == 4));
}

#[tokio::test]
async fn prepend_includes_fold_controls_when_preserving_the_reading_position() {
    let mut app = app(TuiBackend::remote_mock());
    let initial = vec![
        parts_fixtures::run(100, "user", "completed"),
        parts_fixtures::text(
            100,
            101,
            "user",
            &(0..50)
                .map(|i| format!("Reading line {i}\n"))
                .collect::<String>(),
        ),
    ];
    app.transcript.merge_parts(initial);
    app.transcript.set_cursor_line(WIDTH, 8, 10);
    app.transcript.ensure_visual_focus(WIDTH, 8);
    let old_top = app.transcript.viewport_top();
    let reading_line = app.transcript.rendered(WIDTH).lines[old_top].text.clone();
    let older = std::iter::once(parts_fixtures::run(3, "assistant", "completed"))
        .chain(activities(90..95))
        .collect();
    app.transcript.prepend_transcript_parts(
        older,
        vec![SessionTranscriptFoldResource {
            run_id: 3,
            run_ids: vec![3],
            anchor_part_id: 90,
            hidden_count: 50,
            next_cursor: Some("before-90".into()),
        }],
        WIDTH,
        8,
    );
    app.transcript.ensure_visual_focus(WIDTH, 8);
    let top = app.transcript.viewport_top();
    assert_eq!(app.transcript.rendered(WIDTH).lines[top].text, reading_line);
}

#[tokio::test]
async fn obsolete_state_refresh_and_subscription_tokens_cannot_change_a_reopened_session() {
    let mut app = app(TuiBackend::remote_mock());
    assert!(app.apply_transcript_snapshot(snapshot(10, 15, 8)));
    let old = std::time::Instant::now();
    let current = old + std::time::Duration::from_millis(1);
    app.transcript.state_loading = true;
    app.transcript.state_load_in_flight_since = Some(current);
    app.transcript.refreshing = true;
    app.transcript.refresh_in_flight_since = Some(current);
    app.handle_message(crate::AppMessage::SessionStateLoaded {
        session_id: SESSION_ID,
        requested_at: old,
        result: Ok(snapshot(10, 12, 2)),
    });
    app.handle_message(crate::AppMessage::SessionRefreshed {
        session_id: SESSION_ID,
        requested_at: old,
        result: Ok(crate::app_backend::SessionRefresh {
            snapshot: Some(snapshot(10, 12, 2)),
            reconciled_parts: None,
            latest_event_seq: Some(2),
            event_count: 0,
        }),
    });
    assert_eq!(app.transcript.state_load_in_flight_since, Some(current));
    assert_eq!(app.transcript.refresh_in_flight_since, Some(current));
    let scope = crate::SessionLoadScope {
        mode: agena_tui_session::session_view::SessionViewMode::All,
        anchor_session_id: None,
    };
    app.session_load.loading = true;
    app.session_load.pending_scope = Some(scope.clone());
    app.session_load.requested_at = Some(current);
    app.handle_message(crate::AppMessage::SessionsLoaded {
        requested_at: old,
        scope,
        subtree_root_id: None,
        result: Ok(Vec::new()),
    });
    assert_eq!(app.session_load.requested_at, Some(current));
    assert!(app.session_load.loading);
    app.request_sessions(false);
    assert!(
        app.session_load.refresh_queued,
        "invalidation during a list read must survive coalescing"
    );
    app.subscription_generation = 2;
    app.handle_message(crate::AppMessage::SessionEventArrived {
        session_id: SESSION_ID,
        generation: 1,
        live: crate::LiveEvent {
            part_update: None,
            session_deleted: true,
            snapshot: None,
            force_refresh: false,
        },
    });
    assert_eq!(app.transcript.session_id, Some(SESSION_ID));
    app.handle_session_refreshed(
        SESSION_ID,
        Ok(crate::app_backend::SessionRefresh {
            snapshot: Some(snapshot(10, 12, 2)),
            reconciled_parts: None,
            latest_event_seq: Some(2),
            event_count: 0,
        }),
    );
    assert_eq!(app.transcript.last_event_seq, Some(8));
    let mut other = execution(Vec::new(), 100);
    other.session.id = 99;
    assert!(!app.apply_transcript_execution(other));
    assert_eq!(
        app.transcript.execution.as_ref().unwrap().session.id,
        SESSION_ID
    );
}

#[tokio::test]
async fn shared_streaming_part_updates_are_ordered_without_snapshot_polls() {
    let mut app = app(TuiBackend::remote_mock());
    let mut text = parts_fixtures::text(3, 4, "assistant", "latest");
    text.revision = 4;
    text.updated_at_ms = 10;
    app.transcript.merge_parts(vec![
        parts_fixtures::run(3, "assistant", "in_progress"),
        text.clone(),
    ]);
    let patch = |part| crate::LiveEvent {
        part_update: Some((99, part)),
        session_deleted: false,
        snapshot: None,
        force_refresh: false,
    };
    let mut old = text.clone();
    old.updated_at_ms = 9;
    old.content = json!({"text": "stale"});
    app.handle_session_event_arrived(SESSION_ID, patch(old));
    assert_eq!(app.transcript.parts[1].content["text"], "latest");
    text.updated_at_ms = 11;
    text.content = json!({"text": ""});
    app.handle_session_event_arrived(SESSION_ID, patch(text));
    assert_eq!(app.transcript.parts[1].content["text"], "");
    assert!(
        app.pending_refresh.is_none(),
        "known text must not refetch state per checkpoint"
    );
}

#[tokio::test]
async fn reconnect_removes_missed_memberships_without_resurrecting_them_from_old_pages() {
    let mut app = app(TuiBackend::remote_mock());
    app.apply_transcript_snapshot(snapshot(10, 15, 8));
    let removed = app
        .transcript
        .parts
        .iter()
        .find(|part| part.part_id == 12)
        .unwrap()
        .clone();
    let current = app
        .transcript
        .parts
        .iter()
        .filter(|part| part.part_id != 12)
        .cloned()
        .collect();
    app.handle_session_refreshed(
        SESSION_ID,
        Ok(crate::app_backend::SessionRefresh {
            snapshot: None,
            reconciled_parts: Some((vec![12, 13], current)),
            latest_event_seq: Some(8),
            event_count: 0,
        }),
    );
    assert!(!app.transcript.parts.iter().any(|part| part.part_id == 12));
    let mut old_page = app.transcript.parts.clone();
    old_page.push(removed);
    app.transcript.merge_parts(old_page);
    assert!(!app.transcript.parts.iter().any(|part| part.part_id == 12));
}
