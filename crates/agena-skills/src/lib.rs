//! `agena-skills` — parser + discovery helpers for reusable LLM workflows
//! ("skills") packaged as markdown files with YAML frontmatter.
//!
//! This crate is plumbing only: it parses bundled or on-disk workflow content
//! and exposes discovery helpers. Runtime registration into the shared plugin
//! tool registry lives in `agena` inside the bundled `SkillsPlugin`.
//!
//! Discovery roots, in priority order:
//!
//! 1. `<workspace>/.agena/skills/`
//! 2. `~/.agena/skills/`
//!
//! User slash-command markdown is discovered from
//! `<workspace>/.agena/commands/*.md` and `~/.agena/commands/*.md` with the
//! same frontmatter format.

pub mod bundled;
pub mod discovery;
pub mod error;
pub mod skill;

pub use error::{SkillError, SkillResult};
pub use skill::{Skill, SkillFrontmatter};
