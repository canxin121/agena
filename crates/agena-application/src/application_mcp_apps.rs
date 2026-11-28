use std::{sync::Arc, time::Duration};

use agena_api::mcp_apps::{
    McpAppCatalog, McpAppCatalogChange, McpAppDocument, McpAppRequest, McpAppServerRequest,
    McpAppViewOpened, McpAppViewPublication,
};
use agena_domain::mcp_app_tools::{McpAppViewBinding, McpAppViewScope};
use agena_domain::{ContentFormat, RawOutput, StructuredObject, ToolInvocation};
use agena_mcp_client::apps::{
    APP_MESSAGE_LIMIT, APP_METADATA_KEY, APP_RESULT_METADATA_KEY, McpAppBinding, app_resource,
};
use agena_plugin_host::sdk::host_api::{
    McpAppCatalogSnapshot, McpAppCatalogWatch, McpAppCatalogWatches, McpAppHostBinding,
    McpAppHostRequest, McpAppServerTaskPort, scope_mcp_app_request,
};
use agena_runtime::McpAppServerTaskProtocol;
use agena_storage::kernel::{EntryPayload, ToolPhase};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::{Application, ApplicationError};

const MCP_PLUGIN: &str = "agena.mcp";
const TOOL_CALL: &str = "tools.call";
const TOOL_SEARCH: &str = "tools.search";
const RESOURCE_LIST: &str = "resources.list";
const RESOURCE_TEMPLATE_LIST: &str = "resources.templates.list";
const RESOURCE_READ: &str = "resources.read";
const CATALOG_WAIT: Duration = Duration::from_secs(15);
const CATALOG_FALLBACK_MS: u32 = 4_000;
static CATALOG_WAITS: Semaphore = Semaphore::const_new(64);

struct BoundApp {
    call_id: i64,
    binding: McpAppBinding,
    reference: String,
    result: Value,
    invocation: ToolInvocation,
}

#[derive(Default)]
struct AppRequestOptions {
    catalogs: McpAppCatalogWatches,
    server_task: Option<McpAppServerTaskPort>,
    view: Option<McpAppViewScope>,
}

impl Application {
    pub fn mcp_app_views(&self) -> Result<Arc<agena_runtime::McpAppViews>, ApplicationError> {
        self.session_execution_services()?
            .tool_execution
            .mcp_app_views()
            .ok_or_else(unavailable_app)
    }

    pub async fn open_mcp_app_view(
        &self,
        binding: McpAppViewBinding,
        publication: McpAppViewPublication,
    ) -> Result<McpAppViewOpened, ApplicationError> {
        let views = self.mcp_app_views()?;
        let generation = views.admission_generation();
        let app = self
            .bound_mcp_app_at_revision(binding.session_id, binding.part_id, &binding.revision)
            .await?;
        self.mcp_app_catalog(
            binding.session_id,
            &app,
            McpAppCatalog::Tools,
            McpAppCatalogWatch::default(),
        )
        .await?;
        let name = format!("{}/{}", app.binding.server, app.binding.tool);
        let view: Uuid = views
            .open(
                generation,
                binding,
                name,
                publication.tools,
                publication.task_extension,
            )
            .await
            .map_err(crate::mcp_app_view_error)?;
        Ok(McpAppViewOpened { view })
    }

