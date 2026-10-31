//! Shared attachment facts owned by the domain contract.
//!
//! The plugin SDK and runtime contracts re-export these same types so tools,
//! providers and renderers pass attachments without conversion or Host dependencies.

pub use agena_domain::{AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource};
