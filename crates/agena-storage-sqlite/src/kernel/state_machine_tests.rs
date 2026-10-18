//! Seeded command combinations checked against an independent message-group
//! model. Expected membership comes only from submitted inputs and public
//! creation receipts/handles, never from a history read or backend helper.

use std::collections::{BTreeMap, BTreeSet};

use agena_domain::{PermissionReply, PermissionReplyKind};

use super::{tests::fixture, *};

struct Trace {
    seed: u64,
    memory: bool,
    operations: Vec<String>,
}

impl Trace {
    fn record(&mut self, operation: String) {
        self.operations.push(operation);
    }
}

impl Drop for Trace {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "state-machine seed={:#x} memory={} complete operation sequence:\n{}",
                self.seed,
                self.memory,
                self.operations.join("\n")
            );
        }
    }
}

#[derive(Clone)]
struct ExpectedEntry {
    // Permission receipts expose InteractionId, not its transcript EntryId.
    // That entry is checked by its full payload identity instead of guessing
    // an internal integer relationship or reading history into the model.
    id: Option<EntryId>,
    generation: i64,
    payload: EntryPayload,
    text: Option<String>,
}

#[derive(Clone)]
struct MessageGroup {
    origin: SessionId,
    message: EntryId,
    execution: ExecutionId,
    attempts: Vec<ExecutionPhase>,
    entries: Vec<ExpectedEntry>,
    decisions: Vec<(InteractionId, InteractionDecision)>,
}

struct Branch {
    session: SessionId,
    groups: Vec<MessageGroup>,
}

type AttemptFacts = BTreeMap<(String, i64), (i64, String)>;

fn draw(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn phase_text(phase: ExecutionPhase) -> String {
    match phase {
        ExecutionPhase::Succeeded => "succeeded",
        ExecutionPhase::Failed => "failed",
        ExecutionPhase::Cancelled => "cancelled",
        _ => panic!("the reference model stores settled turns only"),
    }
    .into()
}

async fn settle_generation(
    ports: &KernelPorts,
    group: &mut MessageGroup,
    step: usize,
    permission: Option<bool>,
    cancel: bool,
    trace: &mut Trace,
) -> (AttemptHandle, TextOutput) {
    let generation = i64::try_from(group.attempts.len() + 1).unwrap();
    trace.record(format!(
        "step={step} claim {:?} generation={generation}; begin round",
        group.execution
    ));
    let attempt = ports
        .executor
        .claim(RequestId::new(), group.origin, group.execution)
        .await
        .unwrap()
        .value;
    assert_eq!(attempt.generation().get(), generation);
    let round = attempt
        .begin_round(RequestId::new(), vec![group.message])
        .await
        .unwrap()
        .value;

    if let Some(allowed) = permission {
        trace.record(format!(
            "step={step} begin tool; request permission; reply allowed={allowed}"
        ));
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: format!("seed-{}-{step}-{generation}", trace.seed),
                    name: "state-model.permission".into(),
                    input: serde_json::json!({"step":step,"generation":generation}),
                },
            )
            .await
            .unwrap()
            .value;
        // The public canonical tool lookup supplies only the created entry ID;
        // its body, order, generation and eventual outcome stay model-owned.
        let tool_entry = ports
            .reader
            .tool(group.origin, tool.id())
            .await
            .unwrap()
            .entry_id;
        group.entries.push(ExpectedEntry {
            id: Some(tool_entry),
            generation,
            payload: EntryPayload::Tool { tool_id: tool.id() },
            text: None,
        });
        let interaction = tool
            .request_interaction(
                RequestId::new(),
                InteractionRequest::Permission {
                    action_key: format!("model.action.{step}"),
                    description: "精确权限描述 🙂\n".into(),
                },
            )
            .await
            .unwrap()
            .value;
        group.entries.push(ExpectedEntry {
            id: None,
            generation,
            payload: EntryPayload::Interaction {
                interaction_id: interaction,
            },
            text: None,
        });
        let decision = InteractionDecision::Permission {
            reply: PermissionReply {
                request_id: interaction.get().to_string(),
                kind: if allowed {
                    PermissionReplyKind::AllowOnce
                } else {
                    PermissionReplyKind::DenyOnce
                },
                reason: Some(format!("seed={} step={step} 精确理由🙂\n", trace.seed)),
                scope: None,
            },
            allowed,
        };
        let request = RequestId::new();
        let receipt = ports
            .sessions
            .reply(request, group.origin, interaction, decision.clone(), None)
            .await
            .unwrap();
        assert_eq!(
            receipt,
            ports
                .sessions
                .reply(request, group.origin, interaction, decision.clone(), None)
                .await
                .unwrap()
        );
        assert_eq!(receipt.value, decision);
        assert_eq!(
            ports
                .reader
                .interaction(group.origin, interaction)
                .await
                .unwrap()
                .decision
                .as_ref(),
            Some(&decision)
        );
        group.decisions.push((interaction, decision));
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: if allowed {
                    ToolPhase::Succeeded
                } else {
                    ToolPhase::Failed
                },
                value: serde_json::json!({"allowed":allowed,"step":step}),
            },
        )
        .await
        .unwrap();
    }

    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let pieces = [
        format!("seed={} step={step} generation={generation}: ", trace.seed),
        "  界🙂\t\r\n\0".into(),
        " exact tail\n".into(),
    ];
    for piece in &pieces {
        trace.record(format!("step={step} append {:?} {piece:?}", output.id()));
        output.append(piece).await.unwrap();
    }
    group.entries.push(ExpectedEntry {
        id: Some(output.entry_id()),
        generation,
        payload: EntryPayload::Text {
            content_id: output.id(),
        },
        text: Some(pieces.concat()),
    });

    let phase = if cancel {
        trace.record(format!("step={step} cancel {:?}", group.execution));
        let request = RequestId::new();
        let cancelled = ports
            .sessions
            .cancel(request, group.origin, group.execution)
            .await
            .unwrap();
        assert_eq!(cancelled.value.phase, ExecutionPhase::Cancelled);
        assert_eq!(
            cancelled,
            ports
                .sessions
                .cancel(request, group.origin, group.execution)
                .await
                .unwrap()
        );
        ExecutionPhase::Cancelled
    } else {
        let phase = if permission == Some(false) {
            ExecutionPhase::Failed
        } else {
            ExecutionPhase::Succeeded
        };
        trace.record(format!(
            "step={step} finish round; finish attempt {phase:?}"
        ));
        round
            .finish(RequestId::new(), None, phase == ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let finished = attempt.finish(RequestId::new(), phase).await.unwrap();
        assert_eq!(finished.value.phase, phase);
        phase
    };
    group.attempts.push(phase);
    trace.record(format!(
        "step={step} reject terminal append and outcome overwrite"
    ));
    assert!(matches!(
        output.append("must never be captured after terminal").await,
        Err(KernelError::Conflict(_))
    ));
    let opposite = if phase == ExecutionPhase::Succeeded {
        ExecutionPhase::Failed
    } else {
        ExecutionPhase::Succeeded
    };
    assert!(matches!(
        attempt.finish(RequestId::new(), opposite).await,
        Err(KernelError::Conflict(_))
    ));
    (attempt, output)
}

