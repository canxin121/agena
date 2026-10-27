//! A resumable provider stream over kernel capabilities. The pending FIFO and
//! in-flight future belong to this run, so retrying persistence never starts a
//! second provider request or repeats an accepted append.

use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use agena_domain::{
    AttachmentItem, AttachmentKind, AttachmentSource, ModelRef, RawOutput, ToolInvocation,
};
use agena_provider::{
    CompletionRequest, CompletionStreamEvent, CompletionUsage, ProviderNativeToolOutputBlock,
    merge_provider_metadata, normalize_optional_text,
};
use agena_storage::kernel::*;
use futures_util::{StreamExt, future::BoxFuture};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{KernelPrompt, KernelProviderState, reference_replay_text};
use crate::session::processor::{
    ParsedToolApiCall, PendingProviderNativeToolCall, PendingToolCall,
    merge_provider_native_tool_invocation, message_provider_state_from_provider_metadata,
    parse_tool_api_call_lossy, pending_provider_native_tool_call_stream_key,
    pending_tool_call_stream_key,
};
use crate::{AppError, SessionProcessor, provider::ProviderRegistry};

const MAX_TOOLS: usize = 128;
const MAX_STREAMS: usize = 256;
const MAX_ALIASES: usize = 2048;
const MAX_ARGUMENT_BYTES: usize = 512 * 1024;
const MAX_STATE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HOSTED_IMAGE_COUNT: usize = 8;
// The session media download endpoint serves at most 20 MiB per artifact.
const MAX_HOSTED_IMAGE_DECODED_BYTES: usize = 20 * 1024 * 1024;
const MAX_HOSTED_IMAGE_ENCODED_BYTES: usize = MAX_HOSTED_IMAGE_DECODED_BYTES * 4 / 3 + 16 * 1024;
static RUN_BUDGET: OnceLock<ReadBudget> = OnceLock::new();
type CompletionEventStream = std::pin::Pin<
    Box<
        dyn futures_util::Stream<
                Item = std::result::Result<CompletionStreamEvent, crate::provider::ProviderError>,
            > + Send,
    >,
>;

/// Parameters are fixed before the first provider effect. The model input can
/// only come from the round's own frozen, route-bound KernelPrompt.
pub struct KernelRoundRequest {
    round: RoundHandle,
    prompt: KernelPrompt,
    model: ModelRef,
    completion: CompletionRequest,
    cancel: Option<CancellationToken>,
}

impl KernelRoundRequest {
    pub fn new(
        round: RoundHandle,
        mut prompt: KernelPrompt,
        mut completion: CompletionRequest,
        cancel: Option<CancellationToken>,
    ) -> Result<Self> {
        if round.id() != prompt.facts().round_id
            || round.session_id() != prompt.facts().metadata.id
            || round.execution_id() != prompt.facts().execution_id
            || round.generation() != prompt.facts().generation
        {
            return Err(KernelError::Ownership);
        }
        let model = prompt.model().clone();
        prompt.install(&mut completion);
        Ok(Self {
            round,
            prompt,
            model,
            completion,
            cancel,
        })
    }
}

#[derive(Clone)]
pub struct KernelToolApiCall {
    pub tool: ToolHandle,
    pub call: agena_domain::ToolApiCall,
    pub error: Option<agena_tool_api::ToolApiError>,
    /// Digest of the provider's exact function name and arguments. It keeps
    /// malformed calls distinct even when protocol parsing retains no arguments.
    pub signature: String,
}

/// A completed provider round, not the surrounding execution's outcome. Local
/// tools are returned only after their arguments and the round have committed.
#[derive(Clone)]
pub struct KernelRoundOutcome {
    pub round_id: RoundId,
    pub failure: Option<agena_failure::Failure>,
    pub cancelled: bool,
    pub follow_up_requested: bool,
    pub finish_reason: agena_domain::FinishReason,
    pub usage: Option<CompletionUsage>,
    pub tools: Vec<KernelToolApiCall>,
}

/// Stable identity of a provider tool call for the no-progress guard. It is
/// computed from the raw provider arguments before lossy parsing, so two
/// distinct malformed `tools_call` payloads retain distinct signatures.
fn tool_call_dedupe_signature(name: &str, arguments_json: &str) -> String {
    use sha2::Digest as _;

    let normalized = serde_json::from_str::<Value>(arguments_json)
        .ok()
        .and_then(|value| serde_json::to_string(&value).ok())
        .unwrap_or_else(|| arguments_json.to_owned());
    let mut digest = sha2::Sha256::new();
    digest.update(name.as_bytes());
    digest.update([0]);
    digest.update(normalized.as_bytes());
    hex::encode(digest.finalize())
}

struct TextStream {
    reasoning: bool,
    output: Option<TextOutput>,
    head: Option<ContentStreamView>,
}
struct Draft {
    handle: Option<ToolDraftHandle>,
    ready: Option<ToolHandle>,
    protocol: Option<ParsedToolApiCall>,
    signature: Option<String>,
}
struct Hosted {
    handle: Option<ToolHandle>,
    completed: bool,
}

struct Step {
    request: RequestId,
    action: Action,
}
impl Step {
    fn new(action: Action) -> Self {
        Self {
            request: RequestId::new(),
            action,
        }
    }
}
enum Action {
    OpenText {
        slot: usize,
        reasoning: bool,
    },
    Append {
        slot: usize,
        text: String,
    },
    Seal {
        slot: usize,
        diagnostic: Option<String>,
    },
    OpenDraft {
        slot: usize,
        call_id: String,
        name: String,
    },
    Finalize {
        slot: usize,
        input: Value,
    },
    OpenHosted {
        slot: usize,
        call: ToolStart,
    },
    CompleteHosted {
        slot: usize,
        result: ToolResult,
        blocks: Arc<Vec<ProviderNativeToolOutputBlock>>,
        _media_lease: Option<ReadLease>,
    },
    Finish {
        state: Value,
        succeeded: bool,
    },
}
enum Applied {
    OpenText(usize, TextOutput),
    Append,
    Seal(usize, ContentStreamView),
    OpenDraft(usize, ToolDraftHandle),
    Finalize(usize, ToolHandle),
    OpenHosted(usize, ToolHandle),
    CompleteHosted(usize),
    Finish,
}

