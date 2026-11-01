//! Restrict Vim word-motion work to rows actually needed by the motion.
//! Grow geometrically at a provisional boundary; never change the existing
//! word/WORD, semantic-wrap, empty-line, table or Unicode classification rules.

use super::*;

fn positions_in(
    rendered: &RenderedTranscript,
    rows: Range<usize>,
) -> Vec<TranscriptGraphemePosition> {
    let mut positions = Vec::new();
    for (line, rendered_line) in rendered
        .lines
        .iter()
        .enumerate()
        .skip(rows.start)
        .take(rows.end.saturating_sub(rows.start))
    {
        if !transcript_rendered_line_is_focusable(rendered, line) {
            continue;
        }
        let line_positions = transcript_word_grapheme_positions(line, rendered_line);
        let end_column = line_positions
            .iter()
            .rev()
            .find(|position| position.cursorable)
            .map(|position| {
                position
                    .position
                    .column
                    .saturating_add(UnicodeWidthStr::width(position.grapheme.as_str()))
            })
            .unwrap_or(rendered_line.copy_column);
        let empty_line = !line_positions.iter().any(|position| position.cursorable);
        positions.extend(line_positions);
        let continues_navigation_unit = rendered_line.navigation_unit.is_some()
            && rendered
                .lines
                .get(line.saturating_add(1))
                .is_some_and(|next| next.navigation_unit == rendered_line.navigation_unit);
        if continues_navigation_unit {
            continue;
        }
        // Vim's word functions see a NUL after every line. Keeping an
        // explicit non-cursor character here prevents equal Unicode
        // classes on adjacent logical rows from becoming one word.
        // Wrapped terminal rows that share a navigation unit are one
        // logical Vim line and deliberately do not get this boundary.
        positions.push(TranscriptGraphemePosition {
            position: TranscriptTextPosition {
                line,
                column: end_column,
            },
            grapheme: String::new(),
            cursorable: empty_line,
            end_of_line: true,
            empty_line,
        });
    }
    positions
}

pub(super) fn destination(
    rendered: &RenderedTranscript,
    line: usize,
    column: usize,
    forward: bool,
    to_end: bool,
    big_word: bool,
    count: usize,
) -> Option<TranscriptTextPosition> {
    resolve(rendered, line, column, forward, to_end, big_word, count).0
}

