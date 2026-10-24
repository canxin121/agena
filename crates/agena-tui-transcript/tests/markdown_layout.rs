use agena_tui_transcript::{
    push_wrapped_line, push_wrapped_rich_line, render_markdown_document, wrap_rich_line,
    wrap_table_cell,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Widget,
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const INTRO: &str = "我是运行在 Agena 上的智能体（agent），不是单纯的聊天机器人——我可以直接在你这台环境里动手干活。";

fn text(lines: &[Line<'_>]) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

fn without_spaces(value: &str) -> String {
    value.chars().filter(|ch| !ch.is_whitespace()).collect()
}

#[test]
fn chinese_prose_uses_the_row_instead_of_backtracking_to_an_english_space() {
    for width in [32, 60, 80, 100] {
        let lines = wrap_rich_line(&[Span::raw(INTRO)], width, width);
        let lines = text(&lines);
        assert_eq!(lines.len() == 1, UnicodeWidthStr::width(INTRO) <= width);
        // Leave room for an indivisible punctuation/English unit such as
        // “（agent），”, while rejecting the early break after “Agena”.
        assert!(
            UnicodeWidthStr::width(lines[0].as_str()) >= width - 10,
            "{width}: {lines:?}"
        );
        assert!(lines[0].contains("上的智能体"), "{width}: {lines:?}");
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= width)
        );
        assert_eq!(without_spaces(&lines.concat()), without_spaces(INTRO));
    }
}

#[test]
fn english_words_fit_exactly_and_continuations_do_not_start_with_spaces() {
    for source in ["one two three four five", "one two     three four five"] {
        let lines = text(&wrap_rich_line(&[Span::raw(source)], 7, 7));
        assert_eq!(lines, ["one two", "three", "four", "five"]);
    }
    assert_eq!(
        text(&wrap_rich_line(&[Span::raw("alpha beta gamma")], 10, 6)),
        ["alpha beta", "gamma"]
    );
}

#[test]
fn cjk_punctuation_stays_with_the_adjacent_text() {
    let lines = text(&wrap_rich_line(
        &[Span::raw(
            "这是中文（包含括号），也有句号。接着继续测试「标点」。",
        )],
        14,
        14,
    ));
    for line in &lines {
        assert!(!line.starts_with(['，', '。', '）', '」']), "{lines:?}");
        assert!(!line.ends_with(['（', '「']), "{lines:?}");
        assert!(UnicodeWidthStr::width(line.as_str()) <= 14);
    }
}

#[test]
fn long_identifiers_wrap_in_both_plain_and_styled_text_with_copy_offsets() {
    let source = "deepseek-v4.1-flash_abcdefghijklmnopqrstuvwxyz0123456789";
    let style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let mut plain = Vec::new();
    push_wrapped_line(&mut plain, "• ", "  ", source, style, 18);
    let mut rich = Vec::new();
    push_wrapped_rich_line(&mut rich, "• ", "  ", Line::styled(source, style), 18);
    assert_eq!(
        plain.iter().map(|line| &line.text).collect::<Vec<_>>(),
        rich.iter().map(|line| &line.text).collect::<Vec<_>>()
    );
    for lines in [plain, rich] {
        assert!(lines.len() > 1);
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= 18)
        );
        assert!(lines.iter().all(|line| line.copy_column == 2));
        assert_eq!(
            lines
                .iter()
                .map(|line| line.copy_text.as_str())
                .collect::<String>(),
            source
        );
    }
    // A nested prefix may leave exactly one column for ASCII text.
    let mut narrow = Vec::new();
    push_wrapped_line(&mut narrow, "│ ", "  ", "abc", style, 3);
    assert_eq!(
        narrow
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["│ a", "  b", "  c"]
    );
}

#[test]
fn wrapping_preserves_graphemes_and_styles_across_span_boundaries() {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let source = "甲e\u{301}👩\u{200d}💻乙🇨🇳丙";
    let spans = [
        Span::raw("甲e"),
        Span::styled("\u{301}👩\u{200d}", bold),
        Span::styled("💻乙🇨", bold),
        Span::raw("🇳丙"),
    ];
    let lines = wrap_rich_line(&spans, 4, 4);
    assert_eq!(text(&lines), ["甲e\u{301}", "👩\u{200d}💻乙", "🇨🇳丙"]);
    assert_eq!(text(&lines).concat(), source);
    let styled: Vec<_> = lines
        .iter()
        .flat_map(|line| &line.spans)
        .filter(|span| span.style == bold)
        .map(|span| span.content.as_ref())
        .collect();
    // A terminal glyph is atomic; use its leading character's style when a
    // source span boundary falls inside that glyph.
    assert_eq!(styled.concat(), "👩\u{200d}💻乙🇨🇳");
    let boundaries: Vec<_> = source
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain([source.len()])
        .collect();
    let mut offset = 0;
    for line in text(&lines) {
        offset += line.len();
        assert!(boundaries.contains(&offset));
    }
    let area = Rect::new(0, 0, 4, 1);
    for line in &lines {
        let mut buffer = Buffer::empty(area);
        line.render(area, &mut buffer);
        let visible: String = (0..4).map(|x| buffer[(x, 0)].symbol()).collect();
        assert_eq!(without_spaces(&visible), without_spaces(&line.to_string()));
    }
}

