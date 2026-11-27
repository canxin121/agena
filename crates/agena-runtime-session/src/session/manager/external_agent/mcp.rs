use std::net::Ipv4Addr;

use agena_domain::{AttachmentKind, ContentKind, RawOutput, ToolInvocation};
use agena_mcp_server::{
    CallToolParams, CallToolResult, ContentBlock, McpServerBackend, McpServerError, ToolApiAction,
    ToolDescriptor, dispatch_tool_api, text_error, text_result, tool_api_descriptors,
};
use agena_provider::CompletionInputAttachmentSource;
use agena_tool_api::ToolApiCatalog;
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use tokio::{
    net::TcpListener,
    sync::{RwLock, watch},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::DropGuard;

use super::*;
use crate::session::kernel::{media, run_read_budget};

mod sse;
mod stdio;

const RESULT_BYTES: usize = 50 * 1024;

#[derive(Clone)]
struct Scope {
    turn: Arc<ExternalTurn>,
    closing: CancellationToken,
    calls: Arc<RwLock<()>>,
    activity: watch::Sender<ToolActivity>,
}

#[derive(Clone, Copy, Default)]
pub(super) struct ToolActivity {
    pub(super) active: usize,
    pub(super) finished_at: Option<Instant>,
}

struct ActiveCall(watch::Sender<ToolActivity>);

impl Drop for ActiveCall {
    fn drop(&mut self) {
        self.0.send_modify(|activity| {
            activity.active -= 1;
            activity.finished_at = Some(Instant::now());
        });
    }
}

#[derive(Clone)]
pub(super) struct ToolProgress {
    pub(super) activity: watch::Receiver<ToolActivity>,
    calls: Arc<RwLock<()>>,
    closing: CancellationToken,
}

impl ToolProgress {
    pub(super) fn revoke_if_idle(&self) -> bool {
        let Ok(_exclusive) = self.calls.try_write() else {
            return false;
        };
        self.closing.cancel();
        true
    }
}

pub(super) struct Bridge {
    scope: Scope,
    _guard: DropGuard,
    transport: Transport,
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Http,
    Sse,
    Stdio { typed: bool },
    Acp,
}

enum Transport {
    Local {
        descriptor: Value,
        server: JoinHandle<std::io::Result<()>>,
    },
    Acp(AcpEndpoint),
}

#[derive(Clone)]
pub(super) struct AcpEndpoint {
    id: String,
    backend: Backend,
}

impl AcpEndpoint {
    pub(super) fn matches(&self, id: &Value) -> bool {
        id.as_str() == Some(self.id.as_str())
    }

    pub(super) async fn respond(
        &self,
        method: &str,
        params: Value,
        withdrawal: CancellationToken,
    ) -> Result<Value, AppError> {
        const REVISION: &str = "2026-07-28";
        let error = |code, message: &str| json!({"error":{"code":code,"message":message}});
        if withdrawal.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        if method != "server/discover"
            && params
                .pointer("/_meta/io.modelcontextprotocol~1protocolVersion")
                .and_then(Value::as_str)
                != Some(REVISION)
        {
            return Ok(error(-32602, "MCP-over-ACP requires MCP 2026-07-28"));
        }
        let result = match method {
            "server/discover" => Ok(json!({
                "supportedVersions": [REVISION],
                "capabilities": {"tools": {}},
                "ttlMs": 0,
                "cacheScope": "private",
                "_meta": {"io.modelcontextprotocol/serverInfo": {
                    "name": "agena", "version": env!("CARGO_PKG_VERSION")
                }}
            })),
            "tools/list" => self
                .backend
                .list_tools()
                .await
                .map(|tools| json!({"tools": tools})),
            "tools/call" => {
                let Ok(params) = serde_json::from_value::<CallToolParams>(params) else {
                    return Ok(error(-32602, "Invalid MCP tool arguments"));
                };
                self.backend
                    .call_tool_with_cancellation(params, withdrawal)
                    .await
                    .and_then(|result| Ok(serde_json::to_value(result)?))
            }
            _ => return Ok(error(-32601, "MCP method not found")),
        };
        Ok(match result {
            Ok(mut value) => {
                value["resultType"] = json!("complete");
                json!({"result": value})
            }
            Err(McpServerError::InvalidParams(_) | McpServerError::NotFound(_)) => {
                error(-32602, "Invalid MCP tool request")
            }
            Err(McpServerError::Cancelled) => return Err(AppError::Cancelled),
            Err(_) => error(-32603, "Agena could not complete the MCP request"),
        })
    }
}

impl Bridge {
    pub(super) async fn start(
        turn: Arc<ExternalTurn>,
        kind: Kind,
        relay_socket: Option<&std::path::Path>,
    ) -> Result<Self, AppError> {
        let catalog = super::super::tool_api::catalog(&turn.executor).await;
        let scope = Scope {
            closing: turn.control.cancel.child_token(),
            turn,
            calls: Default::default(),
            activity: watch::Sender::new(ToolActivity::default()),
        };
        let backend = Backend {
            scope: scope.clone(),
            catalog: Arc::new(catalog),
        };
        let transport = match kind {
            Kind::Acp => Transport::Acp(AcpEndpoint {
                id: uuid::Uuid::new_v4().to_string(),
                backend,
            }),
            Kind::Http => Self::http(&scope, backend, false).await?,
            Kind::Sse => Self::http(&scope, backend, true).await?,
            Kind::Stdio { typed } => stdio::start(&scope, backend, typed, relay_socket).await?,
        };
        Ok(Self {
            _guard: scope.closing.clone().drop_guard(),
            scope,
            transport,
        })
    }

    async fn http(
        scope: &Scope,
        backend: Backend,
        legacy_sse: bool,
    ) -> Result<Transport, AppError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let authorization = format!("Bearer {}", uuid::Uuid::new_v4());
        let (app, kind, path) = if legacy_sse {
            (sse::router(backend, scope.closing.clone()), "sse", "/sse")
        } else {
            let config = rmcp::transport::StreamableHttpServerConfig::default()
                .with_json_response(true)
                .with_allowed_origins([origin.clone()])
                .with_cancellation_token(scope.closing.clone());
            let service = agena_mcp_server::streamable_http_service(backend, config);
            (Router::new().route_service("/mcp", service), "http", "/mcp")
        };
        let descriptor = json!({
            "type": kind, "name": "agena", "url": format!("{origin}{path}"),
            "headers": [{"name": "Authorization", "value": authorization}]
        });
        let app = app.layer(middleware::from_fn_with_state(
            (scope.closing.clone(), authorization, origin),
            authorize,
        ));
        let closing = scope.closing.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(closing.cancelled_owned())
                .await
        });
        Ok(Transport::Local { descriptor, server })
    }

    pub(super) fn descriptor(&self) -> Value {
        match &self.transport {
            Transport::Acp(endpoint) => {
                json!({"type": "acp", "name": "agena", "serverId": endpoint.id})
            }
            Transport::Local { descriptor, .. } => descriptor.clone(),
        }
    }

    pub(super) fn acp_endpoint(&self) -> Option<AcpEndpoint> {
        match &self.transport {
            Transport::Acp(endpoint) => Some(endpoint.clone()),
            Transport::Local { .. } => None,
        }
    }

    pub(super) fn tool_progress(&self) -> ToolProgress {
        ToolProgress {
            activity: self.scope.activity.subscribe(),
            calls: self.scope.calls.clone(),
            closing: self.scope.closing.clone(),
        }
    }

    pub(super) async fn close(mut self) {
        self.scope.closing.cancel();
        // An ACP prompt normally stops after consuming all its tool results.
        // If the child exits early, cancel and drain accepted effects before
        // the attempt loses ownership. A dropped HTTP waiter cannot skip this.
        drop(self.scope.calls.write().await);
        if let Transport::Local { server, .. } = &mut self.transport
            && tokio::time::timeout(Duration::from_millis(500), &mut *server)
                .await
                .is_err()
        {
            server.abort();
        }
    }
}

