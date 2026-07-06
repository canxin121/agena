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
