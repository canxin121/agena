use std::collections::BTreeMap;
use std::future::Future;

use agena_domain::mcp_app_tools::{MCP_APP_TOOL_MESSAGE_LIMIT, McpAppTaskInput};
use agena_domain::mcp_elicitation::{McpElicitation, McpElicitationAction, McpElicitationReply};
use agena_domain::mcp_sampling::{McpSamplingReply, McpSamplingRequest};
use rmcp::ErrorData;
use rmcp::model::{
    ClientRequest, CustomRequest, CustomResult, DEFAULT_MRTR_MAX_ROUNDS, ElicitRequestParams,
    ElicitResult, ElicitationAction, GetExtensions, InputRequest, InputRequests, InputResponses,
    ListRootsResult, ProtocolVersion, RequestId, ServerResult,
};
use rmcp::service::{
    AssociatedRequestData, Peer, PeerRequestOptions, RequestContext, RoleClient, ServiceError,
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{
    AgenaMcpClientHandler, ConnectedServer,
    tasks::{MAX_APP_TASK_INPUTS, McpTaskViewInputs, TaskRegistry},
};
use crate::{McpError, McpRedactions, McpResult};
use crate::http_transport::ScopedHttpRequest;

mod openai;
mod sampling;

pub(super) use sampling::sampling_request;

pub(super) fn openai_capabilities() -> rmcp::model::ExtensionCapabilities {
    openai::capabilities()
}

pub(super) enum RequestError {
    Wire(ServiceError),
    Client(McpError),
}

impl From<ServiceError> for RequestError {
    fn from(error: ServiceError) -> Self {
        Self::Wire(error)
    }
}

impl From<McpError> for RequestError {
    fn from(error: McpError) -> Self {
        Self::Client(error)
    }
}

impl RequestError {
    pub(super) fn http_auth_rejected(&self) -> bool {
        matches!(self, Self::Wire(error) if crate::header_helper::http_auth_rejected(error))
    }
}

impl From<RequestError> for McpError {
    fn from(error: RequestError) -> Self {
        match error {
            RequestError::Wire(error) => error.into(),
            RequestError::Client(error) => error,
        }
    }
}

/// The interaction port for one authorized MCP operation.
#[async_trait::async_trait]
pub trait McpElicitationHandler: Send + Sync {
    /// Return after cancellation has closed any outstanding user interaction.
    async fn elicit(
        &self,
        server: &str,
        request: McpElicitation,
        cancellation: CancellationToken,
    ) -> McpElicitationReply;

    async fn sample(
        &self,
        _server: &str,
        _request: McpSamplingRequest,
        _cancellation: CancellationToken,
    ) -> McpResult<McpSamplingReply> {
        Err(McpError::SamplingUnsupported)
    }
}

#[allow(deprecated)]
pub(super) enum Incoming {
    Elicitation {
        request: McpElicitation,
        cancellation: CancellationToken,
        reply: oneshot::Sender<ElicitResult>,
    },
    Sampling {
        request: McpSamplingRequest,
        cancellation: CancellationToken,
        reply: oneshot::Sender<Result<rmcp::model::CreateMessageResult, ErrorData>>,
    },
}

impl Incoming {
    fn cancellation(&self) -> &CancellationToken {
        match self {
            Self::Elicitation { cancellation, .. } | Self::Sampling { cancellation, .. } => {
                cancellation
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct IncomingRoute {
    pub(super) sender: mpsc::Sender<Incoming>,
    scope: Option<ScopedHttpRequest>,
}

impl IncomingRoute {
    pub(super) fn new(server: &ConnectedServer, sender: mpsc::Sender<Incoming>) -> Self {
        Self {
            sender,
            scope: server.peer.outbound_extension::<ScopedHttpRequest>().cloned(),
        }
    }

    pub(super) fn attach_reply_scope(&self, context: &RequestContext<RoleClient>) {
        if let Some(scope) = &self.scope {
            // A callback reply is a separate HTTP POST under this call's grant.
            context.set_reply_extension(scope.clone());
        }
    }
}

pub(super) fn associated_incoming(context: &RequestContext<RoleClient>) -> Option<&mpsc::Sender<Incoming>> {
    let route = context.extensions.get::<AssociatedRequestData>()?.get::<IncomingRoute>()?;
    route.attach_reply_scope(context);
    Some(&route.sender)
}

pub(super) async fn standard_request(
    request: ElicitRequestParams,
    context: RequestContext<RoleClient>,
    tasks: &TaskRegistry,
) -> Result<ElicitResult, ErrorData> {
    let request = decode_standard(request)
        .map_err(|_| ErrorData::invalid_params("invalid MCP elicitation", None));
    server_request(request, context, tasks).await
}

pub(super) async fn custom_request(
    request: CustomRequest,
    context: RequestContext<RoleClient>,
    tasks: &TaskRegistry,
) -> Result<CustomResult, ErrorData> {
    let request = openai::decode(request);
    serde_json::to_value(server_request(request, context, tasks).await?)
        .map(CustomResult::new)
        .map_err(|_| ErrorData::internal_error("could not encode MCP elicitation response", None))
}

async fn server_request(
    request: Result<McpElicitation, ErrorData>,
    context: RequestContext<RoleClient>,
    tasks: &TaskRegistry,
) -> Result<ElicitResult, ErrorData> {
    let task = tasks.legacy_route(&context)?;
    let sender = task.as_ref().map(|route| &route.incoming.sender)
        .or_else(|| associated_incoming(&context));
    let request = request?;
    let Some(sender) = sender else {
        return Ok(ElicitResult::new(ElicitationAction::Decline));
    };
    let reply = ask_client(sender, request, context.ct.clone()).await;
    Ok(match task {
        Some(task) => reply.with_meta(task.metadata()),
        None => reply,
    })
}

async fn ask_client(
    sender: &mpsc::Sender<Incoming>,
    request: McpElicitation,
    cancellation: CancellationToken,
) -> ElicitResult {
    let (reply, answer) = oneshot::channel();
    if sender
        .try_send(Incoming::Elicitation {
            request,
            cancellation: cancellation.clone(),
            reply,
        })
        .is_err()
    {
        return ElicitResult::new(ElicitationAction::Cancel);
    }
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => ElicitResult::new(ElicitationAction::Cancel),
        result = answer => result.unwrap_or_else(|_| ElicitResult::new(ElicitationAction::Cancel)),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputRound {
    input_requests: Option<BTreeMap<String, WireInput>>,
    request_state: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum WireInput {
    Standard(InputRequest),
    Custom(CustomRequest),
}

pub(super) enum ClientInput {
    Elicitation(McpElicitation),
    Sampling(McpSamplingRequest),
    Roots,
}

impl WireInput {
    fn decode(self) -> McpResult<ClientInput> {
        match self {
            Self::Standard(request) => ClientInput::decode_standard(request),
            Self::Custom(request) => openai::decode(request)
                .map(ClientInput::Elicitation)
                .map_err(Into::into),
        }
    }
}

impl ClientInput {
    fn decode_standard(request: InputRequest) -> McpResult<Self> {
        match request {
            InputRequest::Elicitation(request) => {
                decode_standard(request.params).map(Self::Elicitation)
            }
            InputRequest::ListRoots(_) => Ok(Self::Roots),
            InputRequest::CreateMessage(request) => {
                sampling::decode(request.params).map(Self::Sampling)
            }
            _ => Err(McpError::Malformed("unsupported MCP input request".into())),
        }
    }

    pub(super) fn for_app(&self, redactions: &McpRedactions) -> McpResult<Option<McpAppTaskInput>> {
        match self {
            Self::Elicitation(request) => app_elicitation(redactions, request.clone()),
            Self::Sampling(request) => sampling::app_input(redactions, request).map(Some),
            Self::Roots => Ok(None),
        }
    }
}

pub(super) async fn request(
    server: &ConnectedServer,
    mut request: ClientRequest,
    handler: Option<&dyn McpElicitationHandler>,
    continuation_started: &mut bool,
) -> Result<ServerResult, RequestError> {
    let mut state_only_rounds = 0;
    for round in 0..DEFAULT_MRTR_MAX_ROUNDS {
        let response = match request.clone() {
            ClientRequest::ReadResourceRequest(request) => {
                server
                    .peer
                    .read_resource_once_with(request.params, |request| {
                        exchange(server, request, handler, continuation_started)
                    })
                    .await?
            }
            request => exchange(server, request, handler, continuation_started).await?,
        };
        let required = match response {
            ServerResult::InputRequiredResult(required) => InputRound {
                input_requests: required.input_requests.map(|requests| {
                    requests
                        .into_iter()
                        .map(|(key, input)| (key, WireInput::Standard(input)))
                        .collect()
                }),
                request_state: required.request_state,
            },
            ServerResult::CustomResult(result) if result.0["resultType"] == "input_required" => {
                serde_json::from_value(result.0).map_err(McpError::from)?
            }
            response => return Ok(response),
        };
        if !server
            .peer
            .peer_info()
            .is_some_and(|info| info.protocol_version >= ProtocolVersion::V_2026_07_28)
        {
            return Err(McpError::Malformed(
                "MCP input_required requires protocol 2026-07-28".into(),
            )
            .into());
        }
        if round + 1 == DEFAULT_MRTR_MAX_ROUNDS {
            break;
        }
        *continuation_started = true;
        let requests = required.input_requests.unwrap_or_default();
        if requests.is_empty() {
            if required.request_state.is_none() {
                return Err(McpError::Malformed("empty MCP input_required result".into()).into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(
                (50_u64 << state_only_rounds.min(3)).min(250),
            ))
            .await;
            state_only_rounds += 1;
        } else {
            state_only_rounds = 0;
        }
        let responses = respond_to_inputs(server, requests, handler).await?;
        let responses = (!responses.is_empty()).then_some(responses);
        let state = required.request_state;
        match &mut request {
            ClientRequest::CallToolRequest(request) => {
                request.params.input_responses = responses;
                request.params.request_state = state;
            }
            ClientRequest::GetPromptRequest(request) => {
                request.params.input_responses = responses;
                request.params.request_state = state;
            }
            ClientRequest::ReadResourceRequest(request) => {
                request.params.input_responses = responses;
                request.params.request_state = state;
            }
            _ => {
                return Err(
                    McpError::Malformed("unexpected MCP input_required result".into()).into(),
                );
            }
        }
    }
    Err(rmcp::service::ServiceError::InputRequiredRoundsExceeded {
        max_rounds: DEFAULT_MRTR_MAX_ROUNDS,
    }
    .into())
}

pub(super) async fn respond_to_inputs(
    server: &ConnectedServer,
    requests: BTreeMap<String, WireInput>,
    handler: Option<&dyn McpElicitationHandler>,
) -> Result<InputResponses, RequestError> {
    // Reject unsupported input before opening any prompts from this round.
    respond_to_decoded_inputs(server, decode_inputs(requests)?, handler).await
}

pub(super) fn decode_inputs(
    requests: BTreeMap<String, WireInput>,
) -> McpResult<BTreeMap<String, ClientInput>> {
    requests
        .into_iter()
        .map(|(key, request)| request.decode().map(|input| (key, input)))
        .collect()
}

pub(super) async fn respond_to_decoded_inputs(
    server: &ConnectedServer,
    requests: BTreeMap<String, ClientInput>,
    handler: Option<&dyn McpElicitationHandler>,
) -> Result<InputResponses, RequestError> {
    let mut responses = InputResponses::new();
    for (key, input) in requests {
        let value = match input {
            ClientInput::Elicitation(elicitation) => {
                serde_json::to_value(continuation_input(server, elicitation, handler).await?)
            }
            ClientInput::Roots => {
                serde_json::to_value(ListRootsResult::new(server.running_roots().await))
            }
            ClientInput::Sampling(request) => {
                serde_json::to_value(sampling::continue_sampling(server, request, handler).await?)
            }
        }
        .map_err(McpError::from)?;
        responses.insert(key, value);
    }
    Ok(responses)
}

/// Resolve the client interactions requested by an App-published tool or task.
pub async fn respond_to_app_tool_inputs(
    app: &str,
    requests: InputRequests,
    handler: &dyn McpElicitationHandler,
    cancellation: &CancellationToken,
) -> McpResult<InputResponses> {
    let requests = requests
        .into_iter()
        .map(|(key, request)| ClientInput::decode_standard(request).map(|input| (key, input)))
        .collect::<McpResult<Vec<_>>>()?;
    if requests
        .iter()
        .any(|(_, input)| matches!(input, ClientInput::Roots))
    {
        return Err(McpError::CapabilityUnavailable {
            server: app.into(),
            capability: "roots".into(),
        });
    }

    let redactions = McpRedactions::default();
    let mut responses = InputResponses::new();
    for (key, input) in requests {
        let context = Some(handler);
        let token = cancellation.clone();
        let value = match input {
            ClientInput::Elicitation(request) => {
                serde_json::to_value(respond(app, &redactions, request, context, token).await?)?
            }
            ClientInput::Sampling(request) => serde_json::to_value(
                sampling::respond(app, &redactions, request, context, token).await?,
            )?,
            ClientInput::Roots => unreachable!("App input methods checked before prompting"),
        };
        responses.insert(key, value);
    }
    Ok(responses)
}

/// Handle an App's direct sampling request with the host's interaction policy.
pub async fn respond_to_app_sampling(
    app: &str,
    params: Value,
    handler: &dyn McpElicitationHandler,
    cancellation: CancellationToken,
) -> McpResult<Value> {
    let params = serde_json::from_value(params)?;
    let request = sampling::decode(params)?;
    let reply = sampling::respond(app, &McpRedactions::default(), request, Some(handler), cancellation)
        .await?;
    Ok(serde_json::to_value(reply)?)
}

async fn continuation_input(
    server: &ConnectedServer,
    request: McpElicitation,
    handler: Option<&dyn McpElicitationHandler>,
) -> Result<ElicitResult, RequestError> {
    let cancel = server.events.shutdown.child_token();
    let _cancel_on_drop = cancel.clone().drop_guard();
    let work = respond(
        &server.name,
        &server.redactions,
        request,
        handler,
        cancel.clone(),
    );
    tokio::pin!(work);
    let close = async {
        tokio::select! {
            _ = server.events.shutdown.cancelled() => {}
            _ = server.peer.transport_closed() => {}
        }
    };
    tokio::select! {
        biased;
        _ = close => {
            cancel.cancel();
            let _ = work.await;
            Err(ServiceError::TransportClosed.into())
        }
        result = &mut work => Ok(result?),
    }
}

async fn exchange(
    server: &ConnectedServer,
    mut request: ClientRequest,
    handler: Option<&dyn McpElicitationHandler>,
    continuation_started: &mut bool,
) -> Result<ServerResult, ServiceError> {
    let Some(handler) = handler else {
        return send(&server.peer, request).await;
    };
    let (sender, incoming) = mpsc::channel::<Incoming>(8);
    request
        .extensions_mut()
        .insert(AssociatedRequestData::new(IncomingRoute::new(server, sender)));
    with_incoming(
        server,
        Some(handler),
        None,
        incoming,
        send(&server.peer, request),
        continuation_started,
    )
    .await
}

pub(super) async fn send(
    peer: &Peer<RoleClient>,
    request: ClientRequest,
) -> Result<ServerResult, ServiceError> {
    let request = peer
        .send_cancellable_request(request, PeerRequestOptions::no_options())
        .await?;
    let mut pending = PendingRequest {
        peer: peer.clone(),
        id: Some(request.id.clone()),
    };
    let result = request.await_response().await;
    pending.id = None;
    result
}

pub(super) async fn with_incoming<T: Send>(
    server: &ConnectedServer,
    handler: Option<&dyn McpElicitationHandler>,
    legacy_view: Option<&dyn McpTaskViewInputs>,
    mut incoming: mpsc::Receiver<Incoming>,
    response: impl Future<Output = T> + Send,
    continuation_started: &mut bool,
) -> T {
    let mut callbacks = 0;
    tokio::pin!(response);
    loop {
        let input = tokio::select! {
            biased;
            result = &mut response => return result,
            Some(input) = incoming.recv() => input,
        };
        *continuation_started = true;
        let cancel = input.cancellation().child_token();
        let _cancel_on_drop = cancel.clone().drop_guard();
        let view = legacy_view.filter(|_| callbacks < MAX_APP_TASK_INPUTS);
        callbacks += usize::from(view.is_some());
        let work = async {
            match input {
                Incoming::Elicitation { request, reply, .. } => {
                    let bridged = relay_legacy_input(view, &cancel, || {
                        app_elicitation(&server.redactions, request.clone())
                    })
                    .await;
                    let result = match bridged {
                        Some(answer) => answer,
                        None => respond(
                            &server.name,
                            &server.redactions,
                            request,
                            handler,
                            cancel.clone(),
                        )
                        .await
                        .unwrap_or_else(|_| ElicitResult::new(ElicitationAction::Decline)),
                    };
                    let _ = reply.send(result);
                }
                Incoming::Sampling { request, reply, .. } => {
                    let bridged = relay_legacy_input(view, &cancel, || {
                        sampling::app_input(&server.redactions, &request).map(Some)
                    })
                    .await;
                    let result = match bridged {
                        Some(answer) => Ok(answer),
                        None => {
                            sampling::respond(
                                &server.name,
                                &server.redactions,
                                request,
                                handler,
                                cancel.clone(),
                            )
                            .await
                        }
                    };
                    let _ = reply.send(result.map_err(sampling::wire_error));
                }
            }
        };
        tokio::pin!(work);
        tokio::select! {
            biased;
            result = &mut response => {
                cancel.cancel();
                // The interaction port settles its pending user request on
                // cancellation before this tool or resource operation ends.
                let _ = work.await;
                return result;
            }
            () = &mut work => {}
        }
    }
}

// Legacy requests carry a reply channel tied to this exact invocation. Only
// the sanitized request crosses to the App; reply delivery stays on that channel.
async fn relay_legacy_input<T: DeserializeOwned>(
    view: Option<&dyn McpTaskViewInputs>,
    cancellation: &CancellationToken,
    project: impl FnOnce() -> McpResult<Option<McpAppTaskInput>>,
) -> Option<T> {
    let view = view?;
    let input = project().ok()??;
    if serde_json::to_vec(&input.request).ok()?.len() > MCP_APP_TOOL_MESSAGE_LIMIT - 2048 {
        return None;
    }
    let key = uuid::Uuid::new_v4().to_string();
    let mut responses = view
        .wait_for_inputs(BTreeMap::from([(key.clone(), input)]), cancellation)
        .await?;
    serde_json::from_value(responses.remove(&key)?).ok()
}

struct PendingRequest {
    peer: Peer<RoleClient>,
    id: Option<RequestId>,
}

impl Drop for PendingRequest {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            self.peer
                .try_cancel_request(id, Some("client operation ended".into()));
        }
    }
}

fn decode_standard(request: ElicitRequestParams) -> McpResult<McpElicitation> {
    match request {
        ElicitRequestParams::FormElicitationParams {
            message,
            requested_schema,
            ..
        } => Ok(McpElicitation::Form {
            message,
            requested_schema: serde_json::to_value(requested_schema)?,
        }),
        ElicitRequestParams::UrlElicitationParams {
            message,
            url,
            elicitation_id,
            ..
        } => Ok(McpElicitation::Url {
            message,
            url,
            elicitation_id,
        }),
        _ => Err(McpError::Malformed(
            "unsupported MCP elicitation mode".into(),
        )),
    }
}

struct NoExternalSchemas;

impl jsonschema::Retrieve for NoExternalSchemas {
    fn retrieve(
        &self,
        _: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err("MCP elicitation schema references must resolve locally".into())
    }
}

enum ElicitationValidation {
    Form(jsonschema::Validator),
    Url,
}

impl ElicitationValidation {
    fn response(&self, action: ElicitationAction, content: Option<Value>) -> Option<ElicitResult> {
        let mut result = ElicitResult::new(action);
        if result.action == ElicitationAction::Accept
            && let Self::Form(schema) = self
        {
            let content = content.unwrap_or_else(|| json!({}));
            if !schema.is_valid(&content) {
                return None;
            }
            result.content = Some(content);
        }
        Some(result)
    }
}

fn prepare_elicitation(
    redactions: &McpRedactions,
    request: &mut McpElicitation,
) -> McpResult<Option<ElicitationValidation>> {
    let validation = match request {
        McpElicitation::Form {
            message,
            requested_schema,
        } => {
            if redactions.payload(requested_schema.clone())? != *requested_schema {
                return Ok(None);
            }
            let Ok(validator) = jsonschema::options()
                .with_retriever(NoExternalSchemas)
                .should_validate_formats(true)
                .build(requested_schema)
            else {
                return Ok(None);
            };
            *message = redactions.text(std::mem::take(message));
            ElicitationValidation::Form(validator)
        }
        McpElicitation::Url {
            message,
            url,
            elicitation_id,
        } => {
            if url.chars().any(char::is_control)
                || redactions.text(url.clone()) != *url
                || !url::Url::parse(url).is_ok_and(|url| {
                    matches!(url.scheme(), "https" | "http")
                        && url.host().is_some()
                        && url.username().is_empty()
                        && url.password().is_none()
                })
            {
                return Ok(None);
            }
            *message = redactions.text(std::mem::take(message));
            *elicitation_id = redactions.text(std::mem::take(elicitation_id));
            ElicitationValidation::Url
        }
    };
    Ok(Some(validation))
}

fn app_elicitation(
    redactions: &McpRedactions,
    mut request: McpElicitation,
) -> McpResult<Option<McpAppTaskInput>> {
    let Some(validation) = prepare_elicitation(redactions, &mut request)? else {
        return Ok(None);
    };
    let params = match request {
        McpElicitation::Form {
            message,
            requested_schema,
        } => {
            json!({"mode":"form", "message":message, "requestedSchema":requested_schema})
        }
        McpElicitation::Url {
            message,
            url,
            elicitation_id,
        } => {
            json!({"mode":"url", "message":message, "url":url, "elicitationId":elicitation_id})
        }
    };
    Ok(Some(McpAppTaskInput::new(
        json!({"method":"elicitation/create", "params":params}),
        move |value| {
            let result: ElicitResult = serde_json::from_value(value).ok()?;
            let result = validation.response(result.action, result.content)?;
            serde_json::to_value(result).ok()
        },
    )))
}

async fn respond(
    server: &str,
    redactions: &McpRedactions,
    mut request: McpElicitation,
    handler: Option<&dyn McpElicitationHandler>,
    cancellation: CancellationToken,
) -> McpResult<ElicitResult> {
    let Some(handler) = handler else {
        return Ok(ElicitResult::new(ElicitationAction::Decline));
    };
    if cancellation.is_cancelled() {
        return Ok(ElicitResult::new(ElicitationAction::Cancel));
    }
    let Some(validation) = prepare_elicitation(redactions, &mut request)? else {
        return Ok(ElicitResult::new(ElicitationAction::Decline));
    };
    let answer = handler.elicit(server, request, cancellation.clone()).await;
    if cancellation.is_cancelled() {
        return Ok(ElicitResult::new(ElicitationAction::Cancel));
    }
    let action = match answer.action {
        McpElicitationAction::Accept => ElicitationAction::Accept,
        McpElicitationAction::Decline => ElicitationAction::Decline,
        McpElicitationAction::Cancel => ElicitationAction::Cancel,
    };
    Ok(validation
        .response(action, answer.content)
        .unwrap_or_else(|| ElicitResult::new(ElicitationAction::Decline)))
}

impl ConnectedServer {
    async fn running_roots(&self) -> Vec<rmcp::model::Root> {
        let handler: Option<AgenaMcpClientHandler> = self
            .running
            .lock()
            .await
            .as_ref()
            .map(|running| running.service().clone());
        match handler {
            Some(handler) => handler.roots.read().await.clone(),
            None => Vec::new(),
        }
    }
}
