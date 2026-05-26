//! `agena.skills` — discovery plugin that scans the standard skill
//! roots (workspace `.agena/skills/` and user `~/.agena/skills/`) plus
//! user slash-command markdown, and registers everything it finds as dynamic
//! plugin tools.
//!
//! Packaged skills from `agena-skills` are also projected here so a fresh
//! install has workflow-like tools before any user-defined content exists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use agena_skills::discovery::{default_command_roots, default_roots, scan, scan_commands};
use agena_skills::skill::Skill;
use async_trait::async_trait;

use crate::message::WorkflowPromptToolInput;
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostToolRegisterRequest, HostToolRemoveRequest};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, Plugin, PluginManifest,
    PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput, ToolTag,
};

pub(crate) const SKILLS_PLUGIN_ID: &str = "agena.skills";

#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SkillToolInput {
    Run {
        #[serde(flatten)]
        args: WorkflowPromptToolInput,
    },
}

#[derive(Clone)]
enum DiscoveredToolKind {
    Skill,
    Command,
}

#[derive(Clone)]
struct DiscoveredTool {
    skill: Skill,
    kind: DiscoveredToolKind,
    alias: bool,
}

pub(crate) struct SkillsPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
    tools: RwLock<BTreeMap<String, DiscoveredTool>>,
}

impl SkillsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            tools: RwLock::new(BTreeMap::new()),
        }
    }

    fn host(&self) -> SdkResult<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| PluginError::new("skills host lock poisoned"))?
            .clone()
            .ok_or_else(|| PluginError::new("skills invoked before init"))
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

    fn parse_skill_input(input: serde_json::Value) -> SdkResult<WorkflowPromptToolInput> {
        match serde_json::from_value::<SkillToolInput>(input) {
            Ok(SkillToolInput::Run { args }) => Ok(args),
            Err(primary) => Err(PluginError::invalid_params(primary.to_string())),
        }
    }

    fn tool_decl(name: &str, discovered_tool: &DiscoveredTool) -> PluginToolDecl {
        let category = match discovered_tool.kind {
            DiscoveredToolKind::Skill => "workflow",
            DiscoveredToolKind::Command => "command",
        };
        let label = if discovered_tool.alias {
            "alias"
        } else {
            category
        };
        let mut tags = vec![
            ToolTag::ReadOnly,
            ToolTag::custom(category).expect("category tags are valid"),
            ToolTag::custom(format!("skill:{}", discovered_tool.skill.frontmatter.name))
                .expect("skill identity tags are valid"),
        ];
        if discovered_tool.alias {
            tags.push(ToolTag::custom("alias").expect("alias tag is valid"));
        }
        let description = if discovered_tool
            .skill
            .frontmatter
            .description
            .trim()
            .is_empty()
        {
            format!(
                "Generate the '{}' {label} prompt.",
                discovered_tool.skill.frontmatter.name
            )
        } else {
            discovered_tool.skill.frontmatter.description.clone()
        };
        PluginToolDecl::new(
            name.to_string(),
            crate::tool::definition::json_schema_for::<SkillToolInput>(),
        )
        .description(description.clone())
        .summary(description)
        .help(discovered_tool.skill.body.clone())
        .tags(tags)
        .concurrency_safe(true)
    }

    fn discovered_tools(ctx: &InitContext) -> BTreeMap<String, DiscoveredTool> {
        let workspace =
            Some(ctx.workspace_root.clone()).filter(|p: &PathBuf| !p.as_os_str().is_empty());
        let roots = default_roots(workspace.as_deref());
        let command_roots = default_command_roots(workspace.as_deref());

        let skills_by_name: BTreeMap<String, Skill> = scan(&roots)
            .unwrap_or_default()
            .into_iter()
            .map(|skill| (skill.frontmatter.name.clone(), skill))
            .collect();

        let mut tools = BTreeMap::new();
        for skill in skills_by_name.into_values() {
            tools.insert(
                skill.frontmatter.name.clone(),
                DiscoveredTool {
                    skill: skill.clone(),
                    kind: DiscoveredToolKind::Skill,
                    alias: false,
                },
            );
            for alias in &skill.frontmatter.aliases {
                tools.insert(
                    alias.clone(),
                    DiscoveredTool {
                        skill: skill.clone(),
                        kind: DiscoveredToolKind::Skill,
                        alias: true,
                    },
                );
            }
        }
        for command in scan_commands(&command_roots).unwrap_or_default() {
            tools.insert(
                command.frontmatter.name.clone(),
                DiscoveredTool {
                    skill: command.clone(),
                    kind: DiscoveredToolKind::Command,
                    alias: false,
                },
            );
            for alias in &command.frontmatter.aliases {
                tools.insert(
                    alias.clone(),
                    DiscoveredTool {
                        skill: command.clone(),
                        kind: DiscoveredToolKind::Command,
                        alias: true,
                    },
                );
            }
        }
        tools
    }

    async fn sync_tools(&self, ctx: &InitContext) -> SdkResult<()> {
        let host = self.host()?;
        let new_tools = Self::discovered_tools(ctx);
        let old_names = self
            .tools
            .read()
            .map_err(|_| PluginError::new("skills tools lock poisoned"))?
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let new_names = new_tools.keys().cloned().collect::<BTreeSet<_>>();

        for removed in old_names.difference(&new_names) {
            let _ = host
                .remove_tool(HostToolRemoveRequest {
                    name: removed.clone(),
                    exposed: false,
                })
                .await?;
        }

        for (name, discovered_tool) in &new_tools {
            let _ = host
                .register_tool(HostToolRegisterRequest {
                    tool: Self::tool_decl(name, discovered_tool),
                })
                .await?;
        }

        *self
            .tools
            .write()
            .map_err(|_| PluginError::new("skills tools lock poisoned"))? = new_tools;
        Ok(())
    }
}

#[async_trait]
impl Plugin for SkillsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder(SKILLS_PLUGIN_ID, env!("CARGO_PKG_VERSION"))
            .description("Discovers SKILL.md files and slash commands, then registers them as dynamic plugin tools.")
            .hooks(HookSubscription::INIT | HookSubscription::TOOL_INVOKE)
            .plugin_capability(HostCapability::ToolRegistry)
            .config_schema(crate::tool::definition::empty_config_schema())
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("skills host lock poisoned"))? = Some(host);
        self.sync_tools(&ctx).await?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let workflow_input = Self::parse_skill_input(input.input.clone())?;
        let discovered_tool = self
            .tools
            .read()
            .map_err(|_| PluginError::new("skills tools lock poisoned"))?
            .get(input.tool_name.as_str())
            .cloned()
            .ok_or_else(|| {
                PluginError::invalid_params(format!("unknown skills tool '{}'", input.tool_name))
            })?;
        let prompt = Self::render_prompt(
            discovered_tool.skill.body.as_str(),
            workflow_input.args.as_deref().unwrap_or_default(),
        );
        let kind = match discovered_tool.kind {
            DiscoveredToolKind::Skill => "skill",
            DiscoveredToolKind::Command => "command",
        };
        Ok(ToolInvokeOutput::text(prompt)
            .with_title(discovered_tool.skill.frontmatter.name.clone())
            .with_metadata("workflow", discovered_tool.skill.frontmatter.name)
            .with_metadata("skill_tool_kind", kind))
    }
}