async fn authorize(
    State((closing, authorization, origin)): State<(CancellationToken, String, String)>,
    request: Request,
    next: Next,
) -> Response {
    if closing.is_cancelled()
        || request
            .headers()
            .get(header::AUTHORIZATION)
            .map(|value| value.as_bytes())
            != Some(authorization.as_bytes())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if request
        .headers()
        .get(header::ORIGIN)
        .is_some_and(|value| value.as_bytes() != origin.as_bytes())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}

#[derive(Clone)]
struct Backend {
    scope: Scope,
    catalog: Arc<ToolApiCatalog>,
}

#[async_trait::async_trait]
impl McpServerBackend for Backend {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError> {
        Ok(tool_api_descriptors())
    }

    async fn call_tool(&self, params: CallToolParams) -> Result<CallToolResult, McpServerError> {
        self.call_tool_scoped(params, None).await
    }

    async fn call_tool_with_cancellation(
        &self,
        params: CallToolParams,
        cancel: CancellationToken,
    ) -> Result<CallToolResult, McpServerError> {
        self.call_tool_scoped(params, Some(cancel)).await
    }
}

impl Backend {
    async fn call_tool_scoped(
        &self,
        params: CallToolParams,
        request_cancel: Option<CancellationToken>,
    ) -> Result<CallToolResult, McpServerError> {
        let turn = self.scope.turn.clone();
        let guard = self.scope.calls.clone().read_owned().await;
        if request_cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(McpServerError::Cancelled);
        }
        if self.scope.closing.is_cancelled() {
            return Ok(text_error("The Agena turn has ended."));
        }
        let request = match dispatch_tool_api(&self.catalog, params)? {
            ToolApiAction::Respond(response) => return Ok(text_result(response.output_text)),
            ToolApiAction::Execute(request) => request,
        };
        let target = self
            .catalog
            .resolve_tool(&request.tool)
            .ok_or(McpServerError::NotFound(request.tool))?;
        let input = Value::from(request.input.clone());
        let invocation = ToolInvocation::new(target.canonical_name.clone(), request.input);
        self.scope
            .activity
            .send_modify(|activity| activity.active += 1);
        let active = ActiveCall(self.scope.activity.clone());
        if self.scope.closing.is_cancelled() {
            return Ok(text_error("The Agena turn has ended."));
        }
        // The accepted call outlives transport disconnects. The bridge holds
        // the attempt open until this call has committed or been cancelled.
        let cancel = self.scope.closing.child_token();
        let result = tokio::spawn(async move {
            let _guard = guard;
            let _active = active;
            let call = turn.call_tool(invocation, input, &cancel);
            tokio::pin!(call);
            if let Some(request_cancel) = request_cancel {
                tokio::select! {
                    biased;
                    _ = request_cancel.cancelled() => {
                        cancel.cancel();
                        call.await
                    }
                    result = &mut call => result,
                }
            } else {
                call.await
            }
        })
        .await
        .map_err(|error| McpServerError::Backend(error.to_string()))?;
        match result {
            Ok(result) => Ok(result),
            Err(AppError::Cancelled) => Err(McpServerError::Cancelled),
            Err(error) => Ok(text_error(error.public_message())),
        }
    }
}

