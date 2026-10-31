pub(crate) fn derive_session_title(i18n: &I18n, text: &str) -> String {
    let fallback = ui_text::t(i18n, "composer-session-new");
    let first_line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(fallback.as_str());
    truncate_display_width(first_line, 60)
}

pub(crate) fn draft_title_source(i18n: &I18n, draft: &ComposerDraft) -> Option<String> {
    let mut preview = String::new();
    for node in &draft.document.0 {
        match node {
            agena_domain::ComposerNode::Text { text } => preview.push_str(text),
            agena_domain::ComposerNode::Activity { activity } => preview.push_str(
                &crate::composer_state_impls::composer_activity_presentation(
                    &activity.payload,
                    i18n,
                )
                .1,
            ),
        }
    }

    if preview.trim().is_empty() {
        None
    } else {
        Some(preview)
    }
}

pub(crate) async fn run_status_line_command(
    command: String,
    session_id: Option<String>,
    focus: String,
) -> Option<String> {
    const STATUS_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
    const STATUS_COMMAND_OUTPUT_LIMIT: usize = 64 * 1024;

    let mut cmd = if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.args(["/d", "/s", "/c", command.as_str()]);
        cmd
    } else {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-lc", command.as_str()]);
        cmd
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd.env("AGENA_TUI_FOCUS", focus);
    if let Some(session_id) = session_id {
        cmd.env("AGENA_SESSION_ID", session_id);
    }
    let result = tokio::time::timeout(STATUS_COMMAND_TIMEOUT, async move {
        let mut child = agena_process::spawn(cmd).context("failed to spawn TUI status command")?;
        let mut stdout = child
            .stdout()
            .take()
            .context("TUI status command stdout is unavailable")?;
        let read_output = async move {
            let mut output = Vec::new();
            let mut exceeded = false;
            let mut chunk = [0_u8; 8 * 1024];
            loop {
                let read = stdout
                    .read(&mut chunk)
                    .await
                    .context("failed to read TUI status command stdout")?;
                if read == 0 {
                    break;
                }
                let remaining = STATUS_COMMAND_OUTPUT_LIMIT.saturating_sub(output.len());
                output.extend_from_slice(&chunk[..read.min(remaining)]);
                exceeded |= read > remaining;
            }
            Ok::<_, anyhow::Error>((output, exceeded))
        };
        let (read, status) = tokio::join!(read_output, child.wait());
        let (output, exceeded) = read?;
        if exceeded {
            anyhow::bail!(
                "TUI status command output exceeded the {STATUS_COMMAND_OUTPUT_LIMIT}-byte limit"
            );
        }
        let status = status.context("failed to wait for TUI status command")?;
        if !status.success() {
            anyhow::bail!("TUI status command exited with status {status}");
        }
        let text = String::from_utf8_lossy(&output);
        let line = text.lines().next().unwrap_or_default().trim();
        Ok::<_, anyhow::Error>((!line.is_empty()).then(|| line.to_string()))
    })
    .await;
    match result {
        Ok(Ok(line)) => line,
        Ok(Err(error)) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "TUI status command failed",
                    error.as_ref(),
                ),
                "TUI status line command did not produce a value"
            );
            None
        }
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "TUI status command timed out after 2 seconds",
                    &error,
                ),
                "TUI status line command timed out"
            );
            None
        }
    }
}

pub(crate) fn find_placeholder_occurrence(
    text: &str,
    placeholder: &str,
    occupied: &[Range<usize>],
) -> Option<Range<usize>> {
    if placeholder.is_empty() {
        return None;
    }

    let mut search_start = 0;
    while search_start < text.len() {
        let relative = text.get(search_start..)?.find(placeholder)?;
        let start = search_start + relative;
        let end = start + placeholder.len();
        let candidate = start..end;
        if occupied
            .iter()
            .all(|range| range.end <= candidate.start || range.start >= candidate.end)
        {
            return Some(candidate);
        }
        search_start = next_grapheme_boundary(text, start);
    }
    None
}
use crate::{
    Command, ComposerDraft, Duration, I18n, Range, Stdio, truncate_display_width, ui_text,
};
use agena_tui_session::session_helpers::next_grapheme_boundary;
use anyhow::Context as _;
use tokio::io::AsyncReadExt;

#[cfg(test)]
mod session_title_tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn generated_session_titles_use_display_columns_and_whole_graphemes() {
        let i18n = I18n::english();
        let prompt = "👩‍💻".repeat(31);
        let title = derive_session_title(&i18n, &prompt);
        assert_eq!(title, "👩‍💻".repeat(30));
        assert_eq!(title.width(), 60);
        assert_eq!(
            derive_session_title(&i18n, &"界".repeat(31)),
            "界".repeat(30)
        );
        assert_eq!(truncate_display_width("界", 1), "");
        assert_eq!(truncate_display_width("e\u{301}x", 1), "e\u{301}");
        assert_eq!(prompt, "👩‍💻".repeat(31));
    }
}
