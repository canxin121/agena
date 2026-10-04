//! Bounded in-process ripgrep search. Count limits apply to files, not lines.
#[cfg(test)]
mod benchmark;
mod scan;
#[cfg(test)]
mod tests;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    discovery::{effective_include_ignored, walk_builder},
    normalize_path_for_display,
    payload::GrepRecord,
};
use crate::part::{GrepCase, GrepMode, GrepToolInput};
use globset::{Glob, GlobSet, GlobSetBuilder};
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const MAX_MATCHES: usize = 500;
const MAX_VISITED_ENTRIES: usize = 100_000;
const MAX_SEARCHED_FILES: usize = 25_000;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SEARCH_DURATION: Duration = Duration::from_secs(20);
const SEARCHER_HEAP_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_LINE_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    Results,
    Entries,
    Files,
    Bytes,
    Deadline,
    Output,
}
impl StopReason {
    fn message(self) -> &'static str {
        match self {
            Self::Results => "additional results exceed max_results",
            Self::Entries => "workspace entry limit reached",
            Self::Files => "searched-file limit reached",
            Self::Bytes => "total-byte limit reached",
            Self::Deadline => "search deadline reached",
            Self::Output => "result-byte limit reached",
        }
    }
}

#[derive(Debug)]
struct SearchLimits {
    max_results: usize,
    max_visited_entries: usize,
    max_searched_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_duration: Duration,
    max_output_bytes: usize,
}
impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_results: MAX_MATCHES,
            max_visited_entries: MAX_VISITED_ENTRIES,
            max_searched_files: MAX_SEARCHED_FILES,
            max_file_bytes: MAX_FILE_BYTES,
            max_total_bytes: MAX_TOTAL_BYTES,
            max_duration: MAX_SEARCH_DURATION,
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}

#[derive(Debug, Default)]
struct SearchStats {
    visited_entries: usize,
    searched_files: usize,
    searched_bytes: u64,
    skipped_large_files: usize,
    skipped_io_errors: usize,
    skipped_changed_files: usize,
    binary_files: usize,
    shortened_lines: usize,
    stop_reason: Option<StopReason>,
}
impl SearchStats {
    fn scan_complete(&self) -> bool {
        self.stop_reason.is_none()
            && self.skipped_large_files == 0
            && self.skipped_io_errors == 0
            && self.skipped_changed_files == 0
    }
    fn truncated(&self) -> bool {
        !self.scan_complete() || self.shortened_lines > 0
    }
    fn truncation_note(&self) -> Option<String> {
        if !self.truncated() {
            return None;
        }
        let mut reasons = Vec::new();
        if let Some(reason) = self.stop_reason {
            reasons.push(reason.message().to_string());
        }
        if self.skipped_large_files > 0 {
            reasons.push(format!(
                "{} file(s) larger than {} MiB skipped",
                self.skipped_large_files,
                MAX_FILE_BYTES / (1024 * 1024)
            ));
        }
        if self.skipped_io_errors > 0 {
            reasons.push(format!(
                "{} unreadable path(s) skipped (including lines exceeding the search buffer)",
                self.skipped_io_errors
            ));
        }
        if self.skipped_changed_files > 0 {
            reasons.push(format!(
                "{} file(s) changed while searching; their results discarded",
                self.skipped_changed_files
            ));
        }
        if self.shortened_lines > 0 {
            reasons.push(format!("{} line(s) shortened; use fs.read for another range or a byte-oriented reader for full long lines", self.shortened_lines));
        }
        Some(format!(
            "...search truncated: {}. Narrow path or include and retry. Partial counts are lower bounds.",
            reasons.join("; ")
        ))
    }
}

