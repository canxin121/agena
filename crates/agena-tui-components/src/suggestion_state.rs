#[derive(Debug, Clone)]
pub struct SuggestionPopupState<TItem, TMeta> {
    pub query: String,
    pub fingerprint: String,
    pub items: Vec<TItem>,
    pub selected: usize,
    pub meta: TMeta,
}

impl<TItem, TMeta> SuggestionPopupState<TItem, TMeta> {
    pub fn new(
        query: String,
        fingerprint: String,
        items: Vec<TItem>,
        selected: usize,
        meta: TMeta,
    ) -> Self {
        let mut state = Self {
            query,
            fingerprint,
            items,
            selected,
            meta,
        };
        state.clamp_selection();
        state
    }

    pub fn selected_item(&self) -> Option<&TItem> {
        self.items.get(self.selected)
    }

    pub fn clamp_selection(&mut self) {
        if self.items.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
        }
    }

    pub fn move_selection_cycle(&mut self, delta: isize) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.items.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }
}

#[derive(Debug, Clone)]
pub struct QuerySuggestionState<TItem, TMeta, TInput> {
    pub query: TInput,
    pub items: Vec<TItem>,
    pub selected: usize,
    pub meta: TMeta,
}

impl<TItem, TMeta, TInput> QuerySuggestionState<TItem, TMeta, TInput> {
    pub fn new(query: TInput, selected: usize, meta: TMeta) -> Self {
        Self {
            query,
            items: Vec::new(),
            selected,
            meta,
        }
    }

    pub fn selected_item(&self) -> Option<&TItem> {
        self.items.get(self.selected)
    }

    pub fn clamp_selection(&mut self) {
        if self.items.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
        }
    }
}
