pub(crate) fn persistent_draft_store_version() -> u32 {
    // Version 4 is the ordered Text/Activity document. There is deliberately
    // no decoder for the former text/items/elements draft shape.
    4
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum OverlayCommit {
    TranscriptSearch,
}

mod persistence;
