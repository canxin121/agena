//! Skill discovery across the standard roots.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::SkillResult;
use crate::skill::Skill;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Diagnostic produced while discovering skills.
pub struct DiscoveryDiagnostic {
    pub path: PathBuf,
    pub diagnostic: String,
    pub failure: agena_failure::UserProblem,
}

#[derive(Debug, Clone, Default)]
/// Result of a skill discovery pass.
pub struct DiscoveryReport {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<DiscoveryDiagnostic>,
}

/// Standard Skill roots ordered from lower to higher precedence. Later roots
/// replace earlier skills with the same canonical name in the runtime catalog.
pub fn default_roots(workspace: Option<&Path>) -> Vec<PathBuf> {
    default_roots_with_home(
        workspace,
        user_home_dir().as_deref(),
        agena_home_dir().as_deref(),
    )
}

/// Standard slash-command roots ordered from lower to higher precedence.
pub fn default_command_roots(workspace: Option<&Path>) -> Vec<PathBuf> {
    let user_home = user_home_dir();
    let agena_home = agena_home_dir();
    let mut roots = Vec::new();
    if let Some(home) = agena_home {
        roots.push(home.join("commands"));
    }
    if let Some(home) = user_home {
        roots.push(home.join(".agents/commands"));
    }
    if let Some(workspace) = workspace {
        roots.push(workspace.join(".agena/commands"));
        roots.push(workspace.join(".agents/commands"));
    }
    roots
}

fn default_roots_with_home(
    workspace: Option<&Path>,
    user_home: Option<&Path>,
    agena_home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = agena_home {
        roots.push(home.join("skills"));
    }
    if let Some(home) = user_home {
        roots.push(home.join(".agents/skills"));
    }
    if let Some(workspace) = workspace {
        roots.push(workspace.join(".agena/skills"));
        roots.push(workspace.join(".agents/skills"));
    }
    roots
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
}

fn agena_home_dir() -> Option<PathBuf> {
    std::env::var_os("AGENA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| user_home_dir().map(|home| home.join("agena")))
}

pub fn scan(roots: &[PathBuf]) -> SkillResult<Vec<Skill>> {
    Ok(scan_with_diagnostics(roots).skills)
}

pub fn scan_with_diagnostics(roots: &[PathBuf]) -> DiscoveryReport {
    scan_matching(
        roots,
        |path| path.file_name().and_then(|s| s.to_str()) == Some("SKILL.md"),
        |path| Skill::from_path(path),
    )
}

pub fn scan_commands(roots: &[PathBuf]) -> SkillResult<Vec<Skill>> {
    Ok(scan_commands_with_diagnostics(roots).skills)
}

pub fn scan_commands_with_diagnostics(roots: &[PathBuf]) -> DiscoveryReport {
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
) -> DiscoveryReport {
    let mut skills = Vec::new();
    let mut diagnostics = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    let path = error
                        .path()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| root.clone());
                    diagnostics.push(discovery_diagnostic(path, error.to_string()));
                    continue;
                }
            };
            let p = entry.path();
            if !p.is_file() || !matches_file(p) {
                continue;
            }
            match load(p) {
                Ok(skill) => skills.push(skill),
                Err(e) => {
                    diagnostics.push(discovery_diagnostic(p.to_path_buf(), e.to_string()));
                }
            }
        }
    }
    DiscoveryReport {
        skills,
        diagnostics,
    }
}

fn discovery_diagnostic(path: PathBuf, diagnostic: String) -> DiscoveryDiagnostic {
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    let failure = Failure::new(
        FailureCode::new("skills.discovery_item_failed"),
        FailureCategory::InvalidInput,
        FailureResponsibility::Caller,
        RetryDirective::CorrectInput,
        RecoveryDirective::OpenSettings,
        FailureImpact::PartialSuccess,
        UserPresentation::new(
            "skills-discovery-item-failed",
            "A skill or command could not be loaded. Review its definition.",
        ),
    );
    tracing::warn!(
        target: "agena_skills::discovery",
        failure_id = %failure.id,
        path = %path.display(),
        diagnostic = %diagnostic,
        "skipping invalid or unreadable discovery item"
    );
    DiscoveryDiagnostic {
        path,
        diagnostic,
        failure: failure.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_roots_are_ordered_by_scope_and_precedence() {
        let roots = default_roots_with_home(
            Some(Path::new("/workspace")),
            Some(Path::new("/user")),
            Some(Path::new("/agena-home")),
        );
        assert_eq!(
            roots,
            [
                PathBuf::from("/agena-home/skills"),
                PathBuf::from("/user/.agents/skills"),
                PathBuf::from("/workspace/.agena/skills"),
                PathBuf::from("/workspace/.agents/skills"),
            ]
        );
    }

    #[test]
    fn malformed_skills_are_reported_without_hiding_valid_entries() {
        let dir = tempfile::tempdir().expect("temp dir");
        let valid = dir.path().join("valid");
        let invalid = dir.path().join("invalid");
        std::fs::create_dir_all(&valid).expect("valid dir");
        std::fs::create_dir_all(&invalid).expect("invalid dir");
        std::fs::write(valid.join("SKILL.md"), "---\nname: valid\n---\nBody").expect("valid skill");
        std::fs::write(invalid.join("SKILL.md"), "not frontmatter").expect("invalid skill");
        let report = scan_with_diagnostics(&[dir.path().to_path_buf()]);
        assert_eq!(report.skills.len(), 1);
        assert_eq!(report.skills[0].frontmatter.name, "valid");
        assert_eq!(report.diagnostics.len(), 1);
        assert!(report.diagnostics[0].path.ends_with("invalid/SKILL.md"));
    }
}
