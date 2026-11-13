use super::*;
use agena_api::live::ToolDetailSection;
use agena_api::part::{LoadedPartSection, PartResource};
use agena_tui_transcript::{TranscriptActivityContent, TranscriptPartContent, parts_entries};
use serde_json::json;

#[test]
fn search_edits_reuse_layout_nodes_and_entry_blocks() {
    let mut state = TranscriptState::default();
    state.merge_parts(vec![part(
        901,
        901,
        "user",
        "text",
        "ÉCLAIR and 中\n\nsecond paragraph",
    )]);
    let rendered = state.rendered(80);
    let lines = rendered.lines.as_ptr();
    let nodes = rendered.nodes.as_ptr();
    let blocks = state
        .entry_render_cache
        .iter()
        .map(|(id, entry)| (*id, entry.block.clone()))
        .collect::<Vec<_>>();
    for query in ["é", "中", "paragraph", "missing", "  "] {
        state.set_search_query(query.to_owned());
        let rendered = state.rendered(80);
        assert_eq!(
            rendered.lines.as_ptr(),
            lines,
            "search is not a layout input"
        );
        assert_eq!(rendered.nodes.as_ptr(), nodes);
        let expected = rendered
            .lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                agena_tui_session::session_helpers::contains_case_insensitive(&line.text, query)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        assert_eq!(rendered.search_matches, expected);
        for (id, block) in &blocks {
            assert!(std::sync::Arc::ptr_eq(
                block,
                &state.entry_render_cache[id].block
            ));
        }
    }
    state.set_search_query("newly arrived".into());
    state.merge_parts(vec![part(902, 902, "user", "text", "newly arrived")]);
    assert!(
        !state.rendered(80).search_matches.is_empty(),
        "content updates still refresh matching"
    );
}

#[test]
fn search_jump_wraps_and_tolerates_a_stale_match_index() {
    let mut state = TranscriptState::default();
    state.merge_parts(vec![part(
        903,
        903,
        "user",
        "text",
        "target one\n\ntarget two\n\ntarget three",
    )]);
    state.set_search_query("target".into());
    let matches = state.rendered(80).search_matches.clone();
    assert!(matches.len() >= 3);
    state.search_match_index = Some(usize::MAX);
    state.jump_search_match(80, 12, true);
    assert_eq!(state.search_match_index, Some(0));
    state.jump_search_match(80, 12, false);
    assert_eq!(state.search_match_index, Some(matches.len() - 1));
    state.set_search_query("absent".into());
    state.jump_search_match(80, 12, true);
    assert_eq!(state.search_match_index, None);
}

#[tokio::test]
async fn completed_image_layout_is_not_invalidated_by_other_media_completions() {
    use base64::Engine as _;
    let mut state = TranscriptState::default();
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4" fill="#1689bb"/></svg>"##;
    let encoded = base64::engine::general_purpose::STANDARD.encode(svg);
    state.merge_parts(vec![part(
        30,
        30,
        "user",
        "text",
        &format!("![ready](data:image/svg+xml;base64,{encoded})"),
    )]);
    let mut context = agena_tui_media::MathRenderContext::new(None, &std::env::temp_dir());
    context.enable_background_rendering();
    state.set_math_render_context(context.clone());
    for attempt in 0..8 {
        state.rendered(80);
        if state
            .entry_render_cache
            .values()
            .all(|entry| entry.remote_image_generation.is_none())
        {
            break;
        }
        assert!(
            attempt < 7,
            "media layout must converge once pixels are ready"
        );
        // This runs a real image decoding worker. Workspace-wide test runs
        // may compile and execute other renderers concurrently, so three
        // seconds is not a reliable bound for an OS-scheduled worker. Keep
        // the completion barrier and the original pixel/layout assertions,
        // but report a meaningful failure after a bounded 15-second wait.
        tokio::time::timeout(
            std::time::Duration::from_secs(15),
            context.background_changed(),
        )
        .await
        .expect("background SVG render did not signal completion within 15 seconds");
    }
    assert!(!state.entry_render_cache.is_empty());
    let rendered = state.rendered(80);
    assert!(
        rendered.lines.iter().any(|line| line.text.contains('▀')),
        "preview must really have completed"
    );
    let lines = rendered.lines.as_ptr();
    let generation = rendered.remote_image_generation;
    let palette = rendered.palette;
    for next in generation + 1..generation + 10 {
        let rendered = state.rendered_inner_at(80, 0, palette, next);
        assert_eq!(
            rendered.lines.as_ptr(),
            lines,
            "stable media must not rebuild the history vectors"
        );
    }
}

