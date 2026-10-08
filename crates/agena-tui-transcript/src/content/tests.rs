use super::*;
use agena_domain::{
    CommandOutputStream, ContentChunk, ContentDocument, ContentId, ContentKind, DocumentMutation,
    TerminalAttributes, TerminalCellRun, TerminalSnapshot, ViewBlock,
};

fn resource(kind: ContentKind) -> ContentResource {
    let id = ContentId::new();
    let cursor = ContentCursor {
        epoch: id.0,
        sequence: 0,
    };
    ContentResource {
        resource_id: id,
        owner_session_id: 1,
        part_id: 2,
        kind,
        state: ContentState::Active,
        cursor,
        committed_cursor: cursor,
        total_bytes: 0,
        dropped_bytes: 0,
        retained_ranges: vec![],
        capture_error: None,
    }
}

fn page(resource: &ContentResource, events: Vec<(u64, ContentPayload)>) -> ContentPage {
    let mut resource = resource.clone();
    let sequence = events
        .last()
        .map_or(resource.cursor.sequence, |(sequence, _)| *sequence);
    resource.cursor.sequence = sequence;
    ContentPage {
        next_cursor: resource.cursor,
        chunks: events
            .into_iter()
            .map(|(sequence, payload)| ContentChunk {
                captured_at_ms: 1000,
                cursor: ContentCursor {
                    sequence,
                    ..resource.cursor
                },
                payload,
            })
            .collect(),
        resource,
        has_more: false,
        gap: false,
    }
}

fn log(text: &str, stream: CommandOutputStream) -> ContentPayload {
    ContentPayload::Log {
        stream,
        text: text.into(),
    }
}

#[test]
fn a_log_observation_window_reports_omitted_history_without_source_loss() {
    let resource = resource(ContentKind::Log);
    let mut view = ContentView::default();
    let text = (0..400)
        .map(|row| format!("line-{row}\n"))
        .collect::<String>();
    assert!(view.apply(page(
        &resource,
        vec![(1, log(&text, CommandOutputStream::Stdout))]
    )));
    let frame = view.frame.as_ref().unwrap();
    assert!(frame.windowed);
    assert!(!frame.gap);
    let lines = text_lines(frame);
    assert!(lines.iter().any(|line| line == "line-399"));
    assert!(!lines.iter().any(|line| line == "line-0"));
}

#[test]
fn copied_log_selection_preserves_blank_and_trailing_rows() {
    let resource = resource(ContentKind::Log);
    let mut view = ContentView::default();
    assert!(view.apply(page(
        &resource,
        vec![(1, log("  first  \n\n", CommandOutputStream::Stdout))]
    )));
    let frame = view.frame.as_ref().unwrap();
    assert_eq!(text_lines(frame), ["  first  ", "", ""]);
    let mut rendered = vec![];
    render_content(
        Some(frame),
        &mut rendered,
        80,
        &agena_tui::i18n::I18n::english(),
        None,
        agena_domain::ContentFormat::Plain,
    );
    let selection = crate::TranscriptTextSelection {
        anchor: crate::TranscriptTextPosition { line: 1, column: 0 },
        head: crate::TranscriptTextPosition {
            line: 3,
            column: usize::MAX,
        },
    };
    assert_eq!(
        crate::transcript_text_selection_text(
            &rendered,
            &[],
            &vec![None; rendered.len()],
            selection
        ),
        "  first  \n\n"
    );
}

fn screen(rows: u16, cols: u16, text: &str) -> TerminalSnapshot {
    TerminalSnapshot {
        rows,
        cols,
        cursor_row: 0,
        cursor_col: 0,
        cursor_visible: true,
        alternate_screen: false,
        bracketed_paste: false,
        application_cursor: false,
        cells: vec![TerminalCellRun {
            row: 0,
            col: 0,
            text: text.into(),
            attributes: TerminalAttributes::default(),
        }],
    }
}

