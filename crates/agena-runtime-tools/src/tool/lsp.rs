//! Authorized LSP queries use a single disk snapshot and bounded results.

mod results;
pub(super) use results::preview as result_preview;
pub use results::{LspDiagnosticResult, LspHoverResult, LspLocationsResult, LspSource, LspSymbol};
use results::{LspTarget, position_label, preview, take_budget};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agena_lsp::lsp_types::{
    DocumentSymbol, DocumentSymbolResponse, GotoDefinitionResponse, Hover, HoverContents, Location,
    MarkedString, OneOf, Position, SymbolInformation, Uri, WorkspaceSymbolResponse,
};
use agena_lsp::{DocumentReceipt, LspClient, LspClientMode, LspConnection, LspResult};
use futures_util::StreamExt;
use sha2::{Digest, Sha256};

use crate::part::{
    LspDefinitionToolInput, LspDiagnosticsManyToolInput, LspDiagnosticsToolInput,
    LspHoverToolInput, LspPositionToolInput, LspReferencesToolInput, LspStartToolInput,
    LspStartToolOutput, LspStopToolInput, LspStopToolOutput, LspSymbolsToolInput,
};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

const MAX_LSP_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
const PUSH_DIAGNOSTIC_WAIT: Duration = Duration::from_secs(2);
const MAX_RESULT_BYTES: usize = 24 * 1024;
const MAX_RESULTS: usize = 100;
pub(super) const MAX_DIAGNOSTIC_FILES: usize = 16;
static LSP_DOCUMENT_READERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

struct FileQuery<'a> {
    path: &'a str,
    folder: Option<&'a str>,
    server: Option<&'a str>,
    expected_revision: Option<&'a str>,
    connection: Option<LspConnection>,
}

impl<'a> From<&'a LspPositionToolInput> for FileQuery<'a> {
    fn from(input: &'a LspPositionToolInput) -> Self {
        Self {
            path: &input.file_path,
            folder: input.folder.as_deref(),
            server: input.server.as_deref(),
            expected_revision: input.expected_revision.as_deref(),
            connection: None,
        }
    }
}

impl<'a> From<&'a LspDiagnosticsToolInput> for FileQuery<'a> {
    fn from(input: &'a LspDiagnosticsToolInput) -> Self {
        Self {
            path: &input.file_path,
            folder: input.folder.as_deref(),
            server: input.server.as_deref(),
            expected_revision: input.expected_revision.as_deref(),
            connection: None,
        }
    }
}

struct QueryResult<T> {
    path: PathBuf,
    source: LspSource,
    value: T,
}

async fn query_document<T, F>(
    executor: &ToolExecutor,
    input: FileQuery<'_>,
    position: Option<Position>,
    query: impl FnOnce(Arc<LspClient>, Uri, DocumentReceipt) -> F + Send,
) -> Result<QueryResult<T>, ToolError>
where
    T: Send,
    F: std::future::Future<Output = LspResult<T>> + Send,
{
    let operation = async {
        let path = lsp_target(executor, input.path.trim(), input.folder)?;
        let registry = registry(executor)?;
        let document = load_document_text(executor, &path).await?;
        if let Some(position) = position
            && !agena_lsp::valid_position(&document.text, position)
        {
            return Err(ToolError::invalid_input(
                "position is outside the document or splits a UTF-16 character",
            ));
        }
        let revision = document.revision;
        if input
            .expected_revision
            .is_some_and(|expected| !expected.trim().eq_ignore_ascii_case(&revision))
        {
            return Err(ToolError::invalid_input(
                "stale_document: source revision changed",
            ));
        }
        let uri = path_to_uri(&path)?;
        let saved = input.connection.is_some();
        let LspConnection {
            client,
            server: spec,
            ..
        } = match input.connection {
            Some(connection) => connection,
            None => load_connection(executor, registry, &path, input.folder, input.server).await?,
        };
        let language_id = language_id_for_path(&path, &spec);
        let query_client = Arc::clone(&client);
        let saved = saved
            .then(|| client.save_includes_text())
            .flatten()
            .map(|include| include.then(|| document.text.clone()));
        let document_query = client.with_document(
            uri.clone(),
            document.text,
            &language_id,
            |receipt| async move {
                if let Some(text) = saved {
                    query_client.document_saved(uri.clone(), text).await?;
                }
                let version = receipt.version;
                Ok((query(query_client, uri, receipt).await?, version))
            },
        );
        let (value, version) = tokio::time::timeout(QUERY_TIMEOUT, document_query)
            .await
            .map_err(|_| ToolError::plugin("LSP query timed out after 20 seconds"))?
            .map_err(map_lsp_err)?;
        let current = load_document_text(executor, &path).await?;
        if current.revision != revision {
            return Err(ToolError::invalid_input(
                "stale_document: source changed during the LSP query",
            ));
        }
        Ok(QueryResult {
            path,
            value,
            source: LspSource {
                server: spec.name,
                source_revision: revision,
                document_version: Some(version),
                position_encoding: "utf-16".into(),
            },
        })
    };

    if let Some(cancel) = executor.cancellation_token() {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err(ToolError::Cancelled),
            result = operation => result,
        }
    } else {
        operation.await
    }
}