/// Keep this object until drive() succeeds or its owning attempt is explicitly
/// cancelled. A storage error leaves the exact command and request ID pending.
/// Dropping a drive future is safe: an in-flight append future stays here and
/// is polled again on the next drive instead of submitting its bytes twice.
/// Dropping the entire run abandons capture; kernel supervision/recovery owns
/// interruption and never resumes external provider effects automatically.
pub struct KernelRoundRun {
    round: RoundHandle,
    reader: KernelReader,
    model: ModelRef,
    workspace_root: PathBuf,
    _prompt: KernelPrompt,
    _lease: ReadLease,
    cancel: Option<CancellationToken>,
    provider: Option<BoxFuture<'static, std::result::Result<CompletionEventStream, AppError>>>,
    stream: Option<CompletionEventStream>,
    definitions: Vec<agena_provider::ToolApiDefinition>,
    texts: Vec<TextStream>,
    active_text: Option<usize>,
    calls: BTreeMap<String, PendingToolCall>,
    aliases: BTreeMap<String, String>,
    drafts: Vec<Draft>,
    native: BTreeMap<String, PendingProviderNativeToolCall>,
    native_aliases: BTreeMap<String, String>,
    hosted: Vec<Hosted>,
    steps: VecDeque<Step>,
    in_flight: Option<BoxFuture<'static, Result<Applied>>>,
    replay_read: Option<BoxFuture<'static, Result<Value>>>,
    provider_metadata: Option<Value>,
    usage: Option<CompletionUsage>,
    finish_reason: agena_domain::FinishReason,
    follow_up_requested: bool,
    failure: Option<agena_failure::Failure>,
    cancelled: bool,
    visible_text: bool,
    substantive_reasoning: bool,
    ended: bool,
    terminal_queued: bool,
    outcome: Option<KernelRoundOutcome>,
}

impl SessionProcessor {
    /// Use the same provider registry and chat.params hook as the production
    /// processor, with kernel identities and writers replacing Part mutation.
    /// Constructing a run performs no provider call; drive owns that future.
    pub fn kernel_round(
        &self,
        request: KernelRoundRequest,
        reader: KernelReader,
        providers: Arc<ProviderRegistry>,
        workspace_root: PathBuf,
    ) -> Result<KernelRoundRun> {
        let session = request.prompt.facts().metadata.id;
        let lease = RUN_BUDGET
            .get_or_init(ReadBudget::default)
            .reserve_view(session, 32 * 1024 * 1024)?;
        let processor = self.clone();
        let KernelRoundRequest {
            round,
            prompt,
            model,
            mut completion,
            cancel,
        } = request;
        let definitions = completion.tool_api_functions.clone();
        let route = model.clone();
        let cancellation = cancel.clone();
        let provider = Box::pin(async move {
            processor
                .apply_chat_params_hook(
                    route.provider_id.as_ref(),
                    route.model_id.as_ref(),
                    session.get(),
                    &mut completion,
                    cancellation.clone(),
                )
                .await;
            if cancellation
                .as_ref()
                .is_some_and(CancellationToken::is_cancelled)
            {
                return Err(AppError::Cancelled);
            }
            let request = crate::provider::with_request_cancellation(
                cancellation.clone(),
                providers.complete_stream_with_tool_drafts(&route, completion),
            );
            let result = match cancellation.as_ref() {
                Some(cancel) => {
                    tokio::select! { biased; _ = cancel.cancelled() => return Err(AppError::Cancelled), result = request => result }
                }
                None => request.await,
            };
            crate::record_provider_stream();
            crate::record_provider_call(result.is_ok());
            result.map_err(AppError::from)
        });
        Ok(KernelRoundRun {
            round,
            reader,
            model,
            workspace_root,
            _prompt: prompt,
            _lease: lease,
            cancel,
            provider: Some(provider),
            stream: None,
            definitions,
            texts: Vec::new(),
            active_text: None,
            calls: BTreeMap::new(),
            aliases: BTreeMap::new(),
            drafts: Vec::new(),
            native: BTreeMap::new(),
            native_aliases: BTreeMap::new(),
            hosted: Vec::new(),
            steps: VecDeque::new(),
            in_flight: None,
            replay_read: None,
            provider_metadata: None,
            usage: None,
            finish_reason: agena_domain::FinishReason::Stop,
            follow_up_requested: false,
            failure: None,
            cancelled: false,
            visible_text: false,
            substantive_reasoning: false,
            ended: false,
            terminal_queued: false,
            outcome: None,
        })
    }
}

impl KernelRoundRun {
    pub fn round_id(&self) -> RoundId {
        self.round.id()
    }
    pub fn outcome(&self) -> Option<&KernelRoundOutcome> {
        self.outcome.as_ref()
    }

