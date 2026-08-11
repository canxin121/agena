//! Inspector-row presentation helper shared by the settings studios.

/// A row in the inspector view.
#[derive(Debug, Clone)]
pub struct InspectorRow {
    pub label: String,
    pub detail: String,
}

/// Joins an optional mode display name and description into a single
/// human-readable inspector detail line.
pub(crate) fn summarize_named_mode(
    display_name: Option<&str>,
    description: Option<&str>,
) -> String {
    match (
        display_name
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        description.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(display_name), Some(description)) => format!("{display_name} · {description}"),
        (Some(display_name), None) => display_name.to_owned(),
        (None, Some(description)) => description.to_owned(),
        (None, None) => "configured mode".to_owned(),
    }
}