pub(super) async fn execute_start_async(
    executor: &ToolExecutor,
    input: &LspStartToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    lsp_session(executor)?;
    let operation = async {
        let registry = registry(executor)?;
        let path = lsp_target(executor, input.file_path.trim(), input.folder.as_deref())?;
        // Reuse the query's guarded file lookup before authorizing the index root.
        load_document_text(executor, &path).await?;
        connection_for_target(
            executor,
            registry,
            &path,
            input.folder.as_deref(),
            input.server.as_deref(),
            LspClientMode::Start,
        )
        .await?
        .ok_or_else(|| ToolError::plugin("LSP connection unavailable"))
    };
    let connection = if let Some(cancel) = executor.cancellation_token() {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ToolError::Cancelled),
            result = operation => result?,
        }
    } else {
        operation.await?
    };
    let server = connection.server.name;
    let root = executor.display_path(&connection.root);
    let view = ToolExecutionView::simple(
        format!("Start {server}"),
        "Language server running",
        root.clone(),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspStart(LspStartToolOutput { server, root }),
        view,
    ))
}

pub(super) async fn execute_stop_async(
    executor: &ToolExecutor,
    input: &LspStopToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let session = lsp_session(executor)?;
    let server = input.server.as_deref().map(str::trim);
    if server == Some("") {
        return Err(ToolError::invalid_input("server must be nonempty"));
    }
    let stopped_instances = registry(executor)?
        .stop_session_servers(session, server)
        .await
        .map_err(map_lsp_err)?;
    let summary = if stopped_instances == 1 {
        "1 instance stopped".to_owned()
    } else {
        format!("{stopped_instances} instances stopped")
    };
    let view =
        ToolExecutionView::simple("Stop language servers", summary, server.unwrap_or_default());
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspStop(LspStopToolOutput {
            server: server.map(str::to_owned),
            stopped_instances,
        }),
        view,
    ))
}

fn lsp_session(executor: &ToolExecutor) -> Result<i64, ToolError> {
    executor
        .session_id
        .filter(|id| *id > 0)
        .ok_or_else(|| ToolError::invalid_input("language server start and stop require a session"))
}

pub(super) async fn execute_definition_async(
    executor: &ToolExecutor,
    input: &LspDefinitionToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let input = &input.position;
    let position = Position::new(input.line, input.character);
    let response = query_document(
        executor,
        input.into(),
        Some(position),
        |client, uri, receipt| async move {
            client
                .definition_after_sync(uri, position, receipt.status)
                .await
        },
    )
    .await?;
    let result = definition_locations(executor, response);
    Ok(locations_execution(
        "definition",
        result,
        ToolPayloadOutput::LspDefinition,
    ))
}

