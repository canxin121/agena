use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectionCursor {
    pub selected: usize,
}

impl SelectionCursor {
    pub fn new(selected: usize) -> Self {
        Self { selected }
    }

    pub fn clamp(&mut self, item_count: usize) {
        clamp_selected_index(&mut self.selected, item_count);
    }

    pub fn move_by(&mut self, item_count: usize, delta: isize) {
        move_selected_index(&mut self.selected, item_count, delta);
    }

    pub fn move_page(&mut self, item_count: usize, delta: isize, page_size: usize) {
        move_selected_index_page(&mut self.selected, item_count, delta, page_size);
    }

    pub fn move_home(&mut self) {
        move_selected_index_home(&mut self.selected);
    }

    pub fn move_end(&mut self, item_count: usize) {
        move_selected_index_end(&mut self.selected, item_count);
    }

    pub fn handle_navigation_key(
        &mut self,
        key: KeyEvent,
        item_count: usize,
        page_size: usize,
    ) -> bool {
        match key.code {
            KeyCode::PageUp => {
                self.move_page(item_count, -1, page_size);
                true
            }
            KeyCode::PageDown => {
                self.move_page(item_count, 1, page_size);
                true
            }
            KeyCode::Home => {
                self.move_home();
                true
            }
            KeyCode::End => {
                self.move_end(item_count);
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_by(item_count, -1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_by(item_count, 1);
                true
            }
            _ => false,
        }
    }
}

pub fn clamp_selected_index(selected: &mut usize, item_count: usize) {
    if item_count == 0 {
        *selected = 0;
    } else {
        *selected = (*selected).min(item_count.saturating_sub(1));
    }
}

pub fn move_selected_index(selected: &mut usize, item_count: usize, delta: isize) {
    if item_count == 0 {
        *selected = 0;
        return;
    }
    let current = *selected as isize;
    let last = item_count.saturating_sub(1) as isize;
    *selected = current.saturating_add(delta).clamp(0, last) as usize;
}

pub fn move_selected_index_page(
    selected: &mut usize,
    item_count: usize,
    delta: isize,
    page_size: usize,
) {
    move_selected_index(
        selected,
        item_count,
        delta.saturating_mul(page_size.max(1) as isize),
    );
}

pub fn move_selected_index_home(selected: &mut usize) {
    *selected = 0;
}

pub fn move_selected_index_end(selected: &mut usize, item_count: usize) {
    if item_count == 0 {
        *selected = 0;
    } else {
        *selected = item_count.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        SelectionCursor, clamp_selected_index, move_selected_index, move_selected_index_end,
        move_selected_index_home, move_selected_index_page,
    };

    #[test]
    fn clamp_selected_index_resets_empty_selection() {
        let mut selected = 7;
        clamp_selected_index(&mut selected, 0);
        assert_eq!(selected, 0);
    }

    #[test]
    fn move_selected_index_clamps_to_bounds() {
        let mut selected = 1;
        move_selected_index(&mut selected, 3, 10);
        assert_eq!(selected, 2);

        move_selected_index(&mut selected, 3, -10);
        assert_eq!(selected, 0);
    }

    #[test]
    fn page_home_and_end_helpers_follow_item_count() {
        let mut selected = 1;
        move_selected_index_page(&mut selected, 5, 1, 10);
        assert_eq!(selected, 4);

        move_selected_index_home(&mut selected);
        assert_eq!(selected, 0);

        move_selected_index_end(&mut selected, 5);
        assert_eq!(selected, 4);
    }

    #[test]
    fn selection_cursor_routes_through_shared_helpers() {
        let mut cursor = SelectionCursor::new(1);
        cursor.move_by(3, 10);
        assert_eq!(cursor.selected, 2);

        cursor.move_home();
        assert_eq!(cursor.selected, 0);

        cursor.move_page(3, 1, 10);
        assert_eq!(cursor.selected, 2);

        cursor.move_end(3);
        assert_eq!(cursor.selected, 2);
    }

    #[test]
    fn selection_cursor_navigation_keys_follow_standard_bindings() {
        let mut cursor = SelectionCursor::new(1);

        assert!(cursor.handle_navigation_key(
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            3,
            10,
        ));
        assert_eq!(cursor.selected, 0);

        assert!(cursor.handle_navigation_key(
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            3,
            10,
        ));
        assert_eq!(cursor.selected, 2);

        assert!(cursor.handle_navigation_key(
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            3,
            10,
        ));
        assert_eq!(cursor.selected, 0);
    }
}
