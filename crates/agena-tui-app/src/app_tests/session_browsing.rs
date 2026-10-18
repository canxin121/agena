use crate::{
    App, AppMessage, I18n, LaunchOptions, Route, SessionResource, SessionSearchOverlay, TuiBackend,
    UiFailure,
};
use agena_tui_components::{Editor, SearchPickerFocus};
use agena_tui_session::{session_hub::HubPageTarget, session_view::SessionViewMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn session(id: i64) -> SessionResource {
    serde_json::from_value(json!({
        "id": id, "depth": 0, "root_id": id, "workspace_id": 1,
        "title": format!("Session {id}"), "version": 1, "pinned": false, "favorite": false,
        "relation_kind": "root", "lifecycle_state": "ready",
        "created_at": "2026-10-18T00:00:00Z", "updated_at": "2026-10-18T00:00:00Z",
        "message_count": 1, "child_session_count": 0
    }))
    .unwrap()
}

fn app(backend: TuiBackend, count: i64) -> App {
    let mut app = App::new_with_backend(backend, LaunchOptions::default(), I18n::english());
    let mut dialog = app.build_session_search_overlay(
        Editor::from_text(String::new()),
        SessionViewMode::All,
        None,
    );
    dialog.replace_items(
        (101..101 + count)
            .map(|id| app.session_search_item(session(id)))
            .collect(),
    );
    dialog.set_loading(false);
    dialog.set_visible_page_size(5);
    app.current_route = Route::SessionSearch(dialog);
    app
}

fn dialog(app: &App) -> &SessionSearchOverlay {
    let Route::SessionSearch(dialog) = &app.current_route else {
        panic!("expected session search");
    };
    dialog
}

fn dialog_mut(app: &mut App) -> &mut SessionSearchOverlay {
    let Route::SessionSearch(dialog) = &mut app.current_route else {
        panic!("expected session search");
    };
    dialog
}

fn key(app: &mut App, code: KeyCode) {
    app.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE));
}

#[tokio::test]
async fn session_browser_keyboard_pages_and_search_cursor_do_not_conflict() {
    let mut app = app(TuiBackend::remote_mock(), 20);
    key(&mut app, KeyCode::Right);
    assert_eq!(dialog(&app).selected_item().unwrap().session_id, 106);
    key(&mut app, KeyCode::PageDown);
    assert_eq!(dialog(&app).selected_item().unwrap().session_id, 111);
    key(&mut app, KeyCode::Left);
    assert_eq!(dialog(&app).selected_item().unwrap().session_id, 106);
    key(&mut app, KeyCode::End);
    assert_eq!(dialog(&app).selected_item().unwrap().session_id, 120);
    let extra = app.session_search_item(session(121));
    dialog_mut(&mut app).append_items(vec![extra]);
    assert_eq!(
        dialog(&app).selected_item().unwrap().session_id,
        120,
        "End must update the stable selection identity before appending"
    );
    key(&mut app, KeyCode::Char('/'));
    dialog_mut(&mut app).input = Editor::from_text("abc".into());
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Char('p'));
    assert_eq!(dialog(&app).input.text(), "abpc");
    key(&mut app, KeyCode::Char('f'));
    assert_eq!(dialog(&app).input.text(), "abpfc");
    key(&mut app, KeyCode::Esc);
    assert_eq!(dialog(&app).focus, SearchPickerFocus::Results);
    assert_eq!(dialog(&app).input.text(), "abpfc");
}

#[tokio::test]
async fn session_browser_append_moves_to_requested_page_once_and_keeps_manual_navigation() {
    let mut app = app(TuiBackend::remote_mock(), 7);
    dialog_mut(&mut app)
        .meta
        .apply_page(Some("next".into()), true);
    dialog_mut(&mut app).move_selection(5);
    key(&mut app, KeyCode::Right);
    let request_id = dialog(&app).meta.request_id;
    key(&mut app, KeyCode::Right); // repeated key while loading must not cancel the pending move
    assert_eq!(dialog(&app).meta.request_id, request_id);
    app.handle_session_search_page_loaded(request_id, SessionViewMode::All, String::new(), 1,
        Ok(serde_json::from_value(json!({"items": (108..=120).map(session).collect::<Vec<_>>(), "page": {"returned": 13, "has_more": true, "next_cursor": "next-2"}})).unwrap()));
    assert_eq!(dialog(&app).current_page(), 2);
    assert_eq!(dialog(&app).selected, 10);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Right);
    let request_id = dialog(&app).meta.request_id;
    key(&mut app, KeyCode::Left); // navigate elsewhere before the response
    let selected = dialog(&app).selected_item().unwrap().session_id;
    app.handle_session_search_page_loaded(
        request_id,
        SessionViewMode::All,
        String::new(),
        2,
        Ok(serde_json::from_value(
            json!({"items": [session(121)], "page": {"returned": 1, "has_more": false}}),
        )
        .unwrap()),
    );
    assert_eq!(dialog(&app).selected_item().unwrap().session_id, selected);
}

