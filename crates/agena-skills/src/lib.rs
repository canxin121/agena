//! Agent Skills documents and packages shared by interactive and agent clients.
//!
//! This crate performs bounded local I/O. Callers own authorization and network
//! access, and must authorize both the source and destination before importing.

mod error;
mod import;
mod manager;
mod metadata;
pub mod source;
mod types;

pub use error::{Result, SkillError, SkillErrorKind};
pub use import::{SkillImport, SkillImportFile};
pub use manager::{SkillManager, default_roots, package_enabled, package_enabled_override};
pub use metadata::{ParsedSkill, parse_document, validate_document, validate_name};
pub use types::*;

pub const MAX_DOCUMENT_BYTES: usize = 1_048_576;
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PACKAGE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_PACKAGE_FILES: usize = 10_000;
pub const MAX_PACKAGE_DEPTH: usize = 32;
pub(crate) const STATE_FILE: &str = ".agena-skill.json";
