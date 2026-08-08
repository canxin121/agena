//! Selectable list widget.

use crossterm::event::KeyEvent;

use crate::{NavigationAction, navigation_action, structural_navigation_action};

use crate::selection::{
    clamp_selected_index, clamped_selected_index, move_selected_index, move_selected_index_end,
    move_selected_index_home, move_selected_index_page,
};

#[derive(Debug, Clone)]
/// State of a selectable list.
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
        let selected = clamped_selected_index(selected, items.len());
        Self { items, selected }
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
        self.handle_navigation_action(navigation_action(key), page_size)
    }

    pub fn handle_structural_navigation_key(&mut self, key: KeyEvent, page_size: usize) -> bool {
        self.handle_navigation_action(structural_navigation_action(key), page_size)
    }

    fn handle_navigation_action(
        &mut self,
        action: Option<NavigationAction>,
        page_size: usize,
    ) -> bool {
        match action {
            Some(NavigationAction::PageUp) => {
                self.move_selection_page(-1, page_size);
                true
            }
            Some(NavigationAction::PageDown) => {
                self.move_selection_page(1, page_size);
                true
            }
            Some(NavigationAction::Home) => {
                self.move_selection_home();
                true
            }
            Some(NavigationAction::End) => {
                self.move_selection_end();
                true
            }
            Some(NavigationAction::Up) => {
                self.move_selection(-1);
                true
            }
            Some(NavigationAction::Down) => {
                self.move_selection(1);
                true
            }
            _ => false,
        }
    }
}
