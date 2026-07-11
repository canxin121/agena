use crossterm::event::KeyEvent;

use crate::{NavigationAction, navigation_action};

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
        match navigation_action(key) {
            Some(NavigationAction::PageUp) => {
                self.move_page(item_count, -1, page_size);
                true
            }
            Some(NavigationAction::PageDown) => {
                self.move_page(item_count, 1, page_size);
                true
            }
            Some(NavigationAction::Home) => {
                self.move_home();
                true
            }
            Some(NavigationAction::End) => {
                self.move_end(item_count);
                true
            }
            Some(NavigationAction::Up) => {
                self.move_by(item_count, -1);
                true
            }
            Some(NavigationAction::Down) => {
                self.move_by(item_count, 1);
                true
            }
            _ => false,
        }
    }
}

pub fn clamp_selected_index(selected: &mut usize, item_count: usize) {
    *selected = clamped_selected_index(*selected, item_count);
}

pub fn clamped_selected_index(selected: usize, item_count: usize) -> usize {
    if item_count == 0 {
        0
    } else {
        selected.min(item_count.saturating_sub(1))
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
