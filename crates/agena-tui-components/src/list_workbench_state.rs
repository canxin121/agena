use crate::SelectableListState;

#[derive(Debug, Clone)]
pub struct ListWorkbenchState<TItem, TEditor> {
    pub title: String,
    pub footer: String,
    pub list: SelectableListState<TItem>,
    pub editor: Option<TEditor>,
}

impl<TItem, TEditor> ListWorkbenchState<TItem, TEditor> {
    pub fn new(title: String, footer: String, list: SelectableListState<TItem>) -> Self {
        Self {
            title,
            footer,
            list,
            editor: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ListWorkbenchState;
    use crate::SelectableListState;

    #[test]
    fn constructor_preserves_workbench_fields() {
        let state = ListWorkbenchState::<u8, u16>::new(
            "Title".to_string(),
            "Footer".to_string(),
            SelectableListState::new(vec![1, 2, 3], 1),
        );

        assert_eq!(state.title, "Title");
        assert_eq!(state.footer, "Footer");
        assert_eq!(state.list.items, vec![1, 2, 3]);
        assert_eq!(state.list.selected, 1);
        assert!(state.editor.is_none());
    }
}