    /// Kernel errors are persistence/admission errors; retry this same object.
    /// Provider/protocol failure is a committed outcome, available after drive.
    pub async fn drive(&mut self) -> Result<&KernelRoundOutcome> {
        while self.outcome.is_none() {
            if !self.steps.is_empty() {
                self.apply_front().await?;
                continue;
            }
            if self.ended {
                if !self.terminal_queued {
                    self.queue_terminal();
                    continue;
                }
                if let Err(error) = self.queue_finish().await {
                    if self.failure.is_none()
                        && matches!(
                            error,
                            KernelError::Invalid(_)
                                | KernelError::ContentReset
                                | KernelError::ContentGap { .. }
                        )
                    {
                        self.failure = Some(
                            protocol(format!("provider replay could not be committed: {error}"))
                                .failure(),
                        );
                    } else {
                        return Err(error);
                    }
                }
                continue;
            }
            if let Some(provider) = self.provider.as_mut() {
                let result = provider.await;
                self.provider = None;
                match result {
                    Ok(stream) => self.stream = Some(stream),
                    Err(error) => {
                        self.end(Some(error));
                        continue;
                    }
                }
            }
            let Some(stream) = self.stream.as_mut() else {
                continue;
            };
            let next =
                crate::provider::with_request_cancellation(self.cancel.clone(), stream.next());
            let item = match self.cancel.as_ref() {
                Some(cancel) => {
                    tokio::select! { biased; _ = cancel.cancelled() => None, item = next => item }
                }
                None => next.await,
            };
            match item {
                None => self.end(None),
                Some(Err(error)) => self.end(Some(error.into())),
                Some(Ok(event)) => {
                    if let Err(error) = self.queue_event(event) {
                        self.end(Some(error));
                    }
                }
            }
        }
        Ok(self.outcome.as_ref().expect("finished outcome"))
    }

    fn end(&mut self, error: Option<AppError>) {
        self.stream = None;
        self.provider = None;
        self.ended = true;
        self.cancelled = self
            .cancel
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
            || matches!(error, Some(AppError::Cancelled));
        let error = if self.cancelled {
            Some(AppError::Cancelled)
        } else {
            error
        };
        self.failure = error.map(|error| error.failure());
    }

    fn seal_active(&mut self) {
        if let Some(slot) = self.active_text.take() {
            self.steps.push_back(Step::new(Action::Seal {
                slot,
                diagnostic: None,
            }));
        }
    }

    fn text_delta(&mut self, delta: String, reasoning: bool) -> std::result::Result<(), AppError> {
        if delta.is_empty() {
            return Ok(());
        }
        if reasoning {
            self.substantive_reasoning |=
                !delta.trim().is_empty() && delta.trim() != "(no reasoning recorded)";
        } else {
            self.visible_text |= !delta.trim().is_empty();
        }
        if self
            .active_text
            .is_some_and(|slot| self.texts[slot].reasoning != reasoning)
        {
            self.seal_active();
        }
        let slot = match self.active_text {
            Some(slot) => slot,
            None => {
                if self.texts.len() >= MAX_STREAMS {
                    return Err(protocol("provider round has too many text segments"));
                }
                let slot = self.texts.len();
                self.texts.push(TextStream {
                    reasoning,
                    output: None,
                    head: None,
                });
                self.active_text = Some(slot);
                self.steps
                    .push_back(Step::new(Action::OpenText { slot, reasoning }));
                slot
            }
        };
        let mut start = 0;
        while start < delta.len() {
            let mut end = (start + 256 * 1024).min(delta.len());
            while !delta.is_char_boundary(end) {
                end -= 1;
            }
            self.steps.push_back(Step::new(Action::Append {
                slot,
                text: delta[start..end].to_owned(),
            }));
            start = end;
        }
        Ok(())
    }

    fn queue_event(&mut self, event: CompletionStreamEvent) -> std::result::Result<(), AppError> {
        check_provider_event_budget(&event)?;
        match event {
            CompletionStreamEvent::TextDelta { delta, .. } => self.text_delta(delta, false)?,
            CompletionStreamEvent::ThinkingDelta { delta, .. } => self.text_delta(delta, true)?,
            CompletionStreamEvent::ToolCallDelta {
                stream_key,
                id,
                name,
                arguments_delta,
                ..
            } => self.call_update(stream_key, id, name, arguments_delta, false)?,
            CompletionStreamEvent::ToolCallSnapshot {
                stream_key,
                id,
                name,
                arguments_json,
                ..
            } => self.call_update(stream_key, id, name, arguments_json, true)?,
            CompletionStreamEvent::ProviderNativeToolCallStarted {
                stream_key,
                id,
                invocation,
                raw,
                ..
            } => {
                self.hosted_update(stream_key, id, invocation, raw, None)?;
            }
            CompletionStreamEvent::ProviderNativeToolCallCompleted {
                stream_key,
                id,
                invocation,
                title,
                summary,
                failure,
                output_text,
                blocks,
                details,
                raw,
                ..
            } => {
                let phase = if failure.is_some() {
                    ToolPhase::Failed
                } else {
                    ToolPhase::Succeeded
                };
                let result = ToolResult {
                    phase,
                    value: json!({"title":title,"summary":summary,"failure":failure,"output_text":output_text,"details":details}),
                };
                self.hosted_update(stream_key, id, invocation, raw, Some((result, blocks)))?;
            }
            CompletionStreamEvent::Completed {
                finish_reason,
                usage,
                provider_metadata,
                end_turn,
                ..
            } => {
                if let Some(reason) = finish_reason {
                    self.finish_reason = crate::session::processor::map_finish_reason(&reason);
                }
                if usage
                    .as_ref()
                    .is_some_and(|usage| usage != &CompletionUsage::default())
                {
                    self.usage = usage;
                }
                let merged =
                    merge_provider_metadata(self.provider_metadata.clone(), provider_metadata);
                check_envelope(&merged)?;
                self.provider_metadata = merged;
                if let Some(end_turn) = end_turn {
                    self.follow_up_requested = !end_turn;
                }
            }
            // These are live request progress, not transcript/replay content.
            // The manager projects the run's retry status separately.
            CompletionStreamEvent::ProviderRetry { .. }
            | CompletionStreamEvent::ProviderRetryStarted { .. } => {}
        }
        let arguments = self
            .calls
            .values()
            .map(|call| call.arguments_json.len().saturating_mul(2) + 1024)
            .sum::<usize>();
        let metadata =
            modeled_read_bytes(&self.provider_metadata).map_err(|e| protocol(e.to_string()))?;
        let native = self
            .native
            .values()
            .map(|call| modeled_read_bytes(&call.raw).unwrap_or(usize::MAX))
            .fold(0usize, usize::saturating_add);
        if arguments.saturating_add(metadata).saturating_add(native) > MAX_STATE_BYTES {
            return Err(protocol(
                "provider round state exceeds its allocation budget",
            ));
        }
        Ok(())
    }

