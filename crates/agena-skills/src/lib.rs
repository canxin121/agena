//! `agena-skills` — parser + discovery helpers for reusable LLM workflows
//! ("skills") packaged as markdown files with YAML frontmatter.
//!
//! This crate is plumbing only: it parses bundled or on-disk workflow content
//! and exposes discovery helpers. Runtime registration into the shared plugin
//! tool registry lives in `agena` inside the bundled `SkillsPlugin`.
//!
//! Discovery supports the standard Agena and cross-agent compatibility roots,
//! while retaining provenance and diagnostics for callers that need to explain
//! why a skill was or was not loaded.

pub mod bundled;
pub mod discovery;
pub mod error;
pub mod skill;

pub use discovery::{DiscoveryDiagnostic, DiscoveryReport};
pub use error::{SkillError, SkillResult};
pub use skill::{Skill, SkillFrontmatter};
