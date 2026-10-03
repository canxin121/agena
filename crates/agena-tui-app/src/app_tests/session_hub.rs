use crate::app_backend::session_hub::HubCatalog;
use crate::{App, AppMessage, HubState, I18n, LaunchOptions, Route, TuiBackend, UiFailure};
use agena_api::resource::{SessionResource, WorkspaceResource};
use agena_tui_session::session_hub::{HubRow, SessionHubSectionKind};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn session(id: i64, workspace: i64, pinned: bool) -> SessionResource {
    serde_json::from_value(json!({
        "id": id, "depth": 0, "root_id": id, "workspace_id": workspace,
        "title": format!("Session {id}"), "version": 1, "pinned": pinned,
        "relation_kind": "root", "lifecycle_state": "ready",
        "created_at": "2026-10-03T00:00:00Z", "updated_at": "2026-10-03T00:00:00Z",
        "message_count": 1, "child_session_count": 0
    }))
    .unwrap()
}

fn workspace(id: i64) -> WorkspaceResource {
    serde_json::from_value(json!({ "id": id, "path": format!("/projects/{id}"),
        "created_at": "2026-10-03T00:00:00Z", "updated_at": "2026-10-03T00:00:00Z" }))
    .unwrap()
}

fn app(backend: TuiBackend) -> App {
    let mut app = App::new_with_backend(backend, LaunchOptions::default(), I18n::english());
    app.current_route = Route::Hub(HubState::new());
    app
}

fn catalog() -> HubCatalog {
    HubCatalog {
        workspaces: vec![workspace(1), workspace(2)],
        sessions: vec![session(7, 1, false), session(8, 2, true)],
        ..Default::default()
    }
}

#[tokio::test]
async fn hub_catalog_groups_all_directories_and_has_independent_pins() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_hub_catalog_loaded(0, Ok(catalog()));
    let Route::Hub(state) = &mut app.current_route else {
        panic!()
    };
    assert!(!state.loading);
    assert_eq!(
        state
            .presentation
            .rows()
            .iter()
            .filter(|row| matches!(row, HubRow::Directory { .. }))
            .count(),
        2
    );
    assert!(
        state
            .presentation
            .rows()
            .contains(&HubRow::Header(SessionHubSectionKind::Pinned))
    );
    let directory = state
        .presentation
        .rows()
        .iter()
        .position(|row| {
            matches!(
                row,
                HubRow::Directory {
                    workspace_id: 2,
                    ..
                }
            )
        })
        .unwrap();
    state.presentation.select_row(directory);
    app.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    let Route::Hub(state) = &app.current_route else {
        panic!()
    };
    assert!(state.presentation.rows().iter().any(
        |row| matches!(row, HubRow::Item(item) if item.is_new_session && item.workspace_id == 2)
    ));
    assert_eq!(
        state
            .presentation
            .rows()
            .iter()
            .filter(|row| matches!(row, HubRow::Item(item) if item.session_id == 8 && item.pinned))
            .count(),
        3
    );
}

#[tokio::test]
async fn hub_search_debounces_server_work_and_errors_preserve_existing_rows() {
    let mut app = app(TuiBackend::remote_mock());
    app.handle_hub_catalog_loaded(0, Ok(catalog()));
    app.handle_key_event(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
    for c in "Session 8".chars() {
        app.handle_key_event(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    let Route::Hub(state) = &app.current_route else {
        panic!()
    };
    assert!(state.search_changed_at.is_some());
    assert!(
        state.request_task.is_none(),
        "typing waits for the debounce tick"
    );
    let request_id = state.request_id;
    assert!(
        state
            .presentation
            .rows()
            .iter()
            .any(|row| matches!(row, HubRow::Item(item) if item.session_id == 8))
    );
    assert!(
        !state
            .presentation
            .rows()
            .iter()
            .any(|row| matches!(row, HubRow::Item(item) if item.session_id == 7))
    );
    app.handle_hub_catalog_loaded(99, Err(UiFailure::internal("stale")));
    app.handle_hub_catalog_loaded(request_id, Err(UiFailure::internal("unavailable")));
    let Route::Hub(state) = &app.current_route else {
        panic!()
    };
    assert!(!state.loading);
    assert!(state.error.as_ref().unwrap().contains("unavailable"));
    assert!(!state.presentation.rows().is_empty());
}

#[tokio::test]
async fn hub_fetches_only_requested_pages_and_creates_in_the_selected_workspace_over_http() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let backend = TuiBackend::remote_mock_at(&format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        let mut paths = Vec::new();
        for _ in 0..8 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0; 16384];
            let count = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..count]);
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            let body = if request.starts_with("PUT ") {
                assert_eq!(path, "/api/v1/sessions/8");
                assert!(request.contains("\"pinned\":true"));
                serde_json::to_value(session(8, 2, true)).unwrap()
            } else if request.starts_with("POST ") {
                assert!(request.contains("\"workspace_id\":2"));
                serde_json::to_value(session(9, 2, false)).unwrap()
            } else {
                assert!(
                    !path.contains("workspace_id="),
                    "collapsed directories must not load sessions"
                );
                assert!(path.contains("limit=20"));
                assert!(!path.contains("cursor="), "the next page is demand driven");
                let items = if path.starts_with("/api/v1/workspaces") {
                    assert!(path.contains("offset=40"));
                    json!([workspace(1)])
                } else {
                    assert!(path.contains("exclude_subagents=true"));
                    assert!(path.contains("include_total=true"));
                    assert!(path.contains("bucket="));
                    if path.contains("bucket=pinned") {
                        assert!(path.contains("offset=60"));
                        json!([session(8, 2, true)])
                    } else {
                        json!([session(7, 1, false)])
                    }
                };
                json!({"items": items, "total": 1000, "page": {"has_more": true, "next_cursor": "page-2", "returned": 1}})
            };
            let body = body.to_string();
            paths.push(path);
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        assert_eq!(
            paths
                .iter()
                .filter(|path| path.starts_with("/api/v1/sessions?"))
                .count(),
            5
        );
        assert!(!paths.iter().any(|path| path.contains("cursor=page-2")));
    });
    let catalog = backend
        .session_hub_catalog(crate::app_backend::session_hub::HubCatalogQuery {
            pages: [
                (agena_tui_session::session_hub::HubPageTarget::Workspaces, 2),
                (
                    agena_tui_session::session_hub::HubPageTarget::Section(
                        crate::SessionHubSectionKind::Pinned,
                    ),
                    3,
                ),
            ]
            .into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(catalog.workspaces.len(), 1);
    assert_eq!(catalog.sessions.len(), 2);
    let mut app = app(backend);
    app.handle_hub_catalog_loaded(0, Ok(catalog));
    let Route::Hub(state) = &mut app.current_route else {
        panic!()
    };
    let pin = state
        .presentation
        .rows()
        .iter()
        .position(|row| matches!(row, HubRow::Item(item) if item.session_id == 8))
        .unwrap();
    state.presentation.select_row(pin);
    // Authoritative updates synchronize duplicate rows and keep pin/favorite independent.
    app.application.set_session_pinned(8, true).await.unwrap();
    app.handle_message(AppMessage::SessionPinnedUpdated {
        session_id: 8,
        result: Ok(session(8, 2, true)),
    });
    app.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    let message = tokio::time::timeout(std::time::Duration::from_secs(3), app.rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(message, AppMessage::SessionCreated { result: Ok(ref item), .. } if item.workspace_id == 2)
    );
    server.await.unwrap();
}
