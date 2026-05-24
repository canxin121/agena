use crossterm::event::{KeyCode, KeyEvent};

use crate::selection::{
    clamp_selected_index, move_selected_index, move_selected_index_end, move_selected_index_home,
    move_selected_index_page,
};

#[derive(Debug, Clone)]
pub struct SelectableListState<T> {
    pub items: Vec<T>,
    pub selected: usize,
}

impl<T> Default for SelectableListState<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
        }
    }
}

impl<T> SelectableListState<T> {
    pub fn new(items: Vec<T>, selected: usize) -> Self {
        let mut state = Self { items, selected };
        state.clamp_selection();
        state
    }

    pub fn selected_item(&self) -> Option<&T> {
        self.items.get(self.selected)
    }

    pub fn selected_item_mut(&mut self) -> Option<&mut T> {
        self.items.get_mut(self.selected)
    }

    pub fn clamp_selection(&mut self) {
        clamp_selected_index(&mut self.selected, self.items.len());
    }

    pub fn move_selection(&mut self, delta: isize) {
        move_selected_index(&mut self.selected, self.items.len(), delta);
    }

    pub fn move_selection_page(&mut self, delta: isize, page_size: usize) {
        move_selected_index_page(&mut self.selected, self.items.len(), delta, page_size);
    }

    pub fn move_selection_home(&mut self) {
        move_selected_index_home(&mut self.selected);
    }

    pub fn move_selection_end(&mut self) {
        move_selected_index_end(&mut self.selected, self.items.len());
    }

    pub fn handle_navigation_key(&mut self, key: KeyEvent, page_size: usize) -> bool {
        match key.code {
            KeyCode::PageUp => {
                self.move_selection_page(-1, page_size);
                true
            }
            KeyCode::PageDown => {
                self.move_selection_page(1, page_size);
                true
            }
            KeyCode::Home => {
                self.move_selection_home();
                true
            }
            KeyCode::End => {
                self.move_selection_end();
                true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::SelectableListState;

    #[test]
    fn new_clamps_selection_to_last_item() {
        let state = SelectableListState::new(vec!["a", "b"], 9);
        assert_eq!(state.selected, 1);
        assert_eq!(state.selected_item(), Some(&"b"));
    }

    #[test]
    fn new_resets_selection_for_empty_items() {
        let state = SelectableListState::<&str>::new(Vec::new(), 9);
        assert_eq!(state.selected, 0);
        assert_eq!(state.selected_item(), None);
    }

    #[test]
    fn movement_helpers_follow_item_count() {
        let mut state = SelectableListState::new(vec!["a", "b", "c"], 1);
        state.move_selection(-1);
        assert_eq!(state.selected, 0);

        state.move_selection_page(1, 10);
        assert_eq!(state.selected, 2);

        state.move_selection_home();
        assert_eq!(state.selected, 0);

        state.move_selection_end();
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn navigation_keys_follow_standard_list_bindings() {
        let mut state = SelectableListState::new(vec!["a", "b", "c"], 1);

        assert!(state.handle_navigation_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), 10,));
        assert_eq!(state.selected, 0);

        assert!(
            state.handle_navigation_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), 10,)
        );
        assert_eq!(state.selected, 2);

        assert!(state.handle_navigation_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE), 10,));
        assert_eq!(state.selected, 0);

        assert!(
            !state.handle_navigation_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10,)
        );
    }
}