    fn call_update(
        &mut self,
        stream_key: String,
        id: Option<String>,
        name: Option<String>,
        arguments: String,
        snapshot: bool,
    ) -> std::result::Result<(), AppError> {
        self.seal_active();
        if self.aliases.len() >= MAX_ALIASES {
            return Err(protocol("provider call aliases exceed the round limit"));
        }
        let key = pending_tool_call_stream_key(
            &mut self.calls,
            &mut self.aliases,
            stream_key,
            id.as_deref(),
        );
        if self.calls.len() >= MAX_TOOLS && !self.calls.contains_key(&key) {
            return Err(protocol("provider round has too many local calls"));
        }
        let call = self.calls.entry(key).or_default();
        if let Some(id) = normalize_optional_text(id) {
            call.id = Some(id);
        }
        if let Some(name) = normalize_optional_text(name) {
            if call.call_id.is_some() && call.name.as_ref().is_some_and(|old| old != &name) {
                return Err(protocol("provider changed an established tool name"));
            }
            call.name = Some(name);
        }
        let degenerate = arguments.trim().is_empty() || arguments.trim() == "{}";
        if !snapshot || !degenerate {
            if arguments.len().saturating_add(if snapshot {
                0
            } else {
                call.arguments_json.len()
            }) > MAX_ARGUMENT_BYTES
            {
                return Err(protocol("tool arguments exceed the round limit"));
            }
            if snapshot {
                call.arguments_json = arguments;
            } else {
                call.arguments_json.push_str(&arguments);
            }
        }
        if call.call_id.is_none()
            && let (Some(id), Some(name)) = (call.id.clone(), call.name.clone())
        {
            let slot = self.drafts.len();
            self.drafts.push(Draft {
                handle: None,
                ready: None,
                protocol: None,
                signature: None,
            });
            call.call_id = Some(slot as i64);
            self.steps.push_back(Step::new(Action::OpenDraft {
                slot,
                call_id: id,
                name,
            }));
        }
        Ok(())
    }

    fn hosted_update(
        &mut self,
        stream_key: String,
        id: Option<String>,
        invocation: ToolInvocation,
        raw: Option<Value>,
        result: Option<(ToolResult, Vec<ProviderNativeToolOutputBlock>)>,
    ) -> std::result::Result<(), AppError> {
        self.seal_active();
        if self.native_aliases.len() >= MAX_ALIASES {
            return Err(protocol("provider hosted aliases exceed the round limit"));
        }
        let key = pending_provider_native_tool_call_stream_key(
            &mut self.native,
            &mut self.native_aliases,
            stream_key,
            id.as_deref(),
        );
        if self.native.len() >= MAX_TOOLS && !self.native.contains_key(&key) {
            return Err(protocol("provider round has too many hosted calls"));
        }
        let pending = self.native.entry(key.clone()).or_default();
        if let Some(id) = normalize_optional_text(id) {
            pending.id = Some(id);
        }
        pending.invocation = Some(merge_provider_native_tool_invocation(
            pending.invocation.as_ref(),
            invocation,
        ));
        pending.raw = merge_provider_metadata(pending.raw.take(), raw);
        let invocation = pending.invocation.as_ref().expect("merged invocation");
        if invocation.name.trim().is_empty() {
            return Err(protocol("hosted tool has no name"));
        }
        let slot = match pending.call_id {
            Some(slot) => slot as usize,
            None => {
                let slot = self.hosted.len();
                self.hosted.push(Hosted {
                    handle: None,
                    completed: false,
                });
                pending.call_id = Some(slot as i64);
                self.steps.push_back(Step::new(Action::OpenHosted {
                    slot,
                    call: ToolStart {
                        call_id: pending
                            .id
                            .clone()
                            .unwrap_or_else(|| format!("hosted:{key}")),
                        name: invocation.name.clone(),
                        input: serde_json::to_value(invocation)
                            .map_err(|e| protocol(e.to_string()))?,
                    },
                }));
                slot
            }
        };
        if self.hosted[slot].completed {
            return Ok(());
        }
        if let Some((mut result, blocks)) = result {
            result.value["invocation"] =
                serde_json::to_value(invocation).map_err(|e| protocol(e.to_string()))?;
            result.value["provider_metadata"] = pending.raw.clone().unwrap_or(Value::Null);
            // The persisted envelope is control data, not a container for
            // inline base64 media. Materialize hosted images before completion.
            result.value["blocks"] = serde_json::to_value(compact_hosted_blocks(&blocks))
                .map_err(|e| protocol(e.to_string()))?;
            check_envelope(&result)?;
            let inline_bytes = blocks
                .iter()
                .filter_map(|block| match block {
                    ProviderNativeToolOutputBlock::Media { artifact, .. }
                        if is_inline_hosted_image(block) =>
                    {
                        Some(artifact.uri.len())
                    }
                    _ => None,
                })
                .sum::<usize>();
            let media_lease = if inline_bytes == 0 {
                None
            } else {
                Some(
                    RUN_BUDGET
                        .get_or_init(ReadBudget::default)
                        .reserve_view(self.round.session_id(), inline_bytes.saturating_mul(2))
                        .map_err(|e| {
                            protocol(format!("provider image allocation budget exhausted: {e}"))
                        })?,
                )
            };
            self.steps.push_back(Step::new(Action::CompleteHosted {
                slot,
                result,
                blocks: Arc::new(blocks),
                _media_lease: media_lease,
            }));
        }
        Ok(())
    }

