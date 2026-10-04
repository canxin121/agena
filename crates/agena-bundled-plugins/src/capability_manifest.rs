//! Deterministic, machine-readable inventory of capabilities compiled into
//! Agena. Documentation and CI can consume this instead of maintaining tool
//! counts by hand.

use agena_plugin_host::registry::RegisteredTool;
use agena_plugin_host::sdk::{Plugin, PluginKey, PluginManifest};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The tag spellings that read as an effect in the capability listing.
///
/// This is a filter over the tool's declared tags, not a second declaration
/// surface: every entry here is spelled the way `ToolTag` spells it, so a tag
/// either appears as an effect in the inventory or is purely discovery
/// metadata. It is the same vocabulary the tool's declared behavior used to
/// project, minus the spellings no tag ever used.
const EFFECT_TAGS: &[&str] = &[
    "read_only",
    "mutate",
    "network",
    "shell",
    "interactive",
    "snapshot",
    "scheduler",
    "subtask",
];

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
    pub bundled_commands: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
/// Manifest of bundled plugin capabilities.
pub struct BundledCapabilityManifest {
    pub schema_version: u32,
    pub snapshot_date: &'static str,
    pub counts: CapabilityCounts,
    pub plugins: Vec<BundledPluginCapability>,
    pub commands: Vec<BundledCommandCapability>,
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
/// Command capability of a bundled plugin.
pub struct BundledCommandCapability {
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
    bundled_plugins()
        .into_iter()
        .map(|(plugin, condition)| (plugin.manifest(), condition))
        .collect()
}

fn bundled_plugins() -> Vec<(Box<dyn Plugin>, Option<String>)> {
    let mut plugins = Vec::new();
    macro_rules! add {
        ($plugin:expr) => {
            plugins.push((Box::new($plugin) as Box<dyn Plugin>, None));
        };
        ($plugin:expr, $condition:literal) => {
            plugins.push((
                Box::new($plugin) as Box<dyn Plugin>,
                Some($condition.to_string()),
            ));
        };
    }
    add!(crate::tool::new_chatgpt_plugin());
    add!(crate::tool::new_gemini_plugin());
    add!(crate::tool::new_claude_plugin());
    add!(crate::tool::new_commands_plugin());

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
    add!(crate::tool::new_session_plugin());
    add!(crate::tool::new_settings_plugin());
    add!(crate::tool::new_shell_plugin());
    add!(crate::tool::new_snapshot_plugin());
    add!(crate::tool::new_tasks_plugin());
    add!(crate::tool::new_tool_api_plugin());
    add!(crate::tool::new_web_plugin());
    plugins
}

/// Return the complete source-level bundled catalog. `agena.mcp` is included
/// and marked conditional because runtime registration requires an MCP
/// manager.
pub fn bundled_capability_manifest() -> BundledCapabilityManifest {
    let mut plugins = bundled_plugin_manifests()
        .into_iter()
        .map(|(manifest, conditional)| plugin_capability(manifest, conditional))
        .collect::<Vec<_>>();
    plugins.sort_by(|left, right| left.id.cmp(&right.id));

    let mut commands = crate::plugins::provided::commands::declared_commands()
        .into_iter()
        .map(|command| {
            let content_sha256 = command.content_sha256;
            BundledCommandCapability {
                name: command.name,
                description: command.description,
                aliases: command.aliases,
                content_sha256,
                bundled: true,
            }
        })
        .collect::<Vec<_>>();
    commands.sort_by(|left, right| left.name.cmp(&right.name));

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
        bundled_commands: commands.len(),
    };

    BundledCapabilityManifest {
        schema_version: 2,
        snapshot_date: "2026-07-27",
        counts,
        plugins,
        commands,
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
    if let Some(commands) = value
        .get_mut("commands")
        .and_then(serde_json::Value::as_array_mut)
    {
        for command in commands {
            let Some(command) = command.as_object_mut() else {
                continue;
            };
            command.remove("description");
            command.remove("bundled");
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
            // Effects are a reading of the tool's declared tags. They are
            // self-description: they describe what a tool does for audit and
            // UI purposes, not what the host will authorize.
            let effects = tags
                .iter()
                .filter(|tag| EFFECT_TAGS.contains(&tag.as_str()))
                .cloned()
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
    let bytes = match serde_json::to_vec(value) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize bundled-plugin capability identity",
                    &error,
                ),
                "bundled-plugin capability identity is using a debug fallback"
            );
            format!("{value:?}").into_bytes()
        }
    };
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

#[cfg(test)]
mod boundary_audit {
    use super::*;
    use agena_plugin_host::sdk::ToolInvokeInput;

    #[tokio::test]
    async fn every_bundled_tool_rejects_malformed_arguments_before_execution() {
        let workspace = tempfile::tempdir().unwrap();
        let mut failures = Vec::new();
        let mut count = 0;
        for (plugin, _) in bundled_plugins() {
            let manifest = plugin.manifest();
            for tool in manifest.tools {
                count += 1;
                for input in [
                    serde_json::json!([]),
                    serde_json::json!(false),
                    serde_json::json!({"__unknown_audit_argument": true}),
                ] {
                    let result = plugin
                        .tool_invoke(ToolInvokeInput {
                            tool_name: tool.name.clone(),
                            session_id: 1,
                            call_id: 1,
                            workspace_root: workspace.path().display().to_string(),
                            input: input.clone(),
                        })
                        .await;
                    if !matches!(&result, Err(error) if error.kind == agena_plugin_host::sdk::PluginErrorKind::InvalidParams)
                    {
                        failures.push(format!(
                            "{}.{}({input}): {result:?}",
                            manifest.name, tool.name
                        ));
                    }
                }
            }
        }
        assert_eq!(count, 137);
        assert!(
            failures.is_empty(),
            "Malformed arguments escaped validation:\n{}",
            failures.join("\n")
        );
    }
}
