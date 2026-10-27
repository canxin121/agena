use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{MAX_DOCUMENT_BYTES, Result, SkillError, SkillMetadata, SkillValidation};

#[derive(Debug, Clone)]
pub struct ParsedSkill {
    pub metadata: SkillMetadata,
    pub body: String,
    /// Semantic problems are retained so older packages remain inspectable.
    pub diagnostics: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Frontmatter {
    name: String,
    description: String,
    license: Option<String>,
    compatibility: Option<String>,
    metadata: BTreeMap<String, String>,
    #[serde(rename = "allowed-tools")]
    allowed_tools: Option<String>,
    #[serde(deserialize_with = "deserialize_aliases")]
    aliases: Vec<String>,
}

fn deserialize_aliases<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Aliases {
        Single(String),
        Multiple(Vec<String>),
    }
    Ok(match Option::<Aliases>::deserialize(deserializer)? {
        None => Vec::new(),
        Some(Aliases::Single(value)) => vec![value],
        Some(Aliases::Multiple(values)) => values,
    })
}

/// Parse complete YAML frontmatter, including folded and literal block strings.
/// Unknown extension keys remain in the original document and are not rejected.
pub fn parse_document(document: &str, expected_name: Option<&str>) -> Result<ParsedSkill> {
    if document.len() > MAX_DOCUMENT_BYTES {
        return Err(SkillError::too_large(
            "SKILL.md exceeds the 1 MiB document limit",
        ));
    }
    let source = document.strip_prefix('\u{feff}').unwrap_or(document);
    let first_end = source.find('\n').ok_or_else(|| {
        SkillError::invalid("SKILL.md must begin with YAML frontmatter delimited by ---")
    })?;
    if source[..first_end].trim_end_matches('\r') != "---" {
        return Err(SkillError::invalid(
            "SKILL.md must begin with YAML frontmatter delimited by ---",
        ));
    }
    let start = first_end + 1;
    let mut offset = start;
    let mut boundary = None;
    for line in source[start..].split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            boundary = Some((offset, offset + line.len()));
            break;
        }
        offset += line.len();
    }
    let (end, body_start) = boundary
        .ok_or_else(|| SkillError::invalid("YAML frontmatter is missing its closing ---"))?;
    let raw: Frontmatter = serde_yaml::from_str(&source[start..end])
        .map_err(|error| SkillError::invalid(format!("Invalid skill frontmatter: {error}")))?;
    let metadata = SkillMetadata {
        name: raw.name,
        description: raw.description,
        license: raw.license,
        compatibility: raw.compatibility,
        metadata: raw.metadata,
        allowed_tools: raw.allowed_tools,
        aliases: raw.aliases,
    };
    let mut diagnostics = Vec::new();
    if let Err(error) = validate_name(&metadata.name) {
        diagnostics.push(error.message);
    }
    if let Some(expected_name) = expected_name
        && metadata.name != expected_name
    {
        diagnostics.push(format!(
            "Skill name '{}' must match its directory name '{expected_name}'",
            metadata.name
        ));
    }
    if metadata.description.trim().is_empty() || metadata.description.chars().count() > 1024 {
        diagnostics.push(
            "description must contain 1–1024 characters and explain when to use the skill".into(),
        );
    }
    if let Some(compatibility) = &metadata.compatibility
        && (compatibility.trim().is_empty() || compatibility.chars().count() > 500)
    {
        diagnostics.push("compatibility must contain 1–500 characters when provided".into());
    }
    Ok(ParsedSkill {
        metadata,
        body: source[body_start..]
            .trim_start_matches(['\r', '\n'])
            .to_owned(),
        diagnostics,
    })
}

pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.chars().count() > 64
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name
            .chars()
            .all(|character| character == '-' || character.is_lowercase() || character.is_numeric())
    {
        return Err(SkillError::invalid(
            "name must contain 1–64 lowercase letters, numbers, or hyphens, without leading, trailing, or consecutive hyphens",
        ));
    }
    Ok(())
}

pub fn validate_document(document: &str, expected_name: Option<&str>) -> SkillValidation {
    match parse_document(document, expected_name) {
        Ok(parsed) => {
            let mut warnings = Vec::new();
            if parsed.body.lines().count() > 500 {
                warnings.push(
                    "Keep SKILL.md focused; move detailed material into references/ when practical"
                        .into(),
                );
            }
            if parsed.body.trim().is_empty() {
                warnings.push("The skill has no instruction body".into());
            }
            SkillValidation {
                valid: parsed.diagnostics.is_empty(),
                metadata: Some(parsed.metadata),
                errors: parsed.diagnostics,
                warnings,
            }
        }
        Err(error) => SkillValidation {
            valid: false,
            metadata: None,
            errors: vec![error.message],
            warnings: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_standard_yaml_and_preserves_extensions_in_document() {
        let document = "\u{feff}---\r\nname: data-review\r\ndescription: >-\r\n  Review data quality.\r\n  Use for source checks.\r\nlicense: MIT\r\ncompatibility: Python 3\r\nmetadata:\r\n  author: Example\r\n  version: \"1\"\r\nallowed-tools: Read Bash(python:*)\r\naliases: [review-data]\r\ncustom-extension:\r\n  visible: true\r\n---\r\n# Review\r\nKeep sources.\r\n";
        let parsed = parse_document(document, Some("data-review")).unwrap();
        assert_eq!(
            parsed.metadata.description,
            "Review data quality. Use for source checks."
        );
        assert_eq!(parsed.metadata.metadata["version"], "1");
        assert_eq!(
            parsed.metadata.allowed_tools.as_deref(),
            Some("Read Bash(python:*)")
        );
        assert_eq!(parsed.metadata.aliases, ["review-data"]);
        assert!(parsed.diagnostics.is_empty());
        assert!(parsed.body.starts_with("# Review"));
    }

    #[test]
    fn validation_distinguishes_semantic_problems_from_yaml_errors() {
        let legacy = "---\nname: legacy_name\n---\nLegacy body.";
        let parsed = parse_document(legacy, Some("legacy-name")).unwrap();
        assert_eq!(parsed.diagnostics.len(), 3);
        assert!(!validate_document(legacy, None).valid);
        assert!(parse_document("---\nname: [invalid]\n---\n", None).is_err());
        assert!(parse_document("---\nname: okay\n--broken\n", None).is_err());
        for name in ["", "-skill", "skill-", "two--parts", "Upper", "../escape"] {
            assert!(validate_name(name).is_err(), "{name}");
        }
        assert!(validate_name("data-review2").is_ok());
    }

    #[test]
    fn delimiter_must_be_a_complete_line() {
        let parsed = parse_document(
            "---\nname: valid\ndescription: |\n  ---embedded is ordinary text.\n---\nBody",
            Some("valid"),
        )
        .unwrap();
        assert_eq!(parsed.body, "Body");
        assert!(parsed.metadata.description.contains("---embedded"));
    }
}
