use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{SkillError, SkillResult};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillFrontmatter {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Names of tools the skill expects to be able to call.  Used to
    /// constrain the catalog while the skill is running.  Not a security
    /// boundary — the LLM can still try to call other tools, they just
    /// won't be advertised.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Optional model preference for the skill turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional aliases used to resolve the skill (e.g. slash command
    /// short names).
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub frontmatter: SkillFrontmatter,
    pub body: String,
    /// Absolute path to the SKILL.md file (or "<bundled>" for compiled-in).
    pub source_path: Option<PathBuf>,
}

impl Skill {
    /// Parse a SKILL.md from disk.
    pub fn from_path(path: impl AsRef<Path>) -> SkillResult<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)?;
        let mut skill = Self::from_raw(&raw)?;
        skill.source_path = Some(path.to_path_buf());
        Ok(skill)
    }

    /// Parse a SKILL.md from in-memory text.  The frontmatter is required;
    /// the body is everything after it.
    pub fn from_raw(raw: &str) -> SkillResult<Self> {
        let stripped = raw.strip_prefix("---\n").ok_or_else(|| {
            SkillError::Malformed("SKILL.md must start with '---' frontmatter".into())
        })?;
        let end = stripped.find("\n---").ok_or_else(|| {
            SkillError::Malformed("SKILL.md frontmatter missing closing '---'".into())
        })?;
        let yaml = &stripped[..end];
        let body = stripped[end + 4..].trim_start_matches('\n').to_string();
        let fm: SkillFrontmatter = serde_yaml::from_str(yaml)?;
        if fm.name.trim().is_empty() {
            return Err(SkillError::Malformed("SKILL.md `name` is required".into()));
        }
        Ok(Self {
            frontmatter: fm,
            body,
            source_path: None,
        })
    }

    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_ascii_lowercase();
        self.frontmatter.name.to_ascii_lowercase() == q
            || self
                .frontmatter
                .aliases
                .iter()
                .any(|a| a.to_ascii_lowercase() == q)
    }
}
