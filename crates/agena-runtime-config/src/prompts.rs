//! Prompt layer configuration and fragment files.
//!
//! Fragments live in `prompts/<id>.md` files with YAML front matter. Each
//! configuration layer selects fragments by id, hides inherited ones, and may
//! truncate everything broader.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agena_prompt_layers::{
    FragmentId, LayerMode, MAX_FRAGMENT_BYTES, PromptFragment, PromptLayer, PromptScope,
};

use crate::ConfigError;

/// One scope's prompt layer plus its route-scoped overlays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PromptScopeConfig {
    pub mode: LayerMode,
    pub order: Vec<FragmentId>,
    pub disabled: Vec<FragmentId>,
    pub inline: Vec<PromptFragment>,
    /// Layers that participate only when the request route matches `match`.
    pub models: Vec<ModelPromptConfig>,
}

impl PromptScopeConfig {
    pub fn layer(&self) -> PromptLayer {
        PromptLayer {
            mode: self.mode,
            order: self.order.clone(),
            disabled: self.disabled.clone(),
            inline: self.inline.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.layer().validate().map_err(|error| error.to_string())?;
        for model in &self.models {
            model.validate()?;
        }
        Ok(())
    }
}

/// A route-scoped layer such as `anthropic/*`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelPromptConfig {
    #[serde(rename = "match")]
    pub match_pattern: String,
    pub mode: LayerMode,
    pub order: Vec<FragmentId>,
    pub disabled: Vec<FragmentId>,
    pub inline: Vec<PromptFragment>,
}

impl ModelPromptConfig {
    pub fn layer(&self) -> PromptLayer {
        PromptLayer {
            mode: self.mode,
            order: self.order.clone(),
            disabled: self.disabled.clone(),
            inline: self.inline.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.match_pattern.trim().is_empty() {
            return Err("model prompt layer requires a non-empty `match` pattern".to_owned());
        }
        self.layer().validate().map_err(|error| error.to_string())
    }
}

/// Both configuration layers, in render order (global first).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PromptLayersConfig {
    pub global: PromptScopeConfig,
    pub workspace: PromptScopeConfig,
    /// Global fragment directory, resolved from the configuration location.
    pub global_dir: PathBuf,
    /// Workspace fragment directory, resolved from the workspace root.
    pub workspace_dir: PathBuf,
}

impl PromptLayersConfig {
    /// Load fragment bodies for one scope; missing directories are empty.
    pub fn library(&self, scope: PromptScope) -> Result<FragmentLibrary, ConfigError> {
        let dir = match scope {
            PromptScope::Global => &self.global_dir,
            PromptScope::Workspace => &self.workspace_dir,
            PromptScope::Model | PromptScope::Session | PromptScope::Request => {
                return Ok(FragmentLibrary::default());
            }
        };
        load_library(dir)
    }

    pub fn scope(&self, scope: PromptScope) -> Option<&PromptScopeConfig> {
        match scope {
            PromptScope::Global => Some(&self.global),
            PromptScope::Workspace => Some(&self.workspace),
            _ => None,
        }
    }
}

/// Fragment bodies keyed by id.
pub type FragmentLibrary = BTreeMap<FragmentId, PromptFragment>;

/// Issues found while reading a fragment directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FragmentIssue {
    pub path: PathBuf,
    pub message: String,
}

/// Read every `*.md` fragment in `dir`. Unreadable or invalid files are
/// reported instead of silently skipped.
pub fn load_library(dir: &Path) -> Result<FragmentLibrary, ConfigError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(ConfigError::ReadFile {
                path: dir.to_path_buf(),
                source: error,
            });
        }
    };
    let mut library = FragmentLibrary::new();
    for entry in entries {
        let entry = entry.map_err(|source| ConfigError::ReadFile {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::ReadFile {
            path: path.clone(),
            source,
        })?;
        let id = match path.file_stem().and_then(|stem| stem.to_str()) {
            Some(stem) => FragmentId::parse(stem)
                .map_err(|error| ConfigError::Validation(format!("{}: {error}", path.display())))?,
            None => continue,
        };
        library.insert(id.clone(), parse_fragment(id, &text, &path)?);
    }
    Ok(library)
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct FragmentFrontMatter {
    title: Option<String>,
    when: Option<agena_prompt_layers::PromptCondition>,
}

/// Parse a fragment file: optional YAML front matter followed by the body.
pub fn parse_fragment(
    id: FragmentId,
    text: &str,
    path: &Path,
) -> Result<PromptFragment, ConfigError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (front_matter, body) = if let Some(rest) = text.strip_prefix("---\n") {
        let Some((front, body)) = rest.split_once("\n---") else {
            return Err(ConfigError::Validation(format!(
                "{}: front matter is missing its closing ---",
                path.display()
            )));
        };
        (Some(front), body.trim_start_matches(['\r', '\n']))
    } else {
        (None, text)
    };
    let meta: FragmentFrontMatter = match front_matter {
        Some(front) => serde_yaml::from_str(front).map_err(|error| {
            ConfigError::Validation(format!(
                "{}: invalid fragment front matter: {error}",
                path.display()
            ))
        })?,
        None => FragmentFrontMatter::default(),
    };
    if body.len() > MAX_FRAGMENT_BYTES {
        return Err(ConfigError::Validation(format!(
            "{}: fragment body is {} bytes; the limit is {MAX_FRAGMENT_BYTES}",
            path.display(),
            body.len()
        )));
    }
    Ok(PromptFragment {
        id,
        title: meta.title.unwrap_or_default(),
        body: body.to_owned(),
        when: meta.when.unwrap_or_default(),
    })
}

