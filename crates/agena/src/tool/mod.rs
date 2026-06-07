pub(crate) mod apply_patch;
pub(crate) mod ask_user;
pub(crate) mod bash;
pub(crate) mod catalog;
pub(crate) mod cron;
pub(crate) mod definition;
pub(crate) mod file_attachment;
pub(crate) mod glob;
pub(crate) mod grep;
pub(crate) mod lsp;
pub(crate) mod monitor;
pub(crate) mod monitor_tool;
pub(crate) mod notebook_edit;
pub(crate) mod orchestrator;
pub(crate) mod payload;
pub(crate) mod powershell;
pub(crate) mod read;
pub(crate) mod result;
pub(crate) mod shell;
pub(crate) mod shell_tools;
pub(crate) mod task;
pub(crate) mod todo_write;
pub(crate) mod tool_search;
pub(crate) mod truncation;
pub(crate) mod worktree;

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use thiserror::Error;

use crate::agent::Agent;
use crate::message::{
    AskUserToolInput, FilesystemEffect, Message, NetworkEffect, PluginInvocation, StructuredObject,
    ToolInvocation, ToolOutput,
};
use crate::permission::{AccessKind, NetworkTarget, PermissionAction, PermissionDecision};
use crate::plugin::{
    PluginHost, PluginHostBuilder, ToolAfterInput as PluginToolAfterInput,
    ToolBeforeInput as PluginToolBeforeInput, ToolDefinitionInput as PluginToolDefinitionInput,
    ToolFailureInput as PluginToolFailureInput, ToolInvokeInput as PluginToolInvokeInput,
    ToolPermissionNetworksInput as PluginToolPermissionNetworksInput,
    ToolPermissionPathsInput as PluginToolPermissionPathsInput,
    registry::RegisteredTool,
    sdk::{
        InputNetworkSpec as SdkInputNetworkSpec, InputPathSpec as SdkInputPathSpec,
        NetworkAccessSpec as SdkNetworkAccessSpec, PathAccessSpec as SdkPathAccessSpec,
        PathKind as SdkPathKind, ShellEnvInput as PluginShellEnvInput,
        ToolResultPolicy as SdkToolResultPolicy, ToolStreamingMode as SdkToolStreamingMode,
    },
};
use crate::plugins::provided::{
    code as provided_code, cron as provided_cron, fs as provided_fs, lsp as provided_lsp, mcp,
    router as in_process_router, schema_lab as provided_schema_lab, settings as provided_settings,
    shell as provided_shell, skills, workflow as provided_workflow,
};

pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};
pub use catalog::{ModelToolProfile, ToolAvailability, ToolCatalog};
pub use monitor::{
    MonitorError, MonitorRead, MonitorRegistry, MonitorService, MonitorStart, MonitorStopOutcome,
    ReadParams as MonitorReadParams, StartParams as MonitorStartParams,
};
pub use payload::{CronJobSummary, ToolPayloadInput, ToolPayloadOutput, WebSearchHit};
pub use result::{ToolExecutionView, ToolInvocationExecution, ToolPayloadExecution};
pub use shell::{ShellError, ShellOutput, ShellRequest};
pub use truncation::{ToolOutputTruncationPolicy, ToolOutputTruncator};
pub use worktree::{
    ActiveWorktree, ManagedWorktree, WorktreeRegistry, list_active as worktree_list_active,
    list_managed as worktree_list_managed, prune_stale as worktree_prune_stale,
    registry_for_executor as worktree_registry_for_executor,
};

pub fn skills_plugin_id() -> &'static str {
    skills::SKILLS_PLUGIN_ID
}

pub(crate) fn model_safe_tool_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "tool".to_owned();
    }

    if trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return trimmed.to_owned();
    }

    crate::plugin::registry::exposed_tool_name_segment(trimmed)
}

pub(crate) fn suggest_tool_names<I, T>(requested: &str, candidates: I, limit: usize) -> Vec<String>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let requested = requested.trim();
    let requested_lower = requested.to_ascii_lowercase();
    let mut ranked: Vec<(usize, String)> = Vec::new();

    for candidate in candidates {
        let name = candidate.as_ref().trim();
        if name.is_empty() {
            continue;
        }
        let score = normalized_tool_name_distance(requested, name);
        if score == 0 {
            continue;
        }
        let name_lower = name.to_ascii_lowercase();
        if score <= 4
            || name_lower.contains(requested_lower.as_str())
            || requested_lower.contains(name_lower.as_str())
        {
            ranked.push((score, name.to_string()));
        }
    }

    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let mut suggestions = Vec::new();
    for (_, name) in ranked {
        if !suggestions.contains(&name) {
            suggestions.push(name);
        }
        if suggestions.len() >= limit {
            break;
        }
    }
    suggestions
}

pub(crate) fn unknown_tool_message(requested: &str, suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        return format!("unknown tool '{requested}'");
    }
    format!(
        "unknown tool '{requested}'. Did you mean {}?",
        suggestions
            .iter()
            .map(|tool| format!("`{tool}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(crate) fn unknown_tool_hint(requested: &str, suggestions: Vec<String>) -> ToolError {
    let suggestion_text = unknown_tool_message(requested, &suggestions);
    ToolError::UnknownToolHint {
        tool: requested.to_string(),
        suggestions,
        suggestion_text,
    }
}

fn normalized_tool_name_distance(left: &str, right: &str) -> usize {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    if left == right {
        return 0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0; right_chars.len() + 1];
    for (i, left_ch) in left_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, right_ch) in right_chars.iter().enumerate() {
            let replace = prev[j] + usize::from(left_ch != right_ch);
            let insert = curr[j] + 1;
            let delete = prev[j + 1] + 1;
            curr[j + 1] = replace.min(insert.min(delete));
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[right_chars.len()]
}

pub(crate) fn tool_matches_model_name(registered_tool: &RegisteredTool, name: &str) -> bool {
    let trimmed = name.trim();
    registered_tool.exposed_name == trimmed
        || model_safe_tool_name(registered_tool.exposed_name.as_str()) == trimmed
        || registered_tool
            .alias_exposed_names()
            .iter()
            .any(|alias| alias == trimmed || model_safe_tool_name(alias) == trimmed)
}

pub(crate) fn model_safe_tool_schema(schema: &serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(mut object) = schema.clone() else {
        return empty_object_schema();
    };

    for key in ["oneOf", "anyOf", "allOf"] {
        let Some(serde_json::Value::Array(variants)) = object.remove(key) else {
            continue;
        };
        if variants
            .iter()
            .all(|variant| json_schema_object(variant).is_some())
        {
            return merge_top_level_object_variants(object, variants);
        }
        return empty_object_schema();
    }

    let is_object = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == "object")
        || object.contains_key("properties");
    if !is_object {
        return empty_object_schema();
    }
    object
        .entry("type".to_owned())
        .or_insert_with(|| serde_json::Value::String("object".to_owned()));
    object
        .entry("properties".to_owned())
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    serde_json::Value::Object(object)
}

fn empty_object_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

fn merge_top_level_object_variants(
    mut base: serde_json::Map<String, serde_json::Value>,
    variants: Vec<serde_json::Value>,
) -> serde_json::Value {
    base.insert(
        "type".to_owned(),
        serde_json::Value::String("object".to_owned()),
    );
    let mut properties = base
        .remove("properties")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let mut required_intersection: Option<BTreeSet<String>> = required_set(&base);

    for variant in variants {
        let Some(variant) = json_schema_object(&variant) else {
            continue;
        };
        if let Some(variant_properties) = variant
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (name, schema) in variant_properties {
                properties
                    .entry(name.clone())
                    .and_modify(|existing| *existing = merge_property_schema(existing, schema))
                    .or_insert_with(|| schema.clone());
            }
        }
        if let Some(variant_required) = required_set(variant) {
            required_intersection = Some(match required_intersection.take() {
                Some(existing) => existing
                    .intersection(&variant_required)
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                None => variant_required,
            });
        }
    }

    base.insert(
        "properties".to_owned(),
        serde_json::Value::Object(properties),
    );
    if let Some(required) = required_intersection.filter(|required| !required.is_empty()) {
        base.insert(
            "required".to_owned(),
            serde_json::Value::Array(
                required
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    } else {
        base.remove("required");
    }
    serde_json::Value::Object(base)
}

fn json_schema_object(
    value: &serde_json::Value,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    let object = value.as_object()?;
    let is_object = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == "object")
        || object.contains_key("properties")
        || object.contains_key("required");
    is_object.then_some(object)
}

fn required_set(object: &serde_json::Map<String, serde_json::Value>) -> Option<BTreeSet<String>> {
    object
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
}

fn merge_property_schema(
    existing: &serde_json::Value,
    next: &serde_json::Value,
) -> serde_json::Value {
    let Some(mut literals) = string_literals(existing) else {
        return existing.clone();
    };
    let Some(next_literals) = string_literals(next) else {
        return existing.clone();
    };
    literals.extend(next_literals);
    serde_json::json!({
        "type": "string",
        "enum": literals.into_iter().collect::<Vec<_>>()
    })
}

fn string_literals(value: &serde_json::Value) -> Option<BTreeSet<String>> {
    let object = value.as_object()?;
    if let Some(value) = object.get("const").and_then(serde_json::Value::as_str) {
        return Some(BTreeSet::from([value.to_owned()]));
    }
    object
        .get("enum")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
}

#[derive(Debug, Clone)]
struct DiscriminatedSchemaVariant {
    field: String,
    value: String,
    schema: serde_json::Value,
}

fn top_level_discriminated_variants(
    schema: &serde_json::Value,
) -> Option<Vec<DiscriminatedSchemaVariant>> {
    let object = schema.as_object()?;
    let variants = ["oneOf", "anyOf", "allOf"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(serde_json::Value::as_array))?;
    if variants.len() <= 1 {
        return None;
    }

    let variant_objects = variants
        .iter()
        .map(json_schema_object)
        .collect::<Option<Vec<_>>>()?;
    let discriminant = variant_objects
        .iter()
        .fold(None::<BTreeSet<String>>, |candidates, variant| {
            let fields = variant
                .get("properties")
                .and_then(serde_json::Value::as_object)
                .map(|properties| {
                    properties
                        .iter()
                        .filter_map(|(name, property)| {
                            let literals = string_literals(property)?;
                            (literals.len() == 1).then_some(name.clone())
                        })
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            Some(match candidates {
                Some(existing) => existing
                    .intersection(&fields)
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                None => fields,
            })
        })
        .and_then(|candidates| {
            ["action", "target"]
                .into_iter()
                .find_map(|preferred| candidates.contains(preferred).then_some(preferred))
                .map(ToOwned::to_owned)
                .or_else(|| candidates.into_iter().next())
        })?;

    let mut seen_values = BTreeSet::new();
    let mut expanded = Vec::with_capacity(variant_objects.len());
    for variant in variant_objects {
        let value = variant
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .and_then(|properties| properties.get(discriminant.as_str()))
            .and_then(string_literals)
            .and_then(|literals| literals.into_iter().next())?;
        if !seen_values.insert(value.clone()) {
            return None;
        }
        expanded.push(DiscriminatedSchemaVariant {
            field: discriminant.clone(),
            value,
            schema: strip_discriminant_from_variant(variant, discriminant.as_str()),
        });
    }

    Some(expanded)
}

fn strip_discriminant_from_variant(
    variant: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> serde_json::Value {
    let mut stripped = variant.clone();
    if let Some(properties) = stripped
        .get_mut("properties")
        .and_then(serde_json::Value::as_object_mut)
    {
        properties.remove(field);
    }
    if let Some(required) = stripped
        .get_mut("required")
        .and_then(serde_json::Value::as_array_mut)
    {
        required.retain(|item| item.as_str() != Some(field));
        if required.is_empty() {
            stripped.remove("required");
        }
    }
    stripped
        .entry("type".to_string())
        .or_insert_with(|| serde_json::Value::String("object".to_string()));
    stripped
        .entry("properties".to_string())
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    stripped.insert(
        "x-agena-discriminant-field".to_string(),
        serde_json::Value::String(field.to_string()),
    );
    serde_json::Value::Object(stripped)
}

fn merge_fixed_tool_input(
    input: serde_json::Value,
    fixed_input: Option<&serde_json::Value>,
) -> serde_json::Value {
    let Some(fixed_input) = fixed_input else {
        return input;
    };
    let mut merged = match input {
        serde_json::Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    let Some(fixed_object) = fixed_input.as_object() else {
        return serde_json::Value::Object(merged);
    };
    for (key, value) in fixed_object {
        merged.insert(key.clone(), value.clone());
    }
    serde_json::Value::Object(merged)
}

fn fixed_input_summary(fixed_input: &serde_json::Value) -> Option<String> {
    let object = fixed_input.as_object()?;
    let parts = object
        .iter()
        .map(|(key, value)| match value {
            serde_json::Value::String(text) => format!("`{key}` = `{text}`"),
            other => format!("`{key}` = `{other}`"),
        })
        .collect::<Vec<_>>();
    (!parts.is_empty()).then_some(parts.join(", "))
}

fn schema_description_text(schema: &serde_json::Value) -> Option<&str> {
    schema
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn model_alias_description(
    base: &RegisteredTool,
    fixed_input: &serde_json::Value,
    schema: &serde_json::Value,
) -> String {
    let fixed = fixed_input_summary(fixed_input)
        .map(|summary| {
            format!(" This specialized model-visible alias is fixed to {summary}; provide only the remaining arguments.")
        })
        .unwrap_or_default();
    let base_description =
        schema_description_text(schema).unwrap_or_else(|| base.description_text().trim());
    if base_description.is_empty() {
        return format!(
            "Specialized model-visible alias for `{}`.{}",
            base.behavior_exposed_name(),
            fixed
        )
        .trim()
        .to_string();
    }
    format!("{base_description}{fixed}")
}

fn model_alias_help(base: &RegisteredTool, fixed_input: &serde_json::Value) -> Option<String> {
    let fixed = fixed_input_summary(fixed_input)?;
    let prefix = format!(
        "This model-visible alias dispatches to `{}` with fixed {}.",
        base.behavior_exposed_name(),
        fixed
    );
    Some(match base.help_text() {
        Some(help) => format!("{prefix}\n\n{help}"),
        None => prefix,
    })
}

fn tool_name_alias_description(base: &RegisteredTool) -> String {
    let prefix = format!("Alias for `{}`.", base.behavior_exposed_name());
    let base_description = base.description_text().trim();
    if base_description.is_empty() {
        prefix
    } else {
        format!("{prefix} {base_description}")
    }
}

fn tool_name_alias_help(base: &RegisteredTool) -> Option<String> {
    let prefix = format!(
        "This tool alias dispatches to `{}`.",
        base.behavior_exposed_name()
    );
    Some(match base.help_text() {
        Some(help) => format!("{prefix}\n\n{help}"),
        None => prefix,
    })
}

fn allocate_model_alias_name(
    base: &RegisteredTool,
    alias_segments: &[String],
    used_exposed_names: &mut BTreeSet<String>,
) -> String {
    let stem = format!("{}.{}", base.original_name, alias_segments.join("."));
    let mut candidate = stem.clone();
    let mut suffix = 2usize;
    loop {
        let exposed = crate::plugin::registry::exposed_tool_name(
            base.plugin_name.as_str(),
            candidate.as_str(),
        );
        if used_exposed_names.insert(exposed) {
            return candidate;
        }
        candidate = format!("{stem}_{suffix}");
        suffix += 1;
    }
}

fn expand_registered_tool_for_model(
    base: &RegisteredTool,
    used_exposed_names: &mut BTreeSet<String>,
    out: &mut Vec<RegisteredTool>,
) {
    for alias_name in base.alias_names() {
        let alias_exposed_name =
            crate::plugin::registry::exposed_tool_name(base.plugin_name.as_str(), alias_name);
        if !used_exposed_names.insert(alias_exposed_name) {
            continue;
        }
        let mut decl = base.decl.clone();
        decl.name = alias_name.to_string();
        decl.aliases.clear();
        decl.description = Some(tool_name_alias_description(base));
        decl.help = tool_name_alias_help(base);
        out.push(base.with_tool_alias(alias_name, decl));
    }
    expand_registered_tool_for_model_inner(
        base,
        base.sanitized_input_schema(),
        serde_json::Map::new(),
        Vec::new(),
        used_exposed_names,
        out,
    );
}

fn expand_registered_tool_for_model_inner(
    base: &RegisteredTool,
    schema: serde_json::Value,
    fixed_input: serde_json::Map<String, serde_json::Value>,
    alias_segments: Vec<String>,
    used_exposed_names: &mut BTreeSet<String>,
    out: &mut Vec<RegisteredTool>,
) {
    if let Some(variants) = top_level_discriminated_variants(&schema) {
        for variant in variants {
            let mut next_fixed_input = fixed_input.clone();
            next_fixed_input.insert(
                variant.field.clone(),
                serde_json::Value::String(variant.value.clone()),
            );
            let mut next_alias_segments = alias_segments.clone();
            next_alias_segments.push(variant.value);
            expand_registered_tool_for_model_inner(
                base,
                variant.schema,
                next_fixed_input,
                next_alias_segments,
                used_exposed_names,
                out,
            );
        }
        return;
    }

    if alias_segments.is_empty() {
        out.push(base.clone());
        return;
    }

    let alias_name = allocate_model_alias_name(base, &alias_segments, used_exposed_names);
    let fixed_input_value = serde_json::Value::Object(fixed_input);
    let mut decl = base.decl.clone();
    decl.name = alias_name.clone();
    decl.input_schema = schema;
    decl.description = Some(model_alias_description(
        base,
        &fixed_input_value,
        &decl.input_schema,
    ));
    decl.help = model_alias_help(base, &fixed_input_value);

    let mut alias = base.with_model_alias(alias_name, decl, fixed_input_value.clone());
    let fixed_input_object = StructuredObject::try_from(fixed_input_value)
        .expect("model tool alias fixed input should always be an object");
    let invocation = ToolInvocation::new(alias.exposed_name.as_str(), fixed_input_object);
    alias.decl.tags = invocation_effective_tags(&alias, &invocation);
    alias.decl.concurrency_safe = is_concurrency_safe_tool_invocation(
        &alias,
        &PluginInvocation::from_tool_invocation(&invocation),
    );
    out.push(alias);
}

pub fn new_skills_plugin() -> impl crate::plugin::sdk::Plugin {
    skills::SkillsPlugin::new()
}

pub fn lsp_plugin_id() -> &'static str {
    provided_lsp::LSP_PLUGIN_ID
}

pub fn new_lsp_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_lsp::LspPlugin::new()
}

pub fn cron_plugin_id() -> &'static str {
    provided_cron::CRON_PLUGIN_ID
}

pub fn new_cron_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_cron::CronPlugin::new()
}

pub fn code_plugin_id() -> &'static str {
    provided_code::CODE_PLUGIN_ID
}

pub fn new_code_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_code::new_plugin()
}

pub fn fs_plugin_id() -> &'static str {
    provided_fs::FS_PLUGIN_ID
}

pub fn new_fs_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_fs::new_plugin()
}

pub fn settings_plugin_id() -> &'static str {
    provided_settings::SETTINGS_PLUGIN_ID
}

pub fn new_settings_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_settings::SettingsPlugin::new()
}

pub fn shell_plugin_id() -> &'static str {
    provided_shell::SHELL_PLUGIN_ID
}

pub fn new_shell_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_shell::new_plugin()
}

pub fn workflow_plugin_id() -> &'static str {
    provided_workflow::WORKFLOW_PLUGIN_ID
}

pub fn new_workflow_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_workflow::new_plugin()
}

pub fn schema_lab_plugin_id() -> &'static str {
    provided_schema_lab::SCHEMA_LAB_PLUGIN_ID
}

pub fn new_schema_lab_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_schema_lab::SchemaLabPlugin::new()
}

pub fn default_tool_host(workspace_root: impl Into<PathBuf>) -> Result<Arc<PluginHost>, String> {
    let workspace_root = workspace_root.into();
    let config =
        crate::plugins::sources::resolve_plugin_config(crate::plugin::PluginsConfig::default());
    mcp::block_on(async move {
        let mcp_config =
            mcp::config_from_plugins(&config).map_err(crate::plugin::HostError::Config)?;
        let mcp_manager = mcp::build_manager(&mcp_config).await;
        let builder =
            PluginHostBuilder::new(workspace_root, env!("CARGO_PKG_VERSION")).with_config(config);
        crate::plugins::sources::register_static_transports(builder, Some(mcp_manager))
            .build()
            .await
    })
    .map_err(|err| err.to_string())
}
/// Stable id used to register configured MCP servers as plugin tools.
pub fn mcp_plugin_id() -> &'static str {
    mcp::MCP_PLUGIN_ID
}

