use globset::{Glob, GlobMatcher, GlobSet, GlobSetBuilder};

use std::time::{Duration, Instant};

use crate::part::{GlobKind, GlobToolInput};

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    discovery::{effective_include_ignored, walk_builder_with_filter},
    normalize_path_for_display,
};

const DEFAULT_MATCHES: usize = 200;
const MAX_MATCHES: usize = 1_000;
const MAX_VISITED_ENTRIES: usize = 100_000;
const MAX_SCAN_DURATION: Duration = Duration::from_secs(10);
const MAX_RESULT_BYTES: usize = 256 * 1024;

#[derive(Default)]
struct ScanOptions<'a> {
    access: Option<agena_tool::path_filter::PathPredicate>,
    kind: GlobKind,
    exclude: GlobSet,
    cancel: Option<&'a tokio_util::sync::CancellationToken>,
}

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &GlobToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    executor.ensure_not_cancelled()?;
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
                "glob base path does not exist: {}",
                executor.display_path(&base_path)
            ),
        ));
    }
    if !base_path.is_dir() {
        return Err(ToolError::invalid_field(
            "path",
            agena_failure::FieldIssueKind::Invalid,
            format!(
                "glob base path is not a directory: {}",
                executor.display_path(&base_path)
            ),
        ));
    }

    let matcher = Glob::new(&input.pattern)?.compile_matcher();
    let offset = input.offset.unwrap_or_default() as usize;
    let limit = input.limit.unwrap_or(DEFAULT_MATCHES as u32) as usize;
    if limit == 0 || limit > MAX_MATCHES {
        return Err(ToolError::invalid_field(
            "limit",
            agena_failure::FieldIssueKind::OutOfRange,
            format!("glob limit must be between 1 and {MAX_MATCHES}"),
        ));
    }
    let include_ignored =
        effective_include_ignored(input.include_ignored, &base_path, executor.workspace_root());
    let mut exclude = GlobSetBuilder::new();
    if input.exclude.len() > 32 {
        return Err(ToolError::invalid_input(
            "glob accepts at most 32 exclusions",
        ));
    }
    for pattern in &input.exclude {
        if pattern.is_empty() || pattern.len() > 4096 {
            return Err(ToolError::invalid_input(
                "exclude globs must contain 1–4096 bytes",
            ));
        }
        exclude.add(Glob::new(pattern)?);
    }
    let access = executor.read_path_filter();
    let options = ScanOptions {
        access: Some(access.predicate()),
        kind: input.kind,
        exclude: exclude.build()?,
        cancel: executor.cancellation_token(),
    };
    let (matched_paths, stop_reason) = access.finish(collect_matches(
        &base_path,
        &matcher,
        offset,
        limit,
        include_ignored,
        &options,
    ))?;
    let truncated = stop_reason.is_some();
    let mut matches = matched_paths
        .iter()
        .map(|path| executor.display_path(path))
        .collect::<Vec<_>>();
    matches.sort();

    let mut output_text = if matches.is_empty() {
        "No paths matched the glob pattern.".to_string()
    } else {
        matches.join("\n")
    };
    if let Some(reason) = stop_reason.as_deref() {
        output_text.push_str("\n...glob scan truncated: ");
        output_text.push_str(reason);
        output_text.push_str(". Narrow `path` or `pattern` and retry.");
    }
    let output = ToolPayloadOutput::Glob {
        count: Some(matches.len() as u32),
        paths: matches.clone(),
        truncated,
    };

    let summary = if truncated {
        format!("{} paths · more available", matches.len())
    } else {
        format!("{} paths", matches.len())
    };
    let mut view =
        ToolExecutionView::simple(format!("Glob {}", input.pattern), summary, output_text);
    view.metadata
        .insert("pattern".to_string(), input.pattern.clone());
    view.metadata
        .insert("base_path".to_string(), executor.display_path(&base_path));
    view.metadata
        .insert("count".to_string(), matches.len().to_string());
    view.metadata
        .insert("offset".to_string(), offset.to_string());
    view.metadata.insert("limit".to_string(), limit.to_string());
    view.metadata
        .insert("include_ignored".to_string(), include_ignored.to_string());
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());
    if let Some(reason) = stop_reason.as_deref() {
        view.metadata
            .insert("stop_reason".to_string(), reason.to_string());
    }

    Ok(ToolPayloadExecution::new(output, view))
}

