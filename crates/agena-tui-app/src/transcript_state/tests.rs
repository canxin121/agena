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

fn image_output(path: &str, hash: &str, localized: bool) -> serde_json::Value {
    use agena_tui_transcript::parts::LOCALIZED_ATTACHMENT_SOURCE_FIELD;
    let source = json!({"source":"local_path","path":path});
    let mut output = json!({"payload":{"text":"full output"},"truncated":false,"attachments":[{
        "kind":"image","mime":"image/png","filename":"same-name.png",
        "sha256":hash,"size_bytes":123,"source":source,
    }]});
    if localized {
        output["attachments"][0][LOCALIZED_ATTACHMENT_SOURCE_FIELD] = source;
        output["attachments"][0]["source"] =
            json!({"source":"data_url","url":"data:image/png;base64,AAAA"});
    }
    output
}

fn image_part(revision: i64, hash: &str, localized: bool) -> PartResource {
    let mut result = part(874, 874, "assistant", "tool_call", "");
    result.revision = revision;
    result.updated_at_ms = revision;
    result.content = json!({"output":image_output("/server/image.png", hash, localized)});
    result
}

fn image_detail(revision: i64, hash: &str) -> agena_api::live::ToolDetailResource {
    agena_api::live::ToolDetailResource {
        part_id: 874,
        revision,
        updated_at_ms: revision,
        section: ToolDetailSection::Output,
        part_state: Some("completed".into()),
        value: image_output("/server/image.png", hash, false),
    }
}

#[test]
fn output_refresh_keeps_only_the_exact_proven_image_identity() {
    for changed in [
        "none",
        "path",
        "hash",
        "size",
        "missing_hash",
        "missing_size",
        "mime",
    ] {
        let mut current = image_part(1, "known-hash", true);
        let mut incoming = image_output("/server/image.png", "known-hash", false);
        let attachment = &mut incoming["attachments"][0];
        match changed {
            "path" => attachment["source"]["path"] = json!("/server/new-reference.png"),
            "hash" => attachment["sha256"] = json!("new-hash"),
            "size" => attachment["size_bytes"] = json!(124),
            "missing_hash" => {
                attachment.as_object_mut().unwrap().remove("sha256");
            }
            "missing_size" => {
                attachment.as_object_mut().unwrap().remove("size_bytes");
            }
            "mime" => attachment["mime"] = json!("image/jpeg"),
            _ => {}
        }
        assert!(apply_tool_detail_value(
            &mut current,
            ToolDetailSection::Output,
            incoming
        ));
        assert_eq!(
            current.content["output"]["attachments"][0]["source"]["source"],
            if changed == "none" {
                "data_url"
            } else {
                "local_path"
            },
            "{changed}"
        );
    }
    let previous = image_part(1, "known-hash", true);
    let mut other_part = image_part(2, "known-hash", false);
    other_part.part_id += 1;
    merge_part_snapshot(&previous, &mut other_part);
    assert_eq!(
        other_part.content["output"]["attachments"][0]["source"]["source"],
        "local_path"
    );
}

#[test]
fn newer_attachment_snapshots_and_details_cannot_be_replaced_by_older_loaded_output() {
    let mut previous = image_part(1, "old-hash", true);
    previous.sections.push(LoadedPartSection {
        section: ToolDetailSection::Output,
        revision: 1,
    });
    let mut incoming = image_part(2, "new-hash", false);
    incoming.content["output"]["payload"] = serde_json::Value::Null;
    merge_part_snapshot(&previous, &mut incoming);
    assert_eq!(
        incoming.content["output"]["attachments"][0]["sha256"],
        "new-hash"
    );
    assert_eq!(
        incoming.content["output"]["attachments"][0]["source"]["source"],
        "local_path"
    );
    assert!(incoming.content["output"]["payload"].is_null());
    assert!(!tool_detail_section_loaded(
        &incoming,
        ToolDetailSection::Output
    ));

    let mut live = TranscriptState::default();
    live.merge_parts(vec![image_part(1, "same-image", true)]);
    let mut update = image_part(2, "same-image", false);
    update.content["output"]["payload"] = serde_json::Value::Null;
    assert!(live.merge_part(update));
    assert_eq!(
        live.parts[0].content["output"]["attachments"][0]["source"]["source"],
        "data_url"
    );
    assert!(live.parts[0].content["output"]["payload"].is_null());

    let mut state = TranscriptState::default();
    state.merge_parts(vec![image_part(2, "new-hash", true)]);
    let before = state.parts[0].content.clone();
    assert!(!state.finish_tool_detail_load(image_detail(1, "old-hash")));
    assert_eq!(state.parts[0].content, before);
    // An independent metadata revision does not invalidate an otherwise
    // identical output ETag; the previously verified preview can stay visible.
    assert!(state.finish_tool_detail_load(image_detail(1, "new-hash")));
    assert_eq!(state.parts[0].content, before);
    assert!(state.finish_tool_detail_load(image_detail(2, "new-hash")));
    assert!(!state.finish_tool_detail_load(image_detail(1, "new-hash")));
}

