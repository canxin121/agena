use std::{fmt, str::FromStr};

const OPENAI_RESPONSES_CALL_ID_MAX_CHARS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderStreamKey(String);

impl AsRef<str> for ProviderStreamKey {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for ProviderStreamKey {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        (!trimmed.is_empty())
            .then(|| Self(trimmed.to_owned()))
            .ok_or(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelToolCallId(String);

impl AsRef<str> for ModelToolCallId {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl From<ModelToolCallId> for String {
    fn from(value: ModelToolCallId) -> Self {
        value.0
    }
}

impl FromStr for ModelToolCallId {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        (!trimmed.is_empty())
            .then(|| Self(trimmed.to_owned()))
            .ok_or(())
    }
}

impl fmt::Display for ModelToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderItemId(String);

impl AsRef<str> for ProviderItemId {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl FromStr for ProviderItemId {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let trimmed = value.trim();
        (!trimmed.is_empty())
            .then(|| Self(trimmed.to_owned()))
            .ok_or(())
    }
}

impl fmt::Display for ProviderItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

pub fn openai_responses_call_id(raw: &str) -> Option<ModelToolCallId> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() <= OPENAI_RESPONSES_CALL_ID_MAX_CHARS {
        return trimmed.parse().ok();
    }

    let hash = blake3::hash(trimmed.as_bytes());
    let hex = hash.to_hex().to_string();
    format!("call_{}", &hex[..32]).parse().ok()
}

pub fn valid_openai_responses_call_id(raw: &str) -> bool {
    let trimmed = raw.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= OPENAI_RESPONSES_CALL_ID_MAX_CHARS
}