    pub async fn mcp_app_document(
        &self,
        session_id: i64,
        part_id: i64,
    ) -> Result<McpAppDocument, ApplicationError> {
        let app = self.bound_mcp_app(session_id, part_id).await?;
        let raw = self.mcp_app_result(session_id, &app.result).await?;
        let tools = McpAppCatalogWatch::default();
        let result = self
            .execute_mcp_app_request(
                session_id,
                &app,
                RESOURCE_READ,
                json!({
                    "server": app.binding.server, "uri": app.binding.uri,
                }),
                AppRequestOptions {
                    catalogs: McpAppCatalogWatches { tools: Some(tools.clone()), resources: None },
                    ..Default::default()
                },
            )
            .await?
            .ok_or_else(unavailable_app)?;
        let tool_catalog_revision = tools.snapshot().ok_or_else(unavailable_app)?.revision;
        let resource = serde_json::from_value(result).map_err(|_| unavailable_app())?;
        let resource = app_resource(&app.binding.uri, resource).ok_or_else(unavailable_app)?;
        let resource_catalog_revision = self
            .mcp_app_catalog(
                session_id,
                &app,
                McpAppCatalog::Resources,
                McpAppCatalogWatch::default(),
            )
            .await
            .ok()
            .map(|snapshot| snapshot.revision);
        let input: Value = app.invocation.input.into();
        let arguments = input
            .get("arguments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if Value::Object(arguments.clone()).to_string().len() > APP_MESSAGE_LIMIT {
            return Err(ApplicationError::bad_request(
                "The MCP App's original input exceeds its size limit.",
            ));
        }
        Ok(McpAppDocument {
            revision: app.binding.revision().to_owned(),
            tool_catalog_revision,
            resource_catalog_revision,
            name: app.binding.tool,
            html: resource.html,
            ui: resource.ui.unwrap_or_default(),
            arguments,
            tool_result: raw,
        })
    }

    pub async fn mcp_app_request(
        &self,
        session_id: i64,
        part_id: i64,
        revision: &str,
        request: McpAppRequest,
    ) -> Result<Value, ApplicationError> {
        let app = self
            .bound_mcp_app_at_revision(session_id, part_id, revision)
            .await?;
        let server = &app.binding.server;
        let (tool_name, input) = match request {
            McpAppRequest::ListTools { cursor } => {
                let offset = cursor
                    .map(|cursor| cursor.parse::<u32>())
                    .transpose()
                    .map_err(|_| ApplicationError::bad_request("The MCP App cursor is invalid."))?
                    .unwrap_or_default();
                (TOOL_SEARCH, json!({"catalog_offset": offset}))
            }
            McpAppRequest::CallTool { name, arguments } if valid_guest_field(&name, 256) => (
                TOOL_CALL,
                json!({"server": server, "name": name, "arguments": arguments}),
            ),
            McpAppRequest::ReadResource { uri } if valid_guest_field(&uri, 2048) => {
                (RESOURCE_READ, json!({"server": server, "uri": uri}))
            }
            McpAppRequest::ListResources { cursor } => {
                (RESOURCE_LIST, resource_list_input(server, cursor)?)
            }
            McpAppRequest::ListResourceTemplates { cursor } => {
                (RESOURCE_TEMPLATE_LIST, resource_list_input(server, cursor)?)
            }
            _ => {
                return Err(ApplicationError::bad_request(
                    "The MCP App request is invalid.",
                ));
            }
        };
        self.execute_mcp_app_request(
            session_id,
            &app,
            tool_name,
            input,
            AppRequestOptions::default(),
        )
        .await?
        .ok_or_else(unavailable_app)
    }

