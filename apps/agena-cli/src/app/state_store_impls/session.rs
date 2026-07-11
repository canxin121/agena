impl SessionViewMode {
    pub(in crate::app) fn next(self) -> Self {
        match self {
            Self::All => Self::Roots,
            Self::Roots => Self::Subtree,
            Self::Subtree => Self::All,
        }
    }

    pub(in crate::app) fn label(self, i18n: &I18n, subtree_root_id: Option<i64>) -> String {
        match (self, subtree_root_id) {
            (Self::All, _) => ui_text::t(i18n, "session-view-all"),
            (Self::Roots, _) => ui_text::t(i18n, "session-view-roots"),
            (Self::Subtree, Some(root_id)) => i18n.text_args(
                "session-view-subtree-root",
                &crate::fl_args!("id" => root_id),
            ),
            (Self::Subtree, None) => ui_text::t(i18n, "session-view-subtree"),
        }
    }
}

impl SessionListState {
    pub(in crate::app) fn current_selected(&self) -> Option<&SessionResource> {
        self.list.selected_item()
    }

    pub(in crate::app) fn current_selected_id(&self) -> Option<i64> {
        self.current_selected().map(|item| item.id)
    }

    pub(in crate::app) fn clamp_selection(&mut self) {
        self.list.clamp_selection();
    }

    pub(in crate::app) fn move_selection(&mut self, delta: isize) {
        self.list.move_selection(delta);
    }

    pub(in crate::app) fn should_load_more(&self) -> bool {
        false
    }

    pub(in crate::app) fn select_by_id(&mut self, session_id: i64) -> bool {
        if let Some(index) = self
            .list
            .items
            .iter()
            .position(|item| item.id == session_id)
        {
            self.list.selected = index;
            true
        } else {
            false
        }
    }
}
use crate::app::{I18n, SessionListState, SessionResource, SessionViewMode, ui_text};