#[derive(Debug)]
struct SearchResult {
    records: Vec<GrepRecord>,
    stats: SearchStats,
    returned_matches: usize,
}

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &GrepToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    executor.ensure_not_cancelled()?;
    let base_path = input
        .path
        .as_deref()
        .map(|path| executor.resolve_target_path(path))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    let limits = SearchLimits {
        max_results: input.max_results.unwrap_or(MAX_MATCHES as u32) as usize,
        ..SearchLimits::default()
    };
    let SearchResult {
        records,
        stats,
        returned_matches,
    } = collect_matches(
        &base_path,
        executor.workspace_root(),
        input,
        executor.cancellation_token(),
        &limits,
    )?;
    let truncated = stats.truncated();
    let scan_complete = stats.scan_complete();
    let note = stats.truncation_note();
    let mut output_text = if records.is_empty() {
        if scan_complete {
            "No lines matched the grep pattern."
        } else {
            "No matches found in the portion searched; search is incomplete."
        }
        .to_string()
    } else {
        records
            .iter()
            .map(GrepRecord::display)
            .collect::<Vec<_>>()
            .join("\n")
    };
    if let Some(note) = &note {
        output_text.push('\n');
        output_text.push_str(note);
    }
    let label = if input.mode == GrepMode::Content {
        "matching lines"
    } else {
        "matching files"
    };
    let output = ToolPayloadOutput::Grep {
        matches: (input.mode == GrepMode::Content).then_some(returned_matches as u32),
        results: Vec::new(),
        mode: input.mode,
        records,
        scan_complete: Some(scan_complete),
        note,
        truncated,
    };
    let mut view = ToolExecutionView::simple(
        format!("Grep {}", input.pattern),
        format!(
            "{returned_matches} {label}{}",
            if truncated { " · truncated" } else { "" }
        ),
        output_text,
    );
    for (key, value) in [
        ("pattern", input.pattern.clone()),
        ("base_path", executor.display_path(&base_path)),
        ("visited_entries", stats.visited_entries.to_string()),
        ("searched_files", stats.searched_files.to_string()),
        ("searched_bytes", stats.searched_bytes.to_string()),
        ("skipped_large_files", stats.skipped_large_files.to_string()),
        ("skipped_io_errors", stats.skipped_io_errors.to_string()),
        (
            "skipped_changed_files",
            stats.skipped_changed_files.to_string(),
        ),
        ("binary_files", stats.binary_files.to_string()),
        ("shortened_lines", stats.shortened_lines.to_string()),
        ("truncated", truncated.to_string()),
        ("scan_complete", scan_complete.to_string()),
    ] {
        view.metadata.insert(key.to_string(), value);
    }
    if let Some(reason) = stats.stop_reason {
        view.metadata
            .insert("stop_reason".into(), reason.message().into());
    }
    Ok(ToolPayloadExecution::new(output, view))
}

fn compile_globs<'a>(patterns: impl Iterator<Item = &'a str>) -> Result<GlobSet, ToolError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if pattern.is_empty() || pattern.len() > 4096 {
            return Err(ToolError::invalid_input(
                "search filter globs must contain 1–4096 bytes",
            ));
        }
        builder.add(Glob::new(pattern)?);
    }
    Ok(builder.build()?)
}

