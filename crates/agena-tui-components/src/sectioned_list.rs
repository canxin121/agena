//! Sectioned list widget.

use crate::selection::{SelectionCursor, clamped_selected_index};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Focus of a sectioned list.
pub enum SectionedListFocus {
    Navigation,
    Items,
}

/// A section of a sectioned list.
pub trait SectionedListSection {
    type Item;

    fn items(&self) -> &[Self::Item];
}

#[derive(Debug, Clone)]
/// State of a sectioned list.
pub struct SectionedListState<TSection>
where
    TSection: SectionedListSection,
{
    sections: Vec<TSection>,
    section_selection: SelectionCursor,
    item_selection: SelectionCursor,
    focus: SectionedListFocus,
}

impl<TSection> SectionedListState<TSection>
where
    TSection: SectionedListSection,
{
    pub fn new(
        sections: Vec<TSection>,
        selected_section: usize,
        selected_item: usize,
        focus: SectionedListFocus,
    ) -> Self {
        let selected_section = clamped_selected_index(selected_section, sections.len());
        let item_count = sections
            .get(selected_section)
            .map(|section| section.items().len())
            .unwrap_or(0);
        let selected_item = clamped_selected_index(selected_item, item_count);
        Self {
            sections,
            section_selection: SelectionCursor::new(selected_section),
            item_selection: SelectionCursor::new(selected_item),
            focus,
        }
    }

    pub fn sections(&self) -> &[TSection] {
        self.sections.as_slice()
    }

    pub fn focus(&self) -> SectionedListFocus {
        self.focus
    }

    pub fn set_focus(&mut self, focus: SectionedListFocus) {
        self.focus = focus;
        self.clamp_selection();
    }

    pub fn selected_section_index(&self) -> usize {
        self.section_selection.selected
    }

    pub fn selected_item_index(&self) -> usize {
        self.item_selection.selected
    }

    pub fn selected_section(&self) -> Option<&TSection> {
        self.sections.get(self.selected_section_index())
    }

    pub fn selected_item(&self) -> Option<&TSection::Item> {
        self.selected_section()
            .and_then(|section| section.items().get(self.selected_item_index()))
    }

    pub fn set_indices(&mut self, selected_section: usize, selected_item: usize) {
        self.section_selection.selected = selected_section;
        self.item_selection.selected = selected_item;
        self.clamp_selection();
    }

    pub fn clamp_selection(&mut self) {
        self.section_selection.clamp(self.sections.len());
        self.item_selection
            .clamp(self.selected_section_item_count());
    }

    pub fn move_selection(&mut self, delta: isize) {
        match self.focus {
            SectionedListFocus::Navigation => {
                self.section_selection.move_by(self.sections.len(), delta);
                self.item_selection
                    .clamp(self.selected_section_item_count());
            }
            SectionedListFocus::Items => {
                self.item_selection
                    .move_by(self.selected_section_item_count(), delta);
            }
        }
    }

    pub fn move_selection_page(&mut self, delta: isize, page_size: usize) {
        match self.focus {
            SectionedListFocus::Navigation => {
                self.section_selection
                    .move_page(self.sections.len(), delta, page_size);
                self.item_selection
                    .clamp(self.selected_section_item_count());
            }
            SectionedListFocus::Items => {
                self.item_selection
                    .move_page(self.selected_section_item_count(), delta, page_size);
            }
        }
    }

    pub fn move_selection_home(&mut self) {
        match self.focus {
            SectionedListFocus::Navigation => {
                self.section_selection.move_home();
                self.item_selection
                    .clamp(self.selected_section_item_count());
            }
            SectionedListFocus::Items => self.item_selection.move_home(),
        }
    }

    pub fn move_selection_end(&mut self) {
        match self.focus {
            SectionedListFocus::Navigation => {
                self.section_selection.move_end(self.sections.len());
                self.item_selection
                    .clamp(self.selected_section_item_count());
            }
            SectionedListFocus::Items => self
                .item_selection
                .move_end(self.selected_section_item_count()),
        }
    }

    fn selected_section_item_count(&self) -> usize {
        self.selected_section()
            .map(|section| section.items().len())
            .unwrap_or(0)
    }
}
