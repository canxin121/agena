use super::*;

impl Editor {
    fn reference_wrapped_view(&self, width: u16, height: u16) -> EditorView {
        let width = max(width as usize, 1);
        let height = max(height as usize, 1);
        let lines = wrapped_editor_lines(self.text.as_str(), width as u16);
        let cursor_line_index = self.current_line_index();
        let cursor_column = self.current_display_column();
        let cursor_visual_line = lines
            .iter()
            .enumerate()
            .find(|(_, line)| {
                line.logical_line_index == cursor_line_index
                    && cursor_column >= line.start_column
                    && cursor_column < line.end_column
            })
            .map(|(index, _)| index)
            .or_else(|| {
                lines
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, line)| line.logical_line_index == cursor_line_index)
                    .map(|(index, _)| index)
            })
            .unwrap_or(0);
        let vscroll = cursor_visual_line.saturating_sub(height.saturating_sub(1));
        let visible_lines = lines
            .iter()
            .skip(vscroll)
            .take(height)
            .map(|line| {
                slice_display_window_styled(
                    self.text.as_str(),
                    line.range.clone(),
                    line.start_column,
                    line.end_column,
                    self.elements.as_slice(),
                )
            })
            .collect::<Vec<_>>();
        let cursor_x = cursor_column
            .saturating_sub(
                lines
                    .get(cursor_visual_line)
                    .map(|line| line.start_column)
                    .unwrap_or_default(),
            )
            .min(width.saturating_sub(1));

        EditorView {
            lines: visible_lines,
            cursor_x: min(cursor_x, u16::MAX as usize) as u16,
            cursor_y: min(
                cursor_visual_line.saturating_sub(vscroll),
                u16::MAX as usize,
            ) as u16,
        }
    }
}

#[test]
fn cached_wrapping_matches_reference_for_unicode_cursor_positions() {
    let texts = [
        "",
        "abcd",
        "abc\ndef\n",
        "界界a👩‍💻b\n測試",
        "a\u{301}b\u{301}中",
        "ｶﾞｷﾞ x",
        "\u{200b}hello",
        "abc\n\n界界",
        "one two three four five",
    ];
    for text in texts {
        let mut editor = Editor::from_text(text.to_owned());
        editor.set_elements(std::iter::once(0..text.len().min(3)).collect());
        for width in [0, 1, 2, 3, 4, 8, 80] {
            for height in [0, 1, 2, 6] {
                for cursor in (0..=editor.text().len())
                    .filter(|&p| editor.text().is_char_boundary(p))
                    .collect::<Vec<_>>()
                {
                    editor.set_cursor(cursor);
                    let expected = editor.reference_wrapped_view(width, height);
                    let actual = editor.render_wrapped_view(width, height);
                    assert_eq!(
                        actual.lines, expected.lines,
                        "text={text:?} cursor={cursor} width={width} height={height}"
                    );
                    assert_eq!(
                        (actual.cursor_x, actual.cursor_y),
                        (expected.cursor_x, expected.cursor_y),
                        "text={text:?} cursor={cursor} width={width} height={height}"
                    );
                }
            }
        }
    }
}

#[test]
fn text_mutations_retire_caches_but_cursor_moves_and_clones_reuse_them() {
    let mut editor = Editor::from_text("first\nsecond".into());
    let lines = editor.wrapped_lines(4);
    let logical = editor.logical_lines();
    editor.set_cursor(0);
    assert!(Arc::ptr_eq(&lines, &editor.wrapped_lines(4)));
    let mut clone = editor.clone();
    assert!(Arc::ptr_eq(&lines, &clone.wrapped_lines(4)));
    clone.insert_str("prefix");
    assert!(!Arc::ptr_eq(&lines, &clone.wrapped_lines(4)));
    assert!(Arc::ptr_eq(&lines, &editor.wrapped_lines(4)));
    assert!(Arc::ptr_eq(&logical, &editor.logical_lines()));
    editor.remove_range(0, 2);
    assert!(!Arc::ptr_eq(&lines, &editor.wrapped_lines(4)));
    editor.set_text("replacement".into());
    assert_eq!(editor.logical_line_count(), 1);
    assert_eq!(editor.wrapped_line_count(4), 3);
    editor.clear();
    assert_eq!(editor.render_wrapped_view(4, 2).lines, vec![Line::from("")]);
    for width in 1..100 {
        editor.wrapped_line_count(width);
    }
    assert!(editor.layout.wrapped.lock().unwrap().len() <= 2);
}

#[test]
fn rendering_tail_keeps_exact_byte_ranges_and_element_highlighting() {
    let mut editor = Editor::from_text("prefix".repeat(20_000));
    editor.insert_element("[附件]");
    let actual = editor.render_wrapped_view(80, 3);
    let expected = editor.reference_wrapped_view(80, 3);
    assert_eq!(actual.lines, expected.lines);
    assert_eq!(
        (actual.cursor_x, actual.cursor_y),
        (expected.cursor_x, expected.cursor_y)
    );
    let lines = editor.wrapped_lines(80);
    assert!(lines.iter().all(|line| line.visible_range.len() <= 80));
    assert!(
        actual
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
    );
}

#[test]
#[ignore = "manual microbenchmark; timings are diagnostic, not a pass/fail budget"]
fn editor_layout_microbenchmark() {
    let editor = Editor::from_text("long prompt text 界 ".repeat(3000));
    let expected = editor.reference_wrapped_view(100, 8);
    let actual = editor.render_wrapped_view(100, 8);
    assert_eq!(actual.lines, expected.lines);
    let measure = |mut draw: Box<dyn FnMut()>| {
        let mut samples = Vec::new();
        for _ in 0..15 {
            let start = std::time::Instant::now();
            draw();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        samples[7]
    };
    let before = measure(Box::new(|| {
        std::hint::black_box(editor.reference_wrapped_view(100, 8));
    }));
    let after = measure(Box::new(|| {
        std::hint::black_box(editor.render_wrapped_view(100, 8));
    }));
    eprintln!(
        "editor render-only microbenchmark: {} bytes, debug test profile, median of 15, before={before:.3}ms after={after:.3}ms",
        editor.text().len()
    );
}