#[tokio::test]
async fn session_browser_stale_results_errors_and_paste_keep_request_identity() {
    let mut app = app(TuiBackend::remote_mock(), 7);
    dialog_mut(&mut app)
        .meta
        .apply_page(Some("next".into()), true);
    dialog_mut(&mut app).move_selection_end();
    key(&mut app, KeyCode::Right);
    let request_id = dialog(&app).meta.request_id;
    app.handle_session_search_page_loaded(
        request_id,
        SessionViewMode::All,
        String::new(),
        1,
        Err(UiFailure::internal("offline")),
    );
    assert_eq!(dialog(&app).meta.page_index, 0);
    assert_eq!(dialog(&app).items.len(), 7);
    assert!(
        dialog(&app)
            .error_message
            .as_deref()
            .unwrap()
            .contains("offline")
    );
    app.handle_paste("new query".into());
    let next_request = dialog(&app).meta.request_id;
    assert!(next_request > request_id);
    assert_eq!(dialog(&app).focus, SearchPickerFocus::Input);
    assert!(dialog(&app).items.is_empty());
    // Even the same query/page cannot allow an older dialog/request to win.
    app.handle_session_search_page_loaded(
        request_id,
        SessionViewMode::All,
        "new query".into(),
        0,
        Ok(serde_json::from_value(
            json!({"items": [session(999)], "page": {"returned": 1, "has_more": false}}),
        )
        .unwrap()),
    );
    assert!(dialog(&app).items.is_empty());
    assert!(dialog(&app).is_loading());
}

#[tokio::test]
async fn session_browser_marks_use_selected_id_and_shared_persistent_api() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let backend = TuiBackend::remote_mock_at(&format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        for field in ["pinned", "favorite"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0; 16384];
            let size = socket.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..size]);
            assert!(request.starts_with("PUT /api/v1/sessions/108 "));
            assert!(request.contains(&format!("\"{field}\":true")));
            let mut updated = session(108);
            updated.pinned = true;
            updated.favorite = field == "favorite";
            let body = serde_json::to_string(&updated).unwrap();
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let mut app = app(backend, 20);
    dialog_mut(&mut app).move_selection(2);
    key(&mut app, KeyCode::Right);
    for mark in ['p', 'f'] {
        if mark == 'p' {
            use agena_tui_components::pointer::{self, PointerAction};
            use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
            let mut map = Default::default();
            terminal
                .draw(|frame| {
                    map = pointer::capture(|| {
                        agena_tui_session::session_search::render_overlay(
                            frame,
                            frame.area(),
                            dialog(&app),
                            &app.i18n,
                        )
                    })
                })
                .unwrap();
            let button = (0..24).flat_map(|y| (0..80).map(move |x| MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row: y, modifiers: KeyModifiers::NONE }))
                .find(|event| matches!(map.action(*event), Some(PointerAction::Key(key)) if key.code == KeyCode::Char('p') && key.modifiers == KeyModifiers::ALT)).unwrap();
            app.pointer_targets = map;
            assert!(app.handle_surface_pointer(button));
        } else {
            key(&mut app, KeyCode::Char(mark));
        }
        let message = tokio::time::timeout(std::time::Duration::from_secs(3), app.rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            message,
            AppMessage::SessionPinnedUpdated {
                session_id: 108,
                result: Ok(_)
            } | AppMessage::SessionFavoriteUpdated {
                session_id: 108,
                result: Ok(_)
            }
        ));
        app.handle_message(message);
    }
    let selected = dialog(&app).selected_item().unwrap();
    assert!(selected.pinned && selected.favorite);
    assert_eq!(selected.session_id, 108);
    assert_eq!(dialog(&app).selected, 7);
    server.await.unwrap();
}

#[tokio::test]
async fn session_browser_hub_arrow_pages_selected_bucket_and_lands_on_new_rows() {
    use crate::app_backend::session_hub::HubCatalog;
    use agena_tui_session::session_hub::{HubPagination, HubRow, SessionHubSectionKind};
    let mut app = app(TuiBackend::remote_mock(), 1);
    app.current_route = Route::Hub(crate::HubState::new());
    let target = HubPageTarget::Section(SessionHubSectionKind::Recent);
    let catalog = |id, page| HubCatalog {
        sessions: vec![session(id)],
        pages: [(
            target,
            HubPagination {
                page,
                has_more: true,
                total: Some(90),
            },
        )]
        .into(),
        members: [(target, [id].into())].into(),
        ..Default::default()
    };
    app.handle_hub_catalog_loaded(0, Ok(catalog(101, 0)));
    let Route::Hub(state) = &mut app.current_route else {
        panic!();
    };
    let index = state
        .presentation
        .rows()
        .iter()
        .position(|row| matches!(row, HubRow::Item(item) if item.session_id == 101))
        .unwrap();
    state.presentation.select_row(index);
    key(&mut app, KeyCode::Right);
    let Route::Hub(state) = &app.current_route else {
        panic!();
    };
    assert_eq!(state.pages[&target], 1);
    let request = state.request_id;
    app.handle_hub_catalog_loaded(request, Ok(catalog(121, 1)));
    let Route::Hub(state) = &app.current_route else {
        panic!();
    };
    assert_eq!(state.presentation.selected_item().unwrap().session_id, 121);
    assert_eq!(state.presentation.selected_page_target(), target);
    key(&mut app, KeyCode::Left);
    let Route::Hub(state) = &app.current_route else {
        panic!();
    };
    assert_eq!(state.pages[&target], 0);
}
