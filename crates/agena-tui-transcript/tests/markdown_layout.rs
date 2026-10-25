use agena_tui_transcript::{
    TranscriptTextPosition, TranscriptTextSelection, push_wrapped_line, push_wrapped_rich_line,
    render_markdown_document, transcript_text_selection_text, wrap_rich_line, wrap_table_cell,
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

#[test]
fn consecutive_image_lines_keep_independent_blocks_without_blank_lines() {
    let source = "不同尺寸与固定种子：\n![大图 960×240](https://picsum.photos/seed/md-big/960/240)\n![占位图 480×240](https://placehold.co/480x240/1f6feb/ffffff?text=Markdown+Image)";
    for width in [24, 40, 80, 120] {
        let lines = render_markdown_document(source, width);
        assert_eq!(lines[0].text, "不同尺寸与固定种子：");
        for caption in ["大图 960×240", "占位图 480×240"] {
            assert!(
                lines
                    .iter()
                    .any(|line| line.text == format!("🖼  {caption}")),
                "{width}: {lines:?}"
            );
        }
        assert!(lines.iter().all(|line| line.math.is_empty()));
        assert!(
            lines
                .iter()
                .all(|line| line.remote_image_sources.is_empty())
        );
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width))
        );
    }
}

#[test]
fn standalone_image_lines_preserve_surrounding_prose_and_one_list_marker() {
    let source = "- before **bold**\n  ![first](first.png)\n  [![second](second.png)](https://example.com/target)\n  after *italic*";
    let lines = render_markdown_document(source, 80);
    assert_eq!(lines[0].text, "• before bold");
    assert_eq!(
        lines.iter().filter(|line| line.text.contains('•')).count(),
        1
    );
    assert!(lines.iter().any(|line| line.text == "  🖼  first"));
    assert!(lines.iter().any(|line| line.text == "  🖼  second"));
    assert!(
        lines
            .iter()
            .any(|line| line.text.contains("↗ https://example.com/target"))
    );
    assert_eq!(lines.last().unwrap().text, "  after italic");
}

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
fn markdown_separates_paragraphs_and_sections_without_padding_every_block() {
    let source = "# 概览\n\n首段说明。\n\n\n第二段说明。\n\n## 执行步骤\n\n按顺序执行：\n\n- 检查配置\n- 运行验证\n\n执行完成。\n\n### 后续\n\n保留结果。";
    let lines = render_markdown_document(source, 80);
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        [
            "▌ 概览  ━━━━━━━━━━━━",
            "  首段说明。",
            "",
            "  第二段说明。",
            "",
            "▎ 执行步骤  ────────────",
            "  按顺序执行：",
            "  • 检查配置",
            "  • 运行验证",
            "",
            "  执行完成。",
            "",
            "  ▸ 后续",
            "    保留结果。",
        ]
    );
    for line in lines.iter().filter(|line| line.text.is_empty()) {
        assert!(line.copy_text.is_empty());
        assert!(line.navigation_unit.is_none());
    }
}

#[test]
fn nested_paragraphs_keep_quote_rails_and_complex_list_items_stay_distinct() {
    let source = "> 引用首段。\n>\n> 引用次段。\n>\n> - 引用项目\n>   - 子项目\n\n- 第一项\n\n  第一项的补充。\n\n- 第二项\n\n  第二项的补充。";
    let lines = render_markdown_document(source, 80);
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        [
            "┃ 引用首段。",
            "┃ ",
            "┃ 引用次段。",
            "┃ • 引用项目",
            "┃ │ ◦ 子项目",
            "• 第一项",
            "  ",
            "  第一项的补充。",
            "",
            "• 第二项",
            "  ",
            "  第二项的补充。",
        ]
    );
    let rail_gap = &lines[1];
    assert!(rail_gap.copy_text.is_empty());
    assert_eq!(rail_gap.copy_column, 2);

    for source in ["- 简单项目一\n- 简单项目二", "- 简单项目一\n\n- 简单项目二"]
    {
        let lines = render_markdown_document(source, 80);
        assert_eq!(lines.len(), 2, "{lines:?}");
    }
}

