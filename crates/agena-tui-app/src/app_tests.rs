use super::{
    ConfigJsonSources, I18n, JsonValue, PermissionAction, PermissionConfig, PermissionMode,
    PermissionOverlayChoice, PermissionPromptDecision, PermissionPromptPage, PermissionReplyKind,
    PermissionRequest, PermissionRuleSubjectKind, PermissionScope, PermissionStudioCatalogKind,
    PermissionStudioModeTarget, RenderedTranscriptNode, SettingsPickerAction, ToolPermissionRules,
    TranscriptMoveDirection, TranscriptNodeKey, TranscriptNodeKind, Utc,
    apply_permission_studio_entries_mode, apply_permission_studio_mode_input,
    initial_search_match_index, path_rule_modes, permission_overlay_choice,
    permission_rule_draft_from_request, settings_studio_permission_items,
    transcript_message_navigation_target, transcript_node_highlight_range,
    transcript_selection_scroll_position,
};

mod ui;

/// Shared v2 parts fixtures for app tests. The wire transcript is an ordered
/// part list (database-design-v2.md §4.1.1); these helpers build the small
/// `run`/`text`/`error`/`think` shapes the rendering tests exercise. Content
/// parts carry no `run_id` — `parts_entries` groups by sequence, so the tests
/// only need the marker/content order.
#[cfg(test)]
mod parts_fixtures {
    use agena_api::part::ErrorPartResource;
    use agena_api::resource::SessionTranscriptPart;

    pub(super) fn run(part_id: i64, role: &str, state: &str) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "run".to_owned(),
            role: role.to_owned(),
            state: state.to_owned(),
            content: serde_json::json!({ "run_kind": "user_send" }),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: Some(part_id),
        }
    }

    pub(super) fn text(part_id: i64, role: &str, text: &str) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "text".to_owned(),
            role: role.to_owned(),
            state: "completed".to_owned(),
            content: serde_json::json!({ "text": text }),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: None,
        }
    }

    pub(super) fn error(
        part_id: i64,
        role: &str,
        problem: agena_failure::UserProblem,
    ) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "error".to_owned(),
            role: role.to_owned(),
            state: "failed".to_owned(),
            content: serde_json::to_value(ErrorPartResource { problem })
                .expect("error part serializes"),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: None,
        }
    }

    pub(super) fn think(part_id: i64, role: &str, summary: Vec<String>) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "think".to_owned(),
            role: role.to_owned(),
            state: "completed".to_owned(),
            content: serde_json::json!({ "summary": summary }),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: None,
        }
    }

    pub(super) fn hook(
        part_id: i64,
        role: &str,
        summary: &str,
        detail: &str,
    ) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "hook".to_owned(),
            role: role.to_owned(),
            state: "completed".to_owned(),
            content: serde_json::json!({
                "hook": "background",
                "summary": summary,
                "detail": detail,
            }),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: None,
        }
    }

    pub(super) fn paste(part_id: i64, role: &str, text: &str) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "paste_ref".to_owned(),
            role: role.to_owned(),
            state: "completed".to_owned(),
            content: serde_json::json!({ "text": text }),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: None,
        }
    }

    pub(super) fn file_ref(part_id: i64, role: &str, path: &str) -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id,
            kind: "file_ref".to_owned(),
            role: role.to_owned(),
            state: "completed".to_owned(),
            content: serde_json::json!({ "path": path, "name": path }),
            summary: None,
            created_at_ms: part_id * 10,
            parent_part_id: None,
            run_id: None,
        }
    }
}

#[cfg(test)]
mod interactive_request_visibility_tests {
    use std::collections::BTreeSet;

    use agena_api::resource::{
        PendingInteractiveRequest, PendingInteractiveRequestResource, PermissionActionResource,
        PermissionRequest, UserInputRequest,
    };
    use chrono::Utc;

    use super::super::{
        first_auto_open_pending_interactive_request, first_unseen_pending_interactive_request,
        pending_interactive_request_id, pending_interactive_request_is_presented,
    };

    fn permission(request_id: &str) -> PendingInteractiveRequestResource {
        PendingInteractiveRequestResource {
            session_id: 8,
            parent_session_id: None,
            task_id: None,
            request: PendingInteractiveRequest::Permission {
                request: PermissionRequest {
                    request_id: request_id.to_owned(),
                    session_id: Some(8),
                    action: PermissionActionResource::Tool {
                        tool_name: "fs.write".to_owned(),
                        qualifier: None,
                    },
                    related_actions: Vec::new(),
                    requested_actions: Vec::new(),
                    reason: "write the requested file".to_owned(),
                    explanation: String::new(),
                    source: Some("static_policy".to_owned()),
                    scope: None,
                    operator: None,
                    trace: Vec::new(),
                    created_at: Utc::now(),
                },
            },
        }
    }

    fn user_input(request_id: &str) -> PendingInteractiveRequestResource {
        PendingInteractiveRequestResource {
            session_id: 8,
            parent_session_id: None,
            task_id: None,
            request: PendingInteractiveRequest::UserInput {
                request: UserInputRequest {
                    request_id: request_id.to_owned(),
                    session_id: Some(8),
                    title: "Choose".to_owned(),
                    body_markdown: String::new(),
                    kind: String::new(),
                    source: agena_domain::UserInputSource::Plugin,
                    auto_resolution_ms: None,
                    presented_at: None,
                    questions: Vec::new(),
                    created_at: Utc::now(),
                },
            },
        }
    }

    fn presented_user_input(request_id: &str) -> PendingInteractiveRequestResource {
        PendingInteractiveRequestResource {
            session_id: 8,
            parent_session_id: None,
            task_id: None,
            request: PendingInteractiveRequest::UserInput {
                request: UserInputRequest {
                    request_id: request_id.to_owned(),
                    session_id: Some(8),
                    title: "Choose".to_owned(),
                    body_markdown: String::new(),
                    kind: String::new(),
                    source: agena_domain::UserInputSource::Plugin,
                    auto_resolution_ms: None,
                    presented_at: Some(Utc::now()),
                    questions: Vec::new(),
                    created_at: Utc::now(),
                },
            },
        }
    }

    #[test]
    fn failed_permission_reply_makes_the_same_durable_request_visible_again() {
        let requests = vec![permission("permission-a"), permission("permission-b")];
        let mut seen_permissions = BTreeSet::from(["permission-a".to_owned()]);
        let seen_user_inputs = BTreeSet::new();

        // Submitting closes the modal, but a rejected backend command must
        // remove exactly that request from the visibility ledger.
        assert!(seen_permissions.remove("permission-a"));

        let next = first_unseen_pending_interactive_request(
            requests.as_slice(),
            &seen_permissions,
            &seen_user_inputs,
        )
        .expect("the rejected permission must be offered again");
        assert_eq!(pending_interactive_request_id(next), "permission-a");
    }

    #[test]
    fn successful_permission_reply_advances_to_the_next_pending_request() {
        let requests = vec![permission("permission-b")];
        let mut seen_permissions = BTreeSet::from(["permission-a".to_owned()]);
        let seen_user_inputs = BTreeSet::new();

        // Applying the authoritative execution snapshot retains only IDs
        // that are still pending; the completed first request disappears.
        seen_permissions.retain(|request_id| {
            requests
                .iter()
                .any(|request| pending_interactive_request_id(request) == request_id)
        });

        let next = first_unseen_pending_interactive_request(
            requests.as_slice(),
            &seen_permissions,
            &seen_user_inputs,
        )
        .expect("the second permission must become the next modal");
        assert_eq!(pending_interactive_request_id(next), "permission-b");
    }

    #[test]
    fn failed_user_input_reply_makes_the_same_durable_request_visible_again() {
        let requests = vec![user_input("input-a")];
        let seen_permissions = BTreeSet::new();
        let mut seen_user_inputs = BTreeSet::from(["input-a".to_owned()]);

        assert!(seen_user_inputs.remove("input-a"));

        let next = first_unseen_pending_interactive_request(
            requests.as_slice(),
            &seen_permissions,
            &seen_user_inputs,
        )
        .expect("the rejected user-input request must be offered again");
        assert_eq!(pending_interactive_request_id(next), "input-a");
    }

    #[test]
    fn never_presented_user_input_is_not_durably_presented_and_auto_opens() {
        let request = user_input("input-fresh");
        assert!(!pending_interactive_request_is_presented(&request));
        let requests = vec![request];
        let next = first_auto_open_pending_interactive_request(
            requests.as_slice(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .expect("a never-presented request must auto-open");
        assert_eq!(pending_interactive_request_id(next), "input-fresh");
    }

    #[test]
    fn presented_but_unanswered_user_input_is_not_auto_opened_again() {
        let request = presented_user_input("input-presented");
        assert!(pending_interactive_request_is_presented(&request));
        let requests = vec![request];
        let next = first_auto_open_pending_interactive_request(
            requests.as_slice(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        );
        assert!(
            next.is_none(),
            "presented-but-unanswered requests must stay behind the persistent hint"
        );
    }

    #[test]
    fn locally_seen_request_is_not_auto_opened_again() {
        let requests = vec![user_input("input-seen")];
        let next = first_auto_open_pending_interactive_request(
            requests.as_slice(),
            &BTreeSet::new(),
            &BTreeSet::from(["input-seen".to_owned()]),
        );
        assert!(
            next.is_none(),
            "a request already shown this session must not re-popup"
        );
    }

    #[test]
    fn permission_requests_always_remain_auto_open_candidates_until_replied() {
        let request = permission("permission-pending");
        assert!(!pending_interactive_request_is_presented(&request));
        let requests = vec![request];
        let next = first_auto_open_pending_interactive_request(
            requests.as_slice(),
            &BTreeSet::new(),
            &BTreeSet::new(),
        )
        .expect("permissions have no durable presentation and must auto-open");
        assert_eq!(pending_interactive_request_id(next), "permission-pending");
    }

    #[test]
    fn auto_open_skips_presented_or_seen_but_keeps_later_fresh_requests() {
        let requests = vec![
            presented_user_input("input-presented"),
            user_input("input-seen"),
            user_input("input-fresh"),
        ];
        let next = first_auto_open_pending_interactive_request(
            requests.as_slice(),
            &BTreeSet::new(),
            &BTreeSet::from(["input-seen".to_owned()]),
        )
        .expect("the fresh request must still auto-open");
        assert_eq!(pending_interactive_request_id(next), "input-fresh");
    }
}

/// The reply-builder shared by the overlay path and the new inline
/// interaction-node routing ("everything is a part"). These tests pin the
/// contract the key router relies on: review decisions map to `answers["0"]`,
/// ask-user questions map by positional index, and custom feedback routes into
/// the right editor field for each flow.
#[cfg(test)]
mod interaction_reply_building_tests {
    use agena_domain::{UserInputKind, UserInputQuestion, UserInputSource};
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::super::{App, I18n};
    use agena_tui::user_input::UserInputEffect;

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }

    fn edit() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)
    }

    fn review_request() -> agena_domain::UserInputRequest {
        agena_domain::UserInputRequest {
            request_id: "host-input:1:2:0".to_owned(),
            session_id: Some(7),
            title: "Approve New Plan".to_owned(),
            body_markdown: String::new(),
            kind: UserInputKind::Review,
            source: UserInputSource::Host,
            auto_resolution_ms: None,
            presented_at: None,
            questions: vec![UserInputQuestion {
                header: "Decision".to_owned(),
                question: "Choose whether this plan should move to active.".to_owned(),
                options: vec![agena_domain::UserInputOption {
                    label: "Approve".to_owned(),
                    description: "Move it to active.".to_owned(),
                }],
                multiple: false,
                allow_custom: true,
            }],
            created_at: Utc::now(),
        }
    }

    fn ask_user_request(allow_custom: bool) -> agena_domain::UserInputRequest {
        agena_domain::UserInputRequest {
            request_id: "host-input:1:2:0".to_owned(),
            session_id: Some(7),
            title: "Choose".to_owned(),
            body_markdown: String::new(),
            kind: UserInputKind::AskUser,
            source: UserInputSource::Plugin,
            auto_resolution_ms: None,
            presented_at: None,
            questions: vec![UserInputQuestion {
                header: "Choice".to_owned(),
                question: "Pick one.".to_owned(),
                options: vec![agena_domain::UserInputOption {
                    label: "yes".to_owned(),
                    description: String::new(),
                }],
                multiple: false,
                allow_custom,
            }],
            created_at: Utc::now(),
        }
    }

    #[test]
    fn review_decision_submit_maps_to_answers_zero_option_label() {
        let mut dialog = App::build_user_input_overlay(7, review_request());
        assert!(dialog.presentation.is_review_decision());

        // Empty plan body → one placeholder row, then the separator, then the
        // decision rows. Walk the cursor through them: Enter on the plan row
        // and the separator are inert, Enter on the decision row submits.
        assert_eq!(
            dialog.presentation.handle_key(enter(), 10),
            UserInputEffect::KeepOpen,
            "Enter on the plan placeholder must not submit"
        );
        assert_eq!(
            dialog.presentation.handle_key(enter(), 10),
            UserInputEffect::KeepOpen,
            "Enter on the separator must not submit"
        );
        assert_eq!(
            dialog.presentation.handle_key(enter(), 10),
            UserInputEffect::Submit
        );

        let reply = App::build_structured_user_input_reply(&I18n::english(), &mut dialog, None)
            .expect("review decision builds a reply");
        assert_eq!(reply.answers["0"], ["Approve"]);
    }

    #[test]
    fn review_custom_feedback_routes_into_the_review_editor_and_submits() {
        let mut dialog = App::build_user_input_overlay(7, review_request());

        // Pasting onto an expanded review part must land in the review custom
        // editor, so a later submit reads it back through `review().custom_text()`.
        assert!(dialog.presentation.insert_custom_text("looks good"));
        let reply = App::build_structured_user_input_reply(&I18n::english(), &mut dialog, None)
            .expect("custom feedback builds a reply");
        assert_eq!(reply.answers["0"], ["looks good"]);
    }

    #[test]
    fn ask_user_option_submit_maps_to_answers_zero() {
        let mut dialog = App::build_user_input_overlay(7, ask_user_request(false));
        assert!(!dialog.presentation.is_review_decision());

        // A single non-multiple question hides the review header, so Enter
        // selects the first option and submits in one step.
        assert_eq!(
            dialog.presentation.handle_key(enter(), 10),
            UserInputEffect::Submit
        );

        let reply = App::build_structured_user_input_reply(&I18n::english(), &mut dialog, None)
            .expect("ask_user option builds a reply");
        assert_eq!(reply.answers["0"], ["yes"]);
    }

    #[test]
    fn ask_user_missing_answer_is_rejected_and_focuses_the_question() {
        let mut dialog = App::build_user_input_overlay(7, ask_user_request(false));

        let error = App::build_structured_user_input_reply(&I18n::english(), &mut dialog, None)
            .expect_err("an unanswered ask_user must not submit");
        assert!(!error.is_empty());
        assert_eq!(dialog.presentation.selected_question(), 0);
    }

    #[test]
    fn ask_user_custom_text_commits_on_enter_and_maps_by_index() {
        let mut dialog = App::build_user_input_overlay(7, ask_user_request(true));

        // `e` opens the question-flow custom editor, paste fills it, Enter
        // commits the draft and (single question) submits.
        assert_eq!(
            dialog.presentation.handle_key(edit(), 10),
            UserInputEffect::KeepOpen
        );
        assert!(dialog.presentation.insert_custom_text("custom answer"));
        assert_eq!(
            dialog.presentation.handle_key(enter(), 10),
            UserInputEffect::Submit
        );

        let reply = App::build_structured_user_input_reply(&I18n::english(), &mut dialog, None)
            .expect("ask_user custom text builds a reply");
        assert_eq!(reply.answers["0"], ["custom answer"]);
    }
}

macro_rules! api_message_part {
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, FixturePart::text($text:expr $(,)?) $(,)?) => {
        crate::TranscriptFixture::text_part($id, $message_id, $created_at, $status, $text)
    };
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, FixturePart::Reasoning($reasoning:expr) $(,)?) => {
        crate::TranscriptFixture::reasoning_part($id, $message_id, $created_at, $status, $reasoning)
    };
    ($id:expr, $message_id:expr, $created_at:expr, $status:expr, FixturePart::Text($text_part:expr) $(,)?) => {{
        let text_part = $text_part;
        crate::TranscriptFixture::text_part_with_flags(
            $id,
            $message_id,
            $created_at,
            $status,
            text_part.text,
            text_part.synthetic,
        )
    }};
}

/// The "everything is a part" interaction routing tests: the transcript cursor
/// IS the review cursor, and a thin context-aware action layer owns only
/// Enter-on-decision-row / Ctrl+X / `e`-on-custom / Esc-to-collapse while every
/// other key falls through to normal chat navigation. These tests pin the
/// plan's contract end-to-end through the real `App` (bootstrapped against an
/// in-memory runtime): key routing, decision-row submit/cancel, the inline
/// custom editor, auto-expand + focus with the revealed guard, selection sync,
/// and the reply handler's cleanup.
#[cfg(test)]
mod interaction_part_routing_tests {
    use agena_api::{
        part::RequestPartResource,
        resource::{
            ExecutionAccess, PendingInteractiveRequest, PendingInteractiveRequestResource,
            SessionExecutionResource, SessionExecutionContextResource, SessionLifecycleState,
            SessionRelationKind, SessionResource, SessionTranscriptPart, SessionUsageResource,
            WorkflowState,
        },
    };
    use agena_application::Application;
    use agena_domain::{UserInputKind, UserInputQuestion, UserInputSource};
    use agena_runtime::{RuntimeBootstrapRequest, bootstrap_application_services};
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::layout::Rect;

    use super::super::{App, I18n, LaunchOptions};
    use crate::app_types::{RunActivityTarget, RunOperation};
    use agena_tui_transcript::{TranscriptContentId, TranscriptEntryId, TranscriptNodeKey};

