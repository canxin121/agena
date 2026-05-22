//! `agena.skills` — discovery plugin that scans the standard skill
//! roots (workspace `.agena/skills/` and user `~/.agena/skills/`) plus
//! user slash-command markdown, and registers everything it finds as dynamic
//! plugin tools.
//!
//! Packaged skills from `agena-skills` are also projected here so a fresh
//! install has workflow-like entries before any user-defined content exists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use agena_skills::discovery::{default_command_roots, default_roots, scan, scan_commands};
use agena_skills::skill::Skill;
use async_trait::async_trait;

use crate::message::WorkflowPromptToolInput;
use crate::plugin::PluginError;
use crate::plugin::sdk::host_api::{HostClient, HostEntryRegisterRequest, HostEntryRemoveRequest};
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
enum DiscoveredEntryKind {
    Skill,
    Command,
}

#[derive(Clone)]
struct DiscoveredEntry {
    skill: Skill,
    kind: DiscoveredEntryKind,
    alias: bool,
}

pub(crate) struct SkillsPlugin {
    host: RwLock<Option<Arc<dyn HostClient>>>,
    entries: RwLock<BTreeMap<String, DiscoveredEntry>>,
}

impl SkillsPlugin {
    pub(crate) fn new() -> Self {
        Self {
            host: RwLock::new(None),
            entries: RwLock::new(BTreeMap::new()),
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

    fn entry_decl(name: &str, entry: &DiscoveredEntry) -> PluginToolDecl {
        let category = match entry.kind {
            DiscoveredEntryKind::Skill => "workflow",
            DiscoveredEntryKind::Command => "command",
        };
        let label = if entry.alias { "alias" } else { category };
        let mut tags = vec![
            ToolTag::ReadOnly,
            ToolTag::custom(category).expect("category tags are valid"),
            ToolTag::custom(format!("skill:{}", entry.skill.frontmatter.name))
                .expect("skill identity tags are valid"),
        ];
        if entry.alias {
            tags.push(ToolTag::custom("alias").expect("alias tag is valid"));
        }
        let description = if entry.skill.frontmatter.description.trim().is_empty() {
            format!(
                "Generate the '{}' {label} prompt.",
                entry.skill.frontmatter.name
            )
        } else {
            entry.skill.frontmatter.description.clone()
        };
        PluginToolDecl::new(
            name.to_string(),
            crate::entry::definition::json_schema_for::<SkillToolInput>(),
        )
        .description(description.clone())
        .summary(description)
        .help(entry.skill.body.clone())
        .tags(tags)
        .concurrency_safe(true)
        .deferred_load()
    }

    fn discovered_entries(ctx: &InitContext) -> BTreeMap<String, DiscoveredEntry> {
        let workspace =
            Some(ctx.workspace_root.clone()).filter(|p: &PathBuf| !p.as_os_str().is_empty());
        let roots = default_roots(workspace.as_deref());
        let command_roots = default_command_roots(workspace.as_deref());

        let skills_by_name: BTreeMap<String, Skill> = scan(&roots)
            .unwrap_or_default()
            .into_iter()
            .map(|skill| (skill.frontmatter.name.clone(), skill))
            .collect();

        let mut entries = BTreeMap::new();
        for skill in skills_by_name.into_values() {
            entries.insert(
                skill.frontmatter.name.clone(),
                DiscoveredEntry {
                    skill: skill.clone(),
                    kind: DiscoveredEntryKind::Skill,
                    alias: false,
                },
            );
            for alias in &skill.frontmatter.aliases {
                entries.insert(
                    alias.clone(),
                    DiscoveredEntry {
                        skill: skill.clone(),
                        kind: DiscoveredEntryKind::Skill,
                        alias: true,
                    },
                );
            }
        }
        for command in scan_commands(&command_roots).unwrap_or_default() {
            entries.insert(
                command.frontmatter.name.clone(),
                DiscoveredEntry {
                    skill: command.clone(),
                    kind: DiscoveredEntryKind::Command,
                    alias: false,
                },
            );
            for alias in &command.frontmatter.aliases {
                entries.insert(
                    alias.clone(),
                    DiscoveredEntry {
                        skill: command.clone(),
                        kind: DiscoveredEntryKind::Command,
                        alias: true,
                    },
                );
            }
        }
        entries
    }

    async fn sync_entries(&self, ctx: &InitContext) -> SdkResult<()> {
        let host = self.host()?;
        let new_entries = Self::discovered_entries(ctx);
        let old_names = self
            .entries
            .read()
            .map_err(|_| PluginError::new("skills entries lock poisoned"))?
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let new_names = new_entries.keys().cloned().collect::<BTreeSet<_>>();

        for removed in old_names.difference(&new_names) {
            let _ = host
                .entry_remove(HostEntryRemoveRequest {
                    name: removed.clone(),
                    exposed: false,
                })
                .await?;
        }

        for (name, entry) in &new_entries {
            let _ = host
                .entry_register(HostEntryRegisterRequest {
                    entry: Self::entry_decl(name, entry),
                })
                .await?;
        }

        *self
            .entries
            .write()
            .map_err(|_| PluginError::new("skills entries lock poisoned"))? = new_entries;
        Ok(())
    }
}

#[async_trait]
impl Plugin for SkillsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-skills", env!("CARGO_PKG_VERSION"))
            .description("Discovers SKILL.md files and slash commands, then registers them as dynamic plugin tools.")
            .hooks(HookSubscription::INIT | HookSubscription::TOOL_INVOKE)
            .plugin_capability(HostCapability::EntryRegistry)
            .build()
    }

    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        *self
            .host
            .write()
            .map_err(|_| PluginError::new("skills host lock poisoned"))? = Some(host);
        self.sync_entries(&ctx).await?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        let workflow_input = Self::parse_skill_input(input.input.clone())?;
        let entry = self
            .entries
            .read()
            .map_err(|_| PluginError::new("skills entries lock poisoned"))?
            .get(input.tool_name.as_str())
            .cloned()
            .ok_or_else(|| {
                PluginError::invalid_params(format!("unknown skills entry '{}'", input.tool_name))
            })?;
        let prompt = Self::render_prompt(
            entry.skill.body.as_str(),
            workflow_input.args.as_deref().unwrap_or_default(),
        );
        let kind = match entry.kind {
            DiscoveredEntryKind::Skill => "skill",
            DiscoveredEntryKind::Command => "command",
        };
        Ok(ToolInvokeOutput::text(prompt)
            .with_title(entry.skill.frontmatter.name.clone())
            .with_metadata("workflow", entry.skill.frontmatter.name)
            .with_metadata("skill_entry_kind", kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_input_accepts_action_shape() {
        let input = SkillsPlugin::parse_skill_input(serde_json::json!({
            "action": "run",
            "args": "inspect auth flow"
        }))
        .expect("action-shaped skill input should parse");

        assert_eq!(input.args.as_deref(), Some("inspect auth flow"));
    }
    #[test]
    fn skill_input_schema_uses_action_discriminator() {
        let schema = crate::entry::definition::json_schema_for::<SkillToolInput>();
        let rendered = serde_json::to_string(&schema).expect("skill schema should serialize");
        assert!(rendered.contains("\"action\""));
        assert!(rendered.contains("\"run\""));
    }
}
