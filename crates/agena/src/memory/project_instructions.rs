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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("agena-pi-{label}-{suffix}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn returns_empty_when_no_files_present() {
        let dir = tmp_dir("none");
        assert!(discover(&dir).is_empty());
    }

    #[test]
    fn reads_agena_instruction_file() {
        let dir = tmp_dir("agena-file");
        fs::write(dir.join("AGENA.md"), "agena content").unwrap();
        let layers = discover(&dir);
        let chosen = layers
            .iter()
            .find(|l| l.path.parent() == Some(dir.as_path()))
            .expect("layer present for dir");
        assert!(chosen.path.ends_with("AGENA.md"));
        assert_eq!(chosen.content, "agena content");
    }

    #[test]
    fn collects_layers_walking_up_outermost_first() {
        let outer = tmp_dir("walk-up");
        let inner = outer.join("nested").join("deep");
        fs::create_dir_all(&inner).unwrap();
        fs::write(outer.join("AGENA.md"), "outer").unwrap();
        fs::write(inner.join("AGENA.md"), "inner").unwrap();
        let layers = discover(&inner);
        assert!(layers.len() >= 2);
        // outermost (parent of inner.parent…) appears first.
        let positions: Vec<&str> = layers
            .iter()
            .filter_map(|l| l.content.as_str().into())
            .collect();
        let outer_idx = positions.iter().position(|s| *s == "outer").unwrap();
        let inner_idx = positions.iter().position(|s| *s == "inner").unwrap();
        assert!(outer_idx < inner_idx, "outer should be rendered first");
    }

    #[test]
    fn truncates_oversized_file_and_marks_layer() {
        let dir = tmp_dir("truncate");
        let big = "x\n".repeat((MAX_FILE_BYTES / 2) + 1024);
        fs::write(dir.join("AGENA.md"), &big).unwrap();
        let layers = discover(&dir);
        let our = layers
            .iter()
            .find(|l| l.path.parent() == Some(dir.as_path()))
            .unwrap();
        assert!(our.truncated, "expected layer to be marked truncated");
        assert!(our.content.len() <= MAX_FILE_BYTES);
    }

    #[test]
    fn discovers_global_instruction_file_from_agena_dir() {
        let dir = tmp_dir("global").join(".agena");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("AGENA.md"), "global instructions").unwrap();

        let layer = discover_global_in(&dir).expect("global layer should load");
        assert!(layer.path.ends_with("AGENA.md"));
        assert_eq!(layer.content, "global instructions");
    }

    #[test]
    fn render_section_returns_none_for_empty_input() {
        assert!(render_section(&[]).is_none());
    }

    #[test]
    fn render_section_groups_layers_by_path() {
        let dir = tmp_dir("render");
        fs::write(dir.join("AGENA.md"), "hello").unwrap();
        let layers = discover(&dir);
        let rendered = render_section(&layers).expect("non-empty layers");
        assert!(rendered.starts_with("# project instructions"));
        assert!(rendered.contains("AGENA.md"));
        assert!(rendered.contains("hello"));
    }
}