    pub async fn mcp_app_server_request(
        &self,
        binding: McpAppViewBinding,
        view: Uuid,
        request: McpAppServerRequest,
    ) -> Result<Value, ApplicationError> {
        use McpAppServerRequest as Request;

        let app = self.bound_mcp_app_at_revision(binding.session_id, binding.part_id, &binding.revision).await?;
        let views = self.mcp_app_views()?;
        let get = |id: &str| views.server_task(&binding, view, id).map_err(mcp_app_view_error);
        let request = match request {
            Request::Get { task_id } => return Ok(get(&task_id)?.status()),
            Request::Update {
                task_id,
                input_responses,
            } => return get(&task_id)?.update(input_responses).await.map_err(mcp_app_view_error),
            Request::Result { task_id } => {
                return get(&task_id)?.legacy_result().await.map_err(mcp_app_view_error);
            }
            Request::Cancel { task_id } => return get(&task_id)?.request_cancel().map_err(mcp_app_view_error),
            request => request,
        };

        let session = binding.session_id;
        let scope = McpAppViewScope { binding, view };
        let operation = views.begin_request(&scope).map_err(mcp_app_view_error)?;
        operation.run(async {
            let mut options = AppRequestOptions {
                view: Some(scope.clone()),
                ..Default::default()
            };
            let (tool, input) = match request {
                Request::CallTool { name, arguments, task } if valid_guest_field(&name, 256) => {
                    let protocol = task.map(|task| McpAppServerTaskProtocol::Legacy { ttl_ms: task.ttl })
                        .or_else(|| operation.task_extension().then_some(McpAppServerTaskProtocol::Extension));
                    let input = json!({"server": app.binding.server, "name": name, "arguments": arguments});
                    if let Some(protocol) = protocol {
                        let task = views.reserve_server_task(&scope.binding, scope.view, protocol)
                            .map_err(mcp_app_view_error)?;
                        let unpublished = task.port().cancellation().clone().drop_guard();
                        options.server_task = Some(task.port().clone());
                        let application = self.clone();
                        let running = task.clone();
                        tokio::spawn(async move {
                            running.run(async {
                                application.execute_mcp_app_request(session, &app, TOOL_CALL, input, options)
                                    .await
                                    .and_then(|result| result.ok_or_else(unavailable_app))
                                    .map_err(|error| error.failure.user.fallback)
                            }).await;
                        });
                        let reply = task.initial_reply().await.map_err(mcp_app_view_error)?;
                        unpublished.disarm();
                        return Ok(reply);
                    }
                    (TOOL_CALL, input)
                }
                Request::ReadResource { uri } if valid_guest_field(&uri, 2048) => {
                    (RESOURCE_READ, json!({"server": app.binding.server, "uri": uri}))
                }
                Request::Sample(params) => {
                    let executor = self.session_execution_services()?.tool_execution;
                    let cancellation = operation.cancellation().child_token();
                    let _cancel_on_drop = cancellation.clone().drop_guard();
                    return executor.sample_mcp_app(scope, Value::Object(params), cancellation)
                        .await.map_err(mcp_app_sampling_error);
                }
                _ => return Err(ApplicationError::bad_request("The MCP App request is invalid.")),
            };
            self.execute_mcp_app_request(session, &app, tool, input, options)
                .await?
                .ok_or_else(unavailable_app)
        }).await.map_err(mcp_app_view_error)?
    }

    pub async fn mcp_app_catalog_change(
        &self,
        session_id: i64,
        part_id: i64,
        revision: &str,
        catalog: McpAppCatalog,
        after: &str,
    ) -> Result<McpAppCatalogChange, ApplicationError> {
        let wait = CATALOG_WAITS.try_acquire().ok();
        let watch = match (catalog, &wait) {
            (McpAppCatalog::Resources, Some(_)) => {
                McpAppCatalogWatch::wait_after(after.to_owned(), CATALOG_WAIT)
            }
            _ => McpAppCatalogWatch::default(),
        };
        let read_catalog = |watch| async move {
            let app = self.bound_mcp_app_at_revision(session_id, part_id, revision).await?;
            self.mcp_app_catalog(session_id, &app, catalog, watch).await
        };
        let mut current = read_catalog(watch).await?;
        let mut retry_after_ms = CATALOG_FALLBACK_MS;
        if current.waited {
            // The Streamable HTTP receive stream runs inside the original
            // authorized request. Recheck the binding and policy after it ends.
            current = read_catalog(McpAppCatalogWatch::default()).await?;
            retry_after_ms = 250;
        } else if wait.is_some()
            && current.revision == after
            && let Some((mut changes, closed)) = current.changes.take()
        {
            let changed = tokio::select! {
                biased;
                _ = closed.cancelled() => return Err(unavailable_app()),
                result = changes.changed() => result.map_err(|_| unavailable_app()).map(|()| true)?,
                _ = tokio::time::sleep(CATALOG_WAIT) => false,
            };
            if changed {
                // The App or the user's permissions may have changed while
                // waiting. Only a fresh authorized catalog reaches the view.
                current = read_catalog(McpAppCatalogWatch::default()).await?;
            }
            retry_after_ms = 250;
        }
        Ok(McpAppCatalogChange {
            revision: current.revision,
            retry_after_ms,
        })
    }

