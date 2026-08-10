use std::cmp::min;
use std::fs;
use std::io::Read;

use crate::part::ReadToolInput;
use agena_tool::ReadMode;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    file_attachment,
};

const DEFAULT_OFFSET: usize = 1;
const DEFAULT_LIMIT: usize = 2000;
const MAX_LINE_CHARS: usize = 2000;
const AUTO_DETECT_BYTES: usize = 8 * 1024;
const MAX_TEXT_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 20_000;

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &ReadToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let target = executor.resolve_target_path(&input.file_path);

    if !target.exists() {
        return Err(ToolError::invalid_field(
            "file_path",
            agena_failure::FieldIssueKind::NotFound,
            format!("read target does not exist: {}", input.file_path),
        ));
    }

    let offset = parse_offset(input.offset);
    let limit = parse_limit(input.limit);
    let display_path = executor.display_path(&target);

    if target.is_dir() {
        if matches!(input.mode, ReadMode::Attachment) {
            return Err(ToolError::invalid_field(
                "mode",
                agena_failure::FieldIssueKind::Unsupported,
                format!(
                    "read mode=attachment does not support directories: {}",
                    input.file_path
                ),
            ));
        }

        let (preview, page_truncated, count, scan_truncated) =
            read_directory_listing(&target, offset, limit)?;
        let truncated = page_truncated || scan_truncated;
        let output = ToolPayloadOutput::Read {
            preview: Some(preview.clone()),
            truncated,
            loaded_paths: vec![display_path.clone()],
            attachment: None,
        };

        let summary = if scan_truncated {
            format!("at least {count} items · directory scan bounded")
        } else if truncated {
            format!("{count} items · more available")
        } else {
            format!("{count} items")
        };
        let mut view =
            ToolExecutionView::simple(format!("Read {}", display_path), summary, preview);
        view.metadata
            .insert("kind".to_string(), "directory".to_string());
        view.metadata
            .insert("offset".to_string(), offset.to_string());
        view.metadata.insert("limit".to_string(), limit.to_string());
        view.metadata
            .insert("item_count".to_string(), count.to_string());
        view.metadata
            .insert("scan_truncated".to_string(), scan_truncated.to_string());
        view.metadata
            .insert("truncated".to_string(), truncated.to_string());
        return Ok(ToolPayloadExecution::new(output, view));
    }

    if matches!(input.mode, ReadMode::Attachment) {
        return file_attachment::execute_for_read_attachment(executor, input.file_path.as_str());
    }
    if matches!(input.mode, ReadMode::Auto) {
        let prefix = read_prefix(&target, AUTO_DETECT_BYTES)?;
        if file_attachment::should_attach_in_read_auto(&target, &prefix) {
            return file_attachment::execute_for_read_attachment(
                executor,
                input.file_path.as_str(),
            );
        }
    }

    let content = read_text_file_bounded(&target, MAX_TEXT_FILE_BYTES)?;

    let text = String::from_utf8(content).map_err(|_| {
        ToolError::invalid_field("mode", agena_failure::FieldIssueKind::Unsupported, format!(
            "read tool currently supports UTF-8 text files only; use mode=attachment or mode=auto for binary files: {}",
            input.file_path
        ))
    })?;

    let (preview, truncated, rendered_lines, total_lines) =
        render_file_preview(&text, offset, limit)?;
    let output = ToolPayloadOutput::Read {
        preview: Some(preview.clone()),
        truncated,
        loaded_paths: vec![display_path.clone()],
        attachment: None,
    };

    let summary = if truncated {
        format!("{rendered_lines} of {total_lines} lines")
    } else {
        format!("{total_lines} lines")
    };
    let mut view = ToolExecutionView::simple(format!("Read {}", display_path), summary, preview);
    view.metadata.insert("kind".to_string(), "file".to_string());
    view.metadata
        .insert("offset".to_string(), offset.to_string());
    view.metadata.insert("limit".to_string(), limit.to_string());
    view.metadata
        .insert("rendered_lines".to_string(), rendered_lines.to_string());
    view.metadata
        .insert("total_lines".to_string(), total_lines.to_string());
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());

    Ok(ToolPayloadExecution::new(output, view))
}

