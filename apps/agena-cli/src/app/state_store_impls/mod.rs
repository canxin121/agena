pub(in crate::app) fn persistent_draft_store_version() -> u32 {
    1
}

#[derive(Debug, Clone, Copy)]
pub(in crate::app) enum OverlayCommit {
    TranscriptSearch,
}

mod persistence;
mod session;