/// Construct the in-process plugin that exposes configured MCP server tools.
pub fn new_mcp_plugin(
    manager: Arc<agena_mcp_client::McpConnectionManager>,
) -> impl crate::plugin::sdk::Plugin {
    mcp::McpPlugin::new(manager)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionCheck {
    pub action: PermissionAction,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedToolInvocation {
    pub invocation: ToolInvocation,
    pub title_override: Option<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedShellCommand {
    pub command: String,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionEnforcementMode {
    Enforced,
    Bypassed,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ToolRuntimeContext {
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
    pub session_context: Option<crate::session::SessionExecutionContext>,
    pub prepared_shell_command: Option<PreparedShellCommand>,
}

static SYNTHETIC_TOOL_CALL_ID: AtomicI64 = AtomicI64::new(-1);

pub struct StreamingToolExecution {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<crate::plugin::sdk::ToolStreamChunk>,
    pub end: tokio::sync::oneshot::Receiver<Result<ToolInvocationExecution, ToolError>>,
    _executor_guard: Option<in_process_router::ExecutorContextGuard>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("permission confirmation required: {0}")]
    PermissionAsk(String),
    #[error("user input required")]
    UserInputRequired(AskUserToolInput),
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    #[error("invalid tool input: {0}")]
    InvalidInput(String),
    #[error("invalid glob pattern: {0}")]
    InvalidGlobPattern(#[from] globset::Error),
    #[error("invalid regex pattern: {0}")]
    InvalidRegexPattern(#[from] regex::Error),
    #[error("shell error: {0}")]
    Shell(#[from] ShellError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin error: {0}")]
    Plugin(String),
    #[error("unknown tool: {tool}")]
    UnknownTool { tool: String },
    #[error("{suggestion_text}")]
    UnknownToolHint {
        tool: String,
        suggestions: Vec<String>,
        suggestion_text: String,
    },
    #[error("unsupported tool invocation in executor: {0}")]
    UnsupportedInvocation(String),
}

fn present_registered_tool(
    mut registered_tool: RegisteredTool,
    presentation: &crate::plugin::ToolPresentationConfig,
) -> RegisteredTool {
    let mode = presentation.mode_for(
        registered_tool.plugin_name.as_str(),
        registered_tool.original_name.as_str(),
        registered_tool.exposed_name.as_str(),
        registered_tool.decl.preferred_description_mode(),
    );
    if mode == crate::plugin::ToolDescriptionMode::Brief {
        registered_tool.decl.description = Some(compact_tool_description(&registered_tool));
    }
    registered_tool.decl.help = None;
    registered_tool
}

fn compact_tool_description(registered_tool: &RegisteredTool) -> String {
    let summary = tool_summary_sentence(registered_tool);
    if let Some(base_tool) = registered_tool.base_exposed_name.as_deref() {
        let alias_note = registered_tool
            .fixed_input
            .as_ref()
            .and_then(fixed_input_summary)
            .map(|summary| format!(" Alias fixed to {summary}."))
            .unwrap_or_default();
        return format!("{summary} See `tools.help` for `{base_tool}`.{alias_note}");
    }
    format!(
        "{summary} See `tools.help` for `{}`.",
        registered_tool.exposed_name
    )
}

fn tool_summary_sentence(registered_tool: &RegisteredTool) -> String {
    let summary = tool_summary(registered_tool);
    if matches!(summary.chars().last(), Some('.' | '!' | '?')) {
        return summary;
    }
    format!("{summary}.")
}

fn tool_summary(registered_tool: &RegisteredTool) -> String {
    if registered_tool.base_exposed_name.is_some() {
        return registered_tool
            .description_text()
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("Tool `{}`.", registered_tool.exposed_name));
    }
    if let Some(summary) = registered_tool.summary_text() {
        return summary.to_string();
    }
    registered_tool
        .description_text()
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Tool `{}`.", registered_tool.exposed_name))
}

#[derive(Clone)]
pub struct ToolExecutor {
    workspace_root: PathBuf,
    agent: Agent,
    model_id: Option<String>,
    subagent_registry: crate::agents::SubagentRegistry,
    monitor_registry: Option<Arc<dyn MonitorService>>,
    truncator: ToolOutputTruncator,
    plugins: Arc<PluginHost>,
    worktree_registry: Option<worktree::WorktreeRegistry>,
    scheduler: Option<Arc<agena_scheduler::Scheduler>>,
    lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    permission_mode: PermissionEnforcementMode,
    tool_presentation: crate::plugin::ToolPresentationConfig,
}

impl ToolExecutor {
    pub fn new(workspace_root: impl Into<PathBuf>, agent: Agent) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
            model_id: None,
            subagent_registry: crate::agents::SubagentRegistry::empty(),
            monitor_registry: monitor::default_registry(),
            truncator: ToolOutputTruncator::default(),
            plugins: PluginHost::new_empty(),
            worktree_registry: None,
            scheduler: None,
            lsp_registry: None,
            permission_mode: PermissionEnforcementMode::Enforced,
            tool_presentation: crate::plugin::ToolPresentationConfig::default(),
        }
    }

    pub fn with_monitor_registry(mut self, registry: Arc<dyn MonitorService>) -> Self {
        self.monitor_registry = Some(registry);
        self
    }

    pub fn without_monitor_registry(mut self) -> Self {
        self.monitor_registry = None;
        self
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_subagent_registry(mut self, registry: crate::agents::SubagentRegistry) -> Self {
        self.subagent_registry = registry;
        self
    }

    pub fn subagent_registry(&self) -> &crate::agents::SubagentRegistry {
        &self.subagent_registry
    }

    pub fn with_plugin_manager(mut self, manager: Arc<PluginHost>) -> Self {
        self.plugins = manager;
        self
    }

    pub fn with_tool_presentation(
        mut self,
        presentation: crate::plugin::ToolPresentationConfig,
    ) -> Self {
        self.tool_presentation = presentation;
        self
    }

    pub fn with_worktree_registry(mut self, reg: worktree::WorktreeRegistry) -> Self {
        self.worktree_registry = Some(reg);
        self
    }

    pub fn worktree_registry(&self) -> Option<&worktree::WorktreeRegistry> {
        self.worktree_registry.as_ref()
    }

    pub fn with_scheduler(mut self, scheduler: Arc<agena_scheduler::Scheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub fn scheduler(&self) -> Option<&Arc<agena_scheduler::Scheduler>> {
        self.scheduler.as_ref()
    }

    pub fn with_lsp_registry(mut self, registry: Arc<agena_lsp::LspRegistry>) -> Self {
        self.lsp_registry = Some(registry);
        self
    }

    pub fn lsp_registry(&self) -> Option<&Arc<agena_lsp::LspRegistry>> {
        self.lsp_registry.as_ref()
    }

    pub fn with_truncation_policy(mut self, policy: ToolOutputTruncationPolicy) -> Self {
        self.truncator = ToolOutputTruncator::new(policy);
        self
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn for_session_context(
        &self,
        session_context: &crate::session::SessionExecutionContext,
    ) -> Self {
        let mut scoped = self.clone();
        if let Some(root) = session_context.effective_workspace_root.as_ref() {
            scoped.workspace_root = root.clone();
        }
        if !session_context.effective_permission.is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .with_permission_config(&session_context.effective_permission);
        }
        if !session_context.allowed_tools.is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .with_allowed_tools(session_context.allowed_tools.iter().map(String::as_str));
        }
        if let Some(model_id) = session_context.selection.model.as_ref() {
            scoped.model_id = Some(model_id.clone());
        }
        scoped
    }

    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    pub fn monitor_registry(&self) -> Option<&Arc<dyn MonitorService>> {
        self.monitor_registry.as_ref()
    }

    pub fn plugin_manager(&self) -> &Arc<PluginHost> {
        &self.plugins
    }

    pub fn tool_catalog(&self) -> ToolCatalog {
        ToolCatalog::for_model(self.model_id.as_deref())
    }

    fn registered_tools_with_definition_overrides(&self) -> Vec<RegisteredTool> {
        let mut tools = self
            .plugins
            .registered_tools()
            .into_iter()
            .collect::<Vec<_>>();

        tools.sort_by(|left, right| {
            left.exposed_name
                .cmp(&right.exposed_name)
                .then_with(|| left.description_text().cmp(right.description_text()))
        });

        // Plugin chain: tool.definition. Let plugins rewrite descriptions /
        // input schemas before the list reaches the LLM.
        if !self.plugins.is_empty() {
            tools = tools
                .into_iter()
                .map(|mut entry| {
                    let input = PluginToolDefinitionInput {
                        tool_name: entry.original_name.clone(),
                        plugin_name: entry.plugin_name.clone(),
                        description: entry.description_text().to_string(),
                        summary: entry.decl.summary.clone(),
                        help: entry.decl.help.clone(),
                        description_mode: entry.decl.description_mode,
                        input_schema: entry.sanitized_input_schema(),
                    };
                    match self.plugins.dispatch_tool_definition_blocking(input) {
                        Ok(patched) => {
                            entry.decl.description = Some(patched.description);
                            entry.decl.summary = patched.summary;
                            entry.decl.help = patched.help;
                            entry.decl.description_mode = patched.description_mode;
                            entry.decl.input_schema = patched.input_schema;
                            entry
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_plugin_host::tool_definition",
                                tool = %entry.exposed_name,
                                "tool.definition hook failed (keeping original): {err}"
                            );
                            entry
                        }
                    }
                })
                .collect();
        }

        tools
    }

    fn catalogued_tools_raw(&self) -> Vec<RegisteredTool> {
        let catalog = self.tool_catalog();
        self.registered_tools_with_definition_overrides()
            .into_iter()
            .filter(|entry| catalog.is_tool_enabled(entry))
            .collect()
    }

    fn catalogued_model_tools_raw(&self) -> Vec<RegisteredTool> {
        let mut used_exposed_names = self
            .plugins
            .registered_tools()
            .into_iter()
            .map(|tool| tool.exposed_name.clone())
            .collect::<BTreeSet<_>>();
        let mut expanded = Vec::new();
        for tool in self.registered_tools_with_definition_overrides() {
            expand_registered_tool_for_model(&tool, &mut used_exposed_names, &mut expanded);
        }
        let catalog = self.tool_catalog();
        expanded.retain(|entry| catalog.is_tool_enabled(entry));
        expanded.sort_by(|left, right| {
            left.exposed_name
                .cmp(&right.exposed_name)
                .then_with(|| left.description_text().cmp(right.description_text()))
        });
        expanded
    }

    fn catalogued_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools_raw()
            .into_iter()
            .map(|entry| present_registered_tool(entry, &self.tool_presentation))
            .collect()
    }

    fn catalogued_model_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_model_tools_raw()
            .into_iter()
            .map(|entry| present_registered_tool(entry, &self.tool_presentation))
            .collect()
    }

    pub fn detailed_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools_raw()
    }

    pub fn searchable_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools()
    }

    pub fn available_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools()
    }

    pub fn available_model_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_model_tools()
    }

    fn suggested_tool_names(&self, requested: &str) -> Vec<String> {
        let mut candidates = self
            .catalogued_tools_raw()
            .into_iter()
            .flat_map(|tool| {
                let mut names = tool.alias_exposed_names();
                names.insert(0, tool.exposed_name);
                names
            })
            .collect::<Vec<_>>();
        candidates.extend(
            self.catalogued_model_tools_raw()
                .into_iter()
                .flat_map(|tool| {
                    let mut names = tool.alias_exposed_names();
                    names.insert(0, tool.exposed_name);
                    names
                }),
        );
        candidates.sort();
        candidates.dedup();
        suggest_tool_names(requested, candidates, 1)
    }

    fn unknown_tool_error(&self, requested: &str) -> ToolError {
        let suggestions = self.suggested_tool_names(requested);
        if suggestions.is_empty() {
            ToolError::UnknownTool {
                tool: requested.to_string(),
            }
        } else {
            unknown_tool_hint(requested, suggestions)
        }
    }

    pub fn is_concurrency_safe_invocation(&self, invocation: &ToolInvocation) -> bool {
        let invocation = PluginInvocation::from_tool_invocation(invocation);
        let Some(entry) = self.plugin_invocation_definition(&invocation) else {
            return false;
        };
        entry.decl.concurrency_safe
            && !entry.has_tag(crate::plugin::sdk::ToolTag::Interactive)
            && is_concurrency_safe_tool_invocation(&entry, &invocation)
    }

    pub fn available_tools_for_messages(&self, messages: &[Message]) -> Vec<RegisteredTool> {
        let _ = messages;
        self.available_tools()
    }

    fn invocation_definition(&self, invocation: &ToolInvocation) -> Option<RegisteredTool> {
        self.plugin_invocation_definition(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_definition(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<RegisteredTool> {
        self.catalogued_tools()
            .into_iter()
            .find(|entry| tool_matches_model_name(entry, invocation.tool_name.as_str()))
            .or_else(|| {
                self.catalogued_model_tools()
                    .into_iter()
                    .find(|entry| tool_matches_model_name(entry, invocation.tool_name.as_str()))
            })
            .or_else(|| {
                let canonical = canonical_tool_name(invocation.tool_name.as_str());
                self.catalogued_tools()
                    .into_iter()
                    .find(|entry| tool_matches_model_name(entry, canonical))
            })
            .or_else(|| {
                let canonical = canonical_tool_name(invocation.tool_name.as_str());
                self.catalogued_model_tools()
                    .into_iter()
                    .find(|entry| tool_matches_model_name(entry, canonical))
            })
    }

    fn invocation_plugin_name_for(&self, invocation: &ToolInvocation) -> String {
        self.plugin_invocation_plugin_name_for(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_plugin_name_for(&self, invocation: &PluginInvocation) -> String {
        if let Some(entry) = self.plugin_invocation_definition(invocation) {
            return entry.plugin_name;
        }

        self.plugins
            .lookup_tool(invocation.tool_name.as_str())
            .map(|entry| entry.plugin_name)
            .unwrap_or_else(|| "custom".to_string())
    }

    fn invocation_streaming_mode(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<SdkToolStreamingMode> {
        self.plugin_invocation_streaming_mode(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_streaming_mode(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<SdkToolStreamingMode> {
        self.plugin_resolution_for_plugin_invocation(invocation)
            .map(|entry| entry.decl.streaming)
    }

    fn authorize_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<(String, PermissionDecision), ToolError> {
        let tool_name = invocation_name(invocation);
        let definition = self
            .invocation_definition(invocation)
            .ok_or_else(|| self.unknown_tool_error(tool_name.as_str()))?;
        let tags = invocation_effective_tags(&definition, invocation);
        if !self.tool_catalog().are_tags_enabled(&tags) {
            return Err(ToolError::PermissionDenied(format!(
                "tool '{tool_name}' disabled for current model profile"
            )));
        }
        let command = shell_command_from_invocation(invocation);
        let resolution = self.plugin_resolution_for_invocation(invocation);
        let mut tool_name_aliases = vec![tool_name.as_str()];
        if let Some(resolution) = resolution.as_ref()
            && resolution.original_name != tool_name
            && self.original_tool_name_is_unambiguous(resolution.original_name.as_str())
        {
            tool_name_aliases.push(resolution.original_name.as_str());
        }
        Ok((
            tool_name.clone(),
            self.agent
                .authorize_tool_aliases(&tool_name_aliases, command.as_deref(), &tags),
        ))
    }

    fn original_tool_name_is_unambiguous(&self, original_name: &str) -> bool {
        self.plugins
            .registered_tools()
            .into_iter()
            .filter(|tool| tool.original_name == original_name)
            .take(2)
            .count()
            == 1
    }

    fn plugin_resolution_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<crate::plugin::registry::RegisteredTool> {
        self.plugin_resolution_for_plugin_invocation(&PluginInvocation::from_tool_invocation(
            invocation,
        ))
    }

    fn plugin_resolution_for_plugin_invocation(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<crate::plugin::registry::RegisteredTool> {
        self.plugins
            .lookup_tool(invocation.tool_name.as_str())
            .or_else(|| {
                self.plugins
                    .lookup_tool(canonical_tool_name(invocation.tool_name.as_str()))
            })
            .or_else(|| {
                self.plugins
                    .registered_tools()
                    .into_iter()
                    .find(|tool| tool_matches_model_name(tool, invocation.tool_name.as_str()))
            })
            .or_else(|| {
                self.catalogued_model_tools_raw()
                    .into_iter()
                    .find(|tool| tool_matches_model_name(tool, invocation.tool_name.as_str()))
            })
    }

    fn collect_declared_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        input: &serde_json::Value,
        specs: &[SdkInputPathSpec],
        static_specs: &[SdkPathAccessSpec],
    ) -> Result<(), ToolError> {
        for spec in static_specs {
            self.push_requested_path_checks(checks, spec.path.as_str(), spec.kind);
        }
        for path_request in extract_input_path_requests(input, specs)? {
            self.push_requested_path_checks(checks, &path_request.path, path_request.kind);
        }
        Ok(())
    }

    fn collect_dynamic_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        registered_tool: &crate::plugin::registry::RegisteredTool,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_paths(
            registered_tool,
            PluginToolPermissionPathsInput {
                tool_name: registered_tool.original_name.clone(),
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: input.clone(),
            },
        );

        let path_requests = match result {
            Ok(path_requests) => path_requests,
            Err(err)
                if err.code == crate::plugin::sdk::PluginErrorCode::NotImplemented
                    || err.message.contains("method not found")
                    || err.message.contains("not implemented") =>
            {
                return Ok(());
            }
            Err(err) if err.code == crate::plugin::sdk::PluginErrorCode::InvalidParams => {
                return Err(ToolError::InvalidInput(err.message));
            }
            Err(err) => return Err(ToolError::Plugin(err.message)),
        };

        for path_request in path_requests {
            self.push_requested_path_checks(checks, &path_request.path, path_request.kind);
        }
        Ok(())
    }

    fn collect_declared_network_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        input: &serde_json::Value,
        input_specs: &[SdkInputNetworkSpec],
        static_specs: &[SdkNetworkAccessSpec],
    ) -> Result<(), ToolError> {
        for spec in static_specs {
            self.push_network_check(checks, spec.target.as_str())?;
        }
        for request in extract_input_network_requests(input, input_specs)? {
            self.push_network_check(checks, request.target.as_str())?;
        }
        Ok(())
    }

    fn collect_dynamic_network_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        registered_tool: &crate::plugin::registry::RegisteredTool,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_networks(
            registered_tool,
            PluginToolPermissionNetworksInput {
                tool_name: registered_tool.original_name.clone(),
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: input.clone(),
            },
        );

        let network_requests = match result {
            Ok(network_requests) => network_requests,
            Err(err)
                if err.code == crate::plugin::sdk::PluginErrorCode::NotImplemented
                    || err.message.contains("method not found")
                    || err.message.contains("not implemented") =>
            {
                return Ok(());
            }
            Err(err) if err.code == crate::plugin::sdk::PluginErrorCode::InvalidParams => {
                return Err(ToolError::InvalidInput(err.message));
            }
            Err(err) => return Err(ToolError::Plugin(err.message)),
        };

        for request in network_requests {
            self.push_network_check(checks, request.target.as_str())?;
        }
        Ok(())
    }

    fn collect_declared_filesystem_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        if let Some(effects) = filesystem_effects_from_input(input)? {
            let command = input
                .pointer("/args/command")
                .or_else(|| {
                    input.get("command").filter(|value| {
                        !matches!(
                            value.as_str(),
                            Some(
                                "bash"
                                    | "powershell"
                                    | "exec"
                                    | "monitor"
                                    | "monitor_start"
                                    | "monitor_list"
                                    | "monitor_read"
                                    | "monitor_stop"
                            )
                        )
                    })
                })
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !command.is_empty() {
                validate_shell_filesystem_effects(tool_name, command, effects.as_slice())?;
            }
            let workdir = input
                .get("workdir")
                .or_else(|| input.pointer("/args/workdir"))
                .and_then(serde_json::Value::as_str);
            let base = self.shell_effect_base_path(workdir);
            self.push_filesystem_effect_checks(checks, effects.as_slice(), base.as_path());
        }
        Ok(())
    }

    fn push_requested_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        path: &str,
        kind: SdkPathKind,
    ) {
        let target = self.resolve_target_path(path);
        self.push_path_checks(checks, sdk_path_kind_to_access_kind(kind), &target);
    }

    pub(crate) fn requested_path_permission_check(
        &self,
        path: &str,
        kind: SdkPathKind,
    ) -> ToolPermissionCheck {
        let mut checks = Vec::with_capacity(1);
        self.push_requested_path_checks(&mut checks, path, kind);
        checks.remove(0)
    }

    fn push_filesystem_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        effects: &[FilesystemEffect],
        base_path: &Path,
    ) {
        for effect in effects {
            let target = self.resolve_filesystem_effect_path(effect.path.as_str(), base_path);
            if effect.access.includes_read() {
                self.push_path_checks(checks, AccessKind::Read, &target);
            }
            if effect.access.includes_write() {
                self.push_path_checks(checks, AccessKind::Write, &target);
            }
        }
    }

    pub fn execute_tool_payload_detailed(
        &self,
        input: &ToolPayloadInput,
    ) -> Result<ToolPayloadExecution, ToolError> {
        self.execute_tool_payload_detailed_with_context(input, ToolRuntimeContext::default())
    }

    pub fn execute_tool_payload_output_for_session(
        &self,
        input: &ToolPayloadInput,
        session_id: i64,
    ) -> Result<ToolPayloadOutput, ToolError> {
        self.execute_tool_payload_detailed_with_context(
            input,
            ToolRuntimeContext {
                session_id: Some(session_id),
                call_id: None,
                session_context: None,
                prepared_shell_command: None,
            },
        )
        .map(|execution| execution.output)
    }

    pub fn execute_tool_payload_for_host(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
        call_id: Option<i64>,
        session_context: Option<&crate::session::SessionExecutionContext>,
    ) -> Result<crate::plugin::ToolInvokeOutput, ToolError> {
        let scoped_executor = session_context
            .map(|session_context| self.for_session_context(session_context))
            .unwrap_or_else(|| self.clone());
        let execution = orchestrator::execute_tool(
            &scoped_executor,
            tool_name,
            input,
            ToolRuntimeContext {
                session_id,
                call_id,
                session_context: None,
                prepared_shell_command: None,
            },
        )?;
        Ok(in_process_router::tool_execution_to_invoke_output(
            scoped_executor.truncator.apply(execution),
        ))
    }

    fn execute_tool_payload_detailed_with_context(
        &self,
        input: &ToolPayloadInput,
        context: ToolRuntimeContext,
    ) -> Result<ToolPayloadExecution, ToolError> {
        let scoped_executor = context
            .session_context
            .as_ref()
            .map(|session_context| self.for_session_context(session_context))
            .unwrap_or_else(|| self.clone());
        let invocation = input.clone().into_invocation();
        let tool_name = input.tool_name();
        let definition = scoped_executor
            .invocation_definition(&invocation)
            .ok_or_else(|| scoped_executor.unknown_tool_error(tool_name))?;
        if !scoped_executor.tool_catalog().is_tool_enabled(&definition) {
            return Err(ToolError::UnsupportedInvocation(tool_name.to_string()));
        }

        if scoped_executor.permission_mode == PermissionEnforcementMode::Enforced {
            for check in scoped_executor.collect_permission_checks_for_invocation_in_session(
                &invocation,
                context.session_id,
            )? {
                match check.decision {
                    PermissionDecision::Allow => {}
                    PermissionDecision::Ask { reason } => {
                        return Err(ToolError::PermissionAsk(reason));
                    }
                    PermissionDecision::Deny { reason } => {
                        return Err(ToolError::PermissionDenied(reason));
                    }
                }
            }
        }
        let session_id = context.session_id.unwrap_or(-1);
        let call_id = context
            .call_id
            .unwrap_or_else(|| SYNTHETIC_TOOL_CALL_ID.fetch_sub(1, Ordering::Relaxed));
        let execution = scoped_executor.execute_invocation_detailed_inner(
            &invocation,
            session_id,
            call_id,
            context.prepared_shell_command,
        )?;
        let output =
            ToolPayloadOutput::from_tool_output(tool_name, &execution.output).ok_or_else(|| {
                ToolError::Plugin(format!(
                    "decode {tool_name} output: payload did not match tool payload schema"
                ))
            })?;
        Ok(scoped_executor.truncator.apply(ToolPayloadExecution {
            output,
            view: execution.view,
            apply_patch: execution.apply_patch,
        }))
    }

    pub fn collect_permission_checks(
        &self,
        input: &ToolPayloadInput,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.collect_permission_checks_for_invocation_in_session(
            &input.clone().into_invocation(),
            None,
        )
    }

    pub fn prepare_shell_command(
        &self,
        input: &crate::message::ShellCommandInput,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<PreparedShellCommand>, ToolError> {
        bash::prepare_command(self, input, session_id, call_id)
    }

    pub fn prepare_bash_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<(ToolInvocation, Option<PreparedShellCommand>), ToolError> {
        let Some(ToolPayloadInput::Bash(bash_input)) =
            ToolPayloadInput::from_invocation(invocation)
        else {
            return Ok((invocation.clone(), None));
        };
        let prepared_shell = self.prepare_shell_command(&bash_input, session_id, call_id)?;
        let Some(prepared_shell) = prepared_shell.clone() else {
            return Ok((invocation.clone(), None));
        };
        if prepared_shell.command == bash_input.command {
            return Ok((invocation.clone(), Some(prepared_shell)));
        }
        let mut rewritten = bash_input;
        rewritten.command = prepared_shell.command.clone();
        let input_value = if invocation.name == "bash" {
            serde_json::to_value(rewritten)
                .map_err(|err| ToolError::InvalidInput(format!("bash input: {err}")))?
        } else {
            let rewritten_invocation = ToolPayloadInput::Bash(rewritten).into_invocation();
            serde_json::Value::from(rewritten_invocation.input)
        };
        let input = StructuredObject::try_from(input_value)
            .map_err(|err| ToolError::InvalidInput(err.to_string()))?;
        Ok((
            ToolInvocation {
                name: invocation.name.clone(),
                plugin_name: invocation.plugin_name.clone(),
                input,
            },
            Some(prepared_shell),
        ))
    }

    pub fn prepare_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<PreparedToolInvocation, ToolError> {
        let exposed_tool_name = invocation_name(invocation).to_owned();
        let definition = self.invocation_definition(invocation);
        let plugin_name = self.invocation_plugin_name_for(invocation);
        if definition.is_none() {
            let mut prepared_invocation = invocation.clone();
            prepared_invocation.plugin_name = Some(plugin_name);
            return Ok(PreparedToolInvocation {
                invocation: prepared_invocation,
                title_override: None,
                metadata: Default::default(),
            });
        }
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.original_name)
            .unwrap_or_else(|| exposed_tool_name.clone());
        let input_json = invocation_input_json(invocation)?;
        let parsed_input_value: serde_json::Value = serde_json::from_str(&input_json)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;
        let input_value = definition
            .as_ref()
            .map(|definition| {
                merge_fixed_tool_input(parsed_input_value.clone(), definition.fixed_input.as_ref())
            })
            .unwrap_or(parsed_input_value);

        let effective_tags = definition
            .as_ref()
            .map(|definition| invocation_effective_tags(definition, invocation))
            .unwrap_or_default();

        let hooked = self
            .plugins
            .dispatch_tool_before(PluginToolBeforeInput {
                tool_name: hook_tool_name,
                plugin_name: plugin_name.clone(),
                session_id,
                call_id,
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                tags: effective_tags,
                input: input_value,
                title_override: None,
                metadata: Default::default(),
            })
            .map_err(|err| ToolError::Plugin(err.message))?;

        let input_json = serde_json::to_string(&hooked.input)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;

        let mut prepared_invocation =
            parse_invocation_from_json(exposed_tool_name.as_str(), input_json.as_str())?;
        prepared_invocation.plugin_name = Some(plugin_name);

        Ok(PreparedToolInvocation {
            invocation: prepared_invocation,
            title_override: hooked.title_override,
            metadata: hooked.metadata.into_iter().collect(),
        })
    }

    pub fn collect_permission_checks_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.collect_permission_checks_for_invocation_in_session(invocation, None)
    }

    pub fn collect_permission_checks_for_invocation_in_session(
        &self,
        invocation: &ToolInvocation,
        _session_id: Option<i64>,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        let (tool_name, decision) = self.authorize_invocation(invocation)?;
        let command = shell_command_from_invocation(invocation);
        let action = crate::permission::tool_action(
            tool_name.as_str(),
            command.as_deref(),
            Some(&self.agent.tool_policy),
        );
        let mut checks = vec![ToolPermissionCheck { action, decision }];

        if let Some(resolution) = self.plugin_resolution_for_invocation(invocation) {
            let input_value = resolved_tool_input_value(&resolution, invocation);
            if resolution.has_tag(crate::plugin::sdk::ToolTag::Shell) {
                self.collect_declared_filesystem_effect_checks(
                    &mut checks,
                    tool_name.as_str(),
                    &input_value,
                )?;
            }
            self.collect_declared_path_checks(
                &mut checks,
                &input_value,
                &resolution.decl.input_paths,
                &resolution.decl.path_access,
            )?;
            self.collect_dynamic_path_checks(&mut checks, &resolution, &input_value)?;
            self.collect_declared_network_checks(
                &mut checks,
                &input_value,
                &resolution.decl.input_networks,
                &resolution.decl.network_access,
            )?;
            self.collect_dynamic_network_checks(&mut checks, &resolution, &input_value)?;
        }
        Ok(checks)
    }

    pub async fn execute_invocation_streaming(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<StreamingToolExecution>, ToolError> {
        if !matches!(
            self.invocation_streaming_mode(invocation),
            Some(SdkToolStreamingMode::Streaming)
        ) {
            return Ok(None);
        }
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);

        let resolution = self
            .plugin_resolution_for_plugin_invocation(&plugin_invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;
        let executor_guard = in_process_router::install_executor_context(
            self,
            session_id,
            call_id,
            resolution.original_name.clone(),
        );
        let stream = self
            .plugins
            .invoke_tool_stream(
                &resolution,
                PluginToolInvokeInput {
                    tool_name: resolution.original_name.clone(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
                },
            )
            .await
            .map_err(|err| ToolError::Plugin(err.message))?;
        let stream_id = stream.stream_id;
        let chunks = stream.chunks;
        let end = stream.end;
        let result_policy = resolution.decl.result_policy.clone();
        let exposed_tool_name = resolution.exposed_name.clone();
        let executor = self.clone();
        let invocation = invocation.clone();
        let (end_tx, end_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = match end.await {
                Ok(Ok(end)) => (|| {
                    let view = ToolExecutionView {
                        title: end.title,
                        output_text: end.output_text,
                        metadata: end.metadata.into_iter().collect(),
                        attachments: end.attachments,
                    };
                    let output = ToolOutput::from_json_payload(end.payload.as_ref())
                        .map_err(ToolError::InvalidInput)?;
                    let mut execution = ToolInvocationExecution::new(output.clone(), view)
                        .with_apply_patch_option(apply_patch_execution_from_tool_output(&output));
                    executor.apply_after_hooks(&invocation, session_id, call_id, &mut execution)?;
                    executor.apply_result_policy(
                        exposed_tool_name.as_str(),
                        &result_policy,
                        call_id,
                        &mut execution,
                    )?;
                    Ok(execution)
                })(),
                Ok(Err(err)) => Err(ToolError::Plugin(err.message)),
                Err(_) => Err(ToolError::Plugin(
                    "stream ended without a terminal frame".to_string(),
                )),
            };
            let _ = end_tx.send(result);
        });
        Ok(Some(StreamingToolExecution {
            stream_id,
            chunks,
            end: end_rx,
            _executor_guard: Some(executor_guard),
        }))
    }

    pub fn execute_invocation_detailed(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_with_prepared_shell(invocation, session_id, call_id, None)
    }

    pub fn execute_invocation_detailed_with_prepared_shell(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let result = self.execute_invocation_detailed_inner(
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
        );
        crate::metrics::record_tool_execution(result.is_ok());
        result
    }

    fn execute_invocation_detailed_inner(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        _prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);
        let tool_name = plugin_invocation_name(&plugin_invocation);
        let _tool_span =
            tracing::info_span!("tool.call", session_id, call_id, tool = tool_name.as_str(),)
                .entered();
        let resolution = self
            .plugin_resolution_for_plugin_invocation(&plugin_invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;
        let _executor_guard = in_process_router::install_executor_context(
            self,
            session_id,
            call_id,
            resolution.original_name.clone(),
        );

        let response = self
            .plugins
            .invoke_tool(
                &resolution,
                PluginToolInvokeInput {
                    tool_name: resolution.original_name.clone(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
                },
            )
            .map_err(|err| ToolError::Plugin(err.message))?;

        let view = ToolExecutionView {
            title: response.title.clone(),
            output_text: response.output_text.clone(),
            metadata: response.metadata.into_iter().collect(),
            attachments: response.attachments,
        };
        let output = ToolOutput::from_json_payload(response.payload.as_ref())
            .map_err(ToolError::InvalidInput)?;
        let mut execution = ToolInvocationExecution::new(output.clone(), view)
            .with_apply_patch_option(apply_patch_execution_from_tool_output(&output));
        self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
        self.apply_result_policy(
            resolution.exposed_name.as_str(),
            &resolution.decl.result_policy,
            call_id,
            &mut execution,
        )?;
        Ok(execution)
    }

    pub fn execute_invocation_detailed_bypassing_permissions(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
            invocation, session_id, call_id, None,
        )
    }

    pub fn execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let mut trusted = self.clone();
        trusted.permission_mode = PermissionEnforcementMode::Bypassed;
        trusted.execute_invocation_detailed_with_prepared_shell(
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
        )
    }

    pub fn shell_env_overrides(
        &self,
        cwd: &Path,
        session_id: Option<i64>,
        call_id: Option<i64>,
    ) -> Result<std::collections::HashMap<String, String>, ToolError> {
        let patch = self
            .plugins
            .dispatch_shell_env(PluginShellEnvInput {
                cwd: cwd.to_path_buf(),
                session_id,
                call_id,
            })
            .map_err(|err| ToolError::Plugin(err.message))?;
        Ok(patch.set.into_iter().collect())
    }

    pub fn execute_tool_payload(
        &self,
        input: &ToolPayloadInput,
    ) -> Result<(ToolPayloadOutput, Option<ApplyPatchExecution>), ToolError> {
        let execution = self.execute_tool_payload_detailed(input)?;
        Ok((execution.output, execution.apply_patch))
    }

    fn apply_after_hooks(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        let exposed_tool_name = invocation_name(invocation).to_owned();
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.original_name)
            .unwrap_or(exposed_tool_name);
        let plugin_name = self.invocation_plugin_name_for(invocation);
        let after_in = PluginToolAfterInput {
            tool_name: hook_tool_name,
            plugin_name: plugin_name.clone(),
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            title: execution.view.title.clone(),
            output_text: execution.view.output_text.clone(),
            payload: execution.output.to_json_payload(),
            metadata: execution.view.metadata.clone().into_iter().collect(),
        };

        let hooked = self
            .plugins
            .dispatch_tool_after(after_in)
            .map_err(|err| ToolError::Plugin(err.message))?;

        execution.view.title = hooked.title;
        execution.view.output_text = hooked.output_text;
        for (k, v) in hooked.metadata {
            execution.view.metadata.insert(k, v);
        }

        if let Some(payload_value) = hooked.payload {
            execution.output = ToolOutput::from_json_payload(Some(&payload_value))
                .map_err(ToolError::InvalidInput)?;
        }

        Ok(())
    }

    fn apply_result_policy(
        &self,
        exposed_tool_name: &str,
        policy: &SdkToolResultPolicy,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        if policy.is_default() {
            return Ok(());
        }

        execution.view.metadata.insert(
            "result_policy_ui_render_kind".to_string(),
            format!("{:?}", policy.ui_render_kind).to_ascii_lowercase(),
        );
        if let Some(preview_lines) = policy.preview_lines {
            execution.view.metadata.insert(
                "result_policy_preview_lines".to_string(),
                preview_lines.to_string(),
            );
        }

        let original = execution.view.output_text.clone();
        if original.is_empty() {
            return Ok(());
        }

        let mut preview = original.clone();
        let mut truncated = false;

        if let Some(max_lines) = policy.preview_lines
            && max_lines > 0
        {
            let mut lines = preview.lines();
            let selected = lines.by_ref().take(max_lines).collect::<Vec<_>>();
            if lines.next().is_some() {
                preview = selected.join("\n");
                truncated = true;
            }
        }

        if let Some(max_chars) = policy.max_model_chars
            && max_chars > 0
            && preview.chars().count() > max_chars
        {
            preview = truncate_to_char_count(preview.as_str(), max_chars);
            truncated = true;
        }

        if !truncated {
            return Ok(());
        }

        execution
            .view
            .metadata
            .insert("result_policy_truncated".to_string(), "true".to_string());
        execution.view.metadata.insert(
            "result_policy_original_chars".to_string(),
            original.chars().count().to_string(),
        );
        execution.view.metadata.insert(
            "result_policy_model_chars".to_string(),
            preview.chars().count().to_string(),
        );

        if policy.persist_large_output {
            if let Some(path) = persist_tool_result_output(
                self.workspace_root(),
                exposed_tool_name,
                call_id,
                &original,
            )? {
                execution.view.metadata.insert(
                    "result_policy_persisted_path".to_string(),
                    path.display().to_string(),
                );
                preview.push_str("\n\n[output truncated; full output persisted at ");
                preview.push_str(path.display().to_string().as_str());
                preview.push(']');
            }
        } else {
            preview.push_str("\n\n[output truncated by tool result policy]");
        }

        execution.view.output_text = preview;
        Ok(())
    }

    /// Fire-and-forget notification to plugins about a tool execution failure.
    pub fn broadcast_tool_failure(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        error: &str,
    ) {
        if self.plugins.is_empty() {
            return;
        }
        let exposed_tool_name = invocation_name(invocation).to_owned();
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.original_name)
            .unwrap_or(exposed_tool_name);
        let plugin_name = self.invocation_plugin_name_for(invocation);
        let input_value = invocation_input_json(invocation)
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .unwrap_or(serde_json::Value::Null);
        let failure_input = PluginToolFailureInput {
            tool_name: hook_tool_name,
            plugin_name,
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            input: input_value,
            error: error.to_owned(),
            is_interrupt: false,
        };
        let plugins = Arc::clone(&self.plugins);
        tokio::spawn(async move {
            plugins.broadcast_tool_failure(failure_input).await;
        });
    }

    pub fn broadcast_notification(
        &self,
        kind: impl Into<String>,
        session_id: Option<i64>,
        title: impl Into<String>,
        message: impl Into<String>,
        payload: serde_json::Value,
    ) {
        if self.plugins.is_empty() {
            return;
        }
        let plugins = Arc::clone(&self.plugins);
        let input = crate::plugin::NotificationInput {
            kind: kind.into(),
            session_id,
            title: title.into(),
            message: message.into(),
            payload,
        };
        tokio::spawn(async move {
            plugins.broadcast_notification(input).await;
        });
    }

    pub(crate) fn resolve_target_path(&self, raw_path: &str) -> PathBuf {
        self.resolve_target_path_with_context(raw_path, None)
    }

    pub(crate) fn shell_effect_base_path(&self, workdir: Option<&str>) -> PathBuf {
        workdir
            .map(|workdir| self.resolve_target_path(workdir))
            .unwrap_or_else(|| self.workspace_root().to_path_buf())
    }

    pub(crate) fn resolve_filesystem_effect_path(
        &self,
        raw_path: &str,
        base_path: &Path,
    ) -> PathBuf {
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            candidate
        } else {
            base_path.join(candidate)
        }
    }

    pub(crate) fn resolve_target_path_with_context(
        &self,
        raw_path: &str,
        session_context: Option<&crate::session::SessionExecutionContext>,
    ) -> PathBuf {
        let workspace_root = self.effective_workspace_root(session_context);
        if let Some(path) = resolve_managed_project_path_alias(raw_path, workspace_root) {
            return path;
        }
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            return candidate;
        }
        workspace_root.join(candidate)
    }

    pub(crate) fn execute_shell_command(
        &self,
        request: &ShellRequest,
    ) -> Result<ShellOutput, ToolError> {
        shell::execute(request).map_err(ToolError::from)
    }

    pub(crate) fn effective_workspace_root<'a>(
        &'a self,
        session_context: Option<&'a crate::session::SessionExecutionContext>,
    ) -> &'a Path {
        session_context
            .and_then(|context| context.effective_workspace_root.as_deref())
            .unwrap_or(self.workspace_root())
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        self.display_path_with_context(path, None)
    }

    pub(crate) fn display_path_with_context(
        &self,
        path: &Path,
        session_context: Option<&crate::session::SessionExecutionContext>,
    ) -> String {
        let workspace_root = self.effective_workspace_root(session_context);
        if let Ok(relative) = path.strip_prefix(workspace_root) {
            let normalized = normalize_path_for_display(relative);
            if normalized.is_empty() {
                return ".".to_string();
            }
            return normalized;
        }
        normalize_path_for_display(path)
    }

    pub(crate) fn ensure_read_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Read, target_path)
    }

    pub(crate) fn ensure_edit_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Write, target_path)
    }

    pub(crate) fn ensure_filesystem_effects_permission(
        &self,
        effects: &[FilesystemEffect],
        base_path: &Path,
    ) -> Result<(), ToolError> {
        for effect in effects {
            let target = self.resolve_filesystem_effect_path(effect.path.as_str(), base_path);
            if effect.access.includes_read() {
                self.ensure_access_permission(AccessKind::Read, &target)?;
            }
            if effect.access.includes_write() {
                self.ensure_access_permission(AccessKind::Write, &target)?;
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_network_effects_permission(
        &self,
        effects: &[NetworkEffect],
    ) -> Result<(), ToolError> {
        for effect in effects {
            let target = NetworkTarget::parse(effect.target.as_str()).map_err(|err| {
                ToolError::InvalidInput(format!(
                    "invalid network effect target `{}`: {err}",
                    effect.target
                ))
            })?;
            self.ensure_network_permission(&target)?;
        }
        Ok(())
    }

    pub(crate) fn ensure_network_permission(
        &self,
        target: &NetworkTarget,
    ) -> Result<(), ToolError> {
        if self.permission_mode == PermissionEnforcementMode::Bypassed {
            return Ok(());
        }

        match self.agent.authorize_network_connect(target) {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }

    fn ensure_access_permission(
        &self,
        access: AccessKind,
        target_path: &Path,
    ) -> Result<(), ToolError> {
        if self.permission_mode == PermissionEnforcementMode::Bypassed {
            return Ok(());
        }

        match self
            .agent
            .authorize_path_access(access, self.workspace_root(), target_path)
        {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }

    fn push_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        access: AccessKind,
        target_path: &Path,
    ) {
        let workspace_root = normalize_path_for_display(self.workspace_root());
        let target = normalize_path_for_display(target_path);

        checks.push(ToolPermissionCheck {
            action: PermissionAction::PathAccess {
                access_kind: access_kind_name(access).to_string(),
                workspace_root,
                target_path: target,
            },
            decision: self
                .agent
                .authorize_path_access(access, self.workspace_root(), target_path),
        });
    }

    fn push_network_check(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        target: &str,
    ) -> Result<(), ToolError> {
        let target = NetworkTarget::parse(target).map_err(|err| {
            ToolError::InvalidInput(format!(
                "invalid network permission target `{target}`: {err}"
            ))
        })?;
        checks.push(ToolPermissionCheck {
            action: PermissionAction::NetworkAccess {
                target: target.original().to_string(),
                host: target.host().to_string(),
                port: target.port(),
            },
            decision: self.agent.authorize_network_connect(&target),
        });
        Ok(())
    }

    pub(crate) fn network_permission_check(
        &self,
        target: &str,
    ) -> Result<ToolPermissionCheck, ToolError> {
        let mut checks = Vec::with_capacity(1);
        self.push_network_check(&mut checks, target)?;
        Ok(checks.remove(0))
    }
}

pub(crate) fn normalize_path_for_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn truncate_to_char_count(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let Some((idx, _)) = value.char_indices().nth(max_chars) else {
        return value.to_string();
    };
    value[..idx].to_string()
}

fn persist_tool_result_output(
    workspace_root: &Path,
    exposed_tool_name: &str,
    call_id: i64,
    output_text: &str,
) -> Result<Option<PathBuf>, ToolError> {
    if output_text.is_empty() {
        return Ok(None);
    }

    let dir = workspace_root.join(".agena").join("tool-results");
    fs::create_dir_all(&dir)?;
    let digest = blake3::hash(output_text.as_bytes()).to_hex().to_string();
    let short_digest = digest.get(..12).unwrap_or(digest.as_str());
    let safe_tool = model_safe_tool_name(exposed_tool_name).replace("__", "_");
    let call_part = if call_id >= 0 {
        call_id.to_string()
    } else {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().to_string())
            .unwrap_or_else(|_| "synthetic".to_string())
    };
    let path = dir.join(format!("{call_part}-{safe_tool}-{short_digest}.txt"));
    let mut file = fs::File::create(&path)?;
    file.write_all(output_text.as_bytes())?;
    Ok(Some(path))
}

fn access_kind_name(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Read => "read",
        AccessKind::Write => "write",
    }
}

fn validate_shell_filesystem_effects(
    tool_name: &str,
    command: &str,
    effects: &[FilesystemEffect],
) -> Result<(), ToolError> {
    shell_tools::validate_declared_filesystem_effects(tool_name, command, effects)
}

fn shell_command_from_invocation(invocation: &ToolInvocation) -> Option<String> {
    if let Some(payload) = ToolPayloadInput::from_invocation(invocation) {
        let command = match payload {
            ToolPayloadInput::Bash(payload) => Some(payload.command),
            ToolPayloadInput::PowerShell(payload) => Some(payload.command),
            ToolPayloadInput::Monitor(crate::message::MonitorToolInput::Start {
                command, ..
            }) => Some(command.command),
            _ => None,
        };
        if command.is_some() {
            return command;
        }
    }
    let value = invocation_input_value(invocation);
    value
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
}

fn filesystem_effects_from_input(
    input: &serde_json::Value,
) -> Result<Option<Vec<FilesystemEffect>>, ToolError> {
    let Some(value) = input
        .get("filesystem_effects")
        .or_else(|| input.pointer("/args/filesystem_effects"))
    else {
        return Ok(None);
    };
    let effects = serde_json::from_value(value.clone())
        .map_err(|err| ToolError::InvalidInput(format!("filesystem_effects: {err}")))?;
    Ok(Some(effects))
}

fn invocation_name(invocation: &ToolInvocation) -> String {
    plugin_invocation_name(&PluginInvocation::from_tool_invocation(invocation))
}

fn plugin_invocation_name(invocation: &PluginInvocation) -> String {
    invocation.tool_name.clone()
}

fn canonical_tool_name(name: &str) -> &str {
    name
}

fn command_from_input_value(input: &serde_json::Value) -> Option<&str> {
    input.get("action").and_then(serde_json::Value::as_str)
}

fn resolved_tool_input_value(
    registered_tool: &RegisteredTool,
    invocation: &ToolInvocation,
) -> serde_json::Value {
    merge_fixed_tool_input(
        invocation_input_value(invocation),
        registered_tool.fixed_input.as_ref(),
    )
}

fn resolved_plugin_invocation_input_value(
    registered_tool: &RegisteredTool,
    invocation: &PluginInvocation,
) -> serde_json::Value {
    merge_fixed_tool_input(
        plugin_invocation_input_value(invocation),
        registered_tool.fixed_input.as_ref(),
    )
}

fn resolve_managed_project_path_alias(raw_path: &str, workspace_root: &Path) -> Option<PathBuf> {
    let normalized = raw_path.trim().replace('\\', "/");
    let prefix = "~/agena/projects/<workspace>";
    let rest = normalized.strip_prefix(prefix)?;
    let rest = rest.trim_start_matches('/');
    let mut resolved = crate::project_paths::project_state_dir(workspace_root);
    if !rest.is_empty() {
        resolved = resolved.join(rest);
    }
    Some(resolved)
}

fn invocation_effective_tags(
    definition: &RegisteredTool,
    invocation: &ToolInvocation,
) -> Vec<crate::plugin::sdk::ToolTag> {
    let mut tags = definition.effective_tags();
    let input = resolved_tool_input_value(definition, invocation);
    let Some(command) = command_from_input_value(&input) else {
        return tags;
    };

    match (definition.behavior_exposed_name(), command) {
        ("agena_fs__fs", "read" | "glob" | "grep") => {
            set_invocation_access_tags(&mut tags, true, false, true, false)
        }
        ("agena_fs__fs", "apply_patch" | "notebook_edit") => {
            set_invocation_access_tags(&mut tags, false, true, false, true)
        }
        ("agena_settings__settings", "get" | "list" | "validate") => {
            set_invocation_access_tags(&mut tags, true, false, false, false)
        }
        ("agena_settings__settings", "set" | "delete" | "patch") => {
            set_invocation_access_tags(&mut tags, false, true, false, true)
        }
        ("agena_cron__schedule", "list") => {
            set_invocation_access_tags(&mut tags, true, false, false, false)
        }
        ("agena_cron__schedule", "create" | "delete" | "wakeup") => {
            set_invocation_access_tags(&mut tags, false, true, false, false)
        }
        ("agena_shell__monitor", "list" | "read") => {
            set_invocation_access_tags(&mut tags, true, false, false, false)
        }
        ("agena_shell__monitor", "start" | "stop") => {
            set_invocation_access_tags(&mut tags, false, true, false, false)
        }
        ("agena_workflow__session", "get") => {
            set_invocation_access_tags(&mut tags, true, false, false, false)
        }
        ("agena_workflow__session", "rename") => {
            set_invocation_access_tags(&mut tags, false, true, false, false)
        }
        ("agena_mcp__mcp", "list_resources" | "read_resource" | "list_prompts" | "get_prompt") => {
            set_invocation_access_tags(&mut tags, true, false, false, false)
        }
        ("agena_mcp__mcp", "call") => {
            set_invocation_access_tags(&mut tags, false, true, false, false)
        }
        _ => {}
    }

    tags
}

fn set_invocation_access_tags(
    tags: &mut Vec<crate::plugin::sdk::ToolTag>,
    read_only: bool,
    mutating: bool,
    filesystem_read: bool,
    filesystem_write: bool,
) {
    tags.retain(|tag| {
        !matches!(
            tag,
            crate::plugin::sdk::ToolTag::ReadOnly
                | crate::plugin::sdk::ToolTag::Mutating
                | crate::plugin::sdk::ToolTag::FilesystemRead
                | crate::plugin::sdk::ToolTag::FilesystemWrite
        )
    });
    if read_only {
        tags.push(crate::plugin::sdk::ToolTag::ReadOnly);
    }
    if mutating {
        tags.push(crate::plugin::sdk::ToolTag::Mutating);
    }
    if filesystem_read {
        tags.push(crate::plugin::sdk::ToolTag::FilesystemRead);
    }
    if filesystem_write {
        tags.push(crate::plugin::sdk::ToolTag::FilesystemWrite);
    }
}

fn is_concurrency_safe_tool_invocation(
    registered_tool: &RegisteredTool,
    invocation: &PluginInvocation,
) -> bool {
    let input = resolved_plugin_invocation_input_value(registered_tool, invocation);
    let Some(command) = command_from_input_value(&input) else {
        return registered_tool.decl.concurrency_safe;
    };

    match (registered_tool.behavior_exposed_name(), command) {
        ("agena_fs__fs", "read" | "glob" | "grep") => true,
        ("agena_fs__fs", "apply_patch" | "notebook_edit") => false,
        ("agena_shell__monitor", "list" | "read") => true,
        ("agena_shell__monitor", "start" | "stop") => false,
        ("agena_settings__settings", "get" | "list" | "validate") => true,
        ("agena_settings__settings", "set" | "delete" | "patch") => false,
        ("agena_cron__schedule", "list") => true,
        ("agena_cron__schedule", "create" | "delete" | "wakeup") => false,
        ("agena_workflow__session", "get") => true,
        ("agena_workflow__session", "rename") => false,
        ("agena_mcp__mcp", "list_resources" | "read_resource" | "list_prompts" | "get_prompt") => {
            true
        }
        ("agena_mcp__mcp", "call") => false,
        _ => registered_tool.decl.concurrency_safe,
    }
}

fn apply_patch_execution_from_tool_output(output: &ToolOutput) -> Option<ApplyPatchExecution> {
    let payload = output.to_json_payload()?;
    let operation_id = payload.get("operation_id")?.as_str()?.to_string();
    let changes: Vec<crate::message::FileChangeRecord> =
        serde_json::from_value(payload.get("changes")?.clone()).ok()?;
    let before_hash = payload
        .get("before_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let after_hash = payload
        .get("after_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let inverse_patch = payload.get("inverse_patch")?.as_str()?.to_string();
    let diff = payload
        .get("diff")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let progress = serde_json::from_value(
        payload
            .get("progress")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .ok()?;
    Some(ApplyPatchExecution {
        operation_id,
        files: changes
            .into_iter()
            .map(|change| AppliedFileChange {
                path: change.path,
                kind: match change.kind {
                    crate::message::FileChangeKind::Added => apply_patch::PatchOpKind::Add,
                    crate::message::FileChangeKind::Updated => apply_patch::PatchOpKind::Update,
                    crate::message::FileChangeKind::Deleted => apply_patch::PatchOpKind::Delete,
                    crate::message::FileChangeKind::Moved => apply_patch::PatchOpKind::Move,
                },
                from_path: change.from_path,
            })
            .collect(),
        before_hash,
        after_hash,
        inverse_patch,
        diff,
        progress,
    })
}

fn invocation_input_json(invocation: &ToolInvocation) -> Result<String, ToolError> {
    plugin_invocation_input_json(&PluginInvocation::from_tool_invocation(invocation))
}

fn plugin_invocation_input_json(invocation: &PluginInvocation) -> Result<String, ToolError> {
    serde_json::to_string(&serde_json::Value::from(invocation.input.clone()))
        .map_err(|err| ToolError::InvalidInput(err.to_string()))
}

fn invocation_input_value(invocation: &ToolInvocation) -> serde_json::Value {
    plugin_invocation_input_value(&PluginInvocation::from_tool_invocation(invocation))
}

fn plugin_invocation_input_value(invocation: &PluginInvocation) -> serde_json::Value {
    serde_json::Value::from(invocation.input.clone())
}

fn parse_invocation_from_json(
    tool_name: &str,
    input_json: &str,
) -> Result<ToolInvocation, ToolError> {
    let value = if input_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(input_json).map_err(|err| ToolError::InvalidInput(err.to_string()))?
    };
    let input = StructuredObject::try_from(value)
        .map_err(|err| ToolError::InvalidInput(err.to_string()))?;

    Ok(ToolInvocation {
        name: tool_name.to_string(),
        plugin_name: None,
        input,
    })
}

fn sdk_path_kind_to_access_kind(kind: SdkPathKind) -> AccessKind {
    match kind {
        SdkPathKind::Read => AccessKind::Read,
        SdkPathKind::Write => AccessKind::Write,
    }
}

fn extract_input_path_requests(
    input: &serde_json::Value,
    specs: &[SdkInputPathSpec],
) -> Result<Vec<crate::plugin::sdk::PathRequest>, ToolError> {
    let mut requests = Vec::new();
    for spec in specs {
        let matches = extract_jsonpath_values(input, spec.jsonpath.as_str())?;
        if matches.is_empty() {
            if spec.optional {
                continue;
            }
            return Err(ToolError::InvalidInput(format!(
                "missing required input path '{}'",
                spec.jsonpath
            )));
        }
        for value in matches {
            let Some(path) = value.as_str() else {
                return Err(ToolError::InvalidInput(format!(
                    "input path '{}' must resolve to a string",
                    spec.jsonpath
                )));
            };
            requests.push(crate::plugin::sdk::PathRequest {
                path: path.to_string(),
                kind: spec.kind,
            });
        }
    }
    Ok(requests)
}

fn extract_input_network_requests(
    input: &serde_json::Value,
    specs: &[SdkInputNetworkSpec],
) -> Result<Vec<crate::plugin::sdk::NetworkRequest>, ToolError> {
    let mut requests = Vec::new();
    for spec in specs {
        let matches = extract_jsonpath_values(input, spec.jsonpath.as_str())?;
        if matches.is_empty() {
            if spec.optional {
                continue;
            }
            return Err(ToolError::InvalidInput(format!(
                "missing required input network '{}'",
                spec.jsonpath
            )));
        }
        for value in matches {
            let Some(target) = value.as_str() else {
                return Err(ToolError::InvalidInput(format!(
                    "input network '{}' must resolve to a string",
                    spec.jsonpath
                )));
            };
            requests.push(crate::plugin::sdk::NetworkRequest {
                target: target.to_string(),
            });
        }
    }
    Ok(requests)
}

fn extract_jsonpath_values<'a>(
    input: &'a serde_json::Value,
    jsonpath: &str,
) -> Result<Vec<&'a serde_json::Value>, ToolError> {
    let segments = parse_input_jsonpath(jsonpath)?;
    let mut current = vec![input];
    for segment in segments {
        let mut next = Vec::new();
        for value in current {
            match segment {
                InputJsonPathSegment::Key(ref key) => {
                    if let Some(object) = value.as_object()
                        && let Some(child) = object.get(key.as_str())
                    {
                        next.push(child);
                    }
                }
                InputJsonPathSegment::ArrayAll => {
                    if let Some(items) = value.as_array() {
                        next.extend(items.iter());
                    }
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    Ok(current)
}

fn parse_input_jsonpath(jsonpath: &str) -> Result<Vec<InputJsonPathSegment>, ToolError> {
    if jsonpath == "$" {
        return Ok(Vec::new());
    }
    let Some(mut rest) = jsonpath.strip_prefix("$.") else {
        return Err(ToolError::InvalidInput(format!(
            "unsupported input path jsonpath '{jsonpath}'"
        )));
    };

    let mut segments = Vec::new();
    while !rest.is_empty() {
        let key_end = rest.find(['.', '[']).unwrap_or(rest.len());
        let key = &rest[..key_end];
        if key.is_empty() {
            return Err(ToolError::InvalidInput(format!(
                "unsupported input path jsonpath '{jsonpath}'"
            )));
        }
        segments.push(InputJsonPathSegment::Key(key.to_string()));
        rest = &rest[key_end..];

        while let Some(tail) = rest.strip_prefix("[*]") {
            segments.push(InputJsonPathSegment::ArrayAll);
            rest = tail;
        }

        if rest.is_empty() {
            break;
        }
        let Some(tail) = rest.strip_prefix('.') else {
            return Err(ToolError::InvalidInput(format!(
                "unsupported input path jsonpath '{jsonpath}'"
            )));
        };
        rest = tail;
    }

    Ok(segments)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputJsonPathSegment {
    Key(String),
    ArrayAll,
}

#[cfg(test)]
mod tests {

    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use chrono::Utc;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use uuid::Uuid;

    use crate::message::{
        ApplyPatchToolInput, EnterWorktreeToolInput, FileChangeKind, FilesystemAccess,
        FilesystemEffect, GlobToolInput, GrepToolInput, LspDefinitionToolInput,
        LspPositionToolInput, Message, MonitorToolInput, NetworkEffect, NotebookEditMode,
        NotebookEditToolInput, PartContent, ReadToolInput, ShellCommandInput, StructuredObject,
        TaskSubagentType, TaskToolInput, TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput,
        ToolInvocation, WebFetchToolInput, WebSearchToolInput,
    };
    use crate::permission::PermissionPolicy;
    use crate::plugin::sdk::host_api::{
        EventSubscription, HostStorageDeleteRequest, HostStorageGetRequest, HostStorageGetResponse,
        HostStorageListRequest, HostStorageListResponse, HostStorageRecord, HostStorageScope,
        HostStorageSetRequest, HostStorageVisibility, HostTodoPriority, HostTodoStatus, LogLevel,
        SpawnSubtaskRequest, SpawnSubtaskResponse, ToolDescriptor,
    };
    use crate::plugin::sdk::prelude::*;
    use crate::plugin::sdk::{
        EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision, Result as SdkResult,
    };
    use crate::plugin::sdk::{
        ToolArgs as SdkToolArgs, ToolCommand as SdkToolCommand,
        ToolSubcommands as SdkToolSubcommands,
    };
    use crate::plugin::{ConfiguredPlugin, PluginHost, PluginHostBuilder, PluginsConfig};
    use crate::role::Role;

    use super::{
        ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadInput,
        ToolPayloadOutput, ToolRuntimeContext, orchestrator,
    };
    use crate::plugins::provided::router as in_process_router;

    const FS_TOOL: &str = "agena_fs__fs";
    const GENERATED_HELP_TOOL: &str = "fixture__generated_help";
    const MERGED_HELP_TOOL: &str = "fixture__merged_help";
    const SHELL_BASH_TOOL: &str = "agena_shell__exec_bash";
    const TOOLS_TOOL: &str = "agena_workflow__tools";
    const TODO_TOOL: &str = "agena_workflow__todo";
    const TASK_TOOL: &str = "agena_workflow__task";
    const FIXTURE_ECHO_TOOL: &str = "fixture__plugin_echo";
    const WEB_FETCH_TOOL: &str = "agena_web__fetch";
    const WEB_QUERY_TOOL: &str = "agena_web__store_query";
    const WEB_SEARCH_TOOL: &str = "agena_web__search";

    #[derive(Debug, Deserialize, Serialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct DocBackedArgs {
        value: String,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[serde(deny_unknown_fields)]
    struct DocBackedShapeInput {
        /// Search text.
        query: String,
        /// Optional result limit.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[serde(deny_unknown_fields)]
    struct OrderedShapeInput {
        /// Second declared field.
        beta: String,
        /// First declared field.
        alpha: String,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolArgs)]
    #[tool_args(trim("value"), non_empty("value"))]
    #[serde(deny_unknown_fields)]
    struct SdkAliasArgs {
        /// Value normalized by the SDK alias derive.
        value: String,
    }

    /// SDK alias-backed command.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolCommand)]
    #[tool_command(
        tool = "fixture.sdk_alias_command",
        alias = "fixture.sdk_alias_short",
        visible_alias = "fixture.sdk_alias_visible",
        aliases("fixture.sdk_alias_lookup")
    )]
    #[serde(deny_unknown_fields)]
    struct SdkAliasCommandInput {
        /// Value routed through the SDK alias surface.
        value: String,
    }

    #[allow(dead_code)]
    #[derive(Debug, SdkToolSubcommands)]
    enum SdkAliasToolSuite {
        Command(SdkAliasCommandInput),
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct DispatchInnerArgs {
        value: String,
    }

    struct DispatchReceiver;

    impl DispatchReceiver {
        async fn handle_struct(&self, args: &DispatchInnerArgs) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text(format!("struct: {}", args.value)))
        }

        async fn handle_struct_stream(
            &self,
            sink: crate::plugin::sdk::ToolStreamSink,
            args: &DispatchInnerArgs,
        ) -> SdkResult<crate::plugin::sdk::ToolStreamEnd> {
            sink.text(format!("struct-stream:{}", args.value)).await;
            Ok(crate::plugin::sdk::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "Struct Stream".to_string(),
                output_text: format!("struct-stream: {}", args.value),
                payload: None,
                metadata: Default::default(),
                attachments: Vec::new(),
            })
        }

        async fn handle_struct_with_context(
            &self,
            context: &crate::plugin::sdk::ToolInvokeContext<'_>,
            args: DispatchInnerArgs,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text(format!(
                "struct-ctx:{}:{}:{}:{}",
                context.tool_name, context.session_id, context.call_id, args.value
            )))
        }

        async fn handle_struct_stream_with_context(
            &self,
            context: &crate::plugin::sdk::ToolInvokeContext<'_>,
            sink: crate::plugin::sdk::ToolStreamSink,
            args: DispatchInnerArgs,
        ) -> SdkResult<crate::plugin::sdk::ToolStreamEnd> {
            sink.text(format!(
                "struct-stream-ctx:{}:{}:{}:{}",
                context.tool_name, context.session_id, context.call_id, args.value
            ))
            .await;
            Ok(crate::plugin::sdk::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "Struct Stream Context".to_string(),
                output_text: format!(
                    "struct-stream-ctx:{}:{}:{}:{}",
                    context.tool_name, context.session_id, context.call_id, args.value
                ),
                payload: None,
                metadata: Default::default(),
                attachments: Vec::new(),
            })
        }

        async fn handle_struct_owned(
            &self,
            args: DispatchInnerArgs,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text(format!(
                "struct-owned: {}",
                args.value
            )))
        }

        async fn handle_usage(&self) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text("usage"))
        }

        async fn handle_run(
            &self,
            value: &String,
            limit: &Option<u32>,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text(format!(
                "run: {}:{}",
                value,
                limit.unwrap_or(0)
            )))
        }

        async fn handle_run_with_context(
            &self,
            context: &crate::plugin::sdk::ToolInvokeContext<'_>,
            value: String,
            limit: Option<u32>,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text(format!(
                "run-ctx:{}:{}:{}:{}:{}",
                context.tool_name,
                context.session_id,
                context.call_id,
                value,
                limit.unwrap_or(0)
            )))
        }

        async fn handle_run_owned(
            &self,
            value: String,
            limit: Option<u32>,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text(format!(
                "run-owned: {}:{}",
                value,
                limit.unwrap_or(0)
            )))
        }

        async fn handle_shape(&self, args: &DispatchInnerArgs) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text(format!("shape: {}", args.value)))
        }

        async fn handle_shape_stream(
            &self,
            sink: crate::plugin::sdk::ToolStreamSink,
            args: &DispatchInnerArgs,
        ) -> SdkResult<crate::plugin::sdk::ToolStreamEnd> {
            sink.text(format!("shape-stream:{}", args.value)).await;
            Ok(crate::plugin::sdk::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "Shape Stream".to_string(),
                output_text: format!("shape-stream: {}", args.value),
                payload: None,
                metadata: Default::default(),
                attachments: Vec::new(),
            })
        }

        async fn handle_shape_with_context(
            &self,
            context: &crate::plugin::sdk::ToolInvokeContext<'_>,
            value: String,
            limit: Option<u32>,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(ToolInvokeOutput::text(format!(
                "shape-ctx:{}:{}:{}:{}:{}",
                context.tool_name,
                context.session_id,
                context.call_id,
                value,
                limit.unwrap_or(0)
            )))
        }

        async fn handle_shape_stream_with_context(
            &self,
            context: &crate::plugin::sdk::ToolInvokeContext<'_>,
            sink: crate::plugin::sdk::ToolStreamSink,
            value: String,
            limit: Option<u32>,
        ) -> SdkResult<crate::plugin::sdk::ToolStreamEnd> {
            sink.text(format!(
                "shape-stream-ctx:{}:{}:{}:{}:{}",
                context.tool_name,
                context.session_id,
                context.call_id,
                value,
                limit.unwrap_or(0)
            ))
            .await;
            Ok(crate::plugin::sdk::ToolStreamEnd {
                stream_id: sink.stream_id().to_string(),
                title: "Shape Stream Context".to_string(),
                output_text: format!(
                    "shape-stream-ctx:{}:{}:{}:{}:{}",
                    context.tool_name,
                    context.session_id,
                    context.call_id,
                    value,
                    limit.unwrap_or(0)
                ),
                payload: None,
                metadata: Default::default(),
                attachments: Vec::new(),
            })
        }

        async fn permission_shape_paths(
            &self,
            args: &DispatchInnerArgs,
        ) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
            Ok(vec![crate::plugin::sdk::PathRequest::read(format!(
                "/shape/{}",
                args.value
            ))])
        }

        async fn permission_shape_networks(
            &self,
            args: &DispatchInnerArgs,
        ) -> SdkResult<Vec<crate::plugin::sdk::NetworkRequest>> {
            Ok(vec![crate::plugin::sdk::NetworkRequest::connect(format!(
                "https://shape-{}.example.com",
                args.value
            ))])
        }

        async fn permission_shape_usage_paths(
            &self,
        ) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
            Ok(Vec::new())
        }

        async fn permission_shape_usage_networks(
            &self,
        ) -> SdkResult<Vec<crate::plugin::sdk::NetworkRequest>> {
            Ok(Vec::new())
        }

        async fn permission_shape_variant_paths(
            &self,
            value: String,
            limit: Option<u32>,
        ) -> SdkResult<Vec<crate::plugin::sdk::PathRequest>> {
            Ok(vec![crate::plugin::sdk::PathRequest::read(format!(
                "/shape-variant/{value}/{}",
                limit.unwrap_or(0)
            ))])
        }

        async fn permission_shape_variant_networks(
            &self,
            value: String,
            limit: Option<u32>,
        ) -> SdkResult<Vec<crate::plugin::sdk::NetworkRequest>> {
            Ok(vec![crate::plugin::sdk::NetworkRequest::connect(format!(
                "https://shape-variant-{value}-{}.example.com",
                limit.unwrap_or(0)
            ))])
        }
    }

    /// Dispatch fixture using a direct field handler binding.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolCommand)]
    #[tool_command(
        tool = "fixture.dispatch_struct",
        handler_receiver = DispatchReceiver,
        handle = DispatchReceiver::handle_struct,
        stream_handle = DispatchReceiver::handle_struct_stream,
        handle_field = args
    )]
    #[serde(deny_unknown_fields)]
    struct DispatchStructToolInput {
        args: DispatchInnerArgs,
    }

    /// Dispatch fixture using owned argument handler binding.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolCommand)]
    #[tool_command(
        tool = "fixture.dispatch_struct_owned",
        handler_receiver = DispatchReceiver,
        handle = DispatchReceiver::handle_struct_owned,
        handle_field = args,
        handle_by_value = true
    )]
    #[serde(deny_unknown_fields)]
    struct DispatchOwnedStructToolInput {
        args: DispatchInnerArgs,
    }

    /// Dispatch fixture using invoke-context-aware owned handler binding.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolCommand)]
    #[tool_command(
        tool = "fixture.dispatch_struct_context",
        handler_receiver = DispatchReceiver,
        handle_with_context = DispatchReceiver::handle_struct_with_context,
        stream_handle_with_context = DispatchReceiver::handle_struct_stream_with_context,
        handle_field = args,
        handle_by_value = true
    )]
    #[serde(deny_unknown_fields)]
    struct DispatchContextStructToolInput {
        args: DispatchInnerArgs,
    }

    /// Dispatch fixture using per-variant handler bindings.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolCommand)]
    #[tool_command(tool = "fixture.dispatch_enum", handler_receiver = DispatchReceiver)]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum DispatchEnumToolInput {
        #[tool(exec = "usage", handle = DispatchReceiver::handle_usage)]
        Usage,
        #[tool(exec = "run", handle = DispatchReceiver::handle_run)]
        Run { value: String, limit: Option<u32> },
    }

    /// Dispatch fixture using per-variant owned handler bindings.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolCommand)]
    #[tool_command(tool = "fixture.dispatch_enum_owned", handler_receiver = DispatchReceiver)]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum DispatchOwnedEnumToolInput {
        #[tool(
            exec = "run",
            handle = DispatchReceiver::handle_run_owned,
            handle_by_value = true
        )]
        Run { value: String, limit: Option<u32> },
    }

    /// Dispatch fixture using per-variant invoke-context-aware handler bindings.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolCommand)]
    #[tool_command(
        tool = "fixture.dispatch_enum_context",
        handler_receiver = DispatchReceiver
    )]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum DispatchContextEnumToolInput {
        #[tool(
            exec = "run",
            handle_with_context = DispatchReceiver::handle_run_with_context,
            handle_by_value = true
        )]
        Run { value: String, limit: Option<u32> },
    }

    #[allow(dead_code)]
    #[derive(Debug, SdkToolSubcommands)]
    #[tool_subcommands(handler_receiver = DispatchReceiver)]
    enum DispatchToolSuite {
        Struct(DispatchStructToolInput),
        StructOwned(DispatchOwnedStructToolInput),
        StructContext(DispatchContextStructToolInput),
        Enum(DispatchEnumToolInput),
        EnumOwned(DispatchOwnedEnumToolInput),
        EnumContext(DispatchContextEnumToolInput),
    }

    /// Dispatch fixture using shape-level direct field handler binding.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolArgs)]
    #[tool_args(
        handler_receiver = DispatchReceiver,
        handle = DispatchReceiver::handle_shape,
        stream_handle = DispatchReceiver::handle_shape_stream,
        permission_paths_handle = DispatchReceiver::permission_shape_paths,
        permission_networks_handle = DispatchReceiver::permission_shape_networks,
        handle_field = args
    )]
    #[serde(deny_unknown_fields)]
    struct DispatchStructShapeInput {
        args: DispatchInnerArgs,
    }

    /// Dispatch fixture using shape-level per-variant context-aware handler bindings.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, SdkToolArgs)]
    #[tool_args(handler_receiver = DispatchReceiver)]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum DispatchEnumShapeInput {
        #[tool(
            default_when_empty = true,
            handle = DispatchReceiver::handle_usage,
            permission_paths_handle = DispatchReceiver::permission_shape_usage_paths,
            permission_networks_handle = DispatchReceiver::permission_shape_usage_networks
        )]
        Usage,
        #[tool(
            exec = "run",
            handle_with_context = DispatchReceiver::handle_shape_with_context,
            stream_handle_with_context = DispatchReceiver::handle_shape_stream_with_context,
            permission_paths_handle = DispatchReceiver::permission_shape_variant_paths,
            permission_networks_handle = DispatchReceiver::permission_shape_variant_networks,
            handle_by_value = true
        )]
        Run { value: String, limit: Option<u32> },
    }

    fn validate_inline_variant_limit(value: &serde_json::Value) -> SdkResult<()> {
        let limit = value
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if limit > 10 {
            return Err(crate::plugin::PluginError::invalid_params(
                "limit must be 10 or less",
            ));
        }
        Ok(())
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum MultiFieldValidatedShapeInput {
        #[tool(non_empty("value"), validate = validate_inline_variant_limit)]
        Run { value: String, limit: Option<u32> },
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.multi_field_validate")]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    /// Multi-field variant validation fixture.
    enum MultiFieldValidatedToolInput {
        #[tool(
            exec = "run",
            non_empty("value"),
            validate = validate_inline_variant_limit
        )]
        Run { value: String, limit: Option<u32> },
    }

    /// Doc-backed struct tool.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.doc_struct_tool")]
    #[serde(deny_unknown_fields)]
    struct DocBackedStructToolInput {
        /// Path to inspect.
        path: String,
    }

    /// Doc-backed tool.
    ///
    /// Second paragraph becomes help text automatically.
    #[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(
        tool = "fixture.doc_tool",
        examples(r#"{"action":"run","value":"ok"}"#),
        tags(ToolTag::ReadOnly),
        concurrency_safe = true
    )]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum DocBackedToolInput {
        /// Run the documented action.
        #[tool(exec = "run")]
        Run {
            #[serde(flatten)]
            args: DocBackedArgs,
        },
        /// Explain the documented action.
        #[tool(exec = "explain")]
        Explain {
            #[serde(flatten)]
            args: DocBackedArgs,
        },
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(
        tool = "fixture.ui_display_tool",
        description = "UI display preset fixture.",
        ui_display = brief
    )]
    #[serde(deny_unknown_fields)]
    struct UiDisplaySurfaceInput;

    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(
        tool = "fixture.brief_display_tool",
        description = "Brief display preset fixture.",
        display = brief
    )]
    #[serde(deny_unknown_fields)]
    struct BriefDisplaySurfaceInput;

    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(
        tool = "fixture.about_surface_tool",
        about = "About summary.",
        long_about = "About description.",
        long_help = "About help."
    )]
    #[serde(deny_unknown_fields)]
    struct AboutAliasSurfaceInput;

    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(
        tool = "fixture.after_help_surface_tool",
        description = "After-help surface fixture.",
        after_help = "After-help text.",
        after_long_help = "After-long-help text."
    )]
    #[serde(deny_unknown_fields)]
    struct AfterHelpAliasSurfaceInput;

    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(
        tool = "fixture.before_help_surface_tool",
        description = "Before-help surface fixture.",
        before_help = "Before-help surface fixture.",
        before_long_help = "Before-long-help surface fixture."
    )]
    #[serde(deny_unknown_fields)]
    struct BeforeHelpAliasSurfaceInput;

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolCommand)]
    #[tool_command(
        tool = "fixture.about_command_tool",
        about = "Command summary.",
        long_about = "Command description.",
        long_help = "Command help."
    )]
    #[serde(deny_unknown_fields)]
    struct AboutAliasCommandInput;

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolCommand)]
    #[tool_command(
        tool = "fixture.after_help_command_tool",
        description = "After-help command fixture.",
        after_help = "Command after-help text.",
        after_long_help = "Command after-long-help text."
    )]
    #[serde(deny_unknown_fields)]
    struct AfterHelpAliasCommandInput;

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolCommand)]
    #[tool_command(
        tool = "fixture.before_help_command_tool",
        description = "Command before-help fixture.",
        before_help = "Command before-help text.",
        before_long_help = "Command before-long-help text."
    )]
    #[serde(deny_unknown_fields)]
    struct BeforeHelpAliasCommandInput;

    #[derive(Debug, Deserialize, JsonSchema, ToolInputShape)]
    #[serde(deny_unknown_fields)]
    struct ShapeFromStringInput {
        value: String,
    }

    fn validate_non_empty_shape(input: &ValidatedShapeFromStringInput) -> SdkResult<()> {
        if input.value.trim().is_empty() {
            return Err(crate::plugin::PluginError::invalid_params(
                "value must not be empty",
            ));
        }
        Ok(())
    }

    #[derive(Debug, Deserialize, JsonSchema, ToolInputShape)]
    #[tool_input(validate = validate_non_empty_shape)]
    #[serde(deny_unknown_fields)]
    struct ValidatedShapeFromStringInput {
        value: String,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(non_empty("value"))]
    #[serde(deny_unknown_fields)]
    struct BuiltInValidatedShapeFromStringInput {
        value: String,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct SelectorArgs {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    }

    /// Selector tool.
    #[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.selector_tool")]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum SelectorToolInput {
        #[tool(exec = "pick", exactly_one_of("id", "url"))]
        Pick {
            #[serde(flatten)]
            args: SelectorArgs,
        },
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(trim("query"))]
    #[serde(deny_unknown_fields)]
    struct SearchArgs {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(trim("tool"))]
    #[serde(deny_unknown_fields)]
    struct HelpArgs {
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        include_schema: Option<bool>,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct RoutedArgs {
        value: String,
    }

    /// Route fixture using direct field serialization.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.route_field")]
    #[serde(deny_unknown_fields)]
    struct RoutedFieldToolInput {
        #[serde(flatten)]
        args: RoutedArgs,
    }

    /// Route fixture using converted payload serialization.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.route_convert")]
    #[serde(deny_unknown_fields)]
    struct RoutedConvertToolInput {
        #[serde(flatten)]
        args: RoutedArgs,
    }

    /// Route fixture using direct field serialization plus ToolInputShape normalization.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.route_field_shape")]
    #[serde(deny_unknown_fields)]
    struct RoutedFieldShapeToolInput {
        #[serde(flatten)]
        args: RoutedArgs,
    }

    /// Route fixture using direct field serialization plus routed action injection.
    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.route_action_shape")]
    #[serde(deny_unknown_fields)]
    struct RoutedActionShapeToolInput {
        #[serde(flatten)]
        args: RoutedArgs,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum RoutedTargetInput {
        Echo { value: String },
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(trim("value"), non_empty("value"))]
    #[serde(deny_unknown_fields)]
    struct RoutedNormalizedValueInput {
        value: String,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(trim("value"), non_empty("value"))]
    #[serde(deny_unknown_fields)]
    struct FlattenedNestedShapeInput {
        value: String,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[serde(deny_unknown_fields)]
    struct FlattenedShapeWrapperInput {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        inner: FlattenedNestedShapeInput,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(
        tool = "fixture.flatten_shape_surface",
        description = "Flattened nested ToolInputShape normalization fixture."
    )]
    #[serde(deny_unknown_fields)]
    struct FlattenedShapeSurfaceInput {
        #[tool(flatten_shape)]
        #[serde(flatten)]
        inner: FlattenedNestedShapeInput,
    }

    #[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.surface_route", description = "Surface route fixture.")]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum SurfaceRoutedToolInput {
        #[tool(exec = "field", route = "fixture.inner_surface_field")]
        Field {
            #[serde(flatten)]
            args: RoutedArgs,
        },
        #[tool(
            exec = "field_shape",
            route = "fixture.inner_surface_field_shape",
            shape = RoutedNormalizedValueInput
        )]
        FieldShape {
            #[serde(flatten)]
            args: RoutedArgs,
        },
        #[tool(
            exec = "action_shape",
            route = "fixture.inner_surface_action_shape",
            route_action = "echo",
            shape = RoutedTargetInput
        )]
        ActionShape {
            #[serde(flatten)]
            args: RoutedArgs,
        },
    }

    fn convert_routed_target(input: RoutedConvertToolInput) -> SdkResult<RoutedTargetInput> {
        Ok(RoutedTargetInput::Echo {
            value: input.args.value,
        })
    }

    #[allow(dead_code)]
    #[derive(Debug, ToolSuite)]
    enum RoutedToolSuite {
        #[tool(route = "fixture.inner_field", field = args)]
        Field(RoutedFieldToolInput),
        #[tool(route = "fixture.inner_field_shape", field = args, shape = RoutedNormalizedValueInput)]
        FieldShape(RoutedFieldShapeToolInput),
        #[tool(
            route = "fixture.inner_action_shape",
            route_action = "echo",
            field = args,
            shape = RoutedTargetInput
        )]
        ActionShape(RoutedActionShapeToolInput),
        #[tool(route = "fixture.inner_convert", convert = convert_routed_target)]
        Convert(RoutedConvertToolInput),
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(requires("confirm", "token"), conflicts_with("id", "url"))]
    #[serde(deny_unknown_fields)]
    struct RelationValidatedShapeInput {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confirm: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(required_unless_present("questions[].allow_custom", "questions[].options"))]
    #[serde(deny_unknown_fields)]
    struct QuestionChoiceShapeInput {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        questions: Vec<crate::message::UserInputQuestion>,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(forbid_substrings("name", "/", "\\"))]
    #[serde(deny_unknown_fields)]
    struct PathValidatedShapeInput {
        name: String,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(trim("name"), trim_suffix("name", ".md"))]
    #[serde(deny_unknown_fields)]
    struct NormalizedNameShapeInput {
        name: String,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(
        non_empty_if_present("questions[].options[].label"),
        distinct_trimmed("questions[].id"),
        distinct_trimmed_within("questions[].options[].label", "questions[]")
    )]
    #[serde(deny_unknown_fields)]
    struct DistinctQuestionShapeInput {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        questions: Vec<crate::message::UserInputQuestion>,
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(trim("query", "tool"))]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum VariantValidatedShapeInput {
        #[tool(non_empty("query"))]
        Search { query: String },
        #[tool(non_empty("tool"))]
        Help { tool: String },
    }

    #[derive(Debug, Deserialize, Serialize, JsonSchema, ToolInputShape)]
    #[tool_input(trim("query", "tool"))]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum NormalizedVariantShapeInput {
        #[tool(default_when_empty = true)]
        Usage,
        #[tool(infer_when_present("query"), drop_keys("tool"))]
        Search { query: String },
        #[tool(
            infer_when_present("tool"),
            action_alias("describe"),
            action_alias_default("quick_help", include_schema = false)
        )]
        Help {
            tool: String,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            include_schema: Option<bool>,
        },
    }

    #[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
    #[tool_surface(tool = "fixture.catalog_tool", description = "Catalog tool fixture.")]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum CatalogToolInput {
        #[tool(exec = "usage", default_when_empty = true)]
        Usage,
        #[tool(
            exec = "search",
            infer_when_present("query"),
            drop_keys("tool", "include_schema")
        )]
        Search {
            #[tool(flatten_shape)]
            #[serde(flatten)]
            args: SearchArgs,
        },
        #[tool(exec = "help", infer_when_present("tool"), drop_keys("query", "limit"))]
        Help {
            #[tool(flatten_shape)]
            #[serde(flatten)]
            args: HelpArgs,
        },
    }

    #[test]
    fn model_safe_tool_name_uses_readable_underscore_separators() {
        assert_eq!(super::model_safe_tool_name("agena_fs__fs"), "agena_fs__fs");
        assert_eq!(
            super::model_safe_tool_name("mcp:docs:search"),
            "mcp_docs_search"
        );
    }

    #[test]
    fn static_tool_surface_can_derive_docs_from_rust_doc_comments() {
        let decl = DocBackedToolInput::tool_decl();
        assert_eq!(
            decl.description_text(),
            "Doc-backed tool.\n\nSecond paragraph becomes help text automatically."
        );
        assert_eq!(decl.summary_text(), Some("Doc-backed tool."));
        assert_eq!(
            decl.help_text(),
            Some("Doc-backed tool.\n\nSecond paragraph becomes help text automatically.")
        );
        assert_eq!(
            decl.example_texts(),
            &[r#"{"action":"run","value":"ok"}"#.to_string()]
        );
    }

    #[test]
    fn static_tool_surface_accepts_about_aliases() {
        let decl = AboutAliasSurfaceInput::tool_decl();
        assert_eq!(decl.summary_text(), Some("About summary."));
        assert_eq!(decl.description_text(), "About description.");
        assert_eq!(decl.help_text(), Some("About help."));
    }

    #[test]
    fn tool_command_accepts_about_aliases() {
        let decl = AboutAliasCommandInput::tool_decl();
        assert_eq!(decl.summary_text(), Some("Command summary."));
        assert_eq!(decl.description_text(), "Command description.");
        assert_eq!(decl.help_text(), Some("Command help."));
    }

    #[test]
    fn static_tool_surface_accepts_after_help_aliases() {
        let decl = AfterHelpAliasSurfaceInput::tool_decl();
        assert_eq!(decl.after_help_text(), Some("After-long-help text."));
        assert_eq!(decl.help_text(), None);
    }

    #[test]
    fn tool_command_accepts_after_help_aliases() {
        let decl = AfterHelpAliasCommandInput::tool_decl();
        assert_eq!(
            decl.after_help_text(),
            Some("Command after-long-help text.")
        );
        assert_eq!(decl.help_text(), None);
    }

    #[test]
    fn static_tool_surface_accepts_before_help_aliases() {
        let decl = BeforeHelpAliasSurfaceInput::tool_decl();
        assert_eq!(
            decl.before_help_text(),
            Some("Before-long-help surface fixture.")
        );
        assert_eq!(decl.description_text(), "Before-help surface fixture.");
    }

    #[test]
    fn tool_command_accepts_before_help_aliases() {
        let decl = BeforeHelpAliasCommandInput::tool_decl();
        assert_eq!(
            decl.before_help_text(),
            Some("Command before-long-help text.")
        );
        assert_eq!(decl.description_text(), "Command before-help fixture.");
    }

    #[test]
    fn tool_input_shape_schema_carries_field_doc_descriptions() {
        let schema = DocBackedShapeInput::input_schema();
        assert_eq!(
            schema
                .pointer("/properties/query/description")
                .and_then(serde_json::Value::as_str),
            Some("Search text.")
        );
        assert_eq!(
            schema
                .pointer("/properties/limit/description")
                .and_then(serde_json::Value::as_str),
            Some("Optional result limit.")
        );
        let usage = crate::tool::definition::schema_usage_text(&schema).expect("usage text");
        assert!(usage.contains("Search text."));
        assert!(usage.contains("Optional result limit."));
    }

    #[test]
    fn tool_input_shape_schema_carries_field_order_metadata() {
        let schema = OrderedShapeInput::input_schema();
        assert_eq!(
            schema
                .pointer("/properties/beta/x-agena-order")
                .and_then(serde_json::Value::as_str),
            Some("000000")
        );
        assert_eq!(
            schema
                .pointer("/properties/alpha/x-agena-order")
                .and_then(serde_json::Value::as_str),
            Some("000001")
        );

        let usage = crate::tool::definition::schema_usage_text(&schema).expect("usage text");
        let beta_index = usage.find("`beta` <string, required>").expect("beta arg");
        let alpha_index = usage.find("`alpha` <string, required>").expect("alpha arg");
        assert!(beta_index < alpha_index);

        let examples = crate::tool::definition::schema_example_texts(&schema);
        assert_eq!(
            examples.as_slice(),
            &[r#"{"beta":"<beta>","alpha":"<alpha>"}"#.to_string()]
        );
    }

    #[test]
    fn sdk_reexported_clap_style_derives_parse_and_describe_tools() {
        let parsed = SdkAliasArgs::parse_json_str(r#"{"value":"  ok  "}"#)
            .expect("sdk alias args should parse through ToolArgs re-export");
        assert_eq!(parsed.value, "ok");

        let command_decl = SdkAliasCommandInput::tool_decl();
        assert_eq!(command_decl.name, "fixture.sdk_alias_command");
        assert_eq!(
            command_decl.alias_texts(),
            &[
                "fixture.sdk_alias_short".to_string(),
                "fixture.sdk_alias_visible".to_string(),
                "fixture.sdk_alias_lookup".to_string()
            ]
        );
        let usage = crate::tool::definition::schema_usage_text(&command_decl.input_schema)
            .expect("sdk alias command usage");
        assert!(usage.contains("Value routed through the SDK alias surface."));

        let decl_names = SdkAliasToolSuite::tool_decls()
            .into_iter()
            .map(|decl| decl.name)
            .collect::<Vec<_>>();
        assert_eq!(decl_names, vec!["fixture.sdk_alias_command".to_string()]);

        let (resolved_tool, resolved_input) =
            SdkAliasCommandInput::resolve_json_str("fixture.sdk_alias_short", r#"{"value":"ok"}"#)
                .expect("single tool resolve should accept aliases");
        assert_eq!(resolved_tool, "fixture.sdk_alias_command");
        assert_eq!(resolved_input, serde_json::json!({"value":"ok"}));

        let (suite_tool, suite_input) = SdkAliasToolSuite::resolve_tool_json_str(
            "fixture.sdk_alias_lookup",
            r#"{"value":"ok"}"#,
        )
        .expect("suite resolve should accept aliases");
        assert_eq!(suite_tool, "fixture.sdk_alias_command");
        assert_eq!(suite_input, serde_json::json!({"value":"ok"}));
    }

    #[test]
    fn clap_style_dispatch_helpers_bind_struct_enum_and_suite_handlers() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime
            .block_on(async {
                let receiver = DispatchReceiver;

                let struct_output = DispatchStructToolInput::parse_input(json!({
                    "args": { "value": "alpha" }
                }))
                .expect("struct dispatch input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("struct handler dispatch");
                assert_eq!(struct_output.output_text, "struct: alpha");

                let enum_usage_output = DispatchEnumToolInput::parse_input(json!({
                    "action": "usage"
                }))
                .expect("enum usage input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("enum usage dispatch");
                assert_eq!(enum_usage_output.output_text, "usage");

                let enum_run_output = DispatchEnumToolInput::parse_input(json!({
                    "action": "run",
                    "value": "beta",
                    "limit": 3
                }))
                .expect("enum run input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("enum run dispatch");
                assert_eq!(enum_run_output.output_text, "run: beta:3");

                let struct_owned_output = DispatchOwnedStructToolInput::parse_input(json!({
                    "args": { "value": "delta" }
                }))
                .expect("owned struct dispatch input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("owned struct handler dispatch");
                assert_eq!(struct_owned_output.output_text, "struct-owned: delta");

                let enum_owned_output = DispatchOwnedEnumToolInput::parse_input(json!({
                    "action": "run",
                    "value": "epsilon",
                    "limit": 5
                }))
                .expect("owned enum run input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("owned enum run dispatch");
                assert_eq!(enum_owned_output.output_text, "run-owned: epsilon:5");

                let context = crate::plugin::sdk::ToolInvokeContext {
                    tool_name: "fixture.dispatch_struct_context",
                    session_id: 41,
                    call_id: 42,
                    workspace_root: "/tmp/project",
                };

                let struct_context_output = DispatchContextStructToolInput::parse_input(json!({
                    "args": { "value": "theta" }
                }))
                .expect("context struct dispatch input")
                .dispatch_tool_invoke_with_context(&receiver, &context)
                .await
                .expect("context struct handler dispatch");
                assert_eq!(
                    struct_context_output.output_text,
                    "struct-ctx:fixture.dispatch_struct_context:41:42:theta"
                );

                let enum_context_output = DispatchContextEnumToolInput::parse_input(json!({
                    "action": "run",
                    "value": "iota",
                    "limit": 7
                }))
                .expect("context enum run input")
                .dispatch_tool_invoke_with_context(&receiver, &context)
                .await
                .expect("context enum run dispatch");
                assert_eq!(
                    enum_context_output.output_text,
                    "run-ctx:fixture.dispatch_struct_context:41:42:iota:7"
                );

                let shape_struct_output = DispatchStructShapeInput::parse_input(json!({
                    "args": { "value": "lambda" }
                }))
                .expect("shape struct dispatch input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("shape struct handler dispatch");
                assert_eq!(shape_struct_output.output_text, "shape: lambda");

                let shape_struct_paths = DispatchStructShapeInput::parse_input(json!({
                    "args": { "value": "lambda" }
                }))
                .expect("shape struct permission paths input")
                .dispatch_permission_paths(&receiver)
                .await
                .expect("shape struct permission paths dispatch");
                assert_eq!(
                    shape_struct_paths,
                    vec![crate::plugin::sdk::PathRequest::read("/shape/lambda")]
                );

                let shape_struct_networks = DispatchStructShapeInput::parse_input(json!({
                    "args": { "value": "lambda" }
                }))
                .expect("shape struct permission networks input")
                .dispatch_permission_networks(&receiver)
                .await
                .expect("shape struct permission networks dispatch");
                assert_eq!(
                    shape_struct_networks,
                    vec![crate::plugin::sdk::NetworkRequest::connect(
                        "https://shape-lambda.example.com"
                    )]
                );

                let shape_enum_usage_output = DispatchEnumShapeInput::parse_input(json!({}))
                    .expect("shape enum usage input")
                    .dispatch_tool_invoke(&receiver)
                    .await
                    .expect("shape enum usage dispatch");
                assert_eq!(shape_enum_usage_output.output_text, "usage");

                let shape_enum_context_output = DispatchEnumShapeInput::parse_input(json!({
                    "action": "run",
                    "value": "mu",
                    "limit": 11
                }))
                .expect("shape enum context input")
                .dispatch_tool_invoke_with_context(&receiver, &context)
                .await
                .expect("shape enum context dispatch");
                assert_eq!(
                    shape_enum_context_output.output_text,
                    "shape-ctx:fixture.dispatch_struct_context:41:42:mu:11"
                );

                let shape_enum_usage_paths = DispatchEnumShapeInput::parse_input(json!({}))
                    .expect("shape enum usage permission paths input")
                    .dispatch_permission_paths(&receiver)
                    .await
                    .expect("shape enum usage permission paths dispatch");
                assert!(shape_enum_usage_paths.is_empty());

                let shape_enum_usage_networks = DispatchEnumShapeInput::parse_input(json!({}))
                    .expect("shape enum usage permission networks input")
                    .dispatch_permission_networks(&receiver)
                    .await
                    .expect("shape enum usage permission networks dispatch");
                assert!(shape_enum_usage_networks.is_empty());

                let shape_enum_run_paths = DispatchEnumShapeInput::parse_input(json!({
                    "action": "run",
                    "value": "mu",
                    "limit": 11
                }))
                .expect("shape enum run permission paths input")
                .dispatch_permission_paths(&receiver)
                .await
                .expect("shape enum run permission paths dispatch");
                assert_eq!(
                    shape_enum_run_paths,
                    vec![crate::plugin::sdk::PathRequest::read(
                        "/shape-variant/mu/11"
                    )]
                );

                let shape_enum_run_networks = DispatchEnumShapeInput::parse_input(json!({
                    "action": "run",
                    "value": "mu",
                    "limit": 11
                }))
                .expect("shape enum run permission networks input")
                .dispatch_permission_networks(&receiver)
                .await
                .expect("shape enum run permission networks dispatch");
                assert_eq!(
                    shape_enum_run_networks,
                    vec![crate::plugin::sdk::NetworkRequest::connect(
                        "https://shape-variant-mu-11.example.com"
                    )]
                );

                let suite_output = DispatchToolSuite::parse_tool(
                    "fixture.dispatch_struct",
                    json!({ "args": { "value": "gamma" } }),
                )
                .expect("suite input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("suite dispatch");
                assert_eq!(suite_output.output_text, "struct: gamma");

                let suite_owned_output = DispatchToolSuite::parse_tool(
                    "fixture.dispatch_enum_owned",
                    json!({ "action": "run", "value": "zeta", "limit": 13 }),
                )
                .expect("owned suite input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("owned suite dispatch");
                assert_eq!(suite_owned_output.output_text, "run-owned: zeta:13");

                let suite_context_struct_output = DispatchToolSuite::parse_tool(
                    "fixture.dispatch_struct_context",
                    json!({ "args": { "value": "eta" } }),
                )
                .expect("context suite struct input")
                .dispatch_tool_invoke_with_context(&receiver, &context)
                .await
                .expect("context suite struct dispatch");
                assert_eq!(
                    suite_context_struct_output.output_text,
                    "struct-ctx:fixture.dispatch_struct_context:41:42:eta"
                );

                let suite_context_enum_output = DispatchToolSuite::parse_tool(
                    "fixture.dispatch_enum_context",
                    json!({ "action": "run", "value": "kappa", "limit": 17 }),
                )
                .expect("context suite enum input")
                .dispatch_tool_invoke_with_context(&receiver, &context)
                .await
                .expect("context suite enum dispatch");
                assert_eq!(
                    suite_context_enum_output.output_text,
                    "run-ctx:fixture.dispatch_struct_context:41:42:kappa:17"
                );

                let shape_dispatch_output = DispatchStructShapeInput::parse_input(json!({
                    "args": { "value": "nu" }
                }))
                .expect("shape dispatch input")
                .dispatch_tool_invoke(&receiver)
                .await
                .expect("shape dispatch");
                assert_eq!(shape_dispatch_output.output_text, "shape: nu");

                let shape_dispatch_context = crate::plugin::sdk::ToolInvokeContext {
                    tool_name: "dynamic.skill",
                    session_id: 61,
                    call_id: 62,
                    workspace_root: "/tmp/project",
                };
                let shape_dispatch_context_output = DispatchEnumShapeInput::parse_input(json!({
                    "action": "run",
                    "value": "xi",
                    "limit": 19
                }))
                .expect("shape dispatch context input")
                .dispatch_tool_invoke_with_context(&receiver, &shape_dispatch_context)
                .await
                .expect("shape dispatch context");
                assert_eq!(
                    shape_dispatch_context_output.output_text,
                    "shape-ctx:dynamic.skill:61:62:xi:19"
                );

                let shape_dispatch_paths = DispatchStructShapeInput::parse_input(json!({
                    "args": { "value": "omicron" }
                }))
                .expect("shape permission paths input")
                .dispatch_permission_paths(&receiver)
                .await
                .expect("shape permission paths");
                assert_eq!(
                    shape_dispatch_paths,
                    vec![crate::plugin::sdk::PathRequest::read("/shape/omicron")]
                );

                let shape_dispatch_networks = DispatchEnumShapeInput::parse_input(json!({
                    "action": "run",
                    "value": "pi",
                    "limit": 29
                }))
                .expect("shape permission networks input")
                .dispatch_permission_networks(&receiver)
                .await
                .expect("shape permission networks");
                assert_eq!(
                    shape_dispatch_networks,
                    vec![crate::plugin::sdk::NetworkRequest::connect(
                        "https://shape-variant-pi-29.example.com"
                    )]
                );
                Ok::<_, crate::plugin::PluginError>(())
            })
            .expect("dispatch helpers should succeed");
    }

    #[test]
    fn clap_style_stream_dispatch_helpers_bind_struct_enum_suite_and_shape_handlers() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime
            .block_on(async {
                let receiver = DispatchReceiver;
                let context = crate::plugin::sdk::ToolInvokeContext {
                    tool_name: "fixture.dispatch_struct_context",
                    session_id: 71,
                    call_id: 72,
                    workspace_root: "/tmp/project",
                };

                let (struct_tx, mut struct_rx) = tokio::sync::mpsc::channel(8);
                let struct_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-struct-stream".to_string(),
                    struct_tx,
                );
                let struct_stream_end = DispatchStructToolInput::parse_input(json!({
                    "args": { "value": "alpha" }
                }))
                .expect("struct stream input")
                .dispatch_tool_invoke_stream(&receiver, struct_sink)
                .await
                .expect("struct stream dispatch");
                assert_eq!(struct_stream_end.output_text, "struct-stream: alpha");
                assert_eq!(
                    struct_rx.recv().await.and_then(|chunk| chunk.text_delta),
                    Some("struct-stream:alpha".to_string())
                );

                let (enum_tx, mut enum_rx) = tokio::sync::mpsc::channel(8);
                let enum_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-enum-stream".to_string(),
                    enum_tx,
                );
                let enum_stream_end = DispatchEnumToolInput::parse_input(json!({
                    "action": "run",
                    "value": "beta",
                    "limit": 23
                }))
                .expect("enum stream input")
                .dispatch_tool_invoke_stream(&receiver, enum_sink)
                .await
                .expect("enum stream dispatch");
                assert_eq!(enum_stream_end.output_text, "run: beta:23");
                assert_eq!(
                    enum_rx.recv().await.and_then(|chunk| chunk.text_delta),
                    Some("run: beta:23".to_string())
                );

                let (struct_ctx_tx, mut struct_ctx_rx) = tokio::sync::mpsc::channel(8);
                let struct_ctx_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-struct-stream-context".to_string(),
                    struct_ctx_tx,
                );
                let struct_context_stream_end =
                    DispatchContextStructToolInput::parse_input(json!({
                        "args": { "value": "gamma" }
                    }))
                    .expect("context struct stream input")
                    .dispatch_tool_invoke_stream_with_context(&receiver, &context, struct_ctx_sink)
                    .await
                    .expect("context struct stream dispatch");
                assert_eq!(
                    struct_context_stream_end.output_text,
                    "struct-stream-ctx:fixture.dispatch_struct_context:71:72:gamma"
                );
                assert_eq!(
                    struct_ctx_rx
                        .recv()
                        .await
                        .and_then(|chunk| chunk.text_delta),
                    Some(
                        "struct-stream-ctx:fixture.dispatch_struct_context:71:72:gamma".to_string()
                    )
                );

                let (shape_tx, mut shape_rx) = tokio::sync::mpsc::channel(8);
                let shape_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-shape-stream".to_string(),
                    shape_tx,
                );
                let shape_stream_end = DispatchStructShapeInput::parse_input(json!({
                    "args": { "value": "delta" }
                }))
                .expect("shape stream input")
                .dispatch_tool_invoke_stream(&receiver, shape_sink)
                .await
                .expect("shape stream dispatch");
                assert_eq!(shape_stream_end.output_text, "shape-stream: delta");
                assert_eq!(
                    shape_rx.recv().await.and_then(|chunk| chunk.text_delta),
                    Some("shape-stream:delta".to_string())
                );

                let (shape_ctx_tx, mut shape_ctx_rx) = tokio::sync::mpsc::channel(8);
                let shape_ctx_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-shape-stream-context".to_string(),
                    shape_ctx_tx,
                );
                let shape_context_stream_end = DispatchEnumShapeInput::parse_input(json!({
                    "action": "run",
                    "value": "epsilon",
                    "limit": 29
                }))
                .expect("shape context stream input")
                .dispatch_tool_invoke_stream_with_context(&receiver, &context, shape_ctx_sink)
                .await
                .expect("shape context stream dispatch");
                assert_eq!(
                    shape_context_stream_end.output_text,
                    "shape-stream-ctx:fixture.dispatch_struct_context:71:72:epsilon:29"
                );
                assert_eq!(
                    shape_ctx_rx.recv().await.and_then(|chunk| chunk.text_delta),
                    Some(
                        "shape-stream-ctx:fixture.dispatch_struct_context:71:72:epsilon:29"
                            .to_string()
                    )
                );

                let (suite_tx, mut suite_rx) = tokio::sync::mpsc::channel(8);
                let suite_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-suite-stream".to_string(),
                    suite_tx,
                );
                let suite_stream_end = DispatchToolSuite::parse_tool(
                    "fixture.dispatch_enum",
                    json!({ "action": "run", "value": "zeta", "limit": 31 }),
                )
                .expect("suite stream input")
                .dispatch_tool_invoke_stream(&receiver, suite_sink)
                .await
                .expect("suite stream dispatch");
                assert_eq!(suite_stream_end.output_text, "run: zeta:31");
                assert_eq!(
                    suite_rx.recv().await.and_then(|chunk| chunk.text_delta),
                    Some("run: zeta:31".to_string())
                );

                let (suite_ctx_tx, mut suite_ctx_rx) = tokio::sync::mpsc::channel(8);
                let suite_ctx_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-suite-stream-context".to_string(),
                    suite_ctx_tx,
                );
                let suite_context_stream_end = DispatchToolSuite::parse_tool(
                    "fixture.dispatch_struct_context",
                    json!({ "args": { "value": "eta" } }),
                )
                .expect("suite context stream input")
                .dispatch_tool_invoke_stream_with_context(&receiver, &context, suite_ctx_sink)
                .await
                .expect("suite context stream dispatch");
                assert_eq!(
                    suite_context_stream_end.output_text,
                    "struct-stream-ctx:fixture.dispatch_struct_context:71:72:eta"
                );
                assert_eq!(
                    suite_ctx_rx.recv().await.and_then(|chunk| chunk.text_delta),
                    Some("struct-stream-ctx:fixture.dispatch_struct_context:71:72:eta".to_string())
                );

                let (macro_surface_tx, mut macro_surface_rx) = tokio::sync::mpsc::channel(8);
                let macro_surface_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-macro-surface-stream".to_string(),
                    macro_surface_tx,
                );
                let macro_surface_stream_end = DispatchStructToolInput::parse_input(json!({
                    "args": { "value": "theta" }
                }))
                .expect("surface stream input")
                .dispatch_tool_invoke_stream(&receiver, macro_surface_sink)
                .await
                .expect("surface stream dispatch");
                assert_eq!(macro_surface_stream_end.output_text, "struct-stream: theta");
                assert_eq!(
                    macro_surface_rx
                        .recv()
                        .await
                        .and_then(|chunk| chunk.text_delta),
                    Some("struct-stream:theta".to_string())
                );

                let (macro_suite_tx, mut macro_suite_rx) = tokio::sync::mpsc::channel(8);
                let macro_suite_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-macro-suite-stream".to_string(),
                    macro_suite_tx,
                );
                let macro_suite_stream_end = DispatchToolSuite::parse_tool(
                    "fixture.dispatch_enum",
                    json!({ "action": "run", "value": "iota", "limit": 37 }),
                )
                .expect("suite stream input")
                .dispatch_tool_invoke_stream(&receiver, macro_suite_sink)
                .await
                .expect("suite stream dispatch");
                assert_eq!(macro_suite_stream_end.output_text, "run: iota:37");
                assert_eq!(
                    macro_suite_rx
                        .recv()
                        .await
                        .and_then(|chunk| chunk.text_delta),
                    Some("run: iota:37".to_string())
                );

                let (macro_shape_tx, mut macro_shape_rx) = tokio::sync::mpsc::channel(8);
                let macro_shape_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-macro-shape-stream".to_string(),
                    macro_shape_tx,
                );
                let macro_shape_stream_end = DispatchStructShapeInput::parse_input(json!({
                    "args": { "value": "kappa" }
                }))
                .expect("shape stream input")
                .dispatch_tool_invoke_stream(&receiver, macro_shape_sink)
                .await
                .expect("shape stream dispatch");
                assert_eq!(macro_shape_stream_end.output_text, "shape-stream: kappa");
                assert_eq!(
                    macro_shape_rx
                        .recv()
                        .await
                        .and_then(|chunk| chunk.text_delta),
                    Some("shape-stream:kappa".to_string())
                );

                let (macro_shape_ctx_tx, mut macro_shape_ctx_rx) = tokio::sync::mpsc::channel(8);
                let macro_shape_ctx_sink = crate::plugin::sdk::ToolStreamSink::new(
                    "fixture-macro-shape-stream-context".to_string(),
                    macro_shape_ctx_tx,
                );
                let macro_shape_stream_context = crate::plugin::sdk::ToolInvokeContext {
                    tool_name: "dynamic.skill",
                    session_id: 87,
                    call_id: 88,
                    workspace_root: "/tmp/project",
                };
                let macro_shape_context_stream_end = DispatchEnumShapeInput::parse_input(json!({
                    "action": "run",
                    "value": "lambda",
                    "limit": 41
                }))
                .expect("shape stream context input")
                .dispatch_tool_invoke_stream_with_context(
                    &receiver,
                    &macro_shape_stream_context,
                    macro_shape_ctx_sink,
                )
                .await
                .expect("shape stream context dispatch");
                assert_eq!(
                    macro_shape_context_stream_end.output_text,
                    "shape-stream-ctx:dynamic.skill:87:88:lambda:41"
                );
                assert_eq!(
                    macro_shape_ctx_rx
                        .recv()
                        .await
                        .and_then(|chunk| chunk.text_delta),
                    Some("shape-stream-ctx:dynamic.skill:87:88:lambda:41".to_string())
                );

                Ok::<_, crate::plugin::PluginError>(())
            })
            .expect("stream dispatch helpers should succeed");
    }

    #[test]
    fn static_tool_surface_schema_carries_variant_and_field_doc_descriptions() {
        let tool_decl = DocBackedToolInput::tool_decl();
        let usage = crate::tool::definition::schema_usage_text(&tool_decl.input_schema)
            .expect("usage text");
        assert!(usage.contains("run: Run the documented action."));

        let struct_decl = DocBackedStructToolInput::tool_decl();
        assert_eq!(
            struct_decl
                .input_schema
                .pointer("/properties/path/description")
                .and_then(serde_json::Value::as_str),
            Some("Path to inspect.")
        );
        let struct_usage = crate::tool::definition::schema_usage_text(&struct_decl.input_schema)
            .expect("struct usage text");
        assert!(struct_usage.contains("Path to inspect."));

        let ui_decl = UiDisplaySurfaceInput::tool_decl();
        assert_eq!(
            ui_decl.preferred_ui_display_mode(),
            Some(crate::plugin::sdk::UiTextDisplayMode::Summary)
        );

        let brief_decl = BriefDisplaySurfaceInput::tool_decl();
        assert_eq!(
            brief_decl.preferred_description_mode(),
            Some(crate::plugin::sdk::ToolDescriptionMode::Brief)
        );
        assert_eq!(
            brief_decl.preferred_ui_display_mode(),
            Some(crate::plugin::sdk::UiTextDisplayMode::Summary)
        );
    }

    #[test]
    fn macro_generated_json_string_parsers_return_structured_results_and_errors() {
        let parsed = ShapeFromStringInput::parse_json_str(r#"{"value":"ok"}"#)
            .expect("shape should parse from raw JSON string");
        assert_eq!(parsed.value, "ok");

        let (_, resolved_args) = DocBackedToolInput::resolve_json_str(
            "fixture.doc_tool",
            r#"{"action":"run","value":"ok"}"#,
        )
        .expect("tool should resolve from raw JSON string");
        assert_eq!(resolved_args, json!({ "value": "ok" }));

        let err = ShapeFromStringInput::parse_json_str(r#"{"value":1}"#)
            .expect_err("invalid type should produce structured parse error");
        let data = err
            .data
            .expect("structured parse error should include data");
        assert_eq!(
            data.get("category").and_then(serde_json::Value::as_str),
            Some("data")
        );
        assert_eq!(
            data.get("path").and_then(serde_json::Value::as_str),
            Some("value")
        );

        let syntax_err = ShapeFromStringInput::parse_json_str(r#"{"value":"ok""#)
            .expect_err("invalid JSON should produce syntax error");
        let syntax_data = syntax_err
            .data
            .expect("syntax parse error should include data");
        assert_eq!(
            syntax_data
                .get("category")
                .and_then(serde_json::Value::as_str),
            Some("eof")
        );
        assert_eq!(
            syntax_data
                .get("source")
                .and_then(serde_json::Value::as_str),
            Some("string")
        );
    }

    #[test]
    fn tool_input_shape_validate_hook_runs_after_parse() {
        let parsed = ValidatedShapeFromStringInput::parse_json_str(r#"{"value":"ok"}"#)
            .expect("validated shape should parse");
        assert_eq!(parsed.value, "ok");

        let err = ValidatedShapeFromStringInput::parse_json_str("{\"value\":\"   \"}")
            .expect_err("validated shape should reject empty trimmed text");
        assert!(err.to_string().contains("value must not be empty"));
    }

    #[test]
    fn built_in_validation_attributes_run_after_parse() {
        let err = BuiltInValidatedShapeFromStringInput::parse_json_str("{\"value\":\"   \"}")
            .expect_err("built-in non_empty should reject blank trimmed text");
        assert!(err.to_string().contains("field `value` must not be empty"));

        let err = SelectorToolInput::parse_input(json!({
            "action": "pick",
            "id": "doc_1",
            "url": "https://example.com"
        }))
        .expect_err("variant-level exactly_one_of should reject duplicate selectors");
        assert!(
            err.to_string()
                .contains("exactly one of `id` or `url` is required")
        );
    }

    #[test]
    fn built_in_action_inference_and_drop_keys_normalize_enum_inputs() {
        let usage = CatalogToolInput::parse_input(json!({}))
            .expect("empty object should map to default usage action");
        assert!(matches!(usage, CatalogToolInput::Usage));

        let search = CatalogToolInput::parse_input(json!({
            "query": "permissions",
            "limit": 5,
            "tool": "noise"
        }))
        .expect("query-only payload should infer search and ignore help-only noise");
        match search {
            CatalogToolInput::Search { args } => {
                assert_eq!(args.query, "permissions");
                assert_eq!(args.limit, Some(5));
            }
            other => panic!("expected search variant, got {other:?}"),
        }

        let help = CatalogToolInput::parse_input(json!({
            "tool": "agena_web__search",
            "include_schema": true
        }))
        .expect("tool payload should infer help");
        match help {
            CatalogToolInput::Help { args } => {
                assert_eq!(args.tool, "agena_web__search");
                assert_eq!(args.include_schema, Some(true));
            }
            other => panic!("expected help variant, got {other:?}"),
        }

        let help_with_noise = CatalogToolInput::parse_input(json!({
            "action": "help",
            "tool": "agena_web__search",
            "include_schema": true,
            "query": "noise"
        }))
        .expect("explicit help action should ignore search-only noise");
        match help_with_noise {
            CatalogToolInput::Help { args } => {
                assert_eq!(args.tool, "agena_web__search");
                assert_eq!(args.include_schema, Some(true));
            }
            other => panic!("expected help variant, got {other:?}"),
        }
    }

    #[test]
    fn enum_inputs_suggest_closest_action_names_for_typos() {
        let err = NormalizedVariantShapeInput::parse_input(json!({
            "action": "describ",
            "tool": "docs"
        }))
        .expect_err("unknown action should suggest a nearby alias");
        let message = err.to_string();
        assert!(message.contains("unknown action 'describ'"));
        assert!(message.contains("Did you mean `describe`?"));

        let err = NormalizedVariantShapeInput::parse_input(json!({
            "action": "quik_help",
            "tool": "docs"
        }))
        .expect_err("unknown action should suggest alias defaults");
        let message = err.to_string();
        assert!(message.contains("unknown action 'quik_help'"));
        assert!(message.contains("Did you mean `quick_help`?"));
    }

    #[test]
    fn static_tool_surface_suggests_closest_action_names_for_typos() {
        let err = CatalogToolInput::parse_input(json!({
            "action": "searc",
            "query": "permissions"
        }))
        .expect_err("unknown action should suggest a close match");
        let message = err.to_string();
        assert!(message.contains("unknown action 'searc'"));
        assert!(message.contains("Did you mean `search`?"));
    }

    #[test]
    fn enum_inputs_suggest_closest_field_names_for_typos() {
        let err = CatalogToolInput::parse_input(json!({
            "action": "search",
            "querry": "permissions"
        }))
        .expect_err("unknown field should suggest the closest matching name");
        let message = err.to_string();
        assert!(message.contains("unknown field 'querry'"));
        assert!(message.contains("Did you mean `query`?"));
    }

    #[test]
    fn static_tool_surface_suggests_closest_field_names_for_typos() {
        let err = DocBackedStructToolInput::parse_input(json!({
            "pat": "notes.txt"
        }))
        .expect_err("unknown field should suggest the closest matching name");
        let message = err.to_string();
        assert!(message.contains("unknown field 'pat'"));
        assert!(message.contains("Did you mean `path`?"));
    }

    #[test]
    fn flattened_shape_unknown_fields_suggest_inner_names() {
        let err = FlattenedShapeSurfaceInput::parse_input(json!({
            "valu": "  ok  "
        }))
        .expect_err("flattened shape should suggest the nested field name");
        let message = err.to_string();
        assert!(message.contains("unknown field 'valu'"));
        assert!(message.contains("Did you mean `value`?"));
    }

    #[test]
    fn tool_input_shape_schema_usage_renders_action_aliases() {
        let schema = NormalizedVariantShapeInput::input_schema();
        let usage = crate::tool::definition::schema_usage_text(&schema).expect("usage text");
        assert!(usage.contains("- help (aliases: describe, quick_help)"));
    }

    #[test]
    fn tool_suite_route_attributes_serialize_fields_and_converted_payloads() {
        let (field_tool, field_input) =
            RoutedToolSuite::resolve_tool("fixture.route_field", json!({ "value": "ok" }))
                .expect("field-routed suite variant should resolve");
        assert_eq!(field_tool, "fixture.inner_field");
        assert_eq!(field_input, json!({ "value": "ok" }));

        let (field_shape_tool, field_shape_input) = RoutedToolSuite::resolve_tool(
            "fixture.route_field_shape",
            json!({ "value": "  ok  " }),
        )
        .expect("shape-routed suite variant should normalize nested ToolInputShape payload");
        assert_eq!(field_shape_tool, "fixture.inner_field_shape");
        assert_eq!(field_shape_input, json!({ "value": "ok" }));

        let (convert_tool, convert_input) =
            RoutedToolSuite::resolve_tool("fixture.route_convert", json!({ "value": "ok" }))
                .expect("converted suite variant should resolve");
        assert_eq!(convert_tool, "fixture.inner_convert");
        assert_eq!(
            convert_input,
            json!({
                "action": "echo",
                "value": "ok"
            })
        );

        let (action_shape_tool, action_shape_input) =
            RoutedToolSuite::resolve_tool("fixture.route_action_shape", json!({ "value": "ok" }))
                .expect("route_action should inject action into routed payload");
        assert_eq!(action_shape_tool, "fixture.inner_action_shape");
        assert_eq!(
            action_shape_input,
            json!({
                "action": "echo",
                "value": "ok"
            })
        );
    }

    #[test]
    fn static_tool_surface_route_attributes_reuse_nested_shape_normalization() {
        let (field_tool, field_input) = SurfaceRoutedToolInput::resolve_tool(
            "fixture.surface_route",
            json!({
                "action": "field",
                "value": "  ok  "
            }),
        )
        .expect("field-routed surface variant should resolve");
        assert_eq!(field_tool, "fixture.inner_surface_field");
        assert_eq!(field_input, json!({ "value": "  ok  " }));

        let (field_shape_tool, field_shape_input) = SurfaceRoutedToolInput::resolve_tool(
            "fixture.surface_route",
            json!({
                "action": "field_shape",
                "value": "  ok  "
            }),
        )
        .expect("shape-routed surface variant should normalize nested ToolInputShape payload");
        assert_eq!(field_shape_tool, "fixture.inner_surface_field_shape");
        assert_eq!(field_shape_input, json!({ "value": "ok" }));

        let (action_shape_tool, action_shape_input) = SurfaceRoutedToolInput::resolve_tool(
            "fixture.surface_route",
            json!({
                "action": "action_shape",
                "value": "ok"
            }),
        )
        .expect("route_action should inject an action before nested ToolInputShape parsing");
        assert_eq!(action_shape_tool, "fixture.inner_surface_action_shape");
        assert_eq!(
            action_shape_input,
            json!({
                "action": "echo",
                "value": "ok"
            })
        );
    }

    #[test]
    fn built_in_relation_constraints_run_after_parse() {
        let err = RelationValidatedShapeInput::parse_input(json!({
            "confirm": "yes"
        }))
        .expect_err("requires should reject missing dependent field");
        assert!(err.to_string().contains("field `confirm` requires `token`"));

        let err = RelationValidatedShapeInput::parse_input(json!({
            "id": "doc_1",
            "url": "https://example.com"
        }))
        .expect_err("conflicts_with should reject conflicting fields");
        assert!(err.to_string().contains("field `id` conflicts with `url`"));
    }

    #[test]
    fn wildcard_required_unless_present_runs_per_array_item() {
        let err = QuestionChoiceShapeInput::parse_input(json!({
            "questions": [{
                "id": "q1",
                "question": "Pick one"
            }]
        }))
        .expect_err("required_unless_present should reject missing allow_custom per item");
        assert!(err.to_string().contains(
            "field `questions[].allow_custom` is required unless `questions[].options` is present"
        ));
    }

    #[test]
    fn built_in_string_constraints_reject_forbidden_and_duplicate_values() {
        let err = PathValidatedShapeInput::parse_input(json!({
            "name": "team/preference"
        }))
        .expect_err("forbid_substrings should reject path separators");
        assert!(
            err.to_string()
                .contains("field `name` must not contain `/`")
        );

        let err = DistinctQuestionShapeInput::parse_input(json!({
            "questions": [
                { "id": "q1", "question": "One?" },
                { "id": " q1 ", "question": "Two?" }
            ]
        }))
        .expect_err("distinct_trimmed should reject duplicate trimmed ids");
        assert!(
            err.to_string()
                .contains("field `questions[].id` must not contain duplicate values")
        );

        let err = DistinctQuestionShapeInput::parse_input(json!({
            "questions": [{
                "id": "q1",
                "question": "Pick one",
                "allow_custom": true,
                "options": [
                    { "label": " ", "description": "" }
                ]
            }]
        }))
        .expect_err("non_empty_if_present should reject blank option labels");
        assert!(
            err.to_string()
                .contains("field `questions[].options[].label` must not be empty when present")
        );

        let err = DistinctQuestionShapeInput::parse_input(json!({
            "questions": [{
                "id": "q1",
                "question": "Pick one",
                "options": [
                    { "label": "A", "description": "" },
                    { "label": " A ", "description": "" }
                ]
            }]
        }))
        .expect_err("distinct_trimmed_within should reject duplicate option labels per question");
        assert!(err.to_string().contains(
            "field `questions[].options[].label` must not contain duplicate values within `questions[]`"
        ));
    }

    #[test]
    fn built_in_string_normalizers_run_before_parse() {
        let parsed = NormalizedNameShapeInput::parse_input(json!({
            "name": "  notes.md  "
        }))
        .expect("trim and trim_suffix should normalize string values before parse");
        assert_eq!(parsed.name, "notes");
    }

    #[test]
    fn flatten_shape_post_parse_normalizers_reuse_nested_shape_rules() {
        let parsed = FlattenedShapeWrapperInput::parse_input(json!({
            "value": "  nested  "
        }))
        .expect("flattened shape should reuse nested ToolInputShape normalization");
        assert_eq!(parsed.inner.value, "nested");

        let surface = FlattenedShapeSurfaceInput::parse_input(json!({
            "value": "  surfaced  "
        }))
        .expect("flattened surface should reuse nested ToolInputShape normalization");
        assert_eq!(surface.inner.value, "surfaced");

        let err = FlattenedShapeSurfaceInput::parse_input(json!({
            "value": "   "
        }))
        .expect_err("flattened surface should reuse nested ToolInputShape validation");
        assert!(err.to_string().contains("field `value` must not be empty"));
    }

    #[test]
    fn tool_input_shape_variant_constraints_run_per_enum_variant() {
        let parsed = VariantValidatedShapeInput::parse_input(json!({
            "action": "search",
            "query": "  monitor read  "
        }))
        .expect("search variant should trim and parse");
        match parsed {
            VariantValidatedShapeInput::Search { query } => {
                assert_eq!(query, "monitor read");
            }
            other => panic!("expected search variant, got {other:?}"),
        }

        let err = VariantValidatedShapeInput::parse_input(json!({
            "action": "help",
            "tool": "   "
        }))
        .expect_err("help variant should reject blank tool");
        assert!(err.to_string().contains("field `tool` must not be empty"));
    }

    #[test]
    fn multi_field_inline_variants_support_parse_time_validation() {
        let parsed = MultiFieldValidatedShapeInput::parse_input(json!({
            "action": "run",
            "value": "ok",
            "limit": 4
        }))
        .expect("multi-field shape variant should validate successfully");
        match parsed {
            MultiFieldValidatedShapeInput::Run { value, limit } => {
                assert_eq!(value, "ok");
                assert_eq!(limit, Some(4));
            }
        }

        let err = MultiFieldValidatedShapeInput::parse_input(json!({
            "action": "run",
            "value": "   ",
            "limit": 4
        }))
        .expect_err("multi-field shape variant should still enforce built-in validation");
        assert!(err.to_string().contains("field `value` must not be empty"));

        let err = MultiFieldValidatedShapeInput::parse_input(json!({
            "action": "run",
            "value": "ok",
            "limit": 99
        }))
        .expect_err("multi-field shape variant should run custom validate hook");
        assert!(err.to_string().contains("limit must be 10 or less"));

        let parsed = MultiFieldValidatedToolInput::parse_input(json!({
            "action": "run",
            "value": "ok",
            "limit": 6
        }))
        .expect("multi-field surface variant should validate successfully");
        match parsed {
            MultiFieldValidatedToolInput::Run { value, limit } => {
                assert_eq!(value, "ok");
                assert_eq!(limit, Some(6));
            }
        }

        let err = MultiFieldValidatedToolInput::parse_input(json!({
            "action": "run",
            "value": "ok",
            "limit": 42
        }))
        .expect_err("multi-field surface variant should run custom validate hook");
        assert!(err.to_string().contains("limit must be 10 or less"));
    }

    #[test]
    fn tool_input_shape_enum_normalization_matches_surface_behaviors() {
        let usage = NormalizedVariantShapeInput::parse_input(json!({}))
            .expect("empty object should map to default usage action");
        assert!(matches!(usage, NormalizedVariantShapeInput::Usage));

        let search = NormalizedVariantShapeInput::parse_input(json!({
            "query": "  permissions  ",
            "tool": "noise"
        }))
        .expect("query-only payload should infer search and drop help-only noise");
        match search {
            NormalizedVariantShapeInput::Search { query } => {
                assert_eq!(query, "permissions");
            }
            other => panic!("expected search variant, got {other:?}"),
        }

        let help = NormalizedVariantShapeInput::parse_input(json!({
            "action": "describe",
            "tool": "  agena_web__search  "
        }))
        .expect("action alias should normalize to help");
        match help {
            NormalizedVariantShapeInput::Help {
                tool,
                include_schema,
            } => {
                assert_eq!(tool, "agena_web__search");
                assert_eq!(include_schema, None);
            }
            other => panic!("expected help variant, got {other:?}"),
        }

        let quick_help = NormalizedVariantShapeInput::parse_input(json!({
            "action": "quick_help",
            "tool": "agena_web__search"
        }))
        .expect("action alias default should inject include_schema");
        match quick_help {
            NormalizedVariantShapeInput::Help {
                tool,
                include_schema,
            } => {
                assert_eq!(tool, "agena_web__search");
                assert_eq!(include_schema, Some(false));
            }
            other => panic!("expected help variant, got {other:?}"),
        }
    }

    #[test]
    fn model_safe_tool_schema_merges_top_level_object_unions() {
        let schema = json!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["action", "query"],
                    "properties": {
                        "action": { "const": "search" },
                        "query": { "type": "string" }
                    }
                },
                {
                    "type": "object",
                    "required": ["action", "path"],
                    "properties": {
                        "action": { "const": "open" },
                        "path": { "type": "string" }
                    }
                }
            ]
        });

        let safe = super::model_safe_tool_schema(&schema);

        assert_eq!(safe["type"], "object");
        assert_eq!(safe["required"], json!(["action"]));
        assert_eq!(
            safe["properties"]["action"]["enum"],
            json!(["open", "search"])
        );
        assert_eq!(safe["properties"]["query"]["type"], "string");
        assert_eq!(safe["properties"]["path"]["type"], "string");
        assert!(safe.get("oneOf").is_none());
    }

    #[derive(Debug)]
    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("agena-tool-tests-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("failed to create temp workspace");
            Self { root }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn build_executor(root: &Path) -> ToolExecutor {
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        ToolExecutor::new(root, agent).with_plugin_manager(build_default_plugin_manager(root))
    }

    #[derive(Debug, Default)]
    struct TestToolHost {
        storage: Mutex<BTreeMap<(String, String, String, String), String>>,
    }

    fn storage_scope_key(scope: HostStorageScope) -> &'static str {
        match scope {
            HostStorageScope::Session => "session",
            HostStorageScope::Workspace => "workspace",
            HostStorageScope::Global => "global",
        }
    }

    fn storage_visibility_key(visibility: HostStorageVisibility) -> &'static str {
        match visibility {
            HostStorageVisibility::Private => "private",
            HostStorageVisibility::Shared => "shared",
        }
    }

    fn storage_slot_key(
        scope: HostStorageScope,
        visibility: HostStorageVisibility,
        namespace: &str,
        key: &str,
    ) -> (String, String, String, String) {
        (
            storage_scope_key(scope).to_string(),
            storage_visibility_key(visibility).to_string(),
            namespace.to_string(),
            key.to_string(),
        )
    }

    #[async_trait::async_trait]
    impl HostClient for TestToolHost {
        async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

        async fn publish_event(&self, _env: EventEnvelope) -> SdkResult<()> {
            Ok(())
        }

        async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
            Ok(EventSubscription { id: "sub".into() })
        }

        async fn ask_permission(&self, _req: PermissionAskInput) -> SdkResult<PermissionDecision> {
            Ok(PermissionDecision::Prompt)
        }

        async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }

        async fn storage_get(
            &self,
            req: HostStorageGetRequest,
        ) -> SdkResult<HostStorageGetResponse> {
            let value = self
                .storage
                .lock()
                .expect("test storage lock should not be poisoned")
                .get(&storage_slot_key(
                    req.scope,
                    req.visibility,
                    &req.namespace,
                    &req.key,
                ))
                .cloned();
            Ok(HostStorageGetResponse { value })
        }

        async fn storage_set(&self, req: HostStorageSetRequest) -> SdkResult<()> {
            self.storage
                .lock()
                .expect("test storage lock should not be poisoned")
                .insert(
                    storage_slot_key(req.scope, req.visibility, &req.namespace, &req.key),
                    req.value,
                );
            Ok(())
        }

        async fn storage_delete(&self, req: HostStorageDeleteRequest) -> SdkResult<()> {
            self.storage
                .lock()
                .expect("test storage lock should not be poisoned")
                .remove(&storage_slot_key(
                    req.scope,
                    req.visibility,
                    &req.namespace,
                    &req.key,
                ));
            Ok(())
        }

        async fn storage_list(
            &self,
            req: HostStorageListRequest,
        ) -> SdkResult<HostStorageListResponse> {
            let scope = storage_scope_key(req.scope);
            let visibility = storage_visibility_key(req.visibility);
            let records = self
                .storage
                .lock()
                .expect("test storage lock should not be poisoned")
                .keys()
                .filter(|(slot_scope, slot_visibility, namespace, key)| {
                    slot_scope == scope
                        && slot_visibility == visibility
                        && req
                            .namespace
                            .as_ref()
                            .is_none_or(|expected| namespace == expected)
                        && req
                            .prefix
                            .as_ref()
                            .is_none_or(|expected| key.starts_with(expected))
                })
                .map(|(_, _, namespace, key)| HostStorageRecord {
                    namespace: namespace.clone(),
                    key: key.clone(),
                })
                .collect();
            Ok(HostStorageListResponse { records })
        }

        async fn invoke_tool(
            &self,
            tool: String,
            _input: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            Err(PluginError::new(format!(
                "unexpected invoke_tool for {tool}"
            )))
        }

        async fn spawn_subtask(&self, req: SpawnSubtaskRequest) -> SdkResult<SpawnSubtaskResponse> {
            Ok(SpawnSubtaskResponse {
                final_text: format!("spawned {}", req.description),
                metadata: std::collections::BTreeMap::from([(
                    "session_id".to_string(),
                    "child-1".to_string(),
                )]),
            })
        }

        async fn list_tools(&self) -> SdkResult<Vec<ToolDescriptor>> {
            Ok(vec![
                crate::plugins::provided::workflow::tools_tool_descriptor_for_tests(),
                ToolDescriptor {
                    name: FS_TOOL.to_string(),
                    aliases: Vec::new(),
                    description: Some("Patch files in the workspace".to_string()),
                    before_help: None,
                    after_help: None,
                    summary: Some("Patch files".to_string()),
                    help: Some("Patch files in the workspace.".to_string()),
                    examples: vec![
                        r#"{"action":"read","path":"Cargo.toml"}"#.to_string(),
                        r#"{"action":"grep","pattern":"StaticToolSurface","path":"crates"}"#
                            .to_string(),
                    ],
                    input_schema: Some(serde_json::json!({
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "action": { "const": "read" },
                                    "path": {
                                        "type": "string",
                                        "description": "File or directory path to preview."
                                    },
                                    "mode": {
                                        "type": "string",
                                        "enum": ["text", "attachment", "auto"],
                                        "default": "auto",
                                        "description": "Read mode."
                                    }
                                },
                                "required": ["action", "path"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "action": { "const": "grep" },
                                    "pattern": {
                                        "type": "string",
                                        "description": "Regex pattern to search."
                                    },
                                    "path": {
                                        "type": "string",
                                        "description": "Base path to search."
                                    }
                                },
                                "required": ["action", "pattern"]
                            }
                        ]
                    })),
                    description_mode: None,
                    tags: vec![
                        crate::plugin::sdk::ToolTag::Mutating,
                        crate::plugin::sdk::ToolTag::FilesystemWrite,
                    ],
                    plugin_id: None,
                },
                ToolDescriptor {
                    name: GENERATED_HELP_TOOL.to_string(),
                    aliases: Vec::new(),
                    description: Some("Structured tool without declared examples".to_string()),
                    before_help: None,
                    after_help: None,
                    summary: Some("Structured tool".to_string()),
                    help: None,
                    examples: Vec::new(),
                    input_schema: Some(serde_json::json!({
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "action": { "const": "search_ast" },
                                    "path": { "type": "string" },
                                    "pattern": { "type": "string" }
                                },
                                "required": ["action", "path", "pattern"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "action": { "const": "syntax_tree" },
                                    "path": { "type": "string" }
                                },
                                "required": ["action", "path"]
                            }
                        ]
                    })),
                    description_mode: None,
                    tags: vec![crate::plugin::sdk::ToolTag::ReadOnly],
                    plugin_id: None,
                },
                ToolDescriptor {
                    name: MERGED_HELP_TOOL.to_string(),
                    aliases: Vec::new(),
                    description: Some("Structured tool with partial declared examples".to_string()),
                    before_help: None,
                    after_help: None,
                    summary: Some("Structured merged tool".to_string()),
                    help: Some("Structured tool help should appear after examples.".to_string()),
                    examples: vec![
                        r#"{"action":"search_ast","path":"src/lib.rs","pattern":"Tool"}"#
                            .to_string(),
                    ],
                    input_schema: Some(serde_json::json!({
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "action": { "const": "search_ast" },
                                    "path": { "type": "string" },
                                    "pattern": { "type": "string" }
                                },
                                "required": ["action", "path", "pattern"]
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "action": { "const": "syntax_tree" },
                                    "path": { "type": "string" }
                                },
                                "required": ["action", "path"]
                            }
                        ]
                    })),
                    description_mode: None,
                    tags: vec![crate::plugin::sdk::ToolTag::ReadOnly],
                    plugin_id: None,
                },
            ])
        }

        async fn todo_write(
            &self,
            req: crate::plugin::sdk::host_api::HostTodoWriteRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(in_process_router::tool_execution_to_invoke_output(
                ToolPayloadExecution::new(
                    ToolPayloadOutput::TodoWrite {
                        items: req
                            .items
                            .into_iter()
                            .map(|item| TodoItem {
                                content: item.content,
                                status: match item.status {
                                    HostTodoStatus::Pending => TodoStatus::Pending,
                                    HostTodoStatus::InProgress => TodoStatus::InProgress,
                                    HostTodoStatus::Completed => TodoStatus::Completed,
                                    HostTodoStatus::Cancelled => TodoStatus::Cancelled,
                                },
                                priority: match item.priority {
                                    HostTodoPriority::High => TodoPriority::High,
                                    HostTodoPriority::Medium => TodoPriority::Medium,
                                    HostTodoPriority::Low => TodoPriority::Low,
                                },
                            })
                            .collect(),
                    },
                    ToolExecutionView::simple("Todo write", "Updated todo list"),
                ),
            ))
        }
    }

    #[derive(Debug, Default)]
    struct FixturePlugin;

    #[async_trait::async_trait]
    impl Plugin for FixturePlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::builder("fixture", "0.1.0")
                .description("fixture plugin")
                .hooks(
                    HookSubscription::TOOL_BEFORE
                        | HookSubscription::TOOL_AFTER
                        | HookSubscription::TOOL_INVOKE
                        | HookSubscription::SHELL_ENV,
                )
                .tool(
                    PluginToolDecl::new(
                        "plugin_echo",
                        json!({
                            "type": "object",
                            "properties": { "message": { "type": "string" } },
                            "required": ["message"]
                        }),
                    )
                    .description("Echo a message from the plugin.")
                    .summary("Echo a plugin message.")
                    .help("Detailed fixture help for plugin_echo.")
                    .max_model_chars(24)
                    .preview_lines(2)
                    .persist_large_output(true)
                    .ui_render_kind(crate::plugin::sdk::ToolResultRenderKind::Markdown)
                    .tag(crate::plugin::sdk::ToolTag::ReadOnly),
                )
                .tool(
                    PluginToolDecl::new(
                        "plugin_paths",
                        json!({
                            "type": "object",
                            "properties": {
                                "file_path": { "type": "string" },
                                "extra_paths": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "dynamic_path": { "type": "string" },
                                "url": { "type": "string" },
                                "dynamic_network": { "type": "string" }
                            },
                            "required": ["file_path"]
                        }),
                    )
                    .description("Expose declared and dynamic permission paths.")
                    .summary("Expose declared and dynamic permission paths.")
                    .description_mode(crate::plugin::ToolDescriptionMode::Brief)
                    .tag(crate::plugin::sdk::ToolTag::ReadOnly)
                    .input_path(InputPathSpec {
                        jsonpath: "$.file_path".to_string(),
                        kind: PathKind::Read,
                        optional: false,
                    })
                    .input_path(InputPathSpec {
                        jsonpath: "$.extra_paths[*]".to_string(),
                        kind: PathKind::Read,
                        optional: true,
                    })
                    .input_network(InputNetworkSpec {
                        jsonpath: "$.url".to_string(),
                        optional: true,
                    }),
                )
                .build()
        }

        async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
            if input.tool_name == "plugin_paths" {
                return Ok(ToolInvokeOutput::text("ok").with_title("Plugin paths"));
            }

            let message = input
                .input
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PluginError::new("missing message"))?
                .to_string();
            Ok(ToolInvokeOutput {
                title: "Plugin echo".to_string(),
                output_text: message.clone(),
                payload: Some(json!({ "echoed": message })),
                metadata: std::collections::BTreeMap::from([(
                    "plugin".to_string(),
                    "fixture".to_string(),
                )]),
                attachments: Vec::new(),
            })
        }

        async fn tool_execute_before(
            &self,
            input: ToolBeforeInput,
        ) -> Result<Option<ToolBeforePatch>> {
            if input.tool_name != "plugin_echo" {
                return Ok(None);
            }
            let mut new_input = input.input.clone();
            let message = new_input
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            new_input["message"] = serde_json::Value::String(format!("{message} prepared"));
            Ok(Some(ToolBeforePatch {
                input: Some(new_input),
                abort_reason: None,
                title_override: Some("Prepared plugin echo".to_string()),
                metadata: Default::default(),
            }))
        }

        async fn tool_execute_after(
            &self,
            input: ToolAfterInput,
        ) -> Result<Option<ToolAfterPatch>> {
            if input.tool_name != "plugin_echo" {
                return Ok(None);
            }
            let mut payload = input.payload.clone().unwrap_or_else(|| json!({}));
            payload["after"] = serde_json::Value::Bool(true);
            Ok(Some(ToolAfterPatch {
                title: Some(format!("{} after", input.title)),
                output_text: Some(format!("{} after", input.output_text)),
                payload: Some(payload),
                metadata: std::collections::BTreeMap::from([(
                    "after_hook".to_string(),
                    "applied".to_string(),
                )]),
            }))
        }

        async fn permission_paths(
            &self,
            tool: &str,
            input: &serde_json::Value,
        ) -> Result<Vec<PathRequest>> {
            if tool != "plugin_paths" {
                return Ok(Vec::new());
            }
            let Some(dynamic_path) = input.get("dynamic_path").and_then(|value| value.as_str())
            else {
                return Ok(Vec::new());
            };
            Ok(vec![PathRequest::write(dynamic_path)])
        }

        async fn permission_networks(
            &self,
            tool: &str,
            input: &serde_json::Value,
        ) -> Result<Vec<NetworkRequest>> {
            if tool != "plugin_paths" {
                return Ok(Vec::new());
            }
            let Some(target) = input
                .get("dynamic_network")
                .and_then(|value| value.as_str())
            else {
                return Ok(Vec::new());
            };
            Ok(vec![NetworkRequest::connect(target)])
        }

        async fn shell_env(&self, _input: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
            Ok(Some(ShellEnvPatch::set("PLUGIN_FLAG", "from_plugin")))
        }
    }

    fn test_plugin_runtime() -> &'static tokio::runtime::Runtime {
        use std::sync::OnceLock;

        static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        TEST_RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .expect("test plugin runtime")
        })
    }

    fn build_plugin_manager(root: &Path) -> Arc<PluginHost> {
        use std::collections::BTreeMap;

        let skills_id = super::skills_plugin_id().to_string();
        let lsp_id = super::lsp_plugin_id().to_string();
        let cron_id = super::cron_plugin_id().to_string();
        let code_id = super::code_plugin_id().to_string();
        let fs_id = super::fs_plugin_id().to_string();
        let settings_id = super::settings_plugin_id().to_string();
        let shell_id = super::shell_plugin_id().to_string();
        let workflow_id = super::workflow_plugin_id().to_string();
        let schema_lab_id = super::schema_lab_plugin_id().to_string();
        let web_id = crate::web::web_plugin_id().to_string();
        let mut list = BTreeMap::new();
        for id in [
            &skills_id,
            &lsp_id,
            &cron_id,
            &code_id,
            &fs_id,
            &settings_id,
            &shell_id,
            &workflow_id,
            &schema_lab_id,
            &web_id,
        ] {
            let config = if id == &workflow_id {
                serde_json::json!({})
            } else {
                serde_json::Value::Null
            };
            list.insert((*id).clone(), ConfiguredPlugin::static_config(config));
        }
        list.insert("fixture".to_string(), ConfiguredPlugin::static_default());
        let config = PluginsConfig {
            host: Default::default(),
            policy: Default::default(),
            list,
        };
        test_plugin_runtime().block_on(async {
            PluginHostBuilder::new(root, "test")
                .with_config(config)
                .with_host_client(Arc::new(TestToolHost::default()))
                .register_static(skills_id, super::new_skills_plugin())
                .register_static(lsp_id, super::new_lsp_plugin())
                .register_static(cron_id, super::new_cron_plugin())
                .register_static(code_id, super::new_code_plugin())
                .register_static(fs_id, super::new_fs_plugin())
                .register_static(settings_id, super::new_settings_plugin())
                .register_static(shell_id, super::new_shell_plugin())
                .register_static(workflow_id, super::new_workflow_plugin())
                .register_static(schema_lab_id, super::new_schema_lab_plugin())
                .register_static(web_id, crate::web::new_web_plugin())
                .register_static("fixture", FixturePlugin)
                .build()
                .await
                .expect("plugin host should build")
        })
    }

    fn build_default_plugin_manager(root: &Path) -> Arc<PluginHost> {
        use std::collections::BTreeMap;

        let skills_id = super::skills_plugin_id().to_string();
        let lsp_id = super::lsp_plugin_id().to_string();
        let cron_id = super::cron_plugin_id().to_string();
        let code_id = super::code_plugin_id().to_string();
        let fs_id = super::fs_plugin_id().to_string();
        let settings_id = super::settings_plugin_id().to_string();
        let shell_id = super::shell_plugin_id().to_string();
        let workflow_id = super::workflow_plugin_id().to_string();
        let schema_lab_id = super::schema_lab_plugin_id().to_string();
        let web_id = crate::web::web_plugin_id().to_string();
        let mut list = BTreeMap::new();
        for id in [
            &skills_id,
            &lsp_id,
            &cron_id,
            &code_id,
            &fs_id,
            &settings_id,
            &shell_id,
            &workflow_id,
            &schema_lab_id,
            &web_id,
        ] {
            let config = if id == &workflow_id {
                serde_json::json!({})
            } else {
                serde_json::Value::Null
            };
            list.insert((*id).clone(), ConfiguredPlugin::static_config(config));
        }
        let config = PluginsConfig {
            host: Default::default(),
            policy: Default::default(),
            list,
        };
        test_plugin_runtime().block_on(async {
            PluginHostBuilder::new(root, "test")
                .with_config(config)
                .with_host_client(Arc::new(TestToolHost::default()))
                .register_static(skills_id, super::new_skills_plugin())
                .register_static(lsp_id, super::new_lsp_plugin())
                .register_static(cron_id, super::new_cron_plugin())
                .register_static(code_id, super::new_code_plugin())
                .register_static(fs_id, super::new_fs_plugin())
                .register_static(settings_id, super::new_settings_plugin())
                .register_static(shell_id, super::new_shell_plugin())
                .register_static(workflow_id, super::new_workflow_plugin())
                .register_static(schema_lab_id, super::new_schema_lab_plugin())
                .register_static(web_id, crate::web::new_web_plugin())
                .build()
                .await
                .expect("default plugin host should build")
        })
    }

    fn build_default_plugin_manager_without_host(root: &Path) -> Arc<PluginHost> {
        use std::collections::BTreeMap;

        let skills_id = super::skills_plugin_id().to_string();
        let lsp_id = super::lsp_plugin_id().to_string();
        let cron_id = super::cron_plugin_id().to_string();
        let code_id = super::code_plugin_id().to_string();
        let fs_id = super::fs_plugin_id().to_string();
        let settings_id = super::settings_plugin_id().to_string();
        let shell_id = super::shell_plugin_id().to_string();
        let workflow_id = super::workflow_plugin_id().to_string();
        let schema_lab_id = super::schema_lab_plugin_id().to_string();
        let web_id = crate::web::web_plugin_id().to_string();
        let mut list = BTreeMap::new();
        for id in [
            &skills_id,
            &lsp_id,
            &cron_id,
            &code_id,
            &fs_id,
            &settings_id,
            &shell_id,
            &workflow_id,
            &schema_lab_id,
            &web_id,
        ] {
            let config = if id == &workflow_id {
                serde_json::json!({})
            } else {
                serde_json::Value::Null
            };
            list.insert((*id).clone(), ConfiguredPlugin::static_config(config));
        }
        let config = PluginsConfig {
            host: Default::default(),
            policy: Default::default(),
            list,
        };
        test_plugin_runtime().block_on(async {
            PluginHostBuilder::new(root, "test")
                .with_config(config)
                .register_static(skills_id, super::new_skills_plugin())
                .register_static(lsp_id, super::new_lsp_plugin())
                .register_static(cron_id, super::new_cron_plugin())
                .register_static(code_id, super::new_code_plugin())
                .register_static(fs_id, super::new_fs_plugin())
                .register_static(settings_id, super::new_settings_plugin())
                .register_static(shell_id, super::new_shell_plugin())
                .register_static(workflow_id, super::new_workflow_plugin())
                .register_static(schema_lab_id, super::new_schema_lab_plugin())
                .register_static(web_id, crate::web::new_web_plugin())
                .build()
                .await
                .expect("default plugin host without host client should build")
        })
    }

    fn sample_png_bytes() -> Vec<u8> {
        STANDARD
            .decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO9W7tYAAAAASUVORK5CYII=",
            )
            .expect("sample png should decode")
    }

    #[test]
    fn read_provided_returns_line_numbered_preview() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "one\ntwo\nthree\n").expect("failed to seed file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Read(ReadToolInput {
                file_path: "notes.txt".to_string(),
                offset: Some(2),
                limit: Some(2),
                mode: crate::message::ReadMode::Auto,
            }))
            .expect("read default tool should succeed");

        match result.output {
            ToolPayloadOutput::Read {
                preview,
                truncated,
                loaded_paths,
                attachment,
            } => {
                let preview = preview.expect("preview must exist");
                assert!(preview.contains("2: two"));
                assert!(preview.contains("3: three"));
                assert_eq!(truncated, Some(false));
                assert_eq!(loaded_paths, vec!["notes.txt".to_string()]);
                assert!(attachment.is_none());
            }
            other => panic!("expected read output, got {other:?}"),
        }
    }

    #[test]
    fn apply_patch_provided_reports_typed_file_changes() {
        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("keep.txt"), "before\n").expect("failed to seed keep.txt");
        fs::write(workspace.root.join("remove.txt"), "delete me\n")
            .expect("failed to seed remove.txt");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
                patch: "\
*** Begin Patch
*** Add File: added.txt
+created
*** Update File: keep.txt
@@
-before
+after
*** Delete File: remove.txt
*** End Patch"
                    .to_string(),
            }))
            .expect("apply_patch should succeed");

        match result.output {
            ToolPayloadOutput::ApplyPatch { changes, .. } => {
                assert_eq!(changes.len(), 3);
                assert!(changes.iter().any(|change| {
                    change.path == "added.txt" && change.kind == FileChangeKind::Added
                }));
                assert!(changes.iter().any(|change| {
                    change.path == "keep.txt" && change.kind == FileChangeKind::Updated
                }));
                assert!(changes.iter().any(|change| {
                    change.path == "remove.txt" && change.kind == FileChangeKind::Deleted
                }));
            }
            other => panic!("expected apply_patch output, got {other:?}"),
        }
    }

    #[test]
    fn apply_patch_provided_moves_files_and_reports_diff() {
        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("old.txt"), "before\n").expect("failed to seed old.txt");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
                patch: "\
*** Begin Patch
*** Update File: old.txt
*** Move to: new.txt
@@
-before
+after
*** End Patch"
                    .to_string(),
            }))
            .expect("apply_patch move should succeed");

        assert!(!workspace.root.join("old.txt").exists());
        assert_eq!(
            fs::read_to_string(workspace.root.join("new.txt")).unwrap(),
            "after\n"
        );
        match result.output {
            ToolPayloadOutput::ApplyPatch {
                changes,
                diff,
                progress,
                ..
            } => {
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[0].path, "new.txt");
                assert_eq!(changes[0].from_path.as_deref(), Some("old.txt"));
                assert_eq!(changes[0].kind, FileChangeKind::Moved);
                assert!(diff.contains("rename from old.txt"));
                assert!(diff.contains("+after"));
                assert!(
                    progress
                        .iter()
                        .any(|line| line == "applied move old.txt -> new.txt")
                );
            }
            other => panic!("expected apply_patch output, got {other:?}"),
        }
    }

    #[test]
    fn apply_patch_diff_focuses_on_hunks_instead_of_full_file_snapshots() {
        let workspace = TempWorkspace::new();
        fs::write(
            workspace.root.join("notes.txt"),
            "alpha\nkeep-one\nbeta\nkeep-two\ngamma\n",
        )
        .expect("failed to seed notes.txt");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
                patch: "\
*** Begin Patch
*** Update File: notes.txt
@@
 alpha
-keep-one
+keep-one-updated
 beta
@@
 beta
-keep-two
+keep-two-updated
 gamma
*** End Patch"
                    .to_string(),
            }))
            .expect("apply_patch should succeed");

        match result.output {
            ToolPayloadOutput::ApplyPatch { diff, .. } => {
                assert!(diff.contains("diff --git a/notes.txt b/notes.txt"));
                assert!(diff.contains("-keep-one"));
                assert!(diff.contains("+keep-one-updated"));
                assert!(!diff.lines().any(|line| line == " keep-one"));
                assert!(!diff.lines().any(|line| line == " keep-two"));
                assert!(diff.matches("@@").count() >= 2);
            }
            other => panic!("expected apply_patch output, got {other:?}"),
        }
    }

    #[test]
    fn notebook_edit_reports_structured_file_change_and_diff() {
        let workspace = TempWorkspace::new();
        fs::write(
            workspace.root.join("demo.ipynb"),
            r#"{
  "cells": [
    {
      "cell_type": "code",
      "execution_count": null,
      "metadata": {},
      "outputs": [],
      "source": [
        "print('before')\n"
      ]
    }
  ],
  "metadata": {},
  "nbformat": 4,
  "nbformat_minor": 5
}
"#,
        )
        .expect("failed to seed notebook");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::NotebookEdit(NotebookEditToolInput {
                notebook_path: "demo.ipynb".to_string(),
                cell_number: Some(0),
                new_source: "print('after')\n".to_string(),
                edit_mode: NotebookEditMode::Replace,
                cell_type: None,
            }))
            .expect("notebook_edit should succeed");

        match result.output {
            ToolPayloadOutput::NotebookEdit { changes, diff, .. } => {
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[0].path, "demo.ipynb");
                assert_eq!(changes[0].kind, FileChangeKind::Updated);
                assert!(diff.contains("diff --git a/demo.ipynb b/demo.ipynb"));
                assert!(diff.contains("print('before')"));
                assert!(diff.contains("print('after')"));
                assert!(diff.lines().any(|line| line.starts_with('-')));
                assert!(diff.lines().any(|line| line.starts_with('+')));
            }
            other => panic!("expected notebook_edit output, got {other:?}"),
        }
    }

    #[test]
    fn read_provided_auto_attaches_image_file() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("pixel.png");
        fs::write(&file_path, sample_png_bytes()).expect("failed to seed png");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Read(ReadToolInput {
                file_path: "pixel.png".to_string(),
                offset: None,
                limit: None,
                mode: crate::message::ReadMode::Auto,
            }))
            .expect("read should attach image files in auto mode");

        match result.output {
            ToolPayloadOutput::Read {
                preview,
                truncated,
                loaded_paths,
                attachment,
            } => {
                assert!(preview.is_none());
                assert!(truncated.is_none());
                assert_eq!(loaded_paths, vec!["pixel.png".to_string()]);
                let attachment = attachment.expect("attachment metadata should exist");
                assert_eq!(attachment.path, "pixel.png");
                assert_eq!(attachment.kind, crate::message::AttachmentKind::Image);
                assert_eq!(attachment.mime, "image/png");
                assert!(attachment.size_bytes > 0);
                assert_eq!(attachment.filename.as_deref(), Some("pixel.png"));
                assert_eq!(attachment.width, Some(1));
                assert_eq!(attachment.height, Some(1));
                assert_eq!(attachment.duration_ms, None);
                assert_eq!(attachment.page_count, None);
            }
            other => panic!("expected read output, got {other:?}"),
        }

        assert_eq!(result.view.attachments.len(), 1);
        let attachment = &result.view.attachments[0];
        assert_eq!(attachment.filename.as_deref(), Some("pixel.png"));
        assert_eq!(attachment.kind, crate::message::AttachmentKind::Image);
        assert_eq!(attachment.mime, "image/png");
        match &attachment.source {
            crate::message::AttachmentSource::Base64 { data } => assert!(!data.is_empty()),
            other => panic!("expected base64 attachment source, got {other:?}"),
        }
    }

    #[test]
    fn read_provided_attachment_mode_attaches_text_file() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "hello from agena\n").expect("failed to seed text file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Read(ReadToolInput {
                file_path: "notes.txt".to_string(),
                offset: None,
                limit: None,
                mode: crate::message::ReadMode::Attachment,
            }))
            .expect("read attachment mode should succeed for text files");

        match result.output {
            ToolPayloadOutput::Read {
                preview,
                truncated,
                loaded_paths,
                attachment,
            } => {
                assert!(preview.is_none());
                assert!(truncated.is_none());
                assert_eq!(loaded_paths, vec!["notes.txt".to_string()]);
                let attachment = attachment.expect("attachment metadata should exist");
                assert_eq!(attachment.path, "notes.txt");
                assert_eq!(attachment.kind, crate::message::AttachmentKind::File);
                assert_eq!(attachment.mime, "text/plain");
                assert_eq!(attachment.filename.as_deref(), Some("notes.txt"));
                assert_eq!(attachment.width, None);
                assert_eq!(attachment.height, None);
            }
            other => panic!("expected read output, got {other:?}"),
        }

        assert_eq!(result.view.attachments.len(), 1);
        let attachment = &result.view.attachments[0];
        assert_eq!(attachment.kind, crate::message::AttachmentKind::File);
        assert_eq!(attachment.mime, "text/plain");
        match &attachment.source {
            crate::message::AttachmentSource::Base64 { data } => assert!(!data.is_empty()),
            other => panic!("expected base64 attachment source, got {other:?}"),
        }
    }

    #[test]
    fn glob_and_grep_report_match_counts() {
        let workspace = TempWorkspace::new();
        fs::create_dir_all(workspace.root.join("src/nested")).expect("failed to create tree");
        fs::write(
            workspace.root.join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .expect("failed to write main.rs");
        fs::write(
            workspace.root.join("src/nested/lib.rs"),
            "pub fn value() -> i32 { 7 }\n",
        )
        .expect("failed to write lib.rs");

        let executor = build_executor(&workspace.root);

        let glob_result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Glob(GlobToolInput {
                pattern: "**/*.rs".to_string(),
                path: Some("src".to_string()),
            }))
            .expect("glob should succeed");

        match glob_result.output {
            ToolPayloadOutput::Glob { count } => {
                assert_eq!(count, Some(2));
            }
            other => panic!("expected glob output, got {other:?}"),
        }

        let grep_result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Grep(GrepToolInput {
                pattern: "hello".to_string(),
                path: Some("src".to_string()),
                include: Some("**/*.rs".to_string()),
            }))
            .expect("grep should succeed");

        match grep_result.output {
            ToolPayloadOutput::Grep { matches } => {
                assert_eq!(matches, Some(1));
            }
            other => panic!("expected grep output, got {other:?}"),
        }
    }

    #[test]
    fn task_plugin_tool_generates_session_id() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = ToolPayloadInput::Task(TaskToolInput {
            description: "inspect code".to_string(),
            prompt: "find modules".to_string(),
            subagent_type: TaskSubagentType::Explore,
            task_id: None,
            command: None,
        })
        .into_invocation();

        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("task plugin tool should succeed");

        assert_eq!(
            result
                .view
                .metadata
                .get("subagent_type")
                .map(String::as_str),
            Some("explore")
        );
        assert_eq!(
            result
                .view
                .metadata
                .get("profile_guidance")
                .map(String::as_str),
            Some(TaskSubagentType::Explore.guidance())
        );

        let payload = ToolPayloadOutput::from_tool_output("task", &result.output)
            .expect("task output should decode as tool payload");
        match payload {
            ToolPayloadOutput::Task { session_id, .. } => {
                assert!(session_id.is_some());
            }
            other => panic!("expected task payload, got {other:?}"),
        }
    }

    #[test]
    fn tools_help_provided_describes_registered_tool() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "help",
                "tool": FS_TOOL,
                "include_schema": false
            }))
            .expect("tools help input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tools help should succeed");

        let usage_index = result
            .view
            .output_text
            .find("Usage:")
            .expect("usage section should be present");
        let description_index = result
            .view
            .output_text
            .find("Description:")
            .expect("description section should be present");

        assert!(result.view.output_text.contains("Tool: agena_fs__fs"));
        assert!(result.view.output_text.contains("Description:"));
        assert!(result.view.output_text.contains("Actions:"));
        assert!(result.view.output_text.contains("Arguments for `read`:"));
        assert!(usage_index < description_index);
    }

    #[test]
    fn tools_help_renders_before_and_after_help_sections() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "help",
                "tool": TOOLS_TOOL,
                "include_schema": false
            }))
            .expect("tools help input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tools help should succeed");

        let before_help_index = result
            .view
            .output_text
            .find("Before help:")
            .expect("before help section should be present");
        let usage_index = result
            .view
            .output_text
            .find("Usage:")
            .expect("usage section should be present");
        let after_help_index = result
            .view
            .output_text
            .find("After help:")
            .expect("after help section should be present");

        assert!(before_help_index < usage_index);
        assert!(usage_index < after_help_index);
        assert!(
            result
                .view
                .output_text
                .contains("Quick reference for browsing the registered tool catalog.")
        );
        assert!(
            result.view.output_text.contains(
                "To actually run a tool, call that tool directly after reading its help."
            )
        );
    }

    #[test]
    fn tools_help_renders_declared_tool_aliases() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "help",
                "tool": TOOLS_TOOL,
                "include_schema": false
            }))
            .expect("tools help input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tools help should succeed");

        assert!(
            result
                .view
                .output_text
                .contains("Aliases: agena_workflow__tool_catalog, agena_workflow__tool_help")
        );
    }

    #[test]
    fn tools_help_accepts_declared_tool_aliases() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "help",
                "tool": "agena_workflow__tool_catalog",
                "include_schema": false
            }))
            .expect("tools help input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tools help should resolve alias");

        assert!(
            result
                .view
                .output_text
                .contains("Tool: agena_workflow__tools")
        );
        assert!(
            result
                .view
                .output_text
                .contains("Aliases: agena_workflow__tool_catalog, agena_workflow__tool_help")
        );
    }

    #[test]
    fn declared_tool_aliases_dispatch_to_canonical_plugin_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            "agena_workflow__tool_catalog",
            StructuredObject::try_from(serde_json::json!({
                "action": "usage"
            }))
            .expect("tools usage input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tool alias should dispatch to canonical tool");

        assert_eq!(result.view.title, "Tool catalog usage");
        assert!(result.view.output_text.contains("Tool catalog usage:"));
    }

    #[test]
    fn available_model_tools_include_declared_tool_aliases() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let tools = executor.available_model_tools();
        let alias = tools
            .iter()
            .find(|tool| tool.exposed_name == "agena_workflow__tool_catalog")
            .expect("declared tool alias should be model-visible");

        assert_eq!(alias.base_exposed_name.as_deref(), Some(TOOLS_TOOL));
        assert_eq!(alias.original_name, "tools");
        assert!(alias.fixed_input.is_none());
        assert!(
            alias
                .description_text()
                .contains("Alias for `agena_workflow__tools`.")
        );
    }

    #[test]
    fn tools_help_provided_includes_declared_examples() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "help",
                "tool": FS_TOOL,
                "include_schema": false
            }))
            .expect("tools help input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tools help should succeed");

        assert!(result.view.output_text.contains("Examples:"));
        assert!(result.view.output_text.contains("Declared examples:"));
        assert!(
            result
                .view
                .output_text
                .contains(r#"{"action":"read","path":"Cargo.toml"}"#)
        );
    }

    #[test]
    fn tools_help_generates_examples_from_schema_when_none_are_declared() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "help",
                "tool": GENERATED_HELP_TOOL,
                "include_schema": false
            }))
            .expect("tools help input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tools help should succeed");

        assert!(result.view.output_text.contains("Examples:"));
        assert!(result.view.output_text.contains("Generated examples:"));
        assert!(result.view.output_text.contains(
            r#"search_ast: {"action":"search_ast","path":"<path>","pattern":"<pattern>"}"#
        ));
    }

    #[test]
    fn tools_help_suggests_similar_tool_names_for_typos() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "help",
                "tool": "agena_fs__fss",
                "include_schema": false
            }))
            .expect("tools help input should serialize"),
        );
        let err = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect_err("unknown tool should suggest a close match");
        let ToolError::Plugin(message) = err else {
            panic!("expected plugin error, got {err:?}");
        };
        assert!(message.contains("unknown tool 'agena_fs__fss'"));
        assert!(message.contains("Did you mean"));
        assert!(message.contains("agena_fs__fs"));
    }

    #[test]
    fn tools_usage_output_is_structured() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "usage"
            }))
            .expect("tools usage input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tools usage should succeed");

        let usage_index = result
            .view
            .output_text
            .find("Usage:")
            .expect("usage section should be present");
        let examples_index = result
            .view
            .output_text
            .find("Examples:")
            .expect("examples section should be present");
        let notes_index = result
            .view
            .output_text
            .find("Notes:")
            .expect("notes section should be present");
        assert!(usage_index < examples_index);
        assert!(examples_index < notes_index);
        assert!(
            result
                .view
                .output_text
                .contains(r#"- {"action":"usage"} or {}"#)
        );
        assert!(
            result
                .view
                .output_text
                .contains(r#"- Search: {"action":"search","query":"web","limit":8}"#)
        );
        assert!(
            result
                .view
                .output_text
                .contains(r#"- Help: {"action":"help","tool":"agena_web__search"}"#)
        );
    }

    #[test]
    fn tools_help_merges_declared_and_generated_examples() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let invocation = ToolInvocation::new(
            TOOLS_TOOL,
            StructuredObject::try_from(serde_json::json!({
                "action": "help",
                "tool": MERGED_HELP_TOOL,
                "include_schema": false
            }))
            .expect("tools help input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("tools help should succeed");

        let usage_index = result
            .view
            .output_text
            .find("Usage:")
            .expect("usage section should be present");
        let examples_index = result
            .view
            .output_text
            .find("Examples:")
            .expect("examples section should be present");
        let help_index = result
            .view
            .output_text
            .find("Help:")
            .expect("help section should be present");
        assert!(usage_index < examples_index);
        assert!(examples_index < help_index);
        assert_eq!(
            result
                .view
                .output_text
                .matches(
                    r#"search_ast: {"action":"search_ast","path":"src/lib.rs","pattern":"Tool"}"#
                )
                .count(),
            0
        );
        assert!(result.view.output_text.contains("Examples:"));
        assert!(result.view.output_text.contains("Declared examples:"));
        assert!(result.view.output_text.contains("Generated examples:"));
        assert_eq!(
            result
                .view
                .output_text
                .matches(r#"- {"action":"search_ast","path":"src/lib.rs","pattern":"Tool"}"#)
                .count(),
            1
        );
        assert!(
            result
                .view
                .output_text
                .contains(r#"syntax_tree: {"action":"syntax_tree","path":"<path>"}"#)
        );
    }

    #[test]
    fn tool_search_messages_do_not_gate_tool_availability() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let initial = executor.available_tools();
        assert!(initial.iter().any(|tool| tool.exposed_name == TOOLS_TOOL));
        assert!(initial.iter().any(|tool| tool.exposed_name == TODO_TOOL));
        assert!(
            initial
                .iter()
                .any(|tool| tool.exposed_name == SHELL_BASH_TOOL)
        );
        assert!(initial.iter().any(|tool| tool.exposed_name == TASK_TOOL));

        let messages = vec![Message {
            id: 99,
            role: Role::Assistant,
            state: crate::message::MessageStatus::Completed,
            parts: vec![crate::message::MessagePart::with_content(
                1,
                99,
                Utc::now(),
                crate::message::ExecutionStatus::Completed,
                PartContent::text("tool search does not gate availability"),
            )],
            created_at: Utc::now(),
            metadata: crate::message::MessageMetadata::default(),
            provider_state: None,
            usage: None,
        }];
        let available = executor.available_tools_for_messages(messages.as_slice());

        assert!(
            available
                .iter()
                .any(|tool| tool.exposed_name == SHELL_BASH_TOOL)
        );
        assert!(available.iter().any(|tool| tool.exposed_name == TASK_TOOL));
    }

    #[test]
    fn plugin_entries_drive_available_tool_catalog() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let tools = executor.available_tools();
        let fs = tools
            .iter()
            .find(|tool| tool.exposed_name == FS_TOOL)
            .expect("fs tool should be available");
        assert_eq!(fs.plugin_name, super::fs_plugin_id());
        assert!(fs.has_tag(crate::plugin::sdk::ToolTag::FilesystemRead));
        let web = tools
            .iter()
            .find(|tool| tool.exposed_name == WEB_FETCH_TOOL)
            .expect("web fetch tool should be available");
        assert_eq!(web.plugin_name, crate::web::web_plugin_id());
        assert!(web.has_tag(crate::plugin::sdk::ToolTag::Network));

        let fs_count = tools
            .iter()
            .filter(|tool| tool.exposed_name == FS_TOOL)
            .count();
        assert_eq!(fs_count, 1);
    }

    #[test]
    fn available_model_tools_split_multi_action_and_nested_tagged_unions() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let tools = executor.available_model_tools();
        assert!(
            !tools
                .iter()
                .any(|tool| tool.exposed_name == "agena_workflow__plan")
        );
        assert!(
            !tools
                .iter()
                .any(|tool| tool.exposed_name == "agena_workflow__plan_get")
        );
        assert!(
            !tools
                .iter()
                .any(|tool| tool.exposed_name == "agena_workflow__plan_submit")
        );
        assert!(
            !tools
                .iter()
                .any(|tool| tool.exposed_name == "agena_workflow__plan_next")
        );
        assert!(
            !tools
                .iter()
                .any(|tool| tool.exposed_name == "agena_workflow__plan_replace")
        );
        assert!(
            !tools
                .iter()
                .any(|tool| tool.exposed_name == "agena_workflow__worktree")
        );

        let plan_set_status = tools
            .iter()
            .find(|tool| tool.exposed_name == "agena_workflow__plan_set_status")
            .expect("plan.set_status alias should be model-visible");
        let plan_current = tools
            .iter()
            .find(|tool| tool.exposed_name == "agena_workflow__plan_current")
            .expect("plan.current alias should be model-visible");
        let check_update = tools
            .iter()
            .find(|tool| tool.exposed_name == "agena_workflow__plan_update_check")
            .expect("plan.update_check alias should be model-visible");
        let worktree_existing = tools
            .iter()
            .find(|tool| tool.exposed_name == "agena_workflow__worktree_enter_existing")
            .expect("nested worktree.enter.existing alias should be model-visible");

        let plan_set_schema =
            super::model_safe_tool_schema(&plan_set_status.sanitized_input_schema());
        let check_schema = super::model_safe_tool_schema(&check_update.sanitized_input_schema());
        let worktree_schema =
            super::model_safe_tool_schema(&worktree_existing.sanitized_input_schema());

        let plan_set_properties = plan_set_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("plan.set_status schema should expose properties");
        let check_properties = check_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("plan.update_check schema should expose properties");
        let worktree_properties = worktree_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("worktree.enter.existing schema should expose properties");

        assert!(!plan_set_properties.contains_key("action"));
        assert!(
            plan_set_properties.contains_key("phase") || plan_set_properties.contains_key("status")
        );
        assert!(!plan_set_properties.contains_key("step_id"));
        assert!(
            plan_current
                .sanitized_input_schema()
                .get("required")
                .and_then(serde_json::Value::as_array)
                .is_none_or(|required| required.is_empty()),
            "plan.current should not require any input fields"
        );
        assert!(check_properties.contains_key("step_id"));
        assert!(check_properties.contains_key("check_id"));
        assert!(check_properties.contains_key("status"));
        assert!(!check_properties.contains_key("phase"));
        assert_eq!(
            worktree_properties
                .get("path")
                .and_then(|value| value.get("type"))
                .and_then(serde_json::Value::as_str),
            Some("string")
        );
        assert!(!worktree_properties.contains_key("target"));
    }

    #[test]
    fn model_alias_descriptions_prefer_action_specific_schema_guidance() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let tools = executor.available_model_tools();
        let plan_create = tools
            .iter()
            .find(|tool| tool.exposed_name == "agena_workflow__plan_create")
            .expect("plan.create alias should be model-visible");
        let plan_set_status = tools
            .iter()
            .find(|tool| tool.exposed_name == "agena_workflow__plan_set_status")
            .expect("plan.set_status alias should be model-visible");

        assert!(
            plan_create.description_text().contains("steps[].title"),
            "plan.create description should explain step titles explicitly"
        );
        assert!(
            plan_create
                .description_text()
                .contains("steps[].checks[].text"),
            "plan.create description should explain check text explicitly"
        );
        assert!(
            plan_set_status
                .description_text()
                .contains("`draft`, `active`, `blocked`, `completed`, and `cancelled`"),
            "plan.set_status description should emphasize canonical phases"
        );
    }

    #[test]
    fn readonly_model_tools_filter_mutating_action_aliases() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root))
            .with_model_id("gpt-readonly");

        let names = executor
            .available_model_tools()
            .into_iter()
            .map(|tool| tool.exposed_name)
            .collect::<std::collections::BTreeSet<_>>();

        assert!(names.contains("agena_settings__settings_get"));
        assert!(names.contains("agena_settings__settings_list"));
        assert!(names.contains("agena_settings__settings_validate"));
        assert!(!names.contains("agena_settings__settings_set"));
        assert!(!names.contains("agena_settings__settings_delete"));
        assert!(!names.contains("agena_settings__settings_patch"));

        assert!(names.contains("agena_fs__fs_read"));
        assert!(names.contains("agena_fs__fs_glob"));
        assert!(names.contains("agena_fs__fs_grep"));
        assert!(!names.contains("agena_fs__fs_apply_patch"));
        assert!(!names.contains("agena_fs__fs_notebook_edit"));
    }

    #[test]
    fn model_tool_aliases_dispatch_back_to_original_plugin_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        fs::write(workspace.root.join("main.rs"), "fn main() {}\n")
            .expect("test source should be written");

        let invocation = ToolInvocation::new(
            "agena_code__code_syntax_tree",
            StructuredObject::try_from(json!({
                "path": "main.rs"
            }))
            .expect("syntax_tree alias input should serialize"),
        );
        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("code.syntax_tree alias should dispatch successfully");

        assert!(!result.view.title.trim().is_empty());
        assert!(!result.view.output_text.trim().is_empty());
    }

    #[test]
    fn model_tool_aliases_round_trip_through_provider_tool_parsing() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        fs::write(workspace.root.join("main.rs"), "fn main() {}\n")
            .expect("test source should be written");

        let tools = executor.available_model_tools();
        let invocation = crate::session::parse_tool_invocation(
            "agena_code__code_syntax_tree",
            r#"{"path":"main.rs"}"#,
            tools.as_slice(),
        )
        .expect("provider tool parsing should accept model alias names");

        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("parsed alias invocation should execute successfully");

        assert_eq!(invocation.name, "agena_code__code_syntax_tree");
        assert!(!result.view.output_text.trim().is_empty());
    }

    #[test]
    fn brief_mode_compacts_model_aliases_back_to_base_tool_help() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root))
            .with_tool_presentation(crate::plugin::ToolPresentationConfig {
                default_mode: crate::plugin::ToolDescriptionMode::Brief,
                ..Default::default()
            });

        let alias = executor
            .available_model_tools()
            .into_iter()
            .find(|tool| tool.exposed_name == "agena_settings__settings_get")
            .expect("settings.get alias should be model-visible");
        assert!(
            alias
                .description_text()
                .contains("`tools.help` for `agena_settings__settings`")
        );
        assert!(
            alias
                .description_text()
                .contains("Alias fixed to `action` = `get`")
        );
    }

    #[test]
    fn brief_mode_compacts_model_visible_tool_descriptions() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root))
            .with_tool_presentation(crate::plugin::ToolPresentationConfig {
                default_mode: crate::plugin::ToolDescriptionMode::Brief,
                ..Default::default()
            });

        let visible = executor
            .available_tools()
            .into_iter()
            .find(|tool| tool.exposed_name == FIXTURE_ECHO_TOOL)
            .expect("plugin_echo should be model-visible");
        assert_eq!(
            visible.description_text(),
            "Echo a plugin message. See `tools.help` for `fixture__plugin_echo`."
        );
        assert!(
            visible.decl.help.is_none(),
            "detailed help should not be carried in provider-visible tool definitions"
        );

        let detailed = executor
            .detailed_tools()
            .into_iter()
            .find(|tool| tool.exposed_name == FIXTURE_ECHO_TOOL)
            .expect("plugin_echo should have detailed help");
        assert_eq!(
            detailed.help_text(),
            Some("Detailed fixture help for plugin_echo.")
        );
    }

    #[test]
    fn tool_defined_brief_mode_compacts_visible_description_without_host_override() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        let visible = executor
            .available_tools()
            .into_iter()
            .find(|tool| tool.exposed_name == "fixture__plugin_paths")
            .expect("plugin_paths should be model-visible");
        assert_eq!(
            visible.description_text(),
            "Expose declared and dynamic permission paths. See `tools.help` for `fixture__plugin_paths`."
        );

        let detailed = executor
            .detailed_tools()
            .into_iter()
            .find(|tool| tool.exposed_name == "fixture__plugin_paths")
            .expect("plugin_paths should have a detailed definition");
        assert_eq!(
            detailed.description_text(),
            "Expose declared and dynamic permission paths."
        );
    }

    #[test]
    fn builtin_tool_defaults_compact_low_frequency_web_and_workflow_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let tools = executor.available_tools();
        let model_tools = executor.available_model_tools();

        let web_query = model_tools
            .iter()
            .find(|tool| tool.exposed_name == WEB_QUERY_TOOL)
            .expect("web query should be model-visible");
        assert_eq!(
            web_query.description_text(),
            "Search locally stored crawl documents. See `tools.help` for `agena_web__store_query`."
        );

        let workflow_tools = tools
            .iter()
            .find(|tool| tool.exposed_name == TOOLS_TOOL)
            .expect("tools catalog should be model-visible");
        assert_eq!(
            workflow_tools.description_text(),
            "Show usage examples, search tools, or fetch detailed tool help. See `tools.help` for `agena_workflow__tools`."
        );

        let web_search = tools
            .iter()
            .find(|tool| tool.exposed_name == WEB_SEARCH_TOOL)
            .expect("web search should be model-visible");
        assert_eq!(
            web_search.description_text(),
            "Search the public web through the configured search engine."
        );

        let task = tools
            .iter()
            .find(|tool| tool.exposed_name == TASK_TOOL)
            .expect("task tool should be model-visible");
        assert!(
            !task.description_text().contains("See `tools.help`"),
            "task should stay detailed by default"
        );
    }

    #[test]
    fn available_tools_are_backed_by_plugin_registry() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        for definition in executor.available_tools() {
            assert!(
                executor
                    .plugin_manager()
                    .lookup_tool(definition.exposed_name.as_str())
                    .is_some(),
                "missing registered tool for {}",
                definition.exposed_name
            );
        }

        for definition in executor.searchable_tools() {
            assert!(
                executor
                    .plugin_manager()
                    .lookup_tool(definition.exposed_name.as_str())
                    .is_some(),
                "missing registered tool for {}",
                definition.exposed_name
            );
        }
    }

    #[test]
    fn plugin_entries_are_projected_into_available_tool_catalog() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let available = executor
            .searchable_tools()
            .into_iter()
            .map(|item| item.exposed_name)
            .collect::<std::collections::BTreeSet<_>>();
        let registry = executor
            .plugin_manager()
            .registered_tools()
            .into_iter()
            .map(|entry| entry.exposed_name.clone())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(available, registry);
    }

    #[test]
    fn available_tools_are_sorted_stably_for_request_fingerprints() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let names = executor
            .available_tools()
            .into_iter()
            .map(|tool| tool.exposed_name)
            .collect::<Vec<_>>();
        let mut expected = names.clone();
        expected.sort();

        assert_eq!(names, expected);
    }

    #[test]
    fn todo_write_provided_returns_items_for_session_state() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::TodoWrite(TodoWriteToolInput {
                items: vec![TodoItem {
                    content: "Implement tool_search".to_string(),
                    status: TodoStatus::InProgress,
                    priority: TodoPriority::High,
                }],
            }))
            .expect("todo_write should succeed");

        match result.output {
            ToolPayloadOutput::TodoWrite { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].content, "Implement tool_search");
                assert_eq!(items[0].status, TodoStatus::InProgress);
            }
            other => panic!("expected todo_write output, got {other:?}"),
        }
    }

    #[test]
    fn bash_provided_runs_command() {
        if cfg!(windows) {
            // Windows host environments can include PATH entries whose ACL cannot be audited
            // in shell preflight, which makes this smoke test flaky/non-portable.
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(ShellCommandInput {
                command: "echo hello_agena".to_string(),
                description: "smoke bash".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: Vec::new(),
                network_effects: Vec::new(),
            }))
            .expect("bash default tool should succeed");

        match &result.output {
            ToolPayloadOutput::Bash {
                output,
                description,
            } => {
                let output = output
                    .as_deref()
                    .expect("output should exist")
                    .to_ascii_lowercase();
                assert!(output.contains("hello_agena"));
                assert!(description.is_some());
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }

    #[test]
    fn shell_exec_bash_invocation_runs_command() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = ToolPayloadInput::Bash(ShellCommandInput {
            command: "echo hello_shell_exec".to_string(),
            description: "grouped shell exec".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: Vec::new(),
            network_effects: Vec::new(),
        })
        .into_invocation();
        assert_eq!(invocation.name, SHELL_BASH_TOOL);

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should succeed for shell.exec");
        let payload = serde_json::Value::from(prepared.invocation.input.clone());
        assert!(payload.get("action").is_none());
        assert!(payload.get("shell").is_none());
        assert_eq!(payload["command"], "echo hello_shell_exec");

        let execution = executor
            .execute_invocation_detailed(&prepared.invocation, 7, 9)
            .expect("grouped shell exec should succeed");

        match ToolPayloadOutput::from_tool_output("bash", &execution.output) {
            Some(ToolPayloadOutput::Bash { output, .. }) => {
                let output = output
                    .as_deref()
                    .expect("output should exist")
                    .to_ascii_lowercase();
                assert!(output.contains("hello_shell_exec"));
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }

    #[test]
    fn bash_provided_explains_no_match_exit_codes() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("notes.txt"), "alpha\nbeta\n")
            .expect("failed to seed notes file");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(ShellCommandInput {
                command: "grep missing notes.txt".to_string(),
                description: "search missing text".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: vec![crate::message::FilesystemEffect {
                    path: "notes.txt".to_string(),
                    access: crate::message::FilesystemAccess::Read,
                }],
                network_effects: Vec::new(),
            }))
            .expect("bash default tool should succeed");

        match result.output {
            ToolPayloadOutput::Bash {
                output,
                description,
            } => {
                assert!(
                    output
                        .as_deref()
                        .is_some_and(|text| text.contains("no matches"))
                );
                assert!(
                    description
                        .as_deref()
                        .is_some_and(|text| text.contains("no matches"))
                );
            }
            other => panic!("expected bash output, got {other:?}"),
        }

        assert_eq!(
            result
                .view
                .metadata
                .get("exit_interpretation")
                .map(String::as_str),
            Some("no_matches")
        );
    }

    #[test]
    fn bash_provided_explains_diff_exit_codes() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("left.txt"), "alpha\n").expect("failed to write left file");
        fs::write(workspace.root.join("right.txt"), "beta\n").expect("failed to write right file");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(ShellCommandInput {
                command: "diff left.txt right.txt".to_string(),
                description: "compare files".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: vec![
                    crate::message::FilesystemEffect {
                        path: "left.txt".to_string(),
                        access: crate::message::FilesystemAccess::Read,
                    },
                    crate::message::FilesystemEffect {
                        path: "right.txt".to_string(),
                        access: crate::message::FilesystemAccess::Read,
                    },
                ],
                network_effects: Vec::new(),
            }))
            .expect("bash default tool should succeed");

        match &result.output {
            ToolPayloadOutput::Bash { description, .. } => {
                assert!(
                    description
                        .as_deref()
                        .is_some_and(|text| text.contains("found differences"))
                );
            }
            other => panic!("expected bash output, got {other:?}"),
        }

        assert_eq!(
            result
                .view
                .metadata
                .get("exit_interpretation")
                .map(String::as_str),
            Some("differences_found")
        );
    }

    #[test]
    fn bash_provided_rejects_obvious_write_without_declared_effects() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let err = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(ShellCommandInput {
                command: "echo hi > created.txt".to_string(),
                description: "attempt write".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: Vec::new(),
                network_effects: Vec::new(),
            }))
            .expect_err("write command should be rejected before execution");

        match err {
            ToolError::InvalidInput(message) => {
                assert!(message.contains("filesystem_effects"));
                assert!(message.contains("touch the filesystem"));
            }
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn readonly_model_profile_keeps_fs_tool_and_disables_task_tool() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root).with_model_id("gpt-readonly");

        let availability = executor.available_tools();
        let names = availability
            .iter()
            .map(|item| item.exposed_name.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        assert!(names.contains(FS_TOOL));
        assert!(!names.contains(TASK_TOOL));

        let err = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
                patch: "*** Begin Patch\n*** Add File: blocked.txt\n+nope\n*** End Patch"
                    .to_string(),
            }))
            .expect_err("readonly profile should reject mutating fs subcommands");
        assert!(matches!(err, ToolError::PermissionDenied(_)));
    }

    #[test]
    fn plugin_custom_tool_hooks_prepare_and_mutate_execution() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        assert!(executor.available_tools().iter().any(|tool| {
            tool.exposed_name == FIXTURE_ECHO_TOOL && tool.plugin_name == "fixture"
        }));

        let invocation = ToolInvocation {
            name: FIXTURE_ECHO_TOOL.to_string(),
            plugin_name: None,
            input: StructuredObject::try_from(json!({ "message": "hello" }))
                .expect("structured object should build"),
        };

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should succeed");
        assert_eq!(
            prepared.title_override.as_deref(),
            Some("Prepared plugin echo")
        );

        let ToolInvocation { input, .. } = &prepared.invocation;
        let prepared_value = serde_json::Value::from(input.clone());
        assert_eq!(prepared_value["message"], "hello prepared");

        let execution = executor
            .execute_invocation_detailed(&prepared.invocation, 7, 9)
            .expect("plugin execution should succeed");

        let payload = serde_json::Value::from(execution.output.payload.clone());
        assert_eq!(payload["echoed"], "hello prepared");
        assert_eq!(payload["after"], true);

        assert_eq!(execution.view.title, "Plugin echo after");
        assert_eq!(execution.view.output_text, "hello prepared after");
        assert_eq!(
            execution
                .view
                .metadata
                .get("after_hook")
                .map(String::as_str),
            Some("applied")
        );
    }

    #[test]
    fn plugin_result_policy_truncates_and_persists_large_output() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));
        let invocation = ToolInvocation {
            name: FIXTURE_ECHO_TOOL.to_string(),
            plugin_name: None,
            input: StructuredObject::try_from(json!({
                "message": "alpha line\nbeta line\ngamma line with tail"
            }))
            .expect("structured object should build"),
        };

        let prepared = executor
            .prepare_invocation(&invocation, 7, 11)
            .expect("prepare should succeed");
        let execution = executor
            .execute_invocation_detailed(&prepared.invocation, 7, 11)
            .expect("plugin execution should succeed");

        assert_eq!(
            execution
                .view
                .metadata
                .get("result_policy_truncated")
                .map(String::as_str),
            Some("true")
        );
        assert_eq!(
            execution
                .view
                .metadata
                .get("result_policy_ui_render_kind")
                .map(String::as_str),
            Some("markdown")
        );
        let persisted_path = execution
            .view
            .metadata
            .get("result_policy_persisted_path")
            .expect("persisted path should be recorded");
        let persisted = fs::read_to_string(persisted_path).expect("persisted output should exist");
        assert!(persisted.contains("alpha line"));
        assert!(persisted.contains("after"));
        assert!(execution.view.output_text.contains("output truncated"));
    }

    #[test]
    fn prepare_invocation_keeps_provided_calls_in_action_wire_shape() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = ToolPayloadInput::Read(ReadToolInput {
            file_path: "notes.txt".to_string(),
            offset: Some(3),
            limit: Some(5),
            mode: crate::message::ReadMode::Auto,
        })
        .into_invocation();

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should succeed for provided");

        let ToolInvocation {
            name,
            input,
            plugin_name,
        } = prepared.invocation;
        assert_eq!(name, FS_TOOL);
        assert_eq!(plugin_name.as_deref(), Some(super::fs_plugin_id()));
        let payload = serde_json::Value::from(input);
        assert_eq!(payload["action"], "read");
        assert_eq!(payload["file_path"], "notes.txt");
        assert_eq!(payload["offset"], 3);
        assert_eq!(payload["limit"], 5);
    }

    #[test]
    fn prepare_invocation_preserves_plugin_tool_name() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = ToolInvocation {
            name: "mcp:docs:search".to_string(),
            plugin_name: None,
            input: StructuredObject::try_from(json!({ "query": "plugin host" }))
                .expect("structured object should build"),
        };

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should preserve plugin tool invocation");

        let ToolInvocation {
            name,
            input,
            plugin_name,
        } = prepared.invocation;
        assert_eq!(name, "mcp:docs:search");
        assert_eq!(plugin_name.as_deref(), Some("custom"));
        let payload = serde_json::Value::from(input);
        assert_eq!(payload["query"], "plugin host");
    }

    #[test]
    fn prepare_unknown_invocation_skips_plugin_hooks_without_host_context() {
        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        let executor = ToolExecutor::new(&workspace.root, agent)
            .with_plugin_manager(build_default_plugin_manager_without_host(&workspace.root));
        let invocation = ToolInvocation {
            name: "agena_fs__fss".to_string(),
            plugin_name: None,
            input: StructuredObject::default(),
        };

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("unknown tools should not trigger plugin before hooks");

        assert_eq!(prepared.invocation.name, "agena_fs__fss");
        assert_eq!(prepared.invocation.plugin_name.as_deref(), Some("custom"));
        assert!(prepared.title_override.is_none());
        assert!(prepared.metadata.is_empty());

        let err = executor
            .execute_invocation_detailed(&prepared.invocation, 7, 9)
            .expect_err("unknown tools should still fail as unknown at execution time");
        assert!(matches!(
            err,
            ToolError::UnknownToolHint { tool, suggestions, suggestion_text }
                if tool == "agena_fs__fss"
                    && suggestions == vec!["agena_fs__fs".to_string()]
                    && suggestion_text == "unknown tool 'agena_fs__fss'. Did you mean `agena_fs__fs`?"
        ));
    }

    #[test]
    fn builtin_unknown_tool_suggests_close_match() {
        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        let executor = ToolExecutor::new(&workspace.root, agent);

        let err = orchestrator::execute_tool(
            &executor,
            "grepp",
            serde_json::Value::Object(Default::default()),
            ToolRuntimeContext::default(),
        )
        .expect_err("unknown built-in tools should suggest a close match");

        assert!(matches!(
            err,
            ToolError::UnknownToolHint { tool, suggestions, suggestion_text }
                if tool == "grepp"
                    && suggestions == vec!["grep".to_string()]
                    && suggestion_text == "unknown tool 'grepp'. Did you mean `grep`?"
        ));
    }

    #[test]
    fn collect_permission_checks_for_plugin_invocation_uses_declared_and_dynamic_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));
        let invocation = ToolInvocation {
            name: "fixture__plugin_paths".to_string(),
            plugin_name: None,
            input: StructuredObject::try_from(json!({
                "file_path": "docs/spec.md",
                "extra_paths": ["notes/a.md", "notes/b.md"],
                "dynamic_path": "logs/output.txt",
                "url": "https://docs.rs/",
                "dynamic_network": "api.example.com:443"
            }))
            .expect("structured object should build"),
        };

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("permission collection should succeed");

        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("docs/spec.md")),
        )));
        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("notes/a.md")),
        )));
        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("notes/b.md")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("logs/output.txt")),
        )));

        let network_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::NetworkAccess { host, port, .. } => {
                    Some((host.clone(), *port))
                }
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(network_actions.contains(&("docs.rs".to_string(), Some(443))));
        assert!(network_actions.contains(&("api.example.com".to_string(), Some(443))));
    }

    #[test]
    fn collect_permission_checks_for_provided_invocation_uses_dynamic_plugin_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
            patch: "*** Begin Patch\n*** Add File: notes.txt\n+hello\n*** Delete File: old.txt\n*** End Patch"
                .to_string(),
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("default permission collection should succeed");

        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("notes.txt")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("old.txt")),
        )));
    }

    #[test]
    fn collect_permission_checks_for_workflow_worktree_use_project_state_paths_only() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let named_invocation = ToolPayloadInput::EnterWorktree(EnterWorktreeToolInput {
            name: Some("demo".to_string()),
            path: None,
        })
        .into_invocation();
        let named_checks = executor
            .collect_permission_checks_for_invocation(&named_invocation)
            .expect("named worktree permission collection should succeed");
        let named_paths = named_checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            named_paths,
            std::collections::HashSet::from([(
                "write".to_string(),
                super::normalize_path_for_display(
                    &crate::project_paths::project_state_dir(&workspace.root).join("worktrees"),
                ),
            )]),
            "new worktree creation should only request the managed worktrees directory"
        );

        let outside = workspace.root.with_file_name(format!(
            "{}-existing-worktree",
            workspace
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("agena")
        ));
        let existing_invocation = ToolPayloadInput::EnterWorktree(EnterWorktreeToolInput {
            name: None,
            path: Some(outside.to_string_lossy().to_string()),
        })
        .into_invocation();
        let existing_checks = executor
            .collect_permission_checks_for_invocation(&existing_invocation)
            .expect("existing worktree permission collection should succeed");
        let existing_paths = existing_checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        let outside_display = super::normalize_path_for_display(&outside);
        assert_eq!(
            existing_paths,
            std::collections::HashSet::from([
                ("read".to_string(), outside_display.clone()),
                ("write".to_string(), outside_display),
            ]),
            "existing worktrees should request read/write access to the selected path only"
        );
    }

    #[test]
    fn resolve_target_path_expands_managed_project_alias() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let resolved = executor.resolve_target_path("~/agena/projects/<workspace>/plans");

        assert_eq!(
            resolved,
            crate::project_paths::project_state_dir(&workspace.root).join("plans"),
        );
    }

    #[test]
    fn collect_permission_checks_for_glob_and_grep_use_explicit_base_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let glob_invocation = ToolPayloadInput::Glob(GlobToolInput {
            pattern: "**/*.rs".to_string(),
            path: Some("packages/app".to_string()),
        })
        .into_invocation();
        let grep_invocation = ToolPayloadInput::Grep(GrepToolInput {
            pattern: "main".to_string(),
            path: Some("src".to_string()),
            include: None,
        })
        .into_invocation();

        let glob_checks = executor
            .collect_permission_checks_for_invocation(&glob_invocation)
            .expect("glob permission collection should succeed");
        let grep_checks = executor
            .collect_permission_checks_for_invocation(&grep_invocation)
            .expect("grep permission collection should succeed");

        let glob_paths = glob_checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        let grep_paths = grep_checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(glob_paths.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app")),
        )));
        assert!(grep_paths.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("src")),
        )));
    }

    #[test]
    fn collect_permission_checks_for_lsp_invocation_uses_file_path() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::LspDefinition(LspDefinitionToolInput {
            position: LspPositionToolInput {
                file_path: "src/lib.rs".to_string(),
                line: 3,
                character: 8,
            },
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("lsp permission collection should succeed");
        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("src/lib.rs")),
        )));
    }

    #[test]
    fn collect_permission_checks_for_monitor_start_uses_declared_targets() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Monitor(MonitorToolInput::Start {
            command: ShellCommandInput {
                command: "curl https://status.example.com/health".to_string(),
                description: "watch status".to_string(),
                timeout_ms: Some(5_000),
                workdir: Some("services/api".to_string()),
                filesystem_effects: Vec::new(),
                network_effects: vec![NetworkEffect {
                    target: "https://status.example.com/health".to_string(),
                }],
            },
            persistent: false,
            include_pattern: None,
            max_buffered_lines: None,
            capture_stderr: true,
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("monitor permission collection should succeed");
        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        let network_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::NetworkAccess { host, port, .. } => {
                    Some((host.clone(), *port))
                }
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("services/api")),
        )));
        assert!(network_actions.contains(&("status.example.com".to_string(), Some(443))));
    }

    #[test]
    fn web_fetch_uses_network_permission_policy() {
        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all())
            .try_with_permission_config(&crate::agent::PermissionConfig {
                network: Some(crate::agent::NetworkPermissionConfig {
                    loopback: Some(crate::permission::PermissionMode::Deny),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .expect("network permission config compiles");
        let executor = ToolExecutor::new(workspace.root.clone(), agent)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let err = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::WebFetch(WebFetchToolInput {
                url: "http://localhost:8000/".to_string(),
                prompt: None,
            }))
            .expect_err("loopback fetch should be denied before request");

        match err {
            ToolError::PermissionDenied(reason) => assert!(reason.contains("loopback")),
            other => panic!("expected network permission denial, got {other:?}"),
        }
    }

    #[test]
    fn web_fetch_and_search_inputs_trim_and_validate_at_parse_time() {
        let fetch = WebFetchToolInput::parse_input(json!({
            "url": "  https://example.com/docs  ",
            "prompt": "  summarize it  "
        }))
        .expect("web fetch input should parse");
        assert_eq!(fetch.url, "https://example.com/docs");
        assert_eq!(fetch.prompt.as_deref(), Some("summarize it"));

        let fetch_schema = WebFetchToolInput::input_schema();
        let fetch_usage =
            crate::tool::definition::schema_usage_text(&fetch_schema).expect("fetch usage");
        assert!(fetch_usage.contains("Absolute URL to fetch."));

        let search = WebSearchToolInput::parse_input(json!({
            "query": "  rust async runtime  ",
            "allowed_domains": ["  docs.rs  ", "  rust-lang.org  "],
            "blocked_domains": ["  example.com  "],
            "max_results": 4
        }))
        .expect("web search input should parse");
        assert_eq!(search.query, "rust async runtime");
        assert_eq!(
            search.allowed_domains,
            vec!["docs.rs".to_string(), "rust-lang.org".to_string()]
        );
        assert_eq!(search.blocked_domains, vec!["example.com".to_string()]);
        assert_eq!(search.max_results, Some(4));

        let search_schema = WebSearchToolInput::input_schema();
        let search_usage =
            crate::tool::definition::schema_usage_text(&search_schema).expect("search usage");
        assert!(search_usage.contains("Search query text."));
        assert!(
            search_usage.contains("Restrict results to these domains; empty means no restriction.")
        );

        let err = WebSearchToolInput::parse_input(json!({
            "query": "   "
        }))
        .expect_err("blank web search query should be rejected");
        assert!(err.to_string().contains("field `query` must not be empty"));
    }

    #[test]
    fn read_glob_grep_and_todo_inputs_trim_and_validate_at_parse_time() {
        let read = ReadToolInput::parse_input(json!({
            "file_path": "  docs/README.md  ",
            "offset": 3,
            "limit": 10,
            "mode": "auto"
        }))
        .expect("read input should parse");
        assert_eq!(read.file_path, "docs/README.md");
        assert_eq!(read.offset, Some(3));
        assert_eq!(read.limit, Some(10));

        let read_schema = ReadToolInput::input_schema();
        let read_usage =
            crate::tool::definition::schema_usage_text(&read_schema).expect("read usage");
        assert!(read_usage.contains("File or directory path to read."));
        assert!(read_usage.contains("1-based offset for file lines or directory entries."));
        assert!(read_usage.contains("`file_path` <string, required, min_length=1>"));

        let glob = GlobToolInput::parse_input(json!({
            "pattern": "  **/*.rs  ",
            "path": "  crates  "
        }))
        .expect("glob input should parse");
        assert_eq!(glob.pattern, "**/*.rs");
        assert_eq!(glob.path.as_deref(), Some("crates"));

        let glob_schema = GlobToolInput::input_schema();
        let glob_usage =
            crate::tool::definition::schema_usage_text(&glob_schema).expect("glob usage");
        assert!(glob_usage.contains("Glob pattern to match."));
        assert!(glob_usage.contains("Optional base path. Defaults to the workspace root."));
        assert!(glob_usage.contains("`path` <string, optional, min_length=1>"));

        let grep = GrepToolInput::parse_input(json!({
            "pattern": "  TODO|FIXME  ",
            "path": "  crates  ",
            "include": "  src/**/*.rs  "
        }))
        .expect("grep input should parse");
        assert_eq!(grep.pattern, "TODO|FIXME");
        assert_eq!(grep.path.as_deref(), Some("crates"));
        assert_eq!(grep.include.as_deref(), Some("src/**/*.rs"));

        let grep_schema = GrepToolInput::input_schema();
        let grep_usage =
            crate::tool::definition::schema_usage_text(&grep_schema).expect("grep usage");
        assert!(grep_usage.contains("Regex pattern to search for."));
        assert!(grep_usage.contains("Optional glob filter applied before matching lines."));
        assert!(grep_usage.contains("`include` <string, optional, min_length=1>"));

        let todo = TodoWriteToolInput::parse_input(json!({
            "items": [
                {
                    "content": "  write docs  ",
                    "status": "pending",
                    "priority": "high"
                }
            ]
        }))
        .expect("todo write input should parse");
        assert_eq!(todo.items[0].content, "write docs");

        let todo_schema = TodoWriteToolInput::input_schema();
        let todo_usage =
            crate::tool::definition::schema_usage_text(&todo_schema).expect("todo usage");
        assert!(todo_usage.contains("Todo items to replace or persist."));
        assert!(todo_usage.contains("Todo item text."));
        assert!(todo_usage.contains("`items[].content` <string, required, min_length=1>"));

        let ask_usage = crate::tool::definition::schema_usage_text(
            &crate::message::AskUserToolInput::input_schema(),
        )
        .expect("ask usage");
        assert!(ask_usage.contains("`questions` <array<object>, optional"));
        assert!(ask_usage.contains("min_items=1"));
        assert!(ask_usage.contains("max_items=3"));
        assert!(ask_usage.contains("`questions[].header` <string, optional, max_length=12>"));
        assert!(ask_usage.contains("`questions[].options` <array<object>, optional, max_items=8>"));
        assert!(ask_usage.contains("`questions[].id` <string, required, min_length=1>"));
        assert!(ask_usage.contains("`questions[].question` <string, required, min_length=1>"));
        assert!(ask_usage.contains("Relations:"));
        assert!(ask_usage.contains("required_unless_present"));

        let err = ReadToolInput::parse_input(json!({
            "file_path": "   "
        }))
        .expect_err("blank read path should be rejected");
        assert!(
            err.to_string()
                .contains("field `file_path` must not be empty")
        );

        let err = GlobToolInput::parse_input(json!({
            "pattern": "   "
        }))
        .expect_err("blank glob pattern should be rejected");
        assert!(
            err.to_string()
                .contains("field `pattern` must not be empty")
        );

        let err = GrepToolInput::parse_input(json!({
            "pattern": "  TODO  ",
            "include": "   "
        }))
        .expect_err("blank grep include should be rejected");
        assert!(
            err.to_string()
                .contains("field `include` must not be empty")
        );

        let err = TodoWriteToolInput::parse_input(json!({
            "items": [
                {
                    "content": "   ",
                    "status": "pending",
                    "priority": "low"
                }
            ]
        }))
        .expect_err("blank todo content should be rejected");
        assert!(
            err.to_string()
                .contains("field `items[].content` must not be empty")
        );
    }

    #[test]
    fn short_tool_name_permissions_match_unambiguous_workflow_tools() {
        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all())
            .try_with_permission_config(&crate::agent::PermissionConfig {
                tools: Some(crate::agent::ToolPermissionConfig {
                    default: Some(crate::permission::PermissionMode::Ask),
                    names: std::collections::BTreeMap::from([(
                        "plan".to_string(),
                        crate::permission::PermissionMode::Allow,
                    )]),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .expect("tool permission config should compile");
        let executor = ToolExecutor::new(workspace.root.clone(), agent)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolInvocation::new(
            "agena_workflow__plan",
            StructuredObject::try_from(json!({ "action": "current" }))
                .expect("plan invocation input should be valid"),
        );

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("permission collection should succeed");

        assert!(matches!(
            checks.first().map(|check| &check.decision),
            Some(crate::permission::PermissionDecision::Allow)
        ));
    }

    #[test]
    fn collect_permission_checks_for_bash_invocation_uses_declared_filesystem_effects() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let outside = workspace.root.with_file_name(format!(
            "{}-outside.txt",
            workspace
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("agena")
        ));
        let invocation = ToolPayloadInput::Bash(ShellCommandInput {
            command: "cat src/lib.rs > target/out.txt".to_string(),
            description: "declared effects".to_string(),
            timeout_ms: Some(30_000),
            workdir: Some("packages/app".to_string()),
            filesystem_effects: vec![
                FilesystemEffect {
                    path: "src/lib.rs".to_string(),
                    access: FilesystemAccess::Read,
                },
                FilesystemEffect {
                    path: "target/out.txt".to_string(),
                    access: FilesystemAccess::Write,
                },
                FilesystemEffect {
                    path: "Cargo.lock".to_string(),
                    access: FilesystemAccess::ReadWrite,
                },
                FilesystemEffect {
                    path: outside.to_string_lossy().to_string(),
                    access: FilesystemAccess::Write,
                },
            ],
            network_effects: vec![
                crate::message::NetworkEffect {
                    target: "https://api.example.com/upload".to_string(),
                },
                crate::message::NetworkEffect {
                    target: "cache.internal:8443".to_string(),
                },
            ],
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("permission collection should succeed");

        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app")),
        )));
        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app/src/lib.rs")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app/target/out.txt")),
        )));
        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app/Cargo.lock")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app/Cargo.lock")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&outside),
        )));

        let network_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::NetworkAccess { host, port, .. } => {
                    Some((host.clone(), *port))
                }
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        assert!(network_actions.contains(&("api.example.com".to_string(), Some(443))));
        assert!(network_actions.contains(&("cache.internal".to_string(), Some(8443))));
    }

    #[test]
    fn collect_permission_checks_for_declared_bash_write_uses_path_policy() {
        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new(
            "build",
            PermissionPolicy::new(
                crate::permission::PermissionMode::Allow,
                crate::permission::PermissionMode::Deny,
            ),
        );
        let executor = ToolExecutor::new(&workspace.root, agent)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Bash(ShellCommandInput {
            command: "touch created.txt".to_string(),
            description: "declared write".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: vec![FilesystemEffect {
                path: "created.txt".to_string(),
                access: FilesystemAccess::Write,
            }],
            network_effects: Vec::new(),
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("permission collection should succeed");
        let write_decision = checks
            .iter()
            .find_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } if access_kind == "write"
                    && target_path
                        == &super::normalize_path_for_display(
                            &workspace.root.join("created.txt"),
                        ) =>
                {
                    Some(&check.decision)
                }
                _ => None,
            })
            .expect("declared write path should be checked");

        match write_decision {
            crate::permission::PermissionDecision::Deny { reason } => {
                assert!(reason.contains("write"));
            }
            other => panic!("expected declared write to follow path policy, got {other:?}"),
        }
    }

    #[test]
    fn bash_input_requires_filesystem_effects_field() {
        let err = serde_json::from_value::<ShellCommandInput>(json!({
            "command": "pwd",
            "description": "",
            "timeout_ms": null,
            "workdir": null,
            "network_effects": []
        }))
        .expect_err("bash input should require filesystem_effects");

        assert!(err.to_string().contains("filesystem_effects"));
    }

    #[test]
    fn bash_input_requires_network_effects_field() {
        let err = serde_json::from_value::<ShellCommandInput>(json!({
            "command": "pwd",
            "description": "",
            "timeout_ms": null,
            "workdir": null,
            "filesystem_effects": []
        }))
        .expect_err("bash input should require network_effects");

        assert!(err.to_string().contains("network_effects"));
    }

    #[test]
    fn bash_tool_schema_requires_declared_effect_fields() {
        let schema = crate::tool::definition::json_schema_for::<ShellCommandInput>();
        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("bash schema should declare required fields");

        assert!(
            required
                .iter()
                .any(|field| field.as_str() == Some("filesystem_effects"))
        );
        assert!(
            schema
                .pointer("/properties/filesystem_effects")
                .and_then(serde_json::Value::as_object)
                .is_some()
        );
        assert!(
            required
                .iter()
                .any(|field| field.as_str() == Some("network_effects"))
        );
        assert!(
            schema
                .pointer("/properties/network_effects")
                .and_then(serde_json::Value::as_object)
                .is_some()
        );
    }

    #[test]
    fn bash_invocation_rejects_obvious_write_without_declared_effects() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Bash(ShellCommandInput {
            command: "touch created.txt".to_string(),
            description: "missing effects".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: Vec::new(),
            network_effects: Vec::new(),
        })
        .into_invocation();

        let err = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect_err("mutating bash without filesystem effects should be rejected");
        match err {
            ToolError::InvalidInput(message) => {
                assert!(message.contains("filesystem_effects"));
                assert!(message.contains("touch the filesystem"));
            }
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn bash_invocation_rejects_filesystem_read_without_declared_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Bash(ShellCommandInput {
            command: "cat notes.txt".to_string(),
            description: "missing read effects".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: Vec::new(),
            network_effects: Vec::new(),
        })
        .into_invocation();

        let err = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect_err("filesystem-reading bash without filesystem effects should be rejected");
        match err {
            ToolError::InvalidInput(message) => {
                assert!(message.contains("filesystem_effects"));
                assert!(message.contains("touch the filesystem"));
            }
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn bash_invocation_rejects_obvious_network_without_declared_targets() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Bash(ShellCommandInput {
            command: "curl https://example.com/health".to_string(),
            description: "missing network targets".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: Vec::new(),
            network_effects: Vec::new(),
        })
        .into_invocation();

        let err = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect_err("network bash without network_effects should be rejected");
        match err {
            ToolError::Plugin(message) | ToolError::InvalidInput(message) => {
                assert!(message.contains("network_effects"));
                assert!(message.contains("use the network"));
            }
            other => panic!("expected invalid input or plugin error, got {other:?}"),
        }
    }

    #[test]
    fn bash_invocation_allows_network_only_curl_without_filesystem_effects() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Bash(ShellCommandInput {
            command: "curl https://example.com/health".to_string(),
            description: "network only curl".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: Vec::new(),
            network_effects: vec![NetworkEffect {
                target: "https://example.com/health".to_string(),
            }],
        })
        .into_invocation();

        executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("network-only curl should not require filesystem effects");
    }

    #[test]
    fn bash_invocation_rejects_curl_file_write_without_declared_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Bash(ShellCommandInput {
            command: "curl -o download.json https://example.com/data.json".to_string(),
            description: "curl writes file".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: Vec::new(),
            network_effects: vec![NetworkEffect {
                target: "https://example.com/data.json".to_string(),
            }],
        })
        .into_invocation();

        let err = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect_err("curl file output without filesystem effects should be rejected");
        match err {
            ToolError::InvalidInput(message) => {
                assert!(message.contains("filesystem_effects"));
                assert!(message.contains("touch the filesystem"));
            }
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn bash_execution_enforces_declared_filesystem_effect_permissions() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new(
            "build",
            PermissionPolicy::new(
                crate::permission::PermissionMode::Allow,
                crate::permission::PermissionMode::Deny,
            ),
        );
        let executor = ToolExecutor::new(&workspace.root, agent)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let err = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(ShellCommandInput {
                command: "printf ok".to_string(),
                description: "declared write denied".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: vec![FilesystemEffect {
                    path: "created.txt".to_string(),
                    access: FilesystemAccess::Write,
                }],
                network_effects: Vec::new(),
            }))
            .expect_err("declared write should be denied by path policy");

        match err {
            ToolError::PermissionDenied(message) => {
                assert!(message.contains("write"));
            }
            other => panic!("expected permission denial, got {other:?}"),
        }
    }

    #[test]
    fn bash_invocation_applies_plugin_shell_env_overrides() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        let execution = executor
            .execute_invocation_detailed(
                &ToolPayloadInput::Bash(ShellCommandInput {
                    command: "printf %s \"$PLUGIN_FLAG\"".to_string(),
                    description: "print plugin env".to_string(),
                    timeout_ms: Some(30_000),
                    workdir: None,
                    filesystem_effects: Vec::new(),
                    network_effects: Vec::new(),
                })
                .into_invocation(),
                10,
                11,
            )
            .expect("bash invocation should succeed");

        match ToolPayloadOutput::from_tool_output("bash", &execution.output) {
            Some(ToolPayloadOutput::Bash {
                output,
                description: _,
            }) => {
                assert_eq!(output.as_deref(), Some("from_plugin"));
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }
}