fn read_prefix(path: &std::path::Path, limit: usize) -> Result<Vec<u8>, ToolError> {
    let mut bytes = Vec::with_capacity(limit);
    fs::File::open(path)?
        .take(limit as u64)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn read_text_file_bounded(path: &std::path::Path, max_bytes: usize) -> Result<Vec<u8>, ToolError> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(ToolError::invalid_field(
            "file_path",
            agena_failure::FieldIssueKind::OutOfRange,
            format!(
                "text read is limited to {max_bytes} bytes to keep tool execution bounded: {}",
                path.display()
            ),
        ));
    }
    Ok(bytes)
}

fn parse_offset(value: Option<u32>) -> usize {
    value
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_OFFSET)
}

fn parse_limit(value: Option<u32>) -> usize {
    value
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_LIMIT)
}

fn read_directory_listing(
    dir: &std::path::Path,
    offset: usize,
    limit: usize,
) -> Result<(String, bool, usize, bool), ToolError> {
    let mut entries = Vec::new();
    let mut scan_truncated = false;
    for (entry_index, entry) in fs::read_dir(dir)?.enumerate() {
        if entry_index >= MAX_DIRECTORY_ENTRIES {
            scan_truncated = true;
            break;
        }
        let entry = entry?;
        let metadata = entry.metadata()?;
        let mut name = entry.file_name().to_string_lossy().to_string();
        if metadata.is_dir() {
            name.push('/');
        }
        entries.push(name);
    }

    entries.sort();

    if entries.is_empty() {
        return Ok((String::new(), false, 0, scan_truncated));
    }

    if offset > entries.len() {
        return Err(ToolError::invalid_field(
            "offset",
            agena_failure::FieldIssueKind::OutOfRange,
            format!(
                "read offset {} exceeds directory entry count {}",
                offset,
                entries.len()
            ),
        ));
    }

    let start = offset - 1;
    let end = min(start + limit, entries.len());
    let preview = entries[start..end].join("\n");
    let truncated = end < entries.len();
    Ok((preview, truncated, entries.len(), scan_truncated))
}

fn render_file_preview(
    text: &str,
    offset: usize,
    limit: usize,
) -> Result<(String, bool, usize, usize), ToolError> {
    let normalized = text.replace("\r\n", "\n");
    let lines = normalized.lines().collect::<Vec<_>>();

    if lines.is_empty() {
        return Ok((String::new(), false, 0, 0));
    }

    if offset > lines.len() {
        return Err(ToolError::invalid_field(
            "offset",
            agena_failure::FieldIssueKind::OutOfRange,
            format!(
                "read offset {} exceeds file line count {}",
                offset,
                lines.len()
            ),
        ));
    }

    let start = offset - 1;
    let end = min(start + limit, lines.len());
    let mut rendered = Vec::with_capacity(end - start);

    for (index, line) in lines[start..end].iter().enumerate() {
        rendered.push(format!(
            "{}: {}",
            start + index + 1,
            truncate_line_chars(line)
        ));
    }

    let truncated = end < lines.len();
    Ok((rendered.join("\n"), truncated, rendered.len(), lines.len()))
}

fn truncate_line_chars(input: &str) -> String {
    let mut iter = input.chars();
    let mut out = String::new();
    for _ in 0..MAX_LINE_CHARS {
        let Some(ch) = iter.next() else {
            return out;
        };
        out.push(ch);
    }

    if iter.next().is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::read_text_file_bounded;

    #[test]
    fn bounded_text_read_rejects_before_loading_the_rest_of_a_large_file() {
        let path = std::env::temp_dir().join(format!(
            "agena-read-bound-{}.txt",
            uuid::Uuid::new_v4().simple()
        ));
        let mut file = std::fs::File::create(&path).expect("create read fixture");
        file.write_all(b"small prefix").expect("write read prefix");
        file.set_len(1_000_000).expect("extend sparse read fixture");

        let error = read_text_file_bounded(&path, 32).expect_err("oversized text must fail");

        assert!(error.to_string().contains("limited to 32 bytes"));
        std::fs::remove_file(path).expect("remove read fixture");
    }
}