    fn queue_terminal(&mut self) {
        if self.failure.is_none() {
            let validation = self.validate_calls();
            if let Err(error) = validation {
                self.failure = Some(error.failure());
            }
        }
        if self.failure.is_none() && self.hosted.iter().any(|hosted| !hosted.completed) {
            self.failure =
                Some(protocol("provider stream ended with unfinished hosted calls").failure());
        }
        if self.failure.is_none()
            && !self.visible_text
            && !self.substantive_reasoning
            && self.calls.is_empty()
            && self.hosted.is_empty()
        {
            self.failure = Some(AppError::EmptyResponse.failure());
        }
        if let Some(slot) = self.active_text.take() {
            let diagnostic = self
                .failure
                .as_ref()
                .map(|_| "provider round did not complete".to_owned());
            self.steps
                .push_back(Step::new(Action::Seal { slot, diagnostic }));
        }
        if self.failure.is_some() {
            for (slot, hosted) in self
                .hosted
                .iter()
                .enumerate()
                .filter(|(_, hosted)| !hosted.completed)
            {
                if hosted.handle.is_some() {
                    self.steps.push_back(Step::new(Action::CompleteHosted {
                        slot,
                        result: ToolResult {
                            phase: if self.cancelled {
                                ToolPhase::Cancelled
                            } else {
                                ToolPhase::Interrupted
                            },
                            value: json!({"failure":self.failure}),
                        },
                        blocks: Arc::new(Vec::new()),
                        _media_lease: None,
                    }));
                }
            }
        }
        self.terminal_queued = true;
    }

    fn validate_calls(&mut self) -> std::result::Result<(), AppError> {
        for call in self.calls.values() {
            let (Some(slot), Some(name), Some(_id)) =
                (call.call_id, call.name.as_deref(), call.id.as_ref())
            else {
                return Err(protocol(
                    "provider stream ended a tool call without its identity/name",
                ));
            };
            self.drafts[slot as usize].protocol = Some(parse_tool_api_call_lossy(
                name,
                &call.arguments_json,
                &self.definitions,
            )?);
            self.drafts[slot as usize].signature =
                Some(tool_call_dedupe_signature(name, &call.arguments_json));
        }
        Ok(())
    }