pub(super) async fn execute_implementation_async(
    executor: &ToolExecutor,
    input: &LspPositionToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let position = Position::new(input.line, input.character);
    let response = query_document(
        executor,
        input.into(),
        Some(position),
        |client, uri, receipt| async move {
            client
                .implementation_after_sync(uri, position, receipt.status)
                .await
        },
    )
    .await?;
    let result = definition_locations(executor, response);
    Ok(locations_execution(
        "implementation",
        result,
        ToolPayloadOutput::LspImplementation,
    ))
}

pub(super) async fn execute_references_async(
    executor: &ToolExecutor,
    input: &LspReferencesToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let position = Position::new(input.position.line, input.position.character);
    let response = query_document(
        executor,
        (&input.position).into(),
        Some(position),
        |client, uri, receipt| async move {
            client
                .references_after_sync(uri, position, input.include_declaration, receipt.status)
                .await
        },
    )
    .await?;
    let locations = response.value.unwrap_or_default();
    let total = locations.len();
    let result = bounded_locations(
        response.source,
        total,
        locations
            .into_iter()
            .map(|location| target_for_location(executor, location)),
    );
    Ok(locations_execution(
        "reference",
        result,
        ToolPayloadOutput::LspReferences,
    ))
}

pub(super) async fn execute_hover_async(
    executor: &ToolExecutor,
    input: &LspHoverToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let input = &input.position;
    let position = Position::new(input.line, input.character);
    let response = query_document(
        executor,
        input.into(),
        Some(position),
        |client, uri, receipt| async move {
            client.hover_after_sync(uri, position, receipt.status).await
        },
    )
    .await?;
    let contents = response.value.as_ref().map(hover_text);
    let truncated = contents.as_ref().is_some_and(|text| text.len() > 6 * 1024);
    let result = LspHoverResult {
        contents: contents.map(|text| preview(&text, 6 * 1024).to_owned()),
        range: response.value.and_then(|hover| hover.range),
        source: response.source,
        truncated,
    };
    let view = ToolExecutionView::simple(
        format!(
            "LSP hover {}",
            position_label(&executor.display_path(&response.path), position)
        ),
        if result.contents.is_some() {
            "Hover information"
        } else {
            "No hover information"
        },
        result.contents.clone().unwrap_or_default(),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspHover(result),
        view,
    ))
}

pub(super) async fn execute_diagnostics_async(
    executor: &ToolExecutor,
    input: &LspDiagnosticsToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let report = diagnostics_for(executor, input, MAX_RESULT_BYTES, None).await?;
    let view = ToolExecutionView::simple(
        format!("LSP diagnostics {}", report.file),
        format!(
            "{} diagnostics · {}",
            report.total_diagnostics, report.state
        ),
        report.entries.join("\n"),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspDiagnostics(report),
        view,
    ))
}

pub(super) async fn diagnostics_for(
    executor: &ToolExecutor,
    input: &LspDiagnosticsToolInput,
    budget: usize,
    connection: Option<LspConnection>,
) -> Result<LspDiagnosticResult, ToolError> {
    let response = query_document(
        executor,
        FileQuery {
            connection,
            ..input.into()
        },
        None,
        |client, uri, receipt| async move {
            client
                .document_diagnostics(&uri, receipt, PUSH_DIAGNOSTIC_WAIT)
                .await
        },
    )
    .await?;
    Ok(LspDiagnosticResult::from_report(
        executor,
        &response.path,
        response.value,
        response.source,
        budget,
    ))
}

