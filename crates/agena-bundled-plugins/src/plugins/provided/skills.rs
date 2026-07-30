//! `agena.skills` — discovery plus workspace-local management for bundled,
//! user and workspace Skill packages.
//!
//! Packaged skills from `agena-skills` are also projected here so a fresh
//! install has reusable instruction packages before any user-defined content
//! exists. Skills are plain text: callers discover them with `list`, read the
//! exact instructions with `get`, and apply those instructions to the current
//! task. Workspace-owned Skills in `.agena/skills` can additionally be
//! created, updated, and deleted through this plugin. The plugin deliberately
//! owns no session activation state.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::{collections::BTreeMap, fmt};

use agena_macros::ToolInput;
use agena_skills::discovery::{
    DiscoveryDiagnostic, default_command_roots, default_roots, scan_commands_with_diagnostics,
    scan_with_diagnostics,
};
use agena_skills::skill::{Skill, SkillFrontmatter};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{
    HostClient, InitContext, InitOutcome, Result as SdkResult, ToolDefinitionInput,
    ToolDefinitionPatch, ToolInvokeOutput,
};

pub(crate) const SKILLS_PLUGIN_ID: &str = "agena.skills";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveredToolKind {
    Skill,
    Command,
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::{
        HookSubscription, InitContext, NoopHostClient, Plugin, PluginKey, PluginSkillDefinition,
    };
    use std::sync::Arc;

    use super::{
        SkillsCreateInput, SkillsDeleteInput, SkillsPlugin, SkillsRefreshInput, SkillsUpdateInput,
    };

    async fn init_test_plugin(plugin: &SkillsPlugin, workspace_root: &std::path::Path) {
        init_test_plugin_with_config(plugin, workspace_root, serde_json::Value::Null).await;
    }

    async fn init_test_plugin_with_config(
        plugin: &SkillsPlugin,
        workspace_root: &std::path::Path,
        config: serde_json::Value,
    ) {
        plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace_root.to_path_buf(),
                    plugin_id: PluginKey::new("agena", "skills").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    config,
                    protocol_version: 1,
                },
                Arc::new(NoopHostClient),
            )
            .await
            .expect("init plugin");
    }

    #[test]
    fn manifest_exposes_plain_text_catalog_tools_without_activation_hooks() {
        let manifest = SkillsPlugin::new().manifest();
        let tool_names = manifest
            .tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            tool_names,
            [
                "list",
                "get",
                "create",
                "update",
                "delete",
                "read_resource",
                "refresh"
            ]
        );
        assert!(manifest.hooks.contains(HookSubscription::TOOL_DEFINITION));
        assert!(
            !manifest
                .hooks
                .contains(HookSubscription::CHAT_SYSTEM_TRANSFORM)
        );
        assert!(!manifest.hooks.contains(HookSubscription::TOOL_BEFORE));
        assert!(
            !manifest
                .hooks
                .contains(HookSubscription::USER_PROMPT_SUBMIT)
        );
    }

    #[test]
    fn bundled_aliases_resolve_to_canonical_names() {
        let dir = tempfile::tempdir().expect("temp dir");
        let catalog = SkillsPlugin::discovered_tools_for_workspace(dir.path());
        let (name, _) =
            SkillsPlugin::resolve_tool(&catalog.tools, "bootstrap").expect("resolve bundled alias");
        assert_eq!(name, "init");
    }

    #[test]
    fn watcher_uses_recursive_roots_and_nonrecursive_existing_parents() {
        let workspace = tempfile::tempdir().expect("workspace");
        let existing = workspace.path().join("skills");
        std::fs::create_dir_all(&existing).expect("existing root");
        let (recursive, recursive_mode) = super::watcher_target(existing.as_path());
        assert_eq!(recursive, existing);
        assert!(matches!(recursive_mode, notify::RecursiveMode::Recursive));

        let missing = workspace.path().join(".agena/skills");
        let (ancestor, ancestor_mode) = super::watcher_target(missing.as_path());
        assert_eq!(ancestor, workspace.path());
        assert!(matches!(ancestor_mode, notify::RecursiveMode::NonRecursive));
    }

    #[tokio::test]
    async fn skills_config_filters_disabled_names_and_accepts_extra_workspace_roots() {
        let workspace = tempfile::tempdir().expect("workspace");
        let disabled_dir = workspace.path().join(".agena/skills/disabled-skill");
        let extra_dir = workspace.path().join("contributed-skills/extra-skill");
        std::fs::create_dir_all(&disabled_dir).expect("disabled skill dir");
        std::fs::create_dir_all(&extra_dir).expect("extra skill dir");
        std::fs::write(
            disabled_dir.join("SKILL.md"),
            "---\nname: disabled_skill\ndescription: should be hidden\n---\nDisabled.",
        )
        .expect("disabled skill");
        std::fs::write(
            extra_dir.join("SKILL.md"),
            "---\nname: extra_skill\ndescription: explicit workspace root\n---\nExtra.",
        )
        .expect("extra skill");

        let plugin = SkillsPlugin::new();
        init_test_plugin_with_config(
            &plugin,
            workspace.path(),
            serde_json::json!({
                "disabled": ["disabled_skill"],
                "additional_roots": ["contributed-skills"],
            }),
        )
        .await;

        let listed = plugin
            .invoke_list(&super::SkillsListInput {
                offset: None,
                limit: None,
                kind: Some("skill".to_string()),
                verbose: false,
            })
            .await
            .expect("list configured catalog");
        let tool_names = listed
            .payload
            .as_ref()
            .and_then(|payload| payload.get("tools"))
            .and_then(serde_json::Value::as_array)
            .expect("tools payload")
            .iter()
            .filter_map(|tool| tool.get("name").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"extra_skill"));
        assert!(!tool_names.contains(&"disabled_skill"));
        assert!(
            plugin
                .invoke_get(&super::SkillsGetInput {
                    name: "disabled_skill".to_string(),
                })
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn refresh_reports_deterministic_catalog_generations() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = SkillsPlugin::new();
        init_test_plugin(&plugin, workspace.path()).await;

        let first = plugin
            .invoke_refresh(&SkillsRefreshInput::default())
            .await
            .expect("initial refresh");
        let first_payload = first.payload.expect("initial refresh payload");
        assert_eq!(first_payload["changed"], true);
        assert_eq!(first_payload["generation"], 1);

        let unchanged = plugin
            .invoke_refresh(&SkillsRefreshInput::default())
            .await
            .expect("unchanged refresh");
        let unchanged_payload = unchanged.payload.expect("unchanged refresh payload");
        assert_eq!(unchanged_payload["changed"], false);
        assert_eq!(unchanged_payload["generation"], 1);

        let added_dir = workspace.path().join(".agena/skills/refreshed-skill");
        std::fs::create_dir_all(&added_dir).expect("added skill dir");
        std::fs::write(
            added_dir.join("SKILL.md"),
            "---\nname: refreshed_skill\ndescription: refresh test\n---\nFresh.",
        )
        .expect("added skill");
        let changed = plugin
            .invoke_refresh(&SkillsRefreshInput::default())
            .await
            .expect("changed refresh");
        let changed_payload = changed.payload.expect("changed refresh payload");
        assert_eq!(changed_payload["changed"], true);
        assert_eq!(changed_payload["generation"], 2);
        plugin
            .invoke_get(&super::SkillsGetInput {
                name: "refreshed_skill".to_string(),
            })
            .await
            .expect("newly discovered skill");
    }

    #[tokio::test]
    async fn workspace_managed_skills_support_create_read_update_and_delete() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = SkillsPlugin::new();
        init_test_plugin(&plugin, workspace.path()).await;

        let created = plugin
            .invoke_create(&SkillsCreateInput {
                document: "---\nname: team_review\ndescription: Review a proposed change\naliases: [review-team]\n---\nReview the change carefully.\n".to_string(),
            })
            .await
            .expect("create managed skill");
        assert_eq!(
            created
                .payload
                .as_ref()
                .and_then(|value| value.get("operation"))
                .and_then(serde_json::Value::as_str),
            Some("created")
        );

        let listed = plugin
            .invoke_list(&super::SkillsListInput {
                offset: None,
                limit: None,
                kind: Some("skill".to_string()),
                verbose: false,
            })
            .await
            .expect("list skills");
        let entry = listed
            .payload
            .as_ref()
            .and_then(|value| value.get("tools"))
            .and_then(serde_json::Value::as_array)
            .and_then(|tools| {
                tools.iter().find(|tool| {
                    tool.get("name").and_then(serde_json::Value::as_str) == Some("team_review")
                })
            })
            .expect("created skill entry");
        assert_eq!(
            entry.get("editable").and_then(serde_json::Value::as_bool),
            Some(true)
        );

        let loaded = plugin
            .invoke_get(&super::SkillsGetInput {
                name: "review-team".to_string(),
            })
            .await
            .expect("read by alias");
        assert!(
            loaded
                .payload
                .as_ref()
                .and_then(|value| value.get("document"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|document| document.contains("Review the change carefully."))
        );

        plugin
            .invoke_update(&SkillsUpdateInput {
                name: "team_review".to_string(),
                document: "---\nname: team_review\ndescription: Review a proposed change\naliases: [review-team]\n---\nReview code, tests, and documentation.\n".to_string(),
            })
            .await
            .expect("update managed skill");
        let updated = plugin
            .invoke_get(&super::SkillsGetInput {
                name: "team_review".to_string(),
            })
            .await
            .expect("read updated skill");
        assert_eq!(
            updated
                .payload
                .as_ref()
                .and_then(|value| value.get("body"))
                .and_then(serde_json::Value::as_str),
            Some("Review code, tests, and documentation.")
        );

        plugin
            .invoke_delete(&SkillsDeleteInput {
                name: "review-team".to_string(),
            })
            .await
            .expect("delete by alias");
        assert!(
            plugin
                .invoke_get(&super::SkillsGetInput {
                    name: "team_review".to_string(),
                })
                .await
                .is_err()
        );
        assert!(
            !workspace
                .path()
                .join(".agena/skills/team_review/SKILL.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn managed_skill_updates_cannot_rewrite_read_only_catalog_entries() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = SkillsPlugin::new();
        init_test_plugin(&plugin, workspace.path()).await;

        let error = plugin
            .invoke_update(&SkillsUpdateInput {
                name: "review".to_string(),
                document: "---\nname: review\n---\nAttempt to replace a bundled Skill.\n"
                    .to_string(),
            })
            .await
            .expect_err("bundled Skill must remain read-only");
        assert!(error.to_string().contains("does not exist"));
    }

    #[tokio::test]
    async fn skills_config_rejects_workspace_escape_roots() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin = SkillsPlugin::new();
        let result = plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace.path().to_path_buf(),
                    plugin_id: PluginKey::new("agena", "skills").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::json!({ "additional_roots": ["../outside"] }),
                    protocol_version: 1,
                },
                Arc::new(NoopHostClient),
            )
            .await;
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skills_config_rejects_existing_roots_that_symlink_outside_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        let link = workspace.path().join("linked-skills");
        std::os::unix::fs::symlink(outside.path(), &link).expect("create outside symlink");
        let plugin = SkillsPlugin::new();
        let result = plugin
            .init(
                InitContext {
                    agena_version: "test".to_string(),
                    workspace_root: workspace.path().to_path_buf(),
                    plugin_id: PluginKey::new("agena", "skills").expect("plugin key"),
                    host_callback_url: None,
                    host_callback_token: None,
                    config: serde_json::json!({ "additional_roots": ["linked-skills"] }),
                    protocol_version: 1,
                },
                Arc::new(NoopHostClient),
            )
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn plugin_manifest_skills_are_non_bundled_catalog_entries_without_resources() {
        let catalog = SkillsPlugin::plugin_contributed_tools_from_manifests(vec![(
            "example.docs".to_string(),
            "1.2.3".to_string(),
            vec![PluginSkillDefinition {
                name: "plugin_docs".to_string(),
                description: "Use the plugin documentation workflow.".to_string(),
                instructions: "Read the plugin documentation first.".to_string(),
                aliases: vec!["docs-plugin".to_string()],
            }],
        )]);
        let tool = catalog
            .get("plugin_docs")
            .expect("plugin skill contribution");
        assert_eq!(tool.kind, super::DiscoveredToolKind::Skill);
        assert_eq!(tool.skill.frontmatter.aliases, ["docs-plugin"]);
        assert_eq!(tool.skill.body, "Read the plugin documentation first.");
        assert_eq!(tool.origin.source_label(), "plugin:example.docs@1.2.3");
        assert!(!tool.origin.supports_resources());
    }
}

