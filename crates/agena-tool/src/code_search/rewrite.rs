//! AST rewrite planning only. The host publishes with its file-lock/revision contract.
#[cfg(test)]
mod tests;
use super::*;

#[derive(Debug)]
pub struct StructuralRewritePlan {
    pub path: PathBuf,
    pub language: String,
    pub original: String,
    pub updated: String,
    pub replacements: usize,
}

/// Plan every match in one bounded UTF-8 source file. Never silently apply a
/// partial rewrite; ambiguous overlaps, parse errors and limits are errors.
pub fn plan_rewrite(
    workspace_root: &Path,
    request: StructuralSearchRequest,
    replacement: &str,
) -> Result<StructuralRewritePlan, CodeSearchError> {
    let path = resolve_input_path(workspace_root, &request.path);
    if path.is_dir() {
        return Err(CodeSearchError::InvalidParameters(
            "AST rewrites require one file path".into(),
        ));
    }
    let language = resolve_code_language(&path, request.language, "rewrite_ast")?;
    let config = compile_rule(
        language,
        &request.pattern,
        request.rule.as_ref(),
        Some(replacement),
    )?;
    let fixer = config
        .fixer
        .first()
        .expect("a replacement always creates one fixer");
    let limit = request.limit.unwrap_or(100);
    if !(1..=100).contains(&limit) {
        return Err(CodeSearchError::InvalidParameters(
            "rewrite limit must be between 1 and 100".into(),
        ));
    }
    let started = Instant::now();
    let original = read_source_bounded(&path)?;
    let ast = language.ast_grep(&original);
    if ast
        .root()
        .dfs()
        .any(|node| node.kind() == "ERROR" || node.is_missing())
    {
        return Err(CodeSearchError::InvalidParameters(
            "source has parse errors; repair it before an AST rewrite".into(),
        ));
    }
    let mut edits = Vec::new();
    for node in ast.root().find_all(&config.matcher) {
        if edits.len() >= limit as usize || started.elapsed() >= MAX_STRUCTURAL_DURATION {
            return Err(CodeSearchError::InvalidParameters(
                "AST rewrite exceeds its match/time limit; narrow the rule".into(),
            ));
        }
        edits.push(node.make_edit(&config.matcher, fixer));
    }
    edits.sort_by_key(|edit| (edit.position, edit.deleted_length));
    let mut end = 0;
    let mut last_start = None;
    let mut result_len = original.len();
    for edit in &edits {
        if edit.position < end || last_start == Some(edit.position) {
            return Err(CodeSearchError::InvalidParameters(
                "AST rewrite has overlapping matches; narrow the rule".into(),
            ));
        }
        end = edit.position + edit.deleted_length;
        last_start = Some(edit.position);
        result_len = result_len
            .checked_sub(edit.deleted_length)
            .and_then(|size| size.checked_add(edit.inserted_text.len()))
            .filter(|size| *size <= MAX_SOURCE_FILE_BYTES as usize)
            .ok_or_else(|| {
                CodeSearchError::InvalidParameters("AST rewrite result exceeds 8 MiB".into())
            })?;
    }
    let mut updated = original.as_bytes().to_vec();
    for edit in edits.iter().rev() {
        updated.splice(
            edit.position..edit.position + edit.deleted_length,
            edit.inserted_text.iter().copied(),
        );
    }
    let updated = String::from_utf8(updated).map_err(|_| {
        CodeSearchError::InvalidParameters("AST rewrite result is not UTF-8".into())
    })?;
    let updated_ast = language.ast_grep(&updated);
    if updated_ast
        .root()
        .dfs()
        .any(|node| node.kind() == "ERROR" || node.is_missing())
    {
        return Err(CodeSearchError::InvalidParameters(
            "AST replacement would introduce parse errors".into(),
        ));
    }
    Ok(StructuralRewritePlan {
        path,
        language: display_language(language).into(),
        original,
        updated,
        replacements: edits.len(),
    })
}
