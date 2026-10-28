use super::*;
use serde_json::json;
fn fixture() -> InlineReply {
    let document = InlineDocument::from_value(
        serde_json::from_str(include_str!(
            "../../../../examples/inline-ui/live-dashboard.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let reference = "ui_123_0123456789abcdef0123456789abcdef";
    InlineReply {
        source: format!("Before\n\n```inline-ui\n{reference}\n```\n\nAfter"),
        documents: [(reference.into(), document)].into(),
        ..Default::default()
    }
}
fn check_geometry(rows: &[RenderedLine], width: u16) {
    for row in rows {
        assert!(
            row.text.width() <= usize::from(width),
            "overflow {width}: {:?} width={}",
            row.text,
            row.text.width()
        );
        for control in &row.controls {
            assert!(
                control.columns.start < control.columns.end
                    && control.columns.end <= usize::from(width),
                "bad hit region {width}: {:?} {}",
                control.columns,
                row.text
            );
        }
        if let Some(line) = &row.rich_line {
            assert_eq!(
                row.text,
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            );
        }
    }
}
#[test]
fn native_ui_preserves_layout_progress_charts_and_real_control_geometry() {
    let reply = fixture();
    let i18n = I18n::english();
    for width in [12, 24, 40, 80, 120, 180] {
        let rows = render_reply(&reply, width, &i18n);
        check_geometry(&rows, width);
        let text = rows
            .iter()
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.starts_with("Before"));
        assert!(text.ends_with("After"));
        assert!(!text.contains("```inline-ui"));
        assert!(rows.iter().flat_map(|row|&row.controls).any(|control|matches!(&control.action,TranscriptControlAction::InlineUi(target)if matches!(target.event,InlineEvent::Choose(_)))));
        if width >= 40 {
            assert!(
                text.chars()
                    .any(|character| ('\u{2801}'..='\u{28ff}').contains(&character)),
                "graph must use actual terminal graphics: {text}"
            );
            assert!(text.contains('┌'));
        }
        if let Ok(directory) = std::env::var("AGENA_INLINE_UI_NATIVE_ARTIFACTS") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::Path::new(&directory).join(format!("inline-native-{width}.txt")),
                text,
            )
            .unwrap();
        }
    }
    assert!(reply.copy_text().contains("118500"));
}
#[test]
fn native_ui_static_blocks_are_native_and_disclosures_are_not_lost() {
    let mut reply = fixture();
    let reference = reply.documents.keys().next().unwrap().clone();
    let document = InlineDocument::from_value(
        serde_json::from_str(include_str!("../../../../examples/inline-ui/reply.json")).unwrap(),
    )
    .unwrap();
    reply.documents.insert(reference.clone(), document);
    let rows = render_reply(&reply, 100, &I18n::english());
    check_geometry(&rows, 100);
    let events = rows
        .iter()
        .flat_map(|row| &row.controls)
        .filter_map(|c| {
            if let TranscriptControlAction::InlineUi(target) = &c.action {
                Some(target)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert!(events.iter().any(|c| c.event == InlineEvent::Details));
    assert!(events.iter().any(|c| c.event == InlineEvent::Data));
    let target = events
        .iter()
        .find(|c| c.event == InlineEvent::Details)
        .unwrap();
    reply
        .presentation
        .entry(reference)
        .or_default()
        .entry(target.block.clone())
        .or_default()
        .open
        .insert(target.node.clone());
    let text = render_reply(&reply, 100, &I18n::english())
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("受控内容块"));
    assert!(reply.copy_text().contains("受控内容块"));
}
#[test]
fn native_ui_all_chart_kinds_zero_gaps_and_negative_values_are_bounded() {
    for kind in ["bar", "line", "area", "pie"] {
        let values = if kind == "pie" {
            json!([0, 2, null, 8])
        } else {
            json!([-3, 0, null, 8])
        };
        let document=InlineDocument::from_value(json!({"version":1,"blocks":[{"type":"chart","id":"chart","title":"图表","chart_type":kind,"labels":["一月","二月","三月","四月"],"series":[{"name":"示例","values":values}]}]})).unwrap();
        let mut reply = fixture();
        let reference = reply.documents.keys().next().unwrap().clone();
        reply.documents.insert(reference.clone(), document);
        for width in [8, 20, 40, 80, 120] {
            let rows = render_reply(&reply, width, &I18n::english());
            check_geometry(&rows, width);
        }
        reply
            .presentation
            .entry(reference)
            .or_default()
            .entry("chart".into())
            .or_default()
            .data
            .insert("chart".into());
        let text = render_reply(&reply, 80, &I18n::english())
            .iter()
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("三月"));
        assert!(text.contains('—'));
        assert!(!text.contains("NaN"));
    }
}
#[test]
fn native_ui_never_activates_examples_or_user_text_and_copy_stays_complete() {
    let mut reply = fixture();
    let reference = reply.documents.keys().next().unwrap().clone();
    reply.source = format!(
        "````\n```inline-ui\n{reference}\n```\n````\n\n> ```inline-ui\n> {reference}\n> ```"
    );
    let rows = render_reply(&reply, 100, &I18n::english());
    assert!(rows.iter().all(|row| row.controls.is_empty()));
    assert_eq!(reply.source, reply.copy_text());
}

#[test]
fn native_ui_all_component_kinds_wrap_with_valid_actions_and_copy_geometry() {
    let nodes = json!([
        {"id":"root","kind":"stack","children":["row","tabs","columns","controls","progress","details","table","tasks","timeline","callout","chart","divider"]},
        {"id":"row","kind":"row","children":["heading","badge","button"]},
        {"id":"heading","kind":"heading","props":{"text":"所有原生组件"}},
        {"id":"badge","kind":"badge","props":{"text":"有状态","tone":"success"}},
        {"id":"button","kind":"button","action":"toggle","props":{"label":"执行明确操作"}},
        {"id":"tabs","kind":"tabs","bind":"choice","props":{"label":"视图"}},
        {"id":"columns","kind":"grid","props":{"columns":2},"children":["card","metric"]},
        {"id":"card","kind":"card","props":{"title":"真实分组"},"children":["text"]},
        {"id":"text","kind":"text","props":{"text":"很长的中文信息以及没有空格的long_identifier_abcdefghijklmnop🌱需要正常换行。"}},
        {"id":"metric","kind":"metric","props":{"label":"数值","value":12345,"detail":"单位和来源","tone":"success"}},
        {"id":"controls","kind":"grid","props":{"columns":2},"children":["select","input","number","slider","checkbox","switch"]},
        {"id":"select","kind":"select","bind":"choice","props":{"label":"选项"}},
        {"id":"input","kind":"input","bind":"query","props":{"label":"文本输入"}},
        {"id":"number","kind":"number","bind":"number","props":{"label":"数值输入"}},
        {"id":"slider","kind":"slider","bind":"number","props":{"label":"滑块"}},
        {"id":"checkbox","kind":"checkbox","bind":"enabled","props":{"label":"勾选"}},
        {"id":"switch","kind":"switch","bind":"enabled","props":{"label":"开关"}},
        {"id":"progress","kind":"progress","props":{"label":"明确进度","value":42,"max":100}},
        {"id":"details","kind":"details","props":{"title":"补充细节"},"children":["detailText"]},
        {"id":"detailText","kind":"text","props":{"text":"可展开，复制仍完整。"}},
        {"id":"table","kind":"table","props":{"title":"表格","columns":[{"key":"key","label":"项目"},{"key":"value","label":"内容"}],"rows":[{"key":"项目一","value":"完整数据"}]}},
        {"id":"tasks","kind":"tasks","bind":"done","props":{"title":"任务","items":[{"id":"task","title":"任务标题","detail":"描述"}]}},
        {"id":"timeline","kind":"timeline","props":{"title":"时间线","items":[{"id":"event","title":"发生的事件","detail":"说明"}]}},
        {"id":"callout","kind":"callout","props":{"title":"注意","text":"只影响本地状态","tone":"warning"}},
        {"id":"chart","kind":"chart","props":{"title":"图表","chart_type":"line","labels":["一","二","三"],"series":[{"name":"数值","values":[1,null,3]}]}},
        {"id":"divider","kind":"divider"}
    ]);
    let kinds = nodes
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["kind"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(kinds.len(), 24);
    let document=InlineDocument::from_value(json!({"version":1,"blocks":[{"type":"panel","id":"all","program":{
        "root":"root","nodes":nodes,
        "state":{"choice":{"kind":"choice","initial":"a","options":[{"value":"a","label":"第一页"},{"value":"b","label":"第二页"}]},"query":{"kind":"text","initial":"中文🌱","max_length":128},"number":{"kind":"number","initial":42,"min":0,"max":100},"enabled":{"kind":"boolean","initial":false},"done":{"kind":"selection","initial":[]}},
        "actions":{"toggle":[{"type":"toggle","key":"enabled"}]}
    }}]})).unwrap();
    let mut reply = fixture();
    let reference = reply.documents.keys().next().unwrap().clone();
    reply.documents.insert(reference, document);
    for width in [12, 20, 40, 80, 120, 180] {
        let rows = render_reply(&reply, width, &I18n::english());
        check_geometry(&rows, width);
        for row in &rows {
            for segment in &row.copy_segments {
                assert!(
                    segment.display_column + segment.text.width() <= usize::from(width),
                    "copy projection overflow: {row:?}"
                );
            }
        }
        assert!(rows.iter().any(|row| row.text.contains('●')));
        assert!(rows.iter().flat_map(|row|&row.controls).any(|control|matches!(&control.action,TranscriptControlAction::InlineUi(target)if target.event==InlineEvent::Slider)));
    }
}

#[test]
fn native_ui_prose_reference_links_keep_one_complete_markdown_context() {
    let mut reply = fixture();
    let reference = reply.documents.keys().next().unwrap().clone();
    reply.source = format!(
        "Read [the source][evidence].\n\n```inline-ui\n{reference}\n```\n\n[evidence]: https://example.com/source\n"
    );
    let native = render_reply(&reply, 100, &I18n::english());
    let original = crate::render_markdown_document(&reply.source, 100);
    assert_eq!(native[0].text, original[0].text);
    assert!(!native[0].text.contains("[evidence]"));
    assert!(native.iter().any(|row| !row.controls.is_empty()));
}

#[test]
fn native_ui_single_line_slots_keep_multiline_text_from_escaping_their_cells() {
    let document=InlineDocument::from_value(json!({"version":1,"title":"标题\n下一行\t安全","blocks":[{"type":"panel","id":"panel","program":{
        "root":"root","state":{"choice":{"kind":"choice","initial":"a","options":[{"value":"a","label":"第一\n选项\t文字"},{"value":"b","label":"另一个\u{202e}选项"}]}},
        "nodes":[{"id":"root","kind":"stack","children":["tabs","metric"]},{"id":"tabs","kind":"tabs","bind":"choice","props":{"label":"选项"}},{"id":"metric","kind":"metric","props":{"label":"多行\n标题\t文字","value":42}}]
    }}]})).unwrap();
    let mut reply = fixture();
    let reference = reply.documents.keys().next().unwrap().clone();
    reply.documents.insert(reference, document);
    let rows = render_reply(&reply, 48, &I18n::english());
    check_geometry(&rows, 48);
    for row in rows {
        assert!(
            !row.text.contains(['\n', '\t', '\u{202e}', '\u{1b}']),
            "unsafe row: {:?}",
            row.text
        );
    }
}

#[test]
fn quick_start_example_has_native_controls_and_matches_web_live_update_values() {
    let mut reply = InlineReply::default();
    let reference = "ui_99_0123456789abcdef0123456789abcdef";
    reply.source = format!("Before\n\n\x60\x60\x60inline-ui\n{reference}\n\x60\x60\x60\n\nAfter");
    let mut document = InlineDocument::from_value(
        serde_json::from_str(include_str!(
            "../../../../examples/inline-ui/quick-start.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let InlineBlock::Panel { program, .. } = &document.blocks[0] else {
        panic!("panel")
    };
    let selected = program
        .input(&UiState::new(), "slider", json!(50))
        .unwrap()
        .state;
    reply.documents.insert(reference.into(), document.clone());
    for width in [24, 40, 80] {
        let rows = render_reply(&reply, width, &I18n::english());
        check_geometry(&rows, width);
        assert!(rows.iter().flat_map(|row| &row.controls).any(
            |control| matches!(&control.action, TranscriptControlAction::InlineUi(target)
                if target.node == "slider")
        ));
    }
    reply
        .states
        .insert(reference.into(), [("scenario".into(), selected)].into());
    let text = render_reply(&reply, 80, &I18n::english())
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("180"), "{text}");
    assert!(text.contains("Above 150"), "{text}");
    let InlineBlock::Panel { program, .. } = &mut document.blocks[0] else {
        panic!("panel")
    };
    program.data.insert("base".into(), json!(160));
    reply.documents.insert(reference.into(), document);
    let text = render_reply(&reply, 80, &I18n::english())
        .iter()
        .map(|row| row.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("240"), "{text}");
    assert!(text.contains("Adjustment"), "{text}");
}