impl ExternalTurn {
    pub(super) async fn invoke_tool(
        &self,
        invocation: ToolInvocation,
        input: Value,
        cancel: &CancellationToken,
    ) -> Result<(ToolHandle, ToolResult), AppError> {
        if cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let tool = self.begin_tool(invocation.name.clone(), input).await?;
        self.control
            .phase(agena_domain::ExecutionPhase::ExecutingTools);
        let result = self
            .manager
            .execute_tool(
                tool.clone(),
                invocation.clone(),
                self.executor.clone(),
                self.control.clone(),
                self.control.state.clone(),
                super::super::media::MediaTarget::ExternalAgent(self.agent),
                Some(cancel.clone()),
            )
            .await?;
        if self.control.cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        self.control
            .phase(agena_domain::ExecutionPhase::StreamingModel);
        if result.phase == ToolPhase::Cancelled && cancel.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        Ok((tool, result))
    }

    async fn call_tool(
        &self,
        invocation: ToolInvocation,
        input: Value,
        cancel: &CancellationToken,
    ) -> Result<CallToolResult, AppError> {
        let (tool, result) = self.invoke_tool(invocation.clone(), input, cancel).await?;
        if result.phase != ToolPhase::Succeeded {
            let message = result
                .value
                .pointer("/failure/user/fallback")
                .and_then(Value::as_str)
                .unwrap_or("The Agena tool did not complete.");
            return Ok(text_error(message));
        }

        let mut raw: RawOutput = serde_json::from_value(result.value["raw_output"].clone())?;
        let reader = &self.manager.kernel().reader;
        let session = self.control.session;
        let stored = read_with_busy_retry(session, "external_agent_tool", || {
            reader.tool(session, tool.id())
        })
        .await?;
        let mut streamed = Vec::new();
        let mut remaining = 2 * 1024 * 1024;
        for output in &stored.outputs {
            let head = match output.kind {
                ContentKind::Text => read_with_busy_retry(session, "external_agent_text", || {
                    reader.text_range(session, output.id, None, 1)
                })
                .await?
                .stream
                .clone(),
                _ => read_with_busy_retry(session, "external_agent_events", || {
                    reader.event_page(session, output.id, None, 1)
                })
                .await?
                .stream
                .clone(),
            };
            if output.kind == ContentKind::Text
                && raw.fields.iter().any(|field| {
                    field.pointer.is_empty()
                        && field.resource.resource_id == output.id
                        && field.format == agena_domain::ContentFormat::Json
                })
                && head.committed <= 512 * 1024
            {
                let json = read_text(reader, session, &head, &mut remaining, None)
                    .await
                    .map_err(kernel_error)?;
                raw.payload = Some(serde_json::from_str(&json)?);
                continue;
            }
            if remaining > 100 * 1024 {
                let value = crate::session::kernel::input::tool_content(
                    reader,
                    session,
                    &head,
                    &mut remaining,
                )
                .await
                .map_err(kernel_error)?;
                streamed.push(value);
            }
        }
        let attachments = std::mem::take(&mut raw.attachments);
        let rendered = self.executor.render_tool_result(&invocation, &raw).await;
        let mut message = rendered
            .model
            .or_else(|| raw.payload.map(|value| value.to_string()))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| {
                result.value["summary"]
                    .as_str()
                    .unwrap_or("Tool completed.")
                    .to_owned()
            });
        for output in streamed {
            message.push_str("\n\n");
            message.push_str(&output.to_string());
        }
        let model = self.agent.run_identity();
        let mut images = Vec::new();
        let mut leases = Vec::new();
        let mut missing_images = false;
        for attachment in attachments
            .iter()
            .filter(|item| item.kind == AttachmentKind::Image)
        {
            // Use the exact tool entry's published blob. URLs and standalone
            // paths do not confer permission to load bytes in the MCP bridge.
            let hydrated = media::hydrate_attachment(
                reader,
                session,
                stored.entry_id,
                attachment,
                &model,
                run_read_budget(),
            )
            .await
            .map_err(kernel_error)?;
            if let Some(hydrated) = hydrated
                && let CompletionInputAttachmentSource::ProviderData { data, .. } =
                    hydrated.attachment.source
                && hydrated.attachment.mime.starts_with("image/")
            {
                leases.extend(hydrated.lease);
                images.push(ContentBlock::Image {
                    data,
                    mime_type: hydrated.attachment.mime,
                    annotations: None,
                    meta: None,
                });
            } else {
                missing_images = true;
            }
        }
        if missing_images {
            message.push_str(
                "\n\n[Agena could not deliver some image attachments to the external agent.]",
            );
        }
        if crate::tool::model_output_exceeds_boundary(&message, 2000, RESULT_BYTES) {
            message = crate::tool::bounded_model_output_preview(
                &message,
                "[Agena tool result preview limited.]",
                2000,
                RESULT_BYTES,
            );
        }
        let mut response = text_result(message);
        response.content.extend(images);
        Ok(response)
    }
}
