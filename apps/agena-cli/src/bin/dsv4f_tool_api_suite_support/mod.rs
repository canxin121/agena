use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use agena::{
    agent::{
        NetworkPermissionConfig, PathAccessModes, PathPermissionConfig, PermissionConfig,
        ToolPermissionConfig,
    },
    event::{EventFilter, EventKind, Scope, Subscription, bus::SubscriptionItem},
    message::{
        ExecutionStatus, OperationPart, PartContent, PendingInteractiveRequest, UserInputReply,
        UserInputReplyKind,
    },
    permission::{PermissionMode, PermissionReply},
    session::{
        Session, SessionCreateRequest, SessionExecutionReplyRequest, SessionPermissionReplyRequest,
        SessionUserMessageRequest,
    },
};
use anyhow::{Context, bail, ensure};
use serde_json::{Value, json};

use super::{
    Harness, MAX_EXACT_INVOCATION_ATTEMPTS, PendingReply, TOOLS_CALL_HANDLER_KEY,
    TOOLS_HELP_HANDLER_KEY, ToolApiOutcome,
};

impl Harness {
    pub(super) async fn create_session(
        &self,
        title: &str,
        execution_tool_keys: &[&str],
        permission: PermissionConfig,
    ) -> anyhow::Result<i64> {
        let session = self
            .manager
            .create_session(SessionCreateRequest {
                title: title.to_string(),
                parent_session_id: None,
            })
            .await
            .with_context(|| format!("create session for {title}"))?;
        let mut allowed = vec![
            TOOLS_HELP_HANDLER_KEY.to_string(),
            TOOLS_CALL_HANDLER_KEY.to_string(),
        ];
        allowed.extend(execution_tool_keys.iter().map(|name| (*name).to_string()));
        allowed.sort();
        allowed.dedup();
        self.manager
            .set_session_allowed_tools(session.id, allowed)
            .await
            .with_context(|| format!("allow Tool API functions + execution tools for {title}"))?;
        self.manager
            .set_session_permission(session.id, permission)
            .await
            .with_context(|| format!("set baseline permission for {title}"))?;
        Ok(session.id)
    }

    pub(super) async fn run_execution_tool(
        &self,
        session_id: i64,
        case: &str,
        tool_name: &str,
        input: Value,
        pending_reply: PendingReply,
        expect_success: bool,
    ) -> anyhow::Result<ToolApiOutcome> {
        self.run_execution_tool_with_timeout(
            session_id,
            case,
            tool_name,
            input,
            pending_reply,
            expect_success,
            self.case_timeout,
        )
        .await
    }

    /// Run an execution tool that implements the plugin streaming protocol and prove
    /// that its chunks become a live update on the model-visible outer
    /// `agena.tools.call` operation. The subscription is installed before
    /// the provider turn begins so this exercises the complete real-provider
    /// Tool API path rather than inspecting only the final persisted result.
    pub(super) async fn run_streaming_execution_tool(
        &self,
        session_id: i64,
        case: &str,
        tool_name: &str,
        input: Value,
        expected_text: &str,
    ) -> anyhow::Result<ToolApiOutcome> {
        let mut subscription = self
            .manager
            .event_bus()
            .subscribe(EventFilter::new(Scope::Session { session_id }));
        let outcome = self
            .run_execution_tool(
                session_id,
                case,
                tool_name,
                input.clone(),
                PendingReply::None,
                true,
            )
            .await?;
        assert_outer_tool_api_stream_update(&mut subscription, tool_name, &input, expected_text)
            .await?;
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_execution_tool_with_timeout(
        &self,
        session_id: i64,
        case: &str,
        tool_name: &str,
        input: Value,
        pending_reply: PendingReply,
        expect_success: bool,
        timeout: Duration,
    ) -> anyhow::Result<ToolApiOutcome> {
        let marker = format!("DSV4F_{}_OK", marker_for(case));
        let input_text = serde_json::to_string(&input)?;
        let outcome_instruction = if expect_success {
            "The requested invocation must succeed."
        } else {
            "The requested invocation is intentionally expected to fail after the test denies its nested permission. Once it fails, do not retry the call and do not request another permission; reply with the terminal marker."
        };
        for attempt in 1..=MAX_EXACT_INVOCATION_ATTEMPTS {
            let start_message_count = self.manager.get_session(session_id).await?.messages.len();
            let retry_notice = (attempt > 1).then_some(
                "A prior attempt omitted the exact required input and did not execute it. Correct that now; do not reuse an empty or partial object.",
            );
            let prompt = format!(
                "This is an automated Tool API integration test. Call the native function tools_help exactly once with {{\"tool\":{tool_name:?}}}. Then call the native function tools_call exactly once with this exact JSON object: {{\"tool\":{tool_name:?},\"input\":{input_text}}}. Every supplied key is mandatory even if the schema marks it optional. A preliminary/default call, an empty input object, a modified value, or any second call is a test failure. Do not call any other function. {outcome_instruction} {} After that one tool result, reply exactly {marker}.",
                retry_notice.unwrap_or_default(),
            );
            let session = self
                .run_model_turn(session_id, prompt, pending_reply, timeout)
                .await
                .with_context(|| format!("run model turn for {case} (attempt {attempt})"))?;
            match extract_tool_api_outcome(
                &session,
                start_message_count,
                tool_name,
                &input,
                &marker,
                expect_success,
            ) {
                Ok(outcome) => return Ok(outcome),
                Err(error)
                    if attempt < MAX_EXACT_INVOCATION_ATTEMPTS
                        && can_retry_missing_tool_api_call(
                            &session,
                            start_message_count,
                            tool_name,
                            &input,
                        ) =>
                {
                    let _ = error;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("verify Tool API trace for {case}"));
                }
            }
        }
        unreachable!("the exact-invocation retry loop either returns or errors")
    }

