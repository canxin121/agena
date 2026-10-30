use agena_api::part::PartResource;
use agena_domain::{PartDocument, ViewBlock};
use agena_failure::{
    Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility, FieldIssue,
    FieldIssueKind, ModelFeedback, RecoveryDirective, RetryDirective, UserPresentation,
    UserProblem,
};
use agena_runtime_contracts::part_content::ToolCallContent;
use agena_tui::i18n::I18n;
use agena_tui_transcript::{
    RenderedMessageBlock, TranscriptActivityContent, TranscriptActivitySection, TranscriptEntry,
    TranscriptNodeKey, TranscriptPartContent, parts_entries, render_entry_detailed,
};
use serde_json::json;

const MODEL_ONLY: &str = "model_only_canary";

fn failure(message: &'static str) -> Failure {
    let failure = Failure::new(
        FailureCode::new("tool.input_schema_invalid"),
        FailureCategory::InvalidInput,
        FailureResponsibility::Caller,
        RetryDirective::CorrectInput,
        RecoveryDirective::Retry,
        FailureImpact::OperationFailed,
        UserPresentation::new("tool-input-invalid", message),
    )
    .with_model_feedback(ModelFeedback::invalid_input_with_fields([FieldIssue::new(
        MODEL_ONLY,
        FieldIssueKind::Required,
    )]));
    // The sentinel must survive the real typed failure, so this test catches
    // accidentally serializing Failure instead of its public UserProblem.
    assert!(
        serde_json::to_string(&failure)
            .unwrap()
            .contains(MODEL_ONLY)
    );
    failure
}

fn projected(name: &str, failure: &Failure, duplicate_body: bool) -> TranscriptEntry<'static> {
    let part = PartResource {
        part_id: 48,
        kind: "tool_call".into(),
        role: "assistant".into(),
        state: "failed".into(),
        run_id: Some(48),
        run_state: Some("completed".into()),
        message_boundary: true,
        // Match the presentation-only API page: no Input, Output or Metadata
        // section has been requested yet. Failure is an independent fact.
        content: json!({
            "name": name,
            "call_id": 564,
            "state": "failed",
            "lifecycle": {"start_ms": 0},
            "error": {"failure": failure}
        }),
        presentation: Some(PartDocument {
            title: name.into(),
            summary: String::new(),
            blocks: if duplicate_body {
                vec![ViewBlock::Text {
                    id: Some("failure".into()),
                    text: failure.user.fallback.clone(),
                }]
            } else {
                Vec::new()
            },
        }),
        ..Default::default()
    };
    decode_and_project(part)
}

fn decode_and_project(part: PartResource) -> TranscriptEntry<'static> {
    // The single part in this fixture is also its own durable boundary.
    // A mismatched run_id intentionally excludes content in parts_entries.
    assert!(part.message_boundary);
    assert_eq!(part.run_id, Some(part.part_id));
    let mut structural = part.content.clone();
    structural
        .as_object_mut()
        .unwrap()
        .entry("input".to_owned())
        .or_insert_with(|| json!({}));
    serde_json::from_value::<ToolCallContent>(structural)
        .unwrap_or_else(|error| panic!("fixture ToolCallContent failed to decode: {error}"));
    let entry = parts_entries(&[part]).remove(0);
    assert_eq!(entry.parts.len(), 1);
    assert!(
        matches!(
            &entry.parts[0].content,
            TranscriptPartContent::Activity(TranscriptActivityContent::Operation(_))
        ),
        "fixture did not project to an Operation despite successful canonical decoding"
    );
    entry
}

fn section_key(
    entry: &TranscriptEntry<'_>,
    section: TranscriptActivitySection,
) -> TranscriptNodeKey {
    TranscriptNodeKey::ActivitySection {
        entry_id: entry.id,
        content_id: entry.parts[0].id,
        section,
    }
}

