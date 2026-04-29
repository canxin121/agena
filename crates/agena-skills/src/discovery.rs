use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::{SkillError, SkillResult};
use crate::skill::Skill;

/// Default discovery roots in priority order.  Workspace skills win over
/// user skills win over claude-code skills win over bundled ones.
pub fn default_roots(workspace: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(ws) = workspace {
        roots.push(ws.join(".agena").join("skills"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".agena").join("skills"));
        roots.push(home.join(".claude").join("skills"));
    }
    roots
}

pub fn scan(roots: &[PathBuf]) -> SkillResult<Vec<Skill>> {
    let mut skills = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            if p.file_name().and_then(|s| s.to_str()) != Some("SKILL.md") {
                continue;
            }
            match Skill::from_path(p) {
                Ok(skill) => skills.push(skill),
                Err(e) => {
                    tracing::warn!(
                        target: "agena_skills::discovery",
                        "skipping {p:?}: {e}"
                    );
                }
            }
        }
    }
    Ok(skills)
}

pub fn dirs_home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

mod dirs {
    use std::path::PathBuf;
    pub fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

// Re-export for callers that want to bypass default_roots logic.
pub use dirs::home_dir;

// Sanity: keep SkillError used (clippy)
#[allow(dead_code)]
fn _silence(_: SkillError) {}
