use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::Display;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FilesystemAccess {
    Read,
    Write,
    ReadWrite,
}
impl FilesystemAccess {
    pub const fn includes_read(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }
    pub const fn includes_write(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct FilesystemEffect {
    pub path: String,
    pub access: FilesystemAccess,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct NetworkEffect {
    pub target: String,
}
