//! `agena-skills` — discovery + loading of reusable LLM workflows
//! ("skills") packaged as `SKILL.md` files with a YAML frontmatter
//! header.
//!
//! Discovery roots, in priority order:
//!
//! 1. `<workspace>/.agena/skills/`
//! 2. `~/.agena/skills/`
//! 3. `~/.claude/skills/` (claude-code-compatible)
//! 4. Built-in bundled skills (compiled into the binary)
//!
//! A skill is a directory with a `SKILL.md` that looks like:
//!
//! ```markdown
//! ---
//! name: review
//! description: Review the current branch
//! allowed_tools: ["Read", "Glob", "Grep", "Bash(git *)"]
//! model: claude-sonnet-4-6
//! ---
//! Body of the skill (becomes the system message injection).
//! ```

pub mod bundled;
pub mod discovery;
pub mod error;
pub mod manager;
pub mod skill;

pub use error::{SkillError, SkillResult};
pub use manager::SkillsManager;
pub use skill::{Skill, SkillFrontmatter};
