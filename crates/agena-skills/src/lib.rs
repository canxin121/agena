//! `agena-skills` — parser + discovery helpers for reusable LLM workflows
//! ("skills") packaged as markdown files with YAML frontmatter.
//!
//! This crate is plumbing only: it parses bundled or on-disk workflow content
//! and exposes discovery helpers. Runtime registration into the shared plugin
//! tool registry lives in `agena` inside the bundled `SkillsPlugin`.
//!
//! Discovery helpers scan explicit roots supplied by callers. Runtime defaults
//! do not include implicit workspace or user-global directories.

pub mod bundled;
pub mod discovery;
pub mod error;
pub mod skill;

pub use error::{SkillError, SkillResult};
pub use skill::{Skill, SkillFrontmatter};
