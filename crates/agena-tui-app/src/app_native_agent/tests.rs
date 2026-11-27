use super::*;
use agena_domain::{
    ExternalAgentAuth, ExternalAgentAuthFlow, ExternalAgentIndexCapabilities,
    ExternalAgentSessionResult as SessionResult,
};
use agena_domain::{
    ExternalAgentAuthMethod, ExternalAgentSessionCapabilities, ExternalAgentSessionInfo,
};
use agena_tui::i18n::I18n;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn app() -> (tempfile::TempDir, App) {
    let temp = tempfile::tempdir().unwrap();
    let backend = TuiBackend::isolated_test_backend(temp.path().to_owned());
    let mut app = App::new_with_backend(backend, crate::LaunchOptions::default(), I18n::english());
    app.draft_store_path = temp.path().join("drafts.json");
    app.prompt_history_path = temp.path().join("history.jsonl");
    app.current_route = crate::Route::Main;
    (temp, app)
}

fn page() -> NativeAgentPage {
    NativeAgentPage {
        workspace_id: 9,
        workspace_path: "/test/project".into(),
        agent: ExternalAgent::ClaudeCode,
        agent_name: "Claude Code".into(),
        archived: ArchiveFilter::Unarchived,
        cursors: Vec::new(),
    }
}

fn session() -> ExternalAgentSessionInfo {
    ExternalAgentSessionInfo {
        native_id: "native-opaque-id".into(),
        cwd: "/test/project".into(),
        title: Some("Saved native conversation".into()),
        updated_at: None,
    }
}

fn choices(app: &App) -> &BTreeMap<String, NativeAgentChoice> {
    let Some(Overlay::Choice(dialog)) = &app.overlay else {
        panic!("expected native ACP choices")
    };
    let ChoiceOverlayAction::NativeAgent(menu) = &dialog.action else {
        panic!("expected native ACP menu")
    };
    &menu.actions
}

fn choose(app: &mut App, matches: impl Fn(&NativeAgentChoice) -> bool) {
    let key = choices(app)
        .iter()
        .find(|(_, action)| matches(action))
        .expect("available action")
        .0
        .clone();
    let Some(Overlay::Choice(dialog)) = &mut app.overlay else {
        unreachable!()
    };
    assert!(
        dialog
            .presentation
            .select_item_where(|item| item.value == key)
    );
    app.handle_overlay_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

#[test]
fn native_browser_preserves_an_opaque_cursor_and_routes_archive_to_the_chosen_id() {
    let (_temp, mut app) = app();
    let page = page();
    let second = page
        .next(Some(String::new()))
        .expect("empty cursor is a valid next page");
    assert_eq!(second.cursors.last().map(String::as_str), Some(""));
    assert!(second.next(Some(String::new())).is_none());
    assert!(second.previous().unwrap().cursors.is_empty());
    app.show_native_page(
        second,
        vec![session().into()],
        None,
        ExternalAgentIndexCapabilities {
            archive: true,
            ..Default::default()
        },
    );
    choose(&mut app, |action| {
        matches!(action, NativeAgentChoice::Conversation { .. })
    });
    assert!(choices(&app).values().any(|action| matches!(action,
            NativeAgentChoice::Open { page, entry } if page.workspace_id == 9 && entry.session.native_id == "native-opaque-id"
        )));
    assert!(choices(&app).values().any(|action| matches!(action,
            NativeAgentChoice::Archive { page, native_id, archived: true } if page.agent == ExternalAgent::ClaudeCode && native_id == "native-opaque-id"
        )));
    app.handle_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(choices(&app).values().any(|action| matches!(action,
        NativeAgentChoice::Browse(page) if page.cursors.is_empty()
    )));
}

#[test]
fn rewind_requires_confirmation_of_the_advertised_position() {
    let (_temp, mut app) = app();
    let session_id = agena_domain::ACP_SESSION_ID_START + 14;
    app.show_native_management(
        session_id,
        SessionResult::Status {
            session: Some(session()),
            native_status: Some("saved".into()),
            additional_directories: Vec::new(),
            capabilities: ExternalAgentSessionCapabilities {
                rewind_before_message: true,
                continue_conversation: true,
                ..Default::default()
            },
            fork_points: Vec::new(),
            rewind_points: vec![ExternalAgentForkPoint {
                message_id: 18,
                label: "Choose this message".into(),
            }],
        },
    );
    choose(&mut app, |action| {
        matches!(
            action,
            NativeAgentChoice::Positions {
                operation: NativeAgentPosition::Rewind,
                ..
            }
        )
    });
    choose(&mut app, |action| {
        matches!(action, NativeAgentChoice::Confirm { .. })
    });
    let Some(Overlay::Confirm(dialog)) = &app.overlay else {
        panic!("expected native rewind confirmation")
    };
    assert!(
        matches!(dialog.action, ConfirmAction::NativeAgentChange { session_id: target, action: SessionAction::Rewind { before_message_id: 18 } } if target == session_id)
    );
    assert_eq!(dialog.body_lines[0], "Choose this message");
    app.handle_overlay_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        choices(&app)
            .values()
            .any(|action| matches!(action, NativeAgentChoice::Confirm { .. }))
    );
}

