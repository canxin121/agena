pub(crate) fn normalize_non_empty(value: impl AsRef<str>) -> Option<String> {
    let trimmed = value.as_ref().trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::normalize_non_empty;

    #[test]
    fn trims_and_discards_empty_text() {
        assert_eq!(normalize_non_empty("  value  "), Some("value".to_owned()));
        assert_eq!(normalize_non_empty(" \t "), None);
    }
}
