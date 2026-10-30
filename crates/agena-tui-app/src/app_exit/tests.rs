use super::*;
use crate::{ComposerDraft, DraftSlot, Route, RunOperation};
use agena_api::resource::{ActiveExecutionResource, SessionExecutionResource, SessionState};
use crossterm::event::KeyEventKind;

fn snapshot(state: SessionState) -> SessionExecutionResource {
    serde_json::from_value(serde_json::json!({
        "session": {
            "id": 17, "parent_id": null, "root_id": 17, "depth": 0,
            "workspace_id": 1, "title": "Exit test", "version": 1,
            "relation_kind": "root", "lifecycle_state": "ready", "state": state,
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "message_count": 0, "child_session_count": 0
        },
        "parts": [], "part_page": null,
        "execution": {
            "agent_id": "test", "selected_permission": {},
            "effective_permission": {}, "permission_ceiling": {}
        },
        "usage": { "current_tokens": 0 }
    }))
    .unwrap()
}

fn running(execution_id: uuid::Uuid, phase: ExecutionPhase) -> SessionState {
    SessionState::Running {
        execution: Some(ActiveExecutionResource {
            execution_id,
            phase,
        }),
        workflow: agena_domain::WorkflowState::Quiescent,
        requests: Vec::new(),
    }
}

fn app(state: Option<SessionState>) -> (tempfile::TempDir, App) {
    let temp = tempfile::tempdir().unwrap();
    let backend = crate::TuiBackend::isolated_test_backend(temp.path().to_path_buf());
    let mut app = App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
    app.draft_store_path = temp.path().join("tui-drafts.json");
    app.prompt_history_path = temp.path().join("tui-prompt-history.jsonl");
    app.current_route = Route::Main;
    app.focus = Focus::Composer;
    if let Some(state) = state {
        app.transcript.session_id = Some(17);
        app.transcript.execution = Some(snapshot(state));
    }
    (temp, app)
}

fn ctrl_c() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
}

fn selection(value: &str) -> ChoiceSelection {
    ChoiceSelection::Item {
        value: value.into(),
    }
}

fn menu_values(app: &App) -> Vec<&'static str> {
    let Some(Overlay::Choice(dialog)) = &app.overlay else {
        panic!("expected an exit menu");
    };
    assert!(matches!(dialog.action, ChoiceOverlayAction::Exit));
    let mut picker = dialog.presentation.clone();
    ["detach", "stop-and-exit", "cancel"]
        .into_iter()
        .filter(|value| picker.select_item_where(|item| item.value == *value))
        .collect()
}

fn select(app: &mut App, value: &str) {
    let Some(Overlay::Choice(dialog)) = &mut app.overlay else {
        panic!("expected an exit menu");
    };
    assert!(
        dialog
            .presentation
            .select_item_where(|item| item.value == value)
    );
}

fn selected(app: &App) -> Option<ChoiceSelection> {
    let Some(Overlay::Choice(dialog)) = &app.overlay else {
        panic!("expected an exit menu");
    };
    choice::selection(&dialog.presentation)
}

fn accept(app: &mut App, value: &str) {
    select(app, value);
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

fn draft(text: &str) -> ComposerDraft {
    ComposerDraft {
        document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
            text: text.into(),
        }]),
    }
}

#[tokio::test]
async fn idle_and_non_chat_routes_exit_without_stopping_a_cached_session() {
    for state in [
        None,
        Some(SessionState::default()),
        Some(SessionState::Failed { failure: None }),
    ] {
        let (_temp, mut app) = app(state);
        app.handle_key_event(ctrl_c());
        assert!(app.should_quit);
        assert!(app.overlay.is_none());
        assert!(app.exit_cancellation.is_none());
    }
    for route in ["hub", "files"] {
        let (_temp, mut app) = app(Some(running(
            uuid::Uuid::new_v4(),
            ExecutionPhase::StreamingModel,
        )));
        match route {
            "hub" => app.open_hub(),
            "files" => app.current_route = Route::SessionWork(17),
            _ => unreachable!(),
        }
        assert!(!app.current_route_is_main(), "fixture must enter {route}");
        app.handle_key_event(ctrl_c());
        assert!(app.should_quit, "{route}");
        assert!(app.overlay.is_none(), "{route}");
        assert!(app.exit_cancellation.is_none(), "{route}");
        assert!(
            app.transcript
                .execution
                .as_ref()
                .unwrap()
                .session
                .state
                .active_execution()
                .is_some()
        );
    }
}

