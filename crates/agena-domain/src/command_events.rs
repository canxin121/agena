use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
/// Stream a command output delta belongs to.
pub enum CommandOutputStream {
    Stdout,
    Stderr,
}