impl AsRef<str> for DiscoveredToolKind {
    fn as_ref(&self) -> &str {
        match self {
            Self::Skill => "skill",
            Self::Command => "command",
        }
    }
}

impl fmt::Display for DiscoveredToolKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

fn watcher_target(root: &Path) -> (PathBuf, RecursiveMode) {
    if root.is_dir() {
        return (root.to_path_buf(), RecursiveMode::Recursive);
    }
    let mut ancestor = root.to_path_buf();
    while !ancestor.is_dir() {
        if !ancestor.pop() {
            return (PathBuf::from("."), RecursiveMode::NonRecursive);
        }
    }
    (ancestor, RecursiveMode::NonRecursive)
}

#[derive(Debug, Clone)]
struct DiscoveredTool {
    skill: Skill,
    kind: DiscoveredToolKind,
    origin: SkillOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SkillOrigin {
    Bundled,
    Filesystem,
    Plugin {
        plugin_id: String,
        plugin_version: String,
    },
}

impl SkillOrigin {
    fn source_label(&self) -> String {
        match self {
            Self::Bundled => "bundled".to_string(),
            Self::Filesystem => "filesystem".to_string(),
            Self::Plugin {
                plugin_id,
                plugin_version,
            } => format!("plugin:{plugin_id}@{plugin_version}"),
        }
    }