pub(super) async fn execute_diagnostics_many_async(
    executor: &ToolExecutor,
    input: &LspDiagnosticsManyToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    validate_diagnostic_paths(&input.file_paths)?;
    let budget = MAX_RESULT_BYTES / input.file_paths.len();
    let deadline = tokio::time::Instant::now() + QUERY_TIMEOUT;
    let files: Vec<_> =
        futures_util::stream::iter(input.file_paths.iter().cloned().map(|path| async move {
            let file = LspDiagnosticsToolInput {
                file_path: path.clone(),
                folder: input.folder.clone(),
                server: input.server.clone(),
                expected_revision: None,
            };
            match tokio::time::timeout_at(deadline, diagnostics_for(executor, &file, budget, None))
                .await
            {
                Ok(Ok(report)) => report,
                Ok(Err(error)) => LspDiagnosticResult {
                    file: path.clone(),
                    state: "error".into(),
                    error: Some(preview(&error.to_string(), 512).into()),
                    ..Default::default()
                },
                Err(_) => LspDiagnosticResult {
                    file: path.clone(),
                    state: "timeout".into(),
                    ..Default::default()
                },
            }
        }))
        .buffered(4)
        .collect()
        .await;
    executor.ensure_not_cancelled()?;
    let truncated = files.iter().any(|file| file.truncated);
    let view = ToolExecutionView::simple(
        "LSP diagnostics",
        format!("{} files checked", files.len()),
        files
            .iter()
            .map(|file| {
                format!(
                    "{} · {} · {} diagnostics",
                    file.file, file.state, file.total_diagnostics
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspDiagnosticsMany { files, truncated },
        view,
    ))
}

pub(super) fn validate_diagnostic_paths(paths: &[String]) -> Result<(), ToolError> {
    if paths.is_empty()
        || paths.len() > MAX_DIAGNOSTIC_FILES
        || paths
            .iter()
            .any(|path| path.trim().is_empty() || path.len() > 4096)
    {
        return Err(ToolError::invalid_input(
            "file_paths must contain 1 to 16 nonempty paths of at most 4096 bytes",
        ));
    }
    Ok(())
}

pub(super) async fn execute_symbols_async(
    executor: &ToolExecutor,
    input: &LspSymbolsToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let query = input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty());
    if query.is_some_and(|query| query.len() > 1024) {
        return Err(ToolError::invalid_input("symbol query exceeds 1024 bytes"));
    }
    let response = query_document(
        executor,
        (&input.file).into(),
        None,
        |client, uri, receipt| async move {
            if let Some(query) = query {
                let symbols = client
                    .workspace_symbols(query.into(), receipt.status)
                    .await?;
                Ok(workspace_symbols(executor, symbols))
            } else {
                let symbols = client.document_symbols(uri.clone()).await?;
                Ok(document_symbols(executor, &uri, symbols))
            }
        },
    )
    .await?;
    let SymbolResults {
        items: symbols,
        total: total_symbols,
        ..
    } = response.value;
    let truncated = symbols.len() < total_symbols;
    let view = ToolExecutionView::simple(
        "LSP symbols",
        format!("{} of {total_symbols} symbols", symbols.len()),
        symbols
            .iter()
            .map(|symbol| format!("{} · {}", symbol.name, symbol.target.label()))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspSymbols {
            source: response.source,
            symbols,
            total_symbols,
            truncated,
        },
        view,
    ))
}

fn definition_locations(
    executor: &ToolExecutor,
    response: QueryResult<Option<GotoDefinitionResponse>>,
) -> LspLocationsResult {
    let mut targets = Vec::new();
    let total = match response.value {
        Some(GotoDefinitionResponse::Scalar(location)) => {
            targets.push(target_for_location(executor, location));
            1
        }
        Some(GotoDefinitionResponse::Array(locations)) => {
            let total = locations.len();
            targets.extend(
                locations
                    .into_iter()
                    .take(MAX_RESULTS)
                    .map(|location| target_for_location(executor, location)),
            );
            total
        }
        Some(GotoDefinitionResponse::Link(links)) => {
            let total = links.len();
            targets.extend(links.into_iter().take(MAX_RESULTS).map(|link| {
                LspTarget::new(
                    executor,
                    &link.target_uri,
                    Some(link.target_range),
                    Some(link.target_selection_range),
                )
            }));
            total
        }
        None => 0,
    };
    bounded_locations(response.source, total, targets)
}

