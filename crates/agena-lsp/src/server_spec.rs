//! Typed configuration for each LSP server (command, args, root).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// User-facing description of a single LSP server agena should manage.
///
/// `root_markers` are the file / directory names whose presence identifies
/// the server's project root for a given file path; the registry walks up
/// the file's ancestor directories until one is found, falling back to the
/// workspace root if none of them are.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LspServerSpec {
    pub name: String,
    /// Runtime-provided servers follow explicitly configured extension routes.
    #[serde(skip)]
    pub built_in: bool,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// File extensions (without the leading `.`) routed to this server.
    /// Empty matches everything — useful for catch-all servers like a
    /// generic ctags daemon.
    #[serde(default)]
    pub file_extensions: Vec<String>,
    #[serde(default)]
    pub language_ids: BTreeMap<String, String>,
    /// Names looked up in each ancestor of a file path until one is
    /// found. Examples: `Cargo.toml`, `package.json`, `.git`.
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub prefer_outermost_root: bool,
    #[serde(default)]
    pub configuration: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initialization_options: Option<serde_json::Value>,
}

impl LspServerSpec {
    pub fn handles_extension(&self, ext: &str) -> bool {
        if self.file_extensions.is_empty() {
            return true;
        }
        let normalized = ext.trim_start_matches('.').to_ascii_lowercase();
        self.file_extensions.iter().any(|candidate| {
            candidate
                .trim_start_matches('.')
                .eq_ignore_ascii_case(&normalized)
        })
    }

    /// Walk `start_dir` upward looking for any of `root_markers`. Falls
    /// back to the supplied directory when nothing matches.
    pub fn resolve_root(
        &self,
        start_dir: &std::path::Path,
        workspace_root: &std::path::Path,
    ) -> PathBuf {
        if self.root_markers.is_empty() {
            return workspace_root.to_path_buf();
        }
        self.resolve_root_with_boundary(start_dir, workspace_root, Some(workspace_root))
    }

    pub(crate) fn resolve_root_with_boundary(
        &self,
        start_dir: &Path,
        fallback: &Path,
        boundary: Option<&Path>,
    ) -> PathBuf {
        let ancestors = start_dir.ancestors().take_while(|dir| {
            boundary.is_none_or(|root| !start_dir.starts_with(root) || dir.starts_with(root))
        });
        let mut candidates = ancestors.filter(|dir| {
            self.root_markers
                .iter()
                .any(|marker| dir.join(marker).exists())
        });
        let found = if self.prefer_outermost_root {
            candidates.last()
        } else {
            candidates.next()
        };
        found.unwrap_or(fallback).to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_root_uses_explicit_fallback_and_finds_markers() {
        let dir = std::env::temp_dir().join(format!("agena-lsp-root-{}", std::process::id()));
        let nested = dir.join("project/src");
        std::fs::create_dir_all(&nested).unwrap();
        let fallback = dir.join("explicit-fallback");
        let mut spec = LspServerSpec {
            name: "test".into(),
            command: "test".into(),
            args: vec![],
            env: Default::default(),
            file_extensions: vec![],
            root_markers: vec!["marker".into()],
            ..Default::default()
        };
        assert_eq!(spec.resolve_root(&nested, &fallback), fallback);
        std::fs::write(dir.join("project/marker"), "").unwrap();
        assert_eq!(spec.resolve_root(&nested, &fallback), dir.join("project"));
        spec.root_markers.clear();
        assert_eq!(spec.resolve_root(&nested, &fallback), fallback);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn project_roots_respect_the_workspace_and_can_prefer_outer_markers() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let outer = root.join("workspace");
        let member = outer.join("member");
        std::fs::create_dir_all(member.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "").unwrap();
        let mut spec = LspServerSpec {
            root_markers: vec!["Cargo.toml".into()],
            ..Default::default()
        };
        assert_eq!(spec.resolve_root(&member.join("src"), &outer), outer);
        std::fs::write(outer.join("Cargo.toml"), "").unwrap();
        std::fs::write(member.join("Cargo.toml"), "").unwrap();
        assert_eq!(spec.resolve_root(&member.join("src"), &outer), member);
        spec.prefer_outermost_root = true;
        assert_eq!(spec.resolve_root(&member.join("src"), &outer), outer);
    }
}