    async fn queue_finish(&mut self) -> Result<()> {
        if self.replay_read.is_none() {
            let reader = self.reader.clone();
            let session = self._prompt.facts().metadata.id;
            let model = self.model.clone();
            let usage = self.usage.clone();
            let finish_reason = self.finish_reason;
            let follow_up_requested = self.follow_up_requested;
            let failure = self.failure.clone();
            let mut replay = self.provider_metadata.clone().unwrap_or_else(|| json!({}));
            if !replay.is_object() {
                replay = json!({"provider_metadata":replay});
            }
            if let Some(normalized) = message_provider_state_from_provider_metadata(&replay) {
                let normalized = serde_json::to_value(normalized)
                    .map_err(|e| KernelError::Invalid(e.to_string()))?;
                let object = replay.as_object_mut().ok_or_else(|| {
                    KernelError::Invalid("provider metadata must be an object".into())
                })?;
                if let Some(normalized) = normalized.as_object() {
                    object.extend(normalized.clone());
                }
            }
            let heads = if self.failure.is_none() {
                self.texts
                    .iter()
                    .filter(|text| text.reasoning)
                    .filter_map(|text| text.head.clone())
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            self.replay_read = Some(Box::pin(async move {
                let mut sources = Vec::new();
                let mut remaining = MAX_STATE_BYTES;
                for head in heads {
                    let text =
                        super::input::read_text(&reader, session, &head, &mut remaining, None)
                            .await?;
                    sources.push((head, text));
                }
                let replay = if failure.is_none() {
                    reference_replay_text(&replay, &sources)?
                } else {
                    replay
                };
                let mut state = KernelProviderState::new(model, replay);
                state.usage = usage;
                state.finish_reason = Some(finish_reason);
                state.follow_up_requested = follow_up_requested;
                state.failure = failure;
                serde_json::to_value(state).map_err(|e| KernelError::Invalid(e.to_string()))
            }));
        }
        let state = self.replay_read.as_mut().expect("replay future").await;
        self.replay_read = None;
        let state = state?;
        if self.failure.is_none() {
            for call in self.calls.values() {
                let slot = call.call_id.expect("validated draft") as usize;
                let input = match serde_json::from_str::<Value>(&call.arguments_json) {
                    Ok(value) if value.is_object() => value,
                    _ => serde_json::to_value(
                        &self.drafts[slot]
                            .protocol
                            .as_ref()
                            .expect("validated protocol call")
                            .call
                            .arguments,
                    )
                    .map_err(|error| KernelError::Invalid(error.to_string()))?,
                };
                self.steps
                    .push_back(Step::new(Action::Finalize { slot, input }));
            }
        }
        self.steps.push_back(Step::new(Action::Finish {
            state,
            succeeded: self.failure.is_none(),
        }));
        Ok(())
    }

    async fn apply_front(&mut self) -> Result<()> {
        if self.in_flight.is_none() {
            self.in_flight = Some(self.start_front()?);
        }
        let result = self.in_flight.as_mut().expect("pending operation").await;
        self.in_flight = None;
        match result? {
            Applied::OpenText(slot, output) => self.texts[slot].output = Some(output),
            Applied::Append => {}
            Applied::Seal(slot, head) => self.texts[slot].head = Some(head),
            Applied::OpenDraft(slot, handle) => self.drafts[slot].handle = Some(handle),
            Applied::Finalize(slot, handle) => self.drafts[slot].ready = Some(handle),
            Applied::OpenHosted(slot, handle) => self.hosted[slot].handle = Some(handle),
            Applied::CompleteHosted(slot) => self.hosted[slot].completed = true,
            Applied::Finish => {
                let tools = if self.failure.is_none() {
                    self.drafts
                        .iter()
                        .filter_map(|draft| {
                            Some(KernelToolApiCall {
                                tool: draft.ready.clone()?,
                                call: draft.protocol.as_ref()?.call.clone(),
                                error: draft.protocol.as_ref()?.error.clone(),
                                signature: draft.signature.clone()?,
                            })
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                self.outcome = Some(KernelRoundOutcome {
                    round_id: self.round.id(),
                    failure: self.failure.clone(),
                    cancelled: self.cancelled,
                    follow_up_requested: self.follow_up_requested,
                    finish_reason: self.finish_reason,
                    usage: self.usage.clone(),
                    tools,
                });
            }
        }
        self.steps.pop_front();
        Ok(())
    }

    fn start_front(&self) -> Result<BoxFuture<'static, Result<Applied>>> {
        let step = self.steps.front().expect("pending step");
        let request = step.request;
        let future: BoxFuture<'static, Result<Applied>> = match &step.action {
            Action::OpenText { slot, reasoning } => {
                let round = self.round.clone();
                let slot = *slot;
                let reasoning = *reasoning;
                Box::pin(async move {
                    Ok(Applied::OpenText(
                        slot,
                        round.begin_text(request, reasoning).await?.value,
                    ))
                })
            }
            Action::Append { slot, text } => {
                let output = self.texts[*slot]
                    .output
                    .clone()
                    .ok_or_else(missing_handle)?;
                let text = text.clone();
                Box::pin(async move {
                    output.append(&text).await?;
                    Ok(Applied::Append)
                })
            }
            Action::Seal { slot, diagnostic } => {
                let output = self.texts[*slot]
                    .output
                    .clone()
                    .ok_or_else(missing_handle)?;
                let slot = *slot;
                let diagnostic = diagnostic.clone();
                Box::pin(async move {
                    let receipt = match diagnostic {
                        Some(diagnostic) => output.interrupt(request, diagnostic).await?,
                        None => output.finish(request).await?,
                    };
                    Ok(Applied::Seal(slot, receipt.value))
                })
            }
            Action::OpenDraft {
                slot,
                call_id,
                name,
            } => {
                let round = self.round.clone();
                let slot = *slot;
                let call_id = call_id.clone();
                let name = name.clone();
                Box::pin(async move {
                    Ok(Applied::OpenDraft(
                        slot,
                        round.begin_tool_draft(request, call_id, name).await?.value,
                    ))
                })
            }
            Action::Finalize { slot, input } => {
                let draft = self.drafts[*slot]
                    .handle
                    .clone()
                    .ok_or_else(missing_handle)?;
                let slot = *slot;
                let input = input.clone();
                Box::pin(async move {
                    Ok(Applied::Finalize(
                        slot,
                        draft.finalize(request, input).await?.value,
                    ))
                })
            }
            Action::OpenHosted { slot, call } => {
                let round = self.round.clone();
                let slot = *slot;
                let call = call.clone();
                Box::pin(async move {
                    Ok(Applied::OpenHosted(
                        slot,
                        round.begin_provider_tool(request, call).await?.value,
                    ))
                })
            }
            Action::CompleteHosted {
                slot,
                result,
                blocks,
                ..
            } => {
                let tool = self.hosted[*slot]
                    .handle
                    .clone()
                    .ok_or_else(missing_handle)?;
                let slot = *slot;
                let result = result.clone();
                let blocks = Arc::clone(blocks);
                let workspace_root = self.workspace_root.clone();
                let session_id = self.round.session_id().get();
                Box::pin(async move {
                    let result = persist_hosted_media(
                        &workspace_root,
                        session_id,
                        tool.id().get(),
                        result,
                        &blocks,
                    )
                    .await?;
                    tool.complete(request, result).await?;
                    Ok(Applied::CompleteHosted(slot))
                })
            }
            Action::Finish { state, succeeded } => {
                let round = self.round.clone();
                let state = state.clone();
                let succeeded = *succeeded;
                Box::pin(async move {
                    round.finish(request, Some(state), succeeded).await?;
                    Ok(Applied::Finish)
                })
            }
        };
        Ok(future)
    }
}

fn protocol(message: impl Into<String>) -> AppError {
    AppError::Provider(message.into())
}
fn missing_handle() -> KernelError {
    KernelError::Invalid("pending provider step has no committed handle".into())
}

fn check_envelope(value: &impl serde::Serialize) -> std::result::Result<(), AppError> {
    let bytes = serde_json::to_vec(value).map_err(|e| protocol(e.to_string()))?;
    if bytes.len() > MAX_ARGUMENT_BYTES {
        return Err(protocol(
            "provider control data exceeds its persisted envelope budget",
        ));
    }
    Ok(())
}

fn is_inline_hosted_image(block: &ProviderNativeToolOutputBlock) -> bool {
    matches!(
        block,
        ProviderNativeToolOutputBlock::Media { mime_type, artifact }
            if mime_type.starts_with("image/") && artifact.uri.starts_with("data:")
    )
}

/// Inline images are not provider control data. Budget their encoded bytes
/// separately while retaining the normal 8 MiB limit for all other event data.
fn check_provider_event_budget(event: &CompletionStreamEvent) -> std::result::Result<(), AppError> {
    let mut inline_count = 0usize;
    let mut encoded_bytes = 0usize;
    let mut decoded_bytes = 0usize;
    if let CompletionStreamEvent::ProviderNativeToolCallCompleted { blocks, .. } = event {
        for block in blocks {
            if is_inline_hosted_image(block) {
                let ProviderNativeToolOutputBlock::Media { artifact, .. } = block else {
                    unreachable!("checked inline image block");
                };
                inline_count += 1;
                let Some((_, encoded)) = artifact.uri.split_once(',') else {
                    return Err(protocol("provider image data URL is malformed"));
                };
                let padding = encoded
                    .as_bytes()
                    .iter()
                    .rev()
                    .take_while(|b| **b == b'=')
                    .count()
                    .min(2);
                let decoded_len = (encoded.len() / 4)
                    .saturating_mul(3)
                    .saturating_sub(padding);
                decoded_bytes = decoded_bytes.saturating_add(decoded_len);
                if decoded_bytes > MAX_HOSTED_IMAGE_DECODED_BYTES {
                    return Err(protocol(
                        "provider hosted image exceeds the 20 MiB media budget",
                    ));
                }
                encoded_bytes = encoded_bytes.saturating_add(artifact.uri.len());
            }
        }
    }
    if inline_count > MAX_HOSTED_IMAGE_COUNT || encoded_bytes > MAX_HOSTED_IMAGE_ENCODED_BYTES {
        return Err(protocol("provider hosted images exceed their media budget"));
    }
    // The estimator charges twice the length of a string (decoded capacity).
    let estimated = modeled_read_bytes(event).map_err(|e| protocol(e.to_string()))?;
    if estimated.saturating_sub(encoded_bytes.saturating_mul(2)) > MAX_STATE_BYTES {
        return Err(protocol(
            "provider event exceeds the round allocation budget",
        ));
    }
    Ok(())
}

/// Preserve the order and metadata of provider-native blocks, but never place
/// image base64 in a persistable JSON envelope, even transiently.
fn compact_hosted_blocks(
    blocks: &[ProviderNativeToolOutputBlock],
) -> Vec<ProviderNativeToolOutputBlock> {
    blocks
        .iter()
        .map(|block| match block {
            ProviderNativeToolOutputBlock::Media {
                mime_type,
                artifact,
            } if is_inline_hosted_image(block) => ProviderNativeToolOutputBlock::Media {
                mime_type: mime_type.clone(),
                artifact: agena_provider::ProviderNativeToolArtifact {
                    uri: String::new(),
                    mime: artifact.mime.clone(),
                    name: artifact.name.clone(),
                    size_bytes: artifact.size_bytes,
                    sha256: artifact.sha256.clone(),
                },
            },
            _ => block.clone(),
        })
        .collect()
}

fn hosted_media_error(error: agena_runtime_tools::ManagedGeneratedImageError) -> KernelError {
    use agena_runtime_tools::ManagedGeneratedImageError;
    // Invalid provider bytes and permanent local I/O failures cannot become
    // infinite kernel-persistence retries. Only retry genuinely transient I/O.
    match &error {
        ManagedGeneratedImageError::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
            ) =>
        {
            KernelError::Storage(format!("persist provider image: {error}"))
        }
        _ => KernelError::Invalid(format!("persist provider image: {error}")),
    }
}

/// Save provider-hosted image bytes using the same managed-artifact store as
/// direct image tools. The tool's persisted value contains only durable file
/// paths and metadata; re-running a pending completion writes the same bytes
/// to the same call-scoped path before retrying its kernel commit.
async fn persist_hosted_media(
    workspace_root: &Path,
    session_id: i64,
    tool_id: i64,
    mut result: ToolResult,
    blocks: &[ProviderNativeToolOutputBlock],
) -> Result<ToolResult> {
    let mut compact = Vec::with_capacity(blocks.len());
    let mut attachments = Vec::new();
    let artifact_id = format!("provider-hosted-image-{tool_id}");
    for (index, block) in blocks.iter().enumerate() {
        if let ProviderNativeToolOutputBlock::Media {
            mime_type,
            artifact,
        } = block
            && is_inline_hosted_image(block)
        {
            let saved = agena_runtime_tools::persist_generated_image_artifact(
                workspace_root,
                session_id,
                &artifact_id,
                index,
                mime_type,
                artifact.name.as_deref(),
                &artifact.uri,
            )
            .await
            .map_err(hosted_media_error)?
            .ok_or_else(|| {
                KernelError::Invalid("provider returned an unsupported image data URL".into())
            })?;
            compact.push(ProviderNativeToolOutputBlock::Media {
                mime_type: mime_type.clone(),
                artifact: agena_provider::ProviderNativeToolArtifact {
                    uri: format!("file://{}", saved.path),
                    mime: mime_type.clone(),
                    name: Some(saved.filename.clone()),
                    size_bytes: Some(saved.size_bytes),
                    sha256: Some(saved.sha256.clone()),
                },
            });
            attachments.push(AttachmentItem {
                kind: AttachmentKind::Image,
                mime: mime_type.clone(),
                source: AttachmentSource::LocalPath { path: saved.path },
                filename: Some(saved.filename),
                title: None,
                size_bytes: Some(saved.size_bytes),
                sha256: Some(saved.sha256),
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            });
        } else {
            compact.push(block.clone());
        }
    }
    result.value["blocks"] =
        serde_json::to_value(compact).map_err(|e| KernelError::Invalid(e.to_string()))?;
    if !attachments.is_empty() {
        let raw = RawOutput::from_parts(
            None,
            result.value["output_text"].as_str().unwrap_or_default(),
            attachments,
            Vec::new(),
            false,
        );
        result.value["raw_output"] =
            serde_json::to_value(raw).map_err(|e| KernelError::Invalid(e.to_string()))?;
    }
    check_envelope(&result).map_err(|e| KernelError::Invalid(e.to_string()))?;
    Ok(result)
}

#[cfg(test)]
mod hosted_image_persistence_tests {
    use super::*;
    use agena_domain::{StructuredObject, ToolOutput};
    use agena_provider::ProviderNativeToolArtifact;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use sha2::Digest as _;

