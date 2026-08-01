#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptBlockCursor {
    pub key: TranscriptNodeKey,
    pub direction: TranscriptMoveDirection,
    pub mode: TranscriptBlockSelectionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptBlockSelectionMode {
    Entering,
    Leaving,
    Direct,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TranscriptNodeKey {
    Entry {
        entry_id: crate::TranscriptEntryId,
    },
    Content {
        entry_id: crate::TranscriptEntryId,
        content_id: Option<crate::TranscriptContentId>,
    },
    MarkdownBlock {
        entry_id: crate::TranscriptEntryId,
        content_id: crate::TranscriptContentId,
        block_index: usize,
    },
    ActivitySummary {
        entry_id: crate::TranscriptEntryId,
        /// Stable anchor for an append-only Activity run. The last Activity
        /// is deliberately excluded so streamed additions do not create a
        /// different, default-collapsed summary node.
        first_content_id: crate::TranscriptContentId,
    },
    Activity {
        entry_id: crate::TranscriptEntryId,
        content_id: crate::TranscriptContentId,
    },
}

impl TranscriptNodeKey {
    pub const fn entry_id(&self) -> crate::TranscriptEntryId {
        match self {
            Self::Entry { entry_id }
            | Self::Content { entry_id, .. }
            | Self::MarkdownBlock { entry_id, .. }
            | Self::ActivitySummary { entry_id, .. }
            | Self::Activity { entry_id, .. } => *entry_id,
        }
    }

    pub const fn is_entry_container(&self) -> bool {
        matches!(self, Self::Entry { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptNodeKind {
    Message,
    MarkdownParagraph,
    MarkdownHeading,
    MarkdownQuote,
    MarkdownAlert,
    MarkdownCode,
    MarkdownList,
    MarkdownTable,
    MarkdownMath,
    MarkdownImage,
    MarkdownFootnote,
    MarkdownDiagram,
    Activity,
}

impl TranscriptNodeKind {
    pub fn uses_atomic_navigation(self) -> bool {
        matches!(
            self,
            Self::MarkdownMath | Self::MarkdownImage | Self::MarkdownDiagram
        )
    }
}

pub fn transcript_node_kind_label(i18n: &I18n, kind: TranscriptNodeKind) -> String {
    let key = match kind {
        TranscriptNodeKind::Message => "transcript-node-kind-message",
        TranscriptNodeKind::MarkdownParagraph => "transcript-node-kind-markdown",
        TranscriptNodeKind::MarkdownHeading => "transcript-node-kind-heading",
        TranscriptNodeKind::MarkdownQuote => "transcript-node-kind-quote",
        TranscriptNodeKind::MarkdownAlert => "transcript-node-kind-alert",
        TranscriptNodeKind::MarkdownCode => "transcript-node-kind-code",
        TranscriptNodeKind::MarkdownList => "transcript-node-kind-list",
        TranscriptNodeKind::MarkdownTable => "transcript-node-kind-table",
        TranscriptNodeKind::MarkdownMath => "transcript-node-kind-math",
        TranscriptNodeKind::MarkdownImage => "transcript-node-kind-image",
        TranscriptNodeKind::MarkdownFootnote => "transcript-node-kind-footnote",
        TranscriptNodeKind::MarkdownDiagram => "transcript-node-kind-diagram",
        TranscriptNodeKind::Activity => "transcript-node-kind-activity",
    };
    i18n.text(key)
}

#[derive(Debug, Clone)]
pub struct RenderedTranscriptNode {
    pub key: TranscriptNodeKey,
    pub kind: TranscriptNodeKind,
    pub start_line: usize,
    pub end_line: usize,
    pub copy_text: String,
    /// A single semantic object whose terminal representation cannot be split
    /// into meaningful rows (for example an image, diagram, or one logical
    /// formula row). Structured display-math blocks may override the formula
    /// default after their top-level equation rows have been resolved.
    /// Multi-row containers such as code blocks and tables remain non-atomic;
    /// their renderer supplies semantic navigation rows instead.
    pub atomic: bool,
    pub toggleable: bool,
    pub expanded: bool,
}

impl RenderedTranscriptNode {
    pub fn contributes_to_aggregate_copy(&self) -> bool {
        !self.key.is_entry_container()
            && !(matches!(&self.key, TranscriptNodeKey::ActivitySummary { .. }) && self.expanded)
            && !self.copy_text.trim().is_empty()
    }
}

pub fn transcript_node_highlight_range(
    nodes: &[RenderedTranscriptNode],
    key: &TranscriptNodeKey,
) -> Option<std::ops::Range<usize>> {
    let selected = nodes.iter().find(|node| &node.key == key)?;
    Some(selected.start_line..selected.end_line)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptVerticalNavigationStep {
    SelectNode {
        node_index: usize,
        mode: TranscriptBlockSelectionMode,
    },
    MoveToLine(usize),
}

pub fn transcript_semantic_line_range(
    lines: &[RenderedLine],
    node: &RenderedTranscriptNode,
    line: usize,
) -> Option<std::ops::Range<usize>> {
    if line < node.start_line || line >= node.end_line {
        return None;
    }
    let rendered_line = lines.get(line)?;
    let Some(unit) = rendered_line.navigation_unit else {
        return (!rendered_line.copy_text.is_empty()).then_some(line..line.saturating_add(1));
    };

    let mut start = line;
    while start > node.start_line
        && lines
            .get(start.saturating_sub(1))
            .is_some_and(|candidate| candidate.navigation_unit == Some(unit))
    {
        start = start.saturating_sub(1);
    }
    let mut end = line.saturating_add(1);
    while end < node.end_line
        && lines
            .get(end)
            .is_some_and(|candidate| candidate.navigation_unit == Some(unit))
    {
        end = end.saturating_add(1);
    }
    Some(start..end)
}

fn transcript_node_entry_line(
    lines: &[RenderedLine],
    node: &RenderedTranscriptNode,
    direction: TranscriptMoveDirection,
) -> Option<usize> {
    match direction {
        TranscriptMoveDirection::Down => (node.start_line..node.end_line)
            .find(|line| transcript_semantic_line_range(lines, node, *line).is_some()),
        TranscriptMoveDirection::Up => (node.start_line..node.end_line)
            .rev()
            .find(|line| transcript_semantic_line_range(lines, node, *line).is_some()),
    }
}

/// Resolve vertical movement through the semantic hierarchy: a destination
/// message is selected before entering it, and a destination child block is
/// selected before entering its rendered rows. Atomic rich nodes never expose
/// renderer-owned rows as independent navigation stops.
pub fn transcript_vertical_navigation_step(
    nodes: &[RenderedTranscriptNode],
    lines: &[RenderedLine],
    cursor_line: usize,
    selected_cursor: Option<&TranscriptBlockCursor>,
    direction: TranscriptMoveDirection,
) -> Option<TranscriptVerticalNavigationStep> {
    let message_parent_at_cursor = || {
        nodes.iter().enumerate().find_map(|(index, node)| {
            (node.key.is_entry_container()
                && cursor_line >= node.start_line
                && cursor_line < node.end_line)
                .then_some(index)
        })
    };
    let previous_message_parent = |before_line: usize| {
        nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, node)| node.key.is_entry_container() && node.end_line <= before_line)
            .map(|(index, _)| index)
    };
    let next_message_parent = |after_line: usize| {
        nodes
            .iter()
            .enumerate()
            .find(|(_, node)| node.key.is_entry_container() && node.start_line > after_line)
            .map(|(index, _)| index)
    };
    let first_child = |entry_id: crate::TranscriptEntryId| {
        nodes
            .iter()
            .enumerate()
            .find(|(_, node)| !node.key.is_entry_container() && node.key.entry_id() == entry_id)
    };
    let last_child = |entry_id: crate::TranscriptEntryId| {
        nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, node)| !node.key.is_entry_container() && node.key.entry_id() == entry_id)
    };
    let adjacent_message_parent =
        |node: &RenderedTranscriptNode, direction: TranscriptMoveDirection| match direction {
            TranscriptMoveDirection::Up => previous_message_parent(node.start_line),
            TranscriptMoveDirection::Down => next_message_parent(node.end_line.saturating_sub(1)),
        };
    let adjacent_child = |selected_index: usize,
                          entry_id: crate::TranscriptEntryId,
                          direction: TranscriptMoveDirection| {
        let mut children = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| !node.key.is_entry_container() && node.key.entry_id() == entry_id);
        match direction {
            TranscriptMoveDirection::Up => children.rfind(|(index, _)| *index < selected_index),
            TranscriptMoveDirection::Down => children.find(|(index, _)| *index > selected_index),
        }
    };
    let select_node = |node_index: usize| TranscriptVerticalNavigationStep::SelectNode {
        node_index,
        mode: TranscriptBlockSelectionMode::Entering,
    };

    let selected_index =
        selected_cursor.and_then(|cursor| nodes.iter().position(|node| node.key == cursor.key));
    let Some(selected_index) = selected_index else {
        return message_parent_at_cursor().map(select_node).or_else(|| {
            match direction {
                TranscriptMoveDirection::Up => previous_message_parent(cursor_line),
                TranscriptMoveDirection::Down => next_message_parent(cursor_line),
            }
            .map(select_node)
        });
    };
    let selected = &nodes[selected_index];
    let entry_id = selected.key.entry_id();
    let selected_cursor = selected_cursor.expect("selected index requires a cursor");

    if selected.key.is_entry_container() {
        // Continuing in the direction that reached this message enters it;
        // reversing returns directly to the adjacent message.
        let enter_selected_message = match selected_cursor.mode {
            TranscriptBlockSelectionMode::Entering => direction == selected_cursor.direction,
            TranscriptBlockSelectionMode::Leaving => direction != selected_cursor.direction,
            TranscriptBlockSelectionMode::Direct => true,
        };
        if !enter_selected_message {
            return adjacent_message_parent(selected, direction).map(select_node);
        }

        let first = first_child(entry_id);
        let last = last_child(entry_id);
        if let Some(((first_index, only_child), (last_index, _))) = first.zip(last)
            && first_index == last_index
        {
            // A message containing only one structured formula and that
            // formula's whole-block selection have the same image-sized
            // highlight. Enter its equation rows without an indistinguishable
            // duplicate stop. Ordinary text/code/table blocks retain the
            // explicit message -> block -> row hierarchy.
            if only_child.kind == TranscriptNodeKind::MarkdownMath
                && !only_child.atomic
                && only_child.end_line.saturating_sub(only_child.start_line) > 1
                && let Some(line) = transcript_node_entry_line(lines, only_child, direction)
            {
                return Some(TranscriptVerticalNavigationStep::MoveToLine(line));
            }
            if only_child.atomic || only_child.end_line.saturating_sub(only_child.start_line) == 1 {
                return adjacent_message_parent(selected, direction).map(select_node);
            }
        }

        return match direction {
            TranscriptMoveDirection::Down => first,
            TranscriptMoveDirection::Up => last,
        }
        .map(|(node_index, _)| select_node(node_index))
        .or_else(|| adjacent_message_parent(selected, direction).map(select_node));
    }

    let enter_selected_block = match selected_cursor.mode {
        TranscriptBlockSelectionMode::Entering => direction == selected_cursor.direction,
        TranscriptBlockSelectionMode::Leaving => direction != selected_cursor.direction,
        TranscriptBlockSelectionMode::Direct => true,
    };
    let selected_height = selected.end_line.saturating_sub(selected.start_line);
    if enter_selected_block
        && selected_height > 1
        && !selected.atomic
        && let Some(line) = transcript_node_entry_line(lines, selected, direction)
    {
        return Some(TranscriptVerticalNavigationStep::MoveToLine(line));
    }

    // Atomic and one-line blocks are already at their only meaningful stop.
    adjacent_child(selected_index, entry_id, direction)
        .map(|(node_index, _)| select_node(node_index))
        .or_else(|| adjacent_message_parent(selected, direction).map(select_node))
}

pub fn transcript_vertical_line_navigation_step(
    nodes: &[RenderedTranscriptNode],
    lines: &[RenderedLine],
    cursor_line: usize,
    direction: TranscriptMoveDirection,
) -> Option<TranscriptVerticalNavigationStep> {
    let (current_index, current) = nodes.iter().enumerate().find(|(_, node)| {
        !node.key.is_entry_container()
            && cursor_line >= node.start_line
            && cursor_line < node.end_line
    })?;
    if current.atomic {
        return Some(TranscriptVerticalNavigationStep::SelectNode {
            node_index: current_index,
            mode: TranscriptBlockSelectionMode::Entering,
        });
    }

    let entry_id = current.key.entry_id();
    let adjacent_child = |direction: TranscriptMoveDirection| {
        let mut children = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| !node.key.is_entry_container() && node.key.entry_id() == entry_id);
        match direction {
            TranscriptMoveDirection::Up => children.rfind(|(index, _)| *index < current_index),
            TranscriptMoveDirection::Down => children.find(|(index, _)| *index > current_index),
        }
    };
    let adjacent_message_parent =
        |direction: TranscriptMoveDirection| match direction {
            TranscriptMoveDirection::Up => nodes.iter().enumerate().rev().find(|(_, node)| {
                node.key.is_entry_container() && node.end_line <= current.start_line
            }),
            TranscriptMoveDirection::Down => nodes.iter().enumerate().find(|(_, node)| {
                node.key.is_entry_container() && node.start_line >= current.end_line
            }),
        };
    let select_node = |node_index: usize, mode: TranscriptBlockSelectionMode| {
        TranscriptVerticalNavigationStep::SelectNode { node_index, mode }
    };

    let current_range = transcript_semantic_line_range(lines, current, cursor_line)
        .unwrap_or(cursor_line..cursor_line.saturating_add(1));
    let adjacent_line = match direction {
        TranscriptMoveDirection::Up => (current.start_line..current_range.start)
            .rev()
            .find(|line| transcript_semantic_line_range(lines, current, *line).is_some()),
        TranscriptMoveDirection::Down => (current_range.end..current.end_line)
            .find(|line| transcript_semantic_line_range(lines, current, *line).is_some()),
    };
    if let Some(line) = adjacent_line {
        return Some(TranscriptVerticalNavigationStep::MoveToLine(line));
    }

    match direction {
        TranscriptMoveDirection::Up => adjacent_child(TranscriptMoveDirection::Up)
            .map(|(node_index, _)| select_node(node_index, TranscriptBlockSelectionMode::Entering))
            .or_else(|| {
                adjacent_message_parent(TranscriptMoveDirection::Up).map(|(node_index, _)| {
                    select_node(node_index, TranscriptBlockSelectionMode::Entering)
                })
            })
            .or(Some(select_node(
                current_index,
                TranscriptBlockSelectionMode::Leaving,
            ))),
        TranscriptMoveDirection::Down => adjacent_child(TranscriptMoveDirection::Down)
            .map(|(node_index, _)| select_node(node_index, TranscriptBlockSelectionMode::Entering))
            .or_else(|| {
                adjacent_message_parent(TranscriptMoveDirection::Down).map(|(node_index, _)| {
                    select_node(node_index, TranscriptBlockSelectionMode::Entering)
                })
            })
            .or(Some(select_node(
                current_index,
                TranscriptBlockSelectionMode::Leaving,
            ))),
    }
}

