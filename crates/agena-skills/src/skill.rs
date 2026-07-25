use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{SkillError, SkillResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Names of tools the skill expects to be able to call.  Used to
    /// constrain the catalog while the skill is running.  Not a security
    /// boundary — the LLM can still try to call other tools, they just
    /// won't be advertised.
    #[serde(default, alias = "allowed-tools")]
    pub allowed_tools: Vec<String>,
    /// Optional model preference for the skill run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional aliases used to resolve the skill (e.g. slash command
    /// short names).
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Whether the skill can be activated explicitly by a user or model.
    #[serde(default = "default_true", alias = "user-invocable")]
    pub user_invocable: bool,
    /// Whether the runtime may activate the skill without an explicit request.
    #[serde(default, alias = "allow-implicit-invocation")]
    pub allow_implicit_invocation: bool,
    /// Optional workspace-relative glob hints used for relevance ranking.
    #[serde(default)]
    pub paths: Vec<String>,
    /// External capabilities required before activation.
    #[serde(default)]
    pub dependencies: SkillDependencies,
}

impl Default for SkillFrontmatter {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            allowed_tools: Vec::new(),
            model: None,
            aliases: Vec::new(),
            user_invocable: true,
            allow_implicit_invocation: false,
            paths: Vec::new(),
            dependencies: SkillDependencies::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SkillDependencies {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub mcp: Vec<String>,
    #[serde(default)]
    pub environment: Vec<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub frontmatter: SkillFrontmatter,
    pub body: String,
    /// Absolute path to the SKILL.md file (or `<bundled>` for compiled-in).
    pub source_path: Option<PathBuf>,
}

impl Skill {
    /// Constructs a built-in skill from typed Rust data.
    ///
    /// Built-ins are product capabilities, not files to parse at compile time;
    /// keeping their metadata structured makes their contract explicit and
    /// avoids a second text-format loading path for the shipped defaults.
    pub fn bundled(frontmatter: SkillFrontmatter, body: impl Into<String>) -> Self {
        Self {
            frontmatter,
            body: body.into(),
            source_path: None,
        }
    }

    /// Parse a SKILL.md from disk.
    pub fn from_path(path: impl AsRef<Path>) -> SkillResult<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)?;
        let mut skill = Self::from_raw(&raw)?;
        skill.source_path = Some(path.to_path_buf());
        Ok(skill)
    }

    /// Parse a user command markdown file from disk.
    pub fn from_command_path(path: impl AsRef<Path>) -> SkillResult<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)?;
        let default_name = path.file_stem().and_then(|name| name.to_str());
        let mut skill = Self::from_raw_with_default_name(&raw, default_name)?;
        skill.source_path = Some(path.to_path_buf());
        Ok(skill)
    }

    /// Parse a SKILL.md from in-memory text.  The frontmatter is required;
    /// the body is everything after it.
    pub fn from_raw(raw: &str) -> SkillResult<Self> {
        Self::from_raw_with_default_name(raw, None)
    }

    fn from_raw_with_default_name(raw: &str, default_name: Option<&str>) -> SkillResult<Self> {
        let stripped = raw.strip_prefix("---\n").ok_or_else(|| {
            SkillError::Malformed("SKILL.md must start with '---' frontmatter".into())
        })?;
        let end = stripped.find("\n---").ok_or_else(|| {
            SkillError::Malformed("SKILL.md frontmatter missing closing '---'".into())
        })?;
        let yaml = &stripped[..end];
        let body = stripped[end + 4..].trim_start_matches('\n').to_string();
        let mut fm: SkillFrontmatter = serde_yaml::from_str(yaml)?;
        if fm.name.trim().is_empty() {
            if let Some(default_name) = default_name.filter(|name| !name.trim().is_empty()) {
                fm.name = default_name.to_string();
            } else {
                return Err(SkillError::Malformed("SKILL.md `name` is required".into()));
            }
        }
        Ok(Self {
            frontmatter: fm,
            body,
            source_path: None,
        })
    }

    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_ascii_lowercase();
        self.frontmatter.name.to_ascii_lowercase() == q
            || self
                .frontmatter
                .aliases
                .iter()
                .any(|a| a.to_ascii_lowercase() == q)
    }

    /// Stable content identity used by catalogs, activation records and stale
    /// resource checks. Paths are deliberately excluded so moving an unchanged
    /// skill does not alter its content identity.
    pub fn content_hash(&self) -> String {
        let mut digest = Sha256::new();
        let frontmatter = serde_yaml::to_string(&self.frontmatter).unwrap_or_default();
        digest.update(frontmatter.as_bytes());
        digest.update([0]);
        digest.update(self.body.as_bytes());
        hex::encode(digest.finalize())
    }

    pub fn resource_root(&self) -> Option<&Path> {
        self.source_path.as_deref()?.parent()
    }

    /// Read a UTF-8 resource contained by the skill directory. Canonical path
    /// checks reject `..` and symlink escapes before any content is returned.
    pub fn read_text_resource(&self, relative: &Path, max_bytes: usize) -> SkillResult<String> {
        if relative.as_os_str().is_empty() || relative.is_absolute() {
            return Err(SkillError::InvalidResourcePath(
                relative.display().to_string(),
            ));
        }
        if relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        }) {
            return Err(SkillError::InvalidResourcePath(
                relative.display().to_string(),
            ));
        }
        let root = self.resource_root().ok_or_else(|| {
            SkillError::InvalidResourcePath("bundled skills have no resource directory".into())
        })?;
        let canonical_root = root.canonicalize()?;
        let candidate = root.join(relative);
        let canonical_candidate = candidate.canonicalize()?;
        if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
            return Err(SkillError::InvalidResourcePath(
                relative.display().to_string(),
            ));
        }
        let metadata = canonical_candidate.metadata()?;
        if metadata.len() > max_bytes as u64 {
            return Err(SkillError::ResourceTooLarge {
                path: relative.display().to_string(),
                limit: max_bytes,
            });
        }
        let bytes = std::fs::read(&canonical_candidate)?;
        String::from_utf8(bytes)
            .map_err(|_| SkillError::ResourceNotText(relative.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_kebab_case_fields_and_hashes_content() {
        let skill = Skill::from_raw(
            "---\nname: verify\nallowed-tools: [agena.fs.read]\nuser-invocable: true\nallow-implicit-invocation: false\ndependencies:\n  mcp: [docs]\n---\nCheck it.\n",
        )
        .expect("parse skill");
        assert_eq!(skill.frontmatter.allowed_tools, ["agena.fs.read"]);
        assert!(skill.frontmatter.user_invocable);
        assert_eq!(skill.frontmatter.dependencies.mcp, ["docs"]);
        assert_eq!(skill.content_hash().len(), 64);
    }

    #[test]
    fn resource_reader_rejects_parent_traversal() {
        let dir = tempfile::tempdir().expect("temp dir");
        let skill_dir = dir.path().join("demo");
        std::fs::create_dir(&skill_dir).expect("skill dir");
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_path, "---\nname: demo\n---\nDemo").expect("skill file");
        std::fs::write(skill_dir.join("reference.md"), "reference").expect("resource");
        std::fs::write(dir.path().join("secret.txt"), "secret").expect("secret");
        let skill = Skill::from_path(skill_path).expect("load skill");
        assert_eq!(
            skill
                .read_text_resource(Path::new("reference.md"), 1024)
                .expect("read resource"),
            "reference"
        );
        assert!(matches!(
            skill.read_text_resource(Path::new("../secret.txt"), 1024),
            Err(SkillError::InvalidResourcePath(_))
        ));
    }
}
