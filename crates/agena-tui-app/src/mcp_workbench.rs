//! External MCP connection management using the server-owned management API.

mod input;
mod render;
mod state;

pub(crate) use state::McpWorkbench;

use agena_api::mcp_connections::{
    McpConnectionCatalog, McpConnectionDetail, McpConnectionScope, McpImportResult,
    McpMutationResult, McpTestResult,
};
use serde_json::Value;
use std::sync::Arc;

use crate::{App, Route};
use state::{McpImport, McpPane, PendingAuthorization};

impl App {
    pub(crate) fn open_mcp_workbench(&mut self) {
        let mut dialog = McpWorkbench::new();
        self.request_mcp_catalog(&mut dialog, None);
        self.current_route = Route::McpWorkbench(Box::new(dialog));
    }

    fn with_mcp_workbench(
        &mut self,
        instance: u64,
        update: impl FnOnce(&mut Self, &mut McpWorkbench),
    ) {
        let route = std::mem::replace(&mut self.current_route, Route::Main);
        match route {
            Route::McpWorkbench(mut dialog) => {
                if dialog.instance == instance {
                    update(self, &mut dialog);
                }
                self.current_route = Route::McpWorkbench(dialog);
            }
            other => self.current_route = other,
        }
    }

    fn request_mcp_catalog(&mut self, dialog: &mut McpWorkbench, preferred: Option<String>) {
        let selected = preferred.or_else(|| dialog.selected_connection().map(|c| c.id.clone()));
        let instance = dialog.instance;
        dialog.catalog_request = dialog.catalog_request.wrapping_add(1);
        let request = dialog.catalog_request;
        dialog.loading = true;
        dialog.error = None;
        self.dispatch_backend_operation(
            |application| async move {
                Ok::<_, anyhow::Error>(application.client().list_mcp_connections().await?)
            },
            move |app, result| {
                app.with_mcp_workbench(instance, |app, dialog| {
                    if dialog.catalog_request != request {
                        return;
                    }
                    dialog.loading = false;
                    match result {
                        Ok(mut catalog) => {
                            catalog
                                .connections
                                .sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                            dialog.catalog = Some(catalog);
                            dialog.reselect(selected.as_deref());
                            app.request_mcp_detail(dialog);
                        }
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                })
            },
        );
    }

    fn request_mcp_detail(&mut self, dialog: &mut McpWorkbench) {
        dialog.detail_request = dialog.detail_request.wrapping_add(1);
        dialog.capability_request = dialog.capability_request.wrapping_add(1);
        dialog.capability_loading = false;
        let Some(id) = dialog.selected_connection().map(|c| c.id.clone()) else {
            dialog.detail = None;
            dialog.capabilities = None;
            dialog.detail_loading = false;
            return;
        };
        let changed = dialog.detail.as_ref().is_none_or(|d| d.summary.id != id);
        if changed {
            dialog.preview = None;
            dialog.capabilities = None;
            dialog.cursor = None;
            dialog.cursor_history.clear();
            dialog.capability_index = 0;
            dialog.scroll = 0;
        }
        dialog.detail = None;
        dialog.detail_loading = true;
        let requested_id = id.clone();
        let instance = dialog.instance;
        let request = dialog.detail_request;
        self.dispatch_backend_operation(
            move |application| async move {
                Ok::<_, anyhow::Error>(application.client().get_mcp_connection(&id).await?)
            },
            move |app, result| {
                app.with_mcp_workbench(instance, |app, dialog| {
                    if dialog.detail_request != request
                        || dialog.selected_connection().map(|c| &c.id) != Some(&requested_id)
                    {
                        return;
                    }
                    dialog.detail_loading = false;
                    match result {
                        Ok(detail) => {
                            dialog.detail = Some(detail);
                            if dialog.draft.is_none() && dialog.import.is_none() {
                                app.request_mcp_capabilities(dialog);
                            }
                        }
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                })
            },
        );
    }

    fn request_mcp_capabilities(&mut self, dialog: &mut McpWorkbench) {
        dialog.capability_request = dialog.capability_request.wrapping_add(1);
        let Some(detail) = &dialog.detail else {
            return;
        };
        if detail.summary.state != "connected" {
            dialog.capabilities = None;
            dialog.capability_loading = false;
            dialog.cursor = None;
            dialog.cursor_history.clear();
            if dialog
                .preview
                .as_ref()
                .is_some_and(|(title, _)| title != "Connection test")
            {
                dialog.preview = None;
            }
            return;
        }
        let id = detail.summary.id.clone();
        let requested_id = id.clone();
        let instance = dialog.instance;
        let request = dialog.capability_request;
        let kind = dialog.kind;
        let cursor = dialog.cursor.clone();
        dialog.capability_loading = true;
        dialog.capabilities = None;
        dialog.capability_index = 0;
        dialog.preview = None;
        dialog.scroll = 0;
        self.dispatch_backend_operation(
            move |application| async move {
                Ok::<_, anyhow::Error>(
                    application
                        .client()
                        .mcp_connection_capabilities(&id, kind.key(), cursor.as_deref())
                        .await?,
                )
            },
            move |app, result| {
                app.with_mcp_workbench(instance, |_, dialog| {
                    if dialog.capability_request != request
                        || dialog.kind != kind
                        || dialog.selected_connection().map(|c| &c.id) != Some(&requested_id)
                    {
                        return;
                    }
                    dialog.capability_loading = false;
                    match result {
                        Ok(page) => dialog.capabilities = Some(page),
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                })
            },
        );
    }

    fn preview_mcp_import(&mut self, dialog: &mut McpWorkbench, document: Value) {
        let scope = dialog.target_scope;
        let instance = dialog.instance;
        let request = dialog.begin_operation();
        let document = Arc::new(document);
        let retained = document.clone();
        let retained_scope = scope;
        self.dispatch_backend_operation(
            move |application| async move {
                Ok::<_, anyhow::Error>(
                    application
                        .client()
                        .preview_mcp_connection_import(scope, &document)
                        .await?,
                )
            },
            move |app, result| {
                app.with_mcp_workbench(instance, |_, dialog| {
                    if dialog.operation_request != request {
                        return;
                    }
                    dialog.busy = false;
                    match result {
                        Ok(preview) => {
                            let selected = preview
                                .candidates
                                .iter()
                                .filter(|c| c.valid && !c.conflict)
                                .map(|c| c.name.clone())
                                .collect();
                            dialog.import = Some(McpImport {
                                scope: retained_scope,
                                document: retained,
                                preview,
                                selected,
                                cursor: 0,
                                replace: false,
                            });
                            dialog.editor = None;
                            dialog.editor_original.clear();
                        }
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                })
            },
        );
    }

    fn submit_mcp_operation(&mut self, dialog: &mut McpWorkbench, operation: McpOperation) {
        let instance = dialog.instance;
        let request = dialog.begin_operation();
        self.dispatch_backend_operation(
            move |application| async move {
                let client = application.client();
                let result = match operation {
                    McpOperation::Save {
                        name,
                        scope,
                        config,
                        preserve_secrets,
                        editing,
                    } => {
                        let result = match editing {
                            Some((id, revision)) => {
                                client
                                    .update_mcp_connection(
                                        &id,
                                        &config,
                                        &preserve_secrets,
                                        &revision,
                                    )
                                    .await?
                            }
                            None => client.create_mcp_connection(&name, scope, &config).await?,
                        };
                        McpOperationResult::Mutation(result)
                    }
                    McpOperation::Delete { id, revision } => McpOperationResult::Mutation(
                        client.delete_mcp_connection(&id, &revision).await?,
                    ),
                    McpOperation::Enabled {
                        id,
                        enabled,
                        revision,
                    } => McpOperationResult::Mutation(
                        client
                            .set_mcp_connection_enabled(&id, enabled, &revision)
                            .await?,
                    ),
                    McpOperation::Bridge { enabled, scope } => McpOperationResult::Catalog(
                        client.set_mcp_bridge_enabled(enabled, scope).await?,
                    ),
                    McpOperation::Reconnect { id } => McpOperationResult::Detail(
                        client.reconnect_mcp_connection(&id).await?,
                        "Connection refreshed",
                        false,
                    ),
                    McpOperation::Test { id } => {
                        McpOperationResult::Test(client.test_mcp_connection(&id).await?)
                    }
                    McpOperation::Import {
                        scope,
                        document,
                        selected,
                        replace,
                        revision,
                    } => McpOperationResult::Import(
                        client
                            .import_mcp_connections(scope, &document, &selected, replace, &revision)
                            .await?,
                    ),
                    McpOperation::Credential { id, token } => McpOperationResult::Detail(
                        client.set_mcp_connection_credential(&id, &token).await?,
                        "Stored credential updated",
                        false,
                    ),
                    McpOperation::DeleteCredential { id } => McpOperationResult::Detail(
                        client.delete_mcp_connection_credential(&id).await?,
                        "Stored bearer credential removed",
                        false,
                    ),
                    McpOperation::StartAuthorization { id, redirect_uri } => {
                        let flow = client.start_mcp_authorization(&id, &redirect_uri).await?;
                        McpOperationResult::Authorization(PendingAuthorization {
                            id,
                            flow_id: flow.flow_id,
                            authorization_url: flow.authorization_url,
                            expires_at: flow.expires_at,
                        })
                    }
                    McpOperation::CompleteAuthorization {
                        id,
                        flow_id,
                        callback_url,
                    } => McpOperationResult::Detail(
                        client
                            .complete_mcp_authorization(&id, &flow_id, &callback_url)
                            .await?,
                        "OAuth authorization completed",
                        true,
                    ),
                    McpOperation::CancelAuthorization { id, flow_id } => {
                        McpOperationResult::Detail(
                            client
                                .cancel_mcp_authorization(&id, flow_id.as_deref())
                                .await?,
                            if flow_id.is_some() {
                                "Authorization cancelled"
                            } else {
                                "OAuth credentials removed"
                            },
                            true,
                        )
                    }
                    McpOperation::Resource { id, uri } => McpOperationResult::Preview(
                        "Resource contents".to_owned(),
                        client.read_mcp_connection_resource(&id, &uri).await?,
                    ),
                    McpOperation::Prompt {
                        id,
                        name,
                        arguments,
                    } => McpOperationResult::Preview(
                        format!("Prompt: {name}"),
                        client
                            .get_mcp_connection_prompt(&id, &name, &arguments)
                            .await?,
                    ),
                };
                Ok::<_, anyhow::Error>(result)
            },
            move |app, result| {
                app.with_mcp_workbench(instance, |app, dialog| {
                    if dialog.operation_request != request {
                        return;
                    }
                    dialog.busy = false;
                    match result {
                        Ok(result) => app.finish_mcp_operation(dialog, result),
                        Err(error) => dialog.error = Some(error.to_string()),
                    }
                })
            },
        );
    }

    fn finish_mcp_operation(&mut self, dialog: &mut McpWorkbench, result: McpOperationResult) {
        match result {
            McpOperationResult::Mutation(result) => {
                if !result.saved {
                    dialog.error = Some(result.warning.unwrap_or_else(|| {
                        "The connection was not saved. Your draft is still available.".to_owned()
                    }));
                    return;
                }
                dialog.draft = None;
                dialog.map_editor = None;
                dialog.editor = None;
                dialog.editor_original.clear();
                dialog.confirm = None;
                dialog.notice=result.warning.or_else(||result.reload_task_id.map(|_|"Configuration saved; the server is applying the change. Refresh to check connection status.".to_owned()));
                let id = result.connection.as_ref().map(|c| c.summary.id.clone());
                dialog.detail = result.connection;
                dialog.query.clear();
                dialog.scope_filter = None;
                self.flash_success("MCP connection saved");
                self.request_mcp_catalog(dialog, id);
            }
            McpOperationResult::Detail(detail, message, finish_authorization) => {
                if finish_authorization {
                    dialog.authorization = None;
                }
                let id = detail.summary.id.clone();
                dialog.detail = Some(detail);
                dialog.editor = None;
                dialog.editor_original.clear();
                dialog.confirm = None;
                dialog.notice = Some(message.to_owned());
                self.request_mcp_catalog(dialog, Some(id));
            }
            McpOperationResult::Test(result) => {
                dialog.notice = Some(if result.ok {
                    format!(
                        "Connection test succeeded in {} ms · {} tools",
                        result.elapsed_ms, result.tool_count
                    )
                } else {
                    format!(
                        "Connection test failed: {}",
                        result.error.as_deref().unwrap_or("No details returned")
                    )
                });
                dialog.preview = Some((
                    "Connection test".to_owned(),
                    serde_json::to_value(result).unwrap_or(Value::Null),
                ));
                dialog.scroll = 0;
                dialog.pane = McpPane::Capabilities;
            }
            McpOperationResult::Import(result) => {
                let count = result.installed.len();
                let id = result.installed.first().map(|c| c.id.clone());
                dialog.import = None;
                dialog.editor = None;
                dialog.editor_original.clear();
                dialog.confirm = None;
                dialog.query.clear();
                dialog.scope_filter = None;
                dialog.notice=result.warning.or_else(||result.reload_task_id.map(|_|"Connections saved; the server is applying the change. Refresh to check status.".to_owned()));
                self.flash_success(format!("Imported {count} MCP connections"));
                self.request_mcp_catalog(dialog, id);
            }
            McpOperationResult::Authorization(flow) => {
                dialog.editor = None;
                dialog.editor_original.clear();
                dialog.authorization = Some(flow);
                dialog.notice=Some("Open the authorization URL below. C copies it; A accepts the complete callback URL after sign-in. X cancels.".to_owned());
            }
            McpOperationResult::Preview(title, value) => {
                dialog.editor = None;
                dialog.editor_original.clear();
                dialog.preview = Some((title, value));
                dialog.scroll = 0;
                dialog.pane = McpPane::Capabilities;
            }
            McpOperationResult::Catalog(catalog) => {
                let id = dialog.selected_connection().map(|c| c.id.clone());
                dialog.catalog = Some(catalog);
                dialog.reselect(id.as_deref());
                dialog.notice = Some(
                    "MCP bridge setting saved. Per-connection availability is unchanged."
                        .to_owned(),
                );
                self.request_mcp_detail(dialog);
            }
        }
    }
}

enum McpOperation {
    Save {
        name: String,
        scope: McpConnectionScope,
        config: Value,
        preserve_secrets: Vec<String>,
        editing: Option<(String, String)>,
    },
    Delete {
        id: String,
        revision: String,
    },
    Enabled {
        id: String,
        enabled: bool,
        revision: String,
    },
    Bridge {
        enabled: bool,
        scope: McpConnectionScope,
    },
    Reconnect {
        id: String,
    },
    Test {
        id: String,
    },
    Import {
        scope: McpConnectionScope,
        document: Arc<Value>,
        selected: Vec<String>,
        replace: bool,
        revision: String,
    },
    Credential {
        id: String,
        token: String,
    },
    DeleteCredential {
        id: String,
    },
    StartAuthorization {
        id: String,
        redirect_uri: String,
    },
    CompleteAuthorization {
        id: String,
        flow_id: String,
        callback_url: String,
    },
    CancelAuthorization {
        id: String,
        flow_id: Option<String>,
    },
    Resource {
        id: String,
        uri: String,
    },
    Prompt {
        id: String,
        name: String,
        arguments: Value,
    },
}
enum McpOperationResult {
    Mutation(McpMutationResult),
    Detail(McpConnectionDetail, &'static str, bool),
    Catalog(McpConnectionCatalog),
    Test(McpTestResult),
    Import(McpImportResult),
    Authorization(PendingAuthorization),
    Preview(String, Value),
}