fn text_lines(frame: &ContentFrame) -> Vec<String> {
    frame
        .lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

#[test]
fn ansi_carriage_returns_blank_rows_and_split_output_render_inside_the_part() {
    let resource = resource(ContentKind::Log);
    let mut view = ContentView::default();
    assert!(view.apply(page(
        &resource,
        vec![(
            1,
            log(
                "loading 10%\r\x1b[32mDone\x1b[0m\x1b[K\n\n中",
                CommandOutputStream::Stdout
            )
        )]
    )));
    assert!(view.apply(page(
        &resource,
        vec![
            (2, log("文\n", CommandOutputStream::Stdout)),
            (3, log("warning", CommandOutputStream::Stderr))
        ]
    )));
    let frame = view.frame.as_ref().unwrap();
    assert_eq!(text_lines(frame), ["Done", "", "中文", "warning"]);
    assert_eq!(frame.lines[0].spans[0].style.fg, Some(Color::Indexed(2)));
    assert_eq!(frame.lines[3].spans[0].style.fg, Some(Color::Indexed(1)));
    let mut rendered = vec![];
    render_content(
        Some(frame),
        &mut rendered,
        80,
        &agena_tui::i18n::I18n::english(),
        None,
        agena_domain::ContentFormat::Plain,
    );
    assert!(rendered.iter().all(|line| !line.text.contains('\x1b')));
    assert_eq!(rendered[1].copy_text, "Done");
    assert_eq!(rendered[1].copy_column, 6);
}

#[test]
fn terminal_geometry_empty_rows_and_wide_cells_survive_completion() {
    let resource = resource(ContentKind::Terminal);
    let mut view = ContentView::default();
    assert!(view.apply(page(
        &resource,
        vec![(
            1,
            ContentPayload::Terminal {
                screen: screen(3, 12, "中文ab")
            }
        )]
    )));
    let frame = view.frame.as_ref().unwrap();
    assert_eq!(frame.terminal_size, Some((3, 12)));
    assert_eq!(text_lines(frame), ["中文ab", "", ""]);
    let lines = frame.lines.clone();
    let mut completed = page(&frame.resource, vec![]);
    completed.resource.state = ContentState::Complete;
    completed.resource.committed_cursor = completed.resource.cursor;
    assert!(view.apply(completed));
    let frame = view.frame.as_ref().unwrap();
    assert_eq!(frame.lines, lines);
    assert_eq!(frame.resource.state, ContentState::Complete);
    let mut rendered = vec![];
    render_content(
        Some(frame),
        &mut rendered,
        9,
        &agena_tui::i18n::I18n::english(),
        None,
        agena_domain::ContentFormat::Plain,
    );
    assert_eq!(
        rendered[1].copy_text, "中",
        "clip at whole display cells without resizing the source"
    );
    assert_eq!(frame.terminal_size, Some((3, 12)));
    assert_eq!(
        rendered.len(),
        5,
        "keep empty PTY rows inside the output frame"
    );
}

#[test]
fn wrapped_log_rows_share_exact_logical_copy_text() {
    let resource = resource(ContentKind::Log);
    let mut view = ContentView::default();
    let text = "x".repeat(300);
    assert!(view.apply(page(
        &resource,
        vec![(1, log(&text, CommandOutputStream::Stdout))]
    )));
    let frame = view.frame.as_ref().unwrap();
    assert_eq!(frame.wrapped, [true, false]);
    let mut rendered = vec![];
    render_content(
        Some(frame),
        &mut rendered,
        26,
        &agena_tui::i18n::I18n::english(),
        None,
        agena_domain::ContentFormat::Plain,
    );
    let content = rendered
        .iter()
        .filter(|line| line.navigation_unit.is_some())
        .collect::<Vec<_>>();
    assert!(content.len() > 2);
    assert!(content.iter().all(|line| line.navigation_copy_text == text));
    assert!(
        content
            .iter()
            .all(|line| line.navigation_unit == content[0].navigation_unit)
    );
    assert_eq!(
        content
            .iter()
            .map(|line| line.copy_text.as_str())
            .collect::<String>(),
        text
    );
}

#[test]
fn invalid_page_is_rejected_without_applying_its_valid_prefix() {
    let resource = resource(ContentKind::Log);
    let mut view = ContentView::default();
    let initial = page(
        &resource,
        vec![(1, log("old", CommandOutputStream::Stdout))],
    );
    assert!(view.apply(initial.clone()));
    let accepted = view.frame.clone().unwrap();
    let bad = page(
        &resource,
        vec![
            (2, log("should not appear", CommandOutputStream::Stdout)),
            (
                3,
                ContentPayload::Terminal {
                    screen: screen(3, 12, "wrong kind"),
                },
            ),
        ],
    );
    assert!(!view.apply(bad));
    assert!(Arc::ptr_eq(&accepted, view.frame.as_ref().unwrap()));
    assert_eq!(view.cursor, Some(initial.next_cursor));
    assert!(view.apply(initial));
    assert_eq!(text_lines(view.frame.as_ref().unwrap()), ["old"]);
}

#[test]
fn missing_terminal_dependency_preserves_the_frame_until_a_valid_snapshot_arrives() {
    let resource = resource(ContentKind::Terminal);
    let mut view = ContentView::default();
    assert!(view.apply(page(
        &resource,
        vec![(
            1,
            ContentPayload::Terminal {
                screen: screen(3, 12, "old")
            }
        )]
    )));
    let old = view.frame.clone().unwrap();
    let bad = ContentPayload::TerminalPatch {
        base_cursor: old.resource.cursor,
        screen: screen(4, 12, "bad geometry"),
        rows_changed: vec![0],
    };
    assert!(!view.apply(page(&resource, vec![(2, bad)])));
    assert!(Arc::ptr_eq(&old, view.frame.as_ref().unwrap()));
    assert!(view.cursor.is_none());
    assert!(view.apply(page(
        &resource,
        vec![(
            3,
            ContentPayload::Terminal {
                screen: screen(4, 12, "new")
            }
        )]
    )));
    assert_eq!(
        text_lines(view.frame.as_ref().unwrap()),
        ["new", "", "", ""]
    );
}

fn progress(completed: u64) -> DocumentMutation {
    DocumentMutation::Progress {
        block_id: "scan".into(),
        phase: "scan".into(),
        completed,
        total: Some(10),
        unit: Some("files".into()),
    }
}

#[test]
fn missing_document_dependency_preserves_the_document_then_replaces_it_atomically() {
    let resource = resource(ContentKind::Document);
    let mut view = ContentView::default();
    let document = ContentDocument::default().updated(&progress(1)).unwrap();
    assert!(view.apply(page(
        &resource,
        vec![(1, ContentPayload::StructuredSnapshot { document })]
    )));
    let old = view.frame.clone().unwrap();
    let missing = ContentCursor {
        sequence: 7,
        ..old.resource.cursor
    };
    assert!(!view.apply(page(
        &resource,
        vec![(
            2,
            ContentPayload::Structured {
                base_cursor: missing,
                event: progress(2)
            }
        )]
    )));
    assert!(Arc::ptr_eq(&old, view.frame.as_ref().unwrap()));
    assert!(view.cursor.is_none());
    let document = ContentDocument::default().updated(&progress(5)).unwrap();
    assert!(view.apply(page(
        &resource,
        vec![(5, ContentPayload::StructuredSnapshot { document })]
    )));
    assert!(matches!(
        &view
            .frame
            .as_ref()
            .unwrap()
            .document
            .as_ref()
            .unwrap()
            .blocks[0],
        ViewBlock::Progress { completed: 5, .. }
    ));
}

#[test]
fn a_local_display_window_and_source_loss_have_independent_indicators() {
    let resource = resource(ContentKind::Log);
    let mut view = ContentView::default();
    assert!(view.apply(page(
        &resource,
        vec![(100, log("tail", CommandOutputStream::Stdout))]
    )));
    let frame = view.frame.as_ref().unwrap();
    assert!(frame.windowed);
    assert!(!frame.gap);
    let mut loss = page(&frame.resource, vec![]);
    loss.resource.dropped_bytes = 10;
    loss.resource.capture_error = Some("capture failed".into());
    loss.resource.state = ContentState::Interrupted;
    assert!(view.apply(loss));
    let frame = view.frame.as_ref().unwrap();
    assert!(frame.windowed);
    assert!(frame.gap);
    assert_eq!(
        frame.resource.capture_error.as_deref(),
        Some("capture failed")
    );
    assert_eq!(text_lines(frame), ["tail"]);
}

#[test]
fn resource_backed_diff_and_plain_text_render_with_semantic_copy_text() {
    let resource = resource(ContentKind::Text);
    let mut view = ContentView::default();
    let diff = "--- a/src.rs\n+++ b/src.rs\n@@ -1 +1 @@\n-old\n+new\n";
    assert!(view.apply(page(
        &resource,
        vec![(1, ContentPayload::Text { text: diff.into() })]
    )));
    let mut rendered = vec![];
    render_content(
        view.frame.as_deref(),
        &mut rendered,
        80,
        &agena_tui::i18n::I18n::english(),
        None,
        agena_domain::ContentFormat::Diff,
    );
    assert!(rendered.iter().any(|line| line.copy_text.contains("+new")));
    assert!(rendered.iter().any(|line| line.copy_text.contains("-old")));
    assert!(
        rendered
            .iter()
            .all(|line| !line.copy_text.starts_with("    │ "))
    );
    let mut rendered = vec![];
    render_content(
        view.frame.as_deref(),
        &mut rendered,
        80,
        &agena_tui::i18n::I18n::english(),
        None,
        agena_domain::ContentFormat::Plain,
    );
    assert!(rendered.iter().any(|line| line.copy_text == "+new"));
}
