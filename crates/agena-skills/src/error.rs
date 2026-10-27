use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, SkillError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillErrorKind {
    Invalid,
    NotFound,
    Conflict,
    ReadOnly,
    TooLarge,
    Io,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillError {
    pub kind: SkillErrorKind,
    pub message: String,
}

impl SkillError {
    pub fn new(kind: SkillErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(SkillErrorKind::Invalid, message)
    }
    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::new(SkillErrorKind::Conflict, message)
    }
    pub(crate) fn too_large(message: impl Into<String>) -> Self {
        Self::new(SkillErrorKind::TooLarge, message)
    }
}

impl std::fmt::Display for SkillError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SkillError {}

impl From<std::io::Error> for SkillError {
    fn from(error: std::io::Error) -> Self {
        let kind = if error.kind() == std::io::ErrorKind::NotFound {
            SkillErrorKind::NotFound
        } else {
            SkillErrorKind::Io
        };
        Self::new(kind, error.to_string())
    }
}
