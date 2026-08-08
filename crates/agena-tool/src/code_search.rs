//! Provider-independent structural code-search and syntax-tree algorithms.
//!
//! The Runtime builtin plugin owns SDK dispatch, permission context, and tool
//! output effects. This module owns the ast-grep/Tree-sitter implementation,
//! request validation, filesystem traversal, and stable result values.

use std::{
    fs,
    path::{Path, PathBuf},
};

use ast_grep_core::Pattern;
use ast_grep_language::{Language as AstGrepLanguage, LanguageExt, SupportLang};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tree_sitter::Parser;

pub const SUPPORTED_CODE_LANGUAGES: &str = "bash, c, cpp, csharp, css, dart, elixir, go, haskell, hcl, html, java, javascript, json, lua, markdown, nix, php, python, ruby, rust, solidity, swift, tsx, typescript, yaml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
/// Language of a code search target.
pub enum CodeLanguage {
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

#[derive(Debug, Clone)]
/// Request for a structural (AST) code search.
pub struct StructuralSearchRequest {
    pub path: PathBuf,
    pub pattern: String,
    pub language: Option<CodeLanguage>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone)]
/// Request for a syntax tree of a source file.
pub struct SyntaxTreeRequest {
    pub path: PathBuf,
    pub language: Option<CodeLanguage>,
    pub max_depth: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
/// Result of a structural code search.
pub struct StructuralSearchResult {
    pub language: String,
    pub scanned_files: usize,
    pub matches: Vec<StructuralCodeMatch>,
}

#[derive(Debug, Clone, Serialize)]
/// A single structural match with its source location.
pub struct StructuralCodeMatch {
    pub path: String,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
/// Syntax tree result for a source file.
pub struct SyntaxTreeResult {
    pub path: String,
    pub language: String,
    pub root_kind: String,
    pub has_error: bool,
    pub tree: SyntaxNodeView,
}

#[derive(Debug, Clone, Serialize)]
/// A node view inside a syntax tree.
pub struct SyntaxNodeView {
    pub kind: String,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub text_preview: String,
    pub children: Vec<SyntaxNodeView>,
}

#[derive(Debug, thiserror::Error)]
/// Error from structural code search.
pub enum CodeSearchError {
    #[error("{0}")]
    InvalidParameters(String),
    #[error("failed to read {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to load {language} parser: {message}")]
    Parser { language: String, message: String },
    #[error("failed to parse {language} source")]
    Parse { language: String },
}

pub fn search_ast(
    workspace_root: &Path,
    request: StructuralSearchRequest,
) -> Result<StructuralSearchResult, CodeSearchError> {
    let root = resolve_input_path(workspace_root, request.path.as_path());
    let language = resolve_code_language(&root, request.language, "search_ast")?;
    let pattern = Pattern::try_new(request.pattern.as_str(), language)
        .map_err(|error| CodeSearchError::InvalidParameters(error.to_string()))?;
    let limit = request.limit.unwrap_or(20).clamp(1, 100) as usize;
    let files = collect_language_files(&root, language);
    let scanned_files = files.len();
    let mut matches = Vec::new();

    for path in files {
        let source = fs::read_to_string(&path).map_err(|source| CodeSearchError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let ast = language.ast_grep(&source);
        for node in ast.root().find_all(pattern.clone()) {
            let start = node.start_pos();
            let end = node.end_pos();
            matches.push(StructuralCodeMatch {
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

    Ok(StructuralSearchResult {
        language: display_language(language).to_string(),
        scanned_files,
        matches,
    })
}

pub fn syntax_tree(
    workspace_root: &Path,
    request: SyntaxTreeRequest,
) -> Result<SyntaxTreeResult, CodeSearchError> {
    let path = resolve_input_path(workspace_root, request.path.as_path());
    let language = resolve_code_language(&path, request.language, "syntax_tree")?;
    let source = fs::read_to_string(&path).map_err(|source| CodeSearchError::Read {
        path: path.display().to_string(),
        source,
    })?;
    let mut parser = Parser::new();
    let parser_language: tree_sitter::Language = language.get_ts_language();
    parser
        .set_language(&parser_language)
        .map_err(|error| CodeSearchError::Parser {
            language: display_language(language).to_string(),
            message: error.to_string(),
        })?;
    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| CodeSearchError::Parse {
            language: display_language(language).to_string(),
        })?;
    let root = tree.root_node();
    let max_depth = request.max_depth.unwrap_or(2).clamp(1, 6);
    Ok(SyntaxTreeResult {
        path: display_path(workspace_root, &path),
        language: display_language(language).to_string(),
        root_kind: root.kind().to_string(),
        has_error: root.has_error(),
        tree: syntax_node_view(root, &source, 0, max_depth),
    })
}

pub fn format_search_output(result: &StructuralSearchResult) -> String {
    if result.matches.is_empty() {
        return format!(
            "No AST matches found in {} {} file(s).",
            result.scanned_files, result.language
        );
    }
    let details = result
        .matches
        .iter()
        .map(|item| {
            format!(
                "{}:{}:{}-{}:{}\n{}",
                item.path, item.start_line, item.start_col, item.end_line, item.end_col, item.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Found {} AST match(es) across {} {} file(s).\n\n{details}",
        result.matches.len(),
        result.scanned_files,
        result.language
    )
}

fn resolve_input_path(workspace_root: &Path, raw: &Path) -> PathBuf {
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        workspace_root.join(raw)
    }
}

fn resolve_code_language(
    path: &Path,
    requested: Option<CodeLanguage>,
    action: &str,
) -> Result<SupportLang, CodeSearchError> {
    if let Some(language) = requested.and_then(CodeLanguage::to_support_lang) {
        return Ok(language);
    }
    if path.is_dir() {
        return Err(CodeSearchError::InvalidParameters(format!(
            "`language` is required when `{action}` targets a directory. Supported languages: {SUPPORTED_CODE_LANGUAGES}."
        )));
    }
    infer_support_lang(path).ok_or_else(|| {
        CodeSearchError::InvalidParameters(format!(
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

fn collect_language_files(path: &Path, language: SupportLang) -> Vec<PathBuf> {
    if !path.is_dir() {
        return vec![path.to_path_buf()];
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
    files
}

fn display_path(workspace_root: &Path, path: &Path) -> String {
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
    use super::{CodeLanguage, StructuralSearchResult, format_search_output};

    #[test]
    fn search_output_is_stable_when_no_match_is_present() {
        let text = format_search_output(&StructuralSearchResult {
            language: "rust".to_string(),
            scanned_files: 3,
            matches: Vec::new(),
        });
        assert_eq!(text, "No AST matches found in 3 rust file(s).");
    }

    #[test]
    fn language_wire_aliases_remain_supported() {
        let language: CodeLanguage = serde_json::from_str("\"rs\"").expect("language alias");
        assert_eq!(language, CodeLanguage::Rust);
    }
}