fn target_for_location(executor: &ToolExecutor, location: Location) -> LspTarget {
    LspTarget::new(executor, &location.uri, Some(location.range), None)
}

fn bounded_locations(
    source: LspSource,
    total: usize,
    targets: impl IntoIterator<Item = LspTarget>,
) -> LspLocationsResult {
    let mut result = LspLocationsResult {
        source,
        total_locations: total,
        ..Default::default()
    };
    let mut remaining = MAX_RESULT_BYTES;
    for target in targets.into_iter().take(MAX_RESULTS) {
        let label = target.label();
        if !take_budget(&mut remaining, &(&target, &label)) {
            break;
        }
        result.locations.push(label);
        result.targets.push(target);
    }
    result.truncated = result.targets.len() < total;
    result
}

fn locations_execution(
    kind: &str,
    result: LspLocationsResult,
    output: impl FnOnce(LspLocationsResult) -> ToolPayloadOutput,
) -> ToolPayloadExecution {
    let view = ToolExecutionView::simple(
        format!("LSP {kind}"),
        format!(
            "{} of {} locations",
            result.targets.len(),
            result.total_locations
        ),
        result.locations.join("\n"),
    );
    ToolPayloadExecution::new(output(result), view)
}

fn flat_symbol(executor: &ToolExecutor, symbol: SymbolInformation) -> LspSymbol {
    LspSymbol {
        name: symbol.name,
        kind: symbol.kind,
        container: symbol.container_name,
        target: target_for_location(executor, symbol.location),
    }
}

struct SymbolResults {
    items: Vec<LspSymbol>,
    total: usize,
    remaining: usize,
}

impl SymbolResults {
    fn new() -> Self {
        Self {
            items: Vec::new(),
            total: 0,
            remaining: MAX_RESULT_BYTES,
        }
    }

    fn push(&mut self, symbol: impl FnOnce() -> LspSymbol) {
        self.total += 1;
        if self.items.len() == MAX_RESULTS || self.remaining == 0 {
            return;
        }
        let item = symbol();
        if take_budget(&mut self.remaining, &item) {
            self.items.push(item);
        } else {
            self.remaining = 0;
        }
    }
}

fn document_symbols(
    executor: &ToolExecutor,
    uri: &Uri,
    result: Option<DocumentSymbolResponse>,
) -> SymbolResults {
    let mut output = SymbolResults::new();
    match result {
        Some(DocumentSymbolResponse::Flat(symbols)) => {
            for symbol in symbols {
                output.push(|| flat_symbol(executor, symbol));
            }
        }
        Some(DocumentSymbolResponse::Nested(symbols)) => {
            let mut stack: Vec<(&DocumentSymbol, Option<&str>)> =
                symbols.iter().rev().map(|symbol| (symbol, None)).collect();
            while let Some((symbol, container)) = stack.pop() {
                if let Some(children) = &symbol.children {
                    stack.extend(
                        children
                            .iter()
                            .rev()
                            .map(|child| (child, Some(symbol.name.as_str()))),
                    );
                }
                output.push(|| LspSymbol {
                    name: symbol.name.clone(),
                    kind: symbol.kind,
                    container: container.map(str::to_owned),
                    target: LspTarget::new(
                        executor,
                        uri,
                        Some(symbol.range),
                        Some(symbol.selection_range),
                    ),
                });
            }
        }
        None => {}
    }
    output
}

