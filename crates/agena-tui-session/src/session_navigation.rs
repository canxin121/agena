//! Display-only session-navigation picker presentation and semantic reducer.
//!
//! The App projects runtime sessions and messages into opaque rows, keeps the
//! key-to-concrete-effect map, and performs the actual session open or rewind
//! confirmation. This module owns the shared searchable navigation state used
//! by lineage, child-session, and rewind-message pickers.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
};

use crossterm::event::KeyEvent;
use ratatui::{Frame, layout::Rect};

use agena_tui_components::{
    Editor, SearchPicker, SearchPickerConfig, SearchPickerDialogSpec, SearchPickerInputResult,
    SearchPickerItem, SearchPickerNoCustom, render_search_picker_dialog,
};

use agena_tui::{i18n::I18n, sanitize_picker_text};

#[derive(Debug, Clone)]
/// A navigation item of a session.
pub struct SessionNavigationItem {
    pub key: String,
    pub label: String,
    pub detail: String,
    pub search_text: String,
}

impl SessionNavigationItem {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
        search_text: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            detail: detail.into(),
            search_text: search_text.into(),
        }
    }
}

impl SearchPickerItem for SessionNavigationItem {
    fn search_picker_key(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.key.as_str())
    }

    fn search_picker_label(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.label.as_str())
    }

    fn search_picker_detail(&self) -> Option<Cow<'_, str>> {
        (!self.detail.trim().is_empty()).then_some(Cow::Borrowed(self.detail.as_str()))
    }

    fn search_picker_search_text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.search_text.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Navigation mode of a session.
pub enum SessionNavigationMode {
    Open,
    Rewind,
}

/// API-independent input used to construct the lineage navigation tree.
/// The App projects `SessionResource` into this scalar-only shape, then keeps
/// localized row text and the concrete open-session effect at its boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionLineageNode {
    pub session_id: i64,
    pub parent_session_id: Option<i64>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Lineage relation of a session.
pub enum SessionLineageRelation {
    Ancestor,
    Current,
    Sibling,
    Child,
}

