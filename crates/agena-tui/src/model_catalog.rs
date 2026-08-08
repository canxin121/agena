//! Display state and reducer for the model-catalog workbench.
//!
//! The application supplies catalog rows and performs the concrete query and
//! refresh effects. This module owns display-only selection, query/page state,
//! and the keyboard-to-effect policy.

use crossterm::event::KeyEvent;

use agena_tui_components::{
    DetailTextLine, DetailTextSpec, SelectableListState, build_detail_text,
};
use ratatui::text::Text;

use crate::keymap::{KeyAction, KeyContext, resolve};

/// An opaque model-catalog row prepared by the application boundary.
///
/// The TUI intentionally stores only display data and an opaque stable key.
/// API catalog resources remain on the application side of that boundary.
#[derive(Debug, Clone)]
pub struct ModelCatalogItem {
    pub key: String,
    pub label: String,
    pub subtitle: String,
    pub detail: ModelCatalogDetail,
}

/// Read-only, localized detail content for a selected catalog row.
#[derive(Debug, Clone, Default)]
pub struct ModelCatalogDetail {
    pub lines: Vec<DetailTextLine<'static>>,
}

impl ModelCatalogDetail {
    pub fn text(&self) -> Text<'static> {
        build_detail_text(self.lines.iter().cloned(), &DetailTextSpec::label_width(14))
    }
}

#[derive(Debug, Clone)]
/// Presentation state of the model catalog.
pub struct ModelCatalogPresentation {
    pub title: String,
    pub footer: String,
    query: String,
    total: usize,
    offset: usize,
    limit: usize,
    loading: bool,
    list: SelectableListState<ModelCatalogItem>,
}

impl ModelCatalogPresentation {
    pub fn new(title: impl Into<String>, footer: impl Into<String>, limit: usize) -> Self {
        Self {
            title: title.into(),
            footer: footer.into(),
            query: String::new(),
            total: 0,
            offset: 0,
            limit: limit.max(1),
            loading: true,
            list: SelectableListState::new(Vec::new(), 0),
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn total(&self) -> usize {
        self.total
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn loading(&self) -> bool {
        self.loading
    }

    pub fn list(&self) -> &SelectableListState<ModelCatalogItem> {
        &self.list
    }

    pub fn selected_item(&self) -> Option<&ModelCatalogItem> {
        self.list.selected_item()
    }

    pub fn begin_query(&mut self, query: impl Into<String>) -> ModelCatalogEffect {
        self.query = query.into();
        self.offset = 0;
        self.list.selected = 0;
        self.loading = true;
        ModelCatalogEffect::LoadPage {
            query: self.query.clone(),
            offset: 0,
        }
    }

    pub fn begin_refresh(&mut self) -> ModelCatalogEffect {
        self.loading = true;
        ModelCatalogEffect::Refresh
    }

    pub fn apply_page(
        &mut self,
        items: Vec<ModelCatalogItem>,
        total: usize,
        offset: usize,
        limit: usize,
    ) {
        self.list.items = items;
        self.total = total;
        self.offset = offset;
        self.limit = limit.max(1);
        self.loading = false;
        self.list.clamp_selection();
    }

    pub fn reject_page(&mut self) {
        self.loading = false;
    }

    pub fn request_first_page_after_refresh(&mut self) -> ModelCatalogEffect {
        self.offset = 0;
        self.list.selected = 0;
        self.loading = true;
        ModelCatalogEffect::LoadPage {
            query: self.query.clone(),
            offset: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Effect produced by the model catalog.
pub enum ModelCatalogEffect {
    KeepOpen,
    Close,
    OpenSearch,
    Refresh,
    LoadPage { query: String, offset: usize },
}

pub fn handle_key(
    presentation: &mut ModelCatalogPresentation,
    key: KeyEvent,
) -> ModelCatalogEffect {
    match resolve(KeyContext::ModelCatalog, key) {
        Some(KeyAction::Close) => ModelCatalogEffect::Close,
        Some(KeyAction::ModelCatalogSearch) => ModelCatalogEffect::OpenSearch,
        Some(KeyAction::Refresh) => presentation.begin_refresh(),
        Some(KeyAction::PageUp) if presentation.offset > 0 => {
            let offset = presentation.offset.saturating_sub(presentation.limit);
            presentation.offset = offset;
            presentation.list.selected = 0;
            presentation.loading = true;
            ModelCatalogEffect::LoadPage {
                query: presentation.query.clone(),
                offset,
            }
        }
        Some(KeyAction::PageDown)
            if presentation.offset + presentation.list.items.len() < presentation.total =>
        {
            let offset = presentation.offset.saturating_add(presentation.limit);
            presentation.offset = offset;
            presentation.list.selected = 0;
            presentation.loading = true;
            ModelCatalogEffect::LoadPage {
                query: presentation.query.clone(),
                offset,
            }
        }
        Some(KeyAction::MoveUp) => {
            presentation.list.move_selection(-1);
            ModelCatalogEffect::KeepOpen
        }
        Some(KeyAction::MoveDown) => {
            presentation.list.move_selection(1);
            ModelCatalogEffect::KeepOpen
        }
        _ => ModelCatalogEffect::KeepOpen,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ModelCatalogDetail, ModelCatalogEffect, ModelCatalogItem, ModelCatalogPresentation,
        handle_key,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn presentation() -> ModelCatalogPresentation {
        let mut presentation = ModelCatalogPresentation::new("Catalog", "footer", 2);
        presentation.apply_page(
            vec![
                ModelCatalogItem {
                    key: "one".to_owned(),
                    label: "one".to_owned(),
                    subtitle: String::new(),
                    detail: ModelCatalogDetail::default(),
                },
                ModelCatalogItem {
                    key: "two".to_owned(),
                    label: "two".to_owned(),
                    subtitle: String::new(),
                    detail: ModelCatalogDetail::default(),
                },
            ],
            4,
            0,
            2,
        );
        presentation
    }

    #[test]
    fn next_page_is_a_typed_load_effect() {
        let mut presentation = presentation();
        assert_eq!(
            handle_key(
                &mut presentation,
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            ),
            ModelCatalogEffect::LoadPage {
                query: String::new(),
                offset: 2,
            },
        );
        assert_eq!(presentation.offset(), 2);
        assert!(presentation.loading());
    }

    #[test]
    fn query_reset_discards_old_page_selection() {
        let mut presentation = presentation();
        presentation.list.move_selection(1);
        assert_eq!(
            presentation.begin_query("gpt"),
            ModelCatalogEffect::LoadPage {
                query: "gpt".to_owned(),
                offset: 0,
            },
        );
        assert_eq!(presentation.offset(), 0);
        assert_eq!(presentation.list().selected, 0);
        assert!(presentation.loading());
    }
}