fn collect_matches(
    base_path: &std::path::Path,
    matcher: &GlobMatcher,
    offset: usize,
    limit: usize,
    include_ignored: bool,
    options: &ScanOptions<'_>,
) -> Result<(Vec<std::path::PathBuf>, Option<String>), ToolError> {
    let started = Instant::now();
    let mut matches = Vec::with_capacity(limit.min(DEFAULT_MATCHES));
    let mut skipped_matches = 0_usize;
    let mut skipped_errors = 0_usize;
    let mut first_skipped_error = None;
    let mut result_bytes = 0;

    for (entry_index, entry) in
        walk_builder_with_filter(base_path, include_ignored, options.access.clone())
            .build()
            .enumerate()
    {
        if options.cancel.is_some_and(|token| token.is_cancelled()) {
            return Err(ToolError::Cancelled);
        }
        if entry_index >= MAX_VISITED_ENTRIES {
            return Ok((
                matches,
                Some(glob_stop_reason(
                    "workspace entry limit reached",
                    skipped_errors,
                    first_skipped_error.as_deref(),
                )),
            ));
        }
        if started.elapsed().saturating_sub(
            options
                .access
                .as_ref()
                .map(|check| check.waited())
                .unwrap_or_default(),
        ) >= MAX_SCAN_DURATION
        {
            return Ok((
                matches,
                Some(glob_stop_reason(
                    "10-second scan deadline reached",
                    skipped_errors,
                    first_skipped_error.as_deref(),
                )),
            ));
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                skipped_errors = skipped_errors.saturating_add(1);
                if first_skipped_error.is_none() {
                    first_skipped_error =
                        Some(agena_failure::diagnostic::format_error_chain_with_context(
                            "glob scan could not read a workspace entry",
                            &error,
                        ));
                }
                continue;
            }
        };
        if entry.path() == base_path {
            continue;
        }
        let kind = entry.file_type();
        if (options.kind == GlobKind::File && !kind.is_some_and(|kind| kind.is_file()))
            || (options.kind == GlobKind::Directory && !kind.is_some_and(|kind| kind.is_dir()))
        {
            continue;
        }

        let relative = entry.path().strip_prefix(base_path).map_err(|err| {
            ToolError::invalid_input(format!(
                "glob failed to build relative path for '{}': {err}",
                entry.path().display()
            ))
        })?;

        let relative_norm = normalize_path_for_display(relative);
        if matcher.is_match(&relative_norm) && !options.exclude.is_match(&relative_norm) {
            if options
                .access
                .as_ref()
                .is_some_and(|check| !check.allows(entry.path()))
            {
                continue;
            }
            if skipped_matches < offset {
                skipped_matches += 1;
                continue;
            }
            if matches.len() >= limit {
                return Ok((
                    matches,
                    Some(glob_stop_reason(
                        "result page limit reached",
                        skipped_errors,
                        first_skipped_error.as_deref(),
                    )),
                ));
            }
            result_bytes += entry.path().to_string_lossy().len() * 6 + 4;
            if result_bytes > MAX_RESULT_BYTES {
                return Ok((
                    matches,
                    Some(glob_stop_reason(
                        "result-byte limit reached",
                        skipped_errors,
                        first_skipped_error.as_deref(),
                    )),
                ));
            }
            matches.push(entry.into_path());
        }
    }

    let stop_reason = (skipped_errors > 0).then(|| {
        glob_stop_reason(
            "workspace entries were skipped because they could not be read",
            skipped_errors,
            first_skipped_error.as_deref(),
        )
    });
    Ok((matches, stop_reason))
}

fn glob_stop_reason(base: &str, skipped_errors: usize, first_error: Option<&str>) -> String {
    if skipped_errors == 0 {
        return base.to_owned();
    }
    match first_error {
        Some(first_error) => format!(
            "{base}; {skipped_errors} workspace entries were unreadable; first error: {first_error}"
        ),
        None => format!("{base}; {skipped_errors} workspace entries were unreadable"),
    }
}
