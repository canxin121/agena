use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    Global,
    #[default]
    Workspace,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub license: Option<String>,
    pub compatibility: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub allowed_tools: Option<String>,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillSource {
    pub kind: String,
    pub location: String,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub subpath: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub scope: SkillScope,
    pub path: String,
    pub editable: bool,
    pub enabled: bool,
    pub valid: bool,
    pub shadowed: bool,
    pub revision: String,
    pub source: Option<SkillSource>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillDiagnostic {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillRoot {
    pub path: String,
    pub scope: SkillScope,
    pub editable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillCatalog {
    pub skills: Vec<SkillSummary>,
    pub diagnostics: Vec<SkillDiagnostic>,
    pub roots: Vec<SkillRoot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillDetail {
    #[serde(flatten)]
    pub summary: SkillSummary,
    pub document: String,
    pub metadata: Option<SkillMetadata>,
    pub files: Vec<SkillFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillResource {
    pub path: String,
    pub content: String,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillValidation {
    pub valid: bool,
    pub metadata: Option<SkillMetadata>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillInstallCandidate {
    pub path: String,
    pub name: String,
    pub description: String,
    pub valid: bool,
    pub diagnostics: Vec<String>,
    pub conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillInstallPreview {
    pub revision: String,
    pub candidates: Vec<SkillInstallCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkillInstallResult {
    pub installed: Vec<SkillSummary>,
}

/// A trusted declaration supplied by an already loaded plugin manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeclaredSkill {
    pub plugin_id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub aliases: Vec<String>,
}
