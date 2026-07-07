use std::fs;
use std::path::{Path, PathBuf};

use agena_macros::{StaticToolSurface, ToolInputShape, ToolSuite};
use ast_grep_core::Pattern;
use ast_grep_language::{Language as AstGrepLanguage, LanguageExt, SupportLang};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tree_sitter::Parser;

use crate::plugin::PluginError;
use crate::plugin::sdk::{
    PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, ToolTag,
};

pub(crate) const CODE_PLUGIN_ID: &str = "agena.code";

const SUPPORTED_CODE_LANGUAGES: &str = "bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml";

pub(crate) struct CodePlugin;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CodeLanguage {
    Auto,
    #[serde(alias = "sh", alias = "shell")]
    Bash,
    C,
    #[serde(alias = "cc", alias = "cxx")]
    Cpp,
    #[serde(alias = "c_sharp", alias = "cs")]
    Csharp,
    Css,
    Dart,
    #[serde(alias = "ex")]
    Elixir,
    Go,
    #[serde(alias = "hs")]
    Haskell,
    Hcl,
    #[serde(alias = "htm")]
    Html,
    Java,
    #[serde(alias = "js", alias = "jsx")]
    Javascript,
    Json,
    Lua,
    #[serde(alias = "md")]
    Markdown,
    Nix,
    Php,
    #[serde(alias = "py")]
    Python,
    #[serde(alias = "rb")]
    Ruby,
    #[serde(alias = "rs")]
    Rust,
    #[serde(alias = "sol")]
    Solidity,
    Swift,
    Tsx,
    #[serde(alias = "ts")]
    Typescript,
    #[serde(alias = "yml")]
    Yaml,
}

impl CodeLanguage {
    fn canonical_name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Bash => "bash",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Csharp => "csharp",
            Self::Css => "css",
            Self::Dart => "dart",
            Self::Elixir => "elixir",
            Self::Go => "go",
            Self::Haskell => "haskell",
            Self::Hcl => "hcl",
            Self::Html => "html",
            Self::Java => "java",
            Self::Javascript => "javascript",
            Self::Json => "json",
            Self::Lua => "lua",
            Self::Markdown => "markdown",
            Self::Nix => "nix",
            Self::Php => "php",
            Self::Python => "python",
            Self::Ruby => "ruby",
            Self::Rust => "rust",
            Self::Solidity => "solidity",
            Self::Swift => "swift",
            Self::Tsx => "tsx",
            Self::Typescript => "typescript",
            Self::Yaml => "yaml",
        }
    }

    fn to_support_lang(self) -> Option<SupportLang> {
        match self {
            Self::Auto => None,
            Self::Bash => Some(SupportLang::Bash),
            Self::C => Some(SupportLang::C),
            Self::Cpp => Some(SupportLang::Cpp),
            Self::Csharp => Some(SupportLang::CSharp),
            Self::Css => Some(SupportLang::Css),
            Self::Dart => Some(SupportLang::Dart),
            Self::Elixir => Some(SupportLang::Elixir),
            Self::Go => Some(SupportLang::Go),
            Self::Haskell => Some(SupportLang::Haskell),
            Self::Hcl => Some(SupportLang::Hcl),
            Self::Html => Some(SupportLang::Html),
            Self::Java => Some(SupportLang::Java),
            Self::Javascript => Some(SupportLang::JavaScript),
            Self::Json => Some(SupportLang::Json),
            Self::Lua => Some(SupportLang::Lua),
            Self::Markdown => Some(SupportLang::Markdown),
            Self::Nix => Some(SupportLang::Nix),
            Self::Php => Some(SupportLang::Php),
            Self::Python => Some(SupportLang::Python),
            Self::Ruby => Some(SupportLang::Ruby),
            Self::Rust => Some(SupportLang::Rust),
            Self::Solidity => Some(SupportLang::Solidity),
            Self::Swift => Some(SupportLang::Swift),
            Self::Tsx => Some(SupportLang::Tsx),
            Self::Typescript => Some(SupportLang::TypeScript),
            Self::Yaml => Some(SupportLang::Yaml),
        }
    }

    fn from_support_lang(language: SupportLang) -> Option<Self> {
        match language {
            SupportLang::Bash => Some(Self::Bash),
            SupportLang::C => Some(Self::C),
            SupportLang::Cpp => Some(Self::Cpp),
            SupportLang::CSharp => Some(Self::Csharp),
            SupportLang::Css => Some(Self::Css),
            SupportLang::Dart => Some(Self::Dart),
            SupportLang::Elixir => Some(Self::Elixir),
            SupportLang::Go => Some(Self::Go),
            SupportLang::Haskell => Some(Self::Haskell),
            SupportLang::Hcl => Some(Self::Hcl),
            SupportLang::Html => Some(Self::Html),
            SupportLang::Java => Some(Self::Java),
            SupportLang::JavaScript => Some(Self::Javascript),
            SupportLang::Json => Some(Self::Json),
            SupportLang::Lua => Some(Self::Lua),
            SupportLang::Markdown => Some(Self::Markdown),
            SupportLang::Nix => Some(Self::Nix),
            SupportLang::Php => Some(Self::Php),
            SupportLang::Python => Some(Self::Python),
            SupportLang::Ruby => Some(Self::Ruby),
            SupportLang::Rust => Some(Self::Rust),
            SupportLang::Solidity => Some(Self::Solidity),
            SupportLang::Swift => Some(Self::Swift),
            SupportLang::Tsx => Some(Self::Tsx),
            SupportLang::TypeScript => Some(Self::Typescript),
            SupportLang::Yaml => Some(Self::Yaml),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "search_ast",
    summary = "Search code structurally with ast-grep.",
    help = "Supported languages: bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml. Use patterns like `if $COND { $BODY }`, `def $NAME($ARGS): $$$`, or `function $NAME($ARGS) { $$$ }`. When `language` is omitted for a file path, Agena infers it from the extension. Directory searches require `language` explicitly.",
    handler_receiver = CodePlugin,
    handle_with_context = CodePlugin::dispatch_search_ast,
    handle_field = args,
    permission_paths_handle = CodePlugin::permission_search_ast,
    handle_by_value = true,
    display = brief,
    tags(ToolTag::ReadOnly, ToolTag::FilesystemRead, ToolTag::Discovery),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct CodeSearchAstToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CodeSearchAstInput,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema, StaticToolSurface)]
