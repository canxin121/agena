#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::app) struct TranscriptBlockCursor {
    pub(super) key: TranscriptNodeKey,
    pub(super) direction: TranscriptMoveDirection,
    pub(super) mode: TranscriptBlockSelectionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum TranscriptBlockSelectionMode {
    Entering,
    Leaving,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::app) enum TranscriptNodeKey {
    Message {
        message_id: i64,
    },
    MessagePart {
        message_id: i64,
        part_id: Option<i64>,
    },
    MarkdownBlock {
        message_id: i64,
        part_id: i64,
        block_index: usize,
    },
    ActivitySummary {
        message_id: i64,
        first_part_id: i64,
        last_part_id: i64,
    },
    ActivityPart {
        message_id: i64,
        part_id: i64,
    },
}

impl TranscriptNodeKey {
    pub(in crate::app) fn message_id(&self) -> i64 {
        match self {
            Self::Message { message_id }
            | Self::MessagePart { message_id, .. }
            | Self::MarkdownBlock { message_id, .. }
            | Self::ActivitySummary { message_id, .. }
            | Self::ActivityPart { message_id, .. } => *message_id,
        }
    }

    pub(in crate::app) fn is_message_container(&self) -> bool {
        matches!(self, Self::Message { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum TranscriptNodeKind {
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
    Activity,
}

pub(in crate::app) fn transcript_node_kind_label(i18n: &I18n, kind: TranscriptNodeKind) -> String {
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
        TranscriptNodeKind::Activity => "transcript-node-kind-activity",
    };
    ui_text::t(i18n, key)
}

#[derive(Debug, Clone)]
pub(in crate::app) struct RenderedTranscriptNode {
    pub(super) key: TranscriptNodeKey,
    pub(super) kind: TranscriptNodeKind,
    pub(super) start_line: usize,
    pub(super) end_line: usize,
    pub(super) copy_text: String,
    pub(super) toggleable: bool,
    pub(super) expanded: bool,
}

pub(in crate::app) fn transcript_node_highlight_range(
    nodes: &[RenderedTranscriptNode],
    key: &TranscriptNodeKey,
) -> Option<std::ops::Range<usize>> {
    let selected = nodes.iter().find(|node| &node.key == key)?;
    if !selected.key.is_message_container() {
        return Some(selected.start_line..selected.end_line);
    }

    // A role header identifies the message but is not part of its selectable
    // content. Derive the visual selection start from the first child while
    // preserving the parent's full range for navigation and scrolling.
    let message_id = selected.key.message_id();
    let content_start = nodes
        .iter()
        .filter(|node| !node.key.is_message_container() && node.key.message_id() == message_id)
        .map(|node| node.start_line)
        .min()
        .unwrap_or(selected.end_line)
        .clamp(selected.start_line, selected.end_line);
    Some(content_start..selected.end_line)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum TranscriptVerticalNavigationStep {
    SelectNode {
        node_index: usize,
        mode: TranscriptBlockSelectionMode,
    },
    MoveToLine(usize),
}

pub(in crate::app) fn transcript_vertical_navigation_step(
    nodes: &[RenderedTranscriptNode],
    cursor_line: usize,
    selected_cursor: Option<&TranscriptBlockCursor>,
    direction: TranscriptMoveDirection,
) -> Option<TranscriptVerticalNavigationStep> {
    let message_parent_at_cursor = || {
        nodes.iter().enumerate().find_map(|(index, node)| {
            (node.key.is_message_container()
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
            .find(|(_, node)| node.key.is_message_container() && node.end_line <= before_line)
            .map(|(index, _)| index)
    };
    let next_message_parent = |after_line: usize| {
        nodes
            .iter()
            .enumerate()
            .find(|(_, node)| node.key.is_message_container() && node.start_line > after_line)
            .map(|(index, _)| index)
    };
    let first_child = |message_id: i64| {
        nodes.iter().enumerate().find(|(_, node)| {
            !node.key.is_message_container() && node.key.message_id() == message_id
        })
    };
    let last_child = |message_id: i64| {
        nodes.iter().enumerate().rev().find(|(_, node)| {
            !node.key.is_message_container() && node.key.message_id() == message_id
        })
    };

    let selected_index =
        selected_cursor.and_then(|cursor| nodes.iter().position(|node| node.key == cursor.key));
    let Some(selected_index) = selected_index else {
        return message_parent_at_cursor()
            .map(|node_index| TranscriptVerticalNavigationStep::SelectNode {
                node_index,
                mode: TranscriptBlockSelectionMode::Entering,
            })
            .or_else(|| {
                match direction {
                    TranscriptMoveDirection::Up => previous_message_parent(cursor_line),
                    TranscriptMoveDirection::Down => next_message_parent(cursor_line),
                }
                .map(|node_index| TranscriptVerticalNavigationStep::SelectNode {
                    node_index,
                    mode: TranscriptBlockSelectionMode::Entering,
                })
            });
    };
    let selected = &nodes[selected_index];
    let message_id = selected.key.message_id();
    let selected_cursor = selected_cursor.expect("selected index requires a cursor");

    if selected.key.is_message_container() {
        if selected_cursor.mode == TranscriptBlockSelectionMode::Leaving {
            return match direction {
                TranscriptMoveDirection::Down => first_child(message_id)
                    .map(|(_, node)| TranscriptVerticalNavigationStep::MoveToLine(node.start_line))
                    .or_else(|| {
                        next_message_parent(selected.end_line.saturating_sub(1)).map(|node_index| {
                            TranscriptVerticalNavigationStep::SelectNode {
                                node_index,
                                mode: TranscriptBlockSelectionMode::Entering,
                            }
                        })
                    }),
                TranscriptMoveDirection::Up => {
                    previous_message_parent(selected.start_line).map(|previous_parent| {
                        let previous_message_id = nodes[previous_parent].key.message_id();
                        last_child(previous_message_id)
                            .map(
                                |(node_index, _)| TranscriptVerticalNavigationStep::SelectNode {
                                    node_index,
                                    mode: TranscriptBlockSelectionMode::Entering,
                                },
                            )
                            .unwrap_or(TranscriptVerticalNavigationStep::SelectNode {
                                node_index: previous_parent,
                                mode: TranscriptBlockSelectionMode::Entering,
                            })
                    })
                }
            };
        }
        return match direction {
            TranscriptMoveDirection::Down => first_child(message_id)
                .map(
                    |(node_index, _)| TranscriptVerticalNavigationStep::SelectNode {
                        node_index,
                        mode: TranscriptBlockSelectionMode::Entering,
                    },
                )
                .or_else(|| {
                    next_message_parent(selected.end_line.saturating_sub(1)).map(|node_index| {
                        TranscriptVerticalNavigationStep::SelectNode {
                            node_index,
                            mode: TranscriptBlockSelectionMode::Entering,
                        }
                    })
                }),
            TranscriptMoveDirection::Up => last_child(message_id).map(|(node_index, _)| {
                TranscriptVerticalNavigationStep::SelectNode {
                    node_index,
                    mode: TranscriptBlockSelectionMode::Entering,
                }
            }),
        };
    }

    let enter_selected_block = match selected_cursor.mode {
        TranscriptBlockSelectionMode::Entering => direction == selected_cursor.direction,
        TranscriptBlockSelectionMode::Leaving => direction != selected_cursor.direction,
    };
    if enter_selected_block {
        return Some(TranscriptVerticalNavigationStep::MoveToLine(
            match direction {
                TranscriptMoveDirection::Up => selected.end_line.saturating_sub(1),
                TranscriptMoveDirection::Down => selected.start_line,
            },
        ));
    }

    match direction {
        TranscriptMoveDirection::Up => nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                !node.key.is_message_container() && node.key.message_id() == message_id
            })
            .rfind(|(index, _)| *index < selected_index)
            .map(|(_, node)| {
                TranscriptVerticalNavigationStep::MoveToLine(node.end_line.saturating_sub(1))
            })
            .or_else(|| {
                nodes
                    .iter()
                    .enumerate()
                    .find(|(_, node)| {
                        node.key.is_message_container() && node.key.message_id() == message_id
                    })
                    .map(
                        |(node_index, _)| TranscriptVerticalNavigationStep::SelectNode {
                            node_index,
                            mode: TranscriptBlockSelectionMode::Leaving,
                        },
                    )
            }),
        TranscriptMoveDirection::Down => nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                !node.key.is_message_container() && node.key.message_id() == message_id
            })
            .find(|(index, _)| *index > selected_index)
            .map(
                |(node_index, _)| TranscriptVerticalNavigationStep::SelectNode {
                    node_index,
                    mode: TranscriptBlockSelectionMode::Entering,
                },
            )
            .or_else(|| {
                next_message_parent(selected.end_line.saturating_sub(1)).map(|node_index| {
                    TranscriptVerticalNavigationStep::SelectNode {
                        node_index,
                        mode: TranscriptBlockSelectionMode::Entering,
                    }
                })
            }),
    }
}

pub(in crate::app) fn transcript_vertical_line_navigation_step(
    nodes: &[RenderedTranscriptNode],
    cursor_line: usize,
    direction: TranscriptMoveDirection,
) -> Option<TranscriptVerticalNavigationStep> {
    let (current_index, current) = nodes.iter().enumerate().find(|(_, node)| {
        !node.key.is_message_container()
            && cursor_line >= node.start_line
            && cursor_line < node.end_line
    })?;
    let message_id = current.key.message_id();

    match direction {
        TranscriptMoveDirection::Up if cursor_line > current.start_line => Some(
            TranscriptVerticalNavigationStep::MoveToLine(cursor_line.saturating_sub(1)),
        ),
        TranscriptMoveDirection::Up => nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                !node.key.is_message_container() && node.key.message_id() == message_id
            })
            .rfind(|(index, _)| *index < current_index)
            .map(
                |(node_index, _)| TranscriptVerticalNavigationStep::SelectNode {
                    node_index,
                    mode: TranscriptBlockSelectionMode::Entering,
                },
            )
            .or_else(|| {
                nodes
                    .iter()
                    .enumerate()
                    .find(|(_, node)| {
                        node.key.is_message_container() && node.key.message_id() == message_id
                    })
                    .map(
                        |(node_index, _)| TranscriptVerticalNavigationStep::SelectNode {
                            node_index,
                            mode: TranscriptBlockSelectionMode::Leaving,
                        },
                    )
            }),
        TranscriptMoveDirection::Down if cursor_line + 1 < current.end_line => Some(
            TranscriptVerticalNavigationStep::MoveToLine(cursor_line.saturating_add(1)),
        ),
        TranscriptMoveDirection::Down => nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                !node.key.is_message_container() && node.key.message_id() == message_id
            })
            .find(|(index, _)| *index > current_index)
            .map(
                |(node_index, _)| TranscriptVerticalNavigationStep::SelectNode {
                    node_index,
                    mode: TranscriptBlockSelectionMode::Entering,
                },
            )
            .or_else(|| {
                nodes
                    .iter()
                    .enumerate()
                    .find(|(_, node)| {
                        node.key.is_message_container() && node.start_line >= current.end_line
                    })
                    .map(
                        |(node_index, _)| TranscriptVerticalNavigationStep::SelectNode {
                            node_index,
                            mode: TranscriptBlockSelectionMode::Entering,
                        },
                    )
            }),
    }
}