fn render(entry: &TranscriptEntry<'_>, details: bool, width: u16) -> RenderedMessageBlock {
    let mut expansions = [(
        TranscriptNodeKey::Activity {
            entry_id: entry.id,
            content_id: entry.parts[0].id,
        },
        true,
    )]
    .into_iter()
    .collect::<std::collections::BTreeMap<_, _>>();
    for section in [
        TranscriptActivitySection::TechnicalDetails,
        TranscriptActivitySection::Input,
        TranscriptActivitySection::Output,
        TranscriptActivitySection::Metadata,
        TranscriptActivitySection::RenderData,
    ] {
        expansions.insert(section_key(entry, section), details);
    }
    render_entry_detailed(
        entry,
        width,
        &I18n::english(),
        &Default::default(),
        &expansions,
    )
}

#[test]
fn failed_tool_families_show_public_details_without_loading_any_payload() {
    for (name, message) in [
        ("shell.exec", "Unexpected fields: cwd, timeout_seconds."),
        ("fs.read", "The required file_path field is missing."),
        ("fs.read_media", "The requested file does not exist."),
        (
            "image_understanding",
            "The required inputs field is missing.",
        ),
        (
            "chatgpt.image_generate",
            "No active model route is configured.",
        ),
    ] {
        let failure = failure(message);
        let entry = projected(name, &failure, false);
        let TranscriptPartContent::Activity(TranscriptActivityContent::Operation(tool)) =
            &entry.parts[0].content
        else {
            panic!("failed tool did not decode: {name}");
        };
        assert!(!tool.input_loaded && !tool.output_loaded && !tool.metadata_loaded);
        assert!(tool.operation.output.is_none());

        let folded = render(&entry, false, 100);
        assert!(
            folded.lines.iter().any(|line| line.text.contains(message)),
            "{name}"
        );
        assert!(
            !folded
                .lines
                .iter()
                .any(|line| line.text.contains("correct_input"))
        );

        let expanded = render(&entry, true, 100);
        let details = expanded
            .nodes
            .iter()
            .find(|node| {
                node.key == section_key(&entry, TranscriptActivitySection::TechnicalDetails)
            })
            .unwrap();
        assert!(details.expanded && details.toggleable);
        let (_, public_json) = details.copy_text.split_once('\n').unwrap();
        let actual: serde_json::Value = serde_json::from_str(public_json).unwrap();
        assert_eq!(
            actual,
            serde_json::to_value(UserProblem::from(&failure)).unwrap()
        );
        assert!(
            expanded.lines[details.start_line..details.end_line]
                .iter()
                .any(|line| line.text.contains("tool.input_schema_invalid"))
        );
        assert!(
            expanded.lines[details.start_line..details.end_line]
                .iter()
                .any(|line| line.text.contains("correct_input"))
        );
        for node in &expanded.nodes {
            assert!(!node.copy_text.contains(MODEL_ONLY));
            assert!(!node.copy_text.contains("\"model\""));
        }
        assert!(
            expanded
                .lines
                .iter()
                .all(|line| !line.text.contains(MODEL_ONLY))
        );

        // Collapsing details hides the technical body again, while retaining
        // the actionable reason in the human output and its copy projection.
        let refolded = render(&entry, false, 100);
        let activity = refolded
            .nodes
            .iter()
            .find(|node| matches!(node.key, TranscriptNodeKey::Activity { .. }))
            .unwrap();
        assert!(activity.copy_text.contains(message));
        assert!(!activity.copy_text.contains("tool.input_schema_invalid"));
    }
}

#[test]
fn failure_body_is_not_duplicated_by_the_human_presentation() {
    let failure = failure("The required file_path field is missing.");
    let entry = projected("fs.read", &failure, true);
    let rendered = render(&entry, false, 100);
    let presentation = rendered
        .nodes
        .iter()
        .find(|node| node.key == section_key(&entry, TranscriptActivitySection::Presentation));
    assert!(
        presentation.is_none_or(|node| !node.copy_text.contains(failure.user.fallback.as_str()))
    );
    let activity = rendered
        .nodes
        .iter()
        .find(|node| matches!(node.key, TranscriptNodeKey::Activity { .. }))
        .unwrap();
    assert_eq!(
        activity
            .copy_text
            .matches(failure.user.fallback.as_str())
            .count(),
        1
    );
}