#[tool_surface(
    tool = "syntax_tree",
    summary = "Inspect a parsed syntax tree.",
    help = "Use `syntax_tree` to inspect named syntax nodes for a supported file. When `language` is omitted, Agena infers it from the file extension.",
    handler_receiver = CodePlugin,
    handle_with_context = CodePlugin::dispatch_syntax_tree,
    handle_field = args,
    permission_paths_handle = CodePlugin::permission_syntax_tree,
    handle_by_value = true,
    display = brief,
    tags(ToolTag::ReadOnly, ToolTag::FilesystemRead, ToolTag::Discovery),
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct CodeSyntaxTreeToolInput {
    #[tool(flatten_shape)]
    #[serde(flatten)]
    args: CodeSyntaxTreeInput,
}

#[allow(dead_code)]
#[derive(Debug, ToolSuite)]
#[tool_suite(handler_receiver = CodePlugin)]
enum CodeToolSuite {
    SearchAst(CodeSearchAstToolInput),
    SyntaxTree(CodeSyntaxTreeToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("path", "pattern"), non_empty("path", "pattern"))]
#[serde(deny_unknown_fields)]
struct CodeSearchAstInput {
    path: String,
    pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInputShape)]
#[tool_input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct CodeSyntaxTreeInput {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<CodeLanguage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_depth: Option<u8>,
}

#[derive(Debug, Serialize)]
struct CodeSearchOutput {
    language: String,
    scanned_files: usize,
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
    language: String,
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

#[crate::plugin::sdk::plugin(
    namespace = "agena",
    name = "code",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Structured code search and syntax inspection tools.",
    display = brief
)]
impl CodePlugin {
    #[tool_suite]
    async fn tool_invoke(
        &self,
        input: CodeToolSuite,
        context: &ToolInvokeContext<'_>,
    ) -> SdkResult<ToolInvokeOutput> {
        input.dispatch_tool_invoke_with_context(self, context).await
    }

    #[permission(paths, suite)]
    async fn permission_paths(&self, input: CodeToolSuite) -> SdkResult<Vec<PathRequest>> {
        input.dispatch_permission_paths(self).await
    }
}

impl CodePlugin {
    async fn dispatch_search_ast(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_search_ast(context.workspace_root, input)
    }

    async fn dispatch_syntax_tree(
        &self,
        context: &ToolInvokeContext<'_>,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.invoke_syntax_tree(context.workspace_root, input)
    }

    async fn permission_search_ast(
        &self,
        input: CodeSearchAstInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(vec![PathRequest::read(input.path)])
    }

    async fn permission_syntax_tree(
        &self,
        input: CodeSyntaxTreeInput,
    ) -> SdkResult<Vec<PathRequest>> {
        Ok(vec![PathRequest::read(input.path)])
    }

