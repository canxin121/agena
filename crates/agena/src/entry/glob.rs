use globset::Glob;
use walkdir::WalkDir;

use crate::message::{FirstPartyToolOutput, GlobToolInput};

use super::{
    FirstPartyExecution, ToolError, ToolExecutionView, ToolExecutor, normalize_path_for_display,
};

const MAX_MATCHES: usize = 5_000;

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &GlobToolInput,
) -> Result<FirstPartyExecution, ToolError> {
    let base_path = input
        .path
        .as_deref()
        .map(|path| executor.resolve_target_path(path))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    executor.ensure_read_permission(&base_path)?;

    if !base_path.exists() {
        return Err(ToolError::InvalidInput(format!(
            "glob base path does not exist: {}",
            executor.display_path(&base_path)
        )));
    }
    if !base_path.is_dir() {
        return Err(ToolError::InvalidInput(format!(
            "glob base path is not a directory: {}",
            executor.display_path(&base_path)
        )));
    }

    let matcher = Glob::new(&input.pattern)?.compile_matcher();
    let mut matches = Vec::new();

    for entry in WalkDir::new(&base_path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.path() == base_path {
            continue;
        }

        let relative = entry.path().strip_prefix(&base_path).map_err(|err| {
            ToolError::InvalidInput(format!(
                "glob failed to build relative path for '{}': {err}",
                entry.path().display()
            ))
        })?;

        let relative_norm = normalize_path_for_display(relative);
        if matcher.is_match(&relative_norm) {
            matches.push(executor.display_path(entry.path()));
            if matches.len() >= MAX_MATCHES {
                break;
            }
        }
    }

    matches.sort();

    let output_text = if matches.is_empty() {
        "No files matched the glob pattern.".to_string()
    } else {
        matches.join("\n")
    };
    let output = FirstPartyToolOutput::Glob {
        count: Some(matches.len() as u32),
    };

    let mut view = ToolExecutionView::simple(format!("Glob {}", input.pattern), output_text);
    view.metadata
        .insert("pattern".to_string(), input.pattern.clone());
    view.metadata
        .insert("base_path".to_string(), executor.display_path(&base_path));
    view.metadata
        .insert("count".to_string(), matches.len().to_string());

    Ok(FirstPartyExecution::new(output, view))
}