    const REQUEST_ID: &str = "host-input:1:2:0";
    const SESSION_ID: i64 = 7;
    const WIDTH: u16 = 80;
    const HEIGHT: u16 = 24;

    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }
    fn edit() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE)
    }
    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }
    fn ctrl_x() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
    }
    fn space_key() -> KeyEvent {
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)
    }
    fn char_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn page_down() -> KeyEvent {
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)
    }

    /// A single-option review decision with a custom slot and a one-line plan
    /// body, so the body-offset layout is fully determined (ONE row per option
    /// since the layout-contract refactor):
    ///   0 plan row | 1 separator | 2 option label
    ///   3 custom label | 4 footer hint (PlanBody).
    fn wire_request() -> agena_api::resource::UserInputRequest {
        agena_api::resource::UserInputRequest {
            request_id: REQUEST_ID.to_owned(),
            session_id: Some(SESSION_ID),
            title: "Approve New Plan".to_owned(),
            body_markdown: "## Proposed Plan".to_owned(),
            kind: "review".to_owned(),
            source: UserInputSource::Host,
            auto_resolution_ms: None,
            presented_at: Some(Utc::now()),
            questions: vec![agena_api::resource::UserInputQuestion {
                header: "Decision".to_owned(),
                question: "Choose whether this plan should move to active.".to_owned(),
                options: vec![agena_api::resource::UserInputOption {
                    label: "Approve".to_owned(),
                    description: "Move it to active.".to_owned(),
                }],
                multiple: false,
                allow_custom: true,
            }],
            created_at: Utc::now(),
        }
    }

    fn domain_request() -> agena_domain::UserInputRequest {
        agena_domain::UserInputRequest {
            request_id: REQUEST_ID.to_owned(),
            session_id: Some(SESSION_ID),
            title: "Approve New Plan".to_owned(),
            body_markdown: "## Proposed Plan".to_owned(),
            kind: UserInputKind::Review,
            source: UserInputSource::Host,
            auto_resolution_ms: None,
            presented_at: None,
            questions: vec![UserInputQuestion {
                header: "Decision".to_owned(),
                question: "Choose whether this plan should move to active.".to_owned(),
                options: vec![agena_domain::UserInputOption {
                    label: "Approve".to_owned(),
                    description: "Move it to active.".to_owned(),
                }],
                multiple: false,
                allow_custom: true,
            }],
            created_at: Utc::now(),
        }
    }

    fn run_marker() -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id: 3,
            kind: "run".to_owned(),
            role: "assistant".to_owned(),
            state: "in_progress".to_owned(),
            content: serde_json::json!({ "run_kind": "user_send" }),
            summary: None,
            created_at_ms: 30,
            parent_part_id: None,
            run_id: Some(3),
        }
    }

    fn interaction_part() -> SessionTranscriptPart {
        SessionTranscriptPart {
            part_id: 5,
            kind: "interaction".to_owned(),
            role: "assistant".to_owned(),
            state: "in_progress".to_owned(),
            content: serde_json::to_value(RequestPartResource::UserInput {
                request: wire_request(),
                reply: None,
            })
            .expect("request part serializes"),
            summary: None,
            created_at_ms: 50,
            parent_part_id: None,
            run_id: Some(3),
        }
    }

    /// The canonical single-activity shape: a `tool_call` operation carrying an
    /// awaiting `user_input` record IS the pending interaction part (there is no
    /// separate `interaction` part anymore).
    fn operation_tool_call_part() -> SessionTranscriptPart {
        let operation = agena_api::part::OperationPartResource {
            call_id: 5,
            invocation: agena_api::part::ToolInvocationResource {
                name: "plan.review".to_owned(),
                ..Default::default()
            },
            user_input: agena_domain::OperationUserInput {
                requests: vec![agena_domain::OperationUserInputRecord {
                    request: domain_request(),
                    reply: None,
                    replied_at_ms: None,
                }],
            },
            ..Default::default()
        };
        SessionTranscriptPart {
            part_id: 5,
            kind: "tool_call".to_owned(),
            role: "assistant".to_owned(),
            state: "in_progress".to_owned(),
            content: serde_json::to_value(serde_json::json!({ "operation": operation }))
                .expect("operation serializes"),
            summary: None,
            created_at_ms: 50,
            parent_part_id: None,
            run_id: Some(3),
        }
    }

    fn parts() -> Vec<SessionTranscriptPart> {
        vec![run_marker(), interaction_part()]
    }

    fn node_key() -> TranscriptNodeKey {
        TranscriptNodeKey::Activity {
            entry_id: TranscriptEntryId::StoredMessage(3),
            content_id: TranscriptContentId::StoredPart(5),
        }
    }

    fn session_resource() -> SessionResource {
        SessionResource {
            id: SESSION_ID,
            parent_id: None,
            depth: 0,
            root_id: SESSION_ID,
            workspace_id: 1,
            title: "Test session".to_owned(),
            version: 1,
            relation_kind: SessionRelationKind::Root,
            lifecycle_state: SessionLifecycleState::Ready,
            source_cutoff_seq_global: None,
            source_message_id: None,
            is_subagent: false,
            task_id: None,
            subtask_access: None,
            subtask_status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            message_count: 1,
            child_session_count: 0,
            last_message_at: None,
        }
    }

    fn execution_with(
        pending: Vec<PendingInteractiveRequestResource>,
        parts: Vec<SessionTranscriptPart>,
    ) -> SessionExecutionResource {
        SessionExecutionResource {
            session: session_resource(),
            parts,
            workflow_state: WorkflowState::Quiescent,
            active_execution: None,
            latest_event_seq: Some(2),
            automation: None,
            execution: SessionExecutionContextResource {
                agent_id: "test".to_owned(),
                execution_access: ExecutionAccess::Inherit,
                selected_permission: Default::default(),
                effective_permission: Default::default(),
                permission_ceiling: Default::default(),
                model_provider_id: None,
                model_adapter_id: None,
                model_id: None,
                model_thinking_mode: None,
                model_speed_mode: None,
                model_verbosity: None,
                model_parallel_tool_calls: None,
                effective_workspace_root: None,
                task_id: None,
                subtask_status: None,
                subtask_started_at: None,
                subtask_finished_at: None,
                subtask_failure: None,
            },
            pending_interactive_requests: pending,
            usage: SessionUsageResource {
                measured_prompt_tokens: None,
                current_tokens: 0,
                projected_tokens: None,
                limit_tokens: None,
                limit_basis: None,
                reserved_tokens: None,
                model_context_window_tokens: None,
                model_max_input_tokens: None,
                model_max_output_tokens: None,
            },
        }
    }

    fn pending_user_input_resource() -> PendingInteractiveRequestResource {
        PendingInteractiveRequestResource {
            session_id: SESSION_ID,
            parent_session_id: None,
            task_id: None,
            request: PendingInteractiveRequest::UserInput {
                request: wire_request(),
            },
        }
    }

    /// Bootstrap a real `App` against an in-memory runtime, sized to an 80x24
    /// transcript viewport. The runtime is required because the reply path
    /// spawns real backend tasks; those fail against the empty in-memory
    /// database, which is fine for these tests (they assert the synchronous
    /// routing/state effects, not the backend round-trip).
    async fn seeded_app() -> App {
        let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
            workspace_root: Some(std::env::temp_dir()),
            database_url: Some("sqlite::memory:".to_owned()),
            initialize_schema: true,
            ..RuntimeBootstrapRequest::default()
        })
        .await
        .expect("build test runtime");
        let application = Application::from_composed_runtime_services(runtime.application_services())
            .expect("compose test application");
        let mut app = App::new(application, LaunchOptions::default(), I18n::english());
        app.layout.transcript_body = Rect::new(0, 0, WIDTH, HEIGHT);
        app.transcript.session_id = Some(SESSION_ID);
        app
    }

    /// Seed the pending review part (parts + execution + live interaction
    /// view + expanded node + the request in the interaction map).
    fn seed_pending_review(app: &mut App) {
        seed_pending_review_with_plan(app, "## Proposed Plan");
    }

    /// Same as [`seed_pending_review`] but with a caller-controlled plan body,
    /// used to build a review page tall enough to exceed the viewport.
    fn seed_pending_review_with_plan(app: &mut App, plan_markdown: &str) {
        let request = agena_domain::UserInputRequest {
            body_markdown: plan_markdown.to_owned(),
            ..domain_request()
        };
        app.transcript
            .apply_execution(execution_with(vec![pending_user_input_resource()], parts()));
        app.user_input_interactions.insert(
            REQUEST_ID.to_owned(),
            App::build_user_input_overlay(SESSION_ID, request),
        );
        app.sync_interaction_documents();
        app.transcript.node_expansions.insert(node_key(), true);
        app.transcript.invalidate_render();
    }

    /// The rendered start line of the interaction part node (absolute line in
    /// the rendered transcript).
    fn interaction_node_start(app: &mut App) -> usize {
        app.transcript
            .rendered(WIDTH)
            .nodes
            .iter()
            .find(|node| node.key == node_key())
            .expect("the interaction part renders a node")
            .start_line
    }

    /// Position the transcript cursor on the body line at `body_offset`
    /// (0 = first line after the activity headline). The activity headline is
    /// the node's 0-indexed `start_line`; `move_cursor_to_visual_line_number`
    /// is 1-indexed, so the target line number is `start_line + 2 + body_offset`.
    fn move_cursor_to_body_offset(app: &mut App, body_offset: usize) {
        let start_line = interaction_node_start(app);
        app.transcript
            .move_cursor_to_visual_line_number(WIDTH, HEIGHT, Some(start_line + 2 + body_offset));
    }

    /// A two-question ask-user request: Q0 "Flavor" (single-pick, 2 options,
    /// custom slot) and Q1 "Toppings" (multi-pick, 2 options, no custom). The
    /// ask-user wizard owns the page + option cursor, so the option rows are
    /// NOT cursor-driven — the `PendingInteractionView` only needs the headline
    /// to be under the cursor for the wizard to activate.
    fn ask_user_wire_request() -> agena_api::resource::UserInputRequest {
        agena_api::resource::UserInputRequest {
            request_id: REQUEST_ID.to_owned(),
            session_id: Some(SESSION_ID),
            title: "Flavor Survey".to_owned(),
            body_markdown: "## Proposed Plan".to_owned(),
            kind: "ask_user".to_owned(),
            source: UserInputSource::Host,
            auto_resolution_ms: None,
            presented_at: Some(Utc::now()),
            questions: vec![
                agena_api::resource::UserInputQuestion {
                    header: "Flavor".to_owned(),
                    question: "Pick one.".to_owned(),
                    options: vec![
                        agena_api::resource::UserInputOption {
                            label: "Vanilla".to_owned(),
                            description: "Classic.".to_owned(),
                        },
                        agena_api::resource::UserInputOption {
                            label: "Chocolate".to_owned(),
                            description: "Rich.".to_owned(),
                        },
                    ],
                    multiple: false,
                    allow_custom: true,
                },
                agena_api::resource::UserInputQuestion {
                    header: "Toppings".to_owned(),
                    question: "Choose any.".to_owned(),
                    options: vec![
                        agena_api::resource::UserInputOption {
                            label: "Sprinkles".to_owned(),
                            description: String::new(),
                        },
                        agena_api::resource::UserInputOption {
                            label: "Nuts".to_owned(),
                            description: String::new(),
                        },
                    ],
                    multiple: true,
                    allow_custom: false,
                },
            ],
            created_at: Utc::now(),
        }
    }

    fn ask_user_domain_request() -> agena_domain::UserInputRequest {
        agena_domain::UserInputRequest {
            request_id: REQUEST_ID.to_owned(),
            session_id: Some(SESSION_ID),
            title: "Flavor Survey".to_owned(),
            body_markdown: "## Proposed Plan".to_owned(),
            kind: UserInputKind::AskUser,
            source: UserInputSource::Host,
            auto_resolution_ms: None,
            presented_at: None,
            questions: vec![
                UserInputQuestion {
                    header: "Flavor".to_owned(),
                    question: "Pick one.".to_owned(),
                    options: vec![
                        agena_domain::UserInputOption {
                            label: "Vanilla".to_owned(),
                            description: "Classic.".to_owned(),
                        },
                        agena_domain::UserInputOption {
                            label: "Chocolate".to_owned(),
                            description: "Rich.".to_owned(),
                        },
                    ],
                    multiple: false,
                    allow_custom: true,
                },
                UserInputQuestion {
                    header: "Toppings".to_owned(),
                    question: "Choose any.".to_owned(),
                    options: vec![
                        agena_domain::UserInputOption {
                            label: "Sprinkles".to_owned(),
                            description: String::new(),
                        },
                        agena_domain::UserInputOption {
                            label: "Nuts".to_owned(),
                            description: String::new(),
                        },
                    ],
                    multiple: true,
                    allow_custom: false,
                },
            ],
            created_at: Utc::now(),
        }
    }

    fn seed_pending_ask_user(app: &mut App) {
        seed_pending_ask_user_with(app, ask_user_wire_request(), ask_user_domain_request());
    }

    /// Seed a pending ask-user part from caller-controlled wire + domain
    /// requests (the fixed `REQUEST_ID`/part id are kept), so a test can build
    /// a page taller than the viewport and pin the fit-scroll contract.
    fn seed_pending_ask_user_with(
        app: &mut App,
        wire: agena_api::resource::UserInputRequest,
        domain: agena_domain::UserInputRequest,
    ) {
        app.transcript.apply_execution(execution_with(
            vec![PendingInteractiveRequestResource {
                session_id: SESSION_ID,
                parent_session_id: None,
                task_id: None,
                request: PendingInteractiveRequest::UserInput {
                    request: wire.clone(),
                },
            }],
            vec![
                run_marker(),
                SessionTranscriptPart {
                    part_id: 5,
                    kind: "interaction".to_owned(),
                    role: "assistant".to_owned(),
                    state: "in_progress".to_owned(),
                    content: serde_json::to_value(RequestPartResource::UserInput {
                        request: wire,
                        reply: None,
                    })
                    .expect("request part serializes"),
                    summary: None,
                    created_at_ms: 50,
                    parent_part_id: None,
                    run_id: Some(3),
                },
            ],
        ));
        app.user_input_interactions.insert(
            REQUEST_ID.to_owned(),
            App::build_user_input_overlay(SESSION_ID, domain),
        );
        app.sync_interaction_documents();
        app.transcript.node_expansions.insert(node_key(), true);
        app.transcript.invalidate_render();
    }

    /// Position the transcript cursor on the interaction part's headline —
    /// the wizard activation boundary ("展开即可交互"): an expanded part whose
    /// headline is under the cursor is the active wizard, regardless of which
    /// body row the cursor would otherwise sit on.
    fn move_cursor_to_part_headline(app: &mut App) {
        let start_line = interaction_node_start(app);
        app.transcript
            .move_cursor_to_visual_line_number(WIDTH, HEIGHT, Some(start_line + 1));
    }

    /// The live wizard dialog, for presentation-state assertions.
    fn dialog<'a>(app: &'a App) -> &'a crate::app_types::UserInputOverlay {
        app.user_input_interactions
            .get(REQUEST_ID)
            .expect("the ask-user dialog is pending")
    }

    /// The projected interaction view, for wizard-page assertions.
    fn wizard_view<'a>(app: &'a App) -> &'a agena_tui_transcript::PendingInteractionView {
        app.transcript
            .interaction_views
            .get(REQUEST_ID)
            .expect("the ask-user view is projected")
    }

    /// Body offset of a question's first option row (its header row when it
    /// has no options) in the LIVE rendered continuous body. Computed from the
    /// projected layout so answered-preview rows growing earlier question
    /// blocks never drift the tests.
    fn q_landing_offset(app: &App, question_index: usize) -> usize {
        let view = wizard_view(app);
        let layouts = agena_tui_transcript::interaction_question_layouts(
            &dialog(app).request,
            &view.answers,
        );
        agena_tui_transcript::ask_user_question_landing_offset(
            view.plan_body_lines,
            &layouts,
            question_index,
        )
    }

    /// Body offset of a question's 其他 (custom) row in the live body.
    fn q_custom_row_offset(app: &App, question_index: usize) -> usize {
        let view = wizard_view(app);
        let layouts = agena_tui_transcript::interaction_question_layouts(
            &dialog(app).request,
            &view.answers,
        );
        let start = agena_tui_transcript::ask_user_question_body_start(
            view.plan_body_lines,
            &layouts,
            question_index,
        );
        start + 2 + layouts[question_index].options_len
    }

    #[tokio::test]
    async fn chat_navigation_keys_fall_through_the_thin_action_layer() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        // Sit on a decision row (body offset 2 = the option label).
        move_cursor_to_body_offset(&mut app, 2);

        // Line-wise motion (`j`/`k`/arrows) is deliberately OWNED while the
        // review part is pending: the decision cursor stays confined inside the
        // part block (see j_and_k_stay_within_the_pending_review_part). Every
        // other navigation key must NOT be intercepted — the chat keeps owning
        // horizontal motion, paging, jumps and prefixes ("everything is a
        // part", no injected review component). `g`/`G` are the motion/end
        // prefixes; `Space` pages.
        for key in [
            char_key('h'),
            char_key('l'),
            page_down(),
            KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            char_key('g'),
            char_key('G'),
        ] {
            assert!(
                !app.handle_active_interaction_action(key),
                "navigation key must fall through to normal transcript dispatch: {key:?}"
            );
        }
        // The interaction stays pending and editable after all those keys.
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
        assert_eq!(app.interaction_editing, None);
    }

    #[tokio::test]
    async fn j_and_k_stay_within_the_pending_review_part() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        move_cursor_to_body_offset(&mut app, 0);
        let (start_line, end_line) = {
            let node = app
                .transcript
                .rendered(WIDTH)
                .nodes
                .iter()
                .find(|node| node.key == node_key())
                .expect("the interaction part renders a node");
            (node.start_line, node.end_line)
        };

        // `k` from the first plan row must never escape onto the message
        // header's role-label column ("用户 / 助手"): the decision cursor stops
        // at the part headline (the part's own top boundary).
        app.handle_transcript_key(char_key('k'));
        assert!(
            app.transcript.navigation_cursor_line().is_some_and(|line| line >= start_line),
            "k must not move the cursor above the part headline"
        );

        // `j` from anywhere inside the part must never leave the block: the
        // cursor stops at the last decision/footer row before the next part.
        for _ in 0..10 {
            app.handle_transcript_key(char_key('j'));
            assert!(
                app.transcript
                    .navigation_cursor_line()
                    .is_some_and(|line| line < end_line),
                "j must keep the cursor inside the part block"
            );
        }
        // The part stays pending and interactive after all that motion.
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
    }

    #[tokio::test]
    async fn enter_on_a_plan_row_collapses_instead_of_submitting() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        move_cursor_to_body_offset(&mut app, 0);

        // Enter on the plan body is a plain Toggle: the thin layer declines it
        // (returns false), so the normal dispatch collapses the part. No
        // decision was taken.
        assert!(
            !app.handle_active_interaction_action(enter()),
            "Enter on a plan row must not be intercepted"
        );
        assert!(
            app.user_input_interactions.contains_key(REQUEST_ID),
            "no reply may be sent from the plan body"
        );
        app.handle_transcript_key(enter());
        assert_eq!(
            app.transcript.node_expansions.get(&node_key()),
            Some(&false),
            "Enter on a plan row toggles the part collapsed (normal chat behavior)"
        );
    }

    #[tokio::test]
    async fn enter_on_a_decision_row_submits_that_option() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        move_cursor_to_body_offset(&mut app, 2);

        assert!(
            app.handle_active_interaction_action(enter()),
            "Enter on the option row submits"
        );
        // The reply was sent: the interaction map is cleared (keep = false on
        // success) and the reply operation is in flight.
        assert!(!app.user_input_interactions.contains_key(REQUEST_ID));
        assert!(!app.transcript.interaction_views.contains_key(REQUEST_ID));
        assert!(app.run_activity.has_operation(
            RunActivityTarget::Session(SESSION_ID),
            RunOperation::UserInputReply,
        ));
    }

    #[tokio::test]
    async fn ctrl_x_cancels_and_keeps_the_part_pending() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        move_cursor_to_body_offset(&mut app, 2);

        assert!(
            app.handle_active_interaction_action(ctrl_x()),
            "Ctrl+X cancels the pending interaction"
        );
        // Cancel sends the reply but the dialog stays in the map until the
        // reply is acknowledged, so the part keeps rendering as pending.
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
        assert!(app.run_activity.has_operation(
            RunActivityTarget::Session(SESSION_ID),
            RunOperation::UserInputReply,
        ));
    }

    #[tokio::test]
    async fn e_on_the_custom_label_opens_the_inline_editor_and_esc_exits_it() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        move_cursor_to_body_offset(&mut app, 3);

        assert!(
            app.handle_active_interaction_action(edit()),
            "`e` on the custom feedback label begins the inline edit"
        );
        assert_eq!(
            app.interaction_editing.as_deref(),
            Some(REQUEST_ID),
            "the inline editor owns the key stream"
        );
        assert!(
            app.user_input_interactions
                .get(REQUEST_ID)
                .is_some_and(|dialog| dialog.presentation.review().is_editing_custom())
        );

        // Esc exits editing back to the part; the request stays pending.
        assert!(
            app.handle_active_interaction_action(esc()),
            "Esc is owned while the editor is open"
        );
        assert_eq!(app.interaction_editing, None);
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
        assert!(
            app.user_input_interactions
                .get(REQUEST_ID)
                .is_some_and(|dialog| !dialog.presentation.review().is_editing_custom())
        );
    }

    #[tokio::test]
    async fn esc_on_the_part_collapses_it_back_to_the_configured_state() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        move_cursor_to_body_offset(&mut app, 2);

        assert!(
            app.handle_active_interaction_action(esc()),
            "Esc with no pending motion prefix collapses the part"
        );
        assert_eq!(
            app.transcript.node_expansions.get(&node_key()),
            Some(&false),
            "the expansion override is cleared so the part falls back to its configured default"
        );
        assert!(
            app.user_input_interactions.contains_key(REQUEST_ID),
            "the request stays reachable by re-expanding the part"
        );
    }

    #[tokio::test]
    async fn reveal_pending_user_input_interaction_expands_and_focuses_on_arrival() {
        let mut app = seeded_app().await;
        app.transcript
            .apply_execution(execution_with(vec![pending_user_input_resource()], parts()));
        app.user_input_interactions.insert(
            REQUEST_ID.to_owned(),
            App::build_user_input_overlay(SESSION_ID, domain_request()),
        );
        app.sync_interaction_documents();

        // Not expanded yet: the reveal is what forces expansion + focus,
        // regardless of the configured default.
        assert!(!app.transcript.node_expansions.contains_key(&node_key()));

        app.reveal_pending_user_input_interaction(REQUEST_ID);

        assert_eq!(
            app.transcript.node_expansions.get(&node_key()),
            Some(&true),
            "a pending interaction part always auto-expands on arrival"
        );
        assert!(app.revealed_user_input_request_ids.contains(REQUEST_ID));
        let start_line = interaction_node_start(&mut app);
        // `move_cursor_to_visual_line_number` is 1-indexed; the reveal targets
        // `start_line + 1` (1-indexed), which is the part's headline row
        // (0-indexed `start_line`).
        assert_eq!(
            app.transcript.navigation_cursor_line(),
            Some(start_line),
            "the transcript cursor lands on the revealed part"
        );
    }

    #[tokio::test]
    async fn reveal_scrolls_so_the_whole_question_page_is_visible() {
        let mut app = seeded_app().await;
        // A plan body tall enough to overflow the 24-row viewport: the reveal
        // must fit-scroll the ENTIRE expanded part, not just show its top.
        let tall_plan = (0..40)
            .map(|i| format!("- step {i} of the review plan"))
            .collect::<Vec<_>>()
            .join("\n");
        seed_pending_review_with_plan(&mut app, &tall_plan);

        app.reveal_pending_user_input_interaction(REQUEST_ID);

        let (start_line, end_line) = {
            let node = app
                .transcript
                .rendered(WIDTH)
                .nodes
                .iter()
                .find(|node| node.key == node_key())
                .expect("the interaction part renders a node");
            (node.start_line, node.end_line)
        };
        // The cursor lands on the part's headline (0-indexed `start_line`).
        assert_eq!(
            app.transcript.navigation_cursor_line(),
            Some(start_line),
            "the reveal lands the cursor on the part headline"
        );
        // The whole question page fits: the part's last body row is inside the
        // viewport (bottom-anchored for a page taller than the screen), and
        // the viewport never scrolls past the part's top.
        let height = usize::from(HEIGHT);
        assert!(
            end_line <= app.transcript.viewport_top() + height,
            "the entire expanded page must fit in the viewport"
        );
        assert!(
            app.transcript.viewport_top() <= start_line,
            "the fit-scroll must not push the part's top above the viewport"
        );
    }

    #[tokio::test]
    async fn reveal_auto_expands_the_tool_call_operation_that_carries_the_ask() {
        let mut app = seeded_app().await;
        // The canonical single-activity shape: the ask lives on the tool_call
        // operation, so the reveal must resolve ITS node key — not the legacy
        // `interaction` part. This is the pre-approval auto-expand behavior.
        app.transcript.apply_execution(execution_with(
            vec![pending_user_input_resource()],
            vec![run_marker(), operation_tool_call_part()],
        ));
        app.user_input_interactions.insert(
            REQUEST_ID.to_owned(),
            App::build_user_input_overlay(SESSION_ID, domain_request()),
        );
        app.sync_interaction_documents();
        app.transcript.invalidate_render();

        let key = app
            .pending_interaction_part_node_key(REQUEST_ID)
            .expect("the operation activity is the pending interaction part");
        assert_eq!(
            key,
            TranscriptNodeKey::Activity {
                entry_id: TranscriptEntryId::StoredMessage(3),
                content_id: TranscriptContentId::StoredPart(5),
            }
        );

        app.reveal_pending_user_input_interaction(REQUEST_ID);
        assert_eq!(
            app.transcript.node_expansions.get(&key),
            Some(&true),
            "a pending ask auto-expands the operation activity on arrival"
        );
        assert!(
            app.transcript
                .navigation_cursor_line()
                .is_some_and(|line| {
                    app.transcript
                        .rendered(WIDTH)
                        .nodes
                        .iter()
                        .find(|node| node.key == key)
                        .is_some_and(|node| line >= node.start_line && line < node.end_line)
                }),
            "the reveal lands the cursor inside the operation activity"
        );
    }

    #[tokio::test]
    async fn reveal_retries_once_the_part_populates() {
        let mut app = seeded_app().await;
        // The execution snapshot arrives before the transcript parts populate:
        // the reveal no-ops because there is no part node yet.
        app.transcript
            .apply_execution(execution_with(vec![pending_user_input_resource()], vec![]));

        app.reveal_outstanding_pending_user_input_interactions();
        assert!(
            app.revealed_user_input_request_ids.is_empty(),
            "a missing part must not be recorded as revealed"
        );
        assert!(!app.transcript.node_expansions.contains_key(&node_key()));

        // Once the parts are present (and the live interaction is staged), the
        // outstanding retry reveals exactly once and never re-yanks.
        app.transcript.merge_parts(parts());
        app.user_input_interactions.insert(
            REQUEST_ID.to_owned(),
            App::build_user_input_overlay(SESSION_ID, domain_request()),
        );
        app.sync_interaction_documents();

        app.reveal_outstanding_pending_user_input_interactions();
        app.reveal_outstanding_pending_user_input_interactions();

        assert!(app.revealed_user_input_request_ids.contains(REQUEST_ID));
        assert_eq!(
            app.transcript.node_expansions.get(&node_key()),
            Some(&true),
            "the deferred reveal expands the part"
        );
    }

    #[tokio::test]
    async fn handle_user_input_replied_clears_map_override_and_revealed_guard() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        app.revealed_user_input_request_ids
            .insert(REQUEST_ID.to_owned());
        assert_eq!(app.transcript.node_expansions.get(&node_key()), Some(&true));

        // The replied execution is terminal and no longer carries the request.
        app.handle_user_input_replied(
            SESSION_ID,
            REQUEST_ID.to_owned(),
            Ok(execution_with(vec![], parts())),
        );

        assert!(!app.user_input_interactions.contains_key(REQUEST_ID));
        assert!(!app.revealed_user_input_request_ids.contains(REQUEST_ID));
        assert!(
            !app.transcript.node_expansions.contains_key(&node_key()),
            "the part returns to its configured expand/collapse default"
        );
    }

    #[tokio::test]
    async fn moving_the_cursor_onto_decision_rows_syncs_the_selection() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);

        // Cursor on the plan body: plan rows are inert (Enter collapses, not
        // submits — covered by the routing test), so the selection keeps the
        // presentation's default rather than deriving a new decision.
        move_cursor_to_body_offset(&mut app, 0);
        app.refresh_interaction_selection(WIDTH);
        assert_eq!(
            app.transcript
                .interaction_views
                .get(REQUEST_ID)
                .and_then(|view| view.selected_option),
            Some(0),
            "plan rows are not decision rows; the seeded default is untouched"
        );
        assert!(
            app.transcript.rendered.is_some(),
            "no selection change on a plan row means no re-render"
        );

        // Cursor onto the custom feedback label (body offset 3 = the custom
        // label, the option detail row was removed by the one-row-per-option
        // layout contract): the selection follows the cursor to the custom
        // slot (options_len == 1), and the render cache is invalidated so the
        // marker moves.
        move_cursor_to_body_offset(&mut app, 3);
        app.refresh_interaction_selection(WIDTH);
        assert_eq!(
            app.transcript
                .interaction_views
                .get(REQUEST_ID)
                .and_then(|view| view.selected_option),
            Some(1)
        );
        assert!(
            app.transcript.rendered.is_none(),
            "selection change invalidates the render cache"
        );

        // Moving back onto the option label (body offset 2) follows the cursor
        // back to the first option and invalidates again.
        move_cursor_to_body_offset(&mut app, 2);
        app.refresh_interaction_selection(WIDTH);
        assert_eq!(
            app.transcript
                .interaction_views
                .get(REQUEST_ID)
                .and_then(|view| view.selected_option),
            Some(0)
        );
        assert!(
            app.transcript.rendered.is_none(),
            "selection change invalidates the render cache"
        );
    }

    #[tokio::test]
    async fn paste_into_the_part_begins_the_inline_edit_and_owns_the_stream() {
        let mut app = seeded_app().await;
        seed_pending_review(&mut app);
        move_cursor_to_body_offset(&mut app, 2);

        assert!(app.paste_into_active_interaction("looks good"));
        assert_eq!(
            app.interaction_editing.as_deref(),
            Some(REQUEST_ID),
            "pasting auto-begins the inline edit"
        );
        assert_eq!(
            app.user_input_interactions
                .get(REQUEST_ID)
                .map(|dialog| dialog.presentation.review().custom_text()),
            Some("looks good".to_owned())
        );

        // The editor owns the stream: Enter commits the non-empty feedback and
        // submits the reply.
        assert!(
            app.handle_active_interaction_action(enter()),
            "Enter in the inline editor submits"
        );
        assert!(!app.user_input_interactions.contains_key(REQUEST_ID));
        assert!(app.run_activity.has_operation(
            RunActivityTarget::Session(SESSION_ID),
            RunOperation::UserInputReply,
        ));
    }

    #[tokio::test]
    async fn ask_user_up_down_are_free_transcript_motion() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        move_cursor_to_part_headline(&mut app);

        let start_line = interaction_node_start(&mut app);
        // Up/Down are NOT owned by the ask part: the transcript cursor is a
        // normal line-by-line cursor (the two-cursor bug fix).
        assert!(
            !app.handle_active_interaction_action(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            "Up must not be intercepted by the ask part"
        );
        assert!(
            !app.handle_active_interaction_action(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            "Down must not be intercepted by the ask part"
        );
        // The normal transcript dispatch moves the cursor one line at a time...
        app.handle_transcript_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.transcript.navigation_cursor_line(),
            Some(start_line + 1),
            "Down is free transcript motion"
        );
        // ...and from the part top, Up leaves the part upward — the core bug
        // fix: the cursor is never trapped inside the ask part.
        app.handle_transcript_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        app.handle_transcript_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert!(
            app.transcript
                .navigation_cursor_line()
                .is_some_and(|line| line < start_line),
            "Up from the part top must leave the part upward"
        );
        // The part stays pending and interactive after all that free motion.
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
    }

    #[tokio::test]
    async fn ask_user_left_right_jump_between_questions() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        move_cursor_to_part_headline(&mut app);

        let start_line = interaction_node_start(&mut app);
        // Left with the cursor on the plan area (before Q0) is a no-op but
        // still consumed.
        assert!(
            app.handle_active_interaction_action(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            "Left is owned while the cursor is on the part"
        );
        assert_eq!(
            app.transcript.navigation_cursor_line(),
            Some(start_line),
            "Left before the first question is a no-op"
        );

        // Right from the plan area jumps to Q0's first option row.
        assert!(
            app.handle_active_interaction_action(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            "Right is owned while the cursor is on the part"
        );
        assert_eq!(
            app.transcript.navigation_cursor_line(),
            Some(start_line + 1 + q_landing_offset(&app, 0)),
            "Right jumps to Q0's first option row"
        );

        // Right again jumps to Q1's first option row.
        assert!(app.handle_active_interaction_action(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)));
        assert_eq!(
            app.transcript.navigation_cursor_line(),
            Some(start_line + 1 + q_landing_offset(&app, 1)),
            "Right jumps to Q1's first option row"
        );

        // Right at the last question is a no-op but still consumed.
        assert!(app.handle_active_interaction_action(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)));
        assert_eq!(
            app.transcript.navigation_cursor_line(),
            Some(start_line + 1 + q_landing_offset(&app, 1)),
            "Right at the last question is a no-op"
        );
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
    }

    #[tokio::test]
    async fn ask_user_continuous_body_is_taller_than_viewport_and_down_reaches_its_bottom() {
        let mut app = seeded_app().await;
        // Give the SECOND question 30 options so the continuous body is far
        // taller than the 24-row viewport.
        let tall_options: Vec<agena_api::resource::UserInputOption> = (0..30)
            .map(|index| agena_api::resource::UserInputOption {
                label: format!("Option {index}"),
                description: String::new(),
            })
            .collect();
        let mut wire = ask_user_wire_request();
        wire.questions[1].options = tall_options.clone();
        let mut domain = ask_user_domain_request();
        domain.questions[1].options = tall_options
            .iter()
            .map(|option| agena_domain::UserInputOption {
                label: option.label.clone(),
                description: option.description.clone(),
            })
            .collect();
        seed_pending_ask_user_with(&mut app, wire, domain);
        move_cursor_to_part_headline(&mut app);

        let height = usize::from(HEIGHT);
        let (start_line, end_line) = {
            let rendered = app.transcript.rendered(WIDTH);
            let node = rendered
                .nodes
                .iter()
                .find(|node| node.key == node_key())
                .expect("the ask part renders a node");
            (node.start_line, node.end_line)
        };
        assert!(
            end_line.saturating_sub(start_line) > height,
            "the continuous body must exceed the viewport for this test to prove free motion"
        );

        // Free Down (transcript dispatch) walks the cursor to the last body
        // row — the footer key-hint.
        let mut cursor = app
            .transcript
            .navigation_cursor_line()
            .expect("the cursor is inside the part");
        while cursor + 1 < end_line {
            app.handle_transcript_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            let next = app
                .transcript
                .navigation_cursor_line()
                .expect("the cursor stays on a line");
            assert!(next > cursor, "Down must keep moving the cursor");
            cursor = next;
        }
        assert_eq!(cursor, end_line - 1, "free Down reaches the footer");
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
    }

    #[tokio::test]
    async fn ask_user_space_toggles_the_option_under_the_cursor() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        let q0_landing = q_landing_offset(&app, 0);
        move_cursor_to_body_offset(&mut app, q0_landing);

        // Space selects the option under the cursor (single-pick Q0 option 0).
        assert!(
            app.handle_active_interaction_action(space_key()),
            "Space on an option row is owned"
        );
        assert_eq!(
            dialog(&app)
                .presentation
                .answer(0)
                .map(|answer| answer.option_indexes.iter().copied().collect::<Vec<_>>()),
            Some(vec![0]),
            "Space selects the option under the cursor"
        );
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));

        // Space again (single-pick) unselects it: select/unselect.
        assert!(app.handle_active_interaction_action(space_key()));
        assert_eq!(
            dialog(&app).presentation.answer(0).map(|answer| answer.option_indexes.len()),
            Some(0),
            "single-pick Space unselects the already-picked option"
        );
        assert!(!app.run_activity.has_operation(
            RunActivityTarget::Session(SESSION_ID),
            RunOperation::UserInputReply,
        ));

        // PgDn is NOT owned by the ask part — it falls through to paging.
        assert!(
            !app.handle_active_interaction_action(page_down()),
            "PgDn must not be intercepted by the ask part"
        );
    }

    #[tokio::test]
    async fn ask_user_enter_on_answered_option_submits_the_request() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        // Answer Q0 with Space, then Enter on Q1's option row: Enter checks it
        // (multi-pick toggles it on) AND submits — every question is answered.
        // The option offset is computed from the live layout after Q0 is
        // answered (Q0's block grows an answered-preview row).
        let q0_landing = q_landing_offset(&app, 0);
        move_cursor_to_body_offset(&mut app, q0_landing);
        app.handle_active_interaction_action(space_key());
        let q1_landing = q_landing_offset(&app, 1);
        move_cursor_to_body_offset(&mut app, q1_landing);

        assert!(
            app.handle_active_interaction_action(enter()),
            "Enter on an option row is owned"
        );
        assert!(
            !app.user_input_interactions.contains_key(REQUEST_ID),
            "submitting drops the dialog"
        );
        assert!(
            !app.transcript.interaction_views.contains_key(REQUEST_ID),
            "the part stops rendering the stale pending body"
        );
        assert!(app.run_activity.has_operation(
            RunActivityTarget::Session(SESSION_ID),
            RunOperation::UserInputReply,
        ));
    }

    #[tokio::test]
    async fn ask_user_enter_validation_jumps_to_first_unanswered_question() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        // Answer only Q0 (Vanilla), leave Q1 unanswered.
        let q0_landing = q_landing_offset(&app, 0);
        move_cursor_to_body_offset(&mut app, q0_landing);
        app.handle_active_interaction_action(space_key());

        // Enter on Q0's option row: Q0 is answered but Q1 is not, so no reply
        // is sent and the cursor jumps to the first unanswered question (Q1).
        assert!(
            app.handle_active_interaction_action(enter()),
            "Enter on an option row is owned"
        );
        assert!(!app.run_activity.has_operation(
            RunActivityTarget::Session(SESSION_ID),
            RunOperation::UserInputReply,
        ));
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
        let start_line = interaction_node_start(&mut app);
        assert_eq!(
            app.transcript.navigation_cursor_line(),
            Some(start_line + 1 + q_landing_offset(&app, 1)),
            "the cursor lands on the first unanswered question's first option row"
        );
    }

    #[tokio::test]
    async fn ask_user_enter_on_plan_row_collapses_instead_of_submitting() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        move_cursor_to_body_offset(&mut app, 0);

        // Enter on the plan body is a plain Toggle: the thin layer declines it
        // (returns false), so the normal dispatch collapses the part. No
        // answer was submitted.
        assert!(
            !app.handle_active_interaction_action(enter()),
            "Enter on a plan row must not be intercepted"
        );
        assert!(
            app.user_input_interactions.contains_key(REQUEST_ID),
            "no reply may be sent from the plan body"
        );
        app.handle_transcript_key(enter());
        assert_eq!(
            app.transcript.node_expansions.get(&node_key()),
            Some(&false),
            "Enter on a plan row toggles the part collapsed (normal chat behavior)"
        );
    }

    #[tokio::test]
    async fn ask_user_space_on_the_custom_row_opens_the_inline_editor() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        let custom_row = q_custom_row_offset(&app, 0);
        move_cursor_to_body_offset(&mut app, custom_row);

        assert!(
            app.handle_active_interaction_action(space_key()),
            "Space on the custom row is owned"
        );
        assert_eq!(
            app.interaction_editing.as_deref(),
            Some(REQUEST_ID),
            "the inline editor owns the key stream"
        );
        assert!(
            app.user_input_interactions
                .get(REQUEST_ID)
                .is_some_and(|dialog| dialog.presentation.is_editing_custom())
        );
    }

    #[tokio::test]
    async fn ask_user_e_does_not_open_the_custom_editor() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        let custom_row = q_custom_row_offset(&app, 0);
        move_cursor_to_body_offset(&mut app, custom_row);

        // `e` is deliberately NOT owned by the ask part: the hint is wrong and
        // `e` must never open the custom editor.
        assert!(
            !app.handle_active_interaction_action(edit()),
            "`e` must NOT be intercepted by the ask part"
        );
        assert_eq!(
            app.interaction_editing, None,
            "`e` never opens the inline custom editor"
        );
        assert!(
            !app.user_input_interactions
                .get(REQUEST_ID)
                .is_some_and(|dialog| dialog.presentation.is_editing_custom())
        );
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
    }

    #[tokio::test]
    async fn ask_user_enter_on_custom_row_submits_committed_text_or_opens_the_editor() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        let custom_row = q_custom_row_offset(&app, 0);
        move_cursor_to_body_offset(&mut app, custom_row);

        // Empty custom: Enter opens the inline editor (no submit yet).
        assert!(
            app.handle_active_interaction_action(enter()),
            "Enter on an empty custom row is owned"
        );
        assert_eq!(
            app.interaction_editing.as_deref(),
            Some(REQUEST_ID),
            "Enter on an empty custom row opens the editor"
        );
        // Type into the editor, then Enter commits the custom text and closes
        // the editor (single-pick advances the presentation to Q1).
        assert!(app.paste_into_active_interaction("mint"));
        assert!(app.handle_active_interaction_action(enter()));
        assert_eq!(
            dialog(&app).presentation.answer(0).map(|answer| answer.custom_values.clone()),
            Some(vec!["mint".to_owned()]),
            "the editor commit stores the custom value"
        );
        assert_eq!(app.interaction_editing, None, "the editor closed");
        assert!(!app.run_activity.has_operation(
            RunActivityTarget::Session(SESSION_ID),
            RunOperation::UserInputReply,
        ));

        // Answer Q1 with Space, then Enter on Q0's custom row: the custom text
        // is committed and every question is answered, so Enter submits.
        let q1_landing = q_landing_offset(&app, 1);
        move_cursor_to_body_offset(&mut app, q1_landing);
        app.handle_active_interaction_action(space_key());
        let custom_row = q_custom_row_offset(&app, 0);
        move_cursor_to_body_offset(&mut app, custom_row);
        assert!(
            app.handle_active_interaction_action(enter()),
            "Enter on a custom row with committed text submits"
        );
        assert!(!app.user_input_interactions.contains_key(REQUEST_ID));
        assert!(app.run_activity.has_operation(
            RunActivityTarget::Session(SESSION_ID),
            RunOperation::UserInputReply,
        ));
    }

    #[tokio::test]
    async fn active_ask_part_cursor_line_highlights_only_the_expanded_ask_body() {
        // Ask part: the cursor line inside the body is the whole-line highlight.
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        let q0_landing = q_landing_offset(&app, 0);
        move_cursor_to_body_offset(&mut app, q0_landing);
        let start_line = interaction_node_start(&mut app);
        let highlighted = app.active_ask_part_cursor_line(WIDTH);
        assert_eq!(
            highlighted,
            Some(start_line + 1 + q0_landing),
            "the cursor line inside the ask body is the whole-line highlight"
        );
        // Off the part: no whole-line highlight.
        app.transcript
            .move_cursor_to_visual_line_number(WIDTH, HEIGHT, Some(1));
        assert_eq!(
            app.active_ask_part_cursor_line(WIDTH),
            None,
            "off the part there is no whole-line highlight"
        );
        // Review part: the whole-line highlight does NOT apply — it keeps the
        // normal single-cell cursor.
        let mut review_app = seeded_app().await;
        seed_pending_review(&mut review_app);
        move_cursor_to_body_offset(&mut review_app, 2);
        assert_eq!(
            review_app.active_ask_part_cursor_line(WIDTH),
            None,
            "review keeps the normal single-cell cursor"
        );
    }

    #[tokio::test]
    async fn ask_user_esc_falls_through_out_of_wizard_mode() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        move_cursor_to_part_headline(&mut app);

        // Esc is deliberately NOT owned by the ask-user wizard: it falls
        // through to the normal transcript dispatch, so the part stays
        // expanded, the wizard stays active, and the request stays pending.
        assert!(
            !app.handle_active_interaction_action(esc()),
            "Esc must not be intercepted by the ask-user wizard"
        );
        assert_ne!(
            app.transcript.node_expansions.get(&node_key()),
            Some(&false),
            "Esc must not collapse the expanded part out from under the wizard"
        );
        assert!(
            app.user_input_interactions.contains_key(REQUEST_ID),
            "the request stays pending"
        );
    }

    #[tokio::test]
    async fn ask_user_keys_are_not_intercepted_off_the_part() {
        let mut app = seeded_app().await;
        seed_pending_ask_user(&mut app);
        move_cursor_to_part_headline(&mut app);

        // Park the cursor on the entry header (line 0), off the part.
        app.transcript
            .move_cursor_to_visual_line_number(WIDTH, HEIGHT, Some(1));
        assert!(
            app.active_user_input_interaction_request_id().is_none(),
            "the cursor must be off the part for this test"
        );

        // No wizard key may be intercepted off the part; the chat keeps
        // owning navigation (`h`/`l` move the transcript cursor).
        for key in [
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            enter(),
            space_key(),
        ] {
            assert!(
                !app.handle_active_interaction_action(key),
                "wizard key must not be intercepted off the part: {key:?}"
            );
        }
        let before = app
            .transcript
            .cursor_text_position(WIDTH)
            .map(|position| position.column);
        app.handle_transcript_key(char_key('l'));
        assert_ne!(
            app.transcript.cursor_text_position(WIDTH).map(|position| position.column),
            before,
            "plain chat navigation still moves the cursor within the line"
        );
        assert!(app.user_input_interactions.contains_key(REQUEST_ID));
    }
}