impl SessionLineageRelation {
    pub fn localization_key(self) -> &'static str {
        match self {
            Self::Ancestor => "session-tag-ancestor",
            Self::Current => "session-tag-current",
            Self::Sibling => "session-tag-sibling",
            Self::Child => "session-tag-child",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A lineage item of a session.
pub struct SessionLineageItem {
    pub session_id: i64,
    pub relation: SessionLineageRelation,
    pub depth: usize,
    pub is_leaf: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Summary of session lineage.
pub struct SessionLineageSummary {
    pub root_session_id: i64,
    pub depth: usize,
    pub side_branch_count: usize,
    pub descendant_count: usize,
}

pub fn build_lineage_items(
    nodes: &[SessionLineageNode],
    current_session_id: i64,
) -> Vec<SessionLineageItem> {
    let by_id = nodes
        .iter()
        .copied()
        .map(|node| (node.session_id, node))
        .collect::<BTreeMap<_, _>>();
    if !by_id.contains_key(&current_session_id) {
        return Vec::new();
    }
    let mut chain = Vec::new();
    let mut current = Some(current_session_id);
    let mut seen = HashSet::new();
    while let Some(session_id) = current {
        if !seen.insert(session_id) {
            break;
        }
        let Some(node) = by_id.get(&session_id) else {
            break;
        };
        chain.push(session_id);
        current = node
            .parent_session_id
            .filter(|parent| by_id.contains_key(parent));
    }
    chain.reverse();
    let Some(root_session_id) = chain.first().copied() else {
        return Vec::new();
    };
    let chain_ids = chain.into_iter().collect::<HashSet<_>>();
    let mut children = BTreeMap::<Option<i64>, Vec<i64>>::new();
    for node in nodes {
        children
            .entry(
                node.parent_session_id
                    .filter(|parent| by_id.contains_key(parent)),
            )
            .or_default()
            .push(node.session_id);
    }
    for child_ids in children.values_mut() {
        child_ids.sort_by(|left, right| {
            chain_ids
                .contains(right)
                .cmp(&chain_ids.contains(left))
                .then_with(|| {
                    by_id[right]
                        .updated_at_ms
                        .cmp(&by_id[left].updated_at_ms)
                        .then_with(|| right.cmp(left))
                })
        });
    }
    let mut items = Vec::new();
    append_lineage_items(
        root_session_id,
        0,
        false,
        current_session_id,
        &chain_ids,
        &children,
        &mut HashSet::new(),
        &mut items,
    );
    items
}

fn append_lineage_items(
    session_id: i64,
    depth: usize,
    under_current: bool,
    current_session_id: i64,
    chain_ids: &HashSet<i64>,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
    visited: &mut HashSet<i64>,
    items: &mut Vec<SessionLineageItem>,
) {
    if !visited.insert(session_id) {
        return;
    }
    let child_ids = children.get(&Some(session_id)).cloned().unwrap_or_default();
    let relation = if session_id == current_session_id {
        SessionLineageRelation::Current
    } else if chain_ids.contains(&session_id) {
        SessionLineageRelation::Ancestor
    } else if under_current {
        SessionLineageRelation::Child
    } else {
        SessionLineageRelation::Sibling
    };
    items.push(SessionLineageItem {
        session_id,
        relation,
        depth,
        is_leaf: child_ids.is_empty(),
    });
    for child_id in child_ids {
        append_lineage_items(
            child_id,
            depth.saturating_add(1),
            under_current || session_id == current_session_id,
            current_session_id,
            chain_ids,
            children,
            visited,
            items,
        );
    }
}

pub fn summarize_lineage_items(items: &[SessionLineageItem]) -> Option<SessionLineageSummary> {
    let root_session_id = items.first()?.session_id;
    let current = items
        .iter()
        .find(|item| item.relation == SessionLineageRelation::Current)?;
    Some(SessionLineageSummary {
        root_session_id,
        depth: current.depth,
        side_branch_count: items
            .iter()
            .filter(|item| item.relation == SessionLineageRelation::Sibling)
            .count(),
        descendant_count: items
            .iter()
            .filter(|item| item.relation == SessionLineageRelation::Child)
            .count(),
    })
}

pub type SessionNavigationPresentation =
    SearchPicker<SessionNavigationItem, SearchPickerNoCustom, SessionNavigationMode, Editor>;

pub fn new_presentation(
    title: String,
    prompt: String,
    footer: String,
    empty_message: String,
    mode: SessionNavigationMode,
) -> SessionNavigationPresentation {
    SessionNavigationPresentation::new(
        title,
        prompt,
        footer,
        empty_message,
        Editor::default(),
        SearchPickerConfig::searchable(),
        None,
        mode,
    )
}

#[derive(Debug, Clone)]
/// Action of session navigation.
pub enum SessionNavigationAction {
    Accept,
    Input(KeyEvent),
    Paste(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Effect of session navigation.
pub enum SessionNavigationEffect {
    Close,
    Open { key: String },
    Rewind { key: String },
    KeepOpen,
}

pub fn reduce(
    presentation: &mut SessionNavigationPresentation,
    action: SessionNavigationAction,
) -> SessionNavigationEffect {
    match action {
        SessionNavigationAction::Accept => {
            let Some(item) = presentation.selected_item() else {
                return SessionNavigationEffect::KeepOpen;
            };
            match presentation.meta {
                SessionNavigationMode::Open => SessionNavigationEffect::Open {
                    key: item.key.clone(),
                },
                SessionNavigationMode::Rewind => SessionNavigationEffect::Rewind {
                    key: item.key.clone(),
                },
            }
        }
        SessionNavigationAction::Input(key) => match presentation.handle_input_key(key) {
            SearchPickerInputResult::Close => SessionNavigationEffect::Close,
            SearchPickerInputResult::Navigated | SearchPickerInputResult::Edited { .. } => {
                SessionNavigationEffect::KeepOpen
            }
        },
        SessionNavigationAction::Paste(text) => {
            presentation.input.insert_str(text.as_str());
            presentation.refresh_results();
            SessionNavigationEffect::KeepOpen
        }
    }
}

/// Renders session navigation from TUI-owned rows/state. The App maps an
/// opaque selection to opening or rewinding a concrete session/message.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &SessionNavigationPresentation,
    i18n: &I18n,
) {
    let spec = SearchPickerDialogSpec::new(
        i18n.text("overlay-picker-loading").into(),
        i18n.text("overlay-attach-matches").into(),
    );
    render_search_picker_dialog(frame, area, dialog, &spec, sanitize_picker_text);
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{
        SessionLineageNode, SessionLineageRelation, SessionNavigationAction,
        SessionNavigationEffect, SessionNavigationItem, SessionNavigationMode, build_lineage_items,
        new_presentation, reduce, summarize_lineage_items,
    };

    #[test]
    fn open_mode_returns_only_the_opaque_row_key() {
        let mut presentation = new_presentation(
            "Lineage".to_owned(),
            "Search".to_owned(),
            "Footer".to_owned(),
            "Empty".to_owned(),
            SessionNavigationMode::Open,
        );
        presentation.replace_items(vec![SessionNavigationItem::new(
            "session:42",
            "Session",
            "#42",
            "Session #42",
        )]);

        assert_eq!(
            reduce(&mut presentation, SessionNavigationAction::Accept),
            SessionNavigationEffect::Open {
                key: "session:42".to_owned(),
            }
        );
    }

    #[test]
    fn rewind_mode_preserves_the_semantic_effect() {
        let mut presentation = new_presentation(
            "Rewind".to_owned(),
            "Search".to_owned(),
            "Footer".to_owned(),
            "Empty".to_owned(),
            SessionNavigationMode::Rewind,
        );
        presentation.replace_items(vec![SessionNavigationItem::new(
            "message:9",
            "Message",
            "#9",
            "Message #9",
        )]);

        assert_eq!(
            reduce(&mut presentation, SessionNavigationAction::Accept),
            SessionNavigationEffect::Rewind {
                key: "message:9".to_owned(),
            }
        );
    }

    #[test]
    fn escape_closes_the_presentation() {
        let mut presentation = new_presentation(
            "Lineage".to_owned(),
            "Search".to_owned(),
            "Footer".to_owned(),
            "Empty".to_owned(),
            SessionNavigationMode::Open,
        );

        assert_eq!(
            reduce(
                &mut presentation,
                SessionNavigationAction::Input(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            ),
            SessionNavigationEffect::Close
        );
    }

    #[test]
    fn lineage_tree_keeps_current_chain_before_recent_siblings_and_summarizes_branches() {
        let items = build_lineage_items(
            &[
                SessionLineageNode {
                    session_id: 1,
                    parent_session_id: None,
                    updated_at_ms: 1,
                },
                SessionLineageNode {
                    session_id: 2,
                    parent_session_id: Some(1),
                    updated_at_ms: 2,
                },
                SessionLineageNode {
                    session_id: 3,
                    parent_session_id: Some(1),
                    updated_at_ms: 99,
                },
                SessionLineageNode {
                    session_id: 4,
                    parent_session_id: Some(2),
                    updated_at_ms: 4,
                },
                SessionLineageNode {
                    session_id: 5,
                    parent_session_id: Some(4),
                    updated_at_ms: 5,
                },
            ],
            4,
        );
        assert_eq!(
            items.iter().map(|item| item.session_id).collect::<Vec<_>>(),
            vec![1, 2, 4, 5, 3],
        );
        assert_eq!(items[0].relation, SessionLineageRelation::Ancestor);
        assert_eq!(items[2].relation, SessionLineageRelation::Current);
        assert_eq!(items[3].relation, SessionLineageRelation::Child);
        assert_eq!(items[4].relation, SessionLineageRelation::Sibling);
        assert!(items[3].is_leaf);
        assert_eq!(
            summarize_lineage_items(&items),
            Some(super::SessionLineageSummary {
                root_session_id: 1,
                depth: 2,
                side_branch_count: 1,
                descendant_count: 1,
            }),
        );
    }
}