#[tokio::test]
async fn stop_exists_only_for_the_current_sessions_confirmed_active_execution() {
    for phase in [
        ExecutionPhase::Starting,
        ExecutionPhase::PreparingModel,
        ExecutionPhase::StreamingModel,
        ExecutionPhase::ExecutingTools,
        ExecutionPhase::AwaitingInteraction,
        ExecutionPhase::Cancelling,
    ] {
        for awaiting in [false, true] {
            let id = uuid::Uuid::new_v4();
            let state = if awaiting {
                SessionState::AwaitingInteraction {
                    run_id: Some(99),
                    execution: Some(ActiveExecutionResource {
                        execution_id: id,
                        phase,
                    }),
                    requests: Vec::new(),
                }
            } else {
                running(id, phase)
            };
            let (_temp, mut app) = app(Some(state));
            // A receipt identifies accepted input, which may be queued behind
            // the active execution; it must never become the stop target.
            let queued = uuid::Uuid::new_v4();
            let snapshot = app.transcript.execution.as_mut().unwrap();
            snapshot.receipt = Some(agena_api::resource::SessionExecutionReceiptResource {
                execution_id: queued,
                turn_id: uuid::Uuid::new_v4(),
                reply_id: uuid::Uuid::new_v4(),
            });
            snapshot
                .queued_messages
                .push(agena_api::resource::QueuedMessageResource {
                    execution_id: queued,
                    message_id: 10,
                });
            app.handle_key_event(ctrl_c());
            let can_stop = phase != ExecutionPhase::Cancelling;
            assert_eq!(
                menu_values(&app).contains(&"stop-and-exit"),
                can_stop,
                "{phase:?}/{awaiting}"
            );
            assert_eq!(
                app.exit_menu_target.map(|target| target.execution_id),
                can_stop.then_some(id.into())
            );
            assert!(!app.should_quit);
            assert!(app.exit_cancellation.is_none());
        }
    }
    for state in [
        SessionState::Creating,
        SessionState::Running {
            execution: None,
            workflow: agena_domain::WorkflowState::Quiescent,
            requests: Vec::new(),
        },
        SessionState::AwaitingInteraction {
            run_id: Some(99),
            execution: None,
            requests: Vec::new(),
        },
    ] {
        let (_temp, mut app) = app(Some(state));
        app.handle_key_event(ctrl_c());
        assert_eq!(menu_values(&app), ["detach", "cancel"]);
    }
    for scenario in [
        "pending send",
        "pending creation",
        "queue only",
        "unloaded",
        "wrong session",
    ] {
        let (_temp, mut app) = app(Some(SessionState::default()));
        match scenario {
            "pending send" => app
                .run_activity
                .begin(RunActivityTarget::Session(17), RunOperation::SubmitMessage),
            "pending creation" => {
                app.transcript.session_id = None;
                app.transcript.execution = None;
                app.run_activity
                    .begin(RunActivityTarget::NewSession, RunOperation::CreateSession);
            }
            "queue only" => app
                .transcript
                .execution
                .as_mut()
                .unwrap()
                .queued_messages
                .push(agena_api::resource::QueuedMessageResource {
                    execution_id: uuid::Uuid::new_v4(),
                    message_id: 10,
                }),
            "unloaded" => app.transcript.execution = None,
            "wrong session" => {
                app.transcript.execution = Some(snapshot(running(
                    uuid::Uuid::new_v4(),
                    ExecutionPhase::StreamingModel,
                )));
                app.transcript.session_id = Some(18);
            }
            _ => unreachable!(),
        }
        app.handle_key_event(ctrl_c());
        assert_eq!(menu_values(&app), ["detach", "cancel"], "{scenario}");
        assert!(app.exit_cancellation.is_none());
    }
}

