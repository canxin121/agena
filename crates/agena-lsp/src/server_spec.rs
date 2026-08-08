//! Typed configuration for each LSP server (command, args, root).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// User-facing description of a single LSP server agena should manage.
///
/// `root_markers` are the file / directory names whose presence identifies
/// the server's project root for a given file path; the registry walks up
/// the file's ancestor directories until one is found, falling back to the
/// workspace root if none of them are.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LspServerSpec {
    pub name: String,
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
    /// Names looked up in each ancestor of a file path until one is
    /// found. Examples: `Cargo.toml`, `package.json`, `.git`.
    #[serde(default)]
    pub root_markers: Vec<String>,
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
    /// back to `workspace_root` when nothing matches.
    pub fn resolve_root(
        &self,
        start_dir: &std::path::Path,
        workspace_root: &std::path::Path,
    ) -> PathBuf {
        if self.root_markers.is_empty() {
            return workspace_root.to_path_buf();
        }
        let mut current = Some(start_dir.to_path_buf());
        while let Some(dir) = current {
            for marker in &self.root_markers {
                if dir.join(marker).exists() {
                    return dir;
                }
            }
            current = dir.parent().map(std::path::Path::to_path_buf);
        }
        workspace_root.to_path_buf()
    }
}
