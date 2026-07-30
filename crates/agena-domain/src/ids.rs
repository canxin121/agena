use std::borrow::Borrow;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use uuid::Uuid;

macro_rules! integer_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub i64);
        impl $name {
            pub const fn raw(self) -> i64 {
                self.0
            }
        }
        impl From<i64> for $name {
            fn from(value: i64) -> Self {
                Self(value)
            }
        }
        impl From<$name> for i64 {
            fn from(value: $name) -> Self {
                value.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

integer_id!(MessageId);
integer_id!(PartId);

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }
        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

uuid_id!(ExecutionId);
uuid_id!(RunId);
uuid_id!(TurnId);
uuid_id!(ResponseId);
uuid_id!(ResponseSegmentId);
uuid_id!(ActivityId);

#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCallId(pub SmolStr);
impl ToolCallId {
    pub fn new(id: impl Into<SmolStr>) -> Self {
        Self(id.into())
    }
}
impl Borrow<str> for ToolCallId {
    fn borrow(&self) -> &str {
        self.0.as_str()
    }
}
impl AsRef<str> for ToolCallId {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}
impl From<&str> for ToolCallId {
    fn from(value: &str) -> Self {
        Self(SmolStr::new(value))
    }
}
impl From<String> for ToolCallId {
    fn from(value: String) -> Self {
        Self(SmolStr::new(value))
    }
}
impl From<SmolStr> for ToolCallId {
    fn from(value: SmolStr) -> Self {
        Self(value)
    }
}
impl std::fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
