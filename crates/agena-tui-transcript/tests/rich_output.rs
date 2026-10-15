use agena_tui_transcript::render_markdown_document;
use unicode_width::UnicodeWidthStr;

#[test]
fn mermaid_renders_supported_families_as_themed_diagrams_with_source_copy() {
    for source in [
        "flowchart TD\n A[Start] --> B[Finish]",
        "sequenceDiagram\n Alice->>Bob: Hello",
        "stateDiagram-v2\n Idle --> Active",
        "classDiagram\n A --> B",
        "erDiagram\n USER ||--o{ TASK : owns",
    ] {
        let markdown = format!("```mermaid\n{source}\n```");
        let lines = render_markdown_document(&markdown, 100);
        assert!(
            lines
                .iter()
                .any(|line| line.text.contains('─') || line.text.contains('│')),
            "{source}"
        );
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= 100)
        );
        assert!(
            lines
                .iter()
                .any(|line| line.navigation_copy_text.contains(source)),
            "copy must preserve source: {source}"
        );
        assert!(lines.iter().any(|line| {
            line.rich_line
                .as_ref()
                .is_some_and(|line| line.spans.iter().any(|span| span.style.fg.is_some()))
        }));
    }
}

#[test]
fn unsupported_narrow_and_unfinished_fences_keep_source() {
    for source in [
        "```mermaid\nflowchart LR\n A[Long left label] --> B[Long right label]\n```",
        "```mermaid\ngantt\n title Unsupported\n```",
        "```mermaid\nflowchart TD\n A --> B",
    ] {
        let lines = render_markdown_document(source, 24);
        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("flowchart") || line.text.contains("gantt"))
        );
    }
}

#[test]
fn chart_is_a_semantic_rendering_and_app_does_not_dump_html() {
    let source = r#"{"type":"bar","title":"Requests","labels":["Mon","Tue"],"series":[{"name":"API","values":[-3,7]}]}"#;
    let lines = render_markdown_document(&format!("```agena-chart\n{source}\n```"), 60);
    assert!(lines.iter().any(|line| line.text.contains('━')));
    assert!(
        lines
            .iter()
            .any(|line| line.navigation_copy_text.contains(source))
    );
    let app = render_markdown_document(
        "```agena-ui\n{\"id\":\"demo\",\"title\":\"Tasks\",\"html\":\"<button>Secret markup</button>\"}\n```",
        80,
    );
    assert!(app.iter().any(|line| line.text.contains("Tasks")));
    assert!(app.iter().all(|line| !line.text.contains("Secret markup")));
}