fn workspace_symbols(
    executor: &ToolExecutor,
    result: Option<WorkspaceSymbolResponse>,
) -> SymbolResults {
    let mut output = SymbolResults::new();
    match result {
        Some(WorkspaceSymbolResponse::Flat(symbols)) => {
            for symbol in symbols {
                output.push(|| flat_symbol(executor, symbol));
            }
        }
        Some(WorkspaceSymbolResponse::Nested(symbols)) => {
            for symbol in symbols {
                output.push(|| {
                    let target = match symbol.location {
                        OneOf::Left(location) => target_for_location(executor, location),
                        OneOf::Right(location) => {
                            LspTarget::new(executor, &location.uri, None, None)
                        }
                    };
                    LspSymbol {
                        name: symbol.name,
                        kind: symbol.kind,
                        container: symbol.container_name,
                        target,
                    }
                });
            }
        }
        None => {}
    }
    output
}

fn hover_text(hover: &Hover) -> String {
    let marked = |value: &MarkedString| match value {
        MarkedString::String(text) => text.clone(),
        MarkedString::LanguageString(code) => format!("```{}\n{}\n```", code.language, code.value),
    };
    match &hover.contents {
        HoverContents::Scalar(value) => marked(value),
        HoverContents::Array(values) => values.iter().map(marked).collect::<Vec<_>>().join("\n\n"),
        HoverContents::Markup(markup) => markup.value.clone(),
    }
}

fn language_id_for_path(path: &Path, spec: &agena_lsp::LspServerSpec) -> String {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if let Some(language) = spec.language_ids.get(&ext) {
        return language.clone();
    }
    match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "sh" | "bash" => "shellscript",
        "" => "plaintext",
        other => other,
    }
    .to_owned()
}

pub(super) fn lsp_target(
    executor: &ToolExecutor,
    raw: &str,
    folder: Option<&str>,
) -> Result<std::path::PathBuf, ToolError> {
    let relative_in_folder =
        executor.is_global() && folder.is_some_and(|folder| Path::new(folder.trim()).is_absolute());
    if executor.is_global() && !Path::new(raw).is_absolute() && !relative_in_folder {
        return Err(ToolError::invalid_input(
            "global sessions require an explicit full absolute path",
        ));
    }
    Ok(executor.resolve_target_path_in_folder(raw, folder))
}

#[cfg(test)]
pub(super) async fn client_for_target(
    executor: &ToolExecutor,
    registry: &agena_lsp::LspRegistry,
    path: &Path,
) -> Result<std::sync::Arc<agena_lsp::LspClient>, ToolError> {
    load_connection(executor, registry, path, None, None)
        .await
        .map(|connection| connection.client)
}

async fn load_connection(
    executor: &ToolExecutor,
    registry: &agena_lsp::LspRegistry,
    path: &Path,
    folder: Option<&str>,
    server: Option<&str>,
) -> Result<LspConnection, ToolError> {
    connection_for_target(
        executor,
        registry,
        path,
        folder,
        server,
        LspClientMode::Query,
    )
    .await?
    .ok_or_else(|| ToolError::plugin("LSP connection unavailable"))
}

pub(super) async fn connection_for_target(
    executor: &ToolExecutor,
    registry: &agena_lsp::LspRegistry,
    path: &Path,
    folder: Option<&str>,
    server: Option<&str>,
    mode: LspClientMode,
) -> Result<Option<LspConnection>, ToolError> {
    let scoped = executor.clone();
    let policy: agena_lsp::LspScopedSandboxBuilder = std::sync::Arc::new(move |root| {
        // A language server indexes its project root, not just the selected
        // file. Authorize the resolved root before every cache lookup so a
        // file grant cannot silently become a grant for its parent project.
        let access = scoped.read_path_filter();
        access.predicate().allows(root);
        access
            .finish(Ok(()))
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::PermissionDenied, error))?;
        let mut sandbox =
            crate::shell_sandbox::policy_for_paths(&scoped, &[root.to_path_buf()], &[], &[])
                .map_err(|error| {
                    std::io::Error::new(std::io::ErrorKind::PermissionDenied, error)
                })?;
        sandbox.private_build_cache = true;
        Ok((sandbox, lsp_cache_scope(&scoped, root)))
    });
    let folder = folder
        .filter(|folder| Path::new(folder.trim()).is_absolute())
        .map(|folder| executor.resolve_folder_base(Some(folder)));
    let workspace = if executor.is_global() {
        folder.as_deref()
    } else {
        Some(executor.workspace_root())
    };
    registry
        .connection_for_path(path, workspace, server.map(str::trim), policy, mode)
        .await
        .map_err(map_lsp_err)
}