    fn supports_resources(&self) -> bool {
        matches!(self, Self::Filesystem)
    }
}

#[derive(Debug, Clone, Default)]
struct DiscoveredCatalog {
    tools: BTreeMap<String, DiscoveredTool>,
    diagnostics: Vec<DiscoveryDiagnostic>,
}

/// Plugin-owned policy for filesystem-backed Skills. The regular roots remain
/// enabled by default; extra roots are deliberately workspace-relative so a
/// project configuration cannot silently turn an arbitrary user directory
/// into model-visible prompt content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
struct SkillsPluginConfig {
    /// Canonical Skill/command names hidden from discovery.
    disabled: Vec<String>,
    /// Extra workspace-relative directories containing `SKILL.md` packages.
    additional_roots: Vec<PathBuf>,
    /// Extra workspace-relative directories containing slash-command `.md`
    /// files.
    additional_command_roots: Vec<PathBuf>,
    /// Cross-platform OS watcher used only to invalidate the filesystem
    /// catalog. Discovery still happens at a normal request boundary; watcher
    /// events never inject instructions on their own.
    watcher: SkillsWatcherConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct SkillsWatcherConfig {
    enabled: bool,
}

impl Default for SkillsWatcherConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl SkillsPluginConfig {
    fn validate_for_workspace(&self, workspace_root: &Path) -> SdkResult<()> {
        let canonical_workspace = workspace_root.canonicalize().map_err(|error| {
            PluginError::invalid_params(format!(
                "skills config cannot canonicalize workspace '{}': {error}",
                workspace_root.display()
            ))
        })?;
        let mut seen = std::collections::BTreeSet::new();
        for name in &self.disabled {
            let normalized = name.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                return Err(PluginError::invalid_params(
                    "skills config disabled entries cannot be blank",
                ));
            }
            if !seen.insert(normalized) {
                return Err(PluginError::invalid_params(format!(
                    "skills config lists the same disabled name more than once: '{name}'"
                )));
            }
        }
        for (label, roots) in [
            ("additional_roots", &self.additional_roots),
            ("additional_command_roots", &self.additional_command_roots),
        ] {
            for root in roots {
                if root.as_os_str().is_empty()
                    || root.is_absolute()
                    || root
                        .components()
                        .any(|component| matches!(component, std::path::Component::ParentDir))
                {
                    return Err(PluginError::invalid_params(format!(
                        "skills config {label} entries must be non-empty workspace-relative paths without '..'"
                    )));
                }
                let candidate = workspace_root.join(root);
                if !candidate.starts_with(workspace_root) {
                    return Err(PluginError::invalid_params(format!(
                        "skills config {label} entry '{}' escapes the workspace",
                        root.display()
                    )));
                }
                // A lexical workspace-relative path may still resolve through
                // a symlink outside the workspace. Existing roots are
                // canonicalized at config-load time so that cannot silently
                // turn an arbitrary user directory into model-visible Skill
                // instructions. A non-existent root is harmless until a
                // later config reload validates it again.
                if candidate.exists() {
                    let canonical_candidate = candidate.canonicalize().map_err(|error| {
                        PluginError::invalid_params(format!(
                            "skills config cannot canonicalize {label} entry '{}': {error}",
                            root.display()
                        ))
                    })?;
                    if !canonical_candidate.starts_with(&canonical_workspace) {
                        return Err(PluginError::invalid_params(format!(
                            "skills config {label} entry '{}' resolves outside the workspace",
                            root.display()
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn disabled_name(&self, name: &str) -> bool {
        self.disabled
            .iter()
            .any(|disabled| disabled.trim().eq_ignore_ascii_case(name.trim()))
    }
}

#[derive(Debug, Clone)]
struct CatalogRefresh {
    catalog: DiscoveredCatalog,
    fingerprint: String,
    generation: u64,
    changed: bool,
}

#[derive(Debug, Clone, Default)]
struct CatalogState {
    fingerprint: Option<String>,
    generation: u64,
}

const MAX_SKILL_DOCUMENT_BYTES: usize = 1_048_576;

#[derive(Debug)]
struct SkillWriteResult {
    name: String,
    path: PathBuf,
    operation: &'static str,
}

fn validate_managed_skill_name(name: &str) -> SdkResult<()> {
    let name = name.trim();
    let valid = !name.is_empty()
        && name.len() <= 96
        && name.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'
            )
        })
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(PluginError::invalid_params(
            "Skill names must start with an ASCII letter or digit and contain only letters, digits, '.', '_' or '-' (maximum 96 characters)",
        ))
    }
}

fn enforce_skill_document_size(document: &str) -> SdkResult<()> {
    if document.len() <= MAX_SKILL_DOCUMENT_BYTES {
        Ok(())
    } else {
        Err(PluginError::invalid_params(format!(
            "Skill document exceeds the {} byte limit",
            MAX_SKILL_DOCUMENT_BYTES
        )))
    }
}

fn parse_managed_skill_document(document: &str) -> SdkResult<Skill> {
    enforce_skill_document_size(document)?;
    let skill = Skill::from_raw(document).map_err(|error| {
        PluginError::invalid_params(format!("invalid SKILL.md document: {error}"))
    })?;
    validate_managed_skill_name(skill.frontmatter.name.as_str())?;
    let mut aliases = std::collections::BTreeSet::new();
    for alias in &skill.frontmatter.aliases {
        let alias = alias.trim();
        validate_managed_skill_name(alias)?;
        if !aliases.insert(alias.to_ascii_lowercase()) {
            return Err(PluginError::invalid_params(format!(
                "Skill aliases contain a duplicate value: '{alias}'"
            )));
        }
    }
    Ok(skill)
}

fn write_skill_document(path: &Path, document: &str) -> SdkResult<()> {
    enforce_skill_document_size(document)?;
    let parent = path.parent().ok_or_else(|| {
        PluginError::new("managed Skill path unexpectedly has no parent directory")
    })?;
    let temporary = parent.join(".SKILL.md.agena-tmp");
    if temporary.exists() {
        std::fs::remove_file(&temporary).map_err(skill_write_error)?;
    }
    std::fs::write(&temporary, document).map_err(skill_write_error)?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        skill_write_error(error)
    })
}

fn skill_write_error(error: std::io::Error) -> PluginError {
    PluginError::new(format!(
        "workspace Skill filesystem operation failed: {error}"
    ))
}

fn workspace_managed_skill_path(workspace_root: &Path, name: &str) -> Option<PathBuf> {
    validate_managed_skill_name(name).ok()?;
    let workspace = workspace_root.canonicalize().ok()?;
    let root = workspace.join(".agena/skills");
    let root = root.canonicalize().ok()?;
    root.starts_with(&workspace)
        .then(|| root.join(name).join("SKILL.md"))
}

fn is_workspace_managed_skill(workspace_root: &Path, name: &str, tool: &DiscoveredTool) -> bool {
    let Some(expected) = workspace_managed_skill_path(workspace_root, name) else {
        return false;
    };
    let Some(source) = tool.skill.source_path.as_ref() else {
        return false;
    };
    match (expected.canonicalize(), source.canonicalize()) {
        (Ok(expected), Ok(source)) => expected == source,
        _ => false,
    }
}

fn format_skill_document(skill: &Skill) -> String {
    let frontmatter = serde_yaml::to_string(&skill.frontmatter).unwrap_or_else(|_| {
        format!(
            "name: {}\ndescription: {}\naliases: []\n",
            skill.frontmatter.name, skill.frontmatter.description
        )
    });
    format!("---\n{}---\n{}\n", frontmatter, skill.body.trim())
}

fn skill_write_output(
    result: SkillWriteResult,
    generation: u64,
    catalog_changed: bool,
) -> ToolInvokeOutput {
    let path = result.path.display().to_string();
    let operation = result.operation;
    ToolInvokeOutput::from_parts(
        format!("Skill {operation}: {}", result.name),
        format!(
            "Skill '{}' was {operation} at {path}. Skill catalog generation {generation}.",
            result.name
        ),
        Some(serde_json::json!({
            "name": result.name,
            "path": path,
            "operation": operation,
            "catalog_generation": generation,
            "catalog_changed": catalog_changed,
            "editable": operation != "deleted",
        })),
        BTreeMap::from([
            ("agena.effect".to_string(), "skill_catalog".to_string()),
            ("operation".to_string(), operation.to_string()),
            ("catalog_generation".to_string(), generation.to_string()),
        ]),
        Vec::new(),
    )
}

/// Owns the platform watcher for as long as the static Skills plugin lives.
/// `notify` invokes the callback on its own worker context; an atomic counter
/// is sufficient because events only invalidate the next discovery result and
/// never carry content into the model context.
struct SkillCatalogWatcher {
    _watcher: RecommendedWatcher,
    watched_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct SkillsListInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default)]
    verbose: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("name"), non_empty("name"))]