#[tokio::test]
async fn composer_clears_once_but_repeat_release_and_other_modifiers_do_not_exit() {
    for text in ["unfinished draft", " \n "] {
        let (_temp, mut app) = app(Some(SessionState::default()));
        app.composer.set_text(text.to_owned());
        let mut repeated = ctrl_c();
        repeated.kind = KeyEventKind::Repeat;
        app.handle_key_event(repeated);
        assert_eq!(app.composer.text(), text);
        let mut release = ctrl_c();
        release.kind = KeyEventKind::Release;
        app.handle_key_event(release);
        assert_eq!(app.composer.text(), text);
        app.handle_key_event(ctrl_c());
        assert!(app.composer.text().is_empty());
        assert!(app.overlay.is_none());
        assert!(!app.should_quit);
        app.handle_key_event(repeated);
        assert!(!app.should_quit);
        app.handle_key_event(ctrl_c());
        assert!(app.should_quit);
    }
    let (_temp, mut app) = app(Some(SessionState::default()));
    app.focus = Focus::Transcript;
    app.handle_key_event(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    ));
    assert!(!app.should_quit);
    assert!(app.overlay.is_none());
}

#[tokio::test]
async fn composer_attachment_clear_invalidates_pending_uploads_before_idle_exit() {
    use agena_api::part::PartExecutionStatusResource;
    use agena_domain::{ResourceActivity, ResourceDelivery, ResourceKind, ResourceReference};
    for state in [
        PartExecutionStatusResource::InProgress,
        PartExecutionStatusResource::Completed,
    ] {
        let (_temp, mut app) = app(Some(SessionState::default()));
        app.reserve_pending_composer_media(
            ResourceActivity {
                delivery: ResourceDelivery::Reference,
                kind: ResourceKind::File,
                reference: ResourceReference::WorkspacePath {
                    path: "draft.txt".into(),
                },
                name: "draft.txt".into(),
                media_type: None,
                size_bytes: Some(1),
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            },
            None,
        );
        app.composer_items[0].state = state;
        assert!(app.toggle_composer_item_selection());
        let epoch = app.composer_media_epoch;
        app.handle_key_event(ctrl_c());
        assert!(app.composer.text().is_empty());
        assert!(app.composer_items.is_empty());
        assert!(!app.composer_item_selection.is_active());
        assert_eq!(app.composer_media_epoch, epoch.wrapping_add(1));
        assert!(!app.should_quit);
        app.handle_key_event(ctrl_c());
        assert!(app.should_quit);
    }
}

#[tokio::test]
async fn local_help_modals_prompt_history_and_btw_own_interrupt_before_exit() {
    for surface in ["help", "search", "history", "btw"] {
        let (_temp, mut app) = app(Some(running(
            uuid::Uuid::new_v4(),
            ExecutionPhase::StreamingModel,
        )));
        app.composer.set_text("keep this draft".into());
        match surface {
            "help" => app.toggle_context_help(),
            "search" => {
                app.focus = Focus::Transcript;
                app.open_transcript_search_overlay(true);
            }
            "history" => {
                app.prompt_history.items.push("previous prompt".into());
                app.open_prompt_history_search();
                assert!(app.prompt_history_search.is_some());
            }
            "btw" => {
                app.open_btw("");
                app.btw_sessions
                    .get_mut(&17)
                    .unwrap()
                    .input
                    .set_text("side question".into());
            }
            _ => unreachable!(),
        }
        app.handle_key_event(ctrl_c());
        assert_eq!(app.composer.text(), "keep this draft", "{surface}");
        assert!(app.context_help.is_none(), "{surface}");
        assert!(app.overlay.is_none(), "{surface}");
        assert!(app.prompt_history_search.is_none(), "{surface}");
        assert!(!app.should_quit, "{surface}");
        assert!(app.exit_cancellation.is_none(), "{surface}");
        if surface == "btw" {
            assert!(app.btw_has_focus());
            assert_eq!(
                app.btw_sessions.get(&17).unwrap().input.text(),
                "side question"
            );
        }
    }
}

