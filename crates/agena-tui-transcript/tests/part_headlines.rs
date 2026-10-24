use agena_api::part::{PartResource, ReasoningPartResource};
use agena_api::resource::{RunRole, RunStatus};
use agena_domain::{ActivityPayload, PartDocument, TextSegmentActivity};
use agena_tui::i18n::I18n;
use agena_tui_transcript::{
    PartExecutionStatusResource, TranscriptActivityContent, TranscriptContentId, TranscriptEntry,
    TranscriptEntryId, TranscriptEntryPart, TranscriptNodeKey, TranscriptPartContent,
    parts_entries, render_entry_detailed,
};
use serde_json::{Value, json};
use unicode_width::UnicodeWidthStr;

fn entry(content: TranscriptPartContent<'_>) -> TranscriptEntry<'_> {
    TranscriptEntry {
        id: TranscriptEntryId::StoredMessage(1),
        reply_id: None,
        role: Some(RunRole::Assistant),
        state: RunStatus::Completed,
        created_at: chrono::Utc::now(),
        parts: vec![TranscriptEntryPart {
            id: TranscriptContentId::StoredPart(2),
            status: PartExecutionStatusResource::Completed,
            headline: Default::default(),
            content,
        }],
    }
}

fn wire(kind: &str, content: Value) -> PartResource {
    PartResource {
        part_id: 2,
        kind: kind.into(),
        role: "assistant".into(),
        state: "completed".into(),
        run_id: Some(1),
        content,
        ..Default::default()
    }
}

fn projected(mut part: PartResource) -> TranscriptEntry<'static> {
    part.message_boundary = true;
    part.run_id = Some(part.part_id);
    part.run_state = Some("completed".into());
    parts_entries(&[part]).remove(0)
}

fn render(
    entry: &TranscriptEntry,
    expanded: bool,
    width: u16,
    i18n: &I18n,
) -> agena_tui_transcript::RenderedMessageBlock {
    let expansions = [(
        TranscriptNodeKey::Activity {
            entry_id: entry.id,
            content_id: entry.parts[0].id,
        },
        expanded,
    )]
    .into();
    render_entry_detailed(entry, width, i18n, &Default::default(), &expansions)
}

#[test]
fn every_wire_activity_retains_a_title_and_content_summary_when_open_or_closed() {
    for (kind, content, title, summary) in [
        (
            "text",
            json!({"text": "\n\n# The answer\n\nFull answer body"}),
            "Answer",
            "The answer",
        ),
        (
            "think",
            json!({"summary": ["\n\n**Inspect streaming**\nThen inspect layout"], "raw": ["raw fallback"]}),
            "Thinking",
            "Inspect streaming",
        ),
        (
            "file_ref",
            json!({"title": " ", "name": "report.txt", "path": "/tmp/report.txt"}),
            "Attachment",
            "report.txt",
        ),
        (
            "file_ref",
            json!({"name": "# report_`$i`.txt", "path": "/tmp/report.txt"}),
            "Attachment",
            "# report_`$i`.txt",
        ),
        (
            "skill_ref",
            json!({"command": "review", "description": "Review changes"}),
            "Command",
            "review",
        ),
        (
            "paste_ref",
            json!({"label": " ", "text": "Pasted snippet\nFull snippet"}),
            "Pasted text",
            "Pasted snippet",
        ),
        (
            "notice",
            json!({"title": " ", "summary": "", "detail": "Notice details\nMore details"}),
            "Notice",
            "Notice details",
        ),
        (
            "hook",
            json!({"hook": "agent.stop", "summary": " ", "message": "Continue reviewing\nContinuation body"}),
            "Hook",
            "Continue reviewing",
        ),
        (
            "compaction",
            json!({"summary": "Summary of the session\nRetained facts"}),
            "Compaction",
            "Summary of the session",
        ),
        (
            "system_notification",
            json!({"operation_kind": "shell", "summary": "", "body": "Process finished\nExit code: 0"}),
            "System notification",
            "Process finished",
        ),
    ] {
        let entry = projected(wire(kind, content));
        assert_eq!(entry.parts.len(), 1, "{kind}");
        for expanded in [false, true] {
            let rendered = render(&entry, expanded, 100, &I18n::english());
            let node = rendered
                .nodes
                .iter()
                .find(|node| matches!(node.key, TranscriptNodeKey::Activity { .. }))
                .unwrap();
            let headline = &rendered.lines[node.start_line];
            assert!(headline.text.contains(title), "{kind}: {}", headline.text);
            assert!(headline.text.contains(summary), "{kind}: {}", headline.text);
            assert_eq!(
                headline.copy_text,
                headline.text.chars().skip(4).collect::<String>()
            );
            if !expanded {
                assert_eq!(rendered.lines.len(), 2, "{kind}");
            }
        }
    }
}