#[test]
fn markdown_preserves_hard_breaks_and_reflows_soft_breaks() {
    let source = "**首段** Agena\n继续说明  \n硬换行<br>下一行\n\n独立段落";
    let lines: Vec<_> = render_markdown_document(source, 80)
        .into_iter()
        .map(|line| line.text)
        .collect();
    assert!(
        lines.iter().any(|line| line == "首段 Agena 继续说明"),
        "{lines:?}"
    );
    assert!(lines.iter().any(|line| line == "硬换行"), "{lines:?}");
    assert!(lines.iter().any(|line| line == "下一行"), "{lines:?}");
    assert!(lines.iter().any(|line| line == "独立段落"), "{lines:?}");
}

#[test]
fn width_sweep_preserves_content_without_overflow_or_empty_continuations() {
    for source in [
        INTRO,
        "one two three four five   ",
        "AST 检索与 LSP 符号、网页检索抓取、文档与图片/音视频解析，以及调用 MCP/子代理等。",
        "https://example.com/abcdefghijklmnopqrstuvwxyz/0123456789?key=long_value",
        "甲e\u{301}👩\u{200d}💻乙🇨🇳丙👨\u{200d}👩\u{200d}👧\u{200d}👦丁",
        "alpha\u{a0}beta gamma\u{202f}delta",
    ] {
        for width in 2..=120 {
            let lines = text(&wrap_rich_line(&[Span::raw(source)], width, width));
            assert!(
                lines.iter().all(|line| !line.is_empty()),
                "{width}: {lines:?}"
            );
            assert!(
                lines
                    .iter()
                    .all(|line| UnicodeWidthStr::width(line.as_str()) <= width),
                "{width}: {lines:?}"
            );
            // Only ordinary spaces at a visual wrap may be omitted. Keep
            // non-breaking spaces, joiners, emoji and all punctuation intact.
            assert_eq!(lines.concat().replace(' ', ""), source.replace(' ', ""));
        }
    }
}

#[test]
fn table_cells_reflow_without_losing_content_at_multiple_widths() {
    let value = "我是 Agena 上的智能体（agent），支持文件操作和交互式终端。";
    let source = format!(
        "| 项 | 值 |\n| --- | --- |\n| 模型 | **deepseek-v4.1-flash** |\n| 说明 | {value} |\n| 工作区 | workspace 2 |"
    );
    for width in [20, 40, 80, 120] {
        let lines = render_markdown_document(&source, width);
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width)),
            "{width}: {lines:?}"
        );
        let mut expected_borders = None;
        for line in &lines {
            let Some(rich) = &line.rich_line else {
                continue;
            };
            let area = Rect::new(0, 0, width, 1);
            let mut buffer = Buffer::empty(area);
            rich.render(area, &mut buffer);
            let borders: Vec<_> = (0..width)
                .filter(|x| buffer[(*x, 0)].symbol() == "│")
                .collect();
            if !borders.is_empty() {
                assert_eq!(borders.len(), 3, "{width}: {line:?}");
                if let Some(expected) = &expected_borders {
                    assert_eq!(&borders, expected, "{width}: {line:?}");
                } else {
                    expected_borders = Some(borders);
                }
            }
        }
        let copied: String = lines.iter().map(|line| line.copy_text.as_str()).collect();
        assert!(
            without_spaces(&copied).contains("deepseek-v4.1-flash"),
            "{width}: {copied}"
        );
        assert!(
            without_spaces(&copied).contains(&without_spaces(value)),
            "{width}: {copied}"
        );
        assert!(
            without_spaces(&copied).contains("workspace2"),
            "{width}: {copied}"
        );
    }
    let emoji = "👩\u{200d}💻🇨🇳e\u{301}👩\u{200d}💻";
    assert_eq!(wrap_table_cell(emoji, 3).concat(), emoji);
}
