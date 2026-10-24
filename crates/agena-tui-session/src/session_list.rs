//! Presentation state for the main session list.
//!
//! The application maps Runtime/API session records into these compact rows,
//! executes reload/open effects, and owns request lifecycle state. This module
//! owns the visible hierarchy, local search, selection, and navigation policy.

use std::collections::{BTreeMap, HashSet};

use agena_tui_components::SelectableListState;

use crate::session_view::SessionViewMode;

/// API-independent display projection of one session list row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListItem {
    pub session_id: i64,
    pub parent_session_id: Option<i64>,
    pub title: String,
    pub favorite: bool,
    /// UTC milliseconds, used only for the stable newest-first presentation
    /// order. It deliberately avoids exposing a transport timestamp type.
    pub updated_at_millis: i64,
}

impl SessionListItem {
    pub fn matches_query(&self, query: &str) -> bool {
        let query = query.to_ascii_lowercase();
        self.title.to_ascii_lowercase().contains(query.as_str())
            || self.session_id.to_string().contains(query.as_str())
    }
}

/// Semantic input accepted by the session-list reducer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionListAction {
    SetViewMode(SessionViewMode),
    CycleViewMode,
    SetSearchQuery(String),
    MoveSelection(isize),
    MoveSelectionHome,
    MoveSelectionEnd,
    OpenSelected,
}

/// Intent emitted by the presentation reducer for the application adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionListEffect {
    None,
    Reload,
    OpenSession { session_id: i64, title: String },
}

/// Read-only projection consumed by a renderer.
#[derive(Debug, Clone, Copy)]
pub struct SessionListView<'a> {
    pub items: &'a [SessionListItem],
    pub selected_index: usize,
    pub view_mode: SessionViewMode,
    pub subtree_root_id: Option<i64>,
}

/// Complete display/navigation state for the session-list feature.
#[derive(Debug, Clone, Default)]
pub struct SessionListPresentation {
    source_items: Vec<SessionListItem>,
    list: SelectableListState<SessionListItem>,
    search_query: String,
    view_mode: SessionViewMode,
    subtree_root_id: Option<i64>,
}

impl SessionListPresentation {
    pub fn new(initial_search_query: impl Into<String>) -> Self {
        Self {
            search_query: initial_search_query.into(),
            ..Self::default()
        }
    }

    pub fn view(&self) -> SessionListView<'_> {
        SessionListView {
            items: self.list.items.as_slice(),
            selected_index: self.list.selected,
            view_mode: self.view_mode,
            subtree_root_id: self.subtree_root_id,
        }
    }

    pub fn view_mode(&self) -> SessionViewMode {
        self.view_mode
    }

    pub fn subtree_root_id(&self) -> Option<i64> {
        self.subtree_root_id
    }

    pub fn current_selected(&self) -> Option<&SessionListItem> {
        self.list.selected_item()
    }

    pub fn current_selected_id(&self) -> Option<i64> {
        self.current_selected().map(|item| item.session_id)
    }

    pub fn select_by_id(&mut self, session_id: i64) -> bool {
        let Some(index) = self
            .list
            .items
            .iter()
            .position(|item| item.session_id == session_id)
        else {
            return false;
        };
        self.list.selected = index;
        true
    }

    /// Drops the current row selection so no session is highlighted.
    ///
    /// Used when the TUI starts with no session at all: without this, the
    /// newest row would be highlighted by default and the next submit would
    /// silently target that session instead of creating a fresh one.
    pub fn clear_selection(&mut self) {
        self.list.selected = self.list.items.len();
    }

    /// Replaces the application-provided row projection while retaining a
    /// useful selected session whenever it remains visible.
    pub fn replace_items(
        &mut self,
        items: Vec<SessionListItem>,
        subtree_root_id: Option<i64>,
        preferred_session_id: Option<i64>,
    ) {
        let selected_id = preferred_session_id.or_else(|| self.current_selected_id());
        self.source_items = items;
        self.subtree_root_id = subtree_root_id;
        self.rebuild_visible_items();
        if let Some(session_id) = selected_id {
            let _ = self.select_by_id(session_id);
        }
        self.list.clamp_selection();
    }

    /// Replaces an application-provided row while preserving filtering,
    /// ordering, and selection in the presentation owner.
    pub fn replace_item(&mut self, item: SessionListItem) {
        let selected_id = self.current_selected_id();
        if let Some(existing) = self
            .source_items
            .iter_mut()
            .find(|existing| existing.session_id == item.session_id)
        {
            *existing = item;
            self.rebuild_visible_items();
            if let Some(session_id) = selected_id {
                let _ = self.select_by_id(session_id);
            }
        }
    }

    /// Updates display-only state and returns the resulting application intent.
    pub fn update(&mut self, action: SessionListAction) -> SessionListEffect {
        match action {
            SessionListAction::SetViewMode(view_mode) => {
                if self.view_mode == view_mode {
                    return SessionListEffect::None;
                }
                self.view_mode = view_mode;
                self.rebuild_visible_items();
                SessionListEffect::Reload
            }
            SessionListAction::CycleViewMode => {
                self.view_mode = self.view_mode.next();
                self.rebuild_visible_items();
                SessionListEffect::Reload
            }
            SessionListAction::SetSearchQuery(query) => {
                self.search_query = query;
                self.rebuild_visible_items();
                SessionListEffect::None
            }
            SessionListAction::MoveSelection(delta) => {
                self.list.move_selection(delta);
                SessionListEffect::None
            }
            SessionListAction::MoveSelectionHome => {
                self.list.move_selection_home();
                SessionListEffect::None
            }
            SessionListAction::MoveSelectionEnd => {
                if !self.list.items.is_empty() {
                    self.list.move_selection_end();
                }
                SessionListEffect::None
            }
            SessionListAction::OpenSelected => self
                .current_selected()
                .map(|item| SessionListEffect::OpenSession {
                    session_id: item.session_id,
                    title: item.title.clone(),
                })
                .unwrap_or(SessionListEffect::None),
        }
    }

    pub fn set_search_query(&mut self, query: impl Into<String>) {
        let _ = self.update(SessionListAction::SetSearchQuery(query.into()));
    }

    fn rebuild_visible_items(&mut self) {
        let selected_id = self.current_selected_id();
        self.list.items = build_visible_session_items(
            self.source_items.as_slice(),
            self.view_mode,
            self.search_query.as_str(),
        );
        self.list.clamp_selection();
        if let Some(session_id) = selected_id {
            let _ = self.select_by_id(session_id);
        }
    }
}