#[serde(deny_unknown_fields)]
struct SkillsGetInput {
    name: String,
}

/// A complete `SKILL.md` document. Keeping the editor boundary at the native
/// document format lets callers preserve a Skill's YAML frontmatter alongside
/// its Markdown instructions instead of maintaining a second, lossy model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("document"), non_empty("document"))]
#[serde(deny_unknown_fields)]
struct SkillsCreateInput {
    document: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("name", "document"), non_empty("name", "document"))]
#[serde(deny_unknown_fields)]
struct SkillsUpdateInput {
    /// Canonical name (or alias) of the workspace-managed Skill to replace.
    name: String,
    /// Replacement `SKILL.md` document. Its frontmatter name must not change.
    document: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("name"), non_empty("name"))]
#[serde(deny_unknown_fields)]
struct SkillsDeleteInput {
    /// Canonical name (or alias) of the workspace-managed Skill to remove.
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("name", "path"), non_empty("name", "path"))]
#[serde(deny_unknown_fields)]
struct SkillsReadResourceInput {
    name: String,
    path: String,
    #[serde(default = "default_resource_limit")]
    #[schemars(range(min = 1, max = 1048576))]
    max_bytes: u32,
}

const fn default_resource_limit() -> u32 {
    262_144
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput, Default)]
#[serde(default, deny_unknown_fields)]
struct SkillsRefreshInput {
    /// Include discovery diagnostics in the human-readable response.
    verbose: bool,
}

pub(crate) struct SkillsPlugin {
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<SkillsPluginConfig>,
    catalog_state: Mutex<CatalogState>,
    watcher: Mutex<Option<SkillCatalogWatcher>>,
    watcher_generation: Arc<AtomicU64>,
}

