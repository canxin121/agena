use std::path::Path;
use std::sync::RwLock;

use crate::bundled;
use crate::discovery::{default_roots, scan};
use crate::error::{SkillError, SkillResult};
use crate::skill::Skill;

/// Cached pool of discovered + bundled skills.
pub struct SkillsManager {
    inner: RwLock<Vec<Skill>>,
}

impl SkillsManager {
    /// Build a manager.  Discovery is performed eagerly so callers see a
    /// stable view of available skills.  Workspace path is optional —
    /// global/bundled skills still load without one.
    pub fn build(workspace: Option<&Path>) -> SkillResult<Self> {
        let roots = default_roots(workspace);
        let mut all = scan(&roots)?;
        // Bundled last so user-defined skills with the same name win.
        for b in bundled::all()? {
            if !all.iter().any(|s| s.frontmatter.name == b.frontmatter.name) {
                all.push(b);
            }
        }
        Ok(Self {
            inner: RwLock::new(all),
        })
    }

    pub fn list(&self) -> Vec<Skill> {
        self.inner.read().unwrap().clone()
    }

    /// Resolve a skill by name or alias.
    pub fn get(&self, name: &str) -> SkillResult<Skill> {
        let g = self.inner.read().unwrap();
        g.iter()
            .find(|s| s.matches(name))
            .cloned()
            .ok_or_else(|| SkillError::NotFound(name.to_string()))
    }

    /// Replace the cache by re-scanning the discovery roots.
    pub fn reload(&self, workspace: Option<&Path>) -> SkillResult<()> {
        let roots = default_roots(workspace);
        let mut next = scan(&roots)?;
        for b in bundled::all()? {
            if !next.iter().any(|s| s.frontmatter.name == b.frontmatter.name) {
                next.push(b);
            }
        }
        *self.inner.write().unwrap() = next;
        Ok(())
    }
}