#[tokio::test]
async fn hidden_draft_has_two_actions_and_exit_flushes_it_without_cancellation() {
    for non_chat in [false, true] {
        let (_temp, mut app) = app(Some(SessionState::default()));
        app.composer.set_text("hidden draft".into());
        app.focus = Focus::Transcript;
        if non_chat {
            app.current_route = Route::SessionWork(17);
        }
        app.handle_key_event(ctrl_c());
        assert_eq!(menu_values(&app), ["detach", "cancel"]);
        let stack_len = app.overlay_stack.len();
        app.handle_key_event(ctrl_c());
        assert_eq!(app.overlay_stack.len(), stack_len);
        assert!(!app.should_quit);
        accept(&mut app, "cancel");
        assert_eq!(app.composer.text(), "hidden draft");
        assert!(app.overlay.is_none());
        app.handle_key_event(ctrl_c());
        accept(&mut app, "detach");
        assert!(app.should_quit);
        assert!(app.exit_cancellation.is_none());
        assert_eq!(
            app.draft_store.get(DraftSlot::Session(17)).unwrap().text(),
            "hidden draft"
        );
        assert!(!app.draft_store_dirty);
    }
}

#[tokio::test]
async fn menu_revalidates_targets_preserves_return_and_never_acquires_a_new_run() {
    for value in ["cancel", "stop-and-exit"] {
        for change in ["finished", "new run", "cancelling", "session", "route"] {
            let id = uuid::Uuid::new_v4();
            let (_temp, mut app) = app(Some(running(id, ExecutionPhase::StreamingModel)));
            app.handle_key_event(ctrl_c());
            select(&mut app, value);
            match change {
                "finished" => {
                    app.transcript.execution.as_mut().unwrap().session.state =
                        SessionState::default()
                }
                "new run" => {
                    app.transcript.execution.as_mut().unwrap().session.state =
                        running(uuid::Uuid::new_v4(), ExecutionPhase::Starting)
                }
                "cancelling" => {
                    app.transcript.execution.as_mut().unwrap().session.state =
                        running(id, ExecutionPhase::Cancelling)
                }
                "session" => app.transcript.session_id = Some(18),
                "route" => app.current_route = Route::SessionWork(17),
                _ => unreachable!(),
            }
            app.on_tick();
            assert_eq!(menu_values(&app), ["detach", "cancel"], "{change}/{value}");
            assert_eq!(
                selected(&app),
                Some(selection("cancel")),
                "{change}/{value}"
            );
            assert!(app.exit_menu_target.is_none());
            assert!(!app.should_quit);
            // Commit also checks independently of the tick or rendered items.
            assert!(!app.commit_exit_menu(&selection("stop-and-exit")));
            assert!(app.exit_cancellation.is_none());
        }
    }
    let (_temp, mut app) = app(Some(SessionState::default()));
    app.run_activity
        .begin(RunActivityTarget::Session(17), RunOperation::SubmitMessage);
    app.open_exit_menu();
    select(&mut app, "cancel");
    app.transcript.execution.as_mut().unwrap().session.state =
        running(uuid::Uuid::new_v4(), ExecutionPhase::Starting);
    app.on_tick();
    assert_eq!(menu_values(&app), ["detach", "cancel"]);
    assert_eq!(selected(&app), Some(selection("cancel")));
    accept(&mut app, "cancel");
    app.open_exit_menu();
    assert_eq!(menu_values(&app), ["detach", "stop-and-exit", "cancel"]);
}

#[tokio::test]
async fn commit_rechecks_a_changed_execution_without_relying_on_menu_refresh() {
    let (_temp, mut app) = app(Some(running(
        uuid::Uuid::new_v4(),
        ExecutionPhase::StreamingModel,
    )));
    app.open_exit_menu();
    app.transcript.execution.as_mut().unwrap().session.state =
        running(uuid::Uuid::new_v4(), ExecutionPhase::Starting);
    assert!(!app.commit_exit_menu(&selection("stop-and-exit")));
    assert!(app.exit_cancellation.is_none());
    assert!(!app.should_quit);
}