/// Render a fragment file, preserving front matter for title and conditions.
pub fn render_fragment(fragment: &PromptFragment) -> String {
    if fragment.title.is_empty() && fragment.when.is_empty() {
        return fragment.body.clone();
    }
    let front = serde_yaml::to_string(&FragmentFrontMatter {
        title: (!fragment.title.is_empty()).then(|| fragment.title.clone()),
        when: (!fragment.when.is_empty()).then(|| fragment.when.clone()),
    })
    .unwrap_or_default();
    format!("---\n{}---\n{}", front, fragment.body)
}

/// Create or replace one fragment file. Callers own path permissions.
pub fn write_fragment(dir: &Path, fragment: &PromptFragment) -> Result<PathBuf, ConfigError> {
    std::fs::create_dir_all(dir).map_err(|source| ConfigError::ReadFile {
        path: dir.to_path_buf(),
        source,
    })?;
    let path = dir.join(format!("{}.md", fragment.id));
    if fragment.body.len() > MAX_FRAGMENT_BYTES {
        return Err(ConfigError::Validation(format!(
            "{}: fragment body is {} bytes; the limit is {MAX_FRAGMENT_BYTES}",
            path.display(),
            fragment.body.len()
        )));
    }
    std::fs::write(&path, render_fragment(fragment)).map_err(|source| ConfigError::ReadFile {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Remove one fragment file; a missing file is not an error.
pub fn delete_fragment(dir: &Path, id: &FragmentId) -> Result<(), ConfigError> {
    let path = dir.join(format!("{id}.md"));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ConfigError::ReadFile { path, source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> FragmentId {
        FragmentId::parse(name).unwrap()
    }

    #[test]
    fn front_matter_carries_title_and_conditions() {
        let fragment = parse_fragment(
            id("style"),
            "---\ntitle: House style\nwhen:\n  models: [\"anthropic/*\"]\n  requires_tools: true\n---\nKeep answers short.\n",
            Path::new("style.md"),
        )
        .unwrap();
        assert_eq!(fragment.title, "House style");
        assert_eq!(fragment.body, "Keep answers short.\n");
        assert_eq!(fragment.when.models, vec!["anthropic/*".to_owned()]);
        assert_eq!(fragment.when.requires_tools, Some(true));
    }

    #[test]
    fn plain_markdown_is_a_fragment_body() {
        let fragment = parse_fragment(id("plain"), "Just text.\n", Path::new("plain.md")).unwrap();
        assert!(fragment.title.is_empty());
        assert_eq!(fragment.body, "Just text.\n");
    }

    #[test]
    fn rendered_fragments_round_trip() {
        let fragment = PromptFragment {
            id: id("round"),
            title: "Round trip".to_owned(),
            body: "Body\n".to_owned(),
            when: agena_prompt_layers::PromptCondition {
                models: vec!["local/*".to_owned()],
                requires_tools: None,
            },
        };
        let text = render_fragment(&fragment);
        let parsed = parse_fragment(id("round"), &text, Path::new("round.md")).unwrap();
        assert_eq!(parsed.title, fragment.title);
        assert_eq!(parsed.body, fragment.body);
        assert_eq!(parsed.when.models, fragment.when.models);
    }

    #[test]
    fn invalid_front_matter_is_rejected_with_the_path() {
        let error =
            parse_fragment(id("bad"), "---\ntitle: [\n---\nBody", Path::new("bad.md")).unwrap_err();
        assert!(error.to_string().contains("bad.md"));
    }

    #[test]
    fn library_round_trips_through_the_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let fragment = PromptFragment::new(id("disk"), "On disk.\n");
        write_fragment(dir.path(), &fragment).unwrap();
        let library = load_library(dir.path()).unwrap();
        assert_eq!(library.get(&id("disk")).unwrap().body, "On disk.\n");
        delete_fragment(dir.path(), &id("disk")).unwrap();
        assert!(load_library(dir.path()).unwrap().is_empty());
    }
}
