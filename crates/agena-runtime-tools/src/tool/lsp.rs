//! `lsp_definition` / `lsp_references` / `lsp_hover` / `lsp_diagnostics`
//! plugin tools.
//!
//! Each tool resolves the right server via the [`agena_lsp::LspRegistry`]
//! threaded through `ToolExecutor::with_lsp_registry`, awaits the LSP request
//! directly on the caller's Tokio runtime, and returns a flattened
//! `path:line:col` representation that the LLM can paste back to the user
//! verbatim.

use std::path::Path;

use agena_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, GotoDefinitionResponse, Hover, HoverContents, Location,
    MarkedString, Position, Range, Uri,
};

use crate::part::{
    LspDefinitionToolInput, LspDiagnosticsToolInput, LspHoverToolInput, LspReferencesToolInput,
};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

const MAX_LSP_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
static LSP_DOCUMENT_READERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

pub(super) async fn execute_definition_async(
    executor: &ToolExecutor,
    input: &LspDefinitionToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let path = lsp_target(
        executor,
        &input.position.file_path,
        input.position.folder.as_deref(),
    )?;
    let registry = registry(executor)?;
    let uri = path_to_uri(&path)?;
    let pos = Position::new(input.position.line, input.position.character);

    let client = client_for_target(executor, registry, &path).await?;
    let sync_status = sync_document(executor, &client, &path, &uri).await?.status;
    let response = client
        .definition_after_sync(uri, pos, sync_status)
        .await
        .map_err(map_lsp_err)?;

    let locations = response.map(format_definition_response).unwrap_or_default();

    let view = ToolExecutionView::simple(
        format!(
            "lsp_definition {}:{}:{}",
            display_path(&path, executor),
            input.position.line + 1,
            input.position.character + 1
        ),
        format!("{} definitions", locations.len()),
        if locations.is_empty() {
            "no definition found".to_string()
        } else {
            locations.join("\n")
        },
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspDefinition { locations },
        view,
    ))
}

pub(super) async fn execute_references_async(
    executor: &ToolExecutor,
    input: &LspReferencesToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let path = lsp_target(
        executor,
        &input.position.file_path,
        input.position.folder.as_deref(),
    )?;
    let registry = registry(executor)?;
    let uri = path_to_uri(&path)?;
    let pos = Position::new(input.position.line, input.position.character);
    let include_definition = input.include_declaration;

    let client = client_for_target(executor, registry, &path).await?;
    let sync_status = sync_document(executor, &client, &path, &uri).await?.status;
    let response = client
        .references_after_sync(uri, pos, include_definition, sync_status)
        .await
        .map_err(map_lsp_err)?;

    let locations = response
        .map(|locs| locs.iter().map(format_location).collect::<Vec<_>>())
        .unwrap_or_default();

    let view = ToolExecutionView::simple(
        format!(
            "lsp_references {}:{}:{}",
            display_path(&path, executor),
            input.position.line + 1,
            input.position.character + 1
        ),
        format!("{} references", locations.len()),
        if locations.is_empty() {
            "no references found".to_string()
        } else {
            locations.join("\n")
        },
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspReferences { locations },
        view,
    ))
}

pub(super) async fn execute_hover_async(
    executor: &ToolExecutor,
    input: &LspHoverToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let path = lsp_target(
        executor,
        &input.position.file_path,
        input.position.folder.as_deref(),
    )?;
    let registry = registry(executor)?;
    let uri = path_to_uri(&path)?;
    let pos = Position::new(input.position.line, input.position.character);

    let client = client_for_target(executor, registry, &path).await?;
    let sync_status = sync_document(executor, &client, &path, &uri).await?.status;
    let response = client
        .hover_after_sync(uri, pos, sync_status)
        .await
        .map_err(map_lsp_err)?;

    let contents = response.as_ref().map(format_hover);

    let view = ToolExecutionView::simple(
        format!(
            "lsp_hover {}:{}:{}",
            display_path(&path, executor),
            input.position.line + 1,
            input.position.character + 1
        ),
        if contents.is_some() {
            "Hover information returned"
        } else {
            "No hover information"
        },
        contents
            .clone()
            .unwrap_or_else(|| "no hover info".to_string()),
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspHover { contents },
        view,
    ))
}

