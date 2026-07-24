use std::fs;

use globset::Glob;
use regex::Regex;
use walkdir::WalkDir;

use crate::message::GrepToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    normalize_path_for_display,
};

const MAX_MATCHES: usize = 500;

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &GrepToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let base_path = input
        .path
        .as_deref()
        .map(|path| executor.resolve_target_path(path))
        .unwrap_or_else(|| executor.workspace_root().to_path_buf());
    executor.ensure_read_permission(&base_path)?;

    if !base_path.exists() {
        return Err(ToolError::InvalidInput(format!(
            "grep base path does not exist: {}",
            executor.display_path(&base_path)
        )));
    }
    if !base_path.is_dir() {
        return Err(ToolError::InvalidInput(format!(
            "grep base path is not a directory: {}",
            executor.display_path(&base_path)
        )));
    }

    let pattern = Regex::new(&input.pattern)?;
    let include = input
        .include
        .as_ref()
        .map(|glob| Glob::new(glob).map(|compiled| compiled.compile_matcher()))
        .transpose()?;

    let mut matches = Vec::new();
    let mut truncated = false;

    'walk: for entry in WalkDir::new(&base_path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let relative = entry.path().strip_prefix(&base_path).map_err(|err| {
            ToolError::InvalidInput(format!(
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

        executor.ensure_read_permission(entry.path())?;

        let Ok(text) = fs::read_to_string(entry.path()) else {
            continue;
        };

        for (index, line) in text.replace("\r\n", "\n").lines().enumerate() {
            if !pattern.is_match(line) {
                continue;
            }

            matches.push(format!(
                "{}:{}: {}",
                executor.display_path(entry.path()),
                index + 1,
                line
            ));

            if matches.len() >= MAX_MATCHES {
                truncated = true;
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

    let mut view = ToolExecutionView::simple(format!("Grep {}", input.pattern), output_text);
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
