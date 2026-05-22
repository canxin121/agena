//! Project instruction file discovery.
//!
//! Walk up from the workspace root collecting any `AGENA.md`
//! files along the way (project closest to the leaf wins on conflict). Each
//! layer is returned with the path it came from so the prompt section can
//! show provenance, and stale entries can be dropped without trawling them all.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const INSTRUCTION_FILES: &[&str] = &["AGENA.md"];
const MAX_FILE_BYTES: usize = 50_000;

#[derive(Debug, Clone)]
pub struct ProjectInstructionLayer {
    pub path: PathBuf,
    pub content: String,
    pub truncated: bool,
}

/// Read a user-global instruction file from `~/.agena`, if one exists.
pub fn discover_global() -> Option<ProjectInstructionLayer> {
    home_dir().and_then(|home| discover_global_in(home.join(".agena").as_path()))
}

fn discover_global_in(dir: &Path) -> Option<ProjectInstructionLayer> {
    read_first_match(dir)
}

/// Walk from `workspace_root` upward, collecting one matching instruction
/// file per directory (first-listed name wins per dir). Layers are returned
/// outermost first so callers can render them as a top-down prompt section.
pub fn discover(workspace_root: &Path) -> Vec<ProjectInstructionLayer> {
    let mut layers = Vec::new();
    let mut current = Some(workspace_root.to_path_buf());

    while let Some(dir) = current {
        if let Some(layer) = read_first_match(&dir) {
            layers.push(layer);
        }
        current = dir.parent().map(Path::to_path_buf);
    }

    layers.reverse();
    layers
}

fn read_first_match(dir: &Path) -> Option<ProjectInstructionLayer> {
    for name in INSTRUCTION_FILES {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        match read_truncated(&path) {
            Ok((content, truncated)) => {
                return Some(ProjectInstructionLayer {
                    path,
                    content,
                    truncated,
                });
            }
            Err(err) => {
                tracing::warn!(
                    target: "agena::memory::project_instructions",
                    "failed to read {}: {err}",
                    path.display()
                );
            }
        }
    }
    None
}

fn read_truncated(path: &Path) -> io::Result<(String, bool)> {
    let raw = fs::read_to_string(path)?;
    if raw.len() <= MAX_FILE_BYTES {
        return Ok((raw, false));
    }
    let cut_at = raw[..MAX_FILE_BYTES].rfind('\n').unwrap_or(MAX_FILE_BYTES);
    Ok((raw[..cut_at].to_string(), true))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}

/// Render the layers as a Markdown section ready to splice into the system
/// prompt. Returns `None` if there is nothing to inject.
pub fn render_section(layers: &[ProjectInstructionLayer]) -> Option<String> {
    if layers.is_empty() {
        return None;
    }
    let mut out = String::from("# project instructions\n\n");
    out.push_str(
        "The following files were discovered walking up from the workspace root. \
         Closer-to-leaf entries override earlier ones on conflict.\n\n",
    );
    for layer in layers {
        out.push_str(&format!("## {}\n\n", layer.path.display()));
        out.push_str(layer.content.trim());
        if layer.truncated {
            out.push_str(&format!(
                "\n\n> NOTE: this file exceeded {} bytes and was truncated.",
                MAX_FILE_BYTES
            ));
        }
        out.push_str("\n\n");
    }
    Some(out.trim_end().to_string())
}