    fn blocks(payload: &str) -> Vec<ProviderNativeToolOutputBlock> {
        vec![
            ProviderNativeToolOutputBlock::Text {
                text: "other hosted output".into(),
            },
            ProviderNativeToolOutputBlock::Media {
                mime_type: "image/png".into(),
                artifact: ProviderNativeToolArtifact {
                    uri: format!("data:image/png;base64,{payload}"),
                    mime: "image/png".into(),
                    name: Some("sunflower.png".into()),
                    size_bytes: None,
                    sha256: None,
                },
            },
        ]
    }

    fn completed_event(blocks: Vec<ProviderNativeToolOutputBlock>) -> CompletionStreamEvent {
        CompletionStreamEvent::ProviderNativeToolCallCompleted {
            provider_id: agena_domain::ProviderId::new("test"),
            model: agena_domain::ModelId::new("test-model"),
            stream_key: "idx:0".into(),
            id: Some("ig_test".into()),
            invocation: ToolInvocation::new("image_generation", StructuredObject::default()),
            title: "Image generation".into(),
            summary: "1 image".into(),
            failure: None,
            output_text: "sunflower".into(),
            blocks,
            details: ToolOutput::default(),
            raw: Some(json!({"type":"image_generation_call","status":"completed"})),
        }
    }

