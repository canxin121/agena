//! Dashboard selection state.

use crate::selection::SelectionCursor;

#[derive(Debug, Clone)]
pub struct DashboardSelectionState<TFocus>
where
    TFocus: Copy + Eq,
{
    focus_order: [TFocus; 3],
    focus: TFocus,
    top: SelectionCursor,
    left: SelectionCursor,
    right: SelectionCursor,
}

impl<TFocus> DashboardSelectionState<TFocus>
where
    TFocus: Copy + Eq,
{
    pub fn new(
        focus_order: [TFocus; 3],
        focus: TFocus,
        top_selected: usize,
        left_selected: usize,
        right_selected: usize,
    ) -> Self {
        Self {
            focus_order,
            focus,
            top: SelectionCursor::new(top_selected),
            left: SelectionCursor::new(left_selected),
            right: SelectionCursor::new(right_selected),
        }
    }

    pub fn focus(&self) -> TFocus {
        self.focus
    }

    pub fn set_focus(&mut self, focus: TFocus) {
        self.focus = focus;
    }

    pub fn next_focus(&mut self) {
        self.focus = if self.focus == self.focus_order[0] {
            self.focus_order[1]
        } else if self.focus == self.focus_order[1] {
            self.focus_order[2]
        } else {
            self.focus_order[0]
        };
    }

    pub fn prev_focus(&mut self) {
        self.focus = if self.focus == self.focus_order[0] {
            self.focus_order[2]
        } else if self.focus == self.focus_order[1] {
            self.focus_order[0]
        } else {
            self.focus_order[1]
        };
    }

    pub fn top_selected(&self) -> usize {
        self.top.selected
    }

    pub fn left_selected(&self) -> usize {
        self.left.selected
    }

    pub fn right_selected(&self) -> usize {
        self.right.selected
    }

    pub fn set_top_selected(&mut self, selected: usize) {
        self.top.selected = selected;
    }

    pub fn set_left_selected(&mut self, selected: usize) {
        self.left.selected = selected;
    }

    pub fn set_right_selected(&mut self, selected: usize) {
        self.right.selected = selected;
    }

    pub fn clamp_top(&mut self, item_count: usize) {
        self.top.clamp(item_count);
    }

    pub fn clamp_left(&mut self, item_count: usize) {
        self.left.clamp(item_count);
    }

    pub fn clamp_right(&mut self, item_count: usize) {
        self.right.clamp(item_count);
    }

    pub fn move_top(&mut self, item_count: usize, delta: isize) {
        self.top.move_by(item_count, delta);
    }

    pub fn move_left(&mut self, item_count: usize, delta: isize) {
        self.left.move_by(item_count, delta);
    }

    pub fn move_right(&mut self, item_count: usize, delta: isize) {
        self.right.move_by(item_count, delta);
    }

    pub fn move_top_page(&mut self, item_count: usize, delta: isize, page_size: usize) {
        self.top.move_page(item_count, delta, page_size);
    }

    pub fn move_left_page(&mut self, item_count: usize, delta: isize, page_size: usize) {
        self.left.move_page(item_count, delta, page_size);
    }

    pub fn move_right_page(&mut self, item_count: usize, delta: isize, page_size: usize) {
        self.right.move_page(item_count, delta, page_size);
    }

    pub fn move_top_home(&mut self) {
        self.top.move_home();
    }

    pub fn move_left_home(&mut self) {
        self.left.move_home();
    }

    pub fn move_right_home(&mut self) {
        self.right.move_home();
    }

    pub fn move_top_end(&mut self, item_count: usize) {
        self.top.move_end(item_count);
    }

    pub fn move_left_end(&mut self, item_count: usize) {
        self.left.move_end(item_count);
    }

    pub fn move_right_end(&mut self, item_count: usize) {
        self.right.move_end(item_count);
    }
}

#[cfg(test)]
mod tests {
    use super::DashboardSelectionState;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Pane {
        Top,
        Left,
        Right,
    }

    #[test]
    fn dashboard_focus_cycles_symmetrically() {
        let mut state =
            DashboardSelectionState::new([Pane::Top, Pane::Left, Pane::Right], Pane::Top, 0, 0, 0);

        state.next_focus();
        assert_eq!(state.focus(), Pane::Left);
        state.prev_focus();
        assert_eq!(state.focus(), Pane::Top);
        state.prev_focus();
        assert_eq!(state.focus(), Pane::Right);
        state.next_focus();
        assert_eq!(state.focus(), Pane::Top);
    }
}
