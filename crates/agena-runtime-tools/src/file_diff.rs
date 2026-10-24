//! Diff previews captured from the exact bytes involved in a file mutation.

use serde::Serialize;
use similar::{ChangeTag, TextDiff};
use std::io::{self, Write};
use std::time::Duration;

const MAX_DIFF_BYTES: usize = 128 * 1024;

#[derive(Debug, Serialize)]
pub struct FileDiffPreview {
    pub diff: String,
    pub diff_truncated: bool,
    pub additions: usize,
    pub deletions: usize,
}

/// Produce a standard unified diff with three context lines. The edit itself
/// is never truncated; only the presentation is bounded. Counts cover the
/// complete change even when the preview is too large to show in a transcript.
pub fn file_diff_preview(path: &str, before: Option<&str>, after: Option<&str>) -> FileDiffPreview {
    let changes = TextDiff::configure()
        .timeout(Duration::from_millis(200))
        .diff_lines(before.unwrap_or_default(), after.unwrap_or_default());
    let mut additions = 0;
    let mut deletions = 0;
    for change in changes.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => additions += 1,
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Equal => {}
        }
    }
    let old_path = before.map_or_else(|| "/dev/null".into(), |_| format!("a/{path}"));
    let new_path = after.map_or_else(|| "/dev/null".into(), |_| format!("b/{path}"));
    let mut output = PreviewWriter(Vec::new());
    let truncated = changes
        .unified_diff()
        .context_radius(3)
        .header(&old_path, &new_path)
        .to_writer(&mut output)
        .is_err();
    if output.0.is_empty() && before.is_some() != after.is_some() {
        // Even an empty file has a creation/deletion to show.
        output.0 = format!("--- {old_path}\n+++ {new_path}\n").into_bytes();
    }
    if truncated {
        // End at a complete line/UTF-8 boundary, never in the middle of source.
        let end = output
            .0
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |i| i + 1);
        output.0.truncate(end);
    }
    FileDiffPreview {
        diff: String::from_utf8(output.0).expect("unified diff is UTF-8"),
        diff_truncated: truncated,
        additions,
        deletions,
    }
}

struct PreviewWriter(Vec<u8>);

impl Write for PreviewWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let available = MAX_DIFF_BYTES.saturating_sub(self.0.len());
        if available == 0 {
            return Err(io::Error::other("diff preview limit"));
        }
        let count = bytes.len().min(available);
        self.0.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
