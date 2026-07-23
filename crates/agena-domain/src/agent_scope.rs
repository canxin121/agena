use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

/// Scope from which an agent profile was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum AgentScope {
    Project,
    User,
    Default,
}

impl AsRef<str> for AgentScope {
    fn as_ref(&self) -> &str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Default => "default",
        }
    }
}