#[test]
fn reasoning_metadata_survives_streaming_without_changing_the_full_body() {
    let source = "\n\n**Inspect streaming**\n    keep literal indent\nMore reasoning";
    let mut part = wire("think", json!({"summary": [source]}));
    part.presentation = Some(PartDocument {
        title: "Reasoning: terminal output".into(),
        summary: "Check duplicate output and checkpoints".into(),
        blocks: vec![],
    });
    part.summary = Some("Older summary".into());
    let mut previous = None;
    for suffix in [
        "",
        "\nNew streaming detail",
        "\nNew streaming detail\nCompleted",
    ] {
        part.content = json!({"summary": [format!("{source}{suffix}")]});
        let entry = projected(part.clone());
        for expanded in [false, true] {
            let rendered = render(&entry, expanded, 120, &I18n::english());
            let headline = rendered.lines[1].text.chars().skip(4).collect::<String>();
            assert_eq!(
                headline,
                "Reasoning: terminal output · Check duplicate output and checkpoints"
            );
            if let Some(previous) = &previous {
                assert_eq!(&headline, previous);
            }
            previous = Some(headline);
            if expanded {
                let body = rendered.lines[2..]
                    .iter()
                    .map(|line| line.copy_text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert_eq!(
                    body.trim_matches('\n'),
                    format!("{source}{suffix}").trim_matches('\n')
                );
                assert!(!body.contains("Older summary"));
            }
        }
    }
    part.presentation.as_mut().unwrap().summary = " ".into();
    let rendered = render(&projected(part), false, 100, &I18n::english());
    assert!(rendered.lines[1].text.ends_with("Older summary"));
}

#[test]
fn blank_summary_falls_back_to_raw_and_private_reasoning_never_displays_ciphertext() {
    let reasoning = ReasoningPartResource {
        summary: vec![" \n".into()],
        raw_content: vec!["\nRaw reasoning\nFull raw body".into()],
        encrypted_content: None,
    };
    assert!(reasoning.preferred_text().contains("Full raw body"));
    let entry = entry(TranscriptPartContent::Activity(
        TranscriptActivityContent::Reasoning(reasoning),
    ));
    let rendered = render(&entry, true, 80, &I18n::english());
    assert!(rendered.lines[1].text.contains("Thinking · Raw reasoning"));
    assert!(
        rendered
            .lines
            .iter()
            .any(|line| line.copy_text == "Full raw body")
    );
    for (locale, expected) in [
        ("en-US", "No visible reasoning provided"),
        ("zh-CN", "提供方未返回可见思考内容"),
    ] {
        let entry = projected(wire(
            "think",
            json!({"encrypted_content": "PRIVATE_OPAQUE_BYTES"}),
        ));
        let i18n = I18n::resolve(Some(locale), None);
        let rendered = render(&entry, true, 100, &i18n);
        assert!(rendered.lines[1].text.contains(expected));
        assert!(
            !rendered
                .lines
                .iter()
                .any(|line| line.text.contains("PRIVATE_OPAQUE_BYTES"))
        );
        assert_eq!(
            rendered.lines.len(),
            2,
            "empty reasoning must not add blank body rows"
        );
    }
}

#[test]
fn empty_streaming_parts_describe_their_state_until_content_arrives() {
    for (state, summary) in [
        ("pending", "Waiting for content"),
        ("in_progress", "Receiving content"),
        ("completed", "No text content"),
        ("cancelled", "cancelled"),
    ] {
        let mut part = wire("think", json!({}));
        part.state = state.into();
        let entry = projected(part);
        for expanded in [false, true] {
            let rendered = render(&entry, expanded, 80, &I18n::english());
            assert!(
                rendered.lines[1].text.contains(summary),
                "{state}: {}",
                rendered.lines[1].text
            );
            assert_eq!(rendered.lines.len(), 2);
        }
    }
}

#[test]
fn canonical_reasoning_and_blank_notice_titles_use_the_same_headline_rules() {
    for (payload, title, summary, full_body) in [
        (
            json!({"activity_type": "reasoning", "content": {"summary": ["\nReasoning topic\nSecond reasoning line"]}}),
            "Thinking",
            "Reasoning topic",
            "Second reasoning line",
        ),
        (
            json!({"activity_type": "notice", "kind": "provider_retry", "title": " ", "summary": "", "detail": "Retry after disconnect\nFull retry details"}),
            "Provider retry",
            "Retry after disconnect",
            "Full retry details",
        ),
        (
            json!({"activity_type": "text_artifact", "label": " ", "text": "Snippet topic\nFull snippet body"}),
            "Pasted text",
            "Snippet topic",
            "Full snippet body",
        ),
        (
            json!({"activity_type": "text_artifact", "text": "\n---\n###\n```rust\nlet useful = true;\n```"}),
            "Pasted text",
            "let useful = true;",
            "let useful = true;",
        ),
        (
            json!({"activity_type": "command_reference", "name": " ", "description": " ", "content_hash": "abc", "source": "plugin"}),
            "Command",
            "plugin",
            "plugin",
        ),
        (
            json!({"activity_type": "resource", "kind": "file", "name": " ", "reference": {"reference_type": "workspace_path", "path": "src/main.rs"}}),
            "Attachment",
            "src/main.rs",
            "src/main.rs",
        ),
        (
            json!({"activity_type": "command_reference", "name": "review", "description": "Review implementation", "content_hash": "abc", "source": "plugin"}),
            "Command: review",
            "Review implementation",
            "plugin",
        ),
        (
            json!({"activity_type": "text_segment", "text": "Working note\nRemaining note"}),
            "Text",
            "Working note",
            "Remaining note",
        ),
        (
            json!({"activity_type": "interaction", "interaction_type": "user_input", "request": {
                "request_id": "question", "kind": "ask_user", "source": "plugin", "created_at": "2026-10-25T00:00:00Z",
                "questions": [{"header": "Display", "question": "Choose a display mode", "options": [], "multiple": false, "allow_custom": true}]
            }}),
            "User input requested",
            "Choose a display mode",
            "Choose a display mode",
        ),
    ] {
        let payload: ActivityPayload = serde_json::from_value(payload).unwrap();
        let entry = entry(TranscriptPartContent::Activity(
            TranscriptActivityContent::Canonical(&payload),
        ));
        let rendered = render(&entry, true, 100, &I18n::english());
        assert!(rendered.lines[1].text.contains(title));
        assert!(rendered.lines[1].text.contains(summary));
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.text.contains(full_body))
        );
    }
}

