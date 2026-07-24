//! Presentation policy for the terminal session-list scope selector.

use agena_tui::i18n::I18n;

/// User-selected scope for the terminal session list and picker presentation.
/// The application maps this presentation intent to its concrete query ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionViewMode {
    #[default]
    All,
    Roots,
    Subtree,
}

impl SessionViewMode {
    /// Advances through the terminal's visible session scopes.
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Roots,
            Self::Roots => Self::Subtree,
            Self::Subtree => Self::All,
        }
    }

    /// Localized label for the selected scope.
    pub fn label(self, i18n: &I18n, subtree_root_id: Option<i64>) -> String {
        match (self, subtree_root_id) {
            (Self::All, _) => i18n.text("session-view-all"),
            (Self::Roots, _) => i18n.text("session-view-roots"),
            (Self::Subtree, Some(root_id)) => i18n.text_args(
                "session-view-subtree-root",
                &agena_tui::fl_args!("id" => root_id),
            ),
            (Self::Subtree, None) => i18n.text("session-view-subtree"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionViewMode;
    use agena_tui::i18n::I18n;

    #[test]
    fn session_scope_cycles_through_all_visible_modes() {
        assert_eq!(SessionViewMode::All.next(), SessionViewMode::Roots);
        assert_eq!(SessionViewMode::Roots.next(), SessionViewMode::Subtree);
        assert_eq!(SessionViewMode::Subtree.next(), SessionViewMode::All);
    }

    #[test]
    fn subtree_label_retains_the_optional_root_identity() {
        let i18n = I18n::english();
        assert!(!SessionViewMode::Subtree.label(&i18n, None).is_empty());
        assert!(
            SessionViewMode::Subtree
                .label(&i18n, Some(42))
                .contains("42")
        );
    }
}
