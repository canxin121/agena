//! Presentation identity and pane-navigation policy for the terminal's main surface.

/// The focused main-surface pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Transcript,
    Composer,
}

impl Focus {
    /// Stable process-facing label for host-provided status-line interpolation.
    pub fn label(self) -> &'static str {
        match self {
            Self::Transcript => "transcript",
            Self::Composer => "composer",
        }
    }

    /// Cycle between the two visible main panes.
    pub fn move_pane(self, delta: isize) -> Self {
        let visible_panes = [Self::Transcript, Self::Composer];
        let index = visible_panes
            .iter()
            .position(|pane| *pane == self)
            .expect("main-surface focus must be in the pane cycle");
        let next = (index as isize + delta).rem_euclid(visible_panes.len() as isize) as usize;
        visible_panes[next]
    }
}

#[cfg(test)]
mod tests {
    use super::Focus;

    #[test]
    fn main_surface_focus_cycles_only_visible_panes() {
        assert_eq!(Focus::Transcript.move_pane(1), Focus::Composer);
        assert_eq!(Focus::Composer.move_pane(1), Focus::Transcript);
        assert_eq!(Focus::Transcript.move_pane(-1), Focus::Composer);
        assert_eq!(Focus::Transcript.move_pane(2), Focus::Transcript);
        assert_eq!(Focus::Composer.move_pane(-2), Focus::Composer);
    }
}
