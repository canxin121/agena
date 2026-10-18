use super::*;
use agena_domain::{
    CommandOutputStream, ComposerDocument, ComposerNode, ContentEpoch, ContentId, ModelRef, Role,
};
use agena_provider::{
    CompletionInputPart, CompletionInputProviderState, CompletionInputToolResultStatus,
    ProviderCompactionContext,
};
use agena_storage::kernel::*;
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use serde_json::json;

fn model() -> ModelRef {
    ModelRef::new_with_adapter("fixture", "responses", "fixture-model")
}
fn document(text: &str) -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text { text: text.into() }])
}

async fn fixture(
    memory: bool,
) -> (
    SqliteKernel,
    tempfile::TempDir,
    SessionId,
    AttemptHandle,
    EntryId,
) {
    let root = tempfile::tempdir().unwrap();
    let url = if memory {
        "sqlite::memory:".into()
    } else {
        format!(
            "sqlite://{}?mode=rwc",
            root.path().join("runtime.db").display()
        )
    };
    let kernel = open_kernel_runtime(&url).await.unwrap();
    let workspace = kernel
        .workspaces
        .create(root.path().to_string_lossy().into())
        .await
        .unwrap();
    let session = kernel
        .ports
        .sessions
        .create(RequestId::new(), workspace.id, "kernel runtime".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = kernel
        .ports
        .sessions
        .send(RequestId::new(), session, document("  exact user 世界\n\0"))
        .await
        .unwrap()
        .value;
    let attempt = kernel
        .ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    (kernel, root, session, attempt, sent.message_id)
}

async fn committed(reader: &KernelReader, session: SessionId, content: ContentId, head: u64) {
    let mut watch = reader.watch_content(session, content).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while watch.head().unwrap().committed < head {
            watch.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn kernel_prompt_restores_signed_reasoning_once_per_round_without_changing_stored_opaque_facts()
 {
    for memory in [true, false] {
        let (kernel, _root, session, attempt, user) = fixture(memory).await;
        let ports = &kernel.ports;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let reasoning = round
            .begin_text(RequestId::new(), true)
            .await
            .unwrap()
            .value;
        let body = "  signed 世界🙂\n\0 reasoning\t";
        reasoning.append(body).await.unwrap();
        let head = reasoning.finish(RequestId::new()).await.unwrap().value;
        let raw = json!({"assistant_reasoning_field":"reasoning_content","response_id":"resp_fixture","anthropic_thinking_blocks":[{"type":"thinking","thinking":body,"signature":"AA+/==\n\0","future_field":{"array":[3,null,true]}}],"openai_reasoning_items":[{"type":"reasoning","id":"rs_1","summary":[{"type":"summary_text","text":"世界🙂"}],"encrypted_content":"  OPAQUE==\n\0  "}],"future_provider_value":{"empty":"","missing":null}});
        let referenced = reference_replay_text(&raw, &[(head, body.into())]).unwrap();
        assert!(
            !serde_json::to_string(&referenced)
                .unwrap()
                .contains("signed 世界")
        );
        let stored = serde_json::to_value(KernelProviderState::new(model(), referenced)).unwrap();
        let output = round
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output.append("first round answer\n").await.unwrap();
        round
            .finish(RequestId::new(), Some(stored.clone()), true)
            .await
            .unwrap();
        let next = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let facts = next.prompt().await.unwrap();
        let hydrator = KernelPromptHydrator::isolated();
        let prompt = hydrator
            .hydrate(&ports.reader, facts.clone(), &model(), false)
            .await
            .unwrap();
        assert_eq!(prompt.turns().len(), 2);
        assert_eq!(prompt.turns()[0].role, Role::User);
        assert_eq!(
            prompt.turns()[0].parts,
            [CompletionInputPart::Text {
                text: "  exact user 世界\n\0".into()
            }]
        );
        assert_eq!(
            prompt.turns()[1].parts,
            [
                CompletionInputPart::Reasoning { text: body.into() },
                CompletionInputPart::Text {
                    text: "first round answer\n".into()
                }
            ]
        );
        let expected: CompletionInputProviderState = serde_json::from_value(raw).unwrap();
        assert_eq!(prompt.turns()[1].provider_state, expected);
        assert_eq!(prompt.facts().entries[0].entry.id, user);
        assert_eq!(
            prompt.facts().replays[0].provider_state,
            Some(stored.clone())
        );
        assert_eq!(
            ports
                .reader
                .round(session, round.id())
                .await
                .unwrap()
                .provider_state,
            Some(stored)
        );
        let switched = hydrator
            .hydrate(
                &ports.reader,
                facts,
                &ModelRef::new("another-provider", "another-model"),
                true,
            )
            .await
            .unwrap();
        assert!(
            switched
                .turns()
                .iter()
                .all(|turn| turn.provider_state.is_empty())
        );
        assert_eq!(switched.turns()[1].parts, prompt.turns()[1].parts);
        drop((prompt, switched));
        assert_eq!(hydrator.budget().bytes, 0);
        kernel.ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn kernel_prompt_uses_frozen_tool_outcomes_and_log_heads_and_keeps_reused_call_ids_separate()
{
    for memory in [true, false] {
        let (kernel, _root, session, attempt, _user) = fixture(memory).await;
        let ports = &kernel.ports;
        let first = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let tool = first
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "reused-id".into(),
                    name: "tools_call".into(),
                    input: json!({"which":1}),
                },
            )
            .await
            .unwrap()
            .value;
        let log = tool.begin_log(RequestId::new()).await.unwrap().value;
        log.append_at(123, CommandOutputStream::Stdout, "\u{1b}[31mold\r世界\n")
            .await
            .unwrap();
        committed(&ports.reader, session, log.id(), 1).await;
        first.finish(RequestId::new(), None, true).await.unwrap();
        let second = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let frozen = second.prompt().await.unwrap();
        log.append_at(124, CommandOutputStream::Stderr, "late must be excluded")
            .await
            .unwrap();
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({"late":true}),
            },
        )
        .await
        .unwrap();
        let later = second
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: "reused-id".into(),
                    name: "tools_call".into(),
                    input: json!({"which":2}),
                },
            )
            .await
            .unwrap()
            .value;
        later
            .complete(
                RequestId::new(),
                ToolResult {
                    phase: ToolPhase::Failed,
                    value: json!({"second":"own result"}),
                },
            )
            .await
            .unwrap();
        second.finish(RequestId::new(), None, true).await.unwrap();
        let hydrator = KernelPromptHydrator::isolated();
        let prompt = hydrator
            .hydrate(&ports.reader, frozen, &model(), false)
            .await
            .unwrap();
        let CompletionInputPart::ToolResult {
            output_json,
            status,
            ..
        } = &prompt.turns()[1].parts[1]
        else {
            panic!("paired result")
        };
        let output: serde_json::Value = serde_json::from_str(output_json).unwrap();
        assert_eq!(*status, CompletionInputToolResultStatus::Failed);
        assert!(output["result"].is_null());
        assert_eq!(
            output["sources"][0]["body"]["records"],
            json!([{"sequence":1,"timestamp_ms":123,"channel":"stdout","text":"\u{1b}[31mold\r世界\n"}])
        );
        assert!(!output_json.contains("late"));
        let third = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let all = hydrator
            .hydrate(
                &ports.reader,
                third.prompt().await.unwrap(),
                &model(),
                false,
            )
            .await
            .unwrap();
        assert_eq!(all.turns().len(), 3);
        let results = all
            .turns()
            .iter()
            .flat_map(|turn| &turn.parts)
            .filter_map(|part| match part {
                CompletionInputPart::ToolResult { output_json, .. } => {
                    Some(serde_json::from_str::<serde_json::Value>(output_json).unwrap())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(results[0]["result"]["late"], true);
        assert_eq!(results[1]["result"]["second"], "own result");
        kernel.ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn native_checkpoint_is_route_bound_and_has_a_canonical_summary_fallback() {
    for memory in [true, false] {
        let (kernel, _root, _session, attempt, user) = fixture(memory).await;
        let ports = &kernel.ports;
        let compact = attempt
            .begin_compaction(RequestId::new(), 2)
            .await
            .unwrap()
            .value;
        let output = compact
            .begin_text(RequestId::new(), false)
            .await
            .unwrap()
            .value;
        output.append("  portable summary 世界\n").await.unwrap();
        let context = ProviderCompactionContext::OpenAiResponses {
            items: vec![
                json!({"type":"compaction","encrypted_content":"  AB+/==\n\0","unknown":[null,1]}),
            ],
        };
        let mut state = KernelProviderState::new(model(), json!({}));
        state.compaction = Some(context.clone());
        compact
            .commit_compaction(
                RequestId::new(),
                output.id(),
                user,
                Some(serde_json::to_value(state).unwrap()),
            )
            .await
            .unwrap();
        let normal = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let facts = normal.prompt().await.unwrap();
        let hydrator = KernelPromptHydrator::isolated();
        let native = hydrator
            .hydrate(&ports.reader, facts.clone(), &model(), true)
            .await
            .unwrap();
        assert_eq!(native.compaction(), Some(&context));
        assert!(native.turns().is_empty());
        for (route, enabled) in [(model(), false), (ModelRef::new("other", "model"), true)] {
            let fallback = hydrator
                .hydrate(&ports.reader, facts.clone(), &route, enabled)
                .await
                .unwrap();
            assert!(fallback.compaction().is_none());
            assert!(
                matches!(&fallback.turns()[0].parts[0],CompletionInputPart::Text {text} if text.contains("  portable summary 世界\n"))
            );
        }
        kernel.ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn mixed_model_preview_preserves_partial_utf8_output_its_resume_cursor_and_the_latest_document()
 {
    let (kernel, _root, session, attempt, _) = fixture(true).await;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "mixed-preview".into(),
                name: "tools_call".into(),
                input: json!({"tool":"fixture.mixed","input":{}}),
            },
        )
        .await
        .unwrap()
        .value;
    let mixed = tool.begin_mixed(RequestId::new()).await.unwrap().value;
    let first = "界🙂".repeat(28_000);
    let second = "後🙂".repeat(28_000);
    mixed
        .append(agena_domain::ContentInput::Text {
            text: first.clone(),
        })
        .await
        .unwrap();
    mixed
        .append(agena_domain::ContentInput::Log {
            stream: CommandOutputStream::Stderr,
            text: second.clone(),
        })
        .await
        .unwrap();
    mixed
        .append(agena_domain::ContentInput::Structured {
            event: agena_domain::DocumentMutation::Insert {
                after: None,
                block: agena_domain::ViewBlock::Text {
                    id: Some("summary".into()),
                    text: "latest retained checkpoint".into(),
                },
            },
        })
        .await
        .unwrap();
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"done":true}),
        },
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    let next = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let prompt = KernelPromptHydrator::default()
        .hydrate(
            &kernel.ports.reader,
            next.prompt().await.unwrap(),
            &model(),
            false,
        )
        .await
        .unwrap();
    let output = prompt
        .turns()
        .iter()
        .flat_map(|turn| &turn.parts)
        .find_map(|part| match part {
            CompletionInputPart::ToolResult { output_json, .. } => {
                Some(serde_json::from_str::<serde_json::Value>(output_json).unwrap())
            }
            _ => None,
        })
        .unwrap();
    let body = &output["sources"][0]["body"];
    assert_eq!(body["preview_limited"], true);
    assert_eq!(body["events"][0]["input"]["text"], first);
    let partial = body["events"][1]["input"]["text"].as_str().unwrap();
    assert!(!partial.is_empty());
    assert!(second.starts_with(partial));
    assert!(partial.len() < second.len());
    assert_eq!(body["partial_event"]["offset_bytes"], partial.len());
    assert_eq!(body["next_cursor"]["sequence"], 1);
    assert_eq!(
        body["document"]["blocks"][0]["text"],
        "latest retained checkpoint"
    );
    let cursor: EventCursor = serde_json::from_value(body["next_cursor"].clone()).unwrap();
    let page = kernel
        .ports
        .reader
        .event_page(session, mixed.id(), Some(cursor), 1)
        .await
        .unwrap();
    assert_eq!(page.records[0].sequence, 2);
    match &page.records[0].payload {
        EventPayload::Mixed { input, .. } => {
            assert_eq!(input.text_content(), Some(second.as_str()))
        }
        _ => panic!("exact mixed event required"),
    };
    kernel.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn hydration_reports_retention_loss_and_releases_admission_on_failure_or_drop() {
    let (kernel, _root, session, attempt, _user) = fixture(false).await;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let text = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    text.append("exact output which cannot be silently shortened")
        .await
        .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    let next = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let facts = next.prompt().await.unwrap();
    let hydrator = KernelPromptHydrator::isolated();
    let mut held = Vec::new();
    for _ in 0..5 {
        held.push(
            hydrator
                .hydrate(&kernel.ports.reader, facts.clone(), &model(), false)
                .await
                .unwrap(),
        );
    }
    assert!(matches!(
        hydrator
            .hydrate(&kernel.ports.reader, facts.clone(), &model(), false)
            .await,
        Err(KernelError::Busy)
    ));
    held.clear();
    assert_eq!(hydrator.budget().bytes, 0);
    kernel
        .maintenance
        .prune_content(RequestId::new(), session, text.id(), 1)
        .await
        .unwrap();
    assert!(matches!(
        hydrator
            .hydrate(&kernel.ports.reader, facts, &model(), false)
            .await,
        Err(KernelError::ContentGap { .. })
    ));
    assert_eq!(hydrator.budget().bytes, 0);
    kernel.ports.shutdown().await.unwrap();
}

#[test]
fn invalid_reasoning_references_fail_instead_of_falling_back_or_expanding_without_a_budget() {
    let head = ContentStreamView {
        activity_id: None,
        id: ContentId::new(),
        entry: EntryId::try_from(1).unwrap(),
        epoch: ContentEpoch::new(),
        kind: agena_domain::ContentKind::Text,
        ordinal: 0,
        capture_loss: Default::default(),
        accepted: 3,
        committed: 3,
        retained_start: 0,
        state: CaptureState::Complete,
        capture_error: None,
    };
    let sources = std::collections::BTreeMap::from([(head.id, (head.clone(), "界".into()))]);
    let valid = json!({"text":{"agena.kernel_text":{"content":head.id,"epoch":head.epoch,"start":0,"length":3}}});
    assert_eq!(
        super::replay::restore_replay_text(&valid, &sources, &mut 1000).unwrap(),
        json!({"text":"界"})
    );
    for (field, value) in [
        ("content", json!(ContentId::new())),
        ("epoch", json!(ContentEpoch::new())),
        ("start", json!(1)),
        ("length", json!(4)),
        ("length", json!(u64::MAX)),
    ] {
        let mut invalid = valid.clone();
        invalid["text"]["agena.kernel_text"][field] = value;
        assert!(super::replay::restore_replay_text(&invalid, &sources, &mut 1000).is_err());
    }
    let repeated = serde_json::Value::Array(vec![valid; 100]);
    assert!(super::replay::restore_replay_text(&repeated, &sources, &mut 1000).is_err());
}

#[tokio::test]
async fn provider_hosted_only_round_replays_opaque_state_without_local_calls_or_results() {
    let (kernel, _root, _session, attempt, _) = fixture(true).await;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_provider_tool(
            RequestId::new(),
            ToolStart {
                call_id: "hosted-only".into(),
                name: "chatgpt.search".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"opaque":"retained"}),
        },
    )
    .await
    .unwrap();
    let replay = json!({"response_id":"hosted-response","openai_reasoning_items":[{"type":"reasoning","encrypted_content":"  opaque+/=\n\0"}]});
    round
        .finish(
            RequestId::new(),
            Some(serde_json::to_value(KernelProviderState::new(model(), replay.clone())).unwrap()),
            true,
        )
        .await
        .unwrap();
    let next = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let hydrator = KernelPromptHydrator::isolated();
    let prompt = hydrator
        .hydrate(
            &kernel.ports.reader,
            next.prompt().await.unwrap(),
            &model(),
            false,
        )
        .await
        .unwrap();
    assert_eq!(prompt.turns().len(), 2);
    assert!(prompt.turns()[1].parts.is_empty());
    assert_eq!(
        prompt.turns()[1].provider_state.response_id.as_deref(),
        Some("hosted-response")
    );
    assert_eq!(
        prompt.turns()[1].provider_state.openai_reasoning_items,
        vec![replay["openai_reasoning_items"][0].clone()]
    );
    let switched = hydrator
        .hydrate(
            &kernel.ports.reader,
            next.prompt().await.unwrap(),
            &ModelRef::new("other", "model"),
            false,
        )
        .await
        .unwrap();
    assert_eq!(switched.turns().len(), 1);
    assert_eq!(switched.turns()[0].role, Role::User);
    kernel.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn even_empty_text_and_log_sources_validate_the_frozen_epoch() {
    let (kernel, _root, session, attempt, _) = fixture(true).await;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "empty".into(),
                name: "fixture".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let _text = tool.begin_text(RequestId::new()).await.unwrap().value;
    let _log = tool.begin_log(RequestId::new()).await.unwrap().value;
    round.finish(RequestId::new(), None, true).await.unwrap();
    let next = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let facts = next.prompt().await.unwrap();
    assert_eq!(facts.sources.len(), 2);
    for index in 0..2 {
        // Stand in for a manifest retained across an epoch reset. No bytes or
        // records exist, so skipping a zero-length read would miss this reset.
        let mut stale = (*facts).clone();
        assert_eq!(stale.sources[index].committed, 0);
        stale.sources[index].epoch = ContentEpoch::new();
        let stale = ReadBudget::default().wrap(session, stale).unwrap();
        assert!(matches!(
            KernelPromptHydrator::isolated()
                .hydrate(&kernel.ports.reader, stale, &model(), false)
                .await,
            Err(KernelError::ContentReset)
        ));
    }
    kernel.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn document_and_dense_styled_terminal_project_at_frozen_heads_with_original_source_references()
 {
    use agena_domain::{
        DocumentMutation, TerminalAttributes, TerminalCellRun, TerminalColor, TerminalSnapshot,
        ViewBlock,
    };
    let (kernel, _root, session, attempt, _) = fixture(true).await;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "views".into(),
                name: "fixture".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let document = tool.begin_document(RequestId::new()).await.unwrap().value;
    document
        .apply(DocumentMutation::Insert {
            after: None,
            block: ViewBlock::Text {
                id: Some("body".into()),
                text: "  exact 世界\n\0".into(),
            },
        })
        .await
        .unwrap();
    let doc_head = document.finish(RequestId::new()).await.unwrap().value;
    let terminal = tool.begin_terminal(RequestId::new()).await.unwrap().value;
    let screen = TerminalSnapshot {
        rows: 200,
        cols: 200,
        cursor_row: 199,
        cursor_col: 0,
        cursor_visible: true,
        alternate_screen: false,
        bracketed_paste: true,
        application_cursor: false,
        cells: (0..200)
            .flat_map(|row| {
                (0..200).map(move |col| TerminalCellRun {
                    row,
                    col,
                    text: "x".into(),
                    attributes: TerminalAttributes {
                        foreground: Some(TerminalColor::Rgb {
                            red: 1,
                            green: 2,
                            blue: 3,
                        }),
                        background: Some(TerminalColor::Rgb {
                            red: 4,
                            green: 5,
                            blue: 6,
                        }),
                        bold: true,
                        dim: true,
                        italic: true,
                        underline: true,
                        inverse: true,
                    },
                })
            })
            .collect(),
    };
    terminal.snapshot(screen.clone()).await.unwrap();
    let terminal_head = terminal.finish(RequestId::new()).await.unwrap().value;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"done":true}),
        },
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    let next = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let prompt = KernelPromptHydrator::isolated()
        .hydrate(
            &kernel.ports.reader,
            next.prompt().await.unwrap(),
            &model(),
            false,
        )
        .await
        .unwrap();
    let result = prompt
        .turns()
        .iter()
        .flat_map(|run| &run.parts)
        .find_map(|part| {
            if let CompletionInputPart::ToolResult { output_json, .. } = part {
                Some(output_json)
            } else {
                None
            }
        })
        .unwrap();
    let result: serde_json::Value = serde_json::from_str(result).unwrap();
    let sources = result["sources"].as_array().unwrap();
    let document = sources
        .iter()
        .find(|source| source["content_id"] == json!(doc_head.id))
        .unwrap();
    assert_eq!(document["committed_head"], doc_head.committed);
    assert_eq!(
        document["body"]["document"]["blocks"][0]["text"],
        "  exact 世界\n\0"
    );
    let terminal = sources
        .iter()
        .find(|source| source["content_id"] == json!(terminal_head.id))
        .unwrap();
    assert_eq!(terminal["body"]["terminal"]["text"], screen.plain_text());
    assert_eq!(terminal["body"]["terminal"]["rows"], 200);
    assert!(terminal["body"]["terminal"].get("cells").is_none());
    assert_eq!(terminal["body"]["terminal"]["preview_limited"], false);
    assert_eq!(
        kernel
            .ports
            .reader
            .event_snapshot_at(session, &terminal_head)
            .await
            .unwrap()
            .terminal
            .as_deref(),
        Some(&screen)
    );
    kernel.ports.shutdown().await.unwrap();
}
