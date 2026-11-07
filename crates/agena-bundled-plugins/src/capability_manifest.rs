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
    "scheduler",
    "agent",
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
/// Counts of bundled plugin capabilities.
pub struct CapabilityCounts {
    pub plugins: usize,
    pub tools: usize,
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
    add!(crate::tool::new_skills_plugin());

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
    add!(crate::tool::new_notifications_plugin());
    add!(crate::tool::new_plan_plugin());
    add!(crate::tool::new_report_plugin());
    add!(crate::tool::new_inline_ui_plugin());
    add!(crate::tool::new_session_plugin());
    add!(crate::tool::new_provider_plugin());
    add!(crate::tool::new_settings_plugin());
    add!(crate::tool::new_shell_plugin());
    add!(crate::tool::new_agent_plugin());
    add!(crate::tool::new_web_plugin());
    plugins
}

#[cfg(test)]
mod registration_inventory_tests {
    use std::collections::BTreeSet;

    #[test]
    fn capability_inventory_includes_every_registered_plugin_including_toolless_plugins() {
        let manifest = super::bundled_capability_manifest();
        let ids: BTreeSet<_> = manifest
            .plugins
            .iter()
            .map(|plugin| plugin.id.clone())
            .collect();
        let registered: BTreeSet<_> = crate::plugins::sources::bundled_plugin_entries()
            .into_keys()
            .collect();
        assert_eq!(
            ids.len(),
            manifest.plugins.len(),
            "duplicate capability plugin identity"
        );
        assert_eq!(
            ids, registered,
            "compiled capability inventory drifted from actual registrations"
        );
    }
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
    let counts = CapabilityCounts {
        plugins: plugins.len(),
        tools: tools.len(),
        bundled_commands: commands.len(),
    };

    BundledCapabilityManifest {
        schema_version: 3,
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
            BundledToolCapability {
                canonical_name: registered.canonical_name(),
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
mod hook_description_tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    // Read both product locale catalogs so this coverage test automatically
    // expands whenever the Web or TUI adds a supported language.
    fn supported_hook_locales() -> BTreeSet<String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates directory")
            .parent()
            .expect("workspace root");
        let mut locales = BTreeSet::new();
        for entry in
            std::fs::read_dir(root.join("crates/agena-tui/locales")).expect("TUI locale catalog")
        {
            let entry = entry.expect("TUI locale directory");
            if entry.path().is_dir() {
                locales.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
        for entry in std::fs::read_dir(root.join("packages/agena-web/src/i18n/settings-text"))
            .expect("Web Settings locale catalog")
        {
            let entry = entry.expect("Web locale file");
            if entry.path().extension().is_some_and(|ext| ext == "json") {
                locales.insert(
                    entry
                        .path()
                        .file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        locales.remove("en-US"); // Required English summary/help live on the Hook itself.
        locales
    }

    fn declared_hook_count(root: &Path) -> usize {
        std::fs::read_dir(root)
            .expect("bundled source directory")
            .map(|entry| {
                let path = entry.expect("bundled source file").path();
                if path.is_dir() {
                    return declared_hook_count(&path);
                }
                if path.extension().is_some_and(|ext| ext == "rs") {
                    return std::fs::read_to_string(&path)
                        .expect("bundled Hook source")
                        .lines()
                        .filter(|line| line.trim_start().starts_with("#[hook("))
                        .count();
                }
                0
            })
            .sum()
    }

    #[test]
    fn every_bundled_hook_translates_summary_and_help_for_all_product_locales() {
        let locales = supported_hook_locales();
        assert!(
            locales.len() >= 10,
            "product locale catalogs unexpectedly incomplete: {locales:?}"
        );

        let manifests = super::bundled_plugin_manifests()
            .into_iter()
            .map(|(manifest, _)| manifest)
            .collect::<Vec<_>>();

        let mut inspected = 0;
        for manifest in &manifests {
            for hook in &manifest.hook_handlers {
                inspected += 1;
                let id = format!("{}.{}::{}", manifest.namespace, manifest.name, hook.id);
                assert!(
                    !hook.summary.trim().is_empty(),
                    "{id} has no English summary"
                );
                assert!(!hook.help.trim().is_empty(), "{id} has no English help");
                for locale in &locales {
                    let translation = hook
                        .translations
                        .get(locale)
                        .unwrap_or_else(|| panic!("{id} is missing the {locale} translation"));
                    assert!(
                        !translation.summary.trim().is_empty(),
                        "{id} has an empty {locale} summary"
                    );
                    assert!(
                        !translation.help.trim().is_empty(),
                        "{id} has an empty {locale} help"
                    );
                    assert_ne!(
                        translation.summary, hook.summary,
                        "{id} has an untranslated {locale} summary"
                    );
                    assert_ne!(
                        translation.help, hook.help,
                        "{id} has untranslated {locale} help"
                    );
                }
            }
        }
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        assert_eq!(
            inspected,
            declared_hook_count(&source_root),
            "Every #[hook(...)] declaration must be represented by a bundled or runtime-provided manifest",
        );
    }
}