pub fn transcript_should_fall_back_to_message_navigation(
    nodes: &[RenderedTranscriptNode],
    cursor_line: usize,
) -> bool {
    !nodes.iter().any(|node| {
        !node.key.is_entry_container()
            && cursor_line >= node.start_line
            && cursor_line < node.end_line
    })
}

pub fn transcript_message_navigation_target(
    nodes: &[RenderedTranscriptNode],
    cursor_line: usize,
    selected_key: Option<&TranscriptNodeKey>,
    direction: TranscriptMoveDirection,
) -> Option<usize> {
    let message_parent_at_cursor = || {
        nodes.iter().enumerate().find_map(|(index, node)| {
            (node.key.is_entry_container()
                && cursor_line >= node.start_line
                && cursor_line < node.end_line)
                .then_some(index)
        })
    };
    let previous_message_parent = |before_line: usize| {
        nodes
            .iter()
            .enumerate()
            .rev()
            .find(|(_, node)| node.key.is_entry_container() && node.end_line <= before_line)
            .map(|(index, _)| index)
    };
    let next_message_parent = |after_line: usize| {
        nodes
            .iter()
            .enumerate()
            .find(|(_, node)| node.key.is_entry_container() && node.start_line > after_line)
            .map(|(index, _)| index)
    };

    let selected_index =
        selected_key.and_then(|key| nodes.iter().position(|node| node.key == *key));
    let Some(selected_index) = selected_index else {
        return message_parent_at_cursor().or_else(|| match direction {
            TranscriptMoveDirection::Up => previous_message_parent(cursor_line),
            TranscriptMoveDirection::Down => next_message_parent(cursor_line),
        });
    };
    let selected = &nodes[selected_index];
    let message_parent = if selected.key.is_entry_container() {
        selected_index
    } else {
        nodes.iter().position(|node| {
            node.key.is_entry_container() && node.key.entry_id() == selected.key.entry_id()
        })?
    };
    match direction {
        TranscriptMoveDirection::Up => previous_message_parent(nodes[message_parent].start_line),
        TranscriptMoveDirection::Down => {
            next_message_parent(nodes[message_parent].end_line.saturating_sub(1))
        }
    }
}