#[test]
fn compaction_failure_keeps_the_reason_and_checkpoint_tokens_remain_a_summary() {
    let entry = projected(wire(
        "compaction",
        json!({"outcome": "failed", "detail": "Provider disconnected\nTechnical cause"}),
    ));
    let rendered = render(&entry, true, 100, &I18n::english());
    assert!(rendered.lines[1].text.contains("Context compaction failed"));
    assert!(
        rendered
            .lines
            .iter()
            .any(|line| line.text.contains("Provider disconnected"))
    );
    let entry = projected(wire(
        "compaction",
        json!({"checkpoint": {"before_tokens": 1000, "after_tokens": 250}}),
    ));
    let rendered = render(&entry, false, 100, &I18n::english());
    assert!(
        rendered.lines[1]
            .text
            .contains("1000 → 250 tokens · 75% reduction")
    );
}

#[test]
fn narrow_markdown_summaries_fit_and_copy_only_visible_cells() {
    let entry = entry(TranscriptPartContent::Activity(TranscriptActivityContent::Answer(Box::new(TextSegmentActivity {
        text: "\n# **中文主题** and [link](https://example.com/long/path)\n\nBody remains available".into(),
    }))));
    for width in [1, 4, 12, 24, 32, 48, 80] {
        for expanded in [false, true] {
            let rendered = render(&entry, expanded, width, &I18n::english());
            let node = rendered
                .nodes
                .iter()
                .find(|node| matches!(node.key, TranscriptNodeKey::Activity { .. }))
                .unwrap();
            let headline = &rendered.lines[node.start_line];
            assert!(UnicodeWidthStr::width(headline.text.as_str()) <= usize::from(width));
            if width >= 4 {
                assert_eq!(
                    headline.copy_text,
                    headline.text.chars().skip(4).collect::<String>()
                );
            }
            assert!(!headline.text.contains("……"), "{}", headline.text);
            if width >= 24 {
                assert!(headline.text.contains("中文"), "{}", headline.text);
            }
        }
    }
}

