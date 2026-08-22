//! Parsed skill model (frontmatter + body).

use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{SkillError, SkillResult};

pub const MAX_SKILL_DOCUMENT_BYTES: usize = 1_048_576;

pub fn read_skill_document(path: impl AsRef<Path>) -> SkillResult<String> {
    read_text_file_bounded(path.as_ref(), MAX_SKILL_DOCUMENT_BYTES)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
/// Frontmatter metadata of a skill.
pub struct SkillFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Optional aliases used to resolve the skill (e.g. slash command
    /// short names).
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
/// A discovered skill.
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
        let raw = read_skill_document(path)?;
        let mut skill = Self::from_raw(&raw)?;
        skill.source_path = Some(path.to_path_buf());
        Ok(skill)
    }

    /// Parse a user command markdown file from disk.
    pub fn from_command_path(path: impl AsRef<Path>) -> SkillResult<Self> {
        let path = path.as_ref();
        let raw = read_skill_document(path)?;
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

    /// Stable content identity used by catalogs, message snapshots and stale
    /// resource checks. Paths are deliberately excluded so moving an unchanged
    /// Skill does not alter its content identity.
    pub fn content_hash(&self) -> String {
        let mut digest = Sha256::new();
        let frontmatter = match serde_yaml::to_string(&self.frontmatter) {
            Ok(frontmatter) => frontmatter,
            Err(error) => {
                tracing::error!(
                    skill_name = %self.frontmatter.name,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize Skill frontmatter for content identity",
                        &error,
                    ),
                    "Skill content identity will include an explicit serialization-failure marker"
                );
                format!("<frontmatter serialization failed: {error}>")
            }
        };
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
        read_text_file_bounded(&canonical_candidate, max_bytes).map_err(|error| match error {
            SkillError::ResourceTooLarge { limit, .. } => SkillError::ResourceTooLarge {
                path: relative.display().to_string(),
                limit,
            },
            SkillError::ResourceNotText { source, .. } => SkillError::ResourceNotText {
                path: relative.display().to_string(),
                source,
            },
            other => other,
        })
    }
}

fn read_text_file_bounded(path: &Path, max_bytes: usize) -> SkillResult<String> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let capacity = usize::try_from(metadata.len().min(max_bytes as u64)).unwrap_or(max_bytes);
    let mut bytes = Vec::with_capacity(capacity);
    file.take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(SkillError::ResourceTooLarge {
            path: path.display().to_string(),
            limit: max_bytes,
        });
    }
    String::from_utf8(bytes).map_err(|source| SkillError::ResourceNotText {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_text_metadata_and_hashes_content() {
        let skill = Skill::from_raw(
            "---\nname: verify\ndescription: Verify a change\naliases: [check]\n---\nCheck it.\n",
        )
        .expect("parse skill");
        assert_eq!(skill.frontmatter.description, "Verify a change");
        assert_eq!(skill.frontmatter.aliases, ["check"]);
        assert_eq!(skill.content_hash().len(), 64);
    }

    #[test]
    fn rejects_removed_activation_frontmatter() {
        let error =
            Skill::from_raw("---\nname: legacy\nallowed-tools: [agena.fs.read]\n---\nLegacy.\n")
                .expect_err("activation metadata is no longer part of a plain-text Skill");
        assert!(error.to_string().contains("allowed-tools"));
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
