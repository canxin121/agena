use globset::{Glob, GlobMatcher};
use std::time::{Duration, Instant};

use crate::part::GlobToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    discovery::{effective_include_ignored, walk_builder},
    normalize_path_for_display,
};

const DEFAULT_MATCHES: usize = 200;
const MAX_MATCHES: usize = 1_000;
const MAX_VISITED_ENTRIES: usize = 100_000;
const MAX_SCAN_DURATION: Duration = Duration::from_secs(10);
pub(super) fn execute(
    executor: &ToolExecutor,
    input: &GlobToolInput,
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
    let (matched_paths, stop_reason) =
        collect_matches(&base_path, &matcher, offset, limit, include_ignored)?;
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
) -> Result<(Vec<std::path::PathBuf>, Option<String>), ToolError> {
    let started = Instant::now();
    let mut matches = Vec::with_capacity(limit.min(DEFAULT_MATCHES));
    let mut skipped_matches = 0_usize;
    let mut skipped_errors = 0_usize;
    let mut first_skipped_error = None;

    for (entry_index, entry) in walk_builder(base_path, include_ignored).build().enumerate() {
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
        if started.elapsed() >= MAX_SCAN_DURATION {
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

        let relative = entry.path().strip_prefix(base_path).map_err(|err| {
            ToolError::invalid_input(format!(
                "glob failed to build relative path for '{}': {err}",
                entry.path().display()
            ))
        })?;

        let relative_norm = normalize_path_for_display(relative);
        if matcher.is_match(&relative_norm) {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use globset::Glob;

    use super::collect_matches;

    #[test]
    fn glob_pages_deterministically_and_skips_heavy_directories_by_default() {
        let root =
            std::env::temp_dir().join(format!("agena-glob-test-{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&root).expect("temporary glob workspace");
        for path in [
            "src/a.rs",
            "src/b.rs",
            "src/c.rs",
            "node_modules/dependency.rs",
            "target/generated.rs",
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
            fs::write(path, "fixture").expect("write fixture");
        }
        let matcher = Glob::new("**/*.rs").expect("glob").compile_matcher();

        let (first, first_stop_reason) =
            collect_matches(&root, &matcher, 0, 2, false).expect("first page");
        let (second, second_stop_reason) =
            collect_matches(&root, &matcher, 2, 2, false).expect("second page");
        assert_eq!(
            first
                .iter()
                .map(|path| path.strip_prefix(&root).expect("relative"))
                .collect::<Vec<_>>(),
            vec![
                std::path::Path::new("src/a.rs"),
                std::path::Path::new("src/b.rs")
            ]
        );
        assert_eq!(
            first_stop_reason.as_deref(),
            Some("result page limit reached")
        );
        assert_eq!(
            second
                .iter()
                .map(|path| path.strip_prefix(&root).expect("relative"))
                .collect::<Vec<_>>(),
            vec![std::path::Path::new("src/c.rs")]
        );
        assert_eq!(second_stop_reason, None);

        let (all, _) = collect_matches(&root, &matcher, 0, 10, true).expect("ignored paths");
        assert_eq!(all.len(), 5);
        fs::remove_dir_all(root).expect("remove glob fixture");
    }
}