    #[test]
    fn large_inline_image_is_media_not_provider_control_data() {
        let encoded = STANDARD.encode(vec![42u8; 1024 * 1024]);
        let blocks = blocks(&encoded);
        let event = completed_event(blocks.clone());
        assert!(check_provider_event_budget(&event).is_ok());
        assert!(check_envelope(&json!({"blocks":blocks})).is_err());
        let preview = compact_hosted_blocks(match &event {
            CompletionStreamEvent::ProviderNativeToolCallCompleted { blocks, .. } => blocks,
            _ => unreachable!(),
        });
        assert!(check_envelope(&json!({"blocks":preview})).is_ok());
        assert!(
            !serde_json::to_string(&preview)
                .unwrap()
                .contains(&encoded[..256])
        );
        // Large non-media control fields still obey the existing limit.
        let mut invalid = event.clone();
        if let CompletionStreamEvent::ProviderNativeToolCallCompleted { output_text, .. } =
            &mut invalid
        {
            *output_text = "text".repeat(3 * 1024 * 1024);
        }
        assert!(check_provider_event_budget(&invalid).is_err());
    }

    #[tokio::test]
    async fn hosted_image_is_durable_and_has_no_inline_base64_in_result() {
        let workspace =
            std::env::temp_dir().join(format!("agena-hosted-image-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&workspace).unwrap();
        let mut bytes = STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg==").unwrap();
        bytes.resize(650 * 1024, 0);
        let encoded = STANDARD.encode(&bytes);
        let blocks = blocks(&encoded);
        let result = ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"title":"image generation","output_text":"sunflower"}),
        };
        let completed = persist_hosted_media(&workspace, 43, 491, result.clone(), &blocks)
            .await
            .unwrap();
        assert_eq!(completed.phase, ToolPhase::Succeeded);
        let persisted_json = serde_json::to_string(&completed).unwrap();
        assert!(persisted_json.len() < MAX_ARGUMENT_BYTES);
        assert!(!persisted_json.contains(&encoded[..256]));
        let raw: RawOutput = serde_json::from_value(completed.value["raw_output"].clone()).unwrap();
        assert_eq!(raw.attachments.len(), 1);
        let attachment = &raw.attachments[0];
        let AttachmentSource::LocalPath { path } = &attachment.source else {
            panic!("hosted image must be a managed local attachment");
        };
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(
            attachment.sha256,
            Some(hex::encode(sha2::Sha256::digest(&bytes)))
        );
        assert_eq!(
            completed.value["blocks"][1]["artifact"]["uri"],
            format!("file://{path}")
        );
        assert_eq!(completed.value["blocks"][0]["text"], "other hosted output");
        // Retrying the same persistence step reuses a stable path and result.
        let repeated = persist_hosted_media(&workspace, 43, 491, result, &blocks)
            .await
            .unwrap();
        assert_eq!(completed, repeated);
        std::fs::remove_dir_all(agena_runtime_tools::project_state_dir(&workspace)).unwrap();
        std::fs::remove_dir_all(workspace).unwrap();
    }

    #[tokio::test]
    async fn invalid_hosted_media_is_not_persisted_as_a_successful_tool() {
        let block = blocks("this-is-not-base64");
        let result = ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({}),
        };
        let err = persist_hosted_media(Path::new("/tmp"), 43, 491, result, &block)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid generated image payload"));
    }

    #[tokio::test]
    async fn hosted_non_image_blocks_remain_unchanged() {
        let blocks = vec![ProviderNativeToolOutputBlock::Text {
            text: "searched the web".into(),
        }];
        let result = ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"title":"web search","summary":"success"}),
        };
        let completed = persist_hosted_media(Path::new("/tmp"), 43, 491, result, &blocks)
            .await
            .unwrap();
        assert_eq!(
            completed.value["blocks"],
            serde_json::to_value(&blocks).unwrap()
        );
        assert!(completed.value.get("raw_output").is_none());
    }
}

#[cfg(test)]
mod hosted_media_failure_tests {
    use super::*;
    #[test]
    fn permanent_image_errors_are_not_storage_retry_requests() {
        use agena_runtime_tools::ManagedGeneratedImageError as E;
        for error in [
            E::InvalidImage("corrupt".into()),
            E::TooLarge { limit_mib: 20 },
            E::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "denied",
            )),
        ] {
            assert!(matches!(hosted_media_error(error), KernelError::Invalid(_)));
        }
        assert!(matches!(
            hosted_media_error(E::Io(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "retry"
            ))),
            KernelError::Storage(_)
        ));
    }
}
