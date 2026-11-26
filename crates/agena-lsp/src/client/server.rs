//! Capabilities and the read-only requests that servers can send to the client.

use super::*;
use lsp_types::{
    ClientCapabilities, ConfigurationParams, DocumentSymbolClientCapabilities,
    GeneralClientCapabilities, GotoCapability, HoverClientCapabilities, HoverProviderCapability,
    ImplementationProviderCapability, InitializeParams, MarkupKind, OneOf, PositionEncodingKind,
    PublishDiagnosticsClientCapabilities, TextDocumentClientCapabilities,
    WorkspaceClientCapabilities, WorkspaceFolder,
};

pub(super) struct Session {
    folders: Option<Vec<WorkspaceFolder>>,
    configuration: Value,
}

impl LspClient {
    /// Initialize with a single workspace folder and default server settings.
    pub async fn initialize(
        &self,
        root_uri: Option<Uri>,
        client_name: &str,
        client_version: &str,
        initialization_options: Option<Value>,
    ) -> LspResult<InitializeResult> {
        self.initialize_with_configuration(
            root_uri,
            client_name,
            client_version,
            initialization_options,
            Value::Null,
        )
        .await
    }

    /// Initialize and answer workspace configuration queries from the configured settings.
    pub async fn initialize_with_configuration(
        &self,
        root_uri: Option<Uri>,
        client_name: &str,
        client_version: &str,
        initialization_options: Option<Value>,
        configuration: Value,
    ) -> LspResult<InitializeResult> {
        let folders = root_uri.clone().map(|uri| {
            vec![WorkspaceFolder {
                uri,
                name: "workspace".into(),
            }]
        });
        self.inner
            .session
            .set(Session {
                folders: folders.clone(),
                configuration,
            })
            .map_err(|_| LspError::Protocol("LSP connection already initialized".into()))?;
        #[allow(deprecated)]
        let params = InitializeParams {
            process_id: Some(std::process::id()),
            root_path: None,
            root_uri,
            initialization_options,
            capabilities: capabilities(),
            trace: None,
            workspace_folders: folders,
            client_info: Some(lsp_types::ClientInfo {
                name: client_name.to_owned(),
                version: Some(client_version.to_owned()),
            }),
            locale: None,
            work_done_progress_params: WorkDoneProgressParams::default(),
        };
        let result: InitializeResult = self.request("initialize", params).await?;
        if let Some(encoding) = result.capabilities.position_encoding.as_ref()
            && encoding != &PositionEncodingKind::UTF16
        {
            return Err(LspError::Protocol(format!(
                "language server selected position encoding {}; this client supports utf-16",
                encoding.as_str()
            )));
        }
        self.notify("initialized", serde_json::json!({})).await?;
        self.inner.initialized.store(Some(Arc::new(result.clone())));
        Ok(result)
    }

    pub fn supports(&self, method: &str) -> bool {
        let Some(server) = self.server_info() else {
            return false;
        };
        let caps = &server.capabilities;
        match method {
            "textDocument/definition" => supported(&caps.definition_provider),
            "textDocument/references" => supported(&caps.references_provider),
            "textDocument/implementation" => matches!(
                caps.implementation_provider,
                Some(
                    ImplementationProviderCapability::Simple(true)
                        | ImplementationProviderCapability::Options(_)
                )
            ),
            "textDocument/hover" => matches!(
                caps.hover_provider,
                Some(HoverProviderCapability::Simple(true) | HoverProviderCapability::Options(_))
            ),
            "textDocument/documentSymbol" => supported(&caps.document_symbol_provider),
            "workspace/symbol" => supported(&caps.workspace_symbol_provider),
            "textDocument/diagnostic" => caps.diagnostic_provider.is_some(),
            _ => false,
        }
    }

    pub(super) async fn request_feature<P, R>(
        &self,
        method: &'static str,
        params: P,
    ) -> LspResult<Option<R>>
    where
        P: serde::Serialize,
        R: DeserializeOwned,
    {
        self.ensure_alive()?;
        if !self.supports(method) {
            return Err(LspError::Unsupported(method));
        }
        self.request_opt(method, params).await
    }
}

fn supported<T>(capability: &Option<OneOf<bool, T>>) -> bool {
    matches!(capability, Some(OneOf::Left(true) | OneOf::Right(_)))
}

fn capabilities() -> ClientCapabilities {
    let links = GotoCapability {
        dynamic_registration: Some(false),
        link_support: Some(true),
    };
    ClientCapabilities {
        general: Some(GeneralClientCapabilities {
            position_encodings: Some(vec![PositionEncodingKind::UTF16]),
            ..Default::default()
        }),
        workspace: Some(WorkspaceClientCapabilities {
            configuration: Some(true),
            workspace_folders: Some(true),
            ..Default::default()
        }),
        text_document: Some(TextDocumentClientCapabilities {
            definition: Some(links),
            implementation: Some(links),
            hover: Some(HoverClientCapabilities {
                dynamic_registration: Some(false),
                content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
            }),
            document_symbol: Some(DocumentSymbolClientCapabilities {
                hierarchical_document_symbol_support: Some(true),
                ..Default::default()
            }),
            publish_diagnostics: Some(PublishDiagnosticsClientCapabilities {
                version_support: Some(true),
                ..Default::default()
            }),
            diagnostic: Some(lsp_types::DiagnosticClientCapabilities::default()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub(super) fn answer_request(
    session: Option<&Session>,
    request: JsonRpcRequest,
) -> JsonRpcResponse {
    let result = match request.method.as_str() {
        "workspace/configuration" => {
            let configuration = session
                .map(|session| &session.configuration)
                .unwrap_or(&Value::Null);
            serde_json::from_value::<ConfigurationParams>(request.params.unwrap_or(Value::Null))
                .map(|params| {
                    Value::Array(
                        params
                            .items
                            .into_iter()
                            .map(|item| {
                                let section = item.section.as_deref().unwrap_or_default();
                                if section.is_empty() {
                                    return configuration.clone();
                                }
                                configuration
                                    .get(section)
                                    .or_else(|| {
                                        section
                                            .split('.')
                                            .try_fold(configuration, |value, key| value.get(key))
                                    })
                                    .cloned()
                                    .unwrap_or(Value::Null)
                            })
                            .collect(),
                    )
                })
                .map_err(|error| crate::protocol::JsonRpcError {
                    code: -32602,
                    message: error.to_string(),
                    data: None,
                })
        }
        "workspace/workspaceFolders" => Ok(serde_json::json!(
            session.and_then(|session| session.folders.as_ref())
        )),
        "window/workDoneProgress/create" | "workspace/diagnostic/refresh" => Ok(Value::Null),
        _ => Err(crate::protocol::JsonRpcError {
            code: -32601,
            message: format!("method '{}' not handled by client", request.method),
            data: None,
        }),
    };
    let (result, error) = match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    };
    JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION.into(),
        id: request.id,
        result,
        error,
    }
}
