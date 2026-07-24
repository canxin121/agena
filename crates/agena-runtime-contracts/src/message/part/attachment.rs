//! Re-export of attachment types from the plugin SDK.
//!
//! These types are the single source of truth for attachments across core and
//! plugins; they live in `agena-plugin-sdk::attachment` so plugins can produce
//! them and the host can pass them through verbatim.

pub use agena_plugin_host::sdk::attachment::{
    AttachmentItem, AttachmentKind, AttachmentPart, AttachmentSource,
};