    pub(super) async fn run_native_tool_api_function(
        &self,
        session_id: i64,
        case: &str,
        model_function: &str,
        canonical_function: &str,
        input: Value,
        tool_marker: Option<&str>,
    ) -> anyhow::Result<ToolApiOutcome> {
        let marker = format!("DSV4F_{}_OK", marker_for(case));
        let input_text = serde_json::to_string(&input)?;
        for attempt in 1..=MAX_EXACT_INVOCATION_ATTEMPTS {
            let start_message_count = self.manager.get_session(session_id).await?.messages.len();
            let retry_notice = (attempt > 1).then_some(
                "A prior attempt did not execute the exact supplied JSON. Correct that now and do not use defaults or partial arguments.",
            );
            let prompt = format!(
                "This is an automated Tool API test. Call the native function {model_function} exactly once with {input_text}. Do not call any other function. {} After the function result, reply exactly {marker}.",
                retry_notice.unwrap_or_default(),
            );
            let session = self
                .run_model_turn(session_id, prompt, PendingReply::None, self.case_timeout)
                .await
                .with_context(|| format!("run model turn for {case} (attempt {attempt})"))?;
            match extract_native_outcome(
                &session,
                start_message_count,
                canonical_function,
                &input,
                &marker,
                tool_marker,
            ) {
                Ok(outcome) => return Ok(outcome),
                Err(error)
                    if attempt < MAX_EXACT_INVOCATION_ATTEMPTS
                        && can_retry_missing_native_invocation(
                            &session,
                            start_message_count,
                            canonical_function,
                            &input,
                        ) =>
                {
                    let _ = error;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("verify Tool API trace for {case}"));
                }
            }
        }
        unreachable!("the exact-invocation retry loop either returns or errors")
    }

    pub(super) async fn run_model_turn(
        &self,
        session_id: i64,
        prompt: String,
        pending_reply: PendingReply,
        timeout: Duration,
    ) -> anyhow::Result<Session> {
        self.run_model_turn_with_options(
            session_id,
            prompt,
            pending_reply,
            timeout,
            self.options.clone(),
        )
        .await
    }

    pub(super) async fn run_model_turn_with_options(
        &self,
        session_id: i64,
        prompt: String,
        pending_reply: PendingReply,
        timeout: Duration,
        options: agena::session::SessionRunOptions,
    ) -> anyhow::Result<Session> {
        let manager = Arc::clone(&self.manager);
        let mut run = tokio::spawn(async move {
            manager
                .submit_user_message(SessionUserMessageRequest::new(
                    session_id,
                    options,
                    vec![PartContent::text(prompt)],
                ))
                .await
        });
        let deadline = Instant::now() + timeout;
        let mut replied = false;
        loop {
            if run.is_finished() {
                break;
            }
            if Instant::now() >= deadline {
                run.abort();
                bail!("model turn exceeded {} seconds", timeout.as_secs());
            }
            let session = self.manager.get_session(session_id).await?;
            let pending = session.pending_interactive_requests().into_iter().next();
            if let Some(pending) = pending {
                if replied {
                    run.abort();
                    bail!("model turn requested more than one interactive reply: {pending:?}");
                }
                match (pending_reply, pending) {
                    (PendingReply::Input, PendingInteractiveRequest::UserInput { request }) => {
                        ensure!(
                            request.request_id.starts_with("host-input:"),
                            "expected host-input request, got {}",
                            request.request_id
                        );
                        let answers = request
                            .questions
                            .iter()
                            .map(|question| (question.id.clone(), vec!["TEST_OK".to_string()]))
                            .collect::<BTreeMap<_, _>>();
                        self.manager
                            .reply_user_input(SessionExecutionReplyRequest::new(
                                session_id,
                                self.options.clone(),
                                UserInputReply {
                                    request_id: request.request_id,
                                    kind: UserInputReplyKind::Submit,
                                    answers,
                                    reason: None,
                                },
                            ))
                            .await
                            .context("reply to host user-input request")?;
                        replied = true;
                    }
                    (
                        PendingReply::Permission(kind),
                        PendingInteractiveRequest::Permission { request },
                    ) => {
                        ensure!(
                            request.request_id.starts_with("host-permission:"),
                            "expected nested host permission request, got {}",
                            request.request_id
                        );
                        self.manager
                            .reply_permission(SessionPermissionReplyRequest::new(
                                session_id,
                                self.options.clone(),
                                PermissionReply {
                                    request_id: request.request_id,
                                    kind,
                                    reason: Some("dsv4f exhaustive Tool API suite".to_string()),
                                    scope: None,
                                },
                                Some("dsv4f_tool_api_suite".to_string()),
                            ))
                            .await
                            .context("reply to nested host permission request")?;
                        replied = true;
                    }
                    (PendingReply::None, pending) => {
                        run.abort();
                        bail!("unexpected interactive request in model turn: {pending:?}");
                    }
                    (_, pending) => {
                        run.abort();
                        bail!("unexpected interactive request type: {pending:?}");
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !matches!(pending_reply, PendingReply::None) && !replied {
            bail!("expected an interactive reply, but the model turn completed without one");
        }
        tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), &mut run)
            .await
            .context("model run join exceeded deadline")?
            .context("join model task")?
            .context("complete model run")
    }
}

pub(super) fn baseline_permission(loopback: PermissionMode) -> PermissionConfig {
    PermissionConfig {
        path: Some(PathPermissionConfig {
            workspace: Some(PathAccessModes {
                read: Some(PermissionMode::Allow),
                write: Some(PermissionMode::Allow),
            }),
            external: Some(PathAccessModes {
                read: Some(PermissionMode::Allow),
                write: Some(PermissionMode::Allow),
            }),
            ..Default::default()
        }),
        network: Some(NetworkPermissionConfig {
            internet: Some(PermissionMode::Allow),
            private: Some(PermissionMode::Allow),
            loopback: Some(loopback),
            ..Default::default()
        }),
        tools: Some(ToolPermissionConfig {
            default: Some(PermissionMode::Allow),
            ..Default::default()
        }),
    }
}

pub(super) fn marker_for(case: &str) -> String {
    case.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) fn operations_since(
    session: &Session,
    start_message_count: usize,
) -> Vec<OperationPart> {
    session
        .messages
        .iter()
        .skip(start_message_count)
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::Operation(operation)) => Some(operation.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn transcript_since(session: &Session, start_message_count: usize) -> String {
    session
        .messages
        .iter()
        .skip(start_message_count)
        .map(|message| message.as_text_lossy())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A real provider can occasionally emit only malformed/default function
/// arguments despite an exact prompt. Retrying is safe only if it never
/// reached the requested invocation *and* none of its attempted Tool API calls
/// completed, so a stateful execution tool cannot be run twice by the harness.
pub(super) fn can_retry_missing_tool_api_call(
    session: &Session,
    start_message_count: usize,
    tool_name: &str,
    input: &Value,
) -> bool {
    if session.blocked() {
        return false;
    }
    let expected = json!({"tool": tool_name, "input": input});
    let calls = operations_since(session, start_message_count)
        .into_iter()
        .filter(|operation| operation.invocation.name == TOOLS_CALL_HANDLER_KEY)
        .collect::<Vec<_>>();
    !calls
        .iter()
        .any(|call| Value::from(call.invocation.input.clone()) == expected)
        && calls.iter().all(operation_failed_or_incomplete)
}

/// Same retry rule for the five Tool API functions.
pub(super) fn can_retry_missing_native_invocation(
    session: &Session,
    start_message_count: usize,
    canonical_function: &str,
    input: &Value,
) -> bool {
    if session.blocked() {
        return false;
    }
    let calls = operations_since(session, start_message_count)
        .into_iter()
        .filter(|operation| operation.invocation.name == canonical_function)
        .collect::<Vec<_>>();
    !calls
        .iter()
        .any(|call| Value::from(call.invocation.input.clone()) == *input)
        && calls.iter().all(operation_failed_or_incomplete)
}

pub(super) fn operation_failed_or_incomplete(operation: &OperationPart) -> bool {
    operation.status() != ExecutionStatus::Completed || operation.error_message().is_some()
}

pub(super) fn extract_tool_api_outcome(
    session: &Session,
    start_message_count: usize,
    tool_name: &str,
    input: &Value,
    marker: &str,
    expect_success: bool,
) -> anyhow::Result<ToolApiOutcome> {
    ensure!(!session.blocked(), "session remained blocked");
    let operations = operations_since(session, start_message_count);
    let helped = operations
        .iter()
        .filter(|operation| operation.invocation.name == TOOLS_HELP_HANDLER_KEY)
        .collect::<Vec<_>>();
    ensure!(
        !helped.is_empty(),
        "model did not call tools.help; operations: {}",
        operation_trace_with_ids(session, start_message_count)
    );
    ensure!(
        helped.iter().all(|help| {
            serde_json::Value::from(help.invocation.input.clone()) == json!({"tool": tool_name})
        }),
        "tools.help input did not identify execution tool {tool_name}; operations: {}",
        operation_trace_with_ids(session, start_message_count)
    );
    // Some Cline responses repeat the same read-only discovery call after a
    // rejected malformed execution-tool invocation. That is provider behavior, not a
    // second execution: require every discovery call to be exact and require exactly
    // one exact execution-tool invocation below.
    ensure!(
        helped.len() <= 16,
        "excessive tools.help retries for {tool_name} ({}); operations: {}",
        helped.len(),
        operation_trace_with_ids(session, start_message_count)
    );
    let calls = operations
        .iter()
        .filter(|operation| operation.invocation.name == TOOLS_CALL_HANDLER_KEY)
        .collect::<Vec<_>>();
    ensure!(
        !calls.is_empty(),
        "model did not call tools.call; operations: {}",
        operation_trace_with_ids(session, start_message_count)
    );
    ensure!(
        calls.iter().all(|call| {
            Value::from(call.invocation.input.clone())
                .get("tool")
                .and_then(Value::as_str)
                == Some(tool_name)
        }),
        "model invoked an execution tool other than {tool_name}; operations: {}",
        operation_trace_with_ids(session, start_message_count)
    );
    let expected_call_input = json!({"tool": tool_name, "input": input});
    let matching_calls = calls
        .into_iter()
        .filter(|call| Value::from(call.invocation.input.clone()) == expected_call_input)
        .collect::<Vec<_>>();
    ensure!(
        matching_calls.len() == 1,
        "expected exactly one tools.call with the requested execution tool/input, found {}; operations: {}",
        matching_calls.len(),
        operation_trace_with_ids(session, start_message_count)
    );
    let call = matching_calls[0].clone();
    ensure!(
        serde_json::Value::from(call.invocation.input.clone()) == expected_call_input,
        "tools.call input was not the supplied exact execution tool/input"
    );
    if expect_success {
        ensure!(
            call.status() == ExecutionStatus::Completed && call.error_message().is_none(),
            "tools.call failed: {}",
            call.error_message().unwrap_or("unknown failure")
        );
    } else {
        ensure!(
            call.status() == ExecutionStatus::Failed || call.error_message().is_some(),
            "tools.call unexpectedly succeeded while failure was expected"
        );
    }
    let terminal_text = transcript_since(session, start_message_count);
    if expect_success {
        ensure!(
            terminal_text.contains(marker),
            "model did not emit terminal marker {marker}: {terminal_text}"
        );
    }
    Ok(ToolApiOutcome { call })
}

pub(super) fn extract_native_outcome(
    session: &Session,
    start_message_count: usize,
    canonical_function: &str,
    input: &Value,
    marker: &str,
    tool_marker: Option<&str>,
) -> anyhow::Result<ToolApiOutcome> {
    ensure!(!session.blocked(), "session remained blocked");
    let matching = operations_since(session, start_message_count)
        .into_iter()
        .filter(|operation| operation.invocation.name == canonical_function)
        .collect::<Vec<_>>();
    ensure!(
        matching.len() == 1,
        "expected exactly one {canonical_function}, found {}; operations: {}",
        matching.len(),
        operation_trace_with_ids(session, start_message_count)
    );
    let call = matching[0].clone();
    ensure!(
        serde_json::Value::from(call.invocation.input.clone()) == *input,
        "{canonical_function} input differed from the supplied exact input"
    );
    ensure!(
        call.status() == ExecutionStatus::Completed && call.error_message().is_none(),
        "{canonical_function} failed: {}",
        call.error_message().unwrap_or("unknown failure")
    );
    let terminal_text = transcript_since(session, start_message_count);
    ensure!(
        terminal_text.contains(marker),
        "model did not emit terminal marker {marker}: {terminal_text}"
    );
    let outcome = ToolApiOutcome { call };
    if let Some(expected) = tool_marker {
        ensure!(
            outcome.visible_text().contains(expected),
            "{canonical_function} result did not contain {expected}: {}",
            outcome.visible_text()
        );
    }
    Ok(outcome)
}

pub(super) fn operation_trace_with_ids(session: &Session, start_message_count: usize) -> String {
    session
        .messages
        .iter()
        .skip(start_message_count)
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::Operation(operation)) => {
                Some((part.operation_id.as_deref(), operation))
            }
            _ => None,
        })
        .map(|(operation_id, operation)| {
            let input = Value::from(operation.invocation.input.clone());
            format!(
                "{}({input}) operation_id={} status={:?} error={}",
                operation.invocation.name,
                operation_id.unwrap_or("<none>"),
                operation.status(),
                operation.error_message().unwrap_or("<none>")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) fn assert_contains(outcome: &ToolApiOutcome, expected: &str) -> anyhow::Result<()> {
    ensure!(
        outcome.visible_text().contains(expected),
        "tool result did not contain {expected}: {}",
        outcome.visible_text()
    );
    Ok(())
}

/// The final operation result is insufficient evidence that a plugin's
/// streaming handler ran: the ordinary non-streaming handler may return the
/// same text. Require a live `MessagePartCheckpointed` snapshot where the outer
/// Tool API operation is still in progress and contains a streamed chunk.
pub(super) async fn assert_outer_tool_api_stream_update(
    subscription: &mut Subscription<EventKind>,
    tool_name: &str,
    input: &Value,
    expected_text: &str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut observed_operations = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!(
                "did not receive an in-progress streamed update for outer {TOOLS_CALL_HANDLER_KEY} running {tool_name}; observed operation updates: {}",
                if observed_operations.is_empty() {
                    "<none>".to_string()
                } else {
                    observed_operations.join("; ")
                }
            );
        }
        let item = match tokio::time::timeout(remaining, subscription.recv()).await {
            Ok(item) => {
                item.context("session event bus closed while awaiting streamed Tool API update")?
            }
            Err(_) => continue,
        };
        match item {
            SubscriptionItem::Lagged(count) => {
                bail!(
                    "session event subscription lagged by {count} event(s) while checking streamed Tool API output"
                );
            }
            SubscriptionItem::Event(event) => {
                let EventKind::MessagePartCheckpointed(update) = &event.kind else {
                    continue;
                };
                let Some(PartContent::Operation(operation)) = update.part.content.as_ref() else {
                    continue;
                };
                if observed_operations.len() < 12 {
                    observed_operations.push(format!(
                        "name={} input={} status={:?} output={}",
                        operation.invocation.name,
                        Value::from(operation.invocation.input.clone()),
                        update.part.status,
                        operation.output_text().unwrap_or_default()
                    ));
                }
                if update.part.status != ExecutionStatus::InProgress {
                    continue;
                }
                if operation.invocation.name != TOOLS_CALL_HANDLER_KEY
                    || Value::from(operation.invocation.input.clone())
                        != json!({"tool": tool_name, "input": input})
                    || !operation
                        .output_text()
                        .is_some_and(|text| text.contains(expected_text))
                {
                    continue;
                }
                return Ok(());
            }
        }
    }
}

pub(super) fn payload_string(value: &Value, field: &str) -> anyhow::Result<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .with_context(|| format!("payload is missing string field `{field}`: {value}"))
}
