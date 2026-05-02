//! Custom subagent profile registry.
//!
//! Discovers Markdown files under `.agena/agents/` (project-local, walked
//! up from the workspace root) and `~/.agena/agents/` (user-global). Each
//! file describes a named subagent the dispatcher can route to: a system
//! prompt, optional tool whitelist, and optional preferred model. Mirrors
//! the layout of `crates/agena/src/commands/` and the SKILL.md frontmatter
//! convention `agena-skills` already exposes, so users who know one know
//! the other.
//!
//! Example `.agena/agents/explorer.md`:
//!
//! ```markdown
//! ---
//! description: "Read-only repo explorer"
//! allowed_tools: ["read", "glob", "grep", "view_file"]
//! model: "claude-haiku-4-5"
//! aliases: ["scout"]
//! ---
//! You are a focused codebase explorer. Read files, grep for symbols, and
//! report concise findings. Do not edit anything.
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentScope {
    Project,
    User,
    Builtin,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentFrontmatter {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentProfile {
    pub name: String,
    pub frontmatter: AgentFrontmatter,
    pub prompt: String,
    pub source_path: Option<PathBuf>,
    pub scope: AgentScope,
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed agent: {0}")]
    Malformed(String),
    #[error("yaml frontmatter error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("unknown subagent profile: {0}")]
    UnknownProfile(String),
}

pub type AgentResult<T> = Result<T, AgentError>;

#[derive(Debug, Clone, Default)]
pub struct SubagentRegistry {
    by_name: BTreeMap<String, AgentProfile>,
}

impl SubagentRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Discover profiles rooted at `workspace_root` (walks up to find any
    /// `.agena/agents/` ancestors) and `user_root` (typically
    /// `~/.agena/agents`). Project entries win on name collisions.
    pub fn discover(workspace_root: &Path, user_root: Option<&Path>) -> Self {
        let mut registry = Self::default();
        if let Some(user) = user_root {
            registry.load_dir(user, AgentScope::User);
        }
        for dir in collect_project_agent_dirs(workspace_root) {
            registry.load_dir(&dir, AgentScope::Project);
        }
        registry
    }

    fn load_dir(&mut self, dir: &Path, scope: AgentScope) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue,
            };
            match AgentProfile::from_path(&path, &stem, scope) {
                Ok(profile) => {
                    self.insert(profile, scope);
                }
                Err(err) => {
                    tracing::warn!(
                        target: "agena::agents",
                        "skipping subagent profile `{}`: {err}",
                        path.display()
                    );
                }
            }
        }
    }

    fn insert(&mut self, profile: AgentProfile, scope: AgentScope) {
        let candidate_keys = std::iter::once(profile.name.clone())
            .chain(profile.frontmatter.aliases.iter().cloned())
            .collect::<Vec<_>>();
        for key in candidate_keys {
            match self.by_name.get(&key) {
                Some(existing) => {
                    if scope_priority(scope) >= scope_priority(existing.scope) {
                        self.by_name.insert(key, profile.clone());
                    }
                }
                None => {
                    self.by_name.insert(key, profile.clone());
                }
            }
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    pub fn list(&self) -> Vec<&AgentProfile> {
        let mut seen: BTreeMap<String, &AgentProfile> = BTreeMap::new();
        for profile in self.by_name.values() {
            seen.entry(profile.name.clone()).or_insert(profile);
        }
        seen.into_values().collect()
    }

    pub fn get(&self, name: &str) -> Option<&AgentProfile> {
        self.by_name.get(name)
    }

    pub fn require(&self, name: &str) -> AgentResult<&AgentProfile> {
        self.get(name)
            .ok_or_else(|| AgentError::UnknownProfile(name.to_string()))
    }
}

impl AgentProfile {
    pub fn from_path(path: &Path, stem: &str, scope: AgentScope) -> AgentResult<Self> {
        let raw = std::fs::read_to_string(path)?;
        let mut profile = Self::from_raw(&raw, stem, scope)?;
        profile.source_path = Some(path.to_path_buf());
        Ok(profile)
    }

    pub fn from_raw(raw: &str, default_name: &str, scope: AgentScope) -> AgentResult<Self> {
        if default_name.trim().is_empty() {
            return Err(AgentError::Malformed(
                "agent file name (stem) must not be empty".into(),
            ));
        }
        let (frontmatter, prompt) = parse_frontmatter(raw)?;
        Ok(Self {
            name: default_name.to_string(),
            frontmatter,
            prompt,
            source_path: None,
            scope,
        })
    }
}

fn parse_frontmatter(raw: &str) -> AgentResult<(AgentFrontmatter, String)> {
    let normalized = raw.replace("\r\n", "\n");
    let Some(stripped) = normalized.strip_prefix("---\n") else {
        return Ok((AgentFrontmatter::default(), normalized.trim().to_string()));
    };
    let Some(end) = stripped.find("\n---") else {
        return Err(AgentError::Malformed(
            "frontmatter missing closing '---'".into(),
        ));
    };
    let yaml = &stripped[..end];
    let body = stripped[end + 4..].trim_start_matches('\n').to_string();
    let frontmatter: AgentFrontmatter = if yaml.trim().is_empty() {
        AgentFrontmatter::default()
    } else {
        serde_yaml::from_str(yaml)?
    };
    Ok((frontmatter, body))
}

fn collect_project_agent_dirs(workspace_root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut current = Some(workspace_root.to_path_buf());
    while let Some(dir) = current {
        let candidate = dir.join(".agena").join("agents");
        if candidate.is_dir() {
            dirs.push(candidate);
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    dirs.reverse();
    dirs
}

fn scope_priority(scope: AgentScope) -> u8 {
    match scope {
        AgentScope::Builtin => 0,
        AgentScope::User => 1,
        AgentScope::Project => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agena-agt-{label}-{suffix}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let raw = "---\ndescription: explorer\nallowed_tools:\n  - read\n  - grep\nmodel: gpt-5\n---\nYou explore the repo.";
        let profile = AgentProfile::from_raw(raw, "explorer", AgentScope::Project).unwrap();
        assert_eq!(profile.name, "explorer");
        assert_eq!(profile.frontmatter.description, "explorer");
        assert_eq!(profile.frontmatter.allowed_tools, vec!["read", "grep"]);
        assert_eq!(profile.frontmatter.model.as_deref(), Some("gpt-5"));
        assert_eq!(profile.prompt.trim(), "You explore the repo.");
    }

    #[test]
    fn missing_frontmatter_treats_whole_file_as_prompt() {
        let profile = AgentProfile::from_raw("just the prompt", "plain", AgentScope::User).unwrap();
        assert_eq!(profile.prompt, "just the prompt");
        assert!(profile.frontmatter.description.is_empty());
    }

    #[test]
    fn frontmatter_without_closing_marker_errors() {
        let raw = "---\ndescription: oops\nbody without closing marker";
        let err = AgentProfile::from_raw(raw, "x", AgentScope::User).unwrap_err();
        assert!(matches!(err, AgentError::Malformed(_)));
    }

    #[test]
    fn project_overrides_user_with_same_name() {
        let work = temp_dir("project-wins");
        let user = temp_dir("user-base");
        let project_agents = work.join(".agena").join("agents");
        let user_agents = user.join("agents");
        fs::create_dir_all(&project_agents).unwrap();
        fs::create_dir_all(&user_agents).unwrap();
        fs::write(
            project_agents.join("review.md"),
            "---\ndescription: project reviewer\n---\nproject prompt",
        )
        .unwrap();
        fs::write(
            user_agents.join("review.md"),
            "---\ndescription: user reviewer\n---\nuser prompt",
        )
        .unwrap();

        let registry = SubagentRegistry::discover(&work, Some(&user_agents));
        let profile = registry.get("review").expect("profile present");
        assert_eq!(profile.scope, AgentScope::Project);
        assert_eq!(profile.frontmatter.description, "project reviewer");
    }

    #[test]
    fn aliases_resolve_to_the_same_profile_and_list_dedupes() {
        let work = temp_dir("aliases");
        let dir = work.join(".agena").join("agents");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("explorer.md"),
            "---\ndescription: explore\naliases: [\"scout\", \"recon\"]\n---\nexplore",
        )
        .unwrap();
        let registry = SubagentRegistry::discover(&work, None);
        assert_eq!(registry.get("explorer").unwrap().name, "explorer");
        assert_eq!(registry.get("scout").unwrap().name, "explorer");
        assert_eq!(registry.get("recon").unwrap().name, "explorer");
        assert_eq!(registry.list().len(), 1);
    }

    #[test]
    fn require_returns_unknown_profile_error() {
        let registry = SubagentRegistry::empty();
        let err = registry.require("nope").unwrap_err();
        assert!(matches!(err, AgentError::UnknownProfile(_)));
    }

    #[test]
    fn discovery_walks_up_to_find_agent_dir() {
        let outer = temp_dir("walk-up");
        fs::create_dir_all(outer.join(".agena").join("agents")).unwrap();
        fs::write(
            outer.join(".agena").join("agents").join("planner.md"),
            "---\ndescription: planner\n---\nyou plan",
        )
        .unwrap();
        let nested = outer.join("nested").join("deep");
        fs::create_dir_all(&nested).unwrap();
        let registry = SubagentRegistry::discover(&nested, None);
        assert!(registry.get("planner").is_some());
    }

    #[test]
    fn malformed_yaml_in_one_file_does_not_kill_discovery() {
        let work = temp_dir("malformed");
        let dir = work.join(".agena").join("agents");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ok.md"), "---\ndescription: ok\n---\nbody").unwrap();
        // missing closing marker → triggers warn + skip, not panic
        fs::write(dir.join("bad.md"), "---\ndescription: bad\nno closing").unwrap();
        let registry = SubagentRegistry::discover(&work, None);
        assert!(registry.get("ok").is_some());
        assert!(registry.get("bad").is_none());
    }
}