#[test]
fn one_line_code_uses_one_row_and_preserves_code_and_copy_when_wrapping() {
    let command = "printf 'hello  world' && cargo test --locked -p agena-tui-transcript";
    let source = format!("```sh\n{command}\n```");
    for width in [16, 24, 40, 80, 120] {
        let lines = render_markdown_document(&source, width);
        assert_eq!(lines.len() == 1, usize::from(width) >= command.len() + 5);
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width)),
            "{width}: {lines:?}"
        );
        assert!(lines[0].text.starts_with("sh │ "));
        assert_eq!(
            lines
                .iter()
                .map(|line| line.copy_text.as_str())
                .collect::<String>(),
            command
        );
        assert!(lines.iter().all(|line| line.copy_column == 5));
        assert!(
            lines
                .iter()
                .all(|line| line.navigation_copy_text == command)
        );
        assert!(lines.iter().all(|line| line.navigation_unit == Some(0)));
    }
}

#[test]
fn multiline_code_frames_fit_the_content_and_keep_literal_blank_lines() {
    let lines = render_markdown_document("```sh\necho one\n\necho two\n```", 100);
    assert_eq!(lines.len(), 5);
    let code_width = UnicodeWidthStr::width(lines[0].text.as_str());
    assert!(code_width <= 20, "{lines:?}");
    assert!(
        lines
            .iter()
            .all(|line| UnicodeWidthStr::width(line.text.as_str()) == code_width)
    );
    assert_eq!(lines[1].copy_text, "echo one");
    assert_eq!(lines[2].copy_text, "");
    assert_eq!(lines[3].copy_text, "echo two");
}

#[test]
fn compact_code_never_spends_more_rows_on_a_long_language_label() {
    let code = "x".repeat(2000);
    let source = format!("```a-long-language-label\n{code}\n```");
    for width in [16, 20, 24, 40, 80] {
        let lines = render_markdown_document(&source, width);
        let framed_rows = code.len().div_ceil(usize::from(width) - 4) + 2;
        assert!(
            lines.len() <= framed_rows,
            "{width}: {} > {framed_rows}",
            lines.len()
        );
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width))
        );
        assert_eq!(
            lines
                .iter()
                .map(|line| line.copy_text.as_str())
                .collect::<String>(),
            code
        );
    }
}

#[test]
fn inline_code_uses_the_code_surface_without_adding_warning_emphasis() {
    let lines = render_markdown_document("使用 `cargo test` 检查。", 80);
    let code = lines[0]
        .rich_line
        .as_ref()
        .unwrap()
        .spans
        .iter()
        .find(|span| span.content == "cargo test")
        .unwrap();
    let palette = agena_tui_components::theme::active_palette();
    assert_eq!(code.style.fg, Some(palette.code_fg));
    assert_eq!(code.style.bg, Some(palette.code_bg));
    assert!(!code.style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn visible_link_destinations_are_not_repeated_in_prose_or_tables() {
    let url = "https://example.com/docs";
    for source in [
        url.to_string(),
        format!("<{url}>"),
        format!("[{url}]({url})"),
        format!("| 文档 |\n| --- |\n| <{url}> |"),
        format!("`参考` $x$ <{url}>"),
    ] {
        let lines = render_markdown_document(&source, 100);
        let copied = lines
            .iter()
            .map(|line| line.copy_text.as_str())
            .collect::<String>();
        assert_eq!(copied.matches(url).count(), 1, "{source}: {copied}");
    }
    let lines = render_markdown_document(&format!("[文档]({url} \"使用说明\")"), 100);
    assert_eq!(lines[0].text, format!("文档 ({url} — 使用说明)"));
}

#[test]
fn explicit_rules_do_not_accumulate_after_headings_or_other_rules() {
    for source in [
        "## 标题\n\n---\n\n***\n\n正文。",
        "标题\n====\n\n---\n\n***\n\n正文。",
    ] {
        let lines = render_markdown_document(source, 80);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[1].text, "  正文。");
    }
    let lines = render_markdown_document("第一段。\n\n---\n\n第二段。", 80);
    assert_eq!(lines.len(), 3);
    assert!(lines[1].text.chars().all(|ch| ch == '─'));
}

#[test]
fn section_bodies_align_with_heading_text_and_deeper_headings_use_distinct_margins() {
    let source = "# 总览\n\n一级正文\n\n## 步骤\n\n二级正文\n\n### 细节\n\n三级正文\n\n#### 补充\n\n四级正文\n\n##### 边界\n\n五级正文\n\n###### 注释\n\n六级正文";
    let lines = render_markdown_document(source, 80);
    for (title, body, marker, column) in [
        ("总览", "一级正文", "▌", 2),
        ("步骤", "二级正文", "▎", 2),
        ("细节", "三级正文", "▸", 4),
        ("补充", "四级正文", "▹", 6),
        ("边界", "五级正文", "◇", 6),
        ("注释", "六级正文", "·", 6),
    ] {
        let heading = lines.iter().find(|line| line.copy_text == title).unwrap();
        let paragraph = lines.iter().find(|line| line.copy_text == body).unwrap();
        assert_eq!(heading.copy_column, column, "{heading:?}");
        assert_eq!(paragraph.copy_column, column, "{paragraph:?}");
        assert!(
            heading
                .text
                .starts_with(&format!("{}{marker} ", " ".repeat(column - 2)))
        );
        assert_eq!(paragraph.text, format!("{}{body}", " ".repeat(column)));
    }
    // Rules decorate spare columns on the title row, without adding rows or
    // leaking into copied Markdown/text.
    assert_eq!(lines.len(), 17);
    assert!(lines[0].text.ends_with("━━━━━━━━━━━━"));
    assert_eq!(lines[0].copy_text, "总览");
}

#[test]
fn ordered_lists_align_numbers_and_nested_guides_use_the_existing_indent() {
    let source = "9. 第一项\n10. 第二项\n    - 子项\n      - 孙项\n11. 第三项";
    let lines = render_markdown_document(source, 80);
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        [
            " 9. 第一项",
            "10. 第二项",
            "  │ ◦ 子项",
            "  │ │ ▪ 孙项",
            "11. 第三项"
        ]
    );
    for (text, column) in [
        ("第一项", 4),
        ("第二项", 4),
        ("子项", 6),
        ("孙项", 8),
        ("第三项", 4),
    ] {
        let line = lines.iter().find(|line| line.copy_text == text).unwrap();
        assert_eq!(line.copy_column, column);
        assert_eq!(
            line.text
                .get(..line.text.len() - text.len())
                .map(UnicodeWidthStr::width),
            Some(column)
        );
    }
}

