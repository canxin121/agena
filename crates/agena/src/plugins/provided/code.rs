use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agena_macros::StaticToolSurface;
use ast_grep_core::Pattern;
use ast_grep_core::matcher::PatternBuilder;
use ast_grep_core::tree_sitter::{LanguageExt, TSLanguage};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tree_sitter::Parser;

use crate::plugin::PluginError;
use crate::plugin::sdk::{
    HookSubscription, InitContext, InitOutcome, PathRequest, Plugin, PluginManifest,
    PluginToolDecl, Result as SdkResult, ToolInvokeInput, ToolInvokeOutput, ToolTag,
};

pub(crate) const CODE_PLUGIN_ID: &str = "agena.code";

pub(crate) struct CodePlugin;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CodeLanguage {
    Rust,
}

impl Default for CodeLanguage {
    fn default() -> Self {
        Self::Rust
    }
}

#[derive(Debug, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    entry = "code",
    description = "Structured code inspection command. Use action `search_ast` for AST-aware pattern matching or `syntax_tree` to inspect the parsed syntax tree.",
    summary = "Search code structurally with ast-grep and inspect syntax trees.",
    help = "Use action `search_ast` with a Rust pattern like `if $COND { $BODY }` or `foo($ARGS)` to find structural matches. Use action `syntax_tree` to inspect the named syntax nodes for a Rust source file.",
    tags(ToolTag::ReadOnly, ToolTag::FilesystemRead, ToolTag::Discovery),
    concurrency_safe = true
)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum CodeToolInput {
    #[tool(exec = "search_ast")]
    SearchAst {
        #[serde(flatten)]
        args: CodeSearchAstInput,
    },
    #[tool(exec = "syntax_tree")]
    SyntaxTree {
        #[serde(flatten)]
        args: CodeSyntaxTreeInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CodeSearchAstInput {
    path: String,
    pattern: String,
    #[serde(default)]
    language: CodeLanguage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CodeSyntaxTreeInput {
    path: String,
    #[serde(default)]
    language: CodeLanguage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_depth: Option<u8>,
}

#[derive(Debug, Serialize)]
struct CodeSearchOutput {
    matches: Vec<CodeMatch>,
}

#[derive(Debug, Serialize)]
struct CodeMatch {
    path: String,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
    text: String,
}

#[derive(Debug, Serialize)]
struct SyntaxTreeOutput {
    path: String,
    root_kind: String,
    has_error: bool,
    tree: SyntaxNodeView,
}

#[derive(Debug, Serialize)]
struct SyntaxNodeView {
    kind: String,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
    text_preview: String,
    children: Vec<SyntaxNodeView>,
}

#[derive(Clone)]
struct RustLanguage;

impl ast_grep_core::Language for RustLanguage {
    fn kind_to_id(&self, kind: &str) -> u16 {
        let language: TSLanguage = tree_sitter_rust::LANGUAGE.into();
        language.id_for_node_kind(kind, true)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        self.get_ts_language()
            .field_id_for_name(field)
            .map(|field| field.get())
    }

    fn build_pattern(
        &self,
        builder: &PatternBuilder,
    ) -> Result<Pattern, ast_grep_core::PatternError> {
        builder.build(|src| ast_grep_core::tree_sitter::StrDoc::try_new(src, self.clone()))
    }
}

impl LanguageExt for RustLanguage {
    fn get_ts_language(&self) -> TSLanguage {
        tree_sitter_rust::LANGUAGE.into()
    }
}

#[async_trait]
impl Plugin for CodePlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("agena-code", env!("CARGO_PKG_VERSION"))
            .description("Structured code search and syntax inspection tools.")
            .hooks(HookSubscription::TOOL_INVOKE)
            .tool(code_decl())
            .build()
    }

    async fn init(
        &self,
        _ctx: InitContext,
        _host: Arc<dyn crate::plugin::sdk::host_api::HostClient>,
    ) -> SdkResult<InitOutcome> {
        Ok(InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> SdkResult<ToolInvokeOutput> {
        if input.tool_name != "code" {
            return Err(PluginError::invalid_params(format!(
                "unknown code plugin tool '{}'",
                input.tool_name
            )));
        }
        match parse_code_input(input.input)? {
            CodeToolInput::SearchAst { args } => {
                self.invoke_search_ast(&input.workspace_root, args)
            }
            CodeToolInput::SyntaxTree { args } => {
                self.invoke_syntax_tree(&input.workspace_root, args)
            }
        }
    }

    async fn permission_paths(
        &self,
        tool: &str,
        input: &serde_json::Value,
    ) -> SdkResult<Vec<PathRequest>> {
        if tool != "code" {
            return Ok(Vec::new());
        }
        let parsed = parse_code_input(input.clone())?;
        let path = match parsed {
            CodeToolInput::SearchAst { args } => args.path,
            CodeToolInput::SyntaxTree { args } => args.path,
        };
        Ok(vec![PathRequest::read(path)])
    }
}

impl CodePlugin {
    fn invoke_search_ast(
        &self,
        workspace_root: &str,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        match input.language {
            CodeLanguage::Rust => self.search_rust_ast(workspace_root, input),
        }
    }

    fn invoke_syntax_tree(
        &self,
        workspace_root: &str,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        match input.language {
            CodeLanguage::Rust => self.render_rust_syntax_tree(workspace_root, input),
        }
    }

    fn search_rust_ast(
        &self,
        workspace_root: &str,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let pattern = Pattern::try_new(input.pattern.trim(), RustLanguage)
            .map_err(|err| PluginError::invalid_params(err.to_string()))?;
        let limit = input.limit.unwrap_or(20).clamp(1, 100) as usize;
        let root = resolve_input_path(workspace_root, input.path.as_str());
        let files = collect_rust_files(&root)?;
        let mut matches = Vec::new();
        for path in files {
            let source = fs::read_to_string(&path).map_err(|err| {
                PluginError::new(format!("failed to read {}: {err}", path.display()))
            })?;
            let ast = RustLanguage.ast_grep(&source);
            for node in ast.root().find_all(pattern.clone()) {
                let start = node.start_pos();
                let end = node.end_pos();
                matches.push(CodeMatch {
                    path: display_path(workspace_root, &path),
                    start_line: start.line() + 1,
                    start_col: start.column(&node) + 1,
                    end_line: end.line() + 1,
                    end_col: end.column(&node) + 1,
                    text: truncate_for_output(node.text().into_owned(), 400),
                });
                if matches.len() >= limit {
                    break;
                }
            }
            if matches.len() >= limit {
                break;
            }
        }

        let payload = serde_json::to_value(CodeSearchOutput { matches })
            .map_err(|err| PluginError::new(err.to_string()))?;
        let output = payload
            .get("matches")
            .and_then(serde_json::Value::as_array)
            .map(|items| {
                if items.is_empty() {
                    "No AST matches found.".to_string()
                } else {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_object)
                        .map(|item| {
                            format!(
                                "{}:{}:{}-{}:{}\n{}",
                                item.get("path")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or(""),
                                item.get("start_line")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0),
                                item.get("start_col")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0),
                                item.get("end_line")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0),
                                item.get("end_col")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0),
                                item.get("text")
                                    .and_then(serde_json::Value::as_str)
                                    .unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n")
                }
            })
            .unwrap_or_else(|| "No AST matches found.".to_string());

        Ok(ToolInvokeOutput::text(output)
            .with_title("code search_ast")
            .with_payload(payload))
    }

    fn render_rust_syntax_tree(
        &self,
        workspace_root: &str,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let path = resolve_input_path(workspace_root, input.path.as_str());
        let source = fs::read_to_string(&path)
            .map_err(|err| PluginError::new(format!("failed to read {}: {err}", path.display())))?;
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|err| PluginError::new(format!("failed to load Rust parser: {err}")))?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| PluginError::new("failed to parse Rust source"))?;
        let root = tree.root_node();
        let max_depth = input.max_depth.unwrap_or(2).clamp(1, 6);
        let payload = serde_json::to_value(SyntaxTreeOutput {
            path: display_path(workspace_root, &path),
            root_kind: root.kind().to_string(),
            has_error: root.has_error(),
            tree: syntax_node_view(root, &source, 0, max_depth),
        })
        .map_err(|err| PluginError::new(err.to_string()))?;
        let output = serde_json::to_string_pretty(&payload)
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::text(output)
            .with_title("code syntax_tree")
            .with_payload(payload))
    }
}

