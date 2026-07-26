//! Deterministic, machine-readable inventory of capabilities compiled into
//! Agena. Documentation and CI can consume this instead of maintaining tool
//! counts by hand.

use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::{Plugin, PluginKey, PluginManifest};
use agena_runtime_tools::tool::{ToolExposure, tool_exposure};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CapabilityCounts {
    pub plugins: usize,
    pub tools: usize,
    pub direct_tools: usize,
    pub deferred_tools: usize,
    pub hidden_tools: usize,
    pub internal_tools: usize,
    pub bundled_skills: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BundledCapabilityManifest {
    pub schema_version: u32,
    pub snapshot_date: &'static str,
    pub counts: CapabilityCounts,
    pub plugins: Vec<BundledPluginCapability>,
    pub skills: Vec<BundledSkillCapability>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BundledPluginCapability {
    pub id: String,
    pub version: String,
    pub summary: Option<String>,
    pub bundled: bool,
    pub conditional: Option<String>,
    pub hooks: Vec<String>,
    pub capabilities: Vec<String>,
    pub tools: Vec<BundledToolCapability>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BundledToolCapability {
    pub canonical_name: String,
    pub exposure: &'static str,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub effects: Vec<String>,
    pub capabilities: Vec<String>,
    pub input_schema_sha256: String,
    pub output_schema_sha256: String,
    pub definition_identity: String,
    pub bundled: bool,
    pub conditional: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BundledSkillCapability {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub user_invocable: bool,
    pub allow_implicit_invocation: bool,
    pub paths: Vec<String>,
    pub dependency_tools: Vec<String>,
    pub dependency_mcp: Vec<String>,
    pub dependency_environment: Vec<String>,
    pub content_sha256: String,
    pub bundled: bool,
}

/// Return the complete source-level bundled catalog. `agena.mcp` is included
/// and marked conditional because runtime registration requires an MCP
/// manager. `agena.schema_lab` is present only when its compile-time feature is
/// enabled and is marked accordingly.
pub fn bundled_capability_manifest() -> BundledCapabilityManifest {
    let mut plugins = Vec::new();
    macro_rules! add {
        ($plugin:expr) => {
            plugins.push(plugin_capability($plugin.manifest(), None));
        };
        ($plugin:expr, $condition:literal) => {
            plugins.push(plugin_capability(
                $plugin.manifest(),
                Some($condition.to_string()),
            ));
        };
    }

    add!(crate::tool::new_agent_plugin());
    add!(crate::tool::new_code_plugin());
    add!(crate::tool::new_context_plugin());
    add!(crate::tool::new_cron_plugin());
    add!(crate::tool::new_environment_plugin());
    add!(crate::tool::new_fs_plugin());
    add!(crate::tool::new_image_plugin());
    add!(crate::tool::new_interaction_plugin());
    add!(crate::tool::new_lsp_plugin());
    add!(
        crate::tool::new_mcp_plugin(std::sync::Arc::new(
            agena_mcp_client::McpConnectionManager::default()
        )),
        "runtime:mcp-manager"
    );
    add!(crate::tool::new_memory_plugin());
    add!(crate::tool::new_notebook_plugin());
    add!(crate::tool::new_plan_plugin());
    add!(crate::tool::new_report_plugin());
    if crate::tool::schema_lab_builtin_enabled() {
        add!(crate::tool::new_schema_lab_plugin(), "feature:schema-lab");
    }
    add!(crate::tool::new_session_plugin());
    add!(crate::tool::new_settings_plugin());
    add!(crate::tool::new_shell_plugin());
    add!(crate::tool::new_skills_plugin());
    add!(crate::tool::new_snapshot_plugin());
    add!(crate::tool::new_tasks_plugin());
    add!(crate::tool::new_tool_api_plugin());
    add!(crate::tool::new_web_plugin());
    plugins.sort_by(|left, right| left.id.cmp(&right.id));

    let mut skills = agena_skills::bundled::all()
        .into_iter()
        .map(|skill| {
            let content_sha256 = skill.content_hash();
            let frontmatter = skill.frontmatter;
            BundledSkillCapability {
                name: frontmatter.name,
                description: frontmatter.description,
                aliases: frontmatter.aliases,
                allowed_tools: frontmatter.allowed_tools,
                model: frontmatter.model,
                user_invocable: frontmatter.user_invocable,
                allow_implicit_invocation: frontmatter.allow_implicit_invocation,
                paths: frontmatter.paths,
                dependency_tools: frontmatter.dependencies.tools,
                dependency_mcp: frontmatter.dependencies.mcp,
                dependency_environment: frontmatter.dependencies.environment,
                content_sha256,
                bundled: true,
            }
        })
        .collect::<Vec<_>>();
    skills.sort_by(|left, right| left.name.cmp(&right.name));

    let tools = plugins
        .iter()
        .flat_map(|plugin| plugin.tools.iter())
        .collect::<Vec<_>>();
    let count_exposure = |exposure: &str| {
        tools
            .iter()
            .filter(|tool| tool.exposure == exposure)
            .count()
    };
    let counts = CapabilityCounts {
        plugins: plugins.len(),
        tools: tools.len(),
        direct_tools: count_exposure("direct"),
        deferred_tools: count_exposure("deferred"),
        hidden_tools: count_exposure("hidden"),
        internal_tools: count_exposure("internal"),
        bundled_skills: skills.len(),
    };

    BundledCapabilityManifest {
        schema_version: 1,
        snapshot_date: "2026-07-25",
        counts,
        plugins,
        skills,
    }
}

/// Render the committed CI drift snapshot. The snapshot deliberately omits
/// display copy and fields derived from the retained identity fields, so an
/// editorial summary change does not produce a large generated diff. Tool
/// schema hashes, definition identities, routing exposure, permissions,
/// plugin hooks/capabilities, and complete bundled Skill execution metadata
/// remain covered.
pub fn bundled_capability_identity_snapshot_json() -> String {
    let mut value = serde_json::to_value(bundled_capability_manifest())
        .expect("bundled capability manifest must serialize");
    if let Some(root) = value.as_object_mut() {
        root.insert(
            "snapshot_kind".to_string(),
            serde_json::Value::String("capability_identity".to_string()),
        );
    }
    if let Some(plugins) = value
        .get_mut("plugins")
        .and_then(serde_json::Value::as_array_mut)
    {
        for plugin in plugins {
            let Some(plugin) = plugin.as_object_mut() else {
                continue;
            };
            plugin.remove("summary");
            plugin.remove("bundled");
            if let Some(tools) = plugin
                .get_mut("tools")
                .and_then(serde_json::Value::as_array_mut)
            {
                for tool in tools {
                    let Some(tool) = tool.as_object_mut() else {
                        continue;
                    };
                    tool.remove("summary");
                    tool.remove("effects");
                    tool.remove("bundled");
                }
            }
        }
    }
    if let Some(skills) = value
        .get_mut("skills")
        .and_then(serde_json::Value::as_array_mut)
    {
        for skill in skills {
            let Some(skill) = skill.as_object_mut() else {
                continue;
            };
            skill.remove("description");
            skill.remove("bundled");
        }
    }
    let mut output =
        serde_json::to_string_pretty(&value).expect("capability identity snapshot must serialize");
    output.push('\n');
    output
}

fn plugin_capability(
    manifest: PluginManifest,
    conditional: Option<String>,
) -> BundledPluginCapability {
    let key = PluginKey::new(manifest.namespace.clone(), manifest.name.clone())
        .expect("bundled plugin manifest key");
    let mut tools = manifest
        .tools
        .iter()
        .cloned()
        .map(|definition| {
            let registered =
                RegisteredTool::new(key.clone(), definition).expect("bundled tool definition");
            let tags = registered
                .effective_tags()
                .into_iter()
                .map(|tag| tag.to_string())
                .collect::<Vec<_>>();
            let effects = tags
                .iter()
                .filter(|tag| {
                    matches!(
                        tag.as_str(),
                        "mutating"
                            | "filesystem_read"
                            | "filesystem_write"
                            | "network"
                            | "internet"
                            | "shell"
                            | "interactive"
                            | "snapshot"
                            | "scheduler"
                            | "subtask"
                    )
                })
                .cloned()
                .collect();
            BundledToolCapability {
                canonical_name: registered.canonical_name(),
                exposure: exposure_name(tool_exposure(&registered)),
                summary: registered.summary_text().map(str::to_owned),
                tags,
                effects,
                capabilities: registered
                    .definition
                    .capabilities
                    .iter()
                    .map(capability_name)
                    .collect(),
                input_schema_sha256: json_sha256(&registered.input_schema()),
                output_schema_sha256: json_sha256(&registered.output_schema()),
                definition_identity: registered.definition_identity(),
                bundled: true,
                conditional: conditional.clone(),
            }
        })
        .collect::<Vec<_>>();
    tools.sort_by(|left, right| left.canonical_name.cmp(&right.canonical_name));

    BundledPluginCapability {
        id: key.to_string(),
        version: manifest.version,
        summary: manifest.summary,
        bundled: true,
        conditional,
        hooks: manifest
            .hooks
            .names()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        capabilities: manifest
            .plugin_capabilities
            .iter()
            .map(capability_name)
            .collect(),
        tools,
    }
}

fn exposure_name(exposure: ToolExposure) -> &'static str {
    match exposure {
        ToolExposure::Direct => "direct",
        ToolExposure::Deferred => "deferred",
        ToolExposure::Hidden => "hidden",
        ToolExposure::Internal => "internal",
    }
}

fn capability_name(capability: &agena_plugin_host::sdk::HostCapability) -> String {
    serde_json::to_value(capability)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{capability:?}").to_ascii_lowercase())
}

fn json_sha256(value: &serde_json::Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(value).unwrap_or_default());
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::{bundled_capability_identity_snapshot_json, bundled_capability_manifest};

    const COMMITTED_IDENTITY_SNAPSHOT: &str =
        include_str!("../../../docs/generated/bundled-capability-identities.json");

    #[test]
    fn bundled_manifest_is_complete_and_has_consistent_counts() {
        let manifest = bundled_capability_manifest();
        assert_eq!(manifest.counts.plugins, manifest.plugins.len());
        assert_eq!(manifest.counts.bundled_skills, manifest.skills.len());
        assert_eq!(
            manifest.counts.tools,
            manifest
                .plugins
                .iter()
                .map(|plugin| plugin.tools.len())
                .sum::<usize>()
        );
        assert_eq!(
            manifest.counts.tools,
            manifest.counts.direct_tools
                + manifest.counts.deferred_tools
                + manifest.counts.hidden_tools
                + manifest.counts.internal_tools
        );
        assert!(
            manifest
                .plugins
                .iter()
                .any(|plugin| plugin.id == "agena.mcp" && plugin.conditional.is_some())
        );
        assert!(
            manifest
                .skills
                .iter()
                .any(|skill| skill.name == "skill_creator")
        );

        eprintln!(
            "CAPABILITY_COUNTS {}",
            serde_json::to_string(&manifest.counts).expect("serialize counts")
        );
        for plugin in &manifest.plugins {
            eprintln!("CAPABILITY_PLUGIN {} {}", plugin.id, plugin.tools.len());
        }
    }

    #[test]
    fn tool_reference_overview_tracks_the_generated_manifest() {
        let manifest = bundled_capability_manifest();
        let reference = include_str!("../../../docs/plugins-and-tools-reference.md");
        let counts = &manifest.counts;
        let overview = format!(
            "**{} 个插件、{} 个工具定义、{} 个 bundled Skills**",
            counts.plugins, counts.tools, counts.bundled_skills
        );
        assert!(
            reference.contains(overview.as_str()),
            "tool reference overview is stale; expected {overview}"
        );
        let exposure = format!(
            "{} 个工具按 exposure 分为 {} Direct、{} Deferred、{} Hidden、{} Internal",
            counts.tools,
            counts.direct_tools,
            counts.deferred_tools,
            counts.hidden_tools,
            counts.internal_tools,
        );
        assert!(
            reference.contains(exposure.as_str()),
            "tool reference exposure overview is stale; expected {exposure}"
        );
        let total = format!(
            "| **合计** | **{}** | {} plugins | {} Direct + {} Deferred + {} Hidden + {} Internal |",
            counts.tools,
            counts.plugins,
            counts.direct_tools,
            counts.deferred_tools,
            counts.hidden_tools,
            counts.internal_tools,
        );
        assert!(
            reference.contains(total.as_str()),
            "tool reference total row is stale; expected {total}"
        );
        for plugin in &manifest.plugins {
            let expected = format!("| `{}` | {} |", plugin.id, plugin.tools.len());
            assert!(
                reference.contains(expected.as_str()),
                "tool reference plugin index is stale; missing {expected}"
            );
        }
    }

    #[test]
    fn capability_identity_snapshot_matches_committed_json() {
        let generated = bundled_capability_identity_snapshot_json();
        assert_eq!(
            generated, COMMITTED_IDENTITY_SNAPSHOT,
            "bundled capability identity drifted; regenerate with `cargo run -p agena -- inspect --json --identity-snapshot > docs/generated/bundled-capability-identities.json` and review the diff"
        );
    }
}