async fn check_history(ports: &KernelPorts, branch: &Branch, check_bytes: bool) {
    let mut after = None;
    let mut membership = None;
    let mut actual = Vec::new();
    loop {
        let page = ports
            .reader
            .history_after(branch.session, after, 31)
            .await
            .unwrap();
        assert!(page.entries.len() <= 31);
        if let Some(membership) = membership {
            assert_eq!(page.metadata.membership_generation, membership);
        } else {
            membership = Some(page.metadata.membership_generation);
        }
        if page.has_more {
            assert!(!page.entries.is_empty());
        }
        after = page.entries.last().map(|entry| entry.id);
        actual.extend(page.entries.iter().cloned());
        if !page.has_more {
            break;
        }
    }
    let expected_count: usize = branch.groups.iter().map(|group| group.entries.len()).sum();
    assert_eq!(actual.len(), expected_count);
    let mut actual = actual.into_iter();
    let mut unique = BTreeSet::new();
    for group in &branch.groups {
        for expected in &group.entries {
            let entry = actual.next().unwrap();
            assert!(unique.insert(entry.id));
            if let Some(id) = expected.id {
                assert_eq!(entry.id, id);
            }
            assert_eq!(entry.origin_session_id, group.origin);
            assert_eq!(entry.message_id, group.message);
            assert_eq!(entry.execution_id, group.execution);
            assert_eq!(entry.generation.get(), expected.generation);
            assert_eq!(entry.payload, expected.payload);
            let phase = if matches!(expected.payload, EntryPayload::User { .. }) {
                *group.attempts.last().unwrap()
            } else {
                group.attempts[usize::try_from(expected.generation - 1).unwrap()]
            };
            assert_eq!(entry.execution_phase, phase);
            let response = group
                .entries
                .iter()
                .find(|candidate| {
                    candidate.generation == expected.generation
                        && !matches!(candidate.payload, EntryPayload::User { .. })
                })
                .and_then(|candidate| candidate.id);
            assert_eq!(entry.response_entry_id, response);
            if check_bytes && let Some(expected_text) = &expected.text {
                let EntryPayload::Text { content_id } = expected.payload else {
                    panic!("model text has no text identity")
                };
                let mut cursor = None;
                let mut bytes = Vec::new();
                loop {
                    let page = ports
                        .reader
                        .text_range(branch.session, content_id, cursor, 7)
                        .await
                        .unwrap();
                    assert_eq!(page.stream.accepted, page.stream.committed);
                    assert_ne!(page.stream.state, CaptureState::Open);
                    bytes.extend_from_slice(&page.bytes);
                    cursor = Some(page.next);
                    if !page.has_more {
                        break;
                    }
                }
                assert_eq!(bytes, expected_text.as_bytes());
            }
        }
        if check_bytes {
            for (interaction, decision) in &group.decisions {
                let actual = ports
                    .reader
                    .interaction(branch.session, *interaction)
                    .await
                    .unwrap();
                assert_eq!(actual.decision.as_ref(), Some(decision));
                assert!(actual.decided_at_ms.is_some());
            }
        }
    }
}

