//! Deterministic, machine-readable inventory of capabilities compiled into
//! Agena. Documentation and CI can consume this instead of maintaining tool
//! counts by hand.

use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::{Plugin, PluginKey, PluginManifest};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
/// Counts of bundled plugin capabilities.
pub struct CapabilityCounts {
    pub plugins: usize,
    pub tools: usize,
    /// The four discovery handlers implemented by the `agena.tools` plugin.
    /// `tools_call` is a runtime-synthesized provider gateway definition, not
    /// an executable plugin handler and therefore is not counted here.
    pub gateway_tools: usize,
    /// Every non-gateway tool. All of these share the same discovery,
    /// authorization, and tools_call execution path.
    pub execution_tools: usize,
    pub bundled_skills: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
/// Manifest of bundled plugin capabilities.
pub struct BundledCapabilityManifest {
    pub schema_version: u32,
    pub snapshot_date: &'static str,
    pub counts: CapabilityCounts,
    pub plugins: Vec<BundledPluginCapability>,
    pub skills: Vec<BundledSkillCapability>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
/// Capability of a bundled plugin.
pub struct BundledPluginCapability {
    pub id: String,
    pub version: String,
    pub summary: Option<String>,
    pub bundled: bool,
    pub conditional: Option<String>,
    pub hooks: Vec<String>,
    pub tools: Vec<BundledToolCapability>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
/// Tool capability of a bundled plugin.
pub struct BundledToolCapability {
    pub canonical_name: String,
    /// True only for the fixed agena.tools discovery protocol handlers. The
    /// `tools_call` provider gateway is runtime-synthesized and has no plugin
    /// tool entry. False means an ordinary execution tool.
    pub gateway: bool,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub effects: Vec<String>,
    pub input_schema_sha256: String,
    pub output_schema_sha256: String,
    pub definition_identity: String,
    pub bundled: bool,
    pub conditional: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
/// Skill capability of a bundled plugin.
pub struct BundledSkillCapability {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub content_sha256: String,
    pub bundled: bool,
}

/// Collect the complete source-level bundled catalog as `(manifest,
/// conditional)` pairs. Shared by the capability manifest, the identity
/// snapshot, and the generated `cargo doc` tool reference so every surface
/// enumerates exactly the same plugins.
pub(crate) fn bundled_plugin_manifests() -> Vec<(PluginManifest, Option<String>)> {
    let mut plugins = Vec::new();
    macro_rules! add {
        ($plugin:expr) => {
            plugins.push(($plugin.manifest(), None));
        };
        ($plugin:expr, $condition:literal) => {
            plugins.push(($plugin.manifest(), Some($condition.to_string())));
        };
    }
    add!(crate::tool::new_chatgpt_plugin());
    add!(crate::tool::new_gemini_plugin());
    add!(crate::tool::new_claude_plugin());

    add!(crate::tool::new_code_plugin());
    add!(crate::tool::new_cron_plugin());
    add!(crate::tool::new_fs_plugin());
    add!(crate::tool::new_interaction_plugin());
    add!(crate::tool::new_lsp_plugin());
    add!(
        crate::tool::new_mcp_plugin(std::sync::Arc::new(
            agena_mcp_client::McpConnectionManager::default()
        )),
        "runtime:mcp-manager"
    );
    add!(crate::tool::new_memory_plugin());
    add!(crate::tool::new_monitor_plugin());
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
    plugins
}

/// Return the complete source-level bundled catalog. `agena.mcp` is included
/// and marked conditional because runtime registration requires an MCP
/// manager. `agena.schema_lab` is present only when its compile-time feature is
/// enabled and is marked accordingly.
pub fn bundled_capability_manifest() -> BundledCapabilityManifest {
    let mut plugins = bundled_plugin_manifests()
        .into_iter()
        .map(|(manifest, conditional)| plugin_capability(manifest, conditional))
        .collect::<Vec<_>>();
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
    let gateway_tools = tools.iter().filter(|tool| tool.gateway).count();
    let counts = CapabilityCounts {
        plugins: plugins.len(),
        tools: tools.len(),
        gateway_tools,
        execution_tools: tools.len().saturating_sub(gateway_tools),
        bundled_skills: skills.len(),
    };

    BundledCapabilityManifest {
        schema_version: 2,
        snapshot_date: "2026-07-27",
        counts,
        plugins,
        skills,
    }
}

/// Render the committed CI drift snapshot. The snapshot deliberately omits
/// display copy and fields derived from retained identity fields, so editorial
/// changes do not produce large generated diffs.
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
            // Effects are derived from the permission contract (authority) and
            // declared metadata tags; tags alone never carry authority.
            let permissions = &registered.definition.permissions;
            let read_specs = permissions
                .input_paths
                .iter()
                .any(|spec| spec.kind == agena_plugin_host::sdk::PathKind::Read)
                || permissions
                    .path_access
                    .iter()
                    .any(|spec| spec.kind == agena_plugin_host::sdk::PathKind::Read);
            let write_specs = permissions
                .input_paths
                .iter()
                .any(|spec| spec.kind == agena_plugin_host::sdk::PathKind::Write)
                || permissions
                    .path_access
                    .iter()
                    .any(|spec| spec.kind == agena_plugin_host::sdk::PathKind::Write);
            let network_specs =
                !permissions.input_networks.is_empty() || !permissions.network_access.is_empty();
            let declared = registered
                .definition
                .tags
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            let mut effects = Vec::new();
            if permissions.read_only {
                effects.push("read_only".to_string());
            }
            if declared.iter().any(|tag| tag == "mutating") {
                effects.push("mutating".to_string());
            }
            if read_specs || declared.iter().any(|tag| tag == "filesystem_read") {
                effects.push("filesystem_read".to_string());
            }
            if write_specs || declared.iter().any(|tag| tag == "filesystem_write") {
                effects.push("filesystem_write".to_string());
            }
            if network_specs || declared.iter().any(|tag| tag == "network") {
                effects.push("network".to_string());
            }
            if declared.iter().any(|tag| tag == "internet") {
                effects.push("internet".to_string());
            }
            if permissions.shell {
                effects.push("shell".to_string());
            }
            if permissions.interactive {
                effects.push("interactive".to_string());
            }
            for metadata in ["snapshot", "scheduler", "subtask"] {
                if declared.iter().any(|tag| tag == metadata) {
                    effects.push(metadata.to_string());
                }
            }
            let tags = registered
                .effective_tags()
                .into_iter()
                .map(|tag| tag.to_string())
                .collect::<Vec<_>>();
            let gateway = key.to_string() == "agena.tools"
                && matches!(
                    registered.tool_name(),
                    "list" | "search" | "help" | "tags" | "call"
                );
            BundledToolCapability {
                canonical_name: registered.canonical_name(),
                gateway,
                summary: registered.summary_text().map(str::to_owned),
                tags,
                effects,
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
        tools,
    }
}

fn json_sha256(value: &serde_json::Value) -> String {
    let digest = Sha256::digest(serde_json::to_vec(value).unwrap_or_default());
    hex::encode(digest)
}