#[cfg(test)]
mod transcript_character_cursor_tests {
    use super::super::{
        ExecutionStatus, RunResource, RunRole, RunStatus, TranscriptFixture,
        TranscriptMoveDirection, TranscriptState, TranscriptTextPosition, Utc,
    };
    use super::parts_fixtures;
    use unicode_width::UnicodeWidthStr;

    fn message(id: i64, text: &str) -> RunResource {
        message_with_role(id, RunRole::Assistant, text)
    }

    fn message_with_role(id: i64, role: RunRole, text: &str) -> RunResource {
        let now = Utc::now();
        RunResource {
            id,
            session_id: 7,
            role,
            state: RunStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![TranscriptFixture::text_part(
                id * 10,
                id,
                now,
                ExecutionStatus::Completed,
                text,
            )]),
        }
    }

    fn display_column_before(text: &str, marker: &str) -> usize {
        let byte_index = text.find(marker).expect("test marker must be rendered");
        UnicodeWidthStr::width(&text[..byte_index])
    }

    #[test]
    fn normal_mode_cursor_moves_one_grapheme_and_restores_its_column_after_short_rows() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "a界🙂z\n\nq\n\nabcdefgh")],
            ..TranscriptState::default()
        };
        let rows = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.text.contains("a界🙂z"))
            .map(|(index, line)| (index, display_column_before(line.text.as_str(), "a界🙂z")))
            .collect::<Vec<_>>();
        let (first_line, first_column) = *rows.first().expect("first source row");
        let short_line = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(index, line)| {
                line.text
                    .contains("q")
                    .then(|| (index, display_column_before(line.text.as_str(), "q")))
            })
            .expect("short source row");
        let long_line = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(index, line)| {
                line.text
                    .contains("abcdefgh")
                    .then(|| (index, display_column_before(line.text.as_str(), "abcdefgh")))
            })
            .expect("later source row");

        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first_line,
                column: 0,
            },
        );
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column..first_column + 1)),
            "the character cursor must clamp past layout-only indentation"
        );

        let header_line = transcript
            .rendered(80)
            .lines
            .iter()
            .position(|line| line.text.starts_with("assistant"))
            .expect("assistant role header");
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: header_line,
                column: 0,
            },
        );
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((header_line, 0..1)),
            "role labels are selectable transcript text"
        );

        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first_line,
                column: first_column,
            },
        );
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column..first_column + 1))
        );

        transcript.move_cursor_horizontally(80, 20, true, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column + 1..first_column + 3))
        );
        transcript.move_cursor_horizontally(80, 20, true, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column + 3..first_column + 5))
        );
        transcript.move_cursor_horizontally(80, 20, false, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((first_line, first_column + 1..first_column + 3))
        );

        // Reset onto the long row, then verify Vim-style desired-column
        // preservation through the one-character middle row.
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first_line,
                column: first_column + 6,
            },
        );
        transcript.move_cursor_by_visual_lines(80, 20, TranscriptMoveDirection::Down, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((short_line.0, short_line.1..short_line.1 + 1))
        );
        transcript.move_cursor_by_visual_lines(80, 20, TranscriptMoveDirection::Down, 1);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((long_line.0, long_line.1 + 6..long_line.1 + 7))
        );
    }

    #[test]
    fn visual_character_and_line_modes_extend_the_cursor_range_like_vim() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "a界🙂z\n\nq\n\nabcdefgh")],
            ..TranscriptState::default()
        };
        let first = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| {
                rendered.text.contains("a界🙂z").then(|| {
                    (
                        line,
                        display_column_before(rendered.text.as_str(), "a界🙂z"),
                    )
                })
            })
            .expect("first source row");
        let second = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| {
                rendered
                    .text
                    .contains("q")
                    .then(|| (line, display_column_before(rendered.text.as_str(), "q")))
            })
            .expect("second source row");

        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first.0,
                column: first.1,
            },
        );
        transcript.toggle_visual_selection(
            80,
            20,
            super::super::TranscriptVisualSelectionMode::Character,
        );
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(first.0)),
            Some(first.1..first.1 + 1)
        );
        transcript.move_cursor_horizontally(80, 20, true, 1);
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(first.0)),
            Some(first.1..first.1 + 3),
            "v plus l selects the next complete wide grapheme"
        );

        transcript.toggle_visual_selection(
            80,
            20,
            super::super::TranscriptVisualSelectionMode::Line,
        );
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(first.0)),
            Some(0..usize::MAX)
        );
        transcript.move_cursor_by_visual_lines(80, 20, TranscriptMoveDirection::Down, 1);
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(first.0)),
            Some(0..usize::MAX)
        );
        assert_eq!(
            transcript
                .text_selection()
                .and_then(|selection| selection.cell_range_for_line(second.0)),
            Some(0..usize::MAX),
            "V plus j selects every complete rendered row between anchor and cursor"
        );

        transcript.cancel_text_selection(80, 20);
        assert!(!transcript.has_visual_selection());
        assert_eq!(transcript.text_selection(), None);

        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first.0,
                column: first.1,
            },
        );
        transcript.toggle_visual_selection(
            80,
            20,
            super::super::TranscriptVisualSelectionMode::Block,
        );
        transcript.move_cursor_by_visual_lines(80, 20, TranscriptMoveDirection::Down, 1);
        let block_ranges = transcript.selection_cell_ranges(80);
        assert_eq!(block_ranges[first.0], Some(first.1..first.1 + 1));
        assert_eq!(block_ranges[second.0], Some(first.1..first.1 + 1));
        assert_eq!(
            transcript.selected_text(80, ""),
            Some("a\n\nq".to_string()),
            "Ctrl+V copies the selected terminal-cell rectangle without card chrome"
        );
    }

    #[test]
    fn ctrl_message_motion_skips_to_the_adjacent_message_without_block_highlighting() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "first message"), message(2, "second message")],
            ..TranscriptState::default()
        };
        let first = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| rendered.text.contains("first message").then_some(line))
            .expect("first message line");
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first,
                column: 0,
            },
        );

        transcript.move_cursor_by_messages(80, 20, TranscriptMoveDirection::Down, 1);

        let (line, _) = transcript.cursor_cell_range(80).expect("character cursor");
        assert!(
            transcript.rendered(80).lines[line]
                .text
                .contains("assistant")
        );
        assert_eq!(transcript.highlighted_block_key(), None);
    }

    #[test]
    fn vim_word_line_find_and_reselect_motions_keep_character_cursor_semantics() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "  one,two three")],
            ..TranscriptState::default()
        };
        let (line, column) = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| {
                rendered.text.contains("one,two three").then(|| {
                    (
                        line,
                        display_column_before(rendered.text.as_str(), "one,two three"),
                    )
                })
            })
            .expect("rendered text line");
        transcript.select_pointer_line(80, 20, TranscriptTextPosition { line, column });

        transcript.move_cursor_to_line_start(80, 20, true);
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((line, column..column + 1))
        );

        transcript.move_cursor_by_words(80, 20, true, false, false, 1);
        let comma_column = display_column_before(
            transcript.rendered(80).lines[line].text.as_str(),
            ",two three",
        );
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((line, comma_column..comma_column + 1)),
            "w stops at punctuation, matching Vim's keyword-word split"
        );

        transcript.move_cursor_to_find(80, 20, true, false, 't', 2);
        let second_t =
            display_column_before(transcript.rendered(80).lines[line].text.as_str(), "three");
        assert_eq!(
            transcript.cursor_cell_range(80),
            Some((line, second_t..second_t + 1))
        );

        transcript.toggle_visual_selection(
            80,
            20,
            super::super::TranscriptVisualSelectionMode::Character,
        );
        transcript.move_cursor_to_line_end(80, 20);
        let selected = transcript.selected_text(80, "").expect("Visual text");
        assert_eq!(selected, "three");
        transcript.cancel_text_selection(80, 20);
        transcript.reselect_last_visual_selection(80, 20);
        assert_eq!(transcript.selected_text(80, ""), Some("three".to_string()));

        let two_column = display_column_before(
            transcript.rendered(80).lines[line].text.as_str(),
            "two three",
        );
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line,
                column: two_column,
            },
        );
        assert!(transcript.select_current_word_text_object(80, 20, false));
        assert_eq!(transcript.selected_text(80, ""), Some("two".to_string()));
        transcript.cancel_text_selection(80, 20);
        assert!(transcript.select_current_word_text_object(80, 20, true));
        assert_eq!(transcript.selected_text(80, ""), Some("two ".to_string()));
    }

    #[test]
    fn markdown_and_message_text_objects_select_semantic_ranges() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![message(1, "first paragraph\n\nsecond paragraph")],
            ..TranscriptState::default()
        };
        let first_line = transcript
            .rendered(80)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| rendered.text.contains("first paragraph").then_some(line))
            .expect("first markdown block");
        transcript.select_pointer_line(
            80,
            20,
            TranscriptTextPosition {
                line: first_line,
                column: 0,
            },
        );
        assert!(transcript.select_current_text_object(80, 20, false));
        assert_eq!(
            transcript.selected_text(80, ""),
            Some("first paragraph".to_string())
        );

        transcript.cancel_text_selection(80, 20);
        assert!(transcript.select_current_text_object(80, 20, true));
        let message = transcript
            .selected_text(80, "")
            .expect("message Visual range");
        assert!(message.contains("first paragraph"));
        assert!(message.contains("second paragraph"));
    }

    #[test]
    fn cancelled_reply_activity_is_rendered_after_its_user_turn_as_an_assistant_outcome() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "user", "completed"),
                parts_fixtures::text(2, "user", "please answer"),
                parts_fixtures::run(3, "assistant", "cancelled"),
                parts_fixtures::text(4, "assistant", "partial assistant reply"),
            ],
            ..TranscriptState::default()
        };
        let lines = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        let user = lines
            .iter()
            .position(|line| line.contains("please answer"))
            .expect("user message");
        let response = lines
            .iter()
            .position(|line| line.contains("partial assistant reply"))
            .expect("assistant reply");
        let cancelled = lines
            .iter()
            .position(|line| line.contains("Response cancelled"))
            .expect("cancelled outcome");

        assert!(user < response && response < cancelled, "{lines:#?}");
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("Response cancelled"))
                .count(),
            1
        );
        assert!(lines.iter().any(|line| line.starts_with("assistant –")));
        assert!(lines.iter().all(|line| !line.starts_with("system –")));
        assert_eq!(
            lines[cancelled], "  ▸ – Response cancelled",
            "a response outcome must use the visible Activity headline contract"
        );
        // The cancelled run marker (part_id 3) projects to an Activity node
        // keyed by its part id in the v2 parts model (database-design-v2.md
        // §4.1.1), rendered just after its user turn.
        assert!(transcript.rendered(80).nodes.iter().any(|node| {
            matches!(
                node.key,
                agena_tui_transcript::TranscriptNodeKey::Activity {
                    content_id: agena_tui_transcript::TranscriptContentId::StoredPart(3),
                    ..
                }
            ) && node.kind == agena_tui_transcript::TranscriptNodeKind::Activity
        }));
    }

    #[test]
    fn cancelled_reply_activity_never_moves_across_a_later_user_turn() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "user", "completed"),
                parts_fixtures::text(2, "user", "cancel this turn"),
                parts_fixtures::run(3, "assistant", "cancelled"),
                parts_fixtures::run(4, "user", "completed"),
                parts_fixtures::text(5, "user", "the next turn"),
                parts_fixtures::run(6, "assistant", "completed"),
                parts_fixtures::text(7, "assistant", "next answer"),
            ],
            ..TranscriptState::default()
        };
        let lines = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        let first_user = lines
            .iter()
            .position(|line| line.contains("cancel this turn"))
            .expect("first user");
        let cancelled = lines
            .iter()
            .position(|line| line.contains("Response cancelled"))
            .expect("cancelled outcome");
        let second_user = lines
            .iter()
            .position(|line| line.contains("the next turn"))
            .expect("later user");

        assert!(
            first_user < cancelled && cancelled < second_user,
            "{lines:#?}"
        );
        assert!(lines[cancelled.saturating_sub(1)].starts_with("assistant –"));
    }

    #[test]
    fn delayed_cancellations_stay_with_their_original_user_turns() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "user", "completed"),
                parts_fixtures::text(2, "user", "first turn"),
                parts_fixtures::run(3, "assistant", "cancelled"),
                parts_fixtures::run(4, "user", "completed"),
                parts_fixtures::text(5, "user", "second turn"),
                parts_fixtures::run(6, "assistant", "cancelled"),
            ],
            ..TranscriptState::default()
        };

        let lines = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        let first_user = lines
            .iter()
            .position(|line| line.contains("first turn"))
            .expect("first user message");
        let second_user = lines
            .iter()
            .position(|line| line.contains("second turn"))
            .expect("second user message");
        let cancellations = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| line.contains("Response cancelled").then_some(index))
            .collect::<Vec<_>>();

        assert_eq!(cancellations.len(), 2, "{lines:#?}");
        assert!(
            first_user < cancellations[0]
                && cancellations[0] < second_user
                && second_user < cancellations[1],
            "{lines:#?}"
        );
    }
}