#[tokio::test]
async fn stopping_is_idempotent_and_return_withdraws_only_the_exit_intention() {
    let id = uuid::Uuid::new_v4();
    let (_temp, mut app) = app(Some(running(id, ExecutionPhase::StreamingModel)));
    app.open_exit_menu();
    accept(&mut app, "stop-and-exit");
    let request = app.exit_cancellation.unwrap();
    assert_eq!(
        request.target,
        ExitCancelTarget {
            session_id: 17,
            execution_id: id.into()
        }
    );
    assert!(request.awaiting_ack && request.quit_on_ack);
    assert_eq!(menu_values(&app), ["detach", "cancel"]);
    for _ in 0..3 {
        app.handle_key_event(ctrl_c());
        assert_eq!(
            app.exit_cancellation.unwrap().request_id,
            request.request_id
        );
        assert!(!app.should_quit);
    }
    accept(&mut app, "cancel");
    assert!(!app.exit_cancellation.unwrap().quit_on_ack);
    app.handle_message(AppMessage::ExitRunCancelled {
        request_id: request.request_id,
        target: request.target,
        result: Ok(CancellationResult::CancellationRequested.into()),
    });
    assert!(!app.should_quit);
    app.handle_key_event(ctrl_c());
    assert_eq!(menu_values(&app), ["detach", "cancel"]);
    accept(&mut app, "detach");
    assert!(app.should_quit);
}

#[tokio::test]
async fn only_the_matching_exact_ack_can_exit_and_failures_never_retry_session_wide() {
    for result in [
        CancellationResult::CancellationRequested,
        CancellationResult::AlreadyTerminal,
        CancellationResult::NotFound,
        CancellationResult::ExecutionMismatch,
    ] {
        let (_temp, mut app) = app(Some(running(
            uuid::Uuid::new_v4(),
            ExecutionPhase::StreamingModel,
        )));
        let target = app.current_exit_cancel_target().unwrap();
        let request_id = uuid::Uuid::new_v4();
        app.exit_cancellation = Some(ExitCancellation {
            target,
            request_id,
            awaiting_ack: true,
            quit_on_ack: true,
        });
        app.handle_message(AppMessage::ExitRunCancelled {
            request_id: uuid::Uuid::new_v4(),
            target,
            result: Ok(CancellationResult::CancellationRequested.into()),
        });
        app.handle_message(AppMessage::ExitRunCancelled {
            request_id,
            target: ExitCancelTarget {
                execution_id: ExecutionId::new(),
                ..target
            },
            result: Ok(CancellationResult::CancellationRequested.into()),
        });
        assert!(!app.should_quit);
        assert!(app.exit_cancellation.unwrap().awaiting_ack);
        app.handle_message(AppMessage::ExitRunCancelled {
            request_id,
            target,
            result: Ok(result.into()),
        });
        assert_eq!(
            app.should_quit,
            result != CancellationResult::ExecutionMismatch,
            "{result:?}"
        );
        if result == CancellationResult::ExecutionMismatch {
            assert!(app.exit_cancellation.is_none());
            assert!(
                app.transcript
                    .execution
                    .as_ref()
                    .unwrap()
                    .session
                    .state
                    .active_execution()
                    .is_some()
            );
        }
    }
    let (_temp, mut app) = app(Some(running(
        uuid::Uuid::new_v4(),
        ExecutionPhase::StreamingModel,
    )));
    let target = app.current_exit_cancel_target().unwrap();
    let request_id = uuid::Uuid::new_v4();
    app.exit_cancellation = Some(ExitCancellation {
        target,
        request_id,
        awaiting_ack: true,
        quit_on_ack: true,
    });
    app.handle_message(AppMessage::ExitRunCancelled {
        request_id,
        target,
        result: Err(crate::UiFailure::from_backend(anyhow::anyhow!(
            "cancellation acknowledgement timed out"
        ))),
    });
    assert!(app.exit_cancellation.is_none());
    assert!(!app.should_quit);
}

