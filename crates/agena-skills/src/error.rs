use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("malformed SKILL.md: {0}")]
    Malformed(String),

    #[error("skill not found: {0}")]
    NotFound(String),
}

pub type SkillResult<T> = Result<T, SkillError>;