    fn invoke_search_ast(
        &self,
        workspace_root: &str,
        input: CodeSearchAstInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let root = resolve_input_path(workspace_root, input.path.as_str());
        let language = resolve_code_language(&root, input.language, "search_ast")?;
        let pattern = Pattern::try_new(input.pattern.as_str(), language)
            .map_err(|err| PluginError::invalid_params(err.to_string()))?;
        let limit = input.limit.unwrap_or(20).clamp(1, 100) as usize;
        let files = collect_language_files(&root, language)?;
        let scanned_files = files.len();
        let mut matches = Vec::new();

        for path in files {
            let source = fs::read_to_string(&path).map_err(|err| {
                PluginError::new(format!("failed to read {}: {err}", path.display()))
            })?;
            let ast = language.ast_grep(&source);
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

        let payload = serde_json::to_value(CodeSearchOutput {
            language: display_language(language).to_string(),
            scanned_files,
            matches,
        })
        .map_err(|err| PluginError::new(err.to_string()))?;
        let output = format_search_output(&payload);
        Ok(ToolInvokeOutput::from_parts(
            "code search_ast",
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
        let path = resolve_input_path(workspace_root, input.path.as_str());
        let language = resolve_code_language(&path, input.language, "syntax_tree")?;
        let source = fs::read_to_string(&path)
            .map_err(|err| PluginError::new(format!("failed to read {}: {err}", path.display())))?;
        let mut parser = Parser::new();
        let parser_language: tree_sitter::Language = language.get_ts_language().into();
        parser.set_language(&parser_language).map_err(|err| {
            PluginError::new(format!(
                "failed to load {} parser: {err}",
                display_language(language)
            ))
        })?;
        let tree = parser.parse(&source, None).ok_or_else(|| {
            PluginError::new(format!(
                "failed to parse {} source",
                display_language(language)
            ))
        })?;
        let root = tree.root_node();
        let max_depth = input.max_depth.unwrap_or(2).clamp(1, 6);
        let payload = serde_json::to_value(SyntaxTreeOutput {
            path: display_path(workspace_root, &path),
            language: display_language(language).to_string(),
            root_kind: root.kind().to_string(),
            has_error: root.has_error(),
            tree: syntax_node_view(root, &source, 0, max_depth),
        })
        .map_err(|err| PluginError::new(err.to_string()))?;
        let output = serde_json::to_string_pretty(&payload)
            .map_err(|err| PluginError::new(err.to_string()))?;
        Ok(ToolInvokeOutput::from_parts(
            "code syntax_tree",
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

fn resolve_input_path(workspace_root: &str, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(workspace_root).join(path)
    }
}

fn resolve_code_language(
    path: &Path,
    requested: Option<CodeLanguage>,
    action: &str,
) -> SdkResult<SupportLang> {
    if let Some(language) = requested.and_then(CodeLanguage::to_support_lang) {
        return Ok(language);
    }
    if path.is_dir() {
        return Err(PluginError::invalid_params(format!(
            "`language` is required when `{action}` targets a directory. Supported languages: {SUPPORTED_CODE_LANGUAGES}."
        )));
    }
    infer_support_lang(path).ok_or_else(|| {
        PluginError::invalid_params(format!(
            "unable to infer code language from '{}'; pass `language` explicitly. Supported languages: {SUPPORTED_CODE_LANGUAGES}.",
            path.display()
        ))
    })
}

fn infer_support_lang(path: &Path) -> Option<SupportLang> {
    let language = <SupportLang as AstGrepLanguage>::from_path(path)?;
    CodeLanguage::from_support_lang(language)?;
    Some(language)
}

fn collect_language_files(path: &Path, language: SupportLang) -> SdkResult<Vec<PathBuf>> {
    if !path.is_dir() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut files = walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .map(|entry| entry.into_path())
        .filter(|candidate| candidate.is_file())
        .filter(|candidate| {
            infer_support_lang(candidate).is_some_and(|detected| detected == language)
        })
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

fn display_language(language: SupportLang) -> &'static str {
    CodeLanguage::from_support_lang(language)
        .map(CodeLanguage::canonical_name)
        .unwrap_or("unknown")
}

fn format_search_output(payload: &serde_json::Value) -> String {
    let language = payload
        .get("language")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("code");
    let scanned_files = payload
        .get("scanned_files")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let Some(matches) = payload.get("matches").and_then(serde_json::Value::as_array) else {
        return "No AST matches found.".to_string();
    };
    if matches.is_empty() {
        return format!("No AST matches found in {scanned_files} {language} file(s).");
    }
    let details = matches
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
        .join("\n\n");
    format!(
        "Found {} AST match(es) across {scanned_files} {language} file(s).\n\n{details}",
        matches.len()
    )
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