fn skills_config_schema() -> serde_json::Value {
    let mut schema = agena_runtime_tools::tool::definition::json_schema_for_default(
        SkillsPluginConfig::default(),
    );
    for (pointer, title, description) in [
        (
            "",
            "Skills Plugin Config",
            "Controls discovery policy for filesystem-backed Skills and slash commands.",
        ),
        (
            "/properties/disabled",
            "Disabled Skills",
            "Canonical Skill or command names to hide from list and get.",
        ),
        (
            "/properties/additional_roots",
            "Additional Skill Roots",
            "Workspace-relative directories scanned after the standard roots.",
        ),
        (
            "/properties/additional_command_roots",
            "Additional Command Roots",
            "Workspace-relative directories scanned after the standard command roots.",
        ),
        (
            "/properties/watcher",
            "Filesystem Watcher",
            "Use the platform filesystem watcher to invalidate the Skill catalog after on-disk changes.",
        ),
        (
            "/properties/watcher/properties/enabled",
            "Enabled",
            "Disable only the OS-level watcher; request-driven discovery remains active for every Skill Tool.",
        ),
    ] {
        agena_runtime_tools::tool::definition::set_schema_metadata(
            &mut schema,
            pointer,
            Some(title),
            Some(description),
        );
    }
    schema
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "skills",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Discover and read plain-text skills and slash commands.",
    config_schema = skills_config_schema(),
    display = brief
)]
impl SkillsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            workspace_root: OnceLock::new(),
            config: OnceLock::new(),
            catalog_state: Mutex::new(CatalogState::default()),
            watcher: Mutex::new(None),
            watcher_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    #[hook(init)]
    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: SkillsPluginConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_config(
                ctx.config,
                "invalid skills plugin config",
            )?;
        config.validate_for_workspace(ctx.workspace_root.as_path())?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::new("skills plugin initialized more than once"))?;
        self.config
            .set(config)
            .map_err(|_| PluginError::new("skills plugin config initialized more than once"))?;
        self.start_filesystem_watcher()?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::new("skills invoked before init"))
    }

    fn config(&self) -> SkillsPluginConfig {
        self.config.get().cloned().unwrap_or_default()
    }

    /// The only mutable Skill location. Discovery intentionally includes
    /// bundled, user and compatibility roots, but mutation must not turn this
    /// plugin into a general filesystem editor or alter another project's
    /// global agent configuration.
    fn managed_root(&self) -> SdkResult<PathBuf> {
        let workspace_root = self.workspace_root()?;
        let canonical_workspace = workspace_root.canonicalize().map_err(|error| {
            PluginError::new(format!(
                "cannot canonicalize Skill workspace '{}': {error}",
                workspace_root.display()
            ))
        })?;
        let root = canonical_workspace.join(".agena/skills");
        std::fs::create_dir_all(&root).map_err(skill_write_error)?;
        let canonical_root = root.canonicalize().map_err(skill_write_error)?;
        if !canonical_root.starts_with(&canonical_workspace) {
            return Err(PluginError::invalid_params(
                "workspace Skill root resolves outside the workspace",
            ));
        }
        Ok(canonical_root)
    }

    fn managed_skill_path(&self, name: &str) -> SdkResult<PathBuf> {
        validate_managed_skill_name(name)?;
        let root = self.managed_root()?;
        Ok(root.join(name).join("SKILL.md"))
    }

    fn managed_skill_document(&self, name: &str) -> SdkResult<(String, PathBuf)> {
        let path = self.managed_skill_path(name)?;
        let parent = path.parent().ok_or_else(|| {
            PluginError::new("managed Skill path unexpectedly has no parent directory")
        })?;
        if !parent.is_dir() || parent.is_symlink() {
            return Err(PluginError::invalid_params(format!(
                "workspace-managed Skill '{name}' does not exist"
            )));
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(skill_write_error)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(PluginError::invalid_params(format!(
                "workspace-managed Skill '{name}' is not a regular SKILL.md file"
            )));
        }
        let canonical_root = self.managed_root()?;
        let canonical_path = path.canonicalize().map_err(skill_write_error)?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(PluginError::invalid_params(
                "workspace-managed Skill resolves outside the workspace",
            ));
        }
        let document = std::fs::read_to_string(&canonical_path).map_err(skill_write_error)?;
        enforce_skill_document_size(document.as_str())?;
        Ok((document, canonical_path))
    }

    fn create_managed_skill(&self, document: &str) -> SdkResult<SkillWriteResult> {
        let skill = parse_managed_skill_document(document)?;
        let name = skill.frontmatter.name;
        let path = self.managed_skill_path(name.as_str())?;
        if path.exists() {
            return Err(PluginError::invalid_params(format!(
                "workspace-managed Skill '{name}' already exists; use update instead"
            )));
        }
        let parent = path.parent().ok_or_else(|| {
            PluginError::new("managed Skill path unexpectedly has no parent directory")
        })?;
        std::fs::create_dir_all(parent).map_err(skill_write_error)?;
        let canonical_root = self.managed_root()?;
        let canonical_parent = parent.canonicalize().map_err(skill_write_error)?;
        if !canonical_parent.starts_with(&canonical_root) || parent.is_symlink() {
            return Err(PluginError::invalid_params(
                "workspace-managed Skill directory resolves outside the workspace",
            ));
        }
        write_skill_document(path.as_path(), document)?;
        Ok(SkillWriteResult {
            name,
            path,
            operation: "created",
        })
    }

    fn update_managed_skill(
        &self,
        requested_name: &str,
        document: &str,
    ) -> SdkResult<SkillWriteResult> {
        let replacement = parse_managed_skill_document(document)?;
        let tools = self.discovered_tools()?;
        let (canonical_name, _) = Self::resolve_tool(&tools, requested_name)?;
        if replacement.frontmatter.name != canonical_name {
            return Err(PluginError::invalid_params(format!(
                "Skill name cannot change during update: expected '{canonical_name}', found '{}'",
                replacement.frontmatter.name
            )));
        }
        let (existing, path) = self.managed_skill_document(canonical_name)?;
        // Parse first so an invalid legacy document cannot be overwritten by
        // accident. The names must remain stable even when an alias was used.
        let existing_skill = parse_managed_skill_document(existing.as_str())?;
        if existing_skill.frontmatter.name != canonical_name {
            return Err(PluginError::invalid_params(format!(
                "workspace-managed Skill file for '{canonical_name}' declares '{}'",
                existing_skill.frontmatter.name
            )));
        }
        write_skill_document(path.as_path(), document)?;
        Ok(SkillWriteResult {
            name: canonical_name.to_owned(),
            path,
            operation: "updated",
        })
    }

    fn delete_managed_skill(&self, requested_name: &str) -> SdkResult<SkillWriteResult> {
        let tools = self.discovered_tools()?;
        let (canonical_name, _) = Self::resolve_tool(&tools, requested_name)?;
        let (_, path) = self.managed_skill_document(canonical_name)?;
        std::fs::remove_file(&path).map_err(skill_write_error)?;
        if let Some(parent) = path.parent()
            && parent
                .read_dir()
                .map_err(skill_write_error)?
                .next()
                .is_none()
        {
            std::fs::remove_dir(parent).map_err(skill_write_error)?;
        }
        Ok(SkillWriteResult {
            name: canonical_name.to_owned(),
            path,
            operation: "deleted",
        })
    }

    fn discovered_catalog(&self) -> SdkResult<DiscoveredCatalog> {
        Ok(self.refresh_catalog()?.catalog)
    }

    fn discovered_tools(&self) -> SdkResult<BTreeMap<String, DiscoveredTool>> {
        Ok(self.discovered_catalog()?.tools)
    }

    #[cfg(test)]
    fn discovered_tools_for_workspace(workspace_root: &Path) -> DiscoveredCatalog {
        Self::discovered_tools_for_workspace_with_config(
            workspace_root,
            &SkillsPluginConfig::default(),
        )
    }

    fn discovered_tools_for_workspace_with_config(
        workspace_root: &Path,
        config: &SkillsPluginConfig,
    ) -> DiscoveredCatalog {
        let (roots, command_roots) = Self::filesystem_roots(workspace_root, config);

        let skill_report = scan_with_diagnostics(&roots);
        let command_report = scan_commands_with_diagnostics(&command_roots);

        let mut tools: BTreeMap<String, DiscoveredTool> = agena_skills::bundled::all()
            .into_iter()
            .map(|skill| {
                let name = skill.frontmatter.name.clone();
                (
                    name,
                    DiscoveredTool {
                        skill,
                        kind: DiscoveredToolKind::Skill,
                        origin: SkillOrigin::Bundled,
                    },
                )
            })
            .collect();
        for (name, tool) in Self::plugin_contributed_tools() {
            tools.insert(name, tool);
        }
        for skill in skill_report.skills {
            let name = skill.frontmatter.name.clone();
            tools.insert(
                name,
                DiscoveredTool {
                    skill,
                    kind: DiscoveredToolKind::Skill,
                    origin: SkillOrigin::Filesystem,
                },
            );
        }
        for command in command_report.skills {
            tools.insert(
                command.frontmatter.name.clone(),
                DiscoveredTool {
                    skill: command,
                    kind: DiscoveredToolKind::Command,
                    origin: SkillOrigin::Filesystem,
                },
            );
        }
        tools.retain(|name, _| !config.disabled_name(name));
        let mut diagnostics = skill_report.diagnostics;
        diagnostics.extend(command_report.diagnostics);
        DiscoveredCatalog { tools, diagnostics }
    }

    fn filesystem_roots(
        workspace_root: &Path,
        config: &SkillsPluginConfig,
    ) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let workspace =
            Some(workspace_root.to_path_buf()).filter(|path| !path.as_os_str().is_empty());
        let mut roots = default_roots(workspace.as_deref());
        let mut command_roots = default_command_roots(workspace.as_deref());
        roots.extend(
            config
                .additional_roots
                .iter()
                .map(|root| workspace_root.join(root)),
        );
        command_roots.extend(
            config
                .additional_command_roots
                .iter()
                .map(|root| workspace_root.join(root)),
        );
        roots.sort();
        roots.dedup();
        command_roots.sort();
        command_roots.dedup();
        (roots, command_roots)
    }

    /// Install a platform watcher over existing catalog roots. For a root
    /// that does not yet exist, watch its nearest existing ancestor without
    /// recursion so creation of `.agena/skills` is still observed without
    /// recursively monitoring an entire workspace. The normal next request
    /// rescans and can then use the newly created root.
    fn start_filesystem_watcher(&self) -> SdkResult<()> {
        if !self.config().watcher.enabled {
            return Ok(());
        }
        let (skill_roots, command_roots) =
            Self::filesystem_roots(self.workspace_root()?, &self.config());
        let mut desired = std::collections::BTreeMap::<PathBuf, RecursiveMode>::new();
        for root in skill_roots.into_iter().chain(command_roots) {
            let (path, mode) = watcher_target(root.as_path());
            desired
                .entry(path)
                .and_modify(|current| {
                    if matches!(mode, RecursiveMode::Recursive) {
                        *current = RecursiveMode::Recursive;
                    }
                })
                .or_insert(mode);
        }

        let generation = Arc::clone(&self.watcher_generation);
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if event.is_ok() {
                    generation.fetch_add(1, Ordering::AcqRel);
                }
            })
            .map_err(|error| {
                PluginError::new(format!("cannot start Skill filesystem watcher: {error}"))
            })?;
        let mut watched_paths = Vec::new();
        for (path, mode) in desired {
            match watcher.watch(path.as_path(), mode) {
                Ok(()) => watched_paths.push(path),
                Err(error) => tracing::warn!(
                    target: "agena_skills::watcher",
                    path = %path.display(),
                    "cannot watch Skill catalog path: {error}"
                ),
            }
        }
        *self
            .watcher
            .lock()
            .map_err(|_| PluginError::new("skills watcher lock poisoned"))? =
            Some(SkillCatalogWatcher {
                _watcher: watcher,
                watched_paths,
            });
        Ok(())
    }

    fn watcher_status(&self) -> SdkResult<(bool, usize, u64)> {
        let watcher = self
            .watcher
            .lock()
            .map_err(|_| PluginError::new("skills watcher lock poisoned"))?;
        Ok((
            self.config().watcher.enabled,
            watcher
                .as_ref()
                .map(|watcher| watcher.watched_paths.len())
                .unwrap_or_default(),
            self.watcher_generation.load(Ordering::Acquire),
        ))
    }

    /// Project declarative Skill contributions from loaded plugin manifests.
    /// Plugin manifests are already authenticated and validated by PluginHost.
    fn plugin_contributed_tools() -> BTreeMap<String, DiscoveredTool> {
        let Some(host) = agena_runtime_plugins::plugin_slot::current_plugin_host() else {
            return BTreeMap::new();
        };
        let mut contributions = host
            .plugins()
            .iter()
            .map(|plugin| {
                (
                    plugin.key().to_string(),
                    plugin.manifest.version.clone(),
                    plugin.manifest.skills.clone(),
                )
            })
            .collect::<Vec<_>>();
        contributions.sort_by(|left, right| left.0.cmp(&right.0));
        Self::plugin_contributed_tools_from_manifests(contributions)
    }

    fn plugin_contributed_tools_from_manifests(
        manifests: impl IntoIterator<
            Item = (
                String,
                String,
                Vec<agena_plugin_host::sdk::PluginSkillDefinition>,
            ),
        >,
    ) -> BTreeMap<String, DiscoveredTool> {
        let mut tools = BTreeMap::new();
        for (plugin_id, plugin_version, skills) in manifests {
            for definition in skills {
                let name = definition.name.clone();
                let skill = Skill {
                    frontmatter: SkillFrontmatter {
                        name: definition.name,
                        description: definition.description,
                        aliases: definition.aliases,
                    },
                    body: definition.instructions,
                    source_path: None,
                };
                tools.insert(
                    name,
                    DiscoveredTool {
                        skill,
                        kind: DiscoveredToolKind::Skill,
                        origin: SkillOrigin::Plugin {
                            plugin_id: plugin_id.clone(),
                            plugin_version: plugin_version.clone(),
                        },
                    },
                );
            }
        }
        tools
    }

    fn catalog_fingerprint(catalog: &DiscoveredCatalog) -> String {
        let mut digest = Sha256::new();
        for (name, tool) in &catalog.tools {
            digest.update(name.as_bytes());
            digest.update([0]);
            digest.update(tool.kind.as_ref().as_bytes());
            digest.update([0]);
            digest.update(tool.origin.source_label().as_bytes());
            digest.update([0]);
            digest.update(tool.skill.content_hash().as_bytes());
            digest.update([0]);
            if let Some(path) = tool.skill.source_path.as_ref() {
                digest.update(path.to_string_lossy().as_bytes());
            }
            digest.update([0xff]);
        }
        for diagnostic in &catalog.diagnostics {
            digest.update(diagnostic.path.to_string_lossy().as_bytes());
            digest.update([0]);
            digest.update(diagnostic.error.as_bytes());
            digest.update([0xfe]);
        }
        hex::encode(digest.finalize())
    }

    /// Rescan on demand instead of keeping stale prompt packages in memory.
    /// This gives every Skills Tool a request-driven hot-reload boundary;
    /// `skills.refresh` surfaces the generation/delta when a caller needs an
    /// explicit audit point.
    fn refresh_catalog(&self) -> SdkResult<CatalogRefresh> {
        let catalog = Self::discovered_tools_for_workspace_with_config(
            self.workspace_root()?,
            &self.config(),
        );
        let fingerprint = Self::catalog_fingerprint(&catalog);
        let mut state = self
            .catalog_state
            .lock()
            .map_err(|_| PluginError::new("skills catalog-state lock poisoned"))?;
        let changed = state.fingerprint.as_deref() != Some(fingerprint.as_str());
        if changed {
            state.generation = state.generation.saturating_add(1);
            state.fingerprint = Some(fingerprint.clone());
        }
        Ok(CatalogRefresh {
            catalog,
            fingerprint,
            generation: state.generation,
            changed,
        })
    }

    fn normalize_kind_filter(kind: Option<&str>) -> SdkResult<Option<DiscoveredToolKind>> {
        match kind.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(None),
            Some("skill") => Ok(Some(DiscoveredToolKind::Skill)),
            Some("command") => Ok(Some(DiscoveredToolKind::Command)),
            Some(other) => Err(PluginError::invalid_params(format!(
                "unknown kind '{other}', expected 'skill' or 'command'"
            ))),
        }
    }

    fn paginate<T>(
        items: Vec<T>,
        offset: Option<u32>,
        limit: Option<u32>,
    ) -> (Vec<T>, usize, usize) {
        let total = items.len();
        let offset = offset.unwrap_or(0) as usize;
        if offset >= total {
            return (Vec::new(), total, offset);
        }
        let limit = limit
            .map(|value| value as usize)
            .unwrap_or(total.saturating_sub(offset));
        let end = offset.saturating_add(limit).min(total);
        let page = items
            .into_iter()
            .skip(offset)
            .take(end.saturating_sub(offset))
            .collect::<Vec<_>>();
        (page, total, offset)
    }

    fn tool_description(discovered_tool: &DiscoveredTool) -> String {
        let category = discovered_tool.kind.as_ref();
        if discovered_tool
            .skill
            .frontmatter
            .description
            .trim()
            .is_empty()
        {
            format!(
                "Read the '{}' {category} instructions.",
                discovered_tool.skill.frontmatter.name
            )
        } else {
            discovered_tool.skill.frontmatter.description.clone()
        }
    }

    fn resolve_tool<'a>(
        tools: &'a BTreeMap<String, DiscoveredTool>,
        requested: &str,
    ) -> SdkResult<(&'a str, &'a DiscoveredTool)> {
        if let Some((name, tool)) = tools.get_key_value(requested) {
            return Ok((name.as_str(), tool));
        }
        let normalized = requested.trim().to_ascii_lowercase();
        tools
            .iter()
            .find(|(_, tool)| tool.skill.matches(normalized.as_str()))
            .map(|(name, tool)| (name.as_str(), tool))
            .ok_or_else(|| PluginError::invalid_params(format!("unknown skill '{}'", requested)))
    }

    #[hook(tool.definition)]
    fn tool_definition_patch(
        &self,
        input: ToolDefinitionInput,
    ) -> SdkResult<Option<ToolDefinitionPatch>> {
        if input.plugin_key().to_string() != SKILLS_PLUGIN_ID {
            return Ok(None);
        }
        let catalog = self.discovered_catalog()?;
        let tools = catalog.tools;
        let skill_count = tools
            .values()
            .filter(|tool| tool.kind == DiscoveredToolKind::Skill)
            .count();
        let command_count = tools
            .values()
            .filter(|tool| tool.kind == DiscoveredToolKind::Command)
            .count();
        let preview = tools
            .iter()
            .take(12)
            .map(|(name, tool)| {
                format!(
                    "- {} [{}]: {}",
                    name,
                    tool.kind,
                    Self::tool_description(tool)
                )
            })
            .collect::<Vec<_>>();
        let mut lines = vec![
            format!("Currently discovered: {skill_count} skill(s), {command_count} command(s)."),
            "Use `agena.skills.list` to page through discovered items.".to_string(),
            "Use `agena.skills.get` to read one item in full, then apply its plain-text body to the current task.".to_string(),
            "Use `agena.skills.read_resource` to read a bounded text resource from a skill package.".to_string(),
            "Use `agena.skills.refresh` to rescan filesystem-backed Skills and inspect the catalog generation.".to_string(),
            "Use `agena.skills.create`, `update`, and `delete` only for workspace-managed `.agena/skills` documents.".to_string(),
            "Skills do not have session activation state, tool restrictions, or model side effects.".to_string(),
        ];
        if !catalog.diagnostics.is_empty() {
            lines.push(format!(
                "Discovery diagnostics: {} invalid or unreadable item(s); call list with verbose=true for details.",
                catalog.diagnostics.len()
            ));
        }
        if !preview.is_empty() {
            lines.push("Preview:".to_string());
            lines.extend(preview);
        }

        let summary = match input.tool_name() {
            "list" => Some(format!(
                "List discovered skills and slash commands. Currently {skill_count} skill(s) and {command_count} command(s)."
            )),
            "get" => Some("Read one discovered skill or slash command.".to_string()),
            "read_resource" => {
                Some("Read a bounded UTF-8 resource contained by one skill package.".to_string())
            }
            "refresh" => Some(
                "Rescan filesystem-backed Skills and report whether the catalog changed."
                    .to_string(),
            ),
            "create" => Some(
                "Create a workspace-managed `.agena/skills/<name>/SKILL.md` document.".to_string(),
            ),
            "update" => Some(
                "Replace one workspace-managed Skill document without changing its canonical name."
                    .to_string(),
            ),
            "delete" => {
                Some("Delete one workspace-managed `.agena/skills` Skill document.".to_string())
            }
            _ => None,
        };

        Ok(Some(ToolDefinitionPatch {
            summary,
            help: Some(lines.join("\n")),
            description_mode: None,
            input_schema: None,
        }))
    }

    #[tool(
        summary = "List discovered skills and slash commands.",
        read_only,
        discovery,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_list(&self, input: &SkillsListInput) -> SdkResult<ToolInvokeOutput> {
        let catalog = self.discovered_catalog()?;
        let workspace_root = self.workspace_root()?;
        let kind_filter = Self::normalize_kind_filter(input.kind.as_deref())?;
        let entries = catalog
            .tools
            .into_iter()
            .filter(|(_, tool)| kind_filter.is_none_or(|kind| tool.kind == kind))
            .collect::<Vec<_>>();
        let (entries, total, offset) = Self::paginate(entries, input.offset, input.limit);
        let mut lines = vec![format!(
            "Discovered skill tool(s): returned {}/{} starting at offset {}.",
            entries.len(),
            total,
            offset
        )];
        for (name, tool) in &entries {
            if input.verbose {
                lines.push(format!(
                    "- {} [{}]: {} ({})",
                    name,
                    tool.kind,
                    Self::tool_description(tool),
                    tool.origin.source_label()
                ));
            } else {
                lines.push(format!("- {} [{}]", name, tool.kind));
            }
        }
        if input.verbose && !catalog.diagnostics.is_empty() {
            lines.push("Discovery diagnostics:".to_string());
            lines.extend(catalog.diagnostics.iter().map(|diagnostic| {
                format!("- {}: {}", diagnostic.path.display(), diagnostic.error)
            }));
        }
        let payload = serde_json::json!({
            "tools": entries.iter().map(|(name, tool)| serde_json::json!({
                "name": name,
                "kind": tool.kind.to_string(),
                "summary": Self::tool_description(tool),
                "aliases": tool.skill.frontmatter.aliases,
                "source_path": tool.skill.source_path,
                "source": tool.origin.source_label(),
                "content_hash": tool.skill.content_hash(),
                "editable": is_workspace_managed_skill(workspace_root, name, tool),
            })).collect::<Vec<_>>(),
            "diagnostics": catalog.diagnostics.iter().map(|diagnostic| serde_json::json!({
                "path": diagnostic.path,
                "error": diagnostic.error,
            })).collect::<Vec<_>>(),
            "total": total,
            "offset": offset,
            "returned": entries.len(),
            "kind": kind_filter.map(|kind| kind.to_string()),
        });
        Ok(ToolInvokeOutput::from_parts(
            "skills list",
            lines.join("\n"),
            Some(payload),
            std::collections::BTreeMap::from([
                ("total_tools".to_string(), total.to_string()),
                ("returned_tools".to_string(), entries.len().to_string()),
                ("offset".to_string(), offset.to_string()),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Read one discovered skill or slash command.",
        read_only,
        discovery,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_get(&self, input: &SkillsGetInput) -> SdkResult<ToolInvokeOutput> {
        let tools = self.discovered_tools()?;
        let workspace_root = self.workspace_root()?;
        let (name, discovered_tool) = Self::resolve_tool(&tools, input.name.as_str())?;
        let summary = Self::tool_description(discovered_tool);
        let body = discovered_tool.skill.body.trim();
        let document = match discovered_tool.skill.source_path.as_ref() {
            Some(path) => std::fs::read_to_string(path).map_err(skill_write_error)?,
            None => format_skill_document(&discovered_tool.skill),
        };
        enforce_skill_document_size(document.as_str())?;
        let text = format!(
            "Name: {name}\nKind: {}\nSummary: {}\n\nBody:\n{}",
            discovered_tool.kind, summary, body
        );
        let payload = serde_json::json!({
            "name": name,
            "kind": discovered_tool.kind.to_string(),
            "summary": summary,
            "body": body,
            "aliases": discovered_tool.skill.frontmatter.aliases,
            "source_path": discovered_tool.skill.source_path,
            "source": discovered_tool.origin.source_label(),
            "content_hash": discovered_tool.skill.content_hash(),
            "document": document,
            "editable": is_workspace_managed_skill(workspace_root, name, discovered_tool),
        });
        Ok(ToolInvokeOutput::from_parts(
            format!("skills get {name}"),
            text,
            Some(payload),
            std::collections::BTreeMap::from([
                ("name".to_string(), name.to_string()),
                ("kind".to_string(), discovered_tool.kind.to_string()),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Create a workspace-managed Skill document.",
        help = "Creates `.agena/skills/<name>/SKILL.md` from a complete SKILL.md document. Only workspace-local Skills are mutable; built-in, plugin, user-global, and compatibility Skills remain read-only.",
        mutating,
        filesystem_write,
        display = detailed
    )]
    async fn invoke_create(&self, input: &SkillsCreateInput) -> SdkResult<ToolInvokeOutput> {
        let result = self.create_managed_skill(input.document.as_str())?;
        let refresh = self.refresh_catalog()?;
        Ok(skill_write_output(
            result,
            refresh.generation,
            refresh.changed,
        ))
    }

    #[tool(
        summary = "Update a workspace-managed Skill document.",
        help = "Replaces an existing `.agena/skills/<name>/SKILL.md` document. The replacement frontmatter must keep the same canonical name.",
        mutating,
        filesystem_write,
        display = detailed
    )]
    async fn invoke_update(&self, input: &SkillsUpdateInput) -> SdkResult<ToolInvokeOutput> {
        let result = self.update_managed_skill(input.name.as_str(), input.document.as_str())?;
        let refresh = self.refresh_catalog()?;
        Ok(skill_write_output(
            result,
            refresh.generation,
            refresh.changed,
        ))
    }

    #[tool(
        summary = "Delete a workspace-managed Skill document.",
        help = "Deletes only `.agena/skills/<name>/SKILL.md`; bundled, plugin, user-global, and compatibility Skills cannot be deleted through this tool.",
        mutating,
        filesystem_write,
        display = detailed
    )]
    async fn invoke_delete(&self, input: &SkillsDeleteInput) -> SdkResult<ToolInvokeOutput> {
        let result = self.delete_managed_skill(input.name.as_str())?;
        let refresh = self.refresh_catalog()?;
        Ok(skill_write_output(
            result,
            refresh.generation,
            refresh.changed,
        ))
    }

    #[tool(
        summary = "Read a bounded UTF-8 resource contained by one skill package.",
        read_only,
        filesystem_read,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_read_resource(
        &self,
        input: &SkillsReadResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let tools = self.discovered_tools()?;
        let (name, discovered_tool) = Self::resolve_tool(&tools, input.name.as_str())?;
        if !discovered_tool.origin.supports_resources() {
            return Err(PluginError::invalid_params(format!(
                "{} '{}' does not expose filesystem-backed resources",
                discovered_tool.origin.source_label(),
                name
            )));
        }
        let content = discovered_tool
            .skill
            .read_text_resource(Path::new(input.path.as_str()), input.max_bytes as usize)
            .map_err(|error| PluginError::invalid_params(error.to_string()))?;
        Ok(ToolInvokeOutput::from_parts(
            format!("skill resource {name}/{}", input.path),
            content.clone(),
            Some(serde_json::json!({
                "name": name,
                "path": input.path,
                "content": content,
                "bytes": content.len(),
                "content_hash": discovered_tool.skill.content_hash(),
                "source_path": discovered_tool.skill.source_path,
                "source": discovered_tool.origin.source_label(),
            })),
            BTreeMap::from([
                ("skill".to_string(), name.to_string()),
                ("resource_path".to_string(), input.path.clone()),
            ]),
            Vec::new(),
        ))
    }

    #[tool(
        summary = "Rescan filesystem-backed Skills and report the catalog generation.",
        read_only,
        discovery,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_refresh(&self, input: &SkillsRefreshInput) -> SdkResult<ToolInvokeOutput> {
        let refresh = self.refresh_catalog()?;
        let (watcher_enabled, watched_path_count, watcher_generation) = self.watcher_status()?;
        let skill_count = refresh
            .catalog
            .tools
            .values()
            .filter(|tool| tool.kind == DiscoveredToolKind::Skill)
            .count();
        let command_count = refresh
            .catalog
            .tools
            .values()
            .filter(|tool| tool.kind == DiscoveredToolKind::Command)
            .count();
        let mut lines = vec![format!(
            "Skill catalog generation {} {} ({} skill(s), {} command(s)).",
            refresh.generation,
            if refresh.changed {
                "changed"
            } else {
                "is unchanged"
            },
            skill_count,
            command_count,
        )];
        if input.verbose && !refresh.catalog.diagnostics.is_empty() {
            lines.push("Discovery diagnostics:".to_string());
            lines.extend(refresh.catalog.diagnostics.iter().map(|diagnostic| {
                format!("- {}: {}", diagnostic.path.display(), diagnostic.error)
            }));
        }
        Ok(ToolInvokeOutput::from_parts(
            "skills refresh",
            lines.join("\n"),
            Some(serde_json::json!({
                "changed": refresh.changed,
                "generation": refresh.generation,
                "fingerprint": refresh.fingerprint,
                "skills": skill_count,
                "commands": command_count,
                "watcher": {
                    "enabled": watcher_enabled,
                    "watched_path_count": watched_path_count,
                    "generation": watcher_generation,
                },
                "diagnostics": refresh.catalog.diagnostics.iter().map(|diagnostic| serde_json::json!({
                    "path": diagnostic.path,
                    "error": diagnostic.error,
                })).collect::<Vec<_>>(),
            })),
            BTreeMap::from([
                (
                    "catalog_generation".to_string(),
                    refresh.generation.to_string(),
                ),
                ("catalog_changed".to_string(), refresh.changed.to_string()),
                ("catalog_fingerprint".to_string(), refresh.fingerprint),
                ("watcher_enabled".to_string(), watcher_enabled.to_string()),
                (
                    "watcher_generation".to_string(),
                    watcher_generation.to_string(),
                ),
            ]),
            Vec::new(),
        ))
    }
}