#[cfg(test)]
mod prompt_history_tests {
    use super::super::{App, Editor, PromptHistory, PromptHistorySearchState, SearchPickerConfig};

    #[test]
    fn history_search_indexes_the_bounded_catalog_newest_first() {
        let history = PromptHistory {
            items: (0..55).map(|index| format!("prompt {index:02}")).collect(),
        };
        let mut search = PromptHistorySearchState::new(
            "History".to_string(),
            String::new(),
            String::new(),
            "No matches".to_string(),
            Editor::default(),
            SearchPickerConfig::searchable(),
            None,
            (),
        );

        App::refresh_prompt_history_search(&history, &mut search);
        assert_eq!(search.items.len(), 55);
        assert_eq!(search.items[0].text, "prompt 54");
        assert_eq!(search.items.last().expect("history item").text, "prompt 00");
        assert_eq!(search.result_count(), 55);

        search.input.set_text("prompt 54".to_string());
        search.refresh_results();
        assert!(search.result_count() < 55);
        assert_eq!(
            search.selected_item().expect("best ranked match").text,
            "prompt 54"
        );
    }
}

#[cfg(test)]
mod pending_message_tests {
    use super::super::{PendingUserMessage, TranscriptState};
    use super::parts_fixtures;
    use agena_domain::{
        ActivityId, ActivityPayload, ActivityProvenance, ComposerActivity, ComposerDocument,
        ComposerNode, SkillReferenceActivity,
    };

