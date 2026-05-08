//! `agena.skills_fs` — discovery plugin that scans the standard skill
//! roots (workspace `.agena/skills/`, user `~/.agena/skills/`,
//! `~/.claude/skills/`) plus user slash-command markdown, and writes
//! everything it finds into the shared [`SkillsManager`] registry.
//! Bundled skills shipped with `agena-skills` are also registered here
//! so a fresh install has something usable out of the box.
//!
//! The plugin exposes no tool entries — its only job is to populate the
//! shared registry that the `agena.skills` plugin's `skill_run` entry
//! later queries via `host.skill.get`.
//!
//! Implementation note: discovery happens in-process and writes
//! directly to the [`SkillsManager`] held by the runtime, rather than
//! going through `host.skill.register`. This keeps the plugin
//! independent of host RPC bootstrapping order — the manager is
//! plumbed in at construction time.

use std::path::PathBuf;
use std::sync::Arc;

use agena_skills::SkillsManager;
use agena_skills::bundled as bundled_skills;
use agena_skills::discovery::{default_command_roots, default_roots, scan, scan_commands};
use agena_skills::skill::Skill;
use async_trait::async_trait;

use crate::plugin::sdk::host_api::HostClient;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, Plugin, PluginManifest, Result as SdkResult,
};

pub(crate) const SKILLS_FS_PLUGIN_ID: &str = "agena.skills_fs";

pub(crate) struct SkillsFsPlugin {
    manager: Arc<SkillsManager>,
}

impl SkillsFsPlugin {
    pub(crate) fn new(manager: Arc<SkillsManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Plugin for SkillsFsPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-skills-fs", env!("CARGO_PKG_VERSION"))
            .description(
                "Discovers SKILL.md files on disk and registers them with the shared skill registry.",
            )
            .hooks(HookSubscription::INIT)
            .build()
    }

    async fn init(&self, ctx: InitContext, _host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let workspace =
            Some(ctx.workspace_root.clone()).filter(|p: &PathBuf| !p.as_os_str().is_empty());
        let roots = default_roots(workspace.as_deref());
        let command_roots = default_command_roots(workspace.as_deref());

        let mut discovered: Vec<Skill> = scan(&roots).unwrap_or_default();
        // Bundled last so user-defined skills with the same name win.
        if let Ok(builtins) = bundled_skills::all() {
            for b in builtins {
                if !discovered
                    .iter()
                    .any(|s| s.frontmatter.name == b.frontmatter.name)
                {
                    discovered.push(b);
                }
            }
        }
        for skill in discovered {
            self.manager.register(SKILLS_FS_PLUGIN_ID, skill);
        }

        for command in scan_commands(&command_roots).unwrap_or_default() {
            self.manager.register_command(SKILLS_FS_PLUGIN_ID, command);
        }

        Ok(InitOutcome::ack(self.manifest()))
    }
}
