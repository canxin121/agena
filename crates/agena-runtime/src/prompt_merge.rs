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

#[cfg(test)]
mod tests {
    use super::merge_system_prompts;

    #[test]
    fn keeps_an_already_prefixed_secondary_prompt_once() {
        assert_eq!(
            merge_system_prompts(Some("base"), Some("base\n\nextra")),
            Some("base\n\nextra".to_string())
        );
    }

    #[test]
    fn joins_distinct_nonempty_prompts() {
        assert_eq!(
            merge_system_prompts(Some("base"), Some("extra")),
            Some("base\n\nextra".to_string())
        );
    }
}
