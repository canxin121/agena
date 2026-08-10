use std::{
    path::Path,
    time::{Duration, Instant},
};

use globset::{Glob, GlobMatcher};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::Lossy};

use crate::part::GrepToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    discovery::{effective_include_ignored, walk_builder},
    normalize_path_for_display,
};

const MAX_MATCHES: usize = 500;
const MAX_VISITED_ENTRIES: usize = 100_000;
const MAX_SEARCHED_FILES: usize = 25_000;
const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SEARCH_DURATION: Duration = Duration::from_secs(20);
const SEARCHER_HEAP_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopReason {
    Matches,
    Entries,
    Files,
    Bytes,
    Deadline,
}

impl StopReason {
    fn message(self) -> &'static str {
        match self {
            Self::Matches => "match limit reached",
            Self::Entries => "workspace entry limit reached",
            Self::Files => "searched-file limit reached",
            Self::Bytes => "total-byte limit reached",
            Self::Deadline => "20-second search deadline reached",
        }
    }
}

#[derive(Debug)]
struct SearchLimits {
    max_matches: usize,
    max_visited_entries: usize,
    max_searched_files: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_duration: Duration,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            max_matches: MAX_MATCHES,
            max_visited_entries: MAX_VISITED_ENTRIES,
            max_searched_files: MAX_SEARCHED_FILES,
            max_file_bytes: MAX_FILE_BYTES,
            max_total_bytes: MAX_TOTAL_BYTES,
            max_duration: MAX_SEARCH_DURATION,
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
    stop_reason: Option<StopReason>,
}

impl SearchStats {
    fn truncated(&self) -> bool {
        self.stop_reason.is_some() || self.skipped_large_files > 0 || self.skipped_io_errors > 0
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
                MAX_FILE_BYTES / (1024 * 1024),
            ));
        }
        if self.skipped_io_errors > 0 {
            reasons.push(format!(
                "{} unreadable path(s) skipped",
                self.skipped_io_errors
            ));
        }
        Some(format!(
            "...search truncated: {}. Narrow `path` or `include` and retry.",
            reasons.join("; ")
        ))
    }
}

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &GrepToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let base_path = input
        .path
        .as_deref()
        .map(|path| executor.resolve_target_path(path))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());

    if !base_path.exists() {
        return Err(ToolError::invalid_field(
            "path",
            agena_failure::FieldIssueKind::NotFound,
            format!(
                "grep base path does not exist: {}",
                executor.display_path(&base_path)
            ),
        ));
    }

    let matcher = RegexMatcherBuilder::new()
        .build(&input.pattern)
        .map_err(|error| {
            ToolError::invalid_field(
                "pattern",
                agena_failure::FieldIssueKind::Invalid,
                format!("invalid grep pattern: {error}"),
            )
        })?;
    let include = input
        .include
        .as_ref()
        .map(|glob| Glob::new(glob).map(|compiled| compiled.compile_matcher()))
        .transpose()?;
    let include_ignored =
        effective_include_ignored(input.include_ignored, &base_path, executor.workspace_root());

    let limits = SearchLimits::default();
    let (matches, stats) = collect_matches(
        executor,
        &base_path,
        &matcher,
        include.as_ref(),
        include_ignored,
        &limits,
    )?;
    let truncated = stats.truncated();

    let mut output_text = if matches.is_empty() {
        "No lines matched the grep pattern.".to_string()
    } else {
        matches.join("\n")
    };
    if let Some(note) = stats.truncation_note() {
        output_text.push('\n');
        output_text.push_str(&note);
    }

    let output = ToolPayloadOutput::Grep {
        matches: Some(matches.len() as u32),
        results: matches.clone(),
        truncated,
    };
    let summary = if truncated {
        format!("{} matches · search truncated", matches.len())
    } else {
        format!("{} matches", matches.len())
    };
    let mut view =
        ToolExecutionView::simple(format!("Grep {}", input.pattern), summary, output_text);
    view.metadata
        .insert("pattern".to_string(), input.pattern.clone());
    if let Some(include) = &input.include {
        view.metadata.insert("include".to_string(), include.clone());
    }
    view.metadata
        .insert("base_path".to_string(), executor.display_path(&base_path));
    view.metadata
        .insert("include_ignored".to_string(), include_ignored.to_string());
    view.metadata
        .insert("matches".to_string(), matches.len().to_string());
    view.metadata.insert(
        "visited_entries".to_string(),
        stats.visited_entries.to_string(),
    );
    view.metadata.insert(
        "searched_files".to_string(),
        stats.searched_files.to_string(),
    );
    view.metadata.insert(
        "searched_bytes".to_string(),
        stats.searched_bytes.to_string(),
    );
    view.metadata.insert(
        "skipped_large_files".to_string(),
        stats.skipped_large_files.to_string(),
    );
    view.metadata.insert(
        "skipped_io_errors".to_string(),
        stats.skipped_io_errors.to_string(),
    );
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());
    if let Some(reason) = stats.stop_reason {
        view.metadata
            .insert("stop_reason".to_string(), reason.message().to_string());
    }

    Ok(ToolPayloadExecution::new(output, view))
}

