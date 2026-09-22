//! Resolve every change against the original file. Matching is exact and
//! line-oriented; ambiguous matches fail before the transaction commits.
use super::{Hunk, MAX_PATCH_FILE_BYTES, ToolError};
use similar::{ChangeTag, TextDiff};

fn body(line: &str) -> &str {
    line.strip_suffix('\n')
        .map(|s| s.strip_suffix('\r').unwrap_or(s))
        .unwrap_or(line)
}
fn ending(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}
fn unique(
    mut values: impl Iterator<Item = usize>,
    label: &str,
    path: &str,
) -> Result<usize, ToolError> {
    let first = values.next().ok_or_else(|| {
        ToolError::invalid_patch(format!("failed to locate {label} in target file: {path}"))
    })?;
    if values.next().is_some() {
        return Err(ToolError::invalid_patch(format!(
            "ambiguous {label} in {path}; include more exact context"
        )));
    }
    Ok(first)
}

pub(super) fn apply_hunks(path: &str, original: &str, hunks: &[Hunk]) -> Result<String, ToolError> {
    let lines: Vec<_> = original.split_inclusive('\n').collect();
    let mut offsets = Vec::with_capacity(lines.len() + 1);
    offsets.push(0);
    for line in &lines {
        offsets.push(offsets.last().copied().unwrap() + line.len());
    }
    let default_eol = lines
        .iter()
        .map(|line| ending(line))
        .find(|eol| !eol.is_empty())
        .unwrap_or("\n");
    let mut cursor = 0;
    let mut copied = 0;
    let mut output = String::new();
    for (index, hunk) in hunks.iter().enumerate() {
        for anchor in &hunk.contexts {
            cursor = unique(
                (cursor..lines.len()).filter(|&i| body(lines[i]) == anchor),
                "context anchor",
                path,
            )? + 1;
        }
        let old: Vec<_> = hunk.old.lines().collect();
        let new: Vec<_> = hunk.new.lines().collect();
        let label = format!("update hunk {}", index + 1);
        let start = if old.is_empty() {
            if hunk.contexts.is_empty() {
                lines.len()
            } else {
                cursor
            }
        } else {
            unique(
                (cursor..=lines.len()).filter(|&start| {
                    let end = start.saturating_add(old.len());
                    end <= lines.len()
                        && (!hunk.end_of_file || end == lines.len())
                        && lines[start..end]
                            .iter()
                            .zip(&old)
                            .all(|(actual, expected)| body(actual) == *expected)
                }),
                &label,
                path,
            )?
        };
        let end = start + old.len();
        if hunk.end_of_file && end != lines.len() {
            return Err(ToolError::invalid_patch(
                "EOF hunk does not reach the end of the file",
            ));
        }
        if (hunk.old_no_newline && (end != lines.len() || original.ends_with('\n')))
            || (hunk.new_no_newline && end != lines.len())
        {
            return Err(ToolError::invalid_patch(
                "no-newline marker does not describe a final file line",
            ));
        }
        let local_eol = lines[start..end]
            .iter()
            .map(|line| ending(line))
            .find(|s| !s.is_empty())
            .unwrap_or(default_eol);
        let replacement_budget = new.iter().try_fold(0usize, |total, line| {
            total.checked_add(line.len() + local_eol.len())
        });
        if replacement_budget.is_none_or(|bytes| bytes as u64 > MAX_PATCH_FILE_BYTES) {
            return Err(ToolError::invalid_patch(
                "updated file exceeds the 16 MiB limit",
            ));
        }
        let mut replacement: Vec<String> = new
            .iter()
            .map(|line| format!("{line}{local_eol}"))
            .collect();
        // Equal/context lines retain their original line endings, including
        // mixed LF/CRLF files. Only genuinely inserted lines choose an ending.
        for change in TextDiff::from_lines(&hunk.old, &hunk.new).iter_all_changes() {
            if change.tag() == ChangeTag::Equal
                && let (Some(old_index), Some(new_index)) = (change.old_index(), change.new_index())
            {
                replacement[new_index] = lines[start + old_index].to_owned();
            }
        }
        let no_final_newline = hunk.new_no_newline
            || (!hunk.old_no_newline && !original.is_empty() && !original.ends_with('\n'));
        let replacement_len = replacement.len();
        for (i, line) in replacement.iter_mut().enumerate() {
            if end == lines.len() && i + 1 == replacement_len && no_final_newline {
                let len = body(line).len();
                line.truncate(len);
            } else if !line.ends_with('\n') {
                line.push_str(local_eol);
            }
        }
        output.push_str(&original[copied..offsets[start]]);
        if old.is_empty()
            && start == lines.len()
            && !original.is_empty()
            && !original.ends_with('\n')
            && !replacement.is_empty()
        {
            output.push_str(local_eol);
        }
        for line in replacement {
            output.push_str(&line);
        }
        copied = offsets[end];
        cursor = end;
        if output.len().saturating_add(original.len() - copied) as u64 > MAX_PATCH_FILE_BYTES {
            return Err(ToolError::invalid_patch(
                "updated file exceeds the 16 MiB limit",
            ));
        }
    }
    output.push_str(&original[copied..]);
    Ok(output)
}