pub(super) async fn execute_diagnostics_async(
    executor: &ToolExecutor,
    input: &LspDiagnosticsToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let path = lsp_target(executor, &input.file_path, input.folder.as_deref())?;
    let registry = registry(executor)?;
    let uri = path_to_uri(&path)?;

    // Make sure the language server has been spawned (it might not have
    // pushed diagnostics yet if no other tool touched this file).
    let client = client_for_target(executor, registry, &path).await?;
    let receipt = sync_document(executor, &client, &path, &uri).await?;
    let waiting = client.wait_diagnostics(
        &uri,
        receipt.version,
        std::time::Duration::from_millis(2000),
    );
    let report = if let Some(cancel) = executor.cancellation_token() {
        tokio::select! {biased; _=cancel.cancelled()=>return Err(ToolError::Cancelled), report=waiting=>report}
    } else {
        waiting.await
    };
    let entries = report.diagnostics;

    let formatted: Vec<String> = entries
        .iter()
        .map(|d| format_diagnostic(&path, executor, d))
        .collect();

    let view = ToolExecutionView::simple(
        format!("lsp_diagnostics {}", display_path(&path, executor)),
        format!(
            "{} diagnostics · {} · version {}",
            formatted.len(),
            report.state,
            report.expected_version
        ),
        if report.state != "current" {
            format!(
                "Diagnostic evidence is {} for document version {} (reported {:?}); an empty result does not certify clean code.\n{}",
                report.state,
                report.expected_version,
                report.reported_version,
                formatted.join("\n")
            )
        } else if formatted.is_empty() {
            format!(
                "No diagnostics for synchronized document version {}.",
                report.expected_version
            )
        } else {
            formatted.join("\n")
        },
    );
    Ok(ToolPayloadExecution::new(
        ToolPayloadOutput::LspDiagnostics {
            entries: formatted,
            state: report.state.into(),
            document_version: Some(report.expected_version),
            reported_version: report.reported_version,
        },
        view,
    ))
}

pub(super) fn lsp_target(
    executor: &ToolExecutor,
    raw: &str,
    folder: Option<&str>,
) -> Result<std::path::PathBuf, ToolError> {
    let relative_in_folder =
        executor.is_global() && folder.is_some_and(|folder| !folder.trim().is_empty());
    if executor.is_global() && !Path::new(raw).is_absolute() && !relative_in_folder {
        return Err(ToolError::invalid_input(
            "global sessions require an explicit full absolute path",
        ));
    }
    Ok(executor.resolve_target_path_in_folder(raw, folder))
}

pub(super) async fn client_for_target(
    executor: &ToolExecutor,
    registry: &agena_lsp::LspRegistry,
    path: &Path,
) -> Result<std::sync::Arc<agena_lsp::LspClient>, ToolError> {
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
    registry
        .client_for_path_scoped(
            path,
            (!executor.is_global()).then_some(executor.workspace_root()),
            policy,
        )
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
    url::Url::parse(uri.as_str())
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| uri.to_string())
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

async fn load_document_text(executor: &ToolExecutor, path: &Path) -> Result<String, ToolError> {
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
            String::from_utf8(bytes).map_err(|_| {
                ToolError::plugin(format!("document is not UTF-8 text: {}", path.display()))
            })
        })
        .await
        .map_err(|error| ToolError::plugin(format!("LSP document reader failed: {error}")))?
}

async fn sync_document(
    executor: &ToolExecutor,
    client: &agena_lsp::LspClient,
    path: &Path,
    uri: &Uri,
) -> Result<agena_lsp::DocumentReceipt, ToolError> {
    let text = load_document_text(executor, path).await?;
    let language_id = language_id_for_path(path);
    client
        .sync_document_with_receipt(uri.clone(), text, &language_id)
        .await
        .map_err(map_lsp_err)
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

        assert_eq!(load_document_text(&executor, &target).await.unwrap(), text);
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
            load_document_text(&executor, &resolved).await.unwrap(),
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