#[test]
fn auth_menu_submits_agent_methods_and_exposes_supported_sign_out() {
    let (_temp, mut app) = app();
    let session_id = agena_domain::ACP_SESSION_ID_START;
    app.show_native_auth(
        session_id,
        ExternalAgentAuth {
            agent: ExternalAgent::ClaudeCode,
            cwd: "/test/project".into(),
            command: None,
            base_args: Vec::new(),
            accounts: Vec::new(),
            state: Some("unauthenticated".into()),
            logout_supported: true,
            methods: vec![
                ExternalAgentAuthMethod {
                    id: "browser".into(),
                    name: "Sign in with browser".into(),
                    description: None,
                    flow: ExternalAgentAuthFlow::Agent,
                },
                ExternalAgentAuthMethod {
                    id: "terminal".into(),
                    name: "Terminal".into(),
                    description: None,
                    flow: ExternalAgentAuthFlow::Terminal {
                        args: Vec::new(),
                        env: BTreeMap::new(),
                    },
                },
            ],
        },
    );
    let methods = choices(&app)
        .values()
        .filter_map(|action| match action {
            NativeAgentChoice::Auth {
                action: AuthAction::Login { method_id },
                ..
            } => Some(method_id.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(methods, ["browser"]);
    assert!(choices(&app).values().any(|action| matches!(
        action,
        NativeAgentChoice::Auth {
            action: AuthAction::Logout,
            ..
        }
    )));
}

#[test]
fn external_agent_slash_command_filter_excludes_agena_session_commands_and_keeps_model() {
    use agena_api::client_command::ClientCommandAction as Action;

    for excluded in [
        Action::Compact,
        Action::Clear,
        Action::Fork,
        Action::Rewind,
        Action::Rename,
        Action::Btw,
        Action::Side,
        Action::Schedules,
        Action::Integrations,
    ] {
        assert!(
            !crate::commands::command_applies_to_external_agent(excluded),
            "{excluded:?} should be hidden in ACP sessions"
        );
    }
    for kept in [Action::Help, Action::Model, Action::Status, Action::Settings, Action::Copy] {
        assert!(
            crate::commands::command_applies_to_external_agent(kept),
            "{kept:?} should stay available in ACP sessions"
        );
    }
}

#[test]
fn archive_filter_cycles_unarchived_archived_all() {
    let (_temp, mut app) = app();
    let caps = ExternalAgentIndexCapabilities {
        archive: true,
        ..Default::default()
    };

    app.show_native_page(page(), vec![session().into()], None, caps);
    assert!(choices(&app).values().any(|action| matches!(
        action,
        NativeAgentChoice::Browse(next) if next.archived == ArchiveFilter::Archived
    )));

    let mut archived_page = page();
    archived_page.archived = ArchiveFilter::Archived;
    app.show_native_page(archived_page, vec![session().into()], None, caps);
    assert!(choices(&app).values().any(|action| matches!(
        action,
        NativeAgentChoice::Browse(next) if next.archived == ArchiveFilter::All
    )));

    let mut all_page = page();
    all_page.archived = ArchiveFilter::All;
    app.show_native_page(all_page, vec![session().into()], None, caps);
    assert!(choices(&app).values().any(|action| matches!(
        action,
        NativeAgentChoice::Browse(next) if next.archived == ArchiveFilter::Unarchived
    )));
}

#[test]
fn providers_menu_renders_supported_routes_and_unconfigured_targets() {
    let (_temp, mut app) = app();
    let session_id = agena_domain::ACP_SESSION_ID_START + 2;
    app.show_native_providers(
        session_id,
        agena_domain::ExternalAgentProviders {
            supported: true,
            providers: vec![
                agena_domain::ExternalAgentProvider {
                    provider_id: "main".into(),
                    supported: vec!["anthropic".into()],
                    required: true,
                    current: Some(agena_domain::ExternalAgentProviderTarget {
                        api_type: "anthropic".into(),
                        base_url: "https://api.example.com".into(),
                    }),
                },
                agena_domain::ExternalAgentProvider {
                    provider_id: "fast".into(),
                    supported: vec!["openai".into()],
                    required: false,
                    current: None,
                },
            ],
        },
    );
    let Some(Overlay::Choice(dialog)) = &app.overlay else {
        panic!("expected provider overlay")
    };
    let prompt = &dialog.presentation.prompt;
    assert!(prompt.contains("main"));
    assert!(prompt.contains("https://api.example.com"));
    assert!(prompt.contains("fast"));
    assert!(choices(&app).values().any(|action| matches!(
        action,
        NativeAgentChoice::Providers(id) if *id == session_id
    )));
}

#[test]
fn auth_menu_renders_live_connection_status_and_full_account_details() {
    let (_temp, mut app) = app();
    let session_id = agena_domain::ACP_SESSION_ID_START + 3;
    app.show_native_auth_with_live(
        session_id,
        ExternalAgentAuth {
            agent: ExternalAgent::ClaudeCode,
            cwd: "/test/project".into(),
            command: None,
            base_args: Vec::new(),
            accounts: Vec::new(),
            state: Some("unauthenticated".into()),
            logout_supported: true,
            methods: Vec::new(),
        },
        Some(agena_domain::ExternalAgentAuthStatus {
            state: "authenticated".into(),
            accounts: vec![agena_domain::ExternalAgentAuthAccount {
                label: "Work".into(),
                active: true,
                description: Some("Enterprise plan".into()),
                name: Some("Alice".into()),
                email: Some("alice@example.com".into()),
                organization: Some("Acme".into()),
            }],
        }),
    );
    let Some(Overlay::Choice(dialog)) = &app.overlay else {
        panic!("expected auth overlay")
    };
    let prompt = &dialog.presentation.prompt;
    assert!(prompt.contains("Alice"));
    assert!(prompt.contains("alice@example.com"));
    assert!(prompt.contains("Acme"));
    assert!(prompt.contains("Enterprise plan"));
}