fn resolve(
    rendered: &RenderedTranscript,
    line: usize,
    column: usize,
    forward: bool,
    to_end: bool,
    big_word: bool,
    count: usize,
) -> (Option<TranscriptTextPosition>, usize) {
    if line >= rendered.lines.len() {
        return (None, 0);
    }
    let mut extent = 16usize;
    let mut visited = 0;
    loop {
        let rows = if forward {
            line..line.saturating_add(extent).min(rendered.lines.len())
        } else {
            line.saturating_sub(extent.saturating_sub(1))..line + 1
        };
        let more = if forward {
            rows.end < rendered.lines.len()
        } else {
            rows.start > 0
        };
        visited += rows.len();
        let positions = positions_in(rendered, rows);
        let Some(mut index) = positions.iter().rposition(|position| {
            position.position.line == line
                && position.position.column <= column
                && position.cursorable
        }) else {
            return (None, visited);
        };
        let first = positions
            .iter()
            .position(|position| position.cursorable)
            .unwrap_or(index);
        let last = positions
            .iter()
            .rposition(|position| position.cursorable)
            .unwrap_or(index);
        let mut moved = false;
        let mut expand = false;
        for _ in 0..count.max(1) {
            let Some(next) =
                transcript_word_motion_target(&positions, index, forward, to_end, big_word)
            else {
                expand = more;
                break;
            };
            // The full-history algorithm is allowed to clamp at real EOF/BOF.
            // A clamped target on our partial boundary is provisional: read
            // more rows and retry before deciding where the cursor should go.
            if more && ((forward && next >= last) || (!forward && next <= first)) {
                expand = true;
                break;
            }
            // A numeric repeat stops when real EOF/BOF cannot move further.
            if next == index {
                break;
            }
            moved = true;
            index = next;
        }
        if expand {
            extent = extent.saturating_mul(2).max(extent.saturating_add(1));
            continue;
        }
        return (moved.then_some(positions[index].position), visited);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rendered() -> RenderedTranscript {
        let mut state = TranscriptState::default();
        state.rendered(80);
        state.rendered.take().unwrap()
    }

    fn reference(
        rendered: &RenderedTranscript,
        line: usize,
        column: usize,
        forward: bool,
        to_end: bool,
        big_word: bool,
        count: usize,
    ) -> Option<TranscriptTextPosition> {
        let positions = positions_in(rendered, 0..rendered.lines.len());
        let mut index = positions.iter().rposition(|position| {
            position.position.line == line
                && position.position.column <= column
                && position.cursorable
        })?;
        let mut moved = false;
        for _ in 0..count.max(1) {
            let Some(next) =
                transcript_word_motion_target(&positions, index, forward, to_end, big_word)
            else {
                break;
            };
            moved |= next != index;
            index = next;
        }
        moved.then_some(positions[index].position)
    }

    #[test]
    fn local_windows_match_full_history_for_all_word_directions_and_wraps() {
        let mut rendered = rendered();
        let sources = [
            "one two_three",
            "界字かなカナ test",
            "",
            "  ",
            "punct.a-b",
            "👩‍💻x y",
            "\t",
            "continued",
            " tail",
        ];
        rendered.lines = (0..80)
            .map(|i| {
                let mut row = RenderedLine::plain(
                    sources[i % sources.len()],
                    ratatui::style::Style::default(),
                );
                // Navigation units join soft wraps and make genuine empty rows stops.
                row.navigation_unit = Some(if i % 9 == 8 { i - 1 } else { i });
                std::sync::Arc::new(row)
            })
            .collect();
        rendered.line_nodes = vec![None; rendered.lines.len()];
        for line in [0, 1, 15, 16, 33, 64, 79] {
            for column in [0, 2, 8, 100] {
                for forward in [false, true] {
                    for to_end in [false, true] {
                        for big_word in [false, true] {
                            for count in [0, 1, 2, 25, 500] {
                                assert_eq!(
                                    destination(
                                        &rendered, line, column, forward, to_end, big_word, count
                                    ),
                                    reference(
                                        &rendered, line, column, forward, to_end, big_word, count
                                    ),
                                    "line={line}, column={column}, fwd={forward}, end={to_end}, WORD={big_word}, count={count}",
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn ordinary_word_moves_do_not_scan_a_twenty_thousand_line_history() {
        let mut rendered = rendered();
        let row = std::sync::Arc::new(RenderedLine::plain(
            "alpha beta gamma",
            ratatui::style::Style::default(),
        ));
        rendered.lines = vec![row; 20_000];
        rendered.line_nodes = vec![None; rendered.lines.len()];
        for forward in [false, true] {
            let (target, rows) = resolve(&rendered, 10_000, 7, forward, false, false, 1);
            assert_eq!(
                target,
                reference(&rendered, 10_000, 7, forward, false, false, 1)
            );
            assert_eq!(
                rows, 16,
                "ordinary movement should only read its initial window"
            );
        }
    }

    #[test]
    fn huge_repeat_counts_stop_at_the_real_navigation_boundary() {
        let mut rendered = rendered();
        let line = std::sync::Arc::new(RenderedLine::plain(
            "alpha beta",
            ratatui::style::Style::default(),
        ));
        rendered.lines = vec![line];
        rendered.line_nodes = vec![None];
        assert_eq!(
            destination(&rendered, 0, 0, true, false, false, usize::MAX),
            Some(TranscriptTextPosition { line: 0, column: 9 })
        );
        assert_eq!(
            destination(&rendered, 0, 9, false, false, false, usize::MAX),
            Some(TranscriptTextPosition { line: 0, column: 0 })
        );
    }

    #[test]
    fn table_separators_keep_the_full_history_word_semantics() {
        let mut rendered = rendered();
        rendered.lines = (0..50)
            .map(|index| {
                let mut row = RenderedLine::plain("table", ratatui::style::Style::default());
                row.navigation_unit = Some(index);
                row.copy_segments = vec![
                    agena_tui_transcript::RenderedCopySegment {
                        display_column: 2,
                        text: "key_one".into(),
                        separator_before: String::new(),
                    },
                    agena_tui_transcript::RenderedCopySegment {
                        display_column: 14,
                        text: "中字 value".into(),
                        separator_before: "\t".into(),
                    },
                ];
                std::sync::Arc::new(row)
            })
            .collect();
        rendered.line_nodes = vec![None; rendered.lines.len()];
        for column in [2, 5, 14, 18, 22] {
            for forward in [false, true] {
                for to_end in [false, true] {
                    for big_word in [false, true] {
                        for count in [1, 2, 50] {
                            assert_eq!(
                                destination(
                                    &rendered, 25, column, forward, to_end, big_word, count
                                ),
                                reference(&rendered, 25, column, forward, to_end, big_word, count)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn moves_across_long_blank_gaps_expand_until_the_real_target() {
        let mut rendered = rendered();
        rendered.lines = (0..500)
            .map(|i| {
                std::sync::Arc::new(RenderedLine::plain(
                    if i == 0 {
                        "first"
                    } else if i == 499 {
                        "last"
                    } else {
                        "   "
                    },
                    ratatui::style::Style::default(),
                ))
            })
            .collect();
        rendered.line_nodes = vec![None; rendered.lines.len()];
        for (line, forward) in [(0, true), (499, false)] {
            assert_eq!(
                destination(&rendered, line, 0, forward, false, false, 1),
                reference(&rendered, line, 0, forward, false, false, 1)
            );
        }
    }
}
