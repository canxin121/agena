use std::sync::RwLock;

use crate::error::{SkillError, SkillResult};
use crate::skill::Skill;

/// Aggregating registry of skills owned by one or more plugins.
///
/// `SkillsManager` no longer scans the filesystem on its own — that
/// responsibility now lives in the bundled `SkillsFsPlugin` (and any
/// third-party plugin that wants to discover skills its own way). The
/// registry is populated through [`SkillsManager::register`] and read
/// back via [`SkillsManager::list`] / [`SkillsManager::get`] (and the
/// command equivalents).
pub struct SkillsManager {
    inner: RwLock<Vec<OwnedSkill>>,
    commands: RwLock<Vec<OwnedSkill>>,
}

#[derive(Debug, Clone)]
struct OwnedSkill {
    plugin_id: String,
    skill: Skill,
}

impl Default for SkillsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillsManager {
    /// Build an empty registry. Use [`SkillsManager::register`] to add
    /// skills (typically from a plugin's `init` hook).
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Vec::new()),
            commands: RwLock::new(Vec::new()),
        }
    }

    /// Register a skill under the given plugin id. Replaces any prior
    /// registration of the same name owned by the same plugin.
    pub fn register(&self, plugin_id: impl Into<String>, skill: Skill) {
        let plugin_id = plugin_id.into();
        let mut g = self.inner.write().unwrap();
        if let Some(slot) = g.iter_mut().find(|owned| {
            owned.plugin_id == plugin_id && owned.skill.frontmatter.name == skill.frontmatter.name
        }) {
            slot.skill = skill;
        } else {
            g.push(OwnedSkill { plugin_id, skill });
        }
    }

    /// Register a slash-command-style skill.
    pub fn register_command(&self, plugin_id: impl Into<String>, skill: Skill) {
        let plugin_id = plugin_id.into();
        let mut g = self.commands.write().unwrap();
        if let Some(slot) = g.iter_mut().find(|owned| {
            owned.plugin_id == plugin_id && owned.skill.frontmatter.name == skill.frontmatter.name
        }) {
            slot.skill = skill;
        } else {
            g.push(OwnedSkill { plugin_id, skill });
        }
    }

    /// Remove a skill owned by this plugin. Returns `true` if a record
    /// was removed.
    pub fn remove(&self, plugin_id: &str, name: &str) -> bool {
        let mut g = self.inner.write().unwrap();
        let before = g.len();
        g.retain(|owned| !(owned.plugin_id == plugin_id && owned.skill.frontmatter.name == name));
        g.len() != before
    }

    /// Remove a slash command owned by this plugin.
    pub fn remove_command(&self, plugin_id: &str, name: &str) -> bool {
        let mut g = self.commands.write().unwrap();
        let before = g.len();
        g.retain(|owned| !(owned.plugin_id == plugin_id && owned.skill.frontmatter.name == name));
        g.len() != before
    }

    /// Drop every skill owned by `plugin_id` (both regular and command).
    pub fn remove_owned_by(&self, plugin_id: &str) {
        if let Ok(mut g) = self.inner.write() {
            g.retain(|owned| owned.plugin_id != plugin_id);
        }
        if let Ok(mut g) = self.commands.write() {
            g.retain(|owned| owned.plugin_id != plugin_id);
        }
    }

    pub fn list(&self) -> Vec<Skill> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .map(|owned| owned.skill.clone())
            .collect()
    }

    pub fn list_with_owners(&self) -> Vec<(String, Skill)> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .map(|owned| (owned.plugin_id.clone(), owned.skill.clone()))
            .collect()
    }

    pub fn list_commands(&self) -> Vec<Skill> {
        self.commands
            .read()
            .unwrap()
            .iter()
            .map(|owned| owned.skill.clone())
            .collect()
    }

    /// Resolve a skill by name or alias.
    pub fn get(&self, name: &str) -> SkillResult<Skill> {
        self.inner
            .read()
            .unwrap()
            .iter()
            .find(|owned| owned.skill.matches(name))
            .map(|owned| owned.skill.clone())
            .ok_or_else(|| SkillError::NotFound(name.to_string()))
    }

    pub fn get_command(&self, name: &str) -> SkillResult<Skill> {
        self.commands
            .read()
            .unwrap()
            .iter()
            .find(|owned| owned.skill.matches(name))
            .map(|owned| owned.skill.clone())
            .ok_or_else(|| SkillError::NotFound(name.to_string()))
    }
}
