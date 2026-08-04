use std::{fs, path::Path};

use globset::Glob;
use regex::Regex;
use walkdir::WalkDir;

use crate::message::GrepToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    normalize_path_for_display,
};

const MAX_MATCHES: usize = 500;

/// Whether `search_file` should keep scanning further files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopSearch {
    Continue,
    Stop,
}

/// Search one file for lines matching `pattern`, appending `path:line: text`
/// results to `matches`. Returns `Stop` once the result cap is reached so the
/// caller can abort the whole search.
fn search_file(
    executor: &ToolExecutor,
    path: &Path,
    pattern: &Regex,
    matches: &mut Vec<String>,
    truncated: &mut bool,
) -> StopSearch {
    let Ok(text) = fs::read_to_string(path) else {
        return StopSearch::Continue;
    };
    search_text(
        &executor.display_path(path),
        &text,
        pattern,
        matches,
        truncated,
    )
}

/// Match every line of `text` against `pattern`, appending
/// `display_path:line: text` results to `matches`. Returns `Stop` once the
/// result cap is reached so the caller can abort the whole search.
fn search_text(
    display_path: &str,
    text: &str,
    pattern: &Regex,
    matches: &mut Vec<String>,
    truncated: &mut bool,
) -> StopSearch {
    for (index, line) in text.replace("\r\n", "\n").lines().enumerate() {
        if !pattern.is_match(line) {
            continue;
        }

        matches.push(format!("{}:{}: {}", display_path, index + 1, line));

        if matches.len() >= MAX_MATCHES {
            *truncated = true;
            return StopSearch::Stop;
        }
    }
    StopSearch::Continue
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

    let pattern = Regex::new(&input.pattern)?;
    let include = input
        .include
        .as_ref()
        .map(|glob| Glob::new(glob).map(|compiled| compiled.compile_matcher()))
        .transpose()?;

    let mut matches = Vec::new();
    let mut truncated = false;

    if base_path.is_file() {
        // A single file target greps just that file. The include glob is
        // matched against the file name, mirroring how a directory walk
        // matches each file's path relative to the base.
        let relative = base_path
            .file_name()
            .map(Path::new)
            .unwrap_or_else(|| Path::new(""));
        let relative_norm = normalize_path_for_display(relative);
        if !include
            .as_ref()
            .is_some_and(|matcher| !matcher.is_match(&relative_norm))
        {
            search_file(executor, &base_path, &pattern, &mut matches, &mut truncated);
        }
    } else {
        'walk: for entry in WalkDir::new(&base_path)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }

            let relative = entry.path().strip_prefix(&base_path).map_err(|err| {
                ToolError::invalid_input(format!(
                    "grep failed to build relative path for '{}': {err}",
                    entry.path().display()
                ))
            })?;
            let relative_norm = normalize_path_for_display(relative);

            if include
                .as_ref()
                .is_some_and(|matcher| !matcher.is_match(&relative_norm))
            {
                continue;
            }

            if search_file(
                executor,
                entry.path(),
                &pattern,
                &mut matches,
                &mut truncated,
            ) == StopSearch::Stop
            {
                break 'walk;
            }
        }
    }

    let output_text = if matches.is_empty() {
        "No lines matched the grep pattern.".to_string()
    } else {
        let mut text = matches.join("\n");
        if truncated {
            text.push_str("\n...truncated");
        }
        text
    };
    let output = ToolPayloadOutput::Grep {
        matches: Some(matches.len() as u32),
        results: matches.clone(),
        truncated,
    };

    let summary = if truncated {
        format!("{} matches · truncated", matches.len())
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
        .insert("matches".to_string(), matches.len().to_string());
    view.metadata
        .insert("truncated".to_string(), truncated.to_string());

    Ok(ToolPayloadExecution::new(output, view))
}

#[cfg(test)]
mod tests {
    use super::{MAX_MATCHES, StopSearch, search_text};

    #[test]
    fn search_text_reports_path_line_and_matching_text() {
        let pattern = regex::Regex::new("alpha").unwrap();
        let mut matches = Vec::new();
        let mut truncated = false;

        let result = search_text(
            "fixture.txt",
            "alpha\nbeta\nalpha again\n",
            &pattern,
            &mut matches,
            &mut truncated,
        );

        assert_eq!(result, StopSearch::Continue);
        assert_eq!(
            matches,
            vec!["fixture.txt:1: alpha", "fixture.txt:3: alpha again"]
        );
        assert!(!truncated);
    }

    #[test]
    fn search_text_normalizes_crlf_before_matching() {
        let pattern = regex::Regex::new("hit").unwrap();
        let mut matches = Vec::new();
        let mut truncated = false;

        search_text(
            "win.txt",
            "hit\r\nmiss\r\nhit\r\n",
            &pattern,
            &mut matches,
            &mut truncated,
        );

        assert_eq!(matches, vec!["win.txt:1: hit", "win.txt:3: hit"]);
        assert!(!truncated);
    }

    #[test]
    fn search_text_stops_at_the_result_cap() {
        let pattern = regex::Regex::new("hit").unwrap();
        let mut matches = Vec::new();
        let mut truncated = false;

        let result = search_text(
            "big.txt",
            "hit\n".repeat(600).as_str(),
            &pattern,
            &mut matches,
            &mut truncated,
        );

        assert_eq!(result, StopSearch::Stop);
        assert_eq!(matches.len(), MAX_MATCHES);
        assert!(truncated);
    }
}
