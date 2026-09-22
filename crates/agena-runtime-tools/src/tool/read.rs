mod stream;
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
            read_info: None,
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

    let page = stream::read_page(&target, offset, limit, executor.cancellation_token())?;
    let truncated = page.next_offset.is_some() || page.truncated_lines;
    let info = serde_json::json!({"offset":offset,"returned_lines":page.returned,"next_offset":page.next_offset,"total_lines":page.total_lines,
        "truncated_lines":page.truncated_lines,"scanned_bytes":page.scanned_bytes,"source_bytes":page.source_bytes,"modified_ns":page.modified_ns.map(|value|value.to_string())});
    let output = ToolPayloadOutput::Read {
        preview: Some(page.preview.clone()),
        truncated,
        loaded_paths: vec![display_path.clone()],
        attachment: None,
        read_info: Some(info),
    };
    let summary = match page.total_lines {
        Some(total) => format!("{} of {total} lines", page.returned),
        None => format!("{} lines · more available", page.returned),
    };
    let mut view = ToolExecutionView::simple(format!("Read {display_path}"), summary, page.preview);
    view.metadata.insert("kind".into(), "file".into());
    view.metadata.insert("offset".into(), offset.to_string());
    view.metadata.insert("limit".into(), limit.to_string());
    view.metadata
        .insert("truncated".into(), truncated.to_string());
    if let Some(next) = page.next_offset {
        view.metadata.insert("next_offset".into(), next.to_string());
    }

    Ok(ToolPayloadExecution::new(output, view))
}

fn read_prefix(path: &std::path::Path, limit: usize) -> Result<Vec<u8>, ToolError> {
    let mut bytes = Vec::with_capacity(limit);
    fs::File::open(path)?
        .take(limit as u64)
        .read_to_end(&mut bytes)?;
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
