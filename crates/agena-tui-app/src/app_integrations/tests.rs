use super::*;
fn provider() -> IntegrationProvider {
    serde_json::from_value(serde_json::json!({
        "plugin_id": "fixture.events", "fingerprint": "fixture",
        "adapter": {
            "kind": "event_source", "title": "Fixture events", "description": "test",
            "config_schema": {"type": "object", "properties": {}},
            "minimum_poll_seconds": 60, "idempotent_send": true
        }
    }))
    .unwrap()
}

#[test]
fn session_draft_and_event_details_use_native_execution_contract() {
    let mut spec = template(&provider());
    spec.mode = IntegrationMode::Session;
    spec.use_source_permissions = true;
    spec.validate(chrono::Utc::now().timestamp_millis())
        .unwrap();
    let json = serde_json::to_value(&spec).unwrap();
    assert_eq!(json["mode"], "session");
    assert_eq!(json["max_execution_seconds"], 300);
    assert!(json.get("max_task_seconds").is_none());
    let event: IntegrationEvent = serde_json::from_value(serde_json::json!({
        "binding_id": Uuid::new_v4(), "epoch": 1,
        "event": {"id": "event", "event_type": "issue.opened", "resource": "exact-resource",
            "actor": "fixture", "occurred_at_ms": 1, "text": "data"},
        "state": "completed", "received_at_ms": 2,
        "session_id": 77, "execution_generation": 8, "execution_id": "native-execution"
    }))
    .unwrap();
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["session_id"], 77);
    assert_eq!(json["execution_generation"], 8);
    assert_eq!(json["execution_id"], "native-execution");
    assert!(json.get("task_id").is_none());
    let mut without_execution = json;
    without_execution["session_id"] = serde_json::Value::Null;
    without_execution["execution_generation"] = serde_json::Value::Null;
    let event: IntegrationEvent = serde_json::from_value(without_execution).unwrap();
    assert_eq!(event.session_id, None);
    assert_eq!(event.execution_generation, None);
}

#[test]
fn channel_draft_keeps_explicit_privileges_without_background_work() {
    let mut provider = provider();
    provider.adapter.kind = AdapterKind::Channel;
    let json = serde_json::to_value(template(&provider)).unwrap();
    let policy = &json["channel_policy"];
    assert!(policy["grant"].is_object());
    assert!(policy["service_grant"].is_object());
    assert!(
        !serde_json::to_string(policy)
            .unwrap()
            .contains("background_work")
    );
    assert_eq!(policy["members"][0]["role"], "owner");
}

#[tokio::test]
async fn enter_opens_the_normal_binding_session_and_restores_its_draft() {
    let root = tempfile::tempdir().unwrap();
    let backend = crate::TuiBackend::isolated_test_backend(root.path().into());
    let mut app = App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
    app.draft_store_path = root.path().join("drafts.json");
    app.prompt_history_path = root.path().join("history.jsonl");
    app.open_session(77, "Original".into());
    app.composer.insert_str("keep normal session draft");
    let binding = IntegrationBinding::new(
        Some(1),
        None,
        77,
        "fixture".into(),
        template(&provider()),
        "fixture".into(),
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap();
    let mut state = IntegrationState::new(77);
    state.detail = Some(IntegrationDetail {
        binding,
        events: vec![],
        deliveries: vec![],
        channel_sessions: vec![],
        access_audit: vec![],
    });
    app.integration_panels.insert(77, state);
    app.current_route = Route::Integrations(77);
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.current_route_is_main());
    assert_eq!(app.transcript.session_id, Some(77));
    assert_eq!(app.composer.text(), "keep normal session draft");
    let detail = app
        .integration_panels
        .get_mut(&77)
        .unwrap()
        .detail
        .as_mut()
        .unwrap();
    for (received, session) in [(2, 88), (1, 99)] {
        detail.events.push(serde_json::from_value(serde_json::json!({
            "binding_id": detail.binding.id, "epoch": 1,
            "event": {"id": format!("event-{received}"), "event_type": "issue.opened",
                "resource": "exact-resource", "actor": "fixture", "occurred_at_ms": received, "text": "data"},
            "state": "completed", "received_at_ms": received,
            "session_id": session, "execution_generation": 8, "execution_id": "native-execution"
        })).unwrap());
    }
    app.current_route = Route::Integrations(77);
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.current_route_is_main());
    assert_eq!(
        app.transcript.session_id,
        Some(88),
        "open newest linked normal session"
    );
    app.open_session(77, "Original".into());
    assert_eq!(app.composer.text(), "keep normal session draft");
}

#[test]
fn panel_renders_with_no_provider_on_narrow_terminal() {
    for (w, h) in [(100, 30), (42, 18)] {
        let mut t = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        t.draw(|f| render(f, f.area(), &IntegrationState::new(1), &I18n::english()))
            .unwrap();
    }
}
