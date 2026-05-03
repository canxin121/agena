use std::cmp::min;
use std::fs;

use crate::message::{BuiltinToolOutput, ReadToolInput};

use super::{BuiltinExecution, ToolError, ToolExecutionView, ToolExecutor};

const DEFAULT_OFFSET: usize = 1;
const DEFAULT_LIMIT: usize = 2000;
const MAX_LINE_CHARS: usize = 2000;

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &ReadToolInput,
) -> Result<BuiltinExecution, ToolError> {
    let target = executor.resolve_target_path(&input.file_path);
    executor.ensure_read_permission(&target)?;

    if !target.exists() {
        return Err(ToolError::InvalidInput(format!(
            "read target does not exist: {}",
            input.file_path
        )));
    }

    let offset = parse_offset(input.offset);
    let limit = parse_limit(input.limit);
    let display_path = executor.display_path(&target);

    if target.is_dir() {
        let (preview, truncated, count) = read_directory_listing(&target, offset, limit)?;
        let output = BuiltinToolOutput::Read {
            preview: Some(preview.clone()),
            truncated: Some(truncated),
            loaded_paths: vec![display_path.clone()],
        };

        let mut view = ToolExecutionView::simple(format!("Read {}", display_path), preview);
        view.metadata
            .insert("kind".to_string(), "directory".to_string());
        view.metadata
            .insert("offset".to_string(), offset.to_string());
        view.metadata.insert("limit".to_string(), limit.to_string());
        view.metadata
            .insert("entry_count".to_string(), count.to_string());
        view.metadata
            .insert("truncated".to_string(), truncated.to_string());
        return Ok(BuiltinExecution::new(output, view));
    }

    let content = fs::read(&target)?;
    let text = String::from_utf8(content).map_err(|_| {
        ToolError::InvalidInput(format!(
            "read tool currently supports UTF-8 text files only: {}",
            input.file_path
        ))
    })?;

    let (preview, truncated, rendered_lines, total_lines) =
        render_file_preview(&text, offset, limit)?;
    let output = BuiltinToolOutput::Read {
        preview: Some(preview.clone()),
        truncated: Some(truncated),
        loaded_paths: vec![display_path.clone()],
    };

    let mut view = ToolExecutionView::simple(format!("Read {}", display_path), preview);
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

    Ok(BuiltinExecution::new(output, view))
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
) -> Result<(String, bool, usize), ToolError> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
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
        return Ok((String::new(), false, 0));
    }

    if offset > entries.len() {
        return Err(ToolError::InvalidInput(format!(
            "read offset {} exceeds directory entry count {}",
            offset,
            entries.len()
        )));
    }

    let start = offset - 1;
    let end = min(start + limit, entries.len());
    let preview = entries[start..end].join("\n");
    let truncated = end < entries.len();
    Ok((preview, truncated, entries.len()))
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
        return Err(ToolError::InvalidInput(format!(
            "read offset {} exceeds file line count {}",
            offset,
            lines.len()
        )));
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
