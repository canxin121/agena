pub(in crate::app) fn persistent_draft_store_version() -> u32 {
    // Version 1 could persist terminal protocol replies after Crossterm
    // decoded them as character keys. The byte-boundary parser fixes future
    // input; rejecting the old schema clears already-corrupted drafts without
    // retaining payload-specific filtering in the composer.
    2
}

#[derive(Debug, Clone, Copy)]
pub(in crate::app) enum OverlayCommit {
    TranscriptSearch,
}

mod persistence;
