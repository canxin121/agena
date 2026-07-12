use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TerminalVersion {
    components: Vec<u64>,
}

impl TerminalVersion {
    pub(super) fn parse(value: &str) -> Option<Self> {
        let components = value
            .trim()
            .split('.')
            .map(|component| component.parse::<u64>())
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        (!components.is_empty()).then_some(Self { components })
    }
}

impl fmt::Display for TerminalVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self
            .components
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".");
        formatter.write_str(value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dotted_versions_without_guessing_vendor_encodings() {
        assert_eq!(
            TerminalVersion::parse("1.2.30").unwrap().to_string(),
            "1.2.30"
        );
        assert!(TerminalVersion::parse("240800").is_some());
        assert!(TerminalVersion::parse("nightly").is_none());
    }
}
