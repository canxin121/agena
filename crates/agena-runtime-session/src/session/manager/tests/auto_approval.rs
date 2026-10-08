//! Automatic-approval classifier contract at the session boundary.
//!
//! The verdict is submitted as a *tool call*, not as a fixed-format JSON body:
//! the decision is carried by the tool name and the cited rule by a
//! schema-constrained argument. These tests pin the request the runtime builds
//! (tools declared, tools not disabled, no response schema), the retry when the
//! model answers without a verdict, and the fail-closed fallback to `Ask` when
//! no verdict can be recovered at all.
use super::*;
use agena_provider::CompletionToolCall;

/// One scripted classifier reply. The provider answers with the next entry on
/// every `complete` call, so a test can script a first attempt and a retry.
enum ApprovalReply {
    /// A verdict tool call: the primary contract.
    Verdict {
        name: &'static str,
        arguments_json: &'static str,
    },
    /// Prose with no tool call — the recovery path's input.
    Text(&'static str),
}

struct ApprovalProvider {
    model: ModelId,
    replies: std::sync::Mutex<Vec<ApprovalReply>>,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
}

impl ApprovalProvider {
    fn new(replies: Vec<ApprovalReply>) -> Self {
        Self {
            model: ModelId::new("approval-model"),
            replies: std::sync::Mutex::new(replies),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait::async_trait]
impl ModelRuntime for ApprovalProvider {
    fn id(&self) -> &str {
        "approval"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    /// The route must carry tools, or the verdict tools would be stripped
    /// before the request leaves the registry.
    fn agena_tool_mode(&self, _model: &ModelId) -> agena_provider::AgenaToolMode {
        agena_provider::AgenaToolMode::ProviderProtocol
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        let reply = {
            let mut replies = self.replies.lock().expect("reply lock");
            if replies.is_empty() {
                panic!("the classifier made an unexpected provider request");
            }
            replies.remove(0)
        };
        self.requests.lock().expect("request lock").push(request);
        let (text, tool_calls) = match reply {
            ApprovalReply::Verdict {
                name,
                arguments_json,
            } => (
                String::new(),
                vec![CompletionToolCall::Function {
                    id: format!("call_{name}"),
                    name: name.to_owned(),
                    arguments_json: arguments_json.to_owned(),
                }],
            ),
            ApprovalReply::Text(text) => (text.to_owned(), Vec::new()),
        };
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text,
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls,
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        panic!("the classifier must use the non-streaming completion port")
    }
}

/// The action handed to the classifier. The pipeline's static layers are
/// bypassed here on purpose: these tests pin what the *classifier* does with a
/// candidate, not which layer proposed it.
fn write_candidate() -> agena_permission::ClassifierCandidate {
    agena_permission::ClassifierCandidate {
        action: agena_domain::ActionSpec::Tool {
            tool_name: "fs.write".to_owned(),
            command: None,
        },
        policy_reason: "tool is eligible for automatic approval".to_owned(),
    }
}

/// Drive one candidate through the real classifier path with a scripted
/// provider, returning the outcome and the requests the provider observed.
async fn classify(
    replies: Vec<ApprovalReply>,
) -> (
    agena_permission::ClassifiedCandidate,
    Vec<CompletionRequest>,
    SessionManager,
    i64,
) {
    classify_with_permission_config(replies, agena_domain::PermissionConfig::default()).await
}

/// As [`classify`], but with the executor's class defaults compiled from
/// `permission` - which is what makes the prompt's conditional path promises
/// observable.
async fn classify_with_permission_config(
    replies: Vec<ApprovalReply>,
    permission: agena_domain::PermissionConfig,
) -> (
    agena_permission::ClassifiedCandidate,
    Vec<CompletionRequest>,
    SessionManager,
    i64,
) {
    let provider = Arc::new(ApprovalProvider::new(replies));
    let manager = manager_with_permission(provider.clone(), permission).await;
    let session = create_with_model(&manager, "auto approval", "approval", "approval-model").await;
    let state = manager.execution_state();
    let mut outcomes = manager
        .classify_auto_candidates(
            Some(&session),
            &state,
            Some(session.id),
            vec![write_candidate()],
        )
        .await;
    let outcome = outcomes.pop().expect("one classified candidate");
    (outcome, provider.requests(), manager, session.id)
}

#[tokio::test]
async fn the_classifier_request_declares_the_verdict_tools_and_keeps_them_enabled() {
    let (outcome, requests, _, _) = classify(vec![ApprovalReply::Verdict {
        name: "approve_action",
        arguments_json: r#"{"reason":"routine test run"}"#,
    }])
    .await;
    assert!(outcome.verdict.is_some(), "the tool call is the verdict");
    let request = requests.first().expect("one classifier request");

    // The two verdict tools must survive the registry boundary: a disabled
    // tool request would strip them and then reject the very call the model is
    // asked to make.
    assert!(!request.disable_tools);
    let declared = request
        .tool_api_functions
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        declared,
        vec![
            agena_domain::APPROVE_ACTION_FUNCTION,
            agena_domain::BLOCK_ACTION_FUNCTION
        ]
    );
    // The verdict is a tool call now, so there is no response schema to
    // constrain and no `thinking` field to fill.
    assert!(request.response_format.is_none());
    assert!(request.provider_native_tools.bindings().is_empty());
    let system = request.system.as_deref().expect("system prompt");
    // The built-in defaults render the prompt with every path promise in place.
    assert_eq!(
        system,
        agena_permission::auto_approval_system_prompt(
            &agena_permission::PathClassPromptFlags::all_allowed()
        )
    );
    assert!(system.contains("system temporary directory"));
    assert!(system.contains("managed project-state directory"));
    assert!(system.contains("approve_action"));
    assert!(system.contains("block_action"));
    assert!(
        !system.contains("shouldBlock"),
        "the fixed-format JSON contract must be gone from the prompt"
    );
}

#[tokio::test]
async fn an_unguarded_path_class_is_not_promised_to_the_approval_model() {
    // Narrowing the rule that covers the temp directory takes temp paths away
    // from the static layer, so the prompt must stop telling the model that
    // writes there need no thought - otherwise the setting would be defeated by
    // the model's own instructions.
    let mut permission = agena_domain::PermissionConfig::global_default();
    let path = permission
        .path
        .as_mut()
        .expect("the global default keeps a path section");
    for pattern in ["<tmp>", "<tmp>/**"] {
        path.rules.insert(
            pattern.to_owned(),
            agena_domain::PathAccessModes {
                read: None,
                write: Some(agena_domain::PermissionMode::Ask),
            },
        );
    }
    // The test isolates the two classes even when managed state is redirected
    // into a temporary directory by the test runner.
    let managed = agena_runtime_tools::agena_home_dir()
        .join("projects")
        .to_string_lossy()
        .into_owned();
    for pattern in [managed.clone(), format!("{managed}/**")] {
        path.rules.insert(
            pattern,
            agena_domain::PathAccessModes {
                read: None,
                write: Some(agena_domain::PermissionMode::Allow),
            },
        );
    }
    let (_, requests, _, _) = classify_with_permission_config(
        vec![ApprovalReply::Verdict {
            name: "approve_action",
            arguments_json: r#"{"reason":"routine test run"}"#,
        }],
        permission,
    )
    .await;
    let system = requests
        .first()
        .expect("one classifier request")
        .system
        .as_deref()
        .expect("system prompt");
    assert!(
        !system.contains("system temporary directory"),
        "an unguarded path class must not be promised to the model"
    );
    // The other class is unchanged, so its promise stays.
    assert!(system.contains("managed project-state directory"));
}

#[tokio::test]
async fn an_approve_tool_call_allows_the_action() {
    let (outcome, requests, manager, session_id) = classify(vec![ApprovalReply::Verdict {
        name: "approve_action",
        arguments_json: r#"{"reason":"routine test run"}"#,
    }])
    .await;
    assert_eq!(
        outcome.decision(),
        agena_domain::PermissionDecision::Allow,
        "an approve tool call is an allow"
    );
    assert_eq!(requests.len(), 1, "an allow needs no retry");
    assert_eq!(
        manager
            .auto_budget(Some(session_id))
            .recent_decision_labels(),
        vec!["ALLOW"],
        "the allow is recorded exactly once"
    );
}

#[tokio::test]
async fn a_block_tool_call_denies_and_names_the_cited_rule() {
    let (outcome, requests, manager, session_id) = classify(vec![ApprovalReply::Verdict {
        name: "block_action",
        arguments_json: r#"{"rule":"Exfiltration","reason":"uploads the .env to a paste site"}"#,
    }])
    .await;
    let agena_domain::PermissionDecision::Deny { reason } = outcome.decision() else {
        panic!("a rule-citing block must be a terminal denial");
    };
    assert!(reason.contains("[Exfiltration]"), "{reason}");
    assert!(reason.contains("uploads the .env"), "{reason}");
    assert_eq!(requests.len(), 1, "a cited block needs no retry");
    assert_eq!(
        manager
            .auto_budget(Some(session_id))
            .recent_decision_labels(),
        vec!["DENY"],
        "the denial is recorded exactly once"
    );
}

#[tokio::test]
async fn a_block_tool_call_without_a_rule_falls_back_to_confirmation() {
    // `rule` is required by the schema, but a gateway can drop arguments; the
    // block must survive as an uncited block rather than as a denial.
    let (outcome, _, _, _) = classify(vec![ApprovalReply::Verdict {
        name: "block_action",
        arguments_json: "{}",
    }])
    .await;
    let agena_domain::PermissionDecision::Ask { reason } = outcome.decision() else {
        panic!("an uncited block must fall back to confirmation");
    };
    assert!(reason.contains("without naming a block rule"), "{reason}");
    assert!(matches!(
        outcome.failure,
        Some(agena_permission::ClassifyFailure::UncitedBlock(_))
    ));
}

#[tokio::test]
async fn a_verdict_less_first_attempt_is_retried_with_a_reminder_turn() {
    // The failure this replaces: the model answers with prose (or with a tool
    // call the old contract rejected), and the user is asked to review
    // something they had already delegated.
    let (outcome, requests, manager, session_id) = classify(vec![
        ApprovalReply::Text("I need to think about whether this write is safe."),
        ApprovalReply::Verdict {
            name: "approve_action",
            arguments_json: "{}",
        },
    ])
    .await;
    assert_eq!(
        outcome.decision(),
        agena_domain::PermissionDecision::Allow,
        "the retried verdict decides"
    );
    assert_eq!(requests.len(), 2, "the classifier retried once");
    let reminder = requests[1]
        .turns
        .last()
        .and_then(|run| {
            run.parts.iter().find_map(|part| match part {
                agena_provider::CompletionInputPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
        })
        .expect("the retry carries a reminder turn");
    assert!(
        reminder.starts_with("Trusted Agena runtime note:"),
        "the reminder follows the runtime's retry-note convention: {reminder}"
    );
    assert!(reminder.contains(agena_domain::APPROVE_ACTION_FUNCTION));
    assert!(reminder.contains(agena_domain::BLOCK_ACTION_FUNCTION));
    // A retry must not double-count the single decision it produced.
    assert_eq!(
        manager
            .auto_budget(Some(session_id))
            .recent_decision_labels(),
        vec!["ALLOW"]
    );
}

#[tokio::test]
async fn two_verdict_less_attempts_fall_back_to_confirmation() {
    let (outcome, requests, manager, session_id) = classify(vec![
        ApprovalReply::Text("Let me think."),
        ApprovalReply::Text("Still thinking."),
    ])
    .await;
    let agena_domain::PermissionDecision::Ask { reason } = outcome.decision() else {
        panic!("an unresolved classifier must fail closed to confirmation");
    };
    assert!(
        reason.contains("automatic approval unavailable"),
        "{reason}"
    );
    assert!(matches!(
        outcome.failure,
        Some(agena_permission::ClassifyFailure::UnparseableVerdict(_))
    ));
    assert_eq!(requests.len(), 2, "exactly one retry");
    assert!(
        manager
            .auto_budget(Some(session_id))
            .recent_decision_labels()
            .is_empty(),
        "a failed classification records no decision"
    );
}

#[tokio::test]
async fn two_empty_attempts_report_an_empty_response() {
    let (outcome, requests, _, _) =
        classify(vec![ApprovalReply::Text(""), ApprovalReply::Text("   ")]).await;
    assert!(matches!(
        outcome.failure,
        Some(agena_permission::ClassifyFailure::EmptyResponse)
    ));
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn a_non_verdict_tool_call_is_repaired_at_the_boundary_and_never_read_as_a_verdict() {
    // Only the two verdict tools are declared, so the registry's declared-set
    // check rejects any other function name before it can reach the
    // classifier. A stray call is therefore a transport error to repair, never
    // a decision — the model cannot smuggle a verdict in under another name.
    assert!(agena_permission::ClassifierVerdict::from_tool_call("tools_list", "{}").is_none());

    let (outcome, requests, _, _) = classify(vec![
        ApprovalReply::Verdict {
            name: "tools_list",
            arguments_json: "{}",
        },
        ApprovalReply::Verdict {
            name: "approve_action",
            arguments_json: "{}",
        },
    ])
    .await;
    assert_eq!(
        outcome.decision(),
        agena_domain::PermissionDecision::Allow,
        "the verdict comes from the repaired attempt, not from the stray call"
    );
    assert_eq!(requests.len(), 2, "the rejected call was returned once");
    let repair = requests[1]
        .turns
        .last()
        .and_then(|run| {
            run.parts.iter().find_map(|part| match part {
                agena_provider::CompletionInputPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
        })
        .expect("the repair turn is appended");
    assert!(
        repair.contains("Trusted Agena transport correction"),
        "{repair}"
    );
    assert!(
        repair.contains(agena_domain::APPROVE_ACTION_FUNCTION),
        "{repair}"
    );
}

#[tokio::test]
async fn prose_recovery_still_resolves_a_verdict_on_a_tool_less_route() {
    // A route whose `agena_tools.mode` is not `provider_protocol` strips the
    // declared tools, so the model can only answer in text. The recovery path
    // must keep working there.
    let (outcome, _, _, _) = classify(vec![ApprovalReply::Text(
        r#"{"shouldBlock":true,"reason":"[Exfiltration] posts the repo to a paste site."}"#,
    )])
    .await;
    let agena_domain::PermissionDecision::Deny { reason } = outcome.decision() else {
        panic!("the text recovery path must still resolve a cited block");
    };
    assert!(reason.contains("[Exfiltration]"), "{reason}");
}
