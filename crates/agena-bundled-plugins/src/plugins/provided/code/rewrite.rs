#[cfg(test)]
mod tests;
use super::*;
use agena_tool::code_search::plan_rewrite;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(
    exactly_one_of("pattern", "rule"),
    non_empty_if_present("pattern", "expected_sha256")
)]
pub(super) struct CodeRewriteInput {
    #[arg(trim, non_empty)]
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 16384)]
    pattern: Option<String>,
    /// Structured ast-grep rule; the same shape as search_ast.rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rule: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    /// ast-grep replacement template. Empty text deletes matched nodes.
    #[arg(max_chars = 16384)]
    replacement: String,
    /// False previews; true publishes after checking expected_sha256.
    #[serde(default)]
    apply: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(trim)]
    expected_sha256: Option<String>,
}

pub(super) fn invoke_rewrite(
    workspace: &Path,
    input: CodeRewriteInput,
) -> SdkResult<ToolInvokeOutput> {
    if input.apply && input.expected_sha256.is_none() {
        return Err(PluginError::invalid_params(
            "apply=true requires expected_sha256 from the preview",
        ));
    }
    let path = Path::new(&input.path);
    let target = agena_runtime_tools::canonicalize_mutation_path(&workspace.join(path));
    let run = || {
        let plan = plan_rewrite(
            workspace,
            StructuralSearchRequest {
                path: target.clone(),
                pattern: input.pattern.clone().unwrap_or_default(),
                rule: input.rule.clone(),
                language: input.language,
                limit: Some(100),
            },
            &input.replacement,
        )
        .map_err(code_search_error_to_plugin)?;
        let before_sha256 = hex::encode(Sha256::digest(plan.original.as_bytes()));
        if input
            .expected_sha256
            .as_ref()
            .is_some_and(|expected| !expected.eq_ignore_ascii_case(&before_sha256))
        {
            return Err(PluginError::invalid_params(
                "stale AST rewrite revision; preview the current file again",
            ));
        }
        let after_sha256 = hex::encode(Sha256::digest(plan.updated.as_bytes()));
        let changed = before_sha256 != after_sha256;
        let diff = agena_runtime_tools::file_diff_preview(
            &input.path,
            Some(&plan.original),
            Some(&plan.updated),
        );
        if input.apply && changed {
            agena_runtime_tools::atomic_replace_file_with_check(
                &plan.path,
                plan.updated.as_bytes(),
                || agena_runtime_tools::verify_file_contents(&plan.path, plan.original.as_bytes()),
            )
            .map_err(|error| PluginError::internal_error(&error))?;
        }
        let status = if input.apply { "applied" } else { "preview" };
        let mut metadata = std::collections::BTreeMap::new();
        if input.apply && changed {
            metadata.insert("agena.effect".into(), "file_changes".into());
            metadata.insert("path".into(), input.path.clone());
            metadata.insert("before_sha256".into(), before_sha256.clone());
            metadata.insert("after_sha256".into(), after_sha256.clone());
        }
        Ok(ToolInvokeOutput::from_parts(
            format!("AST rewrite {status} · {}", input.path),
            format!("{} replacements · {status}", plan.replacements),
            format!(
                "{} replacements ({status}); sha256 {before_sha256} -> {after_sha256}\n{}",
                plan.replacements, diff.diff
            ),
            Some(serde_json::json!({
                "path":input.path, "language":plan.language, "applied":input.apply,
                "changed":changed, "replacements":plan.replacements,
                "before_sha256":before_sha256, "after_sha256":after_sha256,
                "diff":diff.diff, "diff_truncated":diff.diff_truncated,
                "additions":diff.additions, "deletions":diff.deletions,
            })),
            metadata,
            Vec::new(),
        ))
    };
    if input.apply {
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&target), run)
            .map_err(|error| PluginError::internal_error(&error))?
    } else {
        run()
    }
}