#[test]
fn switching_sessions_drops_preview_and_pending_detail_ownership() {
    let mut state = TranscriptState::default();
    state.reset(46, "first".into());
    state.merge_parts(vec![image_part(1, "same-hash", true)]);
    let pending = state
        .begin_tool_detail_load(874, ToolDetailSection::Output)
        .unwrap();
    state.reset(47, "unrelated".into());
    state.merge_parts(vec![image_part(1, "same-hash", false)]);
    assert!(!state.accept_tool_detail_load(874, ToolDetailSection::Output, pending));
    assert!(state.finish_tool_detail_load(image_detail(1, "same-hash")));
    assert_eq!(
        state.parts[0].content["output"]["attachments"][0]["source"]["source"],
        "local_path"
    );
}

fn image_presentation(hash: &str) -> agena_domain::PartDocument {
    serde_json::from_value(json!({"title":"Image","summary":"","blocks":[{
        "type":"media","artifact":{
            "uri":"file:///server/image.png","mime":"image/png",
            "sha256":hash,"size_bytes":123,"name":"same-name.png"
        }
    }]}))
    .unwrap()
}

#[test]
fn older_loaded_presentation_cannot_restore_media_from_a_previous_image_revision() {
    let mut previous = image_part(3, "image-a", false);
    previous.presentation = Some(image_presentation("image-a"));
    previous.sections.push(LoadedPartSection {
        section: ToolDetailSection::Presentation,
        revision: 3,
    });
    for presentation in [None, Some(image_presentation("image-b"))] {
        let mut incoming = image_part(4, "image-b", false);
        incoming.presentation = presentation.clone();
        merge_part_snapshot(&previous, &mut incoming);
        assert_eq!(incoming.presentation, presentation);
        assert!(
            !incoming
                .sections
                .iter()
                .any(|section| section.section == ToolDetailSection::Presentation)
        );
    }
    // Only the envelope changed: identical current raw image identity proves
    // that the independently cached presentation is still valid.
    let mut same_image = image_part(4, "image-a", false);
    merge_part_snapshot(&previous, &mut same_image);
    assert_eq!(same_image.presentation, previous.presentation);
    assert_eq!(same_image.sections[0].revision, 3);
}

#[test]
fn delayed_presentation_details_cannot_replace_the_current_image_but_same_image_etags_work() {
    let mut current = image_part(4, "image-b", false);
    current.presentation = Some(image_presentation("image-b"));
    let mut state = TranscriptState::default();
    state.merge_parts(vec![current]);
    let detail = |hash| agena_api::live::ToolDetailResource {
        part_id: 874,
        revision: 3,
        updated_at_ms: 3,
        section: ToolDetailSection::Presentation,
        part_state: None,
        value: serde_json::to_value(image_presentation(hash)).unwrap(),
    };
    assert!(!state.finish_tool_detail_load(detail("image-a")));
    assert_eq!(
        state.parts[0].presentation,
        Some(image_presentation("image-b"))
    );
    assert!(state.finish_tool_detail_load(detail("image-b")));
    assert_eq!(
        state.parts[0].presentation,
        Some(image_presentation("image-b"))
    );
    assert_eq!(state.parts[0].sections[0].revision, 3);
}
