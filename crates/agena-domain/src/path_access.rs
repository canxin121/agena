//! Concrete filesystem permissions that can be represented by MXC.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::PermissionMode;

/// A complete resource permission. Approval decisions belong to the tool/task
/// authorization layer, never to a persisted filesystem grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PathPermissionMode {
    Readonly,
    Readwrite,
    Deny,
}

/// Internal read/write representation retained for the authorization engine.
/// The public configuration format is a single PathPermissionMode string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PathAccessModes {
    pub read: Option<PermissionMode>,
    pub write: Option<PermissionMode>,
}

impl PathAccessModes {
    pub const fn from_mode(mode: PathPermissionMode) -> Self {
        let (read, write) = match mode {
            PathPermissionMode::Readonly => (PermissionMode::Allow, PermissionMode::Deny),
            PathPermissionMode::Readwrite => (PermissionMode::Allow, PermissionMode::Allow),
            PathPermissionMode::Deny => (PermissionMode::Deny, PermissionMode::Deny),
        };
        Self {
            read: Some(read),
            write: Some(write),
        }
    }

    pub fn sandbox_mode(&self) -> Result<PathPermissionMode, &'static str> {
        match (self.read, self.write) {
            (Some(PermissionMode::Allow), Some(PermissionMode::Deny)) => {
                Ok(PathPermissionMode::Readonly)
            }
            (Some(PermissionMode::Allow), Some(PermissionMode::Allow)) => {
                Ok(PathPermissionMode::Readwrite)
            }
            (Some(PermissionMode::Deny), Some(PermissionMode::Deny)) => {
                Ok(PathPermissionMode::Deny)
            }
            (Some(PermissionMode::Deny), Some(PermissionMode::Allow)) => Err(
                "MXC cannot grant write access while denying read access; use readonly, readwrite, or deny",
            ),
            (Some(PermissionMode::Ask | PermissionMode::Auto), _)
            | (_, Some(PermissionMode::Ask | PermissionMode::Auto)) => Err(
                "filesystem permissions must be readonly, readwrite, or deny; move ask/auto to tool or task approval before migrating this rule",
            ),
            _ => Err(
                "partial read/write permissions cannot be migrated safely; specify readonly, readwrite, or deny",
            ),
        }
    }

    pub fn merge_from(&mut self, overlay: Self) {
        if overlay.read.is_some() {
            self.read = overlay.read;
        }
        if overlay.write.is_some() {
            self.write = overlay.write;
        }
    }
}

impl Serialize for PathAccessModes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.sandbox_mode()
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PathAccessModes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Legacy {
            read: Option<PermissionMode>,
            write: Option<PermissionMode>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(value) => {
                let mode = match value.as_str() {
                    "readonly" => PathPermissionMode::Readonly,
                    "readwrite" => PathPermissionMode::Readwrite,
                    "deny" => PathPermissionMode::Deny,
                    _ => {
                        return Err(serde::de::Error::custom(format!(
                            "invalid filesystem permission {value:?}; expected readonly, readwrite, or deny (ask/auto belong to tool or task approval)"
                        )));
                    }
                };
                Ok(Self::from_mode(mode))
            }
            value @ serde_json::Value::Object(_) => {
                let value: Legacy =
                    serde_json::from_value(value).map_err(serde::de::Error::custom)?;
                let modes = Self {
                    read: value.read,
                    write: value.write,
                };
                modes.sandbox_mode().map_err(serde::de::Error::custom)?;
                Ok(modes)
            }
            _ => Err(serde::de::Error::custom(
                "filesystem permissions must be the string readonly, readwrite, or deny",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concrete_modes_round_trip_as_strings() {
        for (value, mode) in [
            ("readonly", PathPermissionMode::Readonly),
            ("readwrite", PathPermissionMode::Readwrite),
            ("deny", PathPermissionMode::Deny),
        ] {
            let modes: PathAccessModes = serde_json::from_value(value.into()).unwrap();
            assert_eq!(modes.sandbox_mode(), Ok(mode));
            assert_eq!(serde_json::to_value(modes).unwrap(), value);
        }
    }

    #[test]
    fn complete_legacy_permissions_migrate_without_expanding_access() {
        for (value, expected) in [
            (
                serde_json::json!({"read":"allow","write":"deny"}),
                "readonly",
            ),
            (
                serde_json::json!({"read":"allow","write":"allow"}),
                "readwrite",
            ),
            (serde_json::json!({"read":"deny","write":"deny"}), "deny"),
        ] {
            let modes: PathAccessModes = serde_json::from_value(value).unwrap();
            assert_eq!(serde_json::to_value(modes).unwrap(), expected);
        }
    }

    #[test]
    fn unsupported_legacy_permissions_are_rejected_with_migration_guidance() {
        for value in [
            serde_json::json!({"read":"allow","write":"ask"}),
            serde_json::json!({"read":"auto","write":"auto"}),
            serde_json::json!({"read":"allow"}),
            serde_json::json!({"read":"deny","write":"allow"}),
        ] {
            assert!(serde_json::from_value::<PathAccessModes>(value).is_err());
        }
        let error = serde_json::from_value::<PathAccessModes>(
            serde_json::json!({"read":"allow","write":"ask"}),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("move ask/auto"), "{error}");
        for value in ["allow", "ask", "auto", "write", ""] {
            assert!(serde_json::from_value::<PathAccessModes>(value.into()).is_err());
        }
        assert!(
            serde_json::to_value(PathAccessModes {
                read: Some(PermissionMode::Deny),
                write: Some(PermissionMode::Allow),
            })
            .is_err()
        );
    }
}