pub fn initial_search_match_index(matches: &[usize], cursor_line: usize, forward: bool) -> usize {
    if forward {
        matches
            .iter()
            .position(|line| *line > cursor_line)
            .unwrap_or(0)
    } else {
        matches
            .iter()
            .rposition(|line| *line < cursor_line)
            .unwrap_or_else(|| matches.len().saturating_sub(1))
    }
}

pub fn transcript_selection_scroll_position(
    total_lines: usize,
    start_line: usize,
    end_line: usize,
    viewport_height: usize,
    current_scroll: usize,
    direction: TranscriptMoveDirection,
) -> usize {
    let viewport_height = viewport_height.max(1);
    let max_scroll = total_lines.saturating_sub(viewport_height);
    let selection_height = end_line.saturating_sub(start_line);
    let desired = if selection_height <= viewport_height {
        // Do not pin each newly selected block to the top of the viewport.
        // Keep the current context intact until the complete selection no
        // longer fits, then scroll only by the minimum amount needed.
        let current_scroll = current_scroll.min(max_scroll);
        if start_line < current_scroll {
            start_line
        } else if end_line > current_scroll.saturating_add(viewport_height) {
            end_line.saturating_sub(viewport_height)
        } else {
            current_scroll
        }
    } else {
        match direction {
            TranscriptMoveDirection::Up => end_line.saturating_sub(viewport_height),
            TranscriptMoveDirection::Down => start_line,
        }
    };
    desired.min(max_scroll)
}
use agena_tui::i18n::I18n;

use crate::{RenderedLine, TranscriptMoveDirection};
