use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::Display;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Filesystem access level of an effect.
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
/// A filesystem access effect of a tool.
pub struct FilesystemEffect {
    pub path: String,
    pub access: FilesystemAccess,
}

/// Declared filesystem effects grouped by access. A path that both reads and
/// writes appears in both arrays. Paths are relative to the command working
/// directory (shell tools) or the workspace root; executables, interpreters,
/// and their installation directories are never listed.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub struct FilesystemEffects {
    /// Paths the command may read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(example = example_read_paths())]
    pub read: Vec<String>,
    /// Paths the command may write.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(example = example_write_paths())]
    pub write: Vec<String>,
}

fn example_read_paths() -> Vec<String> {
    vec!["src/lib.rs".to_string()]
}

fn example_write_paths() -> Vec<String> {
    vec!["target/out.txt".to_string()]
}

impl FilesystemEffects {
    pub fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty()
    }

    /// Expand into the internal per-path/access model. A path listed in both
    /// groups collapses to a single `ReadWrite` entry so permission checks are
    /// not duplicated for the same path.
    pub fn to_effects(&self) -> Vec<FilesystemEffect> {
        let mut access_by_path = std::collections::BTreeMap::<&str, FilesystemAccess>::new();
        for path in &self.read {
            let access = access_by_path
                .entry(path.as_str())
                .or_insert(FilesystemAccess::Read);
            if access.includes_write() {
                *access = FilesystemAccess::ReadWrite;
            }
        }
        for path in &self.write {
            let access = access_by_path
                .entry(path.as_str())
                .or_insert(FilesystemAccess::Write);
            if access.includes_read() {
                *access = FilesystemAccess::ReadWrite;
            }
        }
        access_by_path
            .into_iter()
            .map(|(path, access)| FilesystemEffect {
                path: path.to_owned(),
                access,
            })
            .collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
/// A network access effect of a tool.
pub struct NetworkEffect {
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::{FilesystemAccess, FilesystemEffects};

    #[test]
    fn grouped_effects_expand_to_internal_access_model() {
        let effects = FilesystemEffects {
            read: vec!["a.rs".to_string(), "both.rs".to_string()],
            write: vec!["out.txt".to_string(), "both.rs".to_string()],
        };
        let expanded = effects.to_effects();
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].path, "a.rs");
        assert_eq!(expanded[0].access, FilesystemAccess::Read);
        assert_eq!(expanded[1].path, "both.rs");
        assert_eq!(expanded[1].access, FilesystemAccess::ReadWrite);
        assert_eq!(expanded[2].path, "out.txt");
        assert_eq!(expanded[2].access, FilesystemAccess::Write);
    }

    #[test]
    fn array_form_is_rejected() {
        let result = serde_json::from_str::<FilesystemEffects>(
            r#"[
                {"access": "read", "path": "a.rs"},
                {"access": "write", "path": "out.txt"},
                {"access": "read_write", "path": "both.rs"}
            ]"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn grouped_object_round_trips() {
        let effects = FilesystemEffects {
            read: vec!["a.rs".to_string()],
            write: vec!["out.txt".to_string()],
        };
        let json = serde_json::to_string(&effects).expect("serialize grouped");
        let parsed: FilesystemEffects = serde_json::from_str(&json).expect("parse grouped");
        assert_eq!(parsed, effects);
    }

    #[test]
    fn empty_grouped_effects_is_empty() {
        assert!(FilesystemEffects::default().is_empty());
        assert!(
            FilesystemEffects {
                read: Vec::new(),
                write: vec!["x".to_string()],
            }
            .to_effects()
            .iter()
            .any(|effect| effect.access == FilesystemAccess::Write)
        );
    }
}