    #[test]
    fn optimistic_user_document_uses_the_same_activity_and_placeholder_projection() {
        let activity_id = ActivityId::new();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 9,
            document: ComposerDocument(vec![
                ComposerNode::Text {
                    text: "use ".to_owned(),
                },
                ComposerNode::Activity {
                    activity: Box::new(ComposerActivity {
                        id: activity_id,
                        payload: ActivityPayload::SkillReference(SkillReferenceActivity {
                            name: "batch".to_owned(),
                            description: "Run independent work".to_owned(),
                            instructions: "Use isolated tasks".to_owned(),
                            content_hash: "sha256:test".to_owned(),
                            source: "test".to_owned(),
                            aliases: Vec::new(),
                        }),
                        provenance: ActivityProvenance::default(),
                    }),
                },
                ComposerNode::Text {
                    text: " now".to_owned(),
                },
            ]),
            confirmed: false,
        });

        let rendered = transcript.rendered(100);
        let activity_line = rendered
            .lines
            .iter()
            .position(|line| line.text.contains("Skill: batch"))
            .expect("full optimistic Skill Activity");
        let document_line = rendered
            .lines
            .iter()
            .position(|line| line.text.contains("use [Skill: batch] now"))
            .expect("inline optimistic placeholder");
        assert!(activity_line < document_line);
        assert!(rendered.nodes.iter().any(|node| {
            node.key
                == agena_tui_transcript::TranscriptNodeKey::Activity {
                    entry_id: agena_tui_transcript::TranscriptEntryId::PendingTurn(9),
                    content_id: agena_tui_transcript::TranscriptContentId::Activity(activity_id),
                }
        }));
    }

    #[test]
    fn confirmed_optimistic_message_is_atomically_replaced_by_its_persisted_message() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 42,
            document: ComposerDocument(vec![ComposerNode::Text {
                text: "send this now".to_string(),
            }]),
            confirmed: false,
        });

        let pending_lines = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        assert!(
            pending_lines
                .iter()
                .any(|line| line.contains("send this now"))
        );
        assert!(
            pending_lines
                .first()
                .is_some_and(|line| line.starts_with("user "))
        );

        transcript.confirm_pending_user_message(42);
        assert!(
            transcript
                .rendered(80)
                .lines
                .iter()
                .any(|line| line.text.contains("send this now"))
        );
        assert_eq!(transcript.rendered(80).lines[0].text, "user");

        transcript.merge_parts(vec![
            parts_fixtures::run(1, "user", "completed"),
            parts_fixtures::text(2, "user", "send this now"),
            parts_fixtures::run(3, "assistant", "in_progress"),
        ]);

        assert!(transcript.pending_user_messages.is_empty());
        assert_eq!(
            transcript
                .rendered(80)
                .lines
                .iter()
                .filter(|line| line.text.contains("send this now"))
                .count(),
            1
        );
    }

    #[test]
    fn empty_continuation_turn_does_not_consume_an_optimistic_user_message() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 43,
            document: ComposerDocument(vec![ComposerNode::Text {
                text: "next real user message".to_owned(),
            }]),
            confirmed: true,
        });

        transcript.merge_parts(vec![
            parts_fixtures::run(1, "assistant", "completed"),
            parts_fixtures::text(2, "assistant", "continued after permission"),
        ]);

        assert_eq!(transcript.pending_user_messages.len(), 1);
        let rendered = transcript
            .rendered(100)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("continued after permission"),
            "{rendered}"
        );
        assert!(rendered.contains("next real user message"), "{rendered}");
        assert!(!rendered.contains("(empty)"), "{rendered}");
    }

    #[test]
    fn input_materializing_on_an_existing_turn_replaces_the_optimistic_message() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 44,
            document: ComposerDocument(vec![ComposerNode::Text {
                text: "materialized once".to_owned(),
            }]),
            confirmed: true,
        });

        let empty_parts = vec![
            parts_fixtures::run(1, "assistant", "in_progress"),
            parts_fixtures::text(2, "assistant", "assistant reply"),
        ];
        let materialized_parts = vec![
            parts_fixtures::run(1, "user", "completed"),
            parts_fixtures::text(2, "user", "materialized once"),
            parts_fixtures::run(3, "assistant", "in_progress"),
            parts_fixtures::text(4, "assistant", "assistant reply"),
        ];

        transcript.merge_parts(empty_parts);
        assert_eq!(transcript.pending_user_messages.len(), 1);

        transcript.merge_parts(materialized_parts);

        assert!(transcript.pending_user_messages.is_empty());
        let rendered = transcript
            .rendered(100)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            rendered.matches("materialized once").count(),
            1,
            "{rendered}"
        );
        assert!(rendered.contains("assistant reply"), "{rendered}");
    }

    #[test]
    fn optimistic_user_message_precedes_its_empty_active_reply_envelope() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![parts_fixtures::run(1, "assistant", "in_progress")],
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 46,
            document: ComposerDocument(vec![ComposerNode::Text {
                text: "must render before the reply".to_owned(),
            }]),
            confirmed: false,
        });

        let lines = transcript
            .rendered(100)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        let user = lines
            .iter()
            .position(|line| line.starts_with("user"))
            .expect("optimistic user header");
        let assistant = lines
            .iter()
            .position(|line| line.starts_with("assistant"))
            .expect("active assistant header");
        assert!(user < assistant, "rendered lines: {lines:?}");
        assert_eq!(
            lines
                .iter()
                .filter(|line| line.contains("must render before the reply"))
                .count(),
            1
        );
    }
}

#[cfg(test)]
mod transcript_mouse_scroll_tests {
    use agena_domain::{ComposerDocument, ComposerNode, ExecutionStatus};

    fn pending_document(text: String) -> ComposerDocument {
        ComposerDocument(vec![ComposerNode::Text { text }])
    }

    use super::super::{
        PendingUserMessage, RunResource, RunRole, RunStatus, TranscriptMoveDirection,
        TranscriptNodeKey, TranscriptState, TranscriptTextPosition, TranscriptTextSelection, Utc,
    };

    #[test]
    fn nonempty_transcript_materializes_a_visible_primary_cursor_before_rendering() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            document: pending_document(
                (0..40)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });
        assert_eq!(transcript.navigation_cursor_line(), None);

        transcript.ensure_visual_focus(40, 10);

        let focused_line = transcript
            .navigation_cursor_line()
            .expect("nonempty transcript should materialize a line focus");
        assert!(focused_line >= transcript.viewport.top);
        assert!(focused_line < transcript.viewport.top + 10);
        assert_eq!(focused_line, transcript.rendered(40).lines.len() - 1);
        assert!(transcript.viewport.follow_tail);
    }

    #[test]
    fn wheel_moves_the_cursor_and_preserves_its_screen_row() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            document: pending_document(
                (0..80)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });

        transcript.scroll_to_bottom(40, 10);
        let bottom = transcript.viewport.top;
        assert!(bottom > 3);
        assert!(transcript.viewport.follow_tail);
        let bottom_cursor = transcript.navigation_cursor_line().expect("tail cursor");
        let bottom_screen_row = bottom_cursor - bottom;

        transcript.move_cursor_by_wheel(40, 10, -3);
        assert!(transcript.viewport.top < bottom);
        assert!(!transcript.viewport.follow_tail);
        let upward_cursor = transcript
            .navigation_cursor_line()
            .expect("wheel keeps a semantic cursor");
        assert!(upward_cursor < bottom_cursor);
        assert_eq!(upward_cursor - transcript.viewport.top, bottom_screen_row);
        assert_eq!(transcript.highlighted_block_key(), None);

        transcript.move_cursor_by_wheel(40, 10, 3);
        assert_eq!(transcript.viewport.top, bottom);
        assert!(transcript.viewport.follow_tail);
        assert_eq!(transcript.navigation_cursor_line(), Some(bottom_cursor));
    }

    #[test]
    fn scrollbar_and_half_page_motion_use_directional_edge_placement() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            document: pending_document(
                (0..80)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });

        transcript.scroll_to_top(40, 10);
        transcript.relocate_cursor_from_scrollbar(40, 10, 5);
        assert_eq!(transcript.viewport.top, 5);
        assert!(matches!(transcript.navigation_cursor_line(), Some(5..=7)));

        transcript.relocate_cursor_from_scrollbar(40, 10, 30);
        assert_eq!(transcript.viewport.top, 30);
        assert!(matches!(transcript.navigation_cursor_line(), Some(30..=32)));

        transcript.move_cursor_by_half_page(40, 10, false);
        let cursor = transcript
            .navigation_cursor_line()
            .expect("half-page motion keeps a semantic cursor");
        assert!(transcript.viewport.top < 30);
        assert!((7..=9).contains(&(cursor - transcript.viewport.top)));
    }

    #[test]
    fn scrollbar_relocation_collapses_a_block_and_selects_the_directional_edge() {
        let now = Utc::now();
        let message = |id: i64, text: String| RunResource {
            id,
            session_id: 7,
            role: RunRole::Assistant,
            state: RunStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![api_message_part!(
                id * 10,
                id,
                now,
                ExecutionStatus::Completed,
                FixturePart::text(text),
            )]),
        };
        let long_text = |prefix: &str| {
            (0..30)
                .map(|line| format!("{prefix} {line}"))
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![
                message(1, long_text("first")),
                message(2, long_text("second")),
            ],
            ..TranscriptState::default()
        };
        let second_message = transcript
            .rendered(40)
            .nodes
            .iter()
            .position(|node| {
                node.key
                    == TranscriptNodeKey::Entry {
                        entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(2),
                    }
            })
            .expect("second message parent");
        transcript.set_block_cursor(40, 10, second_message, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Entry {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(2)
            })
        );

        transcript.relocate_cursor_from_scrollbar(40, 10, 0);
        assert_eq!(transcript.highlighted_block_key(), None);
        let focused_line = transcript
            .navigation_cursor_line()
            .expect("scrollbar relocation should leave a visible line target");
        assert!(focused_line < 10);
        assert!(focused_line >= 7);

        transcript.move_by_blocks(40, 10, TranscriptMoveDirection::Down, 1);
        assert_eq!(transcript.viewport.top, 0);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Entry {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(1)
            })
        );
    }

    #[test]
    fn pointer_can_select_a_markdown_line_or_its_whole_block_and_resume_text_selection() {
        let now = Utc::now();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 1,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    10,
                    1,
                    now,
                    ExecutionStatus::Completed,
                    FixturePart::text(
                        "first paragraph contains enough words to wrap across several rendered lines while remaining one markdown block\n\nsecond paragraph",
                    ),
                )]),
            }],
            ..TranscriptState::default()
        };
        let (clickable_line, block_key, block_range) = {
            let node = transcript
                .rendered(40)
                .nodes
                .iter()
                .find(|node| {
                    matches!(&node.key, TranscriptNodeKey::MarkdownBlock { .. })
                        && node.end_line.saturating_sub(node.start_line) > 1
                })
                .expect("multiline markdown block");
            (
                node.start_line + 1,
                node.key.clone(),
                node.start_line..node.end_line,
            )
        };
        let position = TranscriptTextPosition {
            line: clickable_line,
            column: 3,
        };

        transcript.select_pointer_line(40, 10, position);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(transcript.navigation_cursor_line(), Some(clickable_line));
        assert!(
            transcript
                .current_selected_line_text(40)
                .is_some_and(|line| !line.is_empty())
        );

        transcript.select_pointer_block(40, 10, position);
        assert_eq!(transcript.highlighted_block_key(), Some(block_key));
        assert_eq!(
            transcript.highlighted_block_range(40),
            Some(block_range.clone())
        );
        assert_eq!(transcript.navigation_cursor_line(), Some(clickable_line));
        assert_eq!(transcript.current_selected_line_text(40), None);

        transcript.move_cursor_one_line(40, 10, TranscriptMoveDirection::Down);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(
            transcript.navigation_cursor_line(),
            Some(block_range.start),
            "continuing from a whole-block selection enters at its directional edge"
        );

        let head_line = clickable_line.saturating_add(1);
        let cursor_before_drag = transcript.navigation_cursor_line();
        let selection = transcript.set_text_selection(
            40,
            TranscriptTextSelection {
                anchor: TranscriptTextPosition {
                    line: clickable_line,
                    column: 1,
                },
                head: TranscriptTextPosition {
                    line: head_line,
                    column: 4,
                },
            },
        );
        assert_eq!(transcript.text_selection(), Some(selection));
        assert_eq!(
            transcript.navigation_cursor_line(),
            cursor_before_drag,
            "committing a pointer range must not move the navigation cursor"
        );

        transcript.cancel_text_selection(40, 10);
        assert_eq!(transcript.navigation_cursor_line(), cursor_before_drag);
        assert_eq!(transcript.text_selection(), None);

        transcript.set_text_selection(
            40,
            TranscriptTextSelection {
                anchor: TranscriptTextPosition {
                    line: clickable_line,
                    column: 1,
                },
                head: TranscriptTextPosition {
                    line: head_line,
                    column: 4,
                },
            },
        );
        transcript.select_pointer_line(
            40,
            10,
            TranscriptTextPosition {
                line: head_line,
                column: 2,
            },
        );
        assert_eq!(transcript.navigation_cursor_line(), Some(head_line));
        assert_eq!(
            transcript.text_selection(),
            None,
            "a later click moves the navigation cursor and clears the old drag range"
        );
    }

    #[test]
    fn cursor_reflow_keeps_the_same_semantic_markdown_block() {
        let now = Utc::now();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 1,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    10,
                    1,
                    now,
                    ExecutionStatus::Completed,
                    FixturePart::text(
                        "a deliberately long markdown paragraph whose wrapped rows change when the terminal width changes but whose semantic identity must remain stable",
                    ),
                )]),
            }],
            ..TranscriptState::default()
        };
        transcript.viewport.follow_tail = false;
        let (line, key) = {
            let node = transcript
                .rendered(24)
                .nodes
                .iter()
                .find(|node| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
                .expect("markdown block");
            (node.start_line.saturating_add(2), node.key.clone())
        };
        transcript.select_pointer_line(24, 10, TranscriptTextPosition { line, column: 2 });

        transcript.ensure_visual_focus(70, 10);

        assert_eq!(
            transcript
                .current_cursor_node_cloned(70)
                .map(|node| node.key),
            Some(key)
        );
        let cursor = transcript
            .navigation_cursor_line()
            .expect("cursor after reflow");
        assert!(cursor >= transcript.viewport.top);
        assert!(cursor < transcript.viewport.top + 10);
    }
}

#[cfg(test)]
mod transcript_paging_tests {
    use agena_domain::{ComposerDocument, ComposerNode, ExecutionStatus, ReasoningPart};

    use super::super::{
        PendingUserMessage, RunResource, RunRole, RunStatus, TranscriptNodeKey, TranscriptState,
        TranscriptTextPosition, Utc,
    };

    fn pending_document(text: String) -> ComposerDocument {
        ComposerDocument(vec![ComposerNode::Text { text }])
    }

    fn line_is_focusable(transcript: &mut TranscriptState, line: usize) -> bool {
        transcript
            .rendered(40)
            .lines
            .get(line)
            .is_some_and(|line| line.navigation_unit.is_some() || !line.copy_text.is_empty())
    }

