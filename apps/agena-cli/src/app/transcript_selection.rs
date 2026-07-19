use std::collections::BTreeSet;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::app::{
    RenderedLine, RenderedTranscriptNode, TranscriptPointerSelection, TranscriptTextPosition,
    TranscriptTextSelection, transcript_semantic_line_range, transcript_spinner_placeholder,
};

/// Apply only pointer-specific atomicity to a raw cell range. Keyboard
/// navigation units do not imply pointer atomicity: code and table rows remain
/// character-selectable, while formula/image line boxes opt in explicitly.
pub(in crate::app) fn normalize_transcript_text_selection(
    selection: TranscriptTextSelection,
    lines: &[RenderedLine],
    nodes: &[RenderedTranscriptNode],
    line_nodes: &[Option<usize>],
) -> TranscriptTextSelection {
    let forward = selection.anchor <= selection.head;
    let (mut start, mut end) = if forward {
        (selection.anchor, selection.head)
    } else {
        (selection.head, selection.anchor)
    };
    if let Some(node) = atomic_node_at(nodes, start) {
        start = TranscriptTextPosition {
            line: node.start_line,
            column: 0,
        };
    }
    if let Some(node) = atomic_node_at(nodes, end) {
        end = TranscriptTextPosition {
            line: node.end_line.saturating_sub(1),
            column: usize::MAX,
        };
    }
    if let Some(range) = atomic_semantic_range_at(lines, nodes, line_nodes, start) {
        start = TranscriptTextPosition {
            line: range.start,
            column: 0,
        };
    }
    if let Some(range) = atomic_semantic_range_at(lines, nodes, line_nodes, end) {
        end = TranscriptTextPosition {
            line: range.end.saturating_sub(1),
            column: usize::MAX,
        };
    }
    if forward {
        TranscriptTextSelection {
            anchor: start,
            head: end,
        }
    } else {
        TranscriptTextSelection {
            anchor: end,
            head: start,
        }
    }
}

fn atomic_node_at(
    nodes: &[RenderedTranscriptNode],
    position: TranscriptTextPosition,
) -> Option<&RenderedTranscriptNode> {
    nodes.iter().find(|node| {
        node.atomic
            && !node.key.is_message_container()
            && position.line >= node.start_line
            && position.line < node.end_line
    })
}

fn atomic_semantic_range_at(
    lines: &[RenderedLine],
    nodes: &[RenderedTranscriptNode],
    line_nodes: &[Option<usize>],
    position: TranscriptTextPosition,
) -> Option<std::ops::Range<usize>> {
    let line = lines.get(position.line)?;
    if line.pointer_selection != TranscriptPointerSelection::SemanticUnit {
        return None;
    }
    line.navigation_unit?;
    let node = line_nodes
        .get(position.line)
        .and_then(|index| *index)
        .and_then(|index| nodes.get(index))?;
    transcript_semantic_line_range(lines, node, position.line)
}

/// Convert a committed terminal-cell range into clean semantic clipboard
/// text. UI prefixes are projected away, grapheme clusters remain intact, and
/// graphical units are emitted once even when they occupy multiple rows.
pub(in crate::app) fn transcript_text_selection_text(
    lines: &[RenderedLine],
    nodes: &[RenderedTranscriptNode],
    line_nodes: &[Option<usize>],
    selection: TranscriptTextSelection,
    spinner: &str,
) -> String {
    let first_line = selection.anchor.line.min(selection.head.line);
    let last_line = selection
        .anchor
        .line
        .max(selection.head.line)
        .min(lines.len().saturating_sub(1));
    if first_line >= lines.len() || first_line > last_line {
        return String::new();
    }

    let mut copied_atomic_nodes = BTreeSet::new();
    let mut copied_navigation_units = BTreeSet::new();
    let fragments = (first_line..=last_line)
        .filter_map(|line| {
            let node_index = line_nodes.get(line).and_then(|index| *index);
            if let Some(node_index) = node_index {
                if let Some(node) = nodes.get(node_index)
                    && node.atomic
                {
                    return copied_atomic_nodes
                        .insert(node_index)
                        .then(|| (line, node.copy_text.clone()));
                }
                if let Some(rendered_line) = lines.get(line)
                    && rendered_line.pointer_selection == TranscriptPointerSelection::SemanticUnit
                    && let Some(unit) = rendered_line.navigation_unit
                {
                    return copied_navigation_units
                        .insert((node_index, unit))
                        .then(|| (line, rendered_line.navigation_copy_text.clone()));
                }
            }
            let range = selection.cell_range_for_line(line)?;
            let rendered_line = &lines[line];
            if node_index.is_some() && rendered_line.copy_text.is_empty() {
                return None;
            }
            let text = if rendered_line.copy_segments.is_empty() {
                let copy_text = rendered_line
                    .copy_text
                    .replace(transcript_spinner_placeholder(), spinner);
                let copy_range = range.start.saturating_sub(rendered_line.copy_column)
                    ..range.end.saturating_sub(rendered_line.copy_column);
                display_cell_slice(copy_text.as_str(), copy_range)
            } else {
                segmented_line_slice(rendered_line, range, spinner)
            };
            (!text.is_empty()).then_some((line, text))
        })
        .collect::<Vec<_>>();
    join_selection_fragments(lines, fragments)
}

fn segmented_line_slice(
    line: &RenderedLine,
    selected: std::ops::Range<usize>,
    spinner: &str,
) -> String {
    let mut output = String::new();
    for segment in &line.copy_segments {
        let text = segment
            .text
            .replace(transcript_spinner_placeholder(), spinner);
        let width = UnicodeWidthStr::width(text.as_str());
        let display = segment.display_column..segment.display_column.saturating_add(width);
        let overlap = selected.start.max(display.start)..selected.end.min(display.end);
        if overlap.start >= overlap.end {
            continue;
        }
        let local =
            overlap.start.saturating_sub(display.start)..overlap.end.saturating_sub(display.start);
        let fragment = display_cell_slice(text.as_str(), local);
        if fragment.is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push_str(segment.separator_before.as_str());
        }
        output.push_str(fragment.as_str());
    }
    output
}

