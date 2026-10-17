use super::*;
use agena_tui_transcript::{RenderedTranscriptControl, TranscriptControlAction};

impl TranscriptState {
    pub(crate) fn control_at_cursor(&mut self, width: u16) -> Option<RenderedTranscriptControl> {
        let cursor = self.interaction.cursor.clone()?;
        if cursor.block_cursor.is_some() || self.has_visual_selection() {
            return None;
        }
        self.control_at_position(width, cursor.line, cursor.column)
    }

    fn control_at_position(
        &mut self,
        width: u16,
        line: usize,
        column: usize,
    ) -> Option<RenderedTranscriptControl> {
        self.rendered(width)
            .lines
            .get(line)?
            .controls
            .iter()
            .filter(|control| control.enabled)
            .min_by_key(|control| {
                if column < control.columns.start {
                    control.columns.start - column
                } else {
                    column.saturating_sub(control.columns.end.saturating_sub(1))
                }
            })
            .cloned()
    }

    /// All wrapped rows of the focused action share one restrained highlight.
    pub(crate) fn action_cursor_ranges(&mut self, width: u16) -> BTreeMap<usize, Range<usize>> {
        let Some(control) = self.control_at_cursor(width) else {
            return BTreeMap::new();
        };
        self.rendered(width)
            .lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                line.controls
                    .iter()
                    .find(|candidate| candidate.enabled && candidate.action == control.action)
                    .map(|candidate| (index, candidate.columns.clone()))
            })
            .collect()
    }

    fn action_start(&mut self, width: u16, line: usize, action: &TranscriptControlAction) -> usize {
        let lines = &self.rendered(width).lines;
        let mut start = line;
        while start > 0
            && lines[start - 1]
                .controls
                .iter()
                .any(|control| &control.action == action)
        {
            start -= 1;
        }
        start
    }

    fn action_end(&mut self, width: u16, line: usize, action: &TranscriptControlAction) -> usize {
        let lines = &self.rendered(width).lines;
        let mut end = line;
        while end + 1 < lines.len()
            && lines[end + 1]
                .controls
                .iter()
                .any(|control| &control.action == action)
        {
            end += 1;
        }
        end
    }

    pub(super) fn move_action_cursor_horizontally(
        &mut self,
        width: u16,
        height: u16,
        forward: bool,
        count: usize,
    ) -> bool {
        let Some(current) = self.control_at_cursor(width) else {
            return false;
        };
        let Some(node) = self.current_cursor_node_cloned(width) else {
            return false;
        };
        let mut controls = Vec::new();
        for (line, rendered) in self
            .rendered(width)
            .lines
            .iter()
            .enumerate()
            .take(node.end_line)
            .skip(node.start_line)
        {
            for control in rendered.controls.iter().filter(|control| control.enabled) {
                if !controls
                    .iter()
                    .any(|(_, previous): &(usize, RenderedTranscriptControl)| {
                        previous.action == control.action
                    })
                {
                    controls.push((line, control.clone()));
                }
            }
        }
        let Some(index) = controls
            .iter()
            .position(|(_, control)| control.action == current.action)
        else {
            return false;
        };
        let next = if forward {
            index.saturating_add(count.max(1))
        } else {
            index.saturating_sub(count.max(1))
        };
        let (line, column) = if !forward && count > index {
            (node.start_line, 0)
        } else if let Some((line, control)) = controls.get(next) {
            (*line, control.columns.start)
        } else {
            let line = self.focusable_line_near(width, node.end_line, true);
            (line, 0)
        };
        self.set_cursor_position_with_reveal(
            width,
            height,
            line,
            column,
            column,
            TranscriptRevealPolicy::Minimal,
        );
        true
    }

    pub(super) fn action_cursor_vertical_target(
        &mut self,
        width: u16,
        mut line: usize,
        preferred: usize,
        direction: TranscriptMoveDirection,
        count: usize,
    ) -> (usize, usize) {
        let mut column = preferred;
        for _ in 0..count {
            if let Some(current) = self.control_at_position(width, line, column) {
                line = match direction {
                    TranscriptMoveDirection::Up => self.action_start(width, line, &current.action),
                    TranscriptMoveDirection::Down => self.action_end(width, line, &current.action),
                };
            }
            let mut next = self.focusable_line_after_steps(width, line, direction, 1);
            while next != line
                && self.rendered(width).lines.get(next).is_some_and(|row| {
                    !row.controls.is_empty() && row.controls.iter().all(|control| !control.enabled)
                })
            {
                let after = self.focusable_line_after_steps(width, next, direction, 1);
                if after == next {
                    next = line;
                    break;
                }
                next = after;
            }
            if next == line {
                break;
            }
            line = next;
            if let Some(control) = self.control_at_position(width, line, preferred) {
                line = self.action_start(width, line, &control.action);
                column = control.columns.start;
            } else {
                column = preferred;
            }
        }
        (line, column)
    }

    /// Pointer activation is exact; whitespace and disabled controls stay text.
    pub(crate) fn focus_pointer_control(
        &mut self,
        width: u16,
        height: u16,
        position: TranscriptTextPosition,
    ) -> bool {
        let hit = self
            .rendered(width)
            .lines
            .get(position.line)
            .is_some_and(|line| {
                line.controls
                    .iter()
                    .any(|control| control.enabled && control.columns.contains(&position.column))
            });
        if !hit {
            return false;
        }
        self.cancel_text_selection(width, height);
        self.set_cursor_position_with_reveal(
            width,
            height,
            position.line,
            position.column,
            position.column,
            TranscriptRevealPolicy::Minimal,
        );
        true
    }

    pub(crate) fn focus_part_headline(&mut self, width: u16, height: u16) {
        if let Some(node) = self.current_cursor_node_cloned(width) {
            self.set_cursor_position_with_reveal(
                width,
                height,
                node.start_line,
                0,
                0,
                TranscriptRevealPolicy::Minimal,
            );
        }
    }

    pub(crate) fn focus_request_control(&mut self, width: u16, height: u16, request_id: &str) {
        let target = self.rendered(width).lines.iter().enumerate().find_map(|(line, rendered)|
            rendered.controls.iter().find(|control| control.enabled && matches!(&control.action,
                TranscriptControlAction::UserInput { request_id: id, .. }
                    | TranscriptControlAction::Permission { request_id: id, .. } if id == request_id))
                .map(|control| (line, control.columns.start)));
        if let Some((line, column)) = target {
            self.set_cursor_position_with_reveal(
                width,
                height,
                line,
                column,
                column,
                TranscriptRevealPolicy::Minimal,
            );
        }
    }
}