async fn check_facts(admin: &sea_orm::DatabaseConnection, expected: &AttemptFacts) {
    let actual: Vec<(String, i64, i64, String)> = sqlx::query_as(
        "SELECT execution_id,generation,session_id,phase FROM agena_attempts ORDER BY execution_id,generation",
    )
    .fetch_all(admin.get_sqlite_connection_pool())
    .await
    .unwrap();
    let expected_rows = expected
        .iter()
        .map(|((execution, generation), (owner, phase))| {
            (execution.clone(), *generation, *owner, phase.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected_rows, "immutable generation/terminal facts");
    let actual: Vec<(String, i64)> =
        sqlx::query_as("SELECT id,current_generation FROM agena_executions ORDER BY id")
            .fetch_all(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
    let mut current = BTreeMap::new();
    for (execution, generation) in expected.keys() {
        current.insert(execution.clone(), *generation);
    }
    assert_eq!(actual, current.into_iter().collect::<Vec<_>>());
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(admin.get_sqlite_connection_pool())
            .await
            .unwrap()
            .is_empty()
    );
    let unfinished: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM agena_rounds WHERE phase='running') + (SELECT count(*) FROM agena_tool_calls WHERE phase IN ('preparing','running')) + (SELECT count(*) FROM agena_interactions WHERE decision_json IS NULL) + (SELECT count(*) FROM agena_session_statistics WHERE queued_executions!=0 OR running_executions!=0)",
    )
    .fetch_one(admin.get_sqlite_connection_pool())
    .await
    .unwrap();
    assert_eq!(
        unfinished, 0,
        "settled turns cannot leave active child facts"
    );
}

