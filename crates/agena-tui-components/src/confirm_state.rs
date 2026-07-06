#[derive(Debug, Clone)]
pub struct ConfirmDialogState<TAction> {
    pub title: String,
    pub body_lines: Vec<String>,
    pub footer: String,
    pub action: TAction,
}

impl<TAction> ConfirmDialogState<TAction> {
    pub fn new(title: String, body_lines: Vec<String>, footer: String, action: TAction) -> Self {
        Self {
            title,
            body_lines,
            footer,
            action,
        }
    }
}
