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

/// Declared filesystem effects grouped by access. A path that both reads and
/// writes appears in both arrays. Paths are relative to the command working
/// directory (shell tools) or the workspace root; executables, interpreters,
/// and their installation directories are never listed.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
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

impl<'de> Deserialize<'de> for FilesystemEffects {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Grouped {
                #[serde(default)]
                read: Vec<String>,
                #[serde(default)]
                write: Vec<String>,
            },
            // Legacy `[{ "access": "read|write|read_write", "path": ... }]`
            // form accepted so previously persisted invocations keep parsing.
            Legacy(Vec<FilesystemEffect>),
        }
        match Wire::deserialize(deserializer)? {
            Wire::Grouped { read, write } => Ok(Self { read, write }),
            Wire::Legacy(effects) => {
                let mut grouped = Self::default();
                for effect in effects {
                    if effect.access.includes_read() {
                        grouped.read.push(effect.path.clone());
                    }
                    if effect.access.includes_write() {
                        grouped.write.push(effect.path);
                    }
                }
                Ok(grouped)
            }
        }
    }
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
    fn legacy_array_form_is_still_deserialized() {
        let effects: FilesystemEffects = serde_json::from_str(
            r#"[
                {"access": "read", "path": "a.rs"},
                {"access": "write", "path": "out.txt"},
                {"access": "read_write", "path": "both.rs"}
            ]"#,
        )
        .expect("legacy array parses");
        assert_eq!(effects.read, vec!["a.rs", "both.rs"]);
        assert_eq!(effects.write, vec!["out.txt", "both.rs"]);
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
