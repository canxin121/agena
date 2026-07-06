//! `agena.skills` — discovery plugin for explicit skill roots. Runtime defaults
//! do not scan implicit workspace or user-global directories.
//!
//! Packaged skills from `agena-skills` are also projected here so a fresh
//! install has workflow-like tools before any user-defined content exists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use agena_macros::ToolInputShape;
use agena_skills::discovery::{default_command_roots, default_roots, scan, scan_commands};
use agena_skills::skill::Skill;

use crate::message::WorkflowPromptToolInput;
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostToolRegisterRequest, HostToolRemoveRequest};
use crate::plugin::sdk::{
    HookSubscription, HostCapability, InitContext, InitOutcome, PluginManifest,
    Result as SdkResult, ToolDefinition, ToolInvokeContext, ToolInvokeInput, ToolInvokeOutput,
    ToolTag,
};

pub(crate) const SKILLS_PLUGIN_ID: &str = "agena.skills";

#[derive(
    Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema, ToolInputShape,
)]
#[tool_input(handler_receiver = SkillsPlugin)]
#[serde(tag = "action", rename_all = "snake_case")]
enum SkillToolInput {
    #[tool(
        default_when_empty = true,
        infer_when_present("args"),
        handle_with_context = SkillsPlugin::dispatch_run,
        handle_by_value = true
    )]
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

    async fn dispatch_run(
        &self,
        context: &ToolInvokeContext<'_>,
        args: WorkflowPromptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let discovered_tool = self
            .tools
            .read()
            .map_err(|_| PluginError::new("skills tools lock poisoned"))?
            .get(context.tool_name)
            .cloned()
            .ok_or_else(|| {
                PluginError::invalid_params(format!("unknown skills tool '{}'", context.tool_name))
            })?;
        let prompt = Self::render_prompt(
            discovered_tool.skill.body.as_str(),
            args.args.as_deref().unwrap_or_default(),
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

    fn tool_definition(name: &str, discovered_tool: &DiscoveredTool) -> ToolDefinition {
        let category = match discovered_tool.kind {
            DiscoveredToolKind::Skill => "workflow",
            DiscoveredToolKind::Command => "command",
        };
        let tags = vec![
            ToolTag::ReadOnly,
            ToolTag::custom(category).expect("category tags are valid"),
            ToolTag::custom(format!("skill:{}", discovered_tool.skill.frontmatter.name))
                .expect("skill identity tags are valid"),
        ];
        let description = if discovered_tool
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
        };
        ToolDefinition::new(name.to_string(), SkillToolInput::input_schema())
            .description(description.clone())
            .summary(description)
            .help(discovered_tool.skill.body.clone())
            .compact()
            .tags(tags)
            .concurrency_safe(true)
    }

    fn discovered_tools(ctx: &InitContext) -> BTreeMap<String, DiscoveredTool> {
        let workspace =
            Some(ctx.workspace_root.clone()).filter(|p: &PathBuf| !p.as_os_str().is_empty());
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
                    by_model_name: false,
                })
                .await?;
        }

        for (name, discovered_tool) in &new_tools {
            let _ = host
                .register_tool(HostToolRegisterRequest {
                    tool: Self::tool_definition(name, discovered_tool),
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

#[crate::plugin::sdk::async_trait]
impl crate::plugin::sdk::Plugin for SkillsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena", "skills", env!("CARGO_PKG_VERSION"))
            .description(
                "Discovers SKILL.md files and slash commands, then registers them as dynamic plugin tools.",
            )
            .hooks(HookSubscription::TOOL_INVOKE)
            .brief()
            .plugin_capabilities([HostCapability::ToolRegistry])
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
        let context = input.context();
        let parsed = SkillToolInput::parse_input(input.input.clone())?;
        parsed
            .dispatch_tool_invoke_with_context(self, &context)
            .await
    }
}
