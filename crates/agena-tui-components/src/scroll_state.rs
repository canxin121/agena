use crossterm::event::{KeyCode, KeyEvent};

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
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_by(-1, max_scroll);
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_by(1, max_scroll);
                true
            }
            KeyCode::PageUp => {
                self.move_page(-1, page_size, max_scroll);
                true
            }
            KeyCode::PageDown => {
                self.move_page(1, page_size, max_scroll);
                true
            }
            KeyCode::Home => {
                self.move_home();
                true
            }
            KeyCode::End => {
                self.move_end(max_scroll);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::ScrollState;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn navigation_keys_follow_standard_scroll_bindings() {
        let mut state = ScrollState::new(5);

        assert!(state.handle_navigation_key(key(KeyCode::Char('k')), 20, 8));
        assert_eq!(state.scroll, 4);

        assert!(state.handle_navigation_key(key(KeyCode::PageDown), 20, 8));
        assert_eq!(state.scroll, 12);

        assert!(state.handle_navigation_key(key(KeyCode::End), 20, 8));
        assert_eq!(state.scroll, 20);

        assert!(state.handle_navigation_key(key(KeyCode::Home), 20, 8));
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn move_helpers_clamp_to_bounds() {
        let mut state = ScrollState::new(0);
        state.move_by(-10, 6);
        assert_eq!(state.scroll, 0);

        state.move_page(1, 20, 6);
        assert_eq!(state.scroll, 6);

        state.clamp(3);
        assert_eq!(state.scroll, 3);
    }
}
