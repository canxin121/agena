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

#[cfg(test)]
mod tests {
    use super::{QuerySuggestionState, SuggestionPopupState};

    #[test]
    fn new_clamps_selected_to_last_item() {
        let state =
            SuggestionPopupState::new("q".to_string(), "fp".to_string(), vec!["a", "b"], 10, ());
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn move_selection_cycle_wraps_in_both_directions() {
        let mut state = SuggestionPopupState::new(
            "q".to_string(),
            "fp".to_string(),
            vec!["a", "b", "c"],
            0,
            (),
        );
        state.move_selection_cycle(-1);
        assert_eq!(state.selected, 2);

        state.move_selection_cycle(1);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn query_suggestion_state_clamps_to_last_item() {
        let mut state = QuerySuggestionState::new("typed".to_string(), 5, ());
        state.items = vec!["a", "b"];
        state.clamp_selection();

        assert_eq!(state.selected, 1);
        assert_eq!(state.selected_item(), Some(&"b"));
    }
}
