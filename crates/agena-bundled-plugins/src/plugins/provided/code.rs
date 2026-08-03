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
struct CodeSearchAstInput {
    #[arg(trim, non_empty, path.read)]
    path: String,
    #[arg(trim, non_empty)]
    pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct CodeSyntaxTreeInput {
    #[arg(trim, non_empty, path.read)]
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_depth: Option<u8>,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "code",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Structured code search and syntax inspection tools.",
    display = brief
)]
impl CodePlugin {
    #[tool(
        tags(query, filesystem, discovery),
        summary = "Search code structurally with ast-grep.",
        help = "Supported languages: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml. Use patterns like `if $COND { $BODY }`, `def $NAME($ARGS): $$$`, or `function $NAME($ARGS) { $$$ }`. When `language` is omitted for a file path, Agena infers it from the extension. Directory searches require `language` explicitly.",
        read_only,

        discovery,
        display = brief,
        concurrency_safe
    )]
    async fn dispatch_search_ast(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_search_ast(context.workspace_root, input)
    }

    #[tool(
        tags(query, filesystem),
        summary = "Inspect a parsed syntax tree.",
        help = "Use `syntax_tree` to inspect named syntax nodes for a supported file. When `language` is omitted, Agena infers it from the file extension.",
        read_only,

        discovery,
        display = brief,
        concurrency_safe
    )]
    async fn dispatch_syntax_tree(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_syntax_tree(context.workspace_root, input)
    }

    fn invoke_search_ast(
        &self,
        workspace_root: &str,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let title = format!("Search AST · {}", input.pattern);
        let result = search_ast(
            Path::new(workspace_root),
            StructuralSearchRequest {
                path: input.path.into(),
                pattern: input.pattern,
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
            serde_json::to_value(result).map_err(|err| PluginError::internal(err.to_string()))?;
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
        &self,
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
            serde_json::to_value(result).map_err(|err| PluginError::internal(err.to_string()))?;
        let output = serde_json::to_string_pretty(&payload)
            .map_err(|err| PluginError::internal(err.to_string()))?;
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

fn code_search_error_to_plugin(error: CodeSearchError) -> PluginError {
    match error {
        CodeSearchError::InvalidParameters(message) => PluginError::invalid_params(message),
        error => PluginError::internal(error.to_string()),
    }
}
