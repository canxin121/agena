use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::SkillResult;
use crate::skill::Skill;

/// Runtime defaults no longer scan implicit workspace or user directories.
pub fn default_roots(_workspace: Option<&Path>) -> Vec<PathBuf> {
    Vec::new()
}

/// Runtime defaults no longer scan implicit slash-command directories.
pub fn default_command_roots(_workspace: Option<&Path>) -> Vec<PathBuf> {
    Vec::new()
}

pub fn scan(roots: &[PathBuf]) -> SkillResult<Vec<Skill>> {
    scan_matching(
        roots,
        |path| path.file_name().and_then(|s| s.to_str()) == Some("SKILL.md"),
        |path| Skill::from_path(path),
    )
}

pub fn scan_commands(roots: &[PathBuf]) -> SkillResult<Vec<Skill>> {
    scan_matching(
        roots,
        |path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        },
        |path| Skill::from_command_path(path),
    )
}

fn scan_matching(
    roots: &[PathBuf],
    matches_file: impl Fn(&Path) -> bool,
    load: impl Fn(&Path) -> SkillResult<Skill>,
) -> SkillResult<Vec<Skill>> {
    let mut skills = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.is_file() || !matches_file(p) {
                continue;
            }
            match load(p) {
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
