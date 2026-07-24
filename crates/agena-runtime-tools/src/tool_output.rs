/// Truncate a model-facing tool-output string without splitting UTF-8 scalar
/// values, and append a stable explanation when truncation occurred.
pub fn truncate_tool_output_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let truncated = value.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n\n[truncated to {max_chars} chars]")
}

#[cfg(test)]
mod tests {
    use super::truncate_tool_output_text;

    #[test]
    fn truncates_by_characters_and_preserves_unicode_boundaries() {
        assert_eq!(
            truncate_tool_output_text("ab界cd", 3),
            "ab界\n\n[truncated to 3 chars]"
        );
    }

    #[test]
    fn leaves_short_values_unchanged() {
        assert_eq!(truncate_tool_output_text("界", 1), "界");
    }
}