fn lsp_cache_scope(executor: &ToolExecutor, root: &Path) -> Option<String> {
    // A once-approved Ask/Auto root cannot leave a background reader alive.
    // Persistent Allow is still isolated by the owning session and full policy.
    if matches!(
        executor
            .sandbox_permission_snapshot()
            .authorize_path_access(agena_domain::AccessKind::Read, root,),
        agena_domain::PermissionDecision::Allow
    ) {
        executor
            .session_id
            .map(|session| format!("session:{session}"))
    } else {
        None
    }
}

fn registry(executor: &ToolExecutor) -> Result<&std::sync::Arc<agena_lsp::LspRegistry>, ToolError> {
    executor
        .lsp_registry()
        .ok_or_else(|| ToolError::plugin("no LSP servers configured".to_string()))
}

fn path_to_uri(path: &Path) -> Result<Uri, ToolError> {
    let url = url::Url::from_file_path(path).map_err(|_| {
        ToolError::invalid_input(format!("file path not absolute: {}", path.display()))
    })?;
    url.as_str()
        .parse::<Uri>()
        .map_err(|err| ToolError::invalid_input(format!("invalid uri: {err}")))
}

fn map_lsp_err(err: agena_lsp::LspError) -> ToolError {
    use agena_lsp::LspError;
    match err {
        LspError::MessageTooLarge { .. } => ToolError::invalid_input(err.to_string()),
        LspError::UnknownServer(name) => {
            ToolError::plugin(format!("no LSP server matches `{name}`"))
        }
        LspError::Server { code, message } => {
            ToolError::plugin(format!("lsp server error {code}: {message}"))
        }
        LspError::Io(error) => {
            let diagnostic = error.to_string();
            error
                .into_inner()
                .and_then(|error| error.downcast::<ToolError>().ok())
                .map(|error| *error)
                .unwrap_or_else(|| ToolError::plugin(diagnostic))
        }
        other => ToolError::plugin(other.to_string()),
    }
}

#[derive(Debug)]
struct DocumentText {
    text: String,
    revision: String,
}

