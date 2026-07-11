pub(in crate::app) fn composer_draft_with_text_prefix_stripped(
    mut draft: ComposerDraft,
    count: usize,
) -> ComposerDraft {
    let mut boundary = 0;
    let mut chars = draft.text.char_indices();
    for _ in 0..count {
        let Some((index, ch)) = chars.next() else {
            return draft;
        };
        boundary = index + ch.len_utf8();
    }
    draft.text.drain(..boundary);
    draft
}

pub(in crate::app) fn default_draft_store_path() -> PathBuf {
    let mut base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.push("agena");
    base.push("tui-drafts.json");
    base
}

pub(in crate::app) fn default_prompt_history_path() -> PathBuf {
    let mut base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.push("agena");
    base.push("tui-prompt-history.jsonl");
    base
}

pub(in crate::app) fn non_empty_owned(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(in crate::app) fn permission_mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Allow => "allow",
        PermissionMode::Ask => "ask",
        PermissionMode::Deny => "deny",
    }
}
use crate::app::{ComposerDraft, PathBuf, PermissionMode, env};
