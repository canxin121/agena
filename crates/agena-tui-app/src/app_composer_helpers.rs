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

pub(crate) fn default_draft_store_path() -> PathBuf {
    let mut base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|error| {
            tracing::error!(
                diagnostic = %error,
                "TUI draft-store home is unavailable; using the current-directory compatibility path"
            );
            PathBuf::from(".")
        });
    base.push("agena");
    base.push("tui-drafts.json");
    base
}

pub(crate) fn default_prompt_history_path() -> PathBuf {
    let mut base = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|error| {
            tracing::error!(
                diagnostic = %error,
                "TUI prompt-history home is unavailable; using the current-directory compatibility path"
            );
            PathBuf::from(".")
        });
    base.push("agena");
    base.push("tui-prompt-history.jsonl");
    base
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
use crate::{ComposerDraft, PathBuf, PermissionMode, env};