    async fn mcp_app_catalog(
        &self,
        session_id: i64,
        app: &BoundApp,
        catalog: McpAppCatalog,
        watch: McpAppCatalogWatch,
    ) -> Result<McpAppCatalogSnapshot, ApplicationError> {
        let (tool, input, catalogs) = match catalog {
            McpAppCatalog::Tools => (
                TOOL_SEARCH,
                json!({"catalog_offset": 0}),
                McpAppCatalogWatches {
                    tools: Some(watch.clone()),
                    resources: None,
                },
            ),
            McpAppCatalog::Resources => (
                RESOURCE_LIST,
                json!({"server": app.binding.server}),
                McpAppCatalogWatches {
                    tools: None,
                    resources: Some(watch.clone()),
                },
            ),
        };
        self.execute_mcp_app_request(session_id, app, tool, input, AppRequestOptions {
            catalogs, ..Default::default()
        })
            .await?;
        watch.snapshot().ok_or_else(unavailable_app)
    }

    async fn bound_mcp_app_at_revision(
        &self,
        session_id: i64,
        part_id: i64,
        revision: &str,
    ) -> Result<BoundApp, ApplicationError> {
        let app = self.bound_mcp_app(session_id, part_id).await?;
        if revision == app.binding.revision() {
            Ok(app)
        } else {
            Err(ApplicationError::conflict(
                "The MCP App changed. Reopen it to continue.",
            ))
        }
    }

    async fn bound_mcp_app(
        &self,
        session_id: i64,
        part_id: i64,
    ) -> Result<BoundApp, ApplicationError> {
        let session = crate::kernel_session_id(session_id)?;
        if part_id <= 0 {
            return Err(unavailable_app());
        }
        let reader = &self.session_kernel()?.reader;
        // Forked histories can still display the original tool result. Only its
        // owning session can launch the app or use a live connection.
        let history = reader
            .history_by_ids(session, &[part_id])
            .await
            .map_err(crate::kernel_error)?;
        let entry = history
            .entries
            .iter()
            .find(|entry| entry.id.get() == part_id && entry.origin_session_id == session)
            .ok_or_else(unavailable_app)?;
        let EntryPayload::Tool { tool_id } = entry.payload else {
            return Err(unavailable_app());
        };
        let view = reader
            .tool(session, tool_id)
            .await
            .map_err(crate::kernel_error)?;
        if view.entry_id != entry.id || view.phase != ToolPhase::Succeeded {
            return Err(unavailable_app());
        }
        let result = view.result.as_ref().ok_or_else(unavailable_app)?;
        let reference = result
            .get("metadata")
            .and_then(|meta| meta.get(APP_METADATA_KEY))
            .and_then(Value::as_str)
            .filter(|reference| reference.len() <= 8192)
            .ok_or_else(unavailable_app)?
            .to_owned();
        let binding =
            serde_json::from_str::<McpAppBinding>(&reference).map_err(|_| unavailable_app())?;
        let invocation = result
            .get("invocation")
            .cloned()
            .and_then(|value| serde_json::from_value::<ToolInvocation>(value).ok())
            .ok_or_else(unavailable_app)?;
        let input = &invocation.input;
        if invocation.plugin_name.as_deref() != Some(MCP_PLUGIN)
            || agena_tool::normalized_tool_identity(&invocation.name) != "mcp.tools.call"
            || input
                .get("server")
                .and_then(agena_domain::StructuredValue::as_text)
                .map(str::trim)
                != Some(binding.server.as_str())
            || input
                .get("name")
                .and_then(agena_domain::StructuredValue::as_text)
                .map(str::trim)
                != Some(binding.tool.as_str())
        {
            return Err(unavailable_app());
        }
        Ok(BoundApp {
            call_id: tool_id.get(),
            binding,
            reference,
            result: result.clone(),
            invocation,
        })
    }

    async fn execute_mcp_app_request(
        &self,
        session_id: i64,
        app: &BoundApp,
        tool_name: &'static str,
        input: Value,
        options: AppRequestOptions,
    ) -> Result<Option<Value>, ApplicationError> {
        let invocation = ToolInvocation::plugin_named(
            format!("{MCP_PLUGIN}.{tool_name}"),
            MCP_PLUGIN,
            StructuredObject::try_from(input.clone()).expect("MCP App tool input is an object"),
        );
        let request = McpAppHostRequest {
            session_id,
            tool_name,
            input,
            binding: McpAppHostBinding {
                source_call_id: app.call_id,
                reference: app.reference.clone(),
                catalogs: options.catalogs,
                server_task: options.server_task,
                view: options.view,
            },
        };
        let executor = self.session_execution_services()?.tool_execution;
        let result = scope_mcp_app_request(request, executor.execute_mcp_app_tool(session_id, invocation))
            .await.map_err(|error| ApplicationError::conflict_with_diagnostic(
                "The MCP App request was unavailable under the current connection or session permissions.", error,
            ))?;
        Ok(result.payload)
    }

