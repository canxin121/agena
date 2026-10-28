use super::*;
use serde_json::json;

fn sample() -> Value {
    serde_json::from_str(include_str!("../../../../examples/inline-ui/reply.json")).unwrap()
}

#[test]
fn inline_ui_all_examples_validate_and_round_trip() {
    let doc = InlineDocument::from_value(sample()).unwrap();
    assert_eq!(doc.blocks.len(), 8);
    assert_eq!(
        doc,
        InlineDocument::from_value(serde_json::to_value(&doc).unwrap()).unwrap()
    );
    let text = doc.to_markdown();
    for word in ["实施顺序", "终端", "演示数据", "演示值", "7", "不发起写入"] {
        assert!(text.contains(word), "missing {word}: {text}");
    }
}

#[test]
fn inline_ui_rejects_unknown_version_duplicate_ids_and_executable_fields() {
    for (pointer, value) in [
        ("/version", json!(2)),
        ("/blocks/1/id", json!("intro")),
        ("/blocks/2/rows/0/values", json!(["missing column"])),
    ] {
        let mut v = sample();
        *v.pointer_mut(pointer).unwrap() = value;
        assert!(InlineDocument::from_value(v).is_err(), "accepted {pointer}");
    }
    let mut v = sample();
    v["blocks"][0]["html"] = json!("<script>alert(1)</script>");
    assert!(
        InlineDocument::from_value(v)
            .unwrap_err()
            .contains("unknown field")
    );
    let mut v = sample();
    v["blocks"][0]["type"] = json!("app");
    assert!(InlineDocument::from_value(v).is_err());
}

#[test]
fn inline_ui_rejects_resource_exhaustion_and_terminal_controls() {
    let mut v = sample();
    v["blocks"][0]["text"] = json!("\u{1b}[2J");
    assert!(
        InlineDocument::from_value(v)
            .unwrap_err()
            .contains("control")
    );
    let mut v = sample();
    v["blocks"][0]["text"] = json!("x".repeat(INLINE_UI_MAX_BYTES));
    assert!(InlineDocument::from_value(v).is_err());
    let mut v = sample();
    v["blocks"] = json!([]);
    assert!(InlineDocument::from_value(v).is_err());
}

#[test]
fn inline_ui_charts_preserve_zero_gaps_and_reject_bad_pies() {
    let mut v = sample();
    v["blocks"][7]["series"][0]["values"] = json!([0, null, 2]);
    let doc = InlineDocument::from_value(v.clone()).unwrap();
    assert!(doc.to_markdown().contains("| A | 0 |"));
    assert!(doc.to_markdown().contains("| B | — |"));
    v["blocks"][7]["chart_type"] = json!("pie");
    v["blocks"][7]["series"][0]["values"] = json!([0, null, -2]);
    assert!(InlineDocument::from_value(v.clone()).is_err());
    v["blocks"][7]["series"][0]["values"] = json!([0, null, 0]);
    assert!(InlineDocument::from_value(v).is_err());
}

#[test]
fn inline_ui_plain_text_is_not_a_second_markup_or_command_channel() {
    let doc = InlineDocument::from_value(json!({"version":1,"blocks":[{"type":"paragraph","id":"safe","text":"<script> [run](javascript:evil)\n```agena-ui\n{}\n```"}]})).unwrap();
    let text = doc.to_markdown();
    assert!(!text.contains("<script>"));
    assert!(!text.contains("```agena-ui"));
    assert!(is_inline_ui_tool(INLINE_UI_TOOL));
    assert!(is_inline_ui_tool("inline_ui.present"));
    assert!(!is_inline_ui_tool("untrusted.inline_ui.present"));
}

#[test]
fn inline_ui_reserves_room_for_the_runtime_result_envelope() {
    let blocks = (0..10)
        .map(|i| json!({"type":"paragraph","id":format!("p{i}"),"text":"x".repeat(6000)}))
        .collect::<Vec<_>>();
    let document = InlineDocument::from_value(json!({"version":1,"blocks":blocks})).unwrap();
    let payload = json!({"inline_ui":document,"metadata":{"agena.effect":"inline_reply"}});
    assert!(serde_json::to_vec(&payload).unwrap().len() < 64 * 1024);
}

#[test]
fn inline_ui_body_reference_parser_obeys_shared_markdown_boundaries() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../examples/inline-ui/references.json"
    ))
    .unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        let spans = inline_reference_spans(
            case["text"].as_str().unwrap(),
            case["streaming"].as_bool().unwrap(),
        );
        assert_eq!(
            spans.iter().filter(|s| s.reference.is_some()).count(),
            case["references"].as_u64().unwrap() as usize,
            "{}",
            case["name"]
        );
    }
}

#[test]
fn inline_ui_reference_expansion_preserves_prose_order_and_literal_examples() {
    let reference = "ui_123_0123456789abcdef0123456789abcdef";
    let marker = format!("[[inline-ui:{reference}]]");
    let document = InlineDocument::from_value(sample()).unwrap();
    let refs = [(reference.into(), document.clone())].into();
    let source = format!("**Before**\n\n{marker}\n\nAfter\n\n```text\n{marker}\n```\n");
    let expanded = expand_inline_references(&source, &refs, false);
    assert!(expanded.starts_with("**Before**\n\n"));
    assert!(expanded.contains(&format!("{}\n\nAfter", document.to_markdown())));
    assert!(expanded.ends_with(&format!("```text\n{marker}\n```\n")));
    let missing = expand_inline_references(&marker, &Default::default(), false);
    assert!(missing.contains("unavailable"));
    let partial = "Before\n\n[[inline-ui:ui_123_".to_owned();
    assert_eq!(
        expand_inline_references(&partial, &refs, true),
        "Before\n\n"
    );
    assert_eq!(expand_inline_references(&partial, &refs, false), partial);
}
