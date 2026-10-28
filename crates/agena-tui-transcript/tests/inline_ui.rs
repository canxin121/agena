use agena_domain::InlineDocument;
use agena_tui::i18n::I18n;
use agena_tui_transcript::render_entry_detailed;
use unicode_width::UnicodeWidthStr;

#[test]
fn inline_ui_embeds_are_part_of_the_final_answer_in_order() {
    use agena_api::part::PartResource;
    let reference = "ui_123_0123456789abcdef0123456789abcdef";
    let marker = format!("```inline-ui\n{reference}\n```");
    let document = InlineDocument::from_value(
        serde_json::from_str(include_str!("../../../examples/inline-ui/reply.json")).unwrap(),
    )
    .unwrap();
    let part = PartResource {
        part_id: 10,
        role: "assistant".into(),
        kind: "text".into(),
        state: "completed".into(),
        message_boundary: true,
        run_id: Some(10),
        content: serde_json::json!({"text":format!("Before\n\n{marker}\n\nAfter")}),
        inline_ui: Some([(reference.into(), document)].into()),
        ..Default::default()
    };
    let entries = agena_tui_transcript::parts_entries(&[part]);
    for width in [28, 48, 80, 120] {
        let block = render_entry_detailed(
            &entries[0],
            width,
            &I18n::english(),
            &Default::default(),
            &Default::default(),
        );
        let text = block
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.find("Before").unwrap() < text.find("实施顺序").unwrap());
        assert!(text.find("实施顺序").unwrap() < text.find("After").unwrap());
        assert!(!text.contains("```inline-ui"));
        for line in &block.lines {
            assert!(
                line.text.width() <= usize::from(width),
                "overflow {width}: {}",
                line.text
            );
        }
    }
}

#[test]
fn inline_ui_stream_pages_preserve_source_cursors_and_hide_partial_markers() {
    use agena_domain::{
        ContentChunk, ContentCursor, ContentKind, ContentPage, ContentPayload, ContentResource,
        ContentState,
    };
    use agena_tui_transcript::content::ContentView;
    let reference = "ui_123_0123456789abcdef0123456789abcdef";
    let marker = format!("```inline-ui\n{reference}\n```");
    let document = InlineDocument::from_value(
        serde_json::from_str(include_str!("../../../examples/inline-ui/reply.json")).unwrap(),
    )
    .unwrap();
    let mut view = ContentView::default();
    let epoch = agena_domain::ContentEpoch::new().into();
    let id = agena_domain::ContentId::new();
    let mut raw = String::new();
    for (i, fragment) in [
        "Before\n\n```inline-ui\nui_123_",
        "0123456789abcdef0123456789abcdef\n```\n\nAfter",
    ]
    .into_iter()
    .enumerate()
    {
        raw.push_str(fragment);
        let cursor = ContentCursor {
            epoch,
            sequence: raw.len() as u64,
        };
        let page = ContentPage {
            inline_ui: Some(if i == 1 {
                [(reference.into(), document.clone())].into()
            } else {
                Default::default()
            }),
            resource: ContentResource {
                resource_id: id,
                owner_session_id: 1,
                part_id: 2,
                kind: ContentKind::Text,
                // A completed history source can still arrive in several pages.
                state: ContentState::Complete,
                cursor,
                committed_cursor: cursor,
                total_bytes: Some(cursor.sequence),
                dropped_bytes: 0,
                dropped_packets: 0,
                committed_dropped_bytes: 0,
                committed_dropped_packets: 0,
                retained_ranges: vec![],
                capture_error: None,
            },
            chunks: vec![ContentChunk {
                cursor,
                captured_at_ms: 0,
                payload: ContentPayload::Text {
                    text: fragment.into(),
                },
            }],
            next_cursor: cursor,
            has_more: i == 0,
            gap: false,
        };
        assert!(view.apply(page));
        assert_eq!(view.cursor.unwrap().sequence, raw.len() as u64);
        let displayed = &view.frame.as_ref().unwrap().text;
        assert!(!displayed.contains("```inline-ui"));
        if i == 0 {
            assert_eq!(displayed.as_ref(), "Before\n\n");
        } else {
            assert!(displayed.contains(&document.to_markdown()));
            assert!(displayed.ends_with("After"));
        }
    }
    assert_eq!(raw, format!("Before\n\n{marker}\n\nAfter"));
}