#[tokio::test]
async fn late_cancellation_recovery_merges_the_original_session_draft_only() {
    for same_session in [false, true] {
        let (_temp, mut app) = app(Some(running(
            uuid::Uuid::new_v4(),
            ExecutionPhase::StreamingModel,
        )));
        let target = app.current_exit_cancel_target().unwrap();
        let request_id = uuid::Uuid::new_v4();
        app.exit_cancellation = Some(ExitCancellation {
            target,
            request_id,
            awaiting_ack: true,
            quit_on_ack: false,
        });
        if same_session {
            app.run_activity
                .begin(RunActivityTarget::Session(17), RunOperation::SubmitMessage);
            app.set_draft_for_slot(DraftSlot::Session(17), draft("later submit"));
        } else {
            app.set_draft_for_slot(DraftSlot::Session(17), draft("saved draft"));
            app.transcript.session_id = Some(18);
            app.transcript.execution = None;
        }
        app.composer.set_text("new composer text".into());
        app.session_composer.pending_restore_draft = Some(draft("later submit"));
        app.handle_message(AppMessage::ExitRunCancelled {
            request_id,
            target,
            result: Ok(CancellationOutcome {
                result: CancellationResult::CancellationRequested,
                restored_user_message: Some(draft("recovered input").document),
                restored_user_run_id: None,
            }),
        });
        let saved = app.draft_store.get(DraftSlot::Session(17)).unwrap().text();
        assert!(saved.contains("recovered input"));
        if same_session {
            assert!(saved.contains("new composer text"));
            assert!(saved.contains("later submit"));
            assert_eq!(app.composer.text(), saved);
        } else {
            assert!(saved.contains("saved draft"));
            assert_eq!(app.composer.text(), "new composer text");
        }
        assert_eq!(
            app.session_composer.pending_restore_draft,
            Some(draft("later submit"))
        );
        assert!(!app.should_quit);
    }
}

#[tokio::test]
async fn returning_to_the_original_session_does_not_import_another_sessions_pending_buffer() {
    let (_temp, mut app) = app(Some(running(
        uuid::Uuid::new_v4(),
        ExecutionPhase::StreamingModel,
    )));
    let target = app.current_exit_cancel_target().unwrap();
    let request_id = uuid::Uuid::new_v4();
    app.exit_cancellation = Some(ExitCancellation {
        target,
        request_id,
        awaiting_ack: true,
        quit_on_ack: false,
    });
    // A -> B (send, awaiting receipt) -> A before A's cancel response arrives.
    app.transcript.session_id = Some(18);
    app.run_activity
        .begin(RunActivityTarget::Session(18), RunOperation::SubmitMessage);
    app.set_draft_for_slot(DraftSlot::Session(18), draft("session B pending input"));
    app.session_composer.pending_restore_draft = Some(draft("session B pending input"));
    app.transcript.session_id = Some(17);
    app.composer.set_text("session A current draft".into());
    app.handle_message(AppMessage::ExitRunCancelled {
        request_id,
        target,
        result: Ok(CancellationOutcome {
            result: CancellationResult::CancellationRequested,
            restored_user_message: Some(draft("session A recovered input").document),
            restored_user_run_id: None,
        }),
    });
    let saved = app.draft_store.get(DraftSlot::Session(17)).unwrap().text();
    assert!(saved.contains("session A recovered input"));
    assert!(saved.contains("session A current draft"));
    assert!(!saved.contains("session B"));
    assert_eq!(app.composer.text(), saved);
    assert_eq!(
        app.draft_store.get(DraftSlot::Session(18)),
        Some(&draft("session B pending input"))
    );
    assert_eq!(
        app.session_composer.pending_restore_draft,
        Some(draft("session B pending input"))
    );
    assert!(
        app.run_activity
            .has_operation(RunActivityTarget::Session(18), RunOperation::SubmitMessage)
    );
    assert!(!app.should_quit);
}

#[test]
fn exit_menu_labels_exist_in_all_supported_locales_and_unknown_choices_are_safe() {
    for (locale, _) in agena_tui::i18n::SUPPORTED_LOCALES {
        let i18n = I18n::resolve(Some(locale), None);
        for key in [
            "exit-menu-title",
            "exit-menu-prompt",
            "exit-menu-detach",
            "exit-menu-detach-detail",
            "exit-menu-stop-and-exit",
            "exit-menu-stop-and-exit-detail",
            "exit-menu-cancel",
            "exit-menu-cancel-detail",
            "exit-menu-stopping",
            "exit-menu-draft-prompt",
            "exit-menu-pending-prompt",
            "exit-menu-idle-prompt",
            "exit-menu-target-changed",
            "context-help-key-exit-menu",
        ] {
            let value = i18n.text(key);
            assert!(
                !value.is_empty() && !value.contains("Unknown localization key"),
                "{locale}/{key}: {value}"
            );
        }
    }
    for input in [
        ChoiceSelection::Clear,
        selection("unexpected"),
        selection("cancel"),
    ] {
        assert_eq!(ExitChoice::from_selection(&input), ExitChoice::Cancel);
    }
}
