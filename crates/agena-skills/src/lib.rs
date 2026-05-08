//! `agena-skills` — discovery + loading of reusable LLM workflows
//! ("skills") packaged as `SKILL.md` files with a YAML frontmatter
//! header.
//!
//! This crate is plumbing only: it parses skills off disk and exposes
//! a [`SkillsManager`] registry. The actual integration with the plugin
//! host (filesystem scanning, manifest-declared skills, etc.) lives
//! inside `agena` as the bundled `SkillsFsPlugin`.
//!
//! Discovery roots, in priority order:
//!
//! 1. `<workspace>/.agena/skills/`
//! 2. `~/.agena/skills/`
//! 3. `~/.claude/skills/` (claude-code-compatible)
//! 4. Built-in bundled skills (compiled into the binary)
//!
//! User slash commands are discovered from `<workspace>/.agena/commands/*.md`
//! and `~/.agena/commands/*.md` with the same frontmatter format.

pub mod bundled;
pub mod discovery;
pub mod error;
pub mod manager;
pub mod skill;

pub use error::{SkillError, SkillResult};
pub use manager::SkillsManager;
pub use skill::{Skill, SkillFrontmatter};
