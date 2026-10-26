use super::*;
use agena_api::live::ToolDetailSection;
use agena_api::part::{LoadedPartSection, PartResource};
use agena_tui_transcript::{TranscriptActivityContent, TranscriptPartContent, parts_entries};
use serde_json::json;

fn part(id: i64, run: i64, role: &str, kind: &str, text: &str) -> PartResource {
    PartResource {
        part_id: id,
        run_id: Some(run),
        message_id: Some(if role == "user" { id } else { 873 }),
        message_boundary: id == run,
        role: role.into(),
        kind: kind.into(),
        state: "completed".into(),
        run_state: (id == run).then(|| "completed".into()),
        content: if kind == "think" {
            json!({"summary": [text], "raw": []})
        } else {
            json!({"text": text})
        },
        created_at_ms: id,
        updated_at_ms: id,
        revision: 1,
        ..Default::default()
    }
}

#[test]
fn overlapping_history_pages_do_not_repeat_messages_reasoning_or_answers() {
    let history = vec![
        part(873, 873, "user", "text", "hi"),
        part(
            874,
            874,
            "assistant",
            "think",
            "Thinking about the greeting",
        ),
        part(875, 874, "assistant", "text", "Hi! I’m ready to help."),
    ];
    let mut state = TranscriptState::default();
    state.merge_parts(history.clone());
    // An older page can contain entries already supplied by live updates or
    // a snapshot, including the boundary of a partially loaded reply.
    state.prepend_transcript_parts(history.clone(), vec![], 100, 30);
    state.prepend_transcript_parts(history, vec![], 100, 30);

    assert_eq!(state.parts.len(), 3);
    let entries = parts_entries(&state.parts);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].parts.len(), 1);
    assert_eq!(entries[1].parts.len(), 2);
    assert!(matches!(
        entries[1].parts[0].content,
        TranscriptPartContent::Activity(TranscriptActivityContent::Reasoning(_))
    ));
    assert!(matches!(
        entries[1].parts[1].content,
        TranscriptPartContent::Activity(TranscriptActivityContent::Answer(_))
    ));
    assert_eq!(
        state
            .rendered(100)
            .lines
            .iter()
            .filter(|line| line.copy_text == "hi")
            .count(),
        1
    );
}

#[test]
fn overlapping_snapshots_keep_the_newest_revision_and_loaded_tool_sections() {
    let mut current = part(874, 874, "assistant", "tool_call", "");
    current.revision = 3;
    current.content = json!({"input": {"path": "report.txt"}});
    current.sections = vec![LoadedPartSection {
        section: ToolDetailSection::Input,
        revision: 3,
    }];
    let mut shell = current.clone();
    shell.revision = 4;
    shell.content = json!({});
    shell.sections.clear();
    let mut state = TranscriptState::default();
    state.merge_parts(vec![current.clone()]);

    for incoming in [vec![shell.clone(), current.clone()], vec![current, shell]] {
        state.merge_execution_parts(incoming);
        assert_eq!(state.parts.len(), 1);
        assert_eq!(state.parts[0].revision, 4);
        assert_eq!(state.parts[0].content["input"]["path"], "report.txt");
        assert_eq!(state.parts[0].sections[0].revision, 3);
    }
}
