//! Skill parsing and discovery error types.

use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the skills subsystem.
pub enum SkillError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("malformed SKILL.md: {0}")]
    Malformed(String),

    #[error("skill not found: {0}")]
    NotFound(String),

    #[error("invalid skill resource path: {0}")]
    InvalidResourcePath(String),

    #[error("skill resource exceeds the {limit} byte limit: {path}")]
    ResourceTooLarge { path: String, limit: usize },

    #[error("skill resource is not UTF-8 text: {0}")]
    ResourceNotText(String),
}

/// Result alias for skills operations.
pub type SkillResult<T> = Result<T, SkillError>;
