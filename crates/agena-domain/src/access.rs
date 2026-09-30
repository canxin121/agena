use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// The kind of access being evaluated: read or write.
pub enum AccessKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Selector for access kinds: matches read access, write access, or either.
pub enum AccessSelector {
    Read,
    Write,
    Any,
}

/// The kind of path access a plugin asks the host about, and the kind the host
/// classifies a declared path under.
///
/// It is the wire form of [`AccessKind`]: plugins describe a query or a
/// declared path with it, while policy evaluation works on `AccessKind`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    Read,
    Write,
}

impl From<PathKind> for AccessKind {
    fn from(kind: PathKind) -> Self {
        match kind {
            PathKind::Read => Self::Read,
            PathKind::Write => Self::Write,
        }
    }
}

impl From<AccessKind> for PathKind {
    fn from(kind: AccessKind) -> Self {
        match kind {
            AccessKind::Read => Self::Read,
            AccessKind::Write => Self::Write,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessKind, AccessSelector, PathKind};

    #[test]
    fn access_values_distinguish_read_write_and_any() {
        assert_ne!(AccessKind::Read, AccessKind::Write);
        assert_ne!(AccessSelector::Read, AccessSelector::Write);
        assert_ne!(AccessSelector::Any, AccessSelector::Read);
    }

    #[test]
    fn path_kind_round_trips_through_access_kind() {
        assert_eq!(AccessKind::from(PathKind::Write), AccessKind::Write);
        assert_eq!(PathKind::from(AccessKind::Read), PathKind::Read);
    }
}