#[test]
fn media_completion_reuses_plain_history_but_reflows_image_placeholders() {
    let mut state = TranscriptState::default();
    state.merge_parts(vec![
        part(10, 10, "user", "text", "plain history without any media"),
        part(
            20,
            20,
            "user",
            "text",
            "![pending](https://entry-cache-audit.invalid/image.png)",
        ),
    ]);
    let context = agena_tui_media::MathRenderContext::new(None, &std::env::temp_dir());
    let palette = agena_tui_components::theme::active_palette();
    agena_tui_media::with_math_render_context(&context, || {
        agena_tui_media::with_deferred_remote_images(|| {
            state.rendered_inner_at(80, 0, palette, 10);
        })
    });
    let previous = state
        .entry_render_cache
        .iter()
        .map(|(id, cached)| {
            (
                *id,
                (
                    cached.remote_image_generation.is_some(),
                    cached.block.clone(),
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert!(previous.values().any(|(media, _)| *media));
    assert!(previous.values().any(|(media, _)| !*media));
    agena_tui_media::with_math_render_context(&context, || {
        agena_tui_media::with_deferred_remote_images(|| {
            state.rendered_inner_at(80, 0, palette, 11);
        })
    });
    for (id, (uses_media, block)) in previous {
        assert_eq!(
            std::sync::Arc::ptr_eq(&block, &state.entry_render_cache[&id].block),
            !uses_media
        );
    }
}

#[test]
fn selection_paint_is_window_bounded_without_changing_full_selection() {
    let mut state = TranscriptState::default();
    state.rendered(80);
    let line = std::sync::Arc::new(RenderedLine::plain(
        "abcdef",
        ratatui::style::Style::default(),
    ));
    let rendered = state.rendered.as_mut().unwrap();
    rendered.lines = vec![line; 20_000];
    rendered.line_nodes = vec![None; 20_000];
    state.interaction.text_selection = Some(TranscriptTextSelection {
        anchor: TranscriptTextPosition { line: 3, column: 1 },
        head: TranscriptTextPosition {
            line: 19_990,
            column: 4,
        },
    });
    state.interaction.visual_anchor = Some(TranscriptTextPosition { line: 3, column: 1 });
    state.interaction.cursor = Some(TranscriptCursor {
        line: 19_990,
        column: 4,
        preferred_column: 4,
        anchor: None,
        block_cursor: None,
        preferred_screen_row: 0,
    });
    for mode in [None, Some(TranscriptVisualSelectionMode::Block)] {
        state.interaction.visual_selection = mode;
        let full = state.selection_cell_ranges(80);
        for window in [0..24, 10_000..10_024, 19_980..20_100, 20_100..20_120] {
            let actual = state.selection_cell_ranges_in(80, window.clone());
            let start = window.start.min(20_000);
            let end = window.end.min(20_000);
            assert_eq!(actual, full[start..end]);
            assert!(actual.len() <= window.len());
        }
    }
}

#[test]
fn focused_action_paints_only_visible_rows_of_a_long_transcript() {
    let mut state = TranscriptState::default();
    state.rendered(80);
    let mut line = RenderedLine::plain("approve", ratatui::style::Style::default());
    line.controls
        .push(agena_tui_transcript::RenderedTranscriptControl {
            columns: 1..5,
            action: agena_tui_transcript::TranscriptControlAction::Permission {
                request_id: "request".into(),
                decision: None,
            },
            enabled: true,
        });
    let rendered = state.rendered.as_mut().unwrap();
    let line = std::sync::Arc::new(line);
    rendered.lines = vec![line; 20_000];
    rendered.line_nodes = vec![None; 20_000];
    state.interaction.cursor = Some(TranscriptCursor {
        line: 10_000,
        column: 2,
        preferred_column: 2,
        anchor: None,
        block_cursor: None,
        preferred_screen_row: 0,
    });
    let ranges = state.action_cursor_ranges(80, 10_000..10_024);
    assert_eq!(ranges.len(), 24);
    assert!(
        ranges
            .iter()
            .all(|(row, cells)| (10_000..10_024).contains(row) && *cells == (1..5))
    );
}

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

#[test]
fn a_write_acknowledgement_cannot_insert_unloaded_history_above_the_reader() {
    // The initial page is two messages; the POST /messages acknowledgement
    // carries a much wider recent window. Merging that window verbatim used to
    // prepend unread turns, which moved the reader off the reply they just
    // sent.
    let recent = vec![
        part(310, 310, "user", "text", "question two"),
        part(311, 311, "assistant", "text", "answer two"),
    ];
    let mut older = (300..310)
        .map(|id| part(id, id, "user", "text", "old turn"))
        .collect::<Vec<_>>();
    older.extend(recent.clone());

    let mut pinned = TranscriptState {
        session_id: Some(1),
        ..TranscriptState::default()
    };
    pinned.merge_parts(recent.clone());
    pinned.clamp_scroll(80, 8);
    pinned.scroll_to_bottom(80, 8);
    let lines_before = pinned.rendered(80).lines.len();
    pinned.merge_execution_parts(older.clone());
    assert_eq!(pinned.parts.len(), recent.len());
    assert_eq!(pinned.rendered(80).lines.len(), lines_before);
    pinned.clamp_scroll(80, 8);
    assert!(pinned.viewport.follow_tail);
    let max_scroll = pinned.max_scroll(80, 8);
    assert_eq!(pinned.viewport.top, max_scroll);

    let mut scrolled = TranscriptState {
        session_id: Some(1),
        ..TranscriptState::default()
    };
    scrolled.merge_parts(recent);
    scrolled.clamp_scroll(80, 3);
    scrolled.scroll_viewport_by_lines(80, 3, false, 1);
    let top_before = scrolled.viewport.top;
    let cursor_before = scrolled.interaction_line();
    scrolled.merge_execution_parts(older);
    scrolled.clamp_scroll(80, 3);
    assert_eq!(scrolled.parts.len(), 2);
    assert_eq!(scrolled.viewport.top, top_before);
    assert_eq!(scrolled.interaction_line(), cursor_before);
    assert!(!scrolled.viewport.follow_tail);
}

#[test]
fn a_write_acknowledgement_still_updates_loaded_parts_and_appends_new_ones() {
    let mut state = TranscriptState {
        session_id: Some(1),
        ..TranscriptState::default()
    };
    state.merge_parts(vec![
        part(310, 310, "user", "text", "question two"),
        part(311, 311, "assistant", "text", "answer two"),
    ]);
    let mut revised = part(311, 311, "assistant", "text", "answer two, longer");
    revised.revision = 2;
    revised.updated_at_ms = 2;
    let mut newer = part(312, 312, "user", "text", "question three");
    newer.created_at_ms = 312;
    state.merge_execution_parts(vec![
        part(300, 300, "user", "text", "old turn"),
        revised,
        newer,
    ]);
    assert_eq!(
        state
            .parts
            .iter()
            .map(|part| part.part_id)
            .collect::<Vec<_>>(),
        vec![310, 311, 312]
    );
    assert_eq!(state.parts[1].content["text"], "answer two, longer");
    assert_eq!(state.parts[2].content["text"], "question three");
}
