pub(crate) fn composer_draft_with_text_prefix_stripped(
    mut draft: ComposerDraft,
    count: usize,
) -> ComposerDraft {
    let Some(agena_domain::ComposerNode::Text { text }) = draft.document.0.first_mut() else {
        return draft;
    };
    let mut boundary = 0;
    let mut chars = text.char_indices();
    for _ in 0..count {
        let Some((index, ch)) = chars.next() else {
            return draft;
        };
        boundary = index + ch.len_utf8();
    }
    text.drain(..boundary);
    draft
}

pub(crate) fn default_draft_store_path(scope: &str) -> PathBuf {
    agena_storage_layout::StorageLayout::current()
        .client_scope_dir(scope)
        .join("tui-drafts.json")
}

pub(crate) fn default_prompt_history_path(scope: &str) -> PathBuf {
    agena_storage_layout::StorageLayout::current()
        .client_scope_dir(scope)
        .join("tui-prompt-history.jsonl")
}

pub(crate) fn non_empty_owned(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn permission_mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Allow => "allow",
        PermissionMode::Auto => "auto",
        PermissionMode::Ask => "ask",
        PermissionMode::Deny => "deny",
    }
}
use crate::{ComposerDraft, PathBuf, PermissionMode};