#[test]
fn tool_headlines_keep_result_summaries_with_long_literal_titles() {
    let content = agena_runtime_contracts::part_content::ToolCallContent {
        name: "agena.shell.exec".into(),
        input: json!({"command": "printf '%s' \"$PATH\""}),
        call_id: 12,
        state: agena_domain::ToolResultState::Completed,
        ..Default::default()
    };
    let mut part = wire("tool_call", content.as_value());
    let title = r#"Read a_very_long_file · printf '%s' "$PATH" and $(seq 1 10)"#;
    part.presentation = Some(PartDocument {
        title: title.into(),
        summary: "Updated three files".into(),
        blocks: vec![],
    });
    let entry = projected(part);
    for width in [36, 48, 80, 120] {
        for expanded in [false, true] {
            let rendered = render(&entry, expanded, width, &I18n::english());
            let headline = &rendered.lines[1];
            assert!(headline.text.contains("Updated"), "{}", headline.text);
            assert!(
                headline.text.contains("a_"),
                "titles must stay literal: {}",
                headline.text
            );
            assert!(UnicodeWidthStr::width(headline.text.as_str()) <= usize::from(width));
            assert_eq!(
                headline.copy_text,
                headline.text.chars().skip(4).collect::<String>()
            );
            if width == 120 {
                assert!(headline.text.contains(title));
            }
        }
    }
}

#[test]
fn errors_keep_the_public_reason_when_their_details_are_expanded() {
    use agena_failure::*;
    let problem: UserProblem = Failure::new(
        FailureCode::new("test.connection"),
        FailureCategory::DependencyUnavailable,
        FailureResponsibility::Dependency,
        RetryDirective::Backoff,
        RecoveryDirective::Retry,
        FailureImpact::OperationFailed,
        UserPresentation::new("test-reason", "Provider connection closed"),
    )
    .into();
    let entry = projected(wire("error", json!({"problem": problem})));
    for expanded in [false, true] {
        let rendered = render(&entry, expanded, 100, &I18n::english());
        assert!(
            rendered.lines[1]
                .text
                .contains("Error · Provider connection closed")
        );
    }
}

#[test]
fn notification_severity_and_summary_survive_disclosure() {
    let content = agena_runtime_contracts::part_content::ToolCallContent {
        name: "interaction.notify".into(),
        input: json!({}),
        call_id: 14,
        state: agena_domain::ToolResultState::Completed,
        metadata: [("agena.notification.level".into(), json!("warning"))].into(),
        ..Default::default()
    };
    let mut part = wire("tool_call", content.as_value());
    part.presentation = Some(PartDocument {
        title: "Review warning".into(),
        summary: "Two checks need attention".into(),
        blocks: vec![],
    });
    let entry = projected(part);
    for expanded in [false, true] {
        let rendered = render(&entry, expanded, 100, &I18n::english());
        assert!(
            rendered.lines[1]
                .text
                .contains("▲ Review warning · Two checks need attention")
        );
        assert_eq!(
            rendered.lines[1].copy_text,
            "Review warning · Two checks need attention"
        );
    }
}

#[test]
fn lifecycle_and_fold_parts_keep_their_meaningful_state_labels() {
    use agena_tui_transcript::TranscriptAssistantReplyLifecycle as State;
    for (content, expected) in [
        (
            TranscriptActivityContent::AssistantReplyLifecycle(State::Running),
            "Generating response",
        ),
        (
            TranscriptActivityContent::AssistantReplyLifecycle(State::Completed),
            "Response complete",
        ),
        (
            TranscriptActivityContent::AssistantReplyLifecycle(State::Cancelled),
            "Response cancelled",
        ),
        (
            TranscriptActivityContent::Fold { hidden_count: 3 },
            "3 older parts hidden",
        ),
    ] {
        let entry = entry(TranscriptPartContent::Activity(content));
        let rendered = render(&entry, false, 100, &I18n::english());
        assert!(
            rendered.lines[1].text.contains(expected),
            "{}",
            rendered.lines[1].text
        );
        if !expected.contains("older parts") {
            assert!(!rendered.lines[1].text.contains(" · "));
        }
    }
}
