use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

/// Access policy applied to an additional working directory attached to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SessionWorkingDirectoryAccess {
    /// Inherit the session's primary workspace read/write defaults.
    #[default]
    #[serde(rename = "inherit")]
    Inherit,
    /// Allow reads up to the workspace read default, while denying writes.
    #[serde(rename = "readonly", alias = "read_only")]
    ReadOnly,
    /// Allow reads and writes within this directory (still bounded by rules and ceilings).
    #[serde(rename = "readwrite", alias = "read_write")]
    ReadWrite,
}

impl SessionWorkingDirectoryAccess {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::ReadOnly => "readonly",
            Self::ReadWrite => "readwrite",
        }
    }
}

impl fmt::Display for SessionWorkingDirectoryAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SessionWorkingDirectoryAccess {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "inherit" | "default" => Ok(Self::Inherit),
            "readonly" | "read_only" | "read-only" | "ro" => Ok(Self::ReadOnly),
            "readwrite" | "read_write" | "read-write" | "rw" => Ok(Self::ReadWrite),
            other => Err(format!(
                "invalid working directory access {other:?}; expected inherit, readonly, or readwrite"
            )),
        }
    }
}

/// One additional working directory registered for a session along with its access mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionWorkingDirectory {
    pub path: PathBuf,
    #[serde(default)]
    pub access: SessionWorkingDirectoryAccess,
}

impl SessionWorkingDirectory {
    pub fn new(path: PathBuf, access: SessionWorkingDirectoryAccess) -> Self {
        Self { path, access }
    }
}

impl<'de> Deserialize<'de> for SessionWorkingDirectory {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Entry {
            path: PathBuf,
            #[serde(default)]
            access: SessionWorkingDirectoryAccess,
        }

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Path(PathBuf),
            Entry(Entry),
        }

        match Raw::deserialize(deserializer)? {
            Raw::Path(path) => Ok(Self {
                path,
                access: SessionWorkingDirectoryAccess::Inherit,
            }),
            Raw::Entry(entry) => Ok(Self {
                path: entry.path,
                access: entry.access,
            }),
        }
    }
}

/// Register or update an additional working directory in the session that owns the selected workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterSessionWorkingDirectory {
    pub path: PathBuf,
    #[serde(default)]
    pub access: SessionWorkingDirectoryAccess,
}

/// Remove a previously registered additional working directory from a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveSessionWorkingDirectory {
    pub path: PathBuf,
}

/// Result captured by the same transaction that updates the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWorkingDirectoryRegistration {
    pub added: bool,
    #[serde(default)]
    pub updated: bool,
    pub directory: PathBuf,
    #[serde(default)]
    pub access: SessionWorkingDirectoryAccess,
    pub directories: Vec<PathBuf>,
    #[serde(default)]
    pub entries: Vec<SessionWorkingDirectory>,
}

/// Result captured by the same transaction that removes a directory from the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWorkingDirectoryRemoval {
    pub removed: bool,
    pub directory: PathBuf,
    pub directories: Vec<PathBuf>,
    #[serde(default)]
    pub entries: Vec<SessionWorkingDirectory>,
}

/// Full view of a session's primary workspace root and registered additional directories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionWorkingDirectoriesView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<PathBuf>,
    pub directories: Vec<PathBuf>,
    #[serde(default)]
    pub entries: Vec<SessionWorkingDirectory>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_legacy_paths_and_structured_entries() {
        let legacy: Vec<SessionWorkingDirectory> =
            serde_json::from_str(r#"["/tmp/a","/tmp/b"]"#).unwrap();
        assert_eq!(
            legacy,
            vec![
                SessionWorkingDirectory::new("/tmp/a".into(), SessionWorkingDirectoryAccess::Inherit),
                SessionWorkingDirectory::new("/tmp/b".into(), SessionWorkingDirectoryAccess::Inherit),
            ]
        );

        let mixed: Vec<SessionWorkingDirectory> = serde_json::from_str(
            r#"["/tmp/a",{"path":"/tmp/b","access":"readonly"},{"path":"/tmp/c","access":"readwrite"}]"#,
        )
        .unwrap();
        assert_eq!(mixed[0].access, SessionWorkingDirectoryAccess::Inherit);
        assert_eq!(mixed[1].access, SessionWorkingDirectoryAccess::ReadOnly);
        assert_eq!(mixed[2].access, SessionWorkingDirectoryAccess::ReadWrite);
    }
}