#[test]
fn public_failure_details_wrap_without_changing_copied_data() {
    use unicode_width::UnicodeWidthStr;
    let failure = failure("The required file_path field is missing.");
    let entry = projected("fs.read", &failure, false);
    for width in [24, 48, 100] {
        let rendered = render(&entry, true, width);
        let details = rendered
            .nodes
            .iter()
            .find(|node| {
                node.key == section_key(&entry, TranscriptActivitySection::TechnicalDetails)
            })
            .unwrap();
        assert!(
            rendered.lines[details.start_line..details.end_line]
                .iter()
                .all(|line| { UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width) })
        );
        let (_, public_json) = details.copy_text.split_once('\n').unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(public_json).unwrap(),
            serde_json::to_value(UserProblem::from(&failure)).unwrap(),
        );
    }
}

fn missing_reason_entry(state: &str, failure: Option<&Failure>) -> TranscriptEntry<'static> {
    let mut content = json!({
        "name": "audit.tool", "call_id": 1, "state": state,
        "lifecycle": {"start_ms": 0}
    });
    if let Some(failure) = failure {
        content["error"] = json!({"failure": failure});
    }
    decode_and_project(PartResource {
        part_id: 2,
        kind: "tool_call".into(),
        role: "assistant".into(),
        state: state.into(),
        run_id: Some(2),
        run_state: Some("completed".into()),
        message_boundary: true,
        content,
        ..Default::default()
    })
}

#[test]
fn missing_failure_reason_is_explicit_without_inventing_failure_facts() {
    let mut blank = failure("The public reason is absent in this fixture.");
    blank.user.fallback = "  ".into();
    let unavailable = I18n::english().text("tool-error-unavailable");
    for failure in [None, Some(&blank)] {
        let entry = missing_reason_entry("failed", failure);
        for expanded in [false, true] {
            let rendered = render(&entry, expanded, 100);
            assert!(
                rendered
                    .lines
                    .iter()
                    .any(|line| line.text.contains(unavailable.as_str()))
            );
            let activity = rendered
                .nodes
                .iter()
                .find(|node| matches!(node.key, TranscriptNodeKey::Activity { .. }))
                .unwrap();
            assert!(activity.copy_text.contains(unavailable.as_str()));
            if expanded {
                let details = rendered
                    .nodes
                    .iter()
                    .find(|node| {
                        node.key == section_key(&entry, TranscriptActivitySection::TechnicalDetails)
                    })
                    .unwrap();
                assert!(details.copy_text.contains(unavailable.as_str()));
                if failure.is_none() {
                    assert!(!details.copy_text.contains("\"code\""));
                    assert!(!details.copy_text.contains("\"id\""));
                } else {
                    assert!(details.copy_text.contains("tool.input_schema_invalid"));
                    assert!(!details.copy_text.contains(MODEL_ONLY));
                }
            }
        }
    }
}

#[test]
fn nonfailed_tools_do_not_invent_a_missing_error() {
    let unavailable = I18n::english().text("tool-error-unavailable");
    for state in [
        "pending",
        "running",
        "completed",
        "cancelled",
        "user_declined",
        "policy_denied",
    ] {
        let rendered = render(&missing_reason_entry(state, None), true, 100);
        assert!(
            rendered
                .lines
                .iter()
                .all(|line| !line.text.contains(unavailable.as_str())),
            "{state}"
        );
        assert!(
            rendered
                .nodes
                .iter()
                .all(|node| !node.copy_text.contains(unavailable.as_str())),
            "{state}"
        );
    }
}