pub(crate) fn new_plugin() -> CodePlugin {
    CodePlugin
}

pub(crate) fn code_decl() -> PluginToolDecl {
    CodeToolInput::tool_decl()
}

fn parse_code_input(input: serde_json::Value) -> SdkResult<CodeToolInput> {
    CodeToolInput::parse_input(input)
}

fn resolve_input_path(workspace_root: &str, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(workspace_root).join(path)
    }
}

fn collect_rust_files(path: &Path) -> SdkResult<Vec<PathBuf>> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut files = walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|candidate| candidate.is_file())
        .filter(|candidate| candidate.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn display_path(workspace_root: &str, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn truncate_for_output(text: String, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= limit {
        return trimmed.to_string();
    }
    format!("{}...", &trimmed[..limit])
}

fn syntax_node_view(
    node: tree_sitter::Node<'_>,
    source: &str,
    depth: u8,
    max_depth: u8,
) -> SyntaxNodeView {
    let start = node.start_position();
    let end = node.end_position();
    let text_preview = truncate_for_output(
        source
            .get(node.byte_range())
            .unwrap_or_default()
            .replace('\n', "\\n"),
        120,
    );
    let children = if depth >= max_depth {
        Vec::new()
    } else {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .take(50)
            .map(|child| syntax_node_view(child, source, depth + 1, max_depth))
            .collect()
    };
    SyntaxNodeView {
        kind: node.kind().to_string(),
        start_line: start.row + 1,
        start_col: start.column + 1,
        end_line: end.row + 1,
        end_col: end.column + 1,
        text_preview,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ast_finds_rust_pattern() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("sample.rs");
        fs::write(
            &file,
            "fn demo(value: i32) -> i32 { if value > 0 { value } else { 0 } }",
        )
        .expect("write");

        let plugin = CodePlugin;
        let output = plugin
            .search_rust_ast(
                dir.path().to_string_lossy().as_ref(),
                CodeSearchAstInput {
                    path: ".".to_string(),
                    pattern: "if $COND { $THEN } else { $ELSE }".to_string(),
                    language: CodeLanguage::Rust,
                    limit: Some(5),
                },
            )
            .expect("search_ast");
        assert!(output.output_text.contains("sample.rs"));
        assert!(
            output
                .output_text
                .contains("if value > 0 { value } else { 0 }")
        );
    }

    #[test]
    fn code_tool_input_rejects_unknown_fields() {
        let err = parse_code_input(serde_json::json!({
            "action": "search_ast",
            "path": ".",
            "pattern": "fn $NAME() { $BODY }",
            "extra": true
        }))
        .expect_err("code tool should reject unknown fields");
        assert!(err.to_string().contains("unknown field `extra`"));
    }
}