    async fn mcp_app_result(
        &self,
        session_id: i64,
        result: &Value,
    ) -> Result<Value, ApplicationError> {
        let raw = serde_json::from_value::<RawOutput>(
            result
                .get("raw_output")
                .cloned()
                .ok_or_else(unavailable_app)?,
        )
        .map_err(|_| unavailable_app())?;
        let payload = if let Some(payload) = raw.payload {
            payload
        } else {
            let field = raw
                .fields
                .iter()
                .find(|field| field.pointer.is_empty() && field.format == ContentFormat::Json)
                .ok_or_else(unavailable_app)?;
            let page = self
                .read_content_text(agena_api::content::ReadContentTextParams {
                    session_id,
                    resource_id: field.resource.resource_id,
                    position: None,
                    max_bytes: APP_MESSAGE_LIMIT,
                })
                .await?;
            if page.has_more || page.gap {
                return Err(ApplicationError::bad_request(
                    "The MCP App's original result exceeds its size limit.",
                ));
            }
            serde_json::from_str(
                &page
                    .slices
                    .into_iter()
                    .map(|slice| slice.text)
                    .collect::<String>(),
            )
            .map_err(|_| unavailable_app())?
        };
        let mut protocol_result =
            json!({"content": payload.get("content").cloned().unwrap_or_else(|| json!([]))});
        if let Some(structured) = payload
            .get("structured_content")
            .filter(|value| !value.is_null())
        {
            protocol_result["structuredContent"] = structured.clone();
        }
        if let Some(meta) = result
            .get("metadata")
            .and_then(|meta| meta.get(APP_RESULT_METADATA_KEY))
            .and_then(Value::as_str)
            .and_then(|meta| serde_json::from_str::<Value>(meta).ok())
        {
            protocol_result["_meta"] = meta;
        }
        if protocol_result.to_string().len() > APP_MESSAGE_LIMIT {
            return Err(ApplicationError::bad_request(
                "The MCP App's original result exceeds its size limit.",
            ));
        }
        Ok(protocol_result)
    }
}

pub fn mcp_app_view_error(error: agena_runtime::McpAppViewError) -> ApplicationError {
    use agena_runtime::McpAppViewError;
    match error {
        McpAppViewError::Invalid(_) => ApplicationError::bad_request_error(&error),
        McpAppViewError::Capacity => ApplicationError::service_unavailable(error),
        _ => ApplicationError::conflict("The MCP App view is no longer available."),
    }
}

fn mcp_app_sampling_error(error: agena_mcp_client::McpError) -> ApplicationError {
    use agena_mcp_client::McpError;
    match error {
        McpError::Serde(_) | McpError::Malformed(_) => {
            ApplicationError::bad_request("The MCP App sampling request is invalid.")
        }
        McpError::SamplingUnsupported => ApplicationError::bad_request(
            "MCP App sampling supports text messages and independent model output.",
        ),
        McpError::SamplingDeclined => ApplicationError::conflict(
            "The MCP App sampling request was declined or cancelled.",
        ),
        _ => ApplicationError::conflict_with_diagnostic("MCP App sampling was unavailable.", error),
    }
}

fn unavailable_app() -> ApplicationError {
    ApplicationError::not_found("The MCP App is unavailable in this conversation.")
}

fn valid_guest_field(field: &str, limit: usize) -> bool {
    !field.is_empty()
        && field.len() <= limit
        && field.trim() == field
        && !field.chars().any(char::is_control)
}

fn resource_list_input(server: &str, cursor: Option<String>) -> Result<Value, ApplicationError> {
    match cursor {
        Some(cursor) if cursor.len() <= 2048 && !cursor.chars().any(char::is_control) => {
            Ok(json!({"server": server, "cursor": cursor}))
        }
        Some(_) => Err(ApplicationError::bad_request(
            "The MCP App cursor is invalid.",
        )),
        None => Ok(json!({"server": server})),
    }
}