    #[test]
    fn page_down_advances_exactly_one_page_regardless_of_cursor_row() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            document: pending_document(
                (0..80)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });

        transcript.scroll_to_top(40, 10);
        assert!(transcript.max_scroll(40, 10) > 30);

        // Park the cursor on the bottom focusable row of the first viewport.
        let bottom_line = (0..10)
            .rev()
            .find(|line| line_is_focusable(&mut transcript, *line))
            .expect("first viewport has a focusable row");
        transcript.select_pointer_line(
            40,
            10,
            TranscriptTextPosition {
                line: bottom_line,
                column: 0,
            },
        );
        assert_eq!(transcript.viewport.top, 0);

        transcript.move_cursor_by_page(40, 10, true);
        assert_eq!(
            transcript.viewport.top, 10,
            "PageDown must advance exactly one viewport, not 1.5-2 pages, from any cursor row"
        );
        let cursor = transcript
            .navigation_cursor_line()
            .expect("page-down keeps a semantic cursor");
        assert!(cursor >= transcript.viewport.top);
        assert!(cursor < transcript.viewport.top + 10);
        assert!(
            cursor.saturating_sub(transcript.viewport.top) <= 2,
            "the cursor should sit at the top edge of the new page"
        );

        transcript.move_cursor_by_page(40, 10, false);
        assert_eq!(
            transcript.viewport.top, 0,
            "PageUp must restore the previous viewport exactly"
        );
    }

    #[test]
    fn half_page_advances_half_a_viewport_from_the_bottom_row() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            document: pending_document(
                (0..80)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });

        transcript.scroll_to_top(40, 10);
        let bottom_line = (0..10)
            .rev()
            .find(|line| line_is_focusable(&mut transcript, *line))
            .expect("first viewport has a focusable row");
        transcript.select_pointer_line(
            40,
            10,
            TranscriptTextPosition {
                line: bottom_line,
                column: 0,
            },
        );

        transcript.move_cursor_by_half_page(40, 10, true);
        assert_eq!(transcript.viewport.top, 5);
        let cursor = transcript
            .navigation_cursor_line()
            .expect("half-page keeps a semantic cursor");
        assert!(cursor >= transcript.viewport.top);
        assert!(cursor < transcript.viewport.top + 10);
    }

    #[test]
    fn page_down_to_the_tail_resumes_following() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            document: pending_document(
                (0..80)
                    .map(|line| format!("line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });

        transcript.scroll_to_top(40, 10);
        assert!(!transcript.viewport.follow_tail);
        let max_scroll = transcript.max_scroll(40, 10);
        // Page down until the last page; the final press should land on the
        // last focusable row and re-enable tail following.
        while transcript.viewport.top < max_scroll {
            transcript.move_cursor_by_page(40, 10, true);
        }
        assert_eq!(transcript.viewport.top, max_scroll);
        assert!(
            transcript.viewport.follow_tail,
            "landing on the final page must resume tail-following"
        );
    }

    #[test]
    fn paging_inside_a_tall_expanded_activity_advances_one_page() {
        let now = Utc::now();
        let activity = api_message_part!(
            24,
            18,
            now,
            ExecutionStatus::Completed,
            FixturePart::Reasoning(ReasoningPart {
                summary: vec![
                    (0..40)
                        .map(|line| format!("deep thought line {line}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ],
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        let activity_key = TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(18),
            content_id: agena_tui_transcript::TranscriptContentId::Activity(
                activity.activity_id.expect("reasoning activity identity"),
            ),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 18,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![activity]),
            }],
            ..TranscriptState::default()
        };

        let (node_start, node_end, expanded) = {
            let node = transcript
                .rendered(40)
                .nodes
                .iter()
                .find(|node| node.key == activity_key)
                .expect("expanded reasoning activity");
            (node.start_line, node.end_line, node.expanded)
        };
        assert!(expanded);
        assert!(node_end.saturating_sub(node_start) > 20);

        // Scroll to the bottom, then move the cursor back to the top edge of
        // the viewport while staying inside the tall expanded activity.
        transcript.scroll_to_bottom(40, 10);
        let max_scroll = transcript.viewport.top;
        assert_eq!(max_scroll, transcript.max_scroll(40, 10));
        transcript.move_cursor_by_visual_lines(
            40,
            10,
            agena_tui_transcript::TranscriptMoveDirection::Up,
            9,
        );
        let cursor_line = transcript
            .navigation_cursor_line()
            .expect("cursor inside the expanded activity");
        assert!(cursor_line >= node_start);

        transcript.move_cursor_by_page(40, 10, false);
        assert_eq!(
            transcript.viewport.top,
            max_scroll.saturating_sub(10),
            "PageUp inside an expanded activity must advance exactly one page"
        );
        let page_cursor = transcript
            .navigation_cursor_line()
            .expect("page-up keeps a semantic cursor");
        assert!(
            page_cursor >= node_start,
            "one page up must stay inside the expanded activity, not skip past it"
        );
        assert!(page_cursor < node_end);
    }

    #[test]
    fn page_motions_still_move_the_cursor_when_the_transcript_fits_the_viewport() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 1,
            document: pending_document(
                (0..4)
                    .map(|line| format!("short line {line}"))
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            confirmed: false,
        });

        transcript.scroll_to_top(40, 10);
        let total_lines = transcript.rendered(40).lines.len();
        assert!(
            total_lines < 10,
            "the fixture must be shorter than the viewport so the viewport cannot scroll"
        );
        assert_eq!(transcript.max_scroll(40, 10), 0);

        let first_focusable = (0..total_lines)
            .find(|line| line_is_focusable(&mut transcript, *line))
            .expect("first focusable row");
        let last_focusable = (0..total_lines)
            .rev()
            .find(|line| line_is_focusable(&mut transcript, *line))
            .expect("last focusable row");
        assert!(last_focusable > first_focusable);

        transcript.select_pointer_line(
            40,
            10,
            TranscriptTextPosition {
                line: first_focusable,
                column: 0,
            },
        );
        assert_eq!(transcript.viewport.top, 0);

        transcript.move_cursor_by_page(40, 10, true);
        assert_eq!(
            transcript.viewport.top, 0,
            "the viewport must not scroll when the content is shorter than the window"
        );
        assert_eq!(
            transcript.navigation_cursor_line(),
            Some(last_focusable),
            "PageDown on a short transcript must still move the cursor to the boundary"
        );

        transcript.move_cursor_by_page(40, 10, false);
        assert_eq!(
            transcript.navigation_cursor_line(),
            Some(first_focusable),
            "PageUp on a short transcript must move the cursor back to the boundary"
        );
    }
}

#[cfg(test)]
mod transcript_activity_copy_tests {
    use super::super::{
        ExecutionStatus, RunResource, RunRole, RunStatus, TranscriptNodeKey, TranscriptState,
        TranscriptTextPosition, TranscriptVisualSelectionMode, Utc,
    };

    fn reasoning_activity(message_id: i64, part_id: i64) -> agena_api::part::PartResource {
        crate::TranscriptFixture::reasoning_part(
            part_id,
            message_id,
            Utc::now(),
            ExecutionStatus::Completed,
            agena_domain::ReasoningPart {
                summary: vec![format!("deep thought {part_id}")],
                raw_content: Vec::new(),
                encrypted_content: None,
            },
        )
    }

    fn folded_run_parts() -> Vec<agena_api::part::PartResource> {
        (51..59).map(|part| reasoning_activity(19, part)).collect()
    }

    fn folded_run_transcript(parts: Vec<agena_api::part::PartResource>) -> TranscriptState {
        TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 19,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                metadata: Default::default(),
                usage: None,
                part_count: parts.len() as u64,
                parts: Some(parts),
            }],
            ..TranscriptState::default()
        }
    }

    fn entry_node(
        transcript: &mut TranscriptState,
        message_id: i64,
    ) -> agena_tui_transcript::RenderedTranscriptNode {
        transcript
            .rendered(120)
            .nodes
            .iter()
            .find(|node| {
                node.key
                    == TranscriptNodeKey::Entry {
                        entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(
                            message_id,
                        ),
                    }
            })
            .cloned()
            .expect("entry container node")
    }

    #[test]
    fn folded_run_marker_copies_only_the_visible_marker_not_the_hidden_activities() {
        let mut transcript = folded_run_transcript(folded_run_parts());
        let marker_line = transcript
            .rendered(120)
            .lines
            .iter()
            .enumerate()
            .find_map(|(line, rendered)| {
                rendered
                    .text
                    .contains("older activity blocks collapsed")
                    .then_some(line)
            })
            .expect("folded run marker line");
        transcript.select_pointer_line(
            120,
            20,
            TranscriptTextPosition {
                line: marker_line,
                column: 0,
            },
        );
        transcript.toggle_visual_selection(120, 20, TranscriptVisualSelectionMode::Line);
        let copied = transcript.selected_text(120, "").expect("Visual line copy");
        assert!(
            copied.contains("older activity blocks collapsed"),
            "copying the folded marker should keep the visible marker: {copied}"
        );
        assert!(
            !copied.contains("deep thought"),
            "V-copy must not expand the hidden activities behind a collapsed fold: {copied}"
        );
    }

    #[test]
    fn entry_copy_excludes_collapsed_activities_and_includes_expanded_ones() {
        let parts = folded_run_parts();
        let mut transcript = folded_run_transcript(parts.clone());
        let activity_key = |index: usize| TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(19),
            content_id: agena_tui_transcript::TranscriptContentId::Activity(
                parts[index]
                    .activity_id
                    .expect("reasoning activity identity"),
            ),
        };
        let summary_key = TranscriptNodeKey::ActivitySummary {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(19),
            first_content_id: agena_tui_transcript::TranscriptContentId::Activity(
                parts[0].activity_id.expect("reasoning activity identity"),
            ),
        };

        // Reasoning defaults to expanded, so the visible tail (parts 54..58)
        // is expanded content and belongs in the copy. The older activities
        // (51..53) are folded away behind the marker and must never leak.
        let entry = entry_node(&mut transcript, 19);
        assert!(
            !entry.copy_text.contains("deep thought 51")
                && !entry.copy_text.contains("deep thought 52")
                && !entry.copy_text.contains("deep thought 53"),
            "folded-away activities must not leak into the message copy: {}",
            entry.copy_text
        );
        assert!(
            entry.copy_text.contains("deep thought 54"),
            "default-expanded reasoning belongs in the message copy: {}",
            entry.copy_text
        );
        assert!(
            !entry.copy_text.contains("older activity blocks collapsed"),
            "the fold marker itself must never appear in the message copy: {}",
            entry.copy_text
        );

        // Expand the fold: the hidden activities render as their own nodes.
        // A still-collapsed one stays out of the copy while its expanded
        // siblings come in.
        transcript.node_expansions.insert(summary_key, true);
        transcript.node_expansions.insert(activity_key(0), false);
        transcript.node_expansions.insert(activity_key(1), true);
        transcript.invalidate_render();
        let entry = entry_node(&mut transcript, 19);
        assert!(
            !entry.copy_text.contains("deep thought 51"),
            "a collapsed activity body must not leak into the message copy: {}",
            entry.copy_text
        );
        assert!(
            entry.copy_text.contains("deep thought 52"),
            "an expanded activity body belongs in the message copy: {}",
            entry.copy_text
        );
    }
}

#[cfg(test)]
mod transcript_expansion_tests {
    use agena_domain::{ExecutionStatus, ReasoningPart};

    use super::super::{
        RunResource, RunRole, RunStatus, TranscriptMoveDirection, TranscriptNodeKey,
        TranscriptNodeKind, TranscriptState, TranscriptTextPosition, TranscriptTextSelection, Utc,
        transcript_text_selection_text,
    };
    use super::parts_fixtures;