#[tokio::test]
async fn seeded_command_state_machine_matches_independent_message_groups_for_file_and_memory() {
    for seed in [0x2026_1018_u64, 0x9e37_79b9_7f4a_7c15] {
        for memory in [true, false] {
            let mut trace = Trace {
                seed,
                memory,
                operations: Vec::new(),
            };
            let (ports, admin, _root, _url) = fixture(memory).await;
            trace.record("create root".into());
            let first = ports
                .sessions
                .create(RequestId::new(), 1, "independent seeded model".into())
                .await
                .unwrap()
                .value
                .id;
            let mut branches = vec![Branch {
                session: first,
                groups: Vec::new(),
            }];
            let mut facts = AttemptFacts::new();
            let mut covered = BTreeSet::new();
            let mut state = seed;
            for step in 0..256 {
                let random = draw(&mut state);
                let index = (random as usize) % branches.len();
                let session = branches[index].session;
                let action = (random >> 32) % 8;
                trace.record(format!(
                    "step={step} draw={random:#x} branch={index} session={session:?} action={action}"
                ));
                match action {
                    4 if !branches[index].groups.is_empty() && branches.len() < 8 => {
                        let cut = (random as usize) % branches[index].groups.len();
                        let groups = branches[index].groups[..=cut].to_vec();
                        let through = groups.last().unwrap().entries.last().unwrap().id.unwrap();
                        trace.record(format!("step={step} fork through {through:?} group={cut}"));
                        let child = ports
                            .sessions
                            .fork(RequestId::new(), session, through)
                            .await
                            .unwrap()
                            .value
                            .id;
                        branches.push(Branch {
                            session: child,
                            groups,
                        });
                        covered.insert("fork");
                    }
                    5 if branches[index]
                        .groups
                        .last()
                        .is_some_and(|group| group.origin == session) =>
                    {
                        let group = branches[index].groups.last().unwrap();
                        trace.record(format!(
                            "step={step} withdraw owned group {:?}",
                            group.message
                        ));
                        ports
                            .sessions
                            .withdraw(RequestId::new(), session, group.message)
                            .await
                            .unwrap();
                        branches[index].groups.pop();
                        covered.insert("withdraw");
                    }
                    6 if branches.len() > 1 => {
                        trace.record(format!("step={step} delete {session:?}"));
                        ports
                            .sessions
                            .delete(RequestId::new(), session)
                            .await
                            .unwrap();
                        branches.remove(index);
                        assert!(matches!(
                            ports.reader.metadata(session).await,
                            Err(KernelError::NotFound(_))
                        ));
                        covered.insert("delete");
                    }
                    _ => {
                        trace.record(format!("step={step} submit user input"));
                        let request = RequestId::new();
                        let input = tests::document(&format!("seed={seed} step={step} 用户🙂\n"));
                        let receipt = ports
                            .sessions
                            .send(request, session, input.clone())
                            .await
                            .unwrap();
                        assert_eq!(
                            receipt,
                            ports
                                .sessions
                                .send(request, session, input.clone())
                                .await
                                .unwrap()
                        );
                        let sent = receipt.value;
                        assert_eq!(sent.generation.get(), 1);
                        assert_eq!(sent.phase, ExecutionPhase::Queued);
                        let mut group = MessageGroup {
                            origin: session,
                            message: sent.message_id,
                            execution: sent.id,
                            attempts: Vec::new(),
                            entries: vec![ExpectedEntry {
                                id: Some(sent.message_id),
                                generation: 1,
                                payload: EntryPayload::User { document: input },
                                text: None,
                            }],
                            decisions: Vec::new(),
                        };
                        let permission = matches!(action, 1 | 7).then_some(random & 0x10000 != 0);
                        let cancel = matches!(action, 2 | 3);
                        let (old_attempt, old_output) = settle_generation(
                            &ports, &mut group, step, permission, cancel, &mut trace,
                        )
                        .await;
                        covered.extend(["submit", "append"]);
                        if permission.is_some() {
                            covered.insert("permission reply");
                        }
                        if cancel {
                            covered.insert("cancel");
                        }
                        if action == 3 {
                            trace.record(format!("step={step} retry {:?}", sent.id));
                            let request = RequestId::new();
                            let retried = ports
                                .sessions
                                .retry(request, session, sent.id)
                                .await
                                .unwrap();
                            assert_eq!(retried.value.generation.get(), 2);
                            assert_eq!(retried.value.phase, ExecutionPhase::Queued);
                            assert_eq!(
                                retried,
                                ports
                                    .sessions
                                    .retry(request, session, sent.id)
                                    .await
                                    .unwrap()
                            );
                            assert!(matches!(
                                old_attempt
                                    .begin_round(RequestId::new(), vec![sent.message_id])
                                    .await,
                                Err(KernelError::StaleAttempt)
                            ));
                            assert_eq!(
                                old_output.append("stale producer must stay fenced").await,
                                Err(KernelError::StaleAttempt)
                            );
                            settle_generation(
                                &ports,
                                &mut group,
                                step,
                                Some(true),
                                false,
                                &mut trace,
                            )
                            .await;
                            covered.extend(["retry", "permission reply"]);
                        }
                        for (generation, phase) in group.attempts.iter().enumerate() {
                            facts.insert(
                                (sent.id.to_string(), i64::try_from(generation + 1).unwrap()),
                                (session.get(), phase_text(*phase)),
                            );
                        }
                        branches[index].groups.push(group);
                    }
                }
                for branch in &branches {
                    check_history(&ports, branch, step % 32 == 0 || step == 255).await;
                }
                check_facts(&admin, &facts).await;
            }
            assert_eq!(
                covered,
                BTreeSet::from([
                    "submit",
                    "append",
                    "permission reply",
                    "cancel",
                    "retry",
                    "withdraw",
                    "fork",
                    "delete",
                ]),
                "every seed/backend must exercise every required operation"
            );
            ports.shutdown().await.unwrap();
        }
    }
}