/// Line navigation owns the boundaries of message children. If there is no
/// next/previous child or message at such a boundary, do not fall through to
/// message navigation: that would select the enclosing message at its opposite
/// edge and make Down from a final line appear to wrap to the first line.
pub(in crate::app) fn transcript_should_fall_back_to_message_navigation(
    nodes: &[RenderedTranscriptNode],
    cursor_line: usize,
) -> bool {
    !nodes.iter().any(|node| {
        !node.key.is_message_container()
            && cursor_line >= node.start_line
            && cursor_line < node.end_line
    })
}

pub(in crate::app) fn transcript_message_navigation_target(
    nodes: &[RenderedTranscriptNode],
    cursor_line: usize,
    selected_key: Option<&TranscriptNodeKey>,
    direction: TranscriptMoveDirection,
) -> Option<usize> {
    let message_parent_at_cursor = || {
        nodes.iter().enumerate().find_map(|(index, node)| {
            (node.key.is_message_container()
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
            .find(|(_, node)| node.key.is_message_container() && node.end_line <= before_line)
            .map(|(index, _)| index)
    };
    let next_message_parent = |after_line: usize| {
        nodes
            .iter()
            .enumerate()
            .find(|(_, node)| node.key.is_message_container() && node.start_line > after_line)
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
    let message_parent = if selected.key.is_message_container() {
        selected_index
    } else {
        nodes.iter().position(|node| {
            node.key.is_message_container() && node.key.message_id() == selected.key.message_id()
        })?
    };
    match direction {
        TranscriptMoveDirection::Up => previous_message_parent(nodes[message_parent].start_line),
        TranscriptMoveDirection::Down => {
            next_message_parent(nodes[message_parent].end_line.saturating_sub(1))
        }
    }
}

pub(in crate::app) fn initial_search_match_index(
    matches: &[usize],
    cursor_line: usize,
    forward: bool,
) -> usize {
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

pub(in crate::app) fn transcript_selection_scroll_position(
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
use crate::app::{I18n, TranscriptMoveDirection, ui_text};
