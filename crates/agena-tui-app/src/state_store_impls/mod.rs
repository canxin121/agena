pub(crate) fn persistent_draft_store_version() -> u32 {
    // Version 1 could persist terminal protocol replies after Crossterm
    // decoded them as character keys. Version 3 adds immutable Skill message
    // snapshots, so older drafts are intentionally not retained as a
    // compatibility format.
    3
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum OverlayCommit {
    TranscriptSearch,
}

mod persistence;