fn collect_matches(
    base_path: &Path,
    workspace: &Path,
    input: &GrepToolInput,
    cancel: Option<&CancellationToken>,
    limits: &SearchLimits,
) -> Result<SearchResult, ToolError> {
    if cancel.is_some_and(CancellationToken::is_cancelled) {
        return Err(ToolError::Cancelled);
    }
    let metadata = std::fs::metadata(base_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ToolError::invalid_field(
                "path",
                agena_failure::FieldIssueKind::NotFound,
                format!("grep base path does not exist: {}", base_path.display()),
            )
        } else {
            ToolError::Io(error)
        }
    })?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err(ToolError::invalid_input(
            "grep target must be a regular file or directory",
        ));
    }
    if input.before_context > 20
        || input.after_context > 20
        || !(1..=MAX_MATCHES).contains(&limits.max_results)
    {
        return Err(ToolError::invalid_input(
            "context must be 0–20 and max_results must be 1–500",
        ));
    }
    if input.mode != GrepMode::Content && (input.before_context > 0 || input.after_context > 0) {
        return Err(ToolError::invalid_input(
            "context is supported only in content mode",
        ));
    }
    if input.pattern.is_empty()
        || input.pattern.len() > 65536
        || input.includes.len() > 32
        || input.exclude.len() > 32
    {
        return Err(ToolError::invalid_input(
            "grep pattern or filter budget exceeded",
        ));
    }
    let matcher = RegexMatcherBuilder::new()
        .fixed_strings(input.fixed_strings)
        .case_insensitive(input.case == GrepCase::Insensitive)
        .case_smart(input.case == GrepCase::Smart)
        .line_terminator(Some(b'\n'))
        .size_limit(2 * 1024 * 1024)
        .dfa_size_limit(2 * 1024 * 1024)
        .build(&input.pattern)
        .map_err(|error| {
            ToolError::invalid_field(
                "pattern",
                agena_failure::FieldIssueKind::Invalid,
                format!("invalid grep pattern: {error}"),
            )
        })?;
    let include = compile_globs(
        input
            .include
            .iter()
            .chain(&input.includes)
            .map(String::as_str),
    )?;
    let exclude = compile_globs(input.exclude.iter().map(String::as_str))?;
    let include_ignored = effective_include_ignored(input.include_ignored, base_path, workspace);
    let started = Instant::now();
    // Reuse buffers across files instead of rebuilding a searcher for each path.
    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .before_context(input.before_context as usize)
        .after_context(input.after_context as usize)
        .binary_detection(BinaryDetection::quit(b'\0'))
        .heap_limit(Some(SEARCHER_HEAP_BYTES))
        .build();
    let mut records = Vec::new();
    let mut stats = SearchStats::default();
    let mut returned_matches = 0;
    let mut output_bytes = 0;
    for entry in walk_builder(base_path, include_ignored).build() {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(ToolError::Cancelled);
        }
        if started.elapsed() >= limits.max_duration {
            stats.stop_reason = Some(StopReason::Deadline);
            break;
        }
        if stats.visited_entries >= limits.max_visited_entries {
            stats.stop_reason = Some(StopReason::Entries);
            break;
        }
        stats.visited_entries += 1;
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                stats.skipped_io_errors += 1;
                tracing::debug!(%error, "grep skipped unreadable entry");
                continue;
            }
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let relative = if metadata.is_file() {
            entry
                .path()
                .file_name()
                .map(Path::new)
                .unwrap_or(entry.path())
        } else {
            entry.path().strip_prefix(base_path).unwrap_or(entry.path())
        };
        let relative = normalize_path_for_display(relative);
        if (!include.is_empty() && !include.is_match(&relative)) || exclude.is_match(&relative) {
            continue;
        }
        if stats.searched_files >= limits.max_searched_files {
            stats.stop_reason = Some(StopReason::Files);
            break;
        }
        let display_path = normalize_path_for_display(
            entry.path().strip_prefix(workspace).unwrap_or(entry.path()),
        );
        let result = scan::search_file(
            &mut searcher,
            &matcher,
            scan::FileRequest {
                path: entry.path(),
                display_path: &display_path,
                mode: input.mode,
                remaining_results: limits.max_results - returned_matches,
                remaining_output: limits.max_output_bytes - output_bytes,
                remaining_bytes: limits.max_total_bytes - stats.searched_bytes,
                max_file_bytes: limits.max_file_bytes,
                deadline: started + limits.max_duration,
                cancel,
            },
        )?;
        stats.searched_files += 1;
        stats.searched_bytes += result.bytes_read;
        stats.skipped_large_files += usize::from(result.too_large);
        stats.skipped_io_errors += usize::from(result.io_error);
        stats.skipped_changed_files += usize::from(result.changed);
        stats.binary_files += usize::from(result.binary);
        stats.shortened_lines += result.shortened_lines;
        returned_matches += result.returned_matches;
        output_bytes += result.output_bytes;
        records.extend(result.records);
        if let Some(reason) = result.stop_reason {
            stats.stop_reason = Some(reason);
            break;
        }
    }
    Ok(SearchResult {
        records,
        stats,
        returned_matches,
    })
}
