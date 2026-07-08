//! `agena.skills` — discovery plugin for explicit skill roots. Runtime defaults
//! do not scan implicit workspace or user-global directories.
//!
//! Packaged skills from `agena-skills` are also projected here so a fresh
//! install has workflow-like tools before any user-defined content exists.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::{collections::BTreeMap, fmt};

use agena_macros::ToolInputShape;
use agena_skills::discovery::{default_command_roots, default_roots, scan, scan_commands};
use agena_skills::skill::Skill;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HostClient, InitContext, InitOutcome, Result as SdkResult, ToolDefinitionInput,
    ToolDefinitionPatch, ToolInvokeOutput,
};

pub(crate) const SKILLS_PLUGIN_ID: &str = "agena.skills";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscoveredToolKind {
    Skill,
    Command,
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

#[derive(Debug, Clone)]
struct DiscoveredTool {
    skill: Skill,
    kind: DiscoveredToolKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input()]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("name"), non_empty("name"))]
#[serde(deny_unknown_fields)]
struct SkillsGetInput {
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInputShape)]
#[tool_input(trim("name", "args"), non_empty("name"))]
#[serde(deny_unknown_fields)]
struct SkillsRunInput {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    args: Option<String>,
}

pub(crate) struct SkillsPlugin {
    workspace_root: OnceLock<PathBuf>,
}

#[crate::plugin::sdk::agena_plugin(
    namespace = "agena",
    name = "skills",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Discover, inspect, and render skills and slash commands.",
    display = brief
)]
impl SkillsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            workspace_root: OnceLock::new(),
        }
    }

    #[hook]
    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let _ = self.workspace_root.set(ctx.workspace_root);
        Ok(InitOutcome::ack(crate::plugin::sdk::Plugin::manifest(self)))
    }

    fn render_prompt(body: &str, args: &str) -> String {
        let body = body.trim();
        let args = args.trim();
        if body.contains("$ARGUMENTS") {
            return body.replace("$ARGUMENTS", args);
        }
        if args.is_empty() {
            body.to_string()
        } else {
            format!("{body}\n\nUser arguments:\n{args}")
        }
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::new("skills invoked before init"))
    }

    fn discovered_tools(&self) -> SdkResult<BTreeMap<String, DiscoveredTool>> {
        Ok(Self::discovered_tools_for_workspace(self.workspace_root()?))
    }

    fn discovered_tools_for_workspace(workspace_root: &Path) -> BTreeMap<String, DiscoveredTool> {
        let workspace = Some(workspace_root.to_path_buf()).filter(|p| !p.as_os_str().is_empty());
        let roots = default_roots(workspace.as_deref());
        let command_roots = default_command_roots(workspace.as_deref());

        let mut skills_by_name: BTreeMap<String, Skill> = agena_skills::bundled::all()
            .unwrap_or_default()
            .into_iter()
            .map(|skill| (skill.frontmatter.name.clone(), skill))
            .collect();
        skills_by_name.extend(
            scan(&roots)
                .unwrap_or_default()
                .into_iter()
                .map(|skill| (skill.frontmatter.name.clone(), skill)),
        );

        let mut tools = BTreeMap::new();
        for skill in skills_by_name.into_values() {
            tools.insert(
                skill.frontmatter.name.clone(),
                DiscoveredTool {
                    skill,
                    kind: DiscoveredToolKind::Skill,
                },
            );
        }
        for command in scan_commands(&command_roots).unwrap_or_default() {
            tools.insert(
                command.frontmatter.name.clone(),
                DiscoveredTool {
                    skill: command,
                    kind: DiscoveredToolKind::Command,
                },
            );
        }
        tools
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
                "Generate the '{}' {category} prompt.",
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
            .find(|(name, _)| name.eq_ignore_ascii_case(normalized.as_str()))
            .map(|(name, tool)| (name.as_str(), tool))
            .ok_or_else(|| PluginError::invalid_params(format!("unknown skill '{}'", requested)))
    }

    #[hook(tool_definition)]
    fn tool_definition_patch(
        &self,
        input: ToolDefinitionInput,
    ) -> SdkResult<Option<ToolDefinitionPatch>> {
        if input.plugin_key().to_string() != SKILLS_PLUGIN_ID {
            return Ok(None);
        }
        let tools = self.discovered_tools()?;
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
            "Use `agena.skills/list` to page through discovered items.".to_string(),
            "Use `agena.skills/get` to inspect one item in full.".to_string(),
            "Use `agena.skills/run` with `name` and optional `args` to render one prompt."
                .to_string(),
        ];
        if !preview.is_empty() {
            lines.push("Preview:".to_string());
            lines.extend(preview);
        }

        let summary = match input.tool_name() {
            "list" => Some(format!(
                "List discovered skills and slash commands. Currently {skill_count} skill(s) and {command_count} command(s)."
            )),
            "get" => Some("Read one discovered skill or slash command.".to_string()),
            "run" => Some(format!(
                "Render one discovered skill or slash command prompt. Currently {skill_count} skill(s) and {command_count} command(s)."
            )),
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
        let tools = self.discovered_tools()?;
        let kind_filter = Self::normalize_kind_filter(input.kind.as_deref())?;
        let entries = tools
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
                    "- {} [{}]: {}",
                    name,
                    tool.kind,
                    Self::tool_description(tool)
                ));
            } else {
                lines.push(format!("- {} [{}]", name, tool.kind));
            }
        }
        let payload = serde_json::json!({
            "tools": entries.iter().map(|(name, tool)| serde_json::json!({
                "name": name,
                "kind": tool.kind.to_string(),
                "summary": Self::tool_description(tool),
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
        let (name, discovered_tool) = Self::resolve_tool(&tools, input.name.as_str())?;
        let summary = Self::tool_description(discovered_tool);
        let body = discovered_tool.skill.body.trim();
        let text = format!(
            "Name: {name}\nKind: {}\nSummary: {}\n\nBody:\n{}",
            discovered_tool.kind, summary, body
        );
        let payload = serde_json::json!({
            "name": name,
            "kind": discovered_tool.kind.to_string(),
            "summary": summary,
            "body": body,
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
        summary = "Render one discovered skill or slash command prompt.",
        read_only,
        ui_display = detailed,
        concurrency_safe
    )]
    async fn invoke_run(&self, input: &SkillsRunInput) -> SdkResult<ToolInvokeOutput> {
        let tools = self.discovered_tools()?;
        let (name, discovered_tool) = Self::resolve_tool(&tools, input.name.as_str())?;
        let prompt = Self::render_prompt(
            discovered_tool.skill.body.as_str(),
            input.args.as_deref().unwrap_or_default(),
        );
        Ok(ToolInvokeOutput::from_parts(
            name.to_string(),
            prompt,
            None,
            std::collections::BTreeMap::from([
                ("workflow".to_string(), name.to_string()),
                (
                    "skill_tool_kind".to_string(),
                    discovered_tool.kind.to_string(),
                ),
            ]),
            Vec::new(),
        ))
    }
}