    #[test]
    fn activity_expansion_survives_a_full_parts_refresh() {
        // A `hook` part projects to a toggleable Activity whose content id is
        // the part id (design 4.1.1). Expansion state is keyed by that id, so a
        // full parts re-merge (the v2 refresh path, `merge_parts`) must keep
        // the user-expanded Activity open while folding untouched siblings.
        let hook = parts_fixtures::hook(
            4,
            "assistant",
            "Run background scan",
            "scanned 3 workspaces",
        );
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "user", "completed"),
                parts_fixtures::text(2, "user", "list tools"),
                parts_fixtures::run(3, "assistant", "completed"),
                hook.clone(),
            ],
            detail_expanded_by_default: agena_tui_transcript::TranscriptDetailDefaults {
                activity_default_expanded: false,
                kind_defaults: std::collections::BTreeMap::new(),
            },
            ..TranscriptState::default()
        };
        let key = TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(3),
            content_id: agena_tui_transcript::TranscriptContentId::StoredPart(hook.part_id),
        };
        let collapsed = transcript
            .rendered(100)
            .nodes
            .iter()
            .find(|node| node.key == key)
            .cloned()
            .expect("collapsed hook Activity");
        assert!(collapsed.toggleable);
        assert!(!collapsed.expanded);
        assert!(
            transcript
                .rendered(100)
                .lines
                .iter()
                .all(|line| !line.text.contains("scanned 3 workspaces"))
        );

        transcript.set_cursor_line(100, 20, collapsed.start_line);
        assert_eq!(
            transcript.toggle_cursor_node_expansion(100, 20),
            Some((TranscriptNodeKind::Activity, true))
        );
        let expanded = transcript
            .rendered(100)
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("expanded hook Activity");
        assert!(expanded.expanded);
        assert!(
            transcript
                .rendered(100)
                .lines
                .iter()
                .any(|line| line.text.contains("scanned 3 workspaces"))
        );

        // A full parts refresh appends sibling hooks; the user-expanded
        // Activity must stay open and untouched siblings fold into a summary.
        transcript.merge_parts(vec![
            parts_fixtures::run(1, "user", "completed"),
            parts_fixtures::text(2, "user", "list tools"),
            parts_fixtures::run(3, "assistant", "completed"),
            hook.clone(),
            parts_fixtures::hook(5, "assistant", "Run index", "indexed 1 file"),
            parts_fixtures::hook(6, "assistant", "Run lint", "linted 2 files"),
            parts_fixtures::hook(7, "assistant", "Run test", "ran 3 tests"),
            parts_fixtures::hook(8, "assistant", "Run build", "built 4 targets"),
            parts_fixtures::hook(9, "assistant", "Run deploy", "deployed 5 units"),
            parts_fixtures::hook(10, "assistant", "Run verify", "verified 6 steps"),
            parts_fixtures::hook(11, "assistant", "Run audit", "audited 7 modules"),
        ]);

        let expanded_after_new_content = transcript
            .rendered(100)
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("expanded Activity remains rendered after a full parts refresh");
        assert!(expanded_after_new_content.expanded);
        assert!(
            transcript.rendered(100).nodes.iter().any(|node| {
                matches!(node.key, TranscriptNodeKey::ActivitySummary { .. }) && !node.expanded
            }),
            "the run may still fold untouched Activities without hiding the user-expanded one"
        );
    }

    #[test]
    fn collapsing_from_inside_an_activity_keeps_the_cursor_on_that_activity() {
        let now = Utc::now();
        let message_id = 17;
        let part_id = 23;
        let part = api_message_part!(
            part_id,
            message_id,
            now,
            ExecutionStatus::Completed,
            FixturePart::Reasoning(ReasoningPart {
                summary: vec!["first line\nsecond line\nthird line".to_string()],
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        let key = TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(message_id),
            content_id: agena_tui_transcript::TranscriptContentId::Activity(
                part.activity_id.expect("reasoning activity identity"),
            ),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: message_id,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![part]),
            }],
            ..TranscriptState::default()
        };
        transcript.node_expansions.insert(key.clone(), true);

        let expanded = transcript
            .rendered(80)
            .nodes
            .iter()
            .find(|node| node.key == key)
            .cloned()
            .expect("expanded reasoning node");
        assert!(expanded.end_line.saturating_sub(expanded.start_line) > 2);
        transcript.set_cursor_line(80, 10, expanded.end_line - 1);
        assert_eq!(
            transcript
                .current_cursor_node_cloned(80)
                .map(|node| node.key),
            Some(key.clone())
        );

        let (_, expanded) = transcript
            .toggle_cursor_node_expansion(80, 10)
            .expect("reasoning should be toggleable");

        assert!(!expanded);
        let collapsed = transcript
            .current_cursor_node_cloned(80)
            .expect("cursor should remain on collapsed reasoning");
        assert_eq!(collapsed.key, key);
        assert_eq!(
            transcript.navigation_cursor_line(),
            Some(collapsed.start_line)
        );
        assert_eq!(collapsed.end_line, collapsed.start_line + 1);
    }

    #[test]
    fn reasoning_renders_the_full_body_verbatim_and_expands_by_default() {
        let now = Utc::now();
        let message_id = 25;
        // A long, multi-line reasoning body that must never be ellipsized.
        let body = (0..40)
            .map(|line| format!("deep thought line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let part = api_message_part!(
            26,
            message_id,
            now,
            ExecutionStatus::Completed,
            FixturePart::Reasoning(ReasoningPart {
                summary: vec![body.clone()],
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        let key = TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(message_id),
            content_id: agena_tui_transcript::TranscriptContentId::Activity(
                part.activity_id.expect("reasoning activity identity"),
            ),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: message_id,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![part]),
            }],
            ..TranscriptState::default()
        };

        let rendered = transcript.rendered(120);
        let node = rendered
            .nodes
            .iter()
            .find(|node| node.key == key)
            .expect("reasoning node");
        // Reasoning defaults to expanded so the full trail is immediately visible.
        assert!(node.expanded, "reasoning must default to expanded");
        // The full multi-line body is present verbatim, including the last line.
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.text.contains("deep thought line 0")),
            "reasoning first line must be present"
        );
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.text.contains("deep thought line 39")),
            "reasoning last line must be present and not truncated"
        );
        assert!(
            !rendered
                .lines
                .iter()
                .any(|line| line.text.contains("truncated")),
            "reasoning body must never carry a truncation marker"
        );
    }

    #[test]
    fn expanding_the_final_activity_scrolls_its_new_lines_into_view() {
        let now = Utc::now();
        let preceding_part = api_message_part!(
            22,
            17,
            now,
            ExecutionStatus::Completed,
            FixturePart::text("one\n\ntwo\n\nthree\n\nfour\n\nfive"),
        );
        let activity = api_message_part!(
            24,
            18,
            now,
            ExecutionStatus::Completed,
            FixturePart::Reasoning(ReasoningPart {
                summary: vec!["first line\nsecond line\nthird line".to_string()],
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        let activity_key = TranscriptNodeKey::Activity {
            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(18),
            content_id: agena_tui_transcript::TranscriptContentId::Activity(
                activity.activity_id.expect("reasoning activity identity"),
            ),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![
                RunResource {
                    id: 17,
                    session_id: 7,
                    role: RunRole::User,
                    state: RunStatus::Completed,
                    created_at: now,
                    updated_at: now,
                    metadata: Default::default(),
                    usage: None,
                    part_count: 1,
                    parts: Some(vec![preceding_part]),
                },
                RunResource {
                    id: 18,
                    session_id: 7,
                    role: RunRole::Assistant,
                    state: RunStatus::Completed,
                    created_at: now,
                    updated_at: now,
                    metadata: Default::default(),
                    usage: None,
                    part_count: 1,
                    parts: Some(vec![activity]),
                },
            ],
            ..TranscriptState::default()
        };
        // Reasoning defaults to expanded; explicitly collapse it first so the
        // toggle below exercises the expand-then-scroll path it tests.
        transcript
            .node_expansions
            .insert(activity_key.clone(), false);
        let node_index = transcript
            .rendered(80)
            .nodes
            .iter()
            .position(|node| node.key == activity_key)
            .expect("collapsed final activity");
        transcript.set_block_cursor(80, 5, node_index, TranscriptMoveDirection::Down);
        let collapsed_scroll = transcript.viewport.top;

        let (_, expanded) = transcript
            .toggle_cursor_node_expansion(80, 5)
            .expect("reasoning should expand");

        assert!(expanded);
        let expanded_node = transcript
            .current_cursor_node_cloned(80)
            .expect("cursor remains on expanded final activity");
        assert_eq!(expanded_node.key, activity_key);
        assert!(transcript.viewport.top > collapsed_scroll);
        assert!(
            expanded_node.end_line <= transcript.viewport.top.saturating_add(5),
            "the complete expanded activity should fit in the viewport"
        );
        let max_scroll = transcript.max_scroll(80, 5);
        assert_eq!(transcript.viewport.top, max_scroll);
    }

    #[test]
    fn vertical_navigation_stops_on_messages_and_blocks_before_entering_text() {
        let now = Utc::now();
        let message = |id: i64, role: RunRole, text: &str| RunResource {
            id,
            session_id: 7,
            role,
            state: RunStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![api_message_part!(
                id * 10,
                id,
                now,
                ExecutionStatus::Completed,
                FixturePart::text(text),
            )]),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![
                message(9, RunRole::User, "before"),
                message(
                    10,
                    RunRole::Assistant,
                    "a wrapped answer with several rendered rows that can be navigated independently",
                ),
                message(11, RunRole::User, "after"),
            ],
            ..TranscriptState::default()
        };
        transcript.viewport.follow_tail = false;
        let first_message_index = transcript
            .rendered(24)
            .nodes
            .iter()
            .position(|node| {
                node.key
                    == TranscriptNodeKey::Entry {
                        entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(9),
                    }
            })
            .expect("first message parent");
        transcript.set_block_cursor(24, 20, first_message_index, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Entry {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10)
            }),
            "crossing a boundary first selects the complete destination message"
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert!(matches!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::MarkdownBlock {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10),
                ..
            })
        ));
        let block_start = transcript
            .current_cursor_node_cloned(24)
            .expect("selected Markdown block")
            .start_line;
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(transcript.navigation_cursor_line(), Some(block_start));
        assert_eq!(transcript.highlighted_block_key(), None);

        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert!(
            transcript
                .navigation_cursor_line()
                .is_some_and(|line| line > block_start)
        );
        assert_eq!(transcript.highlighted_block_key(), None);

        let third_message_line = transcript
            .rendered(24)
            .nodes
            .iter()
            .find(|node| {
                !node.key.is_entry_container()
                    && node.key.entry_id()
                        == agena_tui_transcript::TranscriptEntryId::StoredMessage(11)
            })
            .expect("third message child")
            .start_line;
        transcript.select_pointer_line(
            24,
            20,
            TranscriptTextPosition {
                line: third_message_line,
                column: 2,
            },
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Up);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::Entry {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10)
            }),
            "Up crossing a boundary first selects the complete previous message"
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Up);
        assert!(matches!(
            transcript.highlighted_block_key(),
            Some(TranscriptNodeKey::MarkdownBlock {
                entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10),
                ..
            })
        ));
    }

    #[test]
    fn code_blocks_expose_clean_semantic_source_lines_without_stopping_on_card_chrome() {
        let now = Utc::now();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 10,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    20,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    FixturePart::text(
                        "before\n\n```rust\nlet first = \"abcdefghijklmnopqrstuvwxyz\";\nlet second = 2;\n```\n\nafter",
                    ),
                )]),
            }],
            ..TranscriptState::default()
        };
        let blocks = transcript
            .rendered(40)
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .map(|(index, node)| (index, node.key.clone(), node.atomic))
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 3);
        assert!(!blocks[0].2);
        assert!(!blocks[1].2, "a code block has meaningful source lines");
        assert!(!blocks[2].2);

        let (code_range, first_source_range) = {
            let rendered = transcript.rendered(24);
            let node = rendered.nodes.get(blocks[1].0).expect("code node");
            let first_source_line = (node.start_line..node.end_line)
                .find(|line| {
                    rendered.lines[*line].navigation_copy_text
                        == "let first = \"abcdefghijklmnopqrstuvwxyz\";"
                })
                .expect("first code source line");
            let unit = rendered.lines[first_source_line]
                .navigation_unit
                .expect("code source navigation unit");
            let start = (node.start_line..=first_source_line)
                .rev()
                .take_while(|line| rendered.lines[*line].navigation_unit == Some(unit))
                .last()
                .expect("source range start");
            let end = (first_source_line..node.end_line)
                .take_while(|line| rendered.lines[*line].navigation_unit == Some(unit))
                .last()
                .expect("source range end")
                .saturating_add(1);
            (node.start_line..node.end_line, start..end)
        };
        assert!(
            first_source_range.len() > 1,
            "fixture must wrap one source line"
        );

        transcript.select_pointer_line(
            24,
            20,
            TranscriptTextPosition {
                line: first_source_range.start.saturating_add(1),
                column: 5,
            },
        );
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(
            transcript.highlighted_line_range(24),
            Some(first_source_range.clone())
        );
        assert_eq!(
            transcript.current_selected_line_text(24).as_deref(),
            Some("let first = \"abcdefghijklmnopqrstuvwxyz\";")
        );

        let drag_line = first_source_range.start.saturating_add(1);
        let cursor_before_drag = transcript.navigation_cursor_line();
        let selection = transcript.set_text_selection(
            24,
            TranscriptTextSelection {
                anchor: TranscriptTextPosition {
                    line: drag_line,
                    column: 4,
                },
                head: TranscriptTextPosition {
                    line: drag_line,
                    column: 6,
                },
            },
        );
        assert_eq!(selection.anchor.line, drag_line);
        assert_eq!(selection.anchor.column, 4);
        assert_eq!(selection.head.line, drag_line);
        assert_eq!(selection.head.column, 6);
        assert_eq!(transcript.navigation_cursor_line(), cursor_before_drag);
        transcript.cancel_text_selection(24, 20);

        transcript.set_block_cursor(24, 20, blocks[0].0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.clone())
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(transcript.highlighted_block_key(), None);
        assert_eq!(
            transcript.current_selected_line_text(24).as_deref(),
            Some("let first = \"abcdefghijklmnopqrstuvwxyz\";")
        );
        assert!(
            transcript
                .navigation_cursor_line()
                .is_some_and(|line| line > code_range.start && line + 1 < code_range.end),
            "the code-card borders must not become navigation stops"
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(24).as_deref(),
            Some("let second = 2;")
        );
        transcript.move_cursor_one_line(24, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[2].1.clone()),
            "leaving the last source line skips the bottom card border"
        );
    }

    #[test]
    fn tables_expose_one_clean_semantic_unit_per_table_row() {
        let now = Utc::now();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 10,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    20,
                    10,
                    now,
                    ExecutionStatus::Completed,
                    FixturePart::text(
                        "before\n\n| name | value |\n| --- | ---: |\n| answer | 42 |\n\nafter",
                    ),
                )]),
            }],
            ..TranscriptState::default()
        };
        let blocks = transcript
            .rendered(40)
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .map(|(index, node)| (index, node.key.clone(), node.atomic))
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 3);
        assert!(!blocks[1].2, "table rows are semantically divisible");

        transcript.set_block_cursor(40, 20, blocks[0].0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(40, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.clone())
        );

        transcript.move_cursor_one_line(40, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(40).as_deref(),
            Some("name\tvalue")
        );
        let header_line = transcript
            .navigation_cursor_line()
            .expect("table header row");
        assert!(
            !transcript.rendered(40).lines[header_line]
                .text
                .contains('┌')
        );

        transcript.move_cursor_one_line(40, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(40).as_deref(),
            Some("answer\t42")
        );
        let (answer_line, answer_column) = {
            let rendered = transcript.rendered(40);
            rendered
                .lines
                .iter()
                .enumerate()
                .find_map(|(line, rendered_line)| {
                    rendered_line
                        .copy_segments
                        .iter()
                        .find(|segment| segment.text == "answer")
                        .map(|segment| (line, segment.display_column))
                })
                .expect("answer table cell copy segment")
        };
        let cursor_before_drag = transcript.navigation_cursor_line();
        let selection = transcript.set_text_selection(
            40,
            TranscriptTextSelection {
                anchor: TranscriptTextPosition {
                    line: answer_line,
                    column: answer_column.saturating_add(1),
                },
                head: TranscriptTextPosition {
                    line: answer_line,
                    column: answer_column.saturating_add(3),
                },
            },
        );
        assert_eq!(selection.anchor.line, answer_line);
        assert_eq!(selection.head.line, answer_line);
        assert_eq!(transcript.navigation_cursor_line(), cursor_before_drag);
        let copied = {
            let rendered = transcript.rendered(40);
            transcript_text_selection_text(
                rendered.lines.as_slice(),
                rendered.nodes.as_slice(),
                rendered.line_nodes.as_slice(),
                selection,
                "",
            )
        };
        assert_eq!(copied, "nsw");
        transcript.cancel_text_selection(40, 20);
        transcript.move_cursor_one_line(40, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[2].1.clone()),
            "separator and bottom-border rows must not become stops"
        );
    }

    #[test]
    fn single_formulas_remain_atomic_while_inline_formula_canvases_form_one_semantic_line() {
        let now = Utc::now();
        let message = |id, text: &str| RunResource {
            id,
            session_id: 7,
            role: RunRole::Assistant,
            state: RunStatus::Completed,
            created_at: now,
            updated_at: now,
            metadata: Default::default(),
            usage: None,
            part_count: 1,
            parts: Some(vec![api_message_part!(
                id * 10,
                id,
                now,
                ExecutionStatus::Completed,
                FixturePart::text(text),
            )]),
        };
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![
                message(10, "before\n\n$$\n\\frac{a}{b}\n$$\n\nafter"),
                message(
                    11,
                    "inline before $\\begin{bmatrix}a\\\\b\\\\c\\end{bmatrix}$ inline after",
                ),
            ],
            ..TranscriptState::default()
        };
        let (before, formula, after) = {
            let blocks = transcript
                .rendered(80)
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| {
                    matches!(
                        node.key,
                        TranscriptNodeKey::MarkdownBlock {
                            entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(10),
                            ..
                        }
                    )
                })
                .map(|(index, node)| (index, node.key.clone(), node.atomic))
                .collect::<Vec<_>>();
            assert_eq!(blocks.len(), 3);
            assert!(blocks[1].2, "a display formula is one semantic object");
            (blocks[0].clone(), blocks[1].clone(), blocks[2].clone())
        };
        transcript.set_block_cursor(80, 20, before.0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(80, 20, TranscriptMoveDirection::Down);
        assert_eq!(transcript.highlighted_block_key(), Some(formula.1.clone()));
        transcript.move_cursor_one_line(80, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(after.1),
            "formula canvas rows are crossed in one keypress"
        );

        let inline = transcript
            .rendered(80)
            .nodes
            .iter()
            .enumerate()
            .find(|(_, node)| {
                matches!(
                    node.key,
                    TranscriptNodeKey::MarkdownBlock {
                        entry_id: agena_tui_transcript::TranscriptEntryId::StoredMessage(11),
                        ..
                    }
                )
            })
            .map(|(index, node)| (index, node.clone()))
            .expect("inline-formula paragraph");
        assert!(
            !inline.1.atomic,
            "the paragraph itself must remain enterable"
        );
        let units = transcript.rendered(80).lines[inline.1.start_line..inline.1.end_line]
            .iter()
            .filter_map(|line| line.navigation_unit)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            units.len(),
            1,
            "all formula canvas rows form one logical line"
        );
        transcript.set_block_cursor(80, 20, inline.0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(80, 20, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(80).as_deref(),
            Some("inline before \\begin{bmatrix}a\\\\b\\\\c\\end{bmatrix} inline after")
        );
    }

    #[test]
    fn aligned_formula_blocks_enter_and_navigate_by_top_level_equation_row() {
        let now = Utc::now();
        let aligned = concat!(
            "before\n\n$$\n",
            "\\begin{aligned}\n",
            "y &= \\ln u, \\quad u = \\sin v, \\quad v = e^{2x}, \\quad w = 2x \\\\[4pt]\n",
            "\\frac{dy}{dx} &= \\frac{dy}{du} \\cdot \\frac{du}{dv} \\cdot \\frac{dv}{dw} \\cdot \\frac{dw}{dx} \\\\[4pt]\n",
            "&= \\frac{1}{u} \\cdot \\cos v \\cdot e^{w} \\cdot 2 \\\\[4pt]\n",
            "&= \\frac{1}{\\sin(e^{2x})} \\cdot \\cos(e^{2x}) \\cdot e^{2x} \\cdot 2 \\\\[4pt]\n",
            "&= \\frac{2e^{2x} \\cos(e^{2x})}{\\sin(e^{2x})} = 2e^{2x} \\cot(e^{2x})\n",
            "\\end{aligned}\n",
            "$$\n\nafter",
        );
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 12,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    120,
                    12,
                    now,
                    ExecutionStatus::Completed,
                    FixturePart::text(aligned),
                )]),
            }],
            ..TranscriptState::default()
        };
        let blocks = transcript
            .rendered(100)
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .map(|(index, node)| (index, node.clone()))
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 3);
        assert!(
            !blocks[1].1.atomic,
            "a structurally multi-row formula block must be enterable"
        );

        transcript.set_block_cursor(100, 30, blocks[0].0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(100, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.key.clone()),
            "the whole formula block remains the first hierarchy stop"
        );

        let expected = [
            r"y &= \ln u, \quad u = \sin v, \quad v = e^{2x}, \quad w = 2x",
            r"\frac{dy}{dx} &= \frac{dy}{du} \cdot \frac{du}{dv} \cdot \frac{dv}{dw} \cdot \frac{dw}{dx}",
            r"&= \frac{1}{u} \cdot \cos v \cdot e^{w} \cdot 2",
            r"&= \frac{1}{\sin(e^{2x})} \cdot \cos(e^{2x}) \cdot e^{2x} \cdot 2",
            r"&= \frac{2e^{2x} \cos(e^{2x})}{\sin(e^{2x})} = 2e^{2x} \cot(e^{2x})",
        ];
        for source in expected {
            transcript.move_cursor_one_line(100, 30, TranscriptMoveDirection::Down);
            assert_eq!(transcript.highlighted_block_key(), None);
            assert_eq!(
                transcript.current_selected_line_text(100).as_deref(),
                Some(source)
            );
        }
        transcript.move_cursor_one_line(100, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[2].1.key.clone()),
            "leaving the last equation row selects the next Markdown block"
        );
    }

    #[test]
    fn extended_aligned_formula_is_enterable_from_both_directions_end_to_end() {
        let now = Utc::now();
        let source = concat!(
            "before\n\n$$\n",
            "\\begin{aligned}\n",
            "\\lim_{x \\to 0} \\frac{e^x - 1 - x}{x^2}\n",
            "&\\xlongequal{\\text{L'Hôpital}} \\lim_{x \\to 0} \\frac{e^x - 1}{2x} \\\\\n",
            "&\\xlongequal{\\text{L'Hôpital}} \\lim_{x \\to 0} \\frac{e^x}{2}\n",
            "= \\frac{1}{2}\n",
            "\\end{aligned}\n",
            "$$\n\nafter",
        );
        let mut transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 13,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    130,
                    13,
                    now,
                    ExecutionStatus::Completed,
                    FixturePart::text(source),
                )]),
            }],
            ..TranscriptState::default()
        };
        let blocks = transcript
            .rendered(80)
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
            .map(|(index, node)| (index, node.clone()))
            .collect::<Vec<_>>();
        assert_eq!(blocks.len(), 3);
        assert!(!blocks[1].1.atomic, "extended formula must not be atomic");
        let equation_lines = transcript.rendered(80).lines
            [blocks[1].1.start_line..blocks[1].1.end_line]
            .iter()
            .filter_map(|line| {
                line.navigation_unit
                    .map(|unit| (unit, line.navigation_copy_text.clone()))
            })
            .fold(Vec::<(usize, String)>::new(), |mut rows, row| {
                if rows.last().is_none_or(|previous| previous.0 != row.0) {
                    rows.push(row);
                }
                rows
            });
        assert_eq!(equation_lines.len(), 2);

        transcript.set_block_cursor(80, 30, blocks[0].0, TranscriptMoveDirection::Down);
        transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.key.clone())
        );
        transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            transcript.current_selected_line_text(80),
            Some(equation_lines[0].1.clone())
        );

        transcript.set_block_cursor(80, 30, blocks[2].0, TranscriptMoveDirection::Up);
        transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Up);
        assert_eq!(
            transcript.highlighted_block_key(),
            Some(blocks[1].1.key.clone())
        );
        transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Up);
        assert_eq!(
            transcript.current_selected_line_text(80),
            Some(equation_lines[1].1.clone())
        );

        let config = agena_tui_media::MathLayoutConfig {
            native_graphics: true,
            cell_width: 10,
            cell_height: 20,
            ..agena_tui_media::MathLayoutConfig::default()
        };
        let context = agena_tui_media::test_support::test_math_render_context(config);
        let mut native_transcript = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 14,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    140,
                    14,
                    now,
                    ExecutionStatus::Completed,
                    FixturePart::text(source),
                )]),
            }],
            ..TranscriptState::default()
        };
        agena_tui_media::with_math_render_context(&context, || {
            let native_blocks = native_transcript
                .rendered(80)
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| matches!(node.key, TranscriptNodeKey::MarkdownBlock { .. }))
                .map(|(index, node)| (index, node.clone()))
                .collect::<Vec<_>>();
            assert_eq!(native_blocks.len(), 3);
            assert!(
                !native_blocks[1].1.atomic,
                "native end-to-end formula must not be atomic"
            );
            native_transcript.set_block_cursor(
                80,
                30,
                native_blocks[0].0,
                TranscriptMoveDirection::Down,
            );
            native_transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
            native_transcript.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
            assert_eq!(
                native_transcript.current_selected_line_text(80),
                Some(equation_lines[0].1.clone())
            );
        });

        let mut formula_only = TranscriptState {
            session_id: Some(7),
            messages: vec![RunResource {
                id: 15,
                session_id: 7,
                role: RunRole::Assistant,
                state: RunStatus::Completed,
                created_at: now,
                updated_at: now,
                metadata: Default::default(),
                usage: None,
                part_count: 1,
                parts: Some(vec![api_message_part!(
                    150,
                    15,
                    now,
                    ExecutionStatus::Completed,
                    FixturePart::text(concat!(
                        "$$\n",
                        "\\begin{aligned}\n",
                        "\\lim_{x \\to 0} \\frac{e^x - 1 - x}{x^2}\n",
                        "&\\xlongequal{\\text{L'Hôpital}} \\lim_{x \\to 0} \\frac{e^x - 1}{2x} \\\\\n",
                        "&\\xlongequal{\\text{L'Hôpital}} \\lim_{x \\to 0} \\frac{e^x}{2}\n",
                        "= \\frac{1}{2}\n",
                        "\\end{aligned}\n",
                        "$$",
                    )),
                )]),
            }],
            ..TranscriptState::default()
        };
        let (message_index, formula_index, formula_key, formula_start) = {
            let rendered = formula_only.rendered(80);
            let message_index = rendered
                .nodes
                .iter()
                .position(|node| matches!(node.key, TranscriptNodeKey::Entry { .. }))
                .expect("message node");
            let (formula_index, formula) = rendered
                .nodes
                .iter()
                .enumerate()
                .find(|(_, node)| node.kind == TranscriptNodeKind::MarkdownMath)
                .expect("formula node");
            assert!(!formula.atomic);
            (
                message_index,
                formula_index,
                formula.key.clone(),
                formula.start_line,
            )
        };
        formula_only.set_block_cursor(80, 30, message_index, TranscriptMoveDirection::Down);
        formula_only.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
        assert_eq!(
            formula_only.highlighted_block_key(),
            None,
            "a message's only child must not create a duplicate whole-formula stop: {formula_key:?}"
        );
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[0].1.clone())
        );

        formula_only.set_block_cursor(80, 30, message_index, TranscriptMoveDirection::Up);
        formula_only.move_cursor_one_line(80, 30, TranscriptMoveDirection::Up);
        assert_eq!(formula_only.highlighted_block_key(), None);
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[1].1.clone())
        );

        formula_only.select_pointer_block(
            80,
            30,
            TranscriptTextPosition {
                line: formula_start,
                column: 2,
            },
        );
        assert_eq!(
            formula_only.highlighted_block_key(),
            Some(formula_key.clone())
        );
        formula_only.move_cursor_one_line(80, 30, TranscriptMoveDirection::Down);
        assert_eq!(formula_only.highlighted_block_key(), None);
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[0].1.clone())
        );
        formula_only.select_pointer_block(
            80,
            30,
            TranscriptTextPosition {
                line: formula_start,
                column: 2,
            },
        );
        formula_only.move_cursor_one_line(80, 30, TranscriptMoveDirection::Up);
        assert_eq!(formula_only.highlighted_block_key(), None);
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[1].1.clone())
        );
        assert_eq!(
            formula_only
                .rendered(80)
                .nodes
                .get(formula_index)
                .map(|node| node.key.clone()),
            Some(formula_key)
        );

        let second_equation_line = formula_only
            .rendered(80)
            .lines
            .iter()
            .position(|line| line.navigation_copy_text == equation_lines[1].1)
            .expect("second equation display row");
        formula_only.select_pointer_line(
            80,
            30,
            TranscriptTextPosition {
                line: second_equation_line,
                column: 2,
            },
        );
        assert_eq!(formula_only.highlighted_block_key(), None);
        assert_eq!(
            formula_only.current_selected_line_text(80),
            Some(equation_lines[1].1.clone())
        );
    }
}