fn collect_matches(
    executor: &ToolExecutor,
    base_path: &Path,
    matcher: &RegexMatcher,
    include: Option<&GlobMatcher>,
    include_ignored: bool,
    limits: &SearchLimits,
) -> Result<(Vec<String>, SearchStats), ToolError> {
    let started = Instant::now();
    let mut matches = Vec::new();
    let mut stats = SearchStats::default();

    for entry in walk_builder(base_path, include_ignored).build() {
        if started.elapsed() >= limits.max_duration {
            stats.stop_reason = Some(StopReason::Deadline);
            break;
        }
        stats.visited_entries += 1;
        if stats.visited_entries > limits.max_visited_entries {
            stats.stop_reason = Some(StopReason::Entries);
            break;
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                stats.skipped_io_errors += 1;
                continue;
            }
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }

        let relative = if base_path.is_file() {
            entry
                .path()
                .file_name()
                .map(Path::new)
                .unwrap_or_else(|| Path::new(""))
        } else {
            entry.path().strip_prefix(base_path).unwrap_or_else(|_| {
                entry
                    .path()
                    .file_name()
                    .map(Path::new)
                    .unwrap_or_else(|| Path::new(""))
            })
        };
        let relative_norm = normalize_path_for_display(relative);
        if include
            .as_ref()
            .is_some_and(|matcher| !matcher.is_match(&relative_norm))
        {
            continue;
        }

        if stats.searched_files >= limits.max_searched_files {
            stats.stop_reason = Some(StopReason::Files);
            break;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                stats.skipped_io_errors += 1;
                continue;
            }
        };
        if metadata.len() > limits.max_file_bytes {
            stats.skipped_large_files += 1;
            continue;
        }
        if stats.searched_bytes.saturating_add(metadata.len()) > limits.max_total_bytes {
            stats.stop_reason = Some(StopReason::Bytes);
            break;
        }

        stats.searched_files += 1;
        stats.searched_bytes += metadata.len();
        let display_path = executor.display_path(entry.path());
        if search_file(
            entry.path(),
            &display_path,
            matcher,
            &mut matches,
            limits.max_matches,
        )
        .is_err()
        {
            stats.skipped_io_errors += 1;
            continue;
        }
        if matches.len() >= limits.max_matches {
            stats.stop_reason = Some(StopReason::Matches);
            break;
        }
    }

    Ok((matches, stats))
}

fn search_file(
    path: &Path,
    display_path: &str,
    matcher: &RegexMatcher,
    matches: &mut Vec<String>,
    max_matches: usize,
) -> std::io::Result<()> {
    SearcherBuilder::new()
        .line_number(true)
        .binary_detection(BinaryDetection::quit(b'\0'))
        .heap_limit(Some(SEARCHER_HEAP_BYTES))
        .build()
        .search_path(
            matcher,
            path,
            Lossy(|line_number, line| {
                let line = line.strip_suffix('\n').unwrap_or(line);
                let line = line.strip_suffix('\r').unwrap_or(line);
                matches.push(format!("{display_path}:{line_number}: {line}"));
                Ok(matches.len() < max_matches)
            }),
        )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{SearchLimits, SearchStats, StopReason};

    #[test]
    fn truncated_search_explains_every_applied_guardrail() {
        let stats = SearchStats {
            skipped_large_files: 2,
            skipped_io_errors: 1,
            stop_reason: Some(StopReason::Bytes),
            ..SearchStats::default()
        };

        let note = stats.truncation_note().expect("truncation note");
        assert!(note.contains("total-byte limit reached"));
        assert!(note.contains("2 file(s) larger than 32 MiB skipped"));
        assert!(note.contains("1 unreadable path(s) skipped"));
        assert!(note.contains("Narrow `path` or `include`"));
    }

    #[test]
    fn production_search_limits_are_finite() {
        let limits = SearchLimits::default();

        assert!(limits.max_matches > 0);
        assert!(limits.max_visited_entries > limits.max_searched_files);
        assert!(limits.max_file_bytes < limits.max_total_bytes);
        assert!(limits.max_duration > Duration::ZERO);
    }
}
