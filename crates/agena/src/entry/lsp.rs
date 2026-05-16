//! `lsp_definition` / `lsp_references` / `lsp_hover` / `lsp_diagnostics`
//! bundled tools.
//!
//! Each tool resolves the right server via the [`agena_lsp::LspRegistry`]
//! threaded through `ToolExecutor::with_lsp_registry`, runs the LSP
//! request in the host's tokio runtime via the same `mcp::block_on`
//! helper the other async-from-sync tools use, and returns a flattened
//! `path:line:col` representation that the LLM can paste back to the
//! user verbatim.

use std::path::Path;

use agena_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, GotoDefinitionResponse, Hover, HoverContents, Location,
    MarkedString, Position, Range, Uri,
};

use crate::message::{
    BundledToolOutput, LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput,
    LspReferencesToolInput,
};

use super::{BundledExecution, ToolError, ToolExecutionView, ToolExecutor};

pub(super) fn execute_definition(
    executor: &ToolExecutor,
    input: &LspDefinitionToolInput,
) -> Result<BundledExecution, ToolError> {
    let registry = registry(executor)?;
    let path = executor.resolve_target_path(&input.file_path);
    executor.ensure_read_permission(&path)?;
    let uri = path_to_uri(&path)?;
    let pos = Position::new(input.line, input.character);

    let response = super::mcp::block_on(async {
        let client = registry.client_for_path(&path).await?;
        sync_document(&client, &path, &uri).await?;
        client.definition(uri, pos).await
    })
    .map_err(map_lsp_err)?;

    let locations = response.map(format_definition_response).unwrap_or_default();

    let view = ToolExecutionView::simple(
        format!(
            "lsp_definition {}:{}:{}",
            display_path(&path, executor),
            input.line + 1,
            input.character + 1
        ),
        if locations.is_empty() {
            "no definition found".to_string()
        } else {
            locations.join("\n")
        },
    );
    Ok(BundledExecution::new(
        BundledToolOutput::LspDefinition { locations },
        view,
    ))
}

pub(super) fn execute_references(
    executor: &ToolExecutor,
    input: &LspReferencesToolInput,
) -> Result<BundledExecution, ToolError> {
    let registry = registry(executor)?;
    let path = executor.resolve_target_path(&input.file_path);
    executor.ensure_read_permission(&path)?;
    let uri = path_to_uri(&path)?;
    let pos = Position::new(input.line, input.character);
    let include_decl = input.include_declaration;

    let response = super::mcp::block_on(async {
        let client = registry.client_for_path(&path).await?;
        sync_document(&client, &path, &uri).await?;
        client.references(uri, pos, include_decl).await
    })
    .map_err(map_lsp_err)?;

    let locations = response
        .map(|locs| locs.iter().map(format_location).collect::<Vec<_>>())
        .unwrap_or_default();

    let view = ToolExecutionView::simple(
        format!(
            "lsp_references {}:{}:{}",
            display_path(&path, executor),
            input.line + 1,
            input.character + 1
        ),
        if locations.is_empty() {
            "no references found".to_string()
        } else {
            locations.join("\n")
        },
    );
    Ok(BundledExecution::new(
        BundledToolOutput::LspReferences { locations },
        view,
    ))
}

pub(super) fn execute_hover(
    executor: &ToolExecutor,
    input: &LspHoverToolInput,
) -> Result<BundledExecution, ToolError> {
    let registry = registry(executor)?;
    let path = executor.resolve_target_path(&input.file_path);
    executor.ensure_read_permission(&path)?;
    let uri = path_to_uri(&path)?;
    let pos = Position::new(input.line, input.character);

    let response = super::mcp::block_on(async {
        let client = registry.client_for_path(&path).await?;
        sync_document(&client, &path, &uri).await?;
        client.hover(uri, pos).await
    })
    .map_err(map_lsp_err)?;

    let contents = response.as_ref().map(format_hover);

    let view = ToolExecutionView::simple(
        format!(
            "lsp_hover {}:{}:{}",
            display_path(&path, executor),
            input.line + 1,
            input.character + 1
        ),
        contents
            .clone()
            .unwrap_or_else(|| "no hover info".to_string()),
    );
    Ok(BundledExecution::new(
        BundledToolOutput::LspHover { contents },
        view,
    ))
}

