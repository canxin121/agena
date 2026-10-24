//! Transcript indentation belongs to presentation, not the message/part tree.
//! Roles and part headlines share the left edge. Primary content has one
//! small inset; only actual nested details add another level.

pub(crate) const BODY_PREFIX: &str = "  ";
pub(crate) const DETAIL_PREFIX: &str = "    ";
pub(crate) const NESTED_PREFIX: &str = "      ";
pub(crate) const CONTENT_PREFIX: &str = "  │ ";
pub(crate) const CONTENT_GUTTER: u16 = 4;