#[cfg(test)]
mod live_transcript_tests {
    use agena_api::resource::SessionTranscriptPart;
    use agena_domain::{
        ActivityId, ActivityOwner, ActivityState, AssistantReplyId, ComposerDocument, ComposerNode,
        ContentNode, TextSegmentId, TranscriptPatch,
    };
    use agena_runtime::{
        RuntimePresentationEvent, RuntimePresentationEventKind, RuntimePresentationEventMeta,
    };
    use uuid::Uuid;

    use super::super::{PendingUserMessage, TranscriptState, Utc};
    use super::parts_fixtures;

    fn event(kind: RuntimePresentationEventKind, seq: i64) -> RuntimePresentationEvent {
        RuntimePresentationEvent {
            meta: RuntimePresentationEventMeta {
                id: Uuid::new_v4(),
                seq_global: seq,
                seq_session: Some(seq),
                session_id: Some(7),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: 1,
            },
            invalidates_ancestor_projection: false,
            durable: true,
            kind,
        }
    }

    fn text_patch(
        response_id: AssistantReplyId,
        segment_id: TextSegmentId,
        text: &str,
        seq: i64,
    ) -> RuntimePresentationEvent {
        event(
            RuntimePresentationEventKind::TranscriptPatch(Box::new(
                TranscriptPatch::ContentUpserted {
                    seq_session: seq,
                    owner: ActivityOwner::AssistantReply {
                        reply_id: response_id,
                    },
                    node: ContentNode::text_at(segment_id, text, 0, seq),
                },
            )),
            seq,
        )
    }

    /// A completed assistant run carrying a tall multi-line body, used to
    /// exercise viewport follow/recovery across full parts refreshes. Each
    /// line is its own markdown paragraph so the v2 renderer keeps them as
    /// separate focusable rows instead of folding them into soft breaks.
    fn tall_assistant(prefix: &str, run_id: i64, text_id: i64) -> Vec<SessionTranscriptPart> {
        let body = (0..40)
            .map(|line| format!("{prefix} line {line}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        vec![
            parts_fixtures::run(run_id, "assistant", "completed"),
            parts_fixtures::text(text_id, "assistant", &body),
        ]
    }

    #[test]
    fn transcript_patch_triggers_a_parts_reload() {
        let response_id = AssistantReplyId::new();
        let segment_id = TextSegmentId::new();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "user", "completed"),
                parts_fixtures::text(2, "user", "question"),
                parts_fixtures::run(3, "assistant", "in_progress"),
            ],
            ..TranscriptState::default()
        };

        // v2 has no incremental transcript patch surface: a live patch only
        // signals that the terminal must reload the full part list, and the
        // reloaded projection carries the streamed text.
        assert!(
            transcript.apply_presentation_event(
                &text_patch(response_id, segment_id, "I'm Grok", 2),
                80,
                20,
            ),
            "a transcript patch must request a full parts reload"
        );
        transcript.merge_parts(vec![
            parts_fixtures::run(1, "user", "completed"),
            parts_fixtures::text(2, "user", "question"),
            parts_fixtures::run(3, "assistant", "completed"),
            parts_fixtures::text(4, "assistant", "I'm Grok"),
        ]);

        let rendered = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("Grok"),
            "rendered transcript: {rendered:?}"
        );
    }

    #[test]
    fn live_only_events_do_not_advance_the_durable_watermark() {
        let response_id = AssistantReplyId::new();
        let segment_id = TextSegmentId::new();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "user", "completed"),
                parts_fixtures::text(2, "user", "question"),
                parts_fixtures::run(3, "assistant", "in_progress"),
            ],
            ..TranscriptState::default()
        };

        // A refresh-triggering transcript patch does not carry the durable
        // sequence: the terminal reloads the full part list and the next
        // execution updates the watermark. Counting it here would make every
        // later refresh look stale.
        assert!(transcript.apply_presentation_event(
            &text_patch(response_id, segment_id, "hello", 2),
            80,
            20,
        ));
        assert_eq!(transcript.last_event_seq, None);

        // A live-only ActivityV2 event at a HIGHER seq must not advance it:
        // the server's durable `latest_event_seq` never includes live-only
        // events, so counting one would make every later refresh look stale
        // and drop the terminal execution + completed reply.
        let live = RuntimePresentationEvent {
            meta: RuntimePresentationEventMeta {
                id: Uuid::new_v4(),
                seq_global: 50,
                seq_session: Some(50),
                session_id: Some(7),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: 1,
            },
            invalidates_ancestor_projection: false,
            durable: false,
            kind: RuntimePresentationEventKind::ActivityV2(Box::new(
                agena_runtime::session::activity::ActivityLiveEvent::StateChanged {
                    activity_id: ActivityId::new(),
                    state: ActivityState::Completed,
                },
            )),
        };
        assert!(!transcript.apply_presentation_event(&live, 80, 20));
        assert_eq!(
            transcript.last_event_seq, None,
            "live-only events must not advance the durable watermark"
        );
    }

    #[test]
    fn repeated_parts_refreshes_replace_one_stable_segment() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "user", "completed"),
                parts_fixtures::text(2, "user", "question"),
                parts_fixtures::run(3, "assistant", "in_progress"),
                parts_fixtures::text(4, "assistant", "first"),
            ],
            ..TranscriptState::default()
        };

        // A full parts refresh replaces the streamed segment wholesale; the
        // projection must end up with exactly one assistant text segment.
        transcript.merge_parts(vec![
            parts_fixtures::run(1, "user", "completed"),
            parts_fixtures::text(2, "user", "question"),
            parts_fixtures::run(3, "assistant", "completed"),
            parts_fixtures::text(4, "assistant", "first second"),
        ]);
        assert_eq!(
            transcript
                .parts
                .iter()
                .filter(|part| part.role == "assistant" && part.kind == "text")
                .count(),
            1,
            "a refresh must replace the assistant text segment"
        );

        let rendered = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains("first second"));
        assert_eq!(rendered.matches("first second").count(), 1);
    }

    #[test]
    fn user_input_materialization_replaces_the_optimistic_entry() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "assistant", "completed"),
                parts_fixtures::text(2, "assistant", "a reply"),
            ],
            ..TranscriptState::default()
        };
        transcript.add_pending_user_message(PendingUserMessage {
            id: 45,
            document: ComposerDocument(vec![ComposerNode::Text {
                text: "one live message".to_owned(),
            }]),
            confirmed: true,
        });
        assert_eq!(transcript.pending_user_messages.len(), 1);

        // The durable projection now carries the user run with its text; the
        // optimistic entry is reconciled away.
        transcript.merge_parts(vec![
            parts_fixtures::run(1, "user", "completed"),
            parts_fixtures::text(2, "user", "one live message"),
            parts_fixtures::run(3, "assistant", "completed"),
            parts_fixtures::text(4, "assistant", "a reply"),
        ]);
        assert!(transcript.pending_user_messages.is_empty());
        let rendered = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            rendered.matches("one live message").count(),
            1,
            "{rendered}"
        );
    }

    #[test]
    fn refresh_merge_keeps_follow_tail_pinned_to_the_bottom() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: tall_assistant("first", 1, 2),
            ..TranscriptState::default()
        };
        transcript.scroll_to_bottom(80, 20);
        let bottom_before = transcript.viewport.top;
        assert!(transcript.viewport.follow_tail);

        // A periodic refresh merges the full part list without a live patch.
        let mut refreshed = tall_assistant("first", 1, 2);
        refreshed.extend(tall_assistant("reply", 3, 4));
        transcript.merge_parts(refreshed);

        // The next render runs clamp_scroll before ensure_visual_focus.
        transcript.clamp_scroll(80, 20);
        transcript.ensure_visual_focus(80, 20);

        assert!(
            transcript.viewport.follow_tail,
            "follow mode must survive a full-state refresh merge"
        );
        let max_scroll = transcript.max_scroll(80, 20);
        assert_eq!(
            transcript.viewport.top, max_scroll,
            "a following viewport must pin to the new bottom"
        );
        assert!(
            transcript.viewport.top > bottom_before,
            "the new reply must move the viewport downward"
        );
        assert_eq!(
            transcript.navigation_cursor_line(),
            Some(transcript.rendered(80).lines.len().saturating_sub(1)),
            "the cursor must re-anchor to the new tail"
        );
    }

    #[test]
    fn refresh_merge_does_not_hijack_a_scrolled_up_viewport() {
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: tall_assistant("first", 1, 2),
            ..TranscriptState::default()
        };
        transcript.scroll_to_bottom(80, 20);
        assert!(transcript.viewport.follow_tail);

        // The user scrolls up to read history while the reply streams in.
        transcript.move_cursor_by_wheel(80, 20, -4);
        assert!(!transcript.viewport.follow_tail);
        let reading_top = transcript.viewport.top;

        let mut refreshed = tall_assistant("first", 1, 2);
        refreshed.extend(tall_assistant("reply", 3, 4));
        transcript.merge_parts(refreshed);
        transcript.clamp_scroll(80, 20);
        transcript.ensure_visual_focus(80, 20);

        assert!(
            !transcript.viewport.follow_tail,
            "reading history must stay detached from the tail"
        );
        assert_eq!(
            transcript.viewport.top, reading_top,
            "new content must not drag a scrolled-up viewport"
        );
    }
    #[test]
    fn failed_reply_failure_survives_a_recovering_continue() {
        let problem = agena_failure::UserProblem::from(agena_failure::Failure::new(
            agena_failure::FailureCode::new("internal.test"),
            agena_failure::FailureCategory::Internal,
            agena_failure::FailureResponsibility::System,
            agena_failure::RetryDirective::ImmediateOnce,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new(
                "internal-test",
                "The provider response ended unexpectedly.",
            ),
        ));
        // A failed attempt usually leaves partial content behind; the failure
        // must still be visible after the reply recovers.
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "assistant", "failed"),
                parts_fixtures::error(2, "assistant", problem.clone()),
                parts_fixtures::text(
                    3,
                    "assistant",
                    "partial assistant output before the failure",
                ),
            ],
            ..TranscriptState::default()
        };

        // The failed terminal projection arrives and records the failure.
        transcript.merge_parts(transcript.parts.clone());
        assert_eq!(transcript.reply_failures.len(), 1);

        // /continue recovers: the run completes and drops the error part, but
        // the remembered failure stays injected into the assistant entry.
        transcript.merge_parts(vec![
            parts_fixtures::run(1, "assistant", "completed"),
            parts_fixtures::text(
                3,
                "assistant",
                "partial assistant output before the failure",
            ),
            parts_fixtures::text(4, "assistant", "recovered output"),
        ]);
        assert!(
            !transcript.parts.iter().any(|part| part.kind == "error"),
            "the recovering projection must drop the durable error part"
        );

        let rendered = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("The provider response ended unexpectedly."),
            "failure summary should survive continue: {rendered:?}"
        );
        assert!(
            transcript.rendered(80).nodes.iter().any(|node| {
                matches!(
                    node.key,
                    agena_tui_transcript::TranscriptNodeKey::Activity {
                        content_id: agena_tui_transcript::TranscriptContentId::StoredPart(_),
                        ..
                    }
                )
            }),
            "the remembered failure must stay rendered as an error activity"
        );
    }
    #[test]
    fn failed_reply_failure_is_recorded_from_the_parts_projection() {
        let problem = agena_failure::UserProblem::from(agena_failure::Failure::new(
            agena_failure::FailureCode::new("internal.test"),
            agena_failure::FailureCategory::Internal,
            agena_failure::FailureResponsibility::System,
            agena_failure::RetryDirective::ImmediateOnce,
            agena_failure::RecoveryDirective::Retry,
            agena_failure::FailureImpact::OperationFailed,
            agena_failure::UserPresentation::new(
                "internal-test",
                "The provider stream was interrupted.",
            ),
        ));
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "assistant", "failed"),
                parts_fixtures::text(
                    2,
                    "assistant",
                    "partial assistant output before the failure",
                ),
                parts_fixtures::error(3, "assistant", problem.clone()),
            ],
            ..TranscriptState::default()
        };

        // The failed projection records the failure without any live patch:
        // the terminal reloads the full part list and `record_reply_failures`
        // keys the error part under its run marker.
        transcript.merge_parts(transcript.parts.clone());
        assert_eq!(transcript.reply_failures.len(), 1);

        // /continue recovers the reply; the error part is gone from the parts
        // projection but the remembered failure keeps the summary visible.
        transcript.merge_parts(vec![
            parts_fixtures::run(1, "assistant", "completed"),
            parts_fixtures::text(
                2,
                "assistant",
                "partial assistant output before the failure",
            ),
            parts_fixtures::text(4, "assistant", "recovered output"),
        ]);
        assert!(
            !transcript.parts.iter().any(|part| part.kind == "error"),
            "the recovering projection must drop the durable error part"
        );

        let rendered = transcript
            .rendered(80)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("The provider stream was interrupted."),
            "failure summary should survive continue when recorded from the projection: {rendered:?}"
        );
    }

    #[test]
    fn reasoning_part_renders_the_full_trail_expanded() {
        // A long, multi-line reasoning body (the exact shape the runtime
        // persists as a `think` part). It must render through the dedicated
        // full-trail variant: expanded by default and never truncated to the
        // first line.
        let body = (0..40)
            .map(|line| format!("live thought line {line}"))
            .collect::<Vec<_>>();
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "assistant", "completed"),
                parts_fixtures::think(2, "assistant", body.clone()),
            ],
            ..TranscriptState::default()
        };

        let rendered = transcript.rendered(120);
        let text = rendered
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("live thought line 0"), "rendered: {text}");
        assert!(text.contains("live thought line 39"), "rendered: {text}");
        assert!(!text.contains("truncated"), "rendered: {text}");
        let node = rendered
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.key,
                    agena_tui_transcript::TranscriptNodeKey::Activity { .. }
                )
            })
            .expect("reasoning activity node");
        assert!(node.expanded, "reasoning must default to expanded");
        assert!(
            node.copy_text.contains("live thought line 39"),
            "expanded copy text must carry the full trail"
        );
    }

    #[test]
    fn pasted_text_and_file_attachment_parts_render_in_the_user_document() {
        let pasted = "x".repeat(1_000);
        let mut transcript = TranscriptState {
            session_id: Some(7),
            parts: vec![
                parts_fixtures::run(1, "user", "completed"),
                parts_fixtures::paste(2, "user", &pasted),
                parts_fixtures::file_ref(3, "user", "notes.txt"),
                parts_fixtures::text(4, "user", "review this paste"),
                parts_fixtures::run(5, "assistant", "completed"),
                parts_fixtures::text(6, "assistant", "ok"),
            ],
            ..TranscriptState::default()
        };

        let lines = transcript
            .rendered(100)
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>();
        let joined = lines.join("\n");
        // The pasted text renders as a synthetic body segment of the user
        // document; a long contiguous run must survive line wrapping.
        let pasted_head = "x".repeat(64);
        assert!(
            joined.contains(&pasted_head),
            "the pasted body must render: {joined}"
        );
        assert!(
            joined.contains("review this paste"),
            "the user document text must render: {joined}"
        );
        assert!(
            lines.iter().any(|line| line.contains("notes.txt")),
            "file attachment must render: {joined}"
        );
        let document_line = lines
            .iter()
            .position(|line| line.contains("review this paste"))
            .expect("user document");
        let attachment_line = lines
            .iter()
            .position(|line| line.contains("notes.txt"))
            .expect("attachment");
        assert!(
            attachment_line < document_line,
            "the attachment part must render above the user text: {joined}"
        );
    }
}

#[cfg(test)]
mod new_session_model_stack_tests {
    use agena_api::resource::{SessionLifecycleState, SessionRelationKind, SessionResource};
    use agena_application::Application;
    use agena_runtime::{RuntimeBootstrapRequest, bootstrap_application_services};
    use chrono::Utc;
    use ratatui::layout::Rect;

    use super::super::{App, ComposerDraft, I18n, LaunchOptions};

    fn session_resource() -> SessionResource {
        SessionResource {
            id: 7,
            parent_id: None,
            depth: 0,
            root_id: 7,
            workspace_id: 1,
            title: "Test session".to_owned(),
            version: 1,
            relation_kind: SessionRelationKind::Root,
            lifecycle_state: SessionLifecycleState::Ready,
            source_cutoff_seq_global: None,
            source_message_id: None,
            is_subagent: false,
            task_id: None,
            subtask_access: None,
            subtask_status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            message_count: 1,
            child_session_count: 0,
            last_message_at: None,
        }
    }

    async fn app_without_session() -> App {
        let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
            workspace_root: Some(std::env::temp_dir()),
            database_url: Some("sqlite::memory:".to_owned()),
            initialize_schema: true,
            ..RuntimeBootstrapRequest::default()
        })
        .await
        .expect("build test runtime");
        let application = Application::from_composed_runtime_services(runtime.application_services())
            .expect("compose test application");
        let mut app = App::new(application, LaunchOptions::default(), I18n::english());
        app.layout.transcript_body = Rect::new(0, 0, 80, 24);
        app
    }

    #[tokio::test]
    async fn auto_created_session_first_run_keeps_the_switched_model() {
        let mut app = app_without_session().await;
        // The user switched the model with no session open: the stack is only
        // in-memory (nothing to persist to). The send then auto-creates a
        // session; `open_session` would clear this stack before the first
        // submit reads it, so `create_session` carries it through
        // `AppMessage::SessionCreated` and this handler restores it.
        app.run_options.replace_model_stack(
            Some(agena_domain::ModelRef::new("acme", "acme-fast")),
            Some("high".to_owned()),
            Some("fast".to_owned()),
            Some("verbose".to_owned()),
            Some(true),
        );
        let model_stack = app.run_options.clone();
        let draft = ComposerDraft {
            document: agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "hello".to_owned(),
            }]),
        };

        app.handle_session_created(Some(draft), None, Some(model_stack), Ok(session_resource()));

        assert_eq!(
            app.run_options.model.as_ref().map(|model| model.model_id.to_string()),
            Some("acme-fast".to_owned()),
            "the first submit of the auto-created session must use the switched model, not the default"
        );
        assert_eq!(app.run_options.thinking_mode.as_deref(), Some("high"));
        assert_eq!(app.run_options.speed_mode.as_deref(), Some("fast"));
        assert_eq!(app.run_options.verbosity.as_deref(), Some("verbose"));
        assert_eq!(app.run_options.parallel_tool_calls, Some(true));
    }

    #[tokio::test]
    async fn manual_new_session_does_not_restore_a_stale_model_stack() {
        let mut app = app_without_session().await;
        // A stale stack from a previous session must still be cleared when a
        // new session is created explicitly (no submit draft) — the fix only
        // restores the stack captured for an auto-created send.
        app.run_options.replace_model_stack(
            Some(agena_domain::ModelRef::new("old-provider", "old-model")),
            None,
            None,
            None,
            None,
        );

        app.handle_session_created(None, None, None, Ok(session_resource()));

        assert!(
            app.run_options.model.is_none(),
            "an explicit new-session must not inherit a stale model stack"
        );
    }
}