#[test]
fn section_insets_shrink_on_narrow_terminals_and_keep_prose_intact() {
    let prose = "正文混排 Agena terminal content 保留所有文字与标点。";
    let source = format!("#### 详细说明\n\n{prose}");
    for width in [12, 16, 20, 24, 28, 32, 40, 60, 80] {
        let lines = render_markdown_document(&source, width);
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.text.as_str()) <= usize::from(width)),
            "{width}: {lines:?}"
        );
        let body = lines.iter().skip(1).collect::<Vec<_>>();
        let copied = body
            .iter()
            .map(|line| line.copy_text.as_str())
            .collect::<String>();
        assert_eq!(copied.replace(' ', ""), prose.replace(' ', ""));
        let expected_inset = match width {
            12 | 16 | 20 | 24 => 0,
            28 => 2,
            32 | 40 => 4,
            _ => 6,
        };
        assert!(
            body.iter().all(|line| line.copy_column == expected_inset),
            "{width}: {body:?}"
        );
        if width < 28 {
            assert_eq!(
                lines.len(),
                1 + render_markdown_document(prose, width).len()
            );
        }
    }
    // Prose without sections keeps the full original body width.
    let plain = render_markdown_document(prose, 80);
    assert_eq!(plain[0].copy_column, 0);
    assert_eq!(plain[0].text, prose);
}

#[test]
fn section_gutters_and_heading_rules_do_not_leak_into_pointer_copy() {
    let source = "## 检查结果\n\n| 项目 | 结果 |\n| --- | --- |\n| 中文 | 通过 |\n| English | ok |";
    for width in [28, 48, 80] {
        let lines = render_markdown_document(source, width);
        let select = |row, start, end| {
            transcript_text_selection_text(
                &lines,
                &[],
                &[],
                TranscriptTextSelection {
                    anchor: TranscriptTextPosition {
                        line: row,
                        column: start,
                    },
                    head: TranscriptTextPosition {
                        line: row,
                        column: end,
                    },
                },
            )
        };
        assert_eq!(select(0, 0, usize::MAX), "检查结果");
        assert_eq!(select(0, 12, usize::MAX), "");
        let mut cell_count = 0;
        for (row, line) in lines.iter().enumerate() {
            for segment in &line.copy_segments {
                assert!(segment.display_column >= 4);
                assert_eq!(
                    select(
                        row,
                        segment.display_column,
                        segment.display_column + UnicodeWidthStr::width(segment.text.as_str()) - 1
                    ),
                    segment.text,
                    "{width}: {line:?}"
                );
                cell_count += 1;
            }
        }
        assert_eq!(cell_count, 6);
    }
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