pub(super) fn execute_diagnostics(
    executor: &ToolExecutor,
    input: &LspDiagnosticsToolInput,
) -> Result<BundledExecution, ToolError> {
    let registry = registry(executor)?;
    let path = executor.resolve_target_path(&input.file_path);
    executor.ensure_read_permission(&path)?;
    let uri = path_to_uri(&path)?;

    // Make sure the language server has been spawned (it might not have
    // pushed diagnostics yet if no other tool touched this file).
    let entries = super::mcp::block_on(async {
        let client = registry.client_for_path(&path).await?;
        sync_document(&client, &path, &uri).await?;
        // Give the server a brief window to publish diagnostics for the
        // doc we just synced; servers typically push within ~100ms.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        Ok::<_, agena_lsp::LspError>(client.diagnostics_for(&uri))
    })
    .map_err(map_lsp_err)?;

    let formatted: Vec<String> = entries
        .iter()
        .map(|d| format_diagnostic(&path, executor, d))
        .collect();

    let view = ToolExecutionView::simple(
        format!("lsp_diagnostics {}", display_path(&path, executor)),
        if formatted.is_empty() {
            "no diagnostics".to_string()
        } else {
            formatted.join("\n")
        },
    );
    Ok(BundledExecution::new(
        BundledToolOutput::LspDiagnostics { entries: formatted },
        view,
    ))
}

fn registry(executor: &ToolExecutor) -> Result<&std::sync::Arc<agena_lsp::LspRegistry>, ToolError> {
    executor
        .lsp_registry()
        .ok_or_else(|| ToolError::Plugin("no LSP servers configured".to_string()))
}

fn path_to_uri(path: &Path) -> Result<Uri, ToolError> {
    let url = url::Url::from_file_path(path).map_err(|_| {
        ToolError::InvalidInput(format!("file path not absolute: {}", path.display()))
    })?;
    url.as_str()
        .parse::<Uri>()
        .map_err(|err| ToolError::InvalidInput(format!("invalid uri: {err}")))
}

fn map_lsp_err(err: agena_lsp::LspError) -> ToolError {
    use agena_lsp::LspError;
    match err {
        LspError::UnknownServer(name) => {
            ToolError::Plugin(format!("no LSP server matches `{name}`"))
        }
        LspError::Server { code, message } => {
            ToolError::Plugin(format!("lsp server error {code}: {message}"))
        }
        other => ToolError::Plugin(other.to_string()),
    }
}

fn format_definition_response(resp: GotoDefinitionResponse) -> Vec<String> {
    match resp {
        GotoDefinitionResponse::Scalar(loc) => vec![format_location(&loc)],
        GotoDefinitionResponse::Array(arr) => arr.iter().map(format_location).collect(),
        GotoDefinitionResponse::Link(links) => links
            .iter()
            .map(|link| format_uri_range(&link.target_uri, &link.target_range))
            .collect(),
    }
}

fn format_location(loc: &Location) -> String {
    format_uri_range(&loc.uri, &loc.range)
}

fn format_uri_range(uri: &Uri, range: &Range) -> String {
    format!(
        "{}:{}:{}",
        uri_display(uri),
        range.start.line + 1,
        range.start.character + 1
    )
}

fn uri_display(uri: &Uri) -> String {
    let raw = uri.to_string();
    raw.strip_prefix("file://")
        .map(str::to_string)
        .unwrap_or(raw)
}

fn display_path(path: &Path, executor: &ToolExecutor) -> String {
    executor.display_path(path)
}

fn format_hover(hover: &Hover) -> String {
    fn marked_string(ms: &MarkedString) -> String {
        match ms {
            MarkedString::String(s) => s.clone(),
            MarkedString::LanguageString(ls) => ls.value.clone(),
        }
    }
    match &hover.contents {
        HoverContents::Scalar(ms) => marked_string(ms),
        HoverContents::Array(items) => items
            .iter()
            .map(marked_string)
            .collect::<Vec<_>>()
            .join("\n\n"),
        HoverContents::Markup(mc) => mc.value.clone(),
    }
}

fn format_diagnostic(path: &Path, executor: &ToolExecutor, diag: &Diagnostic) -> String {
    let severity = diag
        .severity
        .map(|s| match s {
            DiagnosticSeverity::ERROR => "error",
            DiagnosticSeverity::WARNING => "warning",
            DiagnosticSeverity::INFORMATION => "info",
            DiagnosticSeverity::HINT => "hint",
            _ => "note",
        })
        .unwrap_or("note");
    format!(
        "{}:{}:{} [{severity}] {}",
        display_path(path, executor),
        diag.range.start.line + 1,
        diag.range.start.character + 1,
        diag.message.trim()
    )
}

async fn sync_document(
    client: &agena_lsp::LspClient,
    path: &Path,
    uri: &Uri,
) -> Result<(), agena_lsp::LspError> {
    let text = match tokio::fs::read_to_string(path).await {
        Ok(t) => t,
        Err(_) => return Ok(()), // unreadable file: let the server keep
                                 // whatever it had
    };
    let language_id = language_id_for_path(path);
    client.sync_document(uri.clone(), text, &language_id).await
}

fn language_id_for_path(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "go" => "go",
        "java" => "java",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "sh" | "bash" => "shellscript",
        other if !other.is_empty() => other,
        _ => "plaintext",
    }
    .to_string()
}