async fn load_document_text(
    executor: &ToolExecutor,
    path: &Path,
) -> Result<DocumentText, ToolError> {
    let executor = executor.clone();
    let path = path.to_owned();
    LSP_DOCUMENT_READERS
        .run(move || {
            // Reuse the invocation's concrete-path authorizer. In particular,
            // permission to index a root does not override a denied child.
            // Permission callbacks and filesystem work stay off Tokio workers.
            let access = executor.read_path_filter();
            access.predicate().allows(&path);
            access.finish(Ok(()))?;
            let bytes = crate::read_regular_file_nofollow(&path, MAX_LSP_DOCUMENT_BYTES)?;
            executor.ensure_not_cancelled()?;
            let revision = hex::encode(Sha256::digest(&bytes));
            let text = String::from_utf8(bytes).map_err(|_| {
                ToolError::plugin(format!("document is not UTF-8 text: {}", path.display()))
            })?;
            Ok(DocumentText { text, revision })
        })
        .await
        .map_err(|error| ToolError::plugin(format!("LSP document reader failed: {error}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::ExecutionPrincipal;
    use crate::permission::{PermissionPolicy, ToolPermissionPolicy};
    use std::sync::Arc;

    fn executor(root: &Path) -> ToolExecutor {
        ToolExecutor::new(
            root.to_owned(),
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        )
    }

    #[test]
    fn sandbox_lsp_cache_requires_persistent_allow_and_session_identity() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let mut allowed = executor(&root);
        assert_eq!(lsp_cache_scope(&allowed, &root), None);
        allowed.session_id = Some(42);
        assert_eq!(
            lsp_cache_scope(&allowed, &root).as_deref(),
            Some("session:42")
        );
        // One-call approval is transient authority, not a persisted path mode.
        allowed.principal = ExecutionPrincipal::new(
            PermissionPolicy::new(
                agena_domain::PermissionMode::Ask,
                agena_domain::PermissionMode::Allow,
            ),
            ToolPermissionPolicy::allow_all(),
        );
        assert_eq!(lsp_cache_scope(&allowed, &root), None);
    }

    #[tokio::test]
    async fn lsp_document_reads_only_the_authorized_target() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let target = root.join("source.rs");
        let text = "fn example() { /* 文档 */ }\n";
        std::fs::write(&target, text).unwrap();
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let recorded = seen.clone();
        let executor = executor(&root).with_read_path_authorizer(Arc::new(move |path| {
            recorded.lock().push(path.to_owned());
            Ok(())
        }));

        assert_eq!(
            load_document_text(&executor, &target).await.unwrap().text,
            text
        );
        assert_eq!(*seen.lock(), vec![target]);
    }

    #[tokio::test]
    async fn lsp_document_preserves_a_concrete_child_denial() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let target = root.join("denied.rs");
        std::fs::write(&target, b"private document").unwrap();
        let expected = target.clone();
        let executor = executor(&root).with_read_path_authorizer(Arc::new(move |path| {
            assert_eq!(path, expected);
            Err(ToolError::invalid_input("document permission denied"))
        }));

        let error = load_document_text(&executor, &target).await.unwrap_err();
        assert!(matches!(error, ToolError::InvalidInput { .. }));
        assert!(error.to_string().contains("document permission denied"));
    }

    #[tokio::test]
    async fn lsp_document_keeps_size_and_utf8_checks() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let target = root.join("source.rs");
        std::fs::File::create(&target)
            .unwrap()
            .set_len(MAX_LSP_DOCUMENT_BYTES as u64 + 1)
            .unwrap();
        let executor = executor(&root);
        assert!(load_document_text(&executor, &target).await.is_err());
        std::fs::write(&target, [0xff]).unwrap();
        assert!(
            load_document_text(&executor, &target)
                .await
                .unwrap_err()
                .to_string()
                .contains("not UTF-8")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lsp_document_accepts_an_authorized_canonical_symlink_target() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let target = root.join("source.rs");
        let alias = root.join("alias.rs");
        std::fs::write(&target, b"fn original() {}\n").unwrap();
        symlink(&target, &alias).unwrap();
        let expected = target.clone();
        let executor = executor(&root).with_read_path_authorizer(Arc::new(move |path| {
            assert_eq!(path, expected);
            Ok(())
        }));
        let resolved = lsp_target(&executor, alias.to_str().unwrap(), None).unwrap();
        assert_eq!(resolved, target);
        assert_eq!(
            load_document_text(&executor, &resolved).await.unwrap().text,
            "fn original() {}\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lsp_document_rejects_an_ancestor_swap_after_authorization() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let allowed = root.join("allowed");
        let outside = root.join("outside");
        let retained = root.join("retained");
        std::fs::create_dir(&allowed).unwrap();
        std::fs::create_dir(&outside).unwrap();
        let target = allowed.join("source.rs");
        let secret = outside.join("source.rs");
        std::fs::write(&target, b"fn allowed() {}\n").unwrap();
        std::fs::write(&secret, b"private document").unwrap();
        let expected = target.clone();
        let executor = executor(&root).with_read_path_authorizer(Arc::new(move |path| {
            assert_eq!(path, expected);
            std::fs::rename(&allowed, &retained).unwrap();
            symlink(&outside, &allowed).unwrap();
            Ok(())
        }));

        assert!(load_document_text(&executor, &target).await.is_err());
        assert_eq!(std::fs::read(secret).unwrap(), b"private document");
    }
}
