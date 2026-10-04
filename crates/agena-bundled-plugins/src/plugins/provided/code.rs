mod rewrite;
use rewrite::{CodeRewriteInput, invoke_rewrite};

use std::path::Path;

use agena_macros::ToolInput;
use agena_tool::code_search::{
    CodeLanguage, CodeSearchError, StructuralSearchRequest, SyntaxTreeRequest,
    format_search_output, search_ast, syntax_tree,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};

pub(crate) const CODE_PLUGIN_ID: &str = "agena.code";

pub(crate) struct CodePlugin;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
#[input(exactly_one_of("pattern", "rule"), non_empty_if_present("pattern"))]
struct CodeSearchAstInput {
    #[arg(trim, non_empty)]
    path: String,
    /// Simple ast-grep pattern; provide exactly one of pattern or rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(max_chars = 16384)]
    pattern: Option<String>,
    /// Structured ast-grep rule object (kind, pattern, all/any/not, inside/has, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rule: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(minimum = 1, maximum = 100)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct CodeSyntaxTreeInput {
    #[arg(trim, non_empty)]
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[arg(minimum = 1, maximum = 6)]
    max_depth: Option<u8>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "code",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Structured code search and syntax inspection tools.",
)]
impl CodePlugin {
    #[tool(
        tags(query, filesystem, discovery, read_only),
        summary = "Search code structurally with ast-grep.",
        help = "Supported languages: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml. Use patterns like `if $COND { $BODY }`, `def $NAME($ARGS): $$$`, or `function $NAME($ARGS) { $$$ }`. When `language` is omitted for a file path, Agena infers it from the extension. Directory searches require `language` explicitly. Provide exactly one of pattern or a structured ast-grep rule object; relational/composite rules are supported. Rule bounds: 16 KiB, 16 levels, 512 values. Search returns at most 100 matches, explicitly marks incomplete scans, and flags shortened text previews. Use rewrite_ast to preview a single-file structural edit."
    )]
    async fn dispatch_search_ast(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_owned();
        run_code_blocking(move || Self::invoke_search_ast(&workspace_root, input)).await
    }

    #[tool(
        tags(query, filesystem, discovery, read_only),
        summary = "Inspect a parsed syntax tree.",
        help = "Use `syntax_tree` to inspect named syntax nodes for a supported file. When `language` is omitted, Agena infers it from the file extension. The preview has at most 512 nodes, 50 children per node and max_depth 1–6 (default 2); children_truncated and truncated report omitted descendants. Source files are limited to 8 MiB."
    )]
    async fn dispatch_syntax_tree(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_owned();
        run_code_blocking(move || Self::invoke_syntax_tree(&workspace_root, input)).await
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Preview or apply a revision-checked ast-grep rewrite in one file.",
        help = "Defaults to apply=false: returns a bounded unified diff, replacement count and before_sha256 without writing. Repeat with apply=true and expected_sha256 from the reviewed preview to publish. Provide exactly one pattern or structured rule and a replacement template (metavariables supported; empty deletes). Requires valid UTF-8 source, at most 8 MiB/file and 100 non-overlapping matches; rejects parse errors, unknown replacement variables, stale revisions and partial plans. Same file locks and publication checks as fs.replace; no directory-wide rewrite."
    )]
    async fn dispatch_rewrite_ast(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeRewriteInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace = context.workspace_root.to_owned();
        run_code_blocking(move || invoke_rewrite(Path::new(&workspace), input)).await
    }

    fn invoke_search_ast(
        workspace_root: &str,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let title = format!("Search AST · {}", input.path);
        let result = search_ast(
            Path::new(workspace_root),
            StructuralSearchRequest {
                path: input.path.into(),
                pattern: input.pattern.unwrap_or_default(),
                rule: input.rule,
                language: input.language,
                limit: input.limit,
            },
        )
        .map_err(code_search_error_to_plugin)?;
        let output = format_search_output(&result);
        let summary = format!(
            "{} matches in {} files",
            result.matches.len(),
            result.scanned_files
        );
        let payload =
            serde_json::to_value(result).map_err(|err| PluginError::internal_error(&err))?;
        Ok(ToolInvokeOutput::from_parts(
            title,
            summary,
            output,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }

    fn invoke_syntax_tree(
        workspace_root: &str,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let title = format!("Syntax tree · {}", input.path);
        let result = syntax_tree(
            Path::new(workspace_root),
            SyntaxTreeRequest {
                path: input.path.into(),
                language: input.language,
                max_depth: input.max_depth,
            },
        )
        .map_err(code_search_error_to_plugin)?;
        let summary = format!(
            "{} · root {}{}",
            result.language,
            result.root_kind,
            if result.has_error {
                " · parse errors"
            } else {
                ""
            }
        );
        let payload =
            serde_json::to_value(result).map_err(|err| PluginError::internal_error(&err))?;
        let output = serde_json::to_string_pretty(&payload)
            .map_err(|err| PluginError::internal_error(&err))?;
        Ok(ToolInvokeOutput::from_parts(
            title,
            summary,
            output,
            Some(payload),
            std::collections::BTreeMap::new(),
            Vec::new(),
        ))
    }
}

pub(crate) fn new_plugin() -> CodePlugin {
    CodePlugin
}

async fn run_code_blocking<T, F>(work: F) -> SdkResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> SdkResult<T> + Send + 'static,
{
    let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
        .acquire()
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "acquire a code plugin worker",
                &error,
            ))
        })?;
    tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        work()
    })
    .await
    .map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            "code plugin worker failed",
            &error,
        ))
    })?
}

fn code_search_error_to_plugin(error: CodeSearchError) -> PluginError {
    match error {
        CodeSearchError::InvalidParameters(message) => PluginError::invalid_params(message),
        error => PluginError::internal_error(&error),
    }
}