fn join_selection_fragments(lines: &[RenderedLine], fragments: Vec<(usize, String)>) -> String {
    let mut output = String::new();
    let mut previous_line: Option<usize> = None;
    for (line, fragment) in fragments {
        if let Some(previous) = previous_line {
            let same_character_unit = lines.get(previous).is_some_and(|left| {
                lines.get(line).is_some_and(|right| {
                    left.pointer_selection == TranscriptPointerSelection::Character
                        && right.pointer_selection == TranscriptPointerSelection::Character
                        && left.navigation_unit.is_some()
                        && left.navigation_unit == right.navigation_unit
                })
            });
            if !same_character_unit {
                output.push('\n');
            }
        }
        output.push_str(fragment.as_str());
        previous_line = Some(line);
    }
    output
}

fn display_cell_slice(text: &str, range: std::ops::Range<usize>) -> String {
    let mut column = 0_usize;
    text.graphemes(true)
        .filter(|grapheme| {
            let width = UnicodeWidthStr::width(*grapheme);
            let start = column;
            let end = column.saturating_add(width);
            column = end;
            start < range.end && end > range.start
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{RenderedCopySegment, Style, TranscriptNodeKey, TranscriptNodeKind};

    fn selection(anchor: (usize, usize), head: (usize, usize)) -> TranscriptTextSelection {
        TranscriptTextSelection {
            anchor: TranscriptTextPosition {
                line: anchor.0,
                column: anchor.1,
            },
            head: TranscriptTextPosition {
                line: head.0,
                column: head.1,
            },
        }
    }

    fn node(kind: TranscriptNodeKind, atomic: bool, end_line: usize) -> RenderedTranscriptNode {
        RenderedTranscriptNode {
            key: TranscriptNodeKey::MarkdownBlock {
                message_id: 1,
                part_id: 2,
                block_index: 0,
            },
            kind,
            start_line: 0,
            end_line,
            copy_text: "semantic source".to_string(),
            atomic,
            toggleable: false,
            expanded: true,
        }
    }

    #[test]
    fn keyboard_semantic_rows_do_not_force_pointer_atomicity() {
        for kind in [
            TranscriptNodeKind::MarkdownCode,
            TranscriptNodeKind::MarkdownTable,
        ] {
            let lines = vec![
                RenderedLine::plain("abcdef", Style::default()).with_navigation_unit(0, "abcdef"),
            ];
            let nodes = vec![node(kind, false, 1)];
            let raw = selection((0, 1), (0, 3));
            assert_eq!(
                normalize_transcript_text_selection(raw, &lines, &nodes, &[Some(0)]),
                raw
            );
            assert_eq!(
                transcript_text_selection_text(&lines, &nodes, &[Some(0)], raw, ""),
                "bcd"
            );
        }
    }

    #[test]
    fn table_copy_segments_support_partial_cells_and_clean_cross_cell_tabs() {
        let line =
            RenderedLine::plain("│ answer │ 42 │", Style::default()).with_copy_segments(vec![
                RenderedCopySegment {
                    display_column: 2,
                    text: "answer".to_string(),
                    separator_before: String::new(),
                },
                RenderedCopySegment {
                    display_column: 11,
                    text: "42".to_string(),
                    separator_before: "\t".to_string(),
                },
            ]);
        assert_eq!(
            transcript_text_selection_text(
                std::slice::from_ref(&line),
                &[],
                &[None],
                selection((0, 3), (0, 5)),
                "",
            ),
            "nsw"
        );
        assert_eq!(
            transcript_text_selection_text(
                std::slice::from_ref(&line),
                &[],
                &[None],
                selection((0, 2), (0, 12)),
                "",
            ),
            "answer\t42"
        );
    }

    #[test]
    fn graphical_formula_rows_expand_to_one_semantic_unit() {
        let mut lines = (0..3)
            .map(|_| {
                RenderedLine::plain("formula pixels", Style::default())
                    .with_navigation_unit(0, r"a &= \frac{b}{c}")
            })
            .collect::<Vec<_>>();
        for line in &mut lines {
            line.pointer_selection = TranscriptPointerSelection::SemanticUnit;
        }
        let nodes = vec![node(TranscriptNodeKind::MarkdownMath, false, 3)];
        let normalized = normalize_transcript_text_selection(
            selection((1, 4), (1, 6)),
            &lines,
            &nodes,
            &[Some(0), Some(0), Some(0)],
        );
        assert_eq!(normalized, selection((0, 0), (2, usize::MAX)));
        assert_eq!(
            transcript_text_selection_text(
                &lines,
                &nodes,
                &[Some(0), Some(0), Some(0)],
                normalized,
                "",
            ),
            r"a &= \frac{b}{c}"
        );
    }

    #[test]
    fn atomic_images_expand_independently_of_terminal_height() {
        let lines = (0..4)
            .map(|_| RenderedLine::plain("image cells", Style::default()))
            .collect::<Vec<_>>();
        let nodes = vec![node(TranscriptNodeKind::MarkdownImage, true, 4)];
        let normalized = normalize_transcript_text_selection(
            selection((2, 2), (2, 5)),
            &lines,
            &nodes,
            &[Some(0), Some(0), Some(0), Some(0)],
        );
        assert_eq!(normalized, selection((0, 0), (3, usize::MAX)));
    }
}
