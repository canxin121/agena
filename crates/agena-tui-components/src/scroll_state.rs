use crossterm::event::KeyEvent;

use crate::{NavigationAction, navigation_action};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScrollState {
    pub scroll: u16,
}

impl ScrollState {
    pub fn new(scroll: u16) -> Self {
        Self { scroll }
    }

    pub fn clamp(&mut self, max_scroll: u16) {
        self.scroll = self.scroll.min(max_scroll);
    }

    pub fn move_by(&mut self, delta: i16, max_scroll: u16) {
        let next = i32::from(self.scroll) + i32::from(delta);
        self.scroll = next.clamp(0, i32::from(max_scroll)) as u16;
    }

    pub fn move_page(&mut self, delta: i16, page_size: u16, max_scroll: u16) {
        let step = delta.saturating_mul(page_size.max(1) as i16);
        self.move_by(step, max_scroll);
    }

    pub fn move_home(&mut self) {
        self.scroll = 0;
    }

    pub fn move_end(&mut self, max_scroll: u16) {
        self.scroll = max_scroll;
    }

    pub fn handle_navigation_key(
        &mut self,
        key: KeyEvent,
        max_scroll: u16,
        page_size: u16,
    ) -> bool {
        match navigation_action(key) {
            Some(NavigationAction::Up) => {
                self.move_by(-1, max_scroll);
                true
            }
            Some(NavigationAction::Down) => {
                self.move_by(1, max_scroll);
                true
            }
            Some(NavigationAction::PageUp) => {
                self.move_page(-1, page_size, max_scroll);
                true
            }
            Some(NavigationAction::PageDown) => {
                self.move_page(1, page_size, max_scroll);
                true
            }
            Some(NavigationAction::Home) => {
                self.move_home();
                true
            }
            Some(NavigationAction::End) => {
                self.move_end(max_scroll);
                true
            }
            _ => false,
        }
    }
}
