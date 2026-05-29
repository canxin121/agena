use std::fmt;

const OPENAI_RESPONSES_CALL_ID_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ProviderStreamKey(String);

impl ProviderStreamKey {
    pub(crate) fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ModelToolCallId(String);

impl ModelToolCallId {
    pub(crate) fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ModelToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ProviderItemId(String);

impl ProviderItemId {
    pub(crate) fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| Self(trimmed.to_owned()))
    }
}

impl fmt::Display for ProviderItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub(crate) fn openai_responses_call_id(raw: &str) -> Option<ModelToolCallId> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= OPENAI_RESPONSES_CALL_ID_MAX_CHARS {
        return ModelToolCallId::new(trimmed);
    }

    let hash = blake3::hash(trimmed.as_bytes());
    let hex = hash.to_hex().to_string();
    ModelToolCallId::new(format!("call_{}", &hex[..32]))
}

pub(crate) fn valid_openai_responses_call_id(raw: &str) -> bool {
    let trimmed = raw.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= OPENAI_RESPONSES_CALL_ID_MAX_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_responses_call_id_preserves_valid_ids() {
        assert_eq!(
            openai_responses_call_id(" call_1 ").expect("id").as_str(),
            "call_1"
        );
    }

    #[test]
    fn openai_responses_call_id_hashes_oversized_ids() {
        let oversized = "x".repeat(412);
        let id = openai_responses_call_id(&oversized).expect("id");

        assert!(id.as_str().starts_with("call_"));
        assert!(id.as_str().chars().count() <= 64);
        assert_ne!(id.as_str(), oversized);
        assert_eq!(id, openai_responses_call_id(&oversized).expect("stable id"));
    }
}