fn build_visible_session_items(
    items: &[SessionListItem],
    mode: SessionViewMode,
    query: &str,
) -> Vec<SessionListItem> {
    let trimmed_query = query.trim();
    match mode {
        SessionViewMode::Roots => {
            let mut roots = items
                .iter()
                .filter(|session| session.parent_session_id.is_none())
                .cloned()
                .collect::<Vec<_>>();
            roots.sort_by(session_sort_recent);
            if !trimmed_query.is_empty() {
                roots.retain(|session| session.matches_query(trimmed_query));
            }
            roots
        }
        SessionViewMode::All | SessionViewMode::Subtree => {
            let by_id = items
                .iter()
                .cloned()
                .map(|session| (session.session_id, session))
                .collect::<BTreeMap<_, _>>();
            let mut children = BTreeMap::<Option<i64>, Vec<i64>>::new();
            for session in items {
                let parent_id = session
                    .parent_session_id
                    .filter(|parent_id| by_id.contains_key(parent_id));
                children
                    .entry(parent_id)
                    .or_default()
                    .push(session.session_id);
            }
            for child_ids in children.values_mut() {
                child_ids.sort_by(|left, right| session_sort_recent(&by_id[left], &by_id[right]));
            }

            let kept_ids = if trimmed_query.is_empty() {
                by_id.keys().copied().collect::<HashSet<_>>()
            } else {
                let mut kept = HashSet::new();
                for session in items
                    .iter()
                    .filter(|session| session.matches_query(trimmed_query))
                {
                    let mut current = Some(session.session_id);
                    while let Some(session_id) = current {
                        if !kept.insert(session_id) {
                            break;
                        }
                        current = by_id
                            .get(&session_id)
                            .and_then(|item| item.parent_session_id);
                    }
                }
                kept
            };

            let mut out = Vec::new();
            for root_id in children.get(&None).cloned().unwrap_or_default() {
                append_session_subtree(root_id, &children, &by_id, &kept_ids, &mut out);
            }
            out
        }
    }
}

fn append_session_subtree(
    session_id: i64,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
    by_id: &BTreeMap<i64, SessionListItem>,
    kept_ids: &HashSet<i64>,
    out: &mut Vec<SessionListItem>,
) {
    if !kept_ids.contains(&session_id) {
        return;
    }
    if let Some(session) = by_id.get(&session_id) {
        out.push(session.clone());
    }
    if let Some(child_ids) = children.get(&Some(session_id)) {
        for child_id in child_ids {
            append_session_subtree(*child_id, children, by_id, kept_ids, out);
        }
    }
}

fn session_sort_recent(left: &SessionListItem, right: &SessionListItem) -> std::cmp::Ordering {
    right
        .updated_at_millis
        .cmp(&left.updated_at_millis)
        .then_with(|| right.session_id.cmp(&left.session_id))
}
