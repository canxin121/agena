/// Combine a primary and secondary system prompt without duplicating a primary
/// prefix the secondary prompt already includes.
pub fn merge_system_prompts(primary: Option<&str>, secondary: Option<&str>) -> Option<String> {
    match (
        primary.map(str::trim).filter(|value| !value.is_empty()),
        secondary.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(primary), Some(secondary))
            if secondary == primary
                || secondary
                    .strip_prefix(primary)
                    .is_some_and(|suffix| suffix.starts_with("\n\n")) =>
        {
            Some(secondary.to_string())
        }
        (Some(primary), Some(secondary)) => Some(format!("{primary}\n\n{secondary}")),
        (Some(primary), None) => Some(primary.to_string()),
        (None, Some(secondary)) => Some(secondary.to_string()),
        (None, None) => None,
    }
}
