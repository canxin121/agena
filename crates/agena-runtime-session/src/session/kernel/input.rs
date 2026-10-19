use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

use agena_domain::{
    ActivityPayload, ComposerNode, ContentId, ContentKind, ModelRef, ResourceDelivery, Role,
};
use agena_provider::{
    CompletionInputPart, CompletionInputProviderState, CompletionInputRun,
    CompletionInputToolResultStatus, ModelToolFunction, ProviderCompactionContext,
};
use agena_storage::kernel::{
    ContentStreamView, EntryId, EntryPayload, EventCursor, EventPayload, KernelError, KernelReader,
    PromptView, ProviderRoundPhase, ReadBudget, ReadBudgetSnapshot, ReadLease, ReadView, Result,
    RoundId, SessionId, TextCursor, ToolPhase, modeled_read_bytes,
};
use serde::Serialize;
use serde_json::{Value, json};

use super::replay::{KernelProviderState, restore_replay_text};

const MAX_MODEL_BYTES: usize = 32 * 1024 * 1024;
const TOOL_PREVIEW_BYTES: usize = 512 * 1024;
static PROMPT_BUDGET: OnceLock<ReadBudget> = OnceLock::new();

/// One request's hydrated model input. The projection's lifetime retains its
/// admission lease, independently of the immutable kernel facts it borrows.
pub struct KernelPrompt {
    facts: ReadView<PromptView>,
    turns: Vec<CompletionInputRun>,
    compaction: Option<ProviderCompactionContext>,
    model: ModelRef,
    _lease: ReadLease,
    _media_leases: Vec<ReadLease>,
    media_modeled: usize,
}

impl KernelPrompt {
    pub fn model(&self) -> &ModelRef {
        &self.model
    }
    pub(crate) fn install(&mut self, request: &mut agena_provider::CompletionRequest) {
        request.turns = std::mem::take(&mut self.turns);
        request.provider_compaction = self.compaction.take();
        request.previous_response_id = None;
        request.model = self.model.model_id.clone();
    }
    pub fn facts(&self) -> &PromptView {
        &self.facts
    }
    pub fn turns(&self) -> &[CompletionInputRun] {
        &self.turns
    }
    pub fn compaction(&self) -> Option<&ProviderCompactionContext> {
        self.compaction.as_ref()
    }
    /// Count provider-visible text and protocol metadata without copying the
    /// request or charging encoded media as ordinary text tokens.
    pub fn estimate_tokens(
        &self,
        system: Option<&str>,
        tools: &[crate::tool::ToolApiBinding],
    ) -> Result<u64> {
        estimate_request_tokens(&self.turns, self.compaction.as_ref(), system, tools)
    }
}

fn json_bytes(value: &impl Serialize) -> Result<usize> {
    #[derive(Default)]
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter::default();
    serde_json::to_writer(&mut count, value).map_err(|e| invalid(e.to_string()))?;
    Ok(count.0)
}
fn estimate_request_tokens(
    turns: &[CompletionInputRun],
    compaction: Option<&ProviderCompactionContext>,
    system: Option<&str>,
    tools: &[crate::tool::ToolApiBinding],
) -> Result<u64> {
    let mut chars = 1024usize.saturating_add(system.map_or(0, str::len));
    // Only declarations actually sent to the provider belong to the prompt.
    for tool in tools {
        chars = chars.saturating_add(json_bytes(&tool.definition())?);
    }
    if let Some(compaction) = compaction {
        chars = chars.saturating_add(json_bytes(compaction)?);
    }
    for turn in turns {
        chars = chars
            .saturating_add(64)
            .saturating_add(json_bytes(&turn.provider_state)?);
        for part in &turn.parts {
            chars = chars.saturating_add(32);
            let bytes = match part {
                CompletionInputPart::Attachment { .. } => 64,
                CompletionInputPart::ToolResult {
                    tool_call_id,
                    function,
                    arguments_json,
                    output_json,
                    attachments,
                    ..
                } => tool_call_id
                    .len()
                    .saturating_add(function.function_name().len())
                    .saturating_add(arguments_json.len())
                    .saturating_add(output_json.len())
                    .saturating_add(attachments.len().saturating_mul(64)),
                other => json_bytes(other)?,
            };
            chars = chars.saturating_add(bytes);
        }
    }
    Ok(crate::estimate_prompt_tokens_from_chars(chars))
}

#[cfg(test)]
mod token_estimate_tests {
    use super::*;
    #[test]
    fn input_budget_includes_dynamic_system_tools_text_and_exact_replay_without_counting_base64_as_text()
     {
        let mut turn = CompletionInputRun {
            role: Role::User,
            parts: vec![CompletionInputPart::Text {
                text: "instructions".into(),
            }],
            provider_state: Default::default(),
        };
        let baseline =
            estimate_request_tokens(std::slice::from_ref(&turn), None, None, &[]).unwrap();
        let tools = [crate::tool::ToolApiBinding::call_gateway()];
        let system = "dynamic project instructions ".repeat(100);
        let expanded =
            estimate_request_tokens(std::slice::from_ref(&turn), None, Some(&system), &tools)
                .unwrap();
        assert!(expanded > baseline + system.len() as u64 / 4);
        turn.provider_state.openai_reasoning_items = vec![
            json!({"type":"reasoning","encrypted_content":"OPAQUE".repeat(10_000),"unknown":[true,null]}),
        ];
        let replay = estimate_request_tokens(std::slice::from_ref(&turn), None, None, &[]).unwrap();
        assert!(replay > baseline + 10_000);
        let attachment = agena_provider::CompletionInputAttachment {
            kind: agena_provider::CompletionInputAttachmentKind::Image,
            mime: "image/png".into(),
            source: agena_provider::CompletionInputAttachmentSource::Base64 {
                data: "A".repeat(1024 * 1024),
            },
            filename: None,
            title: None,
            size_bytes: None,
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        };
        turn.parts
            .push(CompletionInputPart::Attachment { attachment });
        let with_media =
            estimate_request_tokens(std::slice::from_ref(&turn), None, None, &[]).unwrap();
        assert!(with_media - replay < 100);
        let compaction = ProviderCompactionContext::OpenAiResponses {
            items: vec![json!({"type":"compaction","encrypted_content":"Z".repeat(40_000)})],
        };
        assert!(
            estimate_request_tokens(&[turn], Some(&compaction), None, &[]).unwrap()
                > with_media + 9_999
        );
    }
}

/// All instances share process-wide admission. Each projection reserves
/// 32 MiB of conservative modeled allocations before copying or hydration;
/// the ordinary read budget also bounds aggregate and per-session retention.
/// SQLite, codec transients and adapter-owned request copies are separate.
#[derive(Clone)]
pub struct KernelPromptHydrator {
    budget: ReadBudget,
}

impl Default for KernelPromptHydrator {
    fn default() -> Self {
        Self {
            budget: PROMPT_BUDGET.get_or_init(ReadBudget::default).clone(),
        }
    }
}

impl KernelPromptHydrator {
    /// Runtime projection includes current plugin rendering, reconstructed
    /// from immutable raw tool results. Plugins do not receive control handles.
    pub async fn hydrate_with_executor(
        &self,
        reader: &KernelReader,
        facts: ReadView<PromptView>,
        model: &ModelRef,
        native_compaction: bool,
        executor: &crate::tool::ToolExecutor,
        conversation: Option<&agena_domain::SessionConversation>,
    ) -> Result<KernelPrompt> {
        let mut prompt = self
            .hydrate_conversation(reader, facts, model, native_compaction, conversation)
            .await?;
        let definitions = executor
            .available_tool_api_bindings_async()
            .await
            .into_iter()
            .map(|b| b.definition())
            .collect::<Vec<_>>();
        let tools = prompt
            .facts
            .entries
            .iter()
            .filter_map(|entry| {
                entry
                    .tool
                    .as_ref()
                    .map(|tool| (tool.call.call_id.clone(), (tool, entry.entry.id)))
            })
            .collect::<BTreeMap<_, _>>();

        for part in prompt
            .turns
            .iter_mut()
            .flat_map(|turn| turn.parts.iter_mut())
        {
            let CompletionInputPart::ToolResult {
                tool_call_id,
                output_json,
                attachments,
                ..
            } = part
            else {
                continue;
            };
            let Some((tool, entry_id)) = tools.get(tool_call_id) else {
                continue;
            };
            let Some(value) = tool.result.as_ref().and_then(|r| r.get("raw_output")) else {
                continue;
            };
            let mut raw: agena_domain::RawOutput = serde_json::from_value(value.clone())
                .map_err(|e| invalid(format!("invalid raw tool output: {e}")))?;
            let invocation = crate::session::processor::parse_tool_invocation_lossy(
                prompt.facts.metadata.id.get(),
                &tool.call.name,
                &serde_json::to_string(&tool.call.input).map_err(|e| invalid(e.to_string()))?,
                &definitions,
            )
            .map_err(|e| invalid(e.to_string()))?;
            for item in &raw.attachments {
                if let Some(hydrated) = super::media::hydrate_attachment(
                    reader,
                    prompt.facts.metadata.id,
                    *entry_id,
                    item,
                    model,
                    &self.budget,
                )
                .await?
                {
                    prompt.media_modeled = prompt.media_modeled.saturating_add(hydrated.modeled);
                    prompt._media_leases.extend(hydrated.lease);
                    attachments.push(hydrated.attachment);
                } else if matches!(
                    item.source,
                    agena_domain::AttachmentSource::Url { .. }
                        | agena_domain::AttachmentSource::FileId { .. }
                        // A degraded attachment still has to reach the model as
                        // an explicit notice, otherwise the historical tool
                        // result silently loses what it once delivered.
                        | agena_domain::AttachmentSource::Unavailable { .. }
                ) {
                    attachments.push(super::media::completion_attachment(item));
                }
            }
            for field in &raw.fields {
                if field.pointer.is_empty()
                    && field.format == agena_domain::ContentFormat::Json
                    && let Some(head) = prompt
                        .facts
                        .sources
                        .iter()
                        .find(|source| source.id == field.resource.resource_id)
                    && head.committed <= 512 * 1024
                {
                    let mut allowance = 2 * 1024 * 1024;
                    let text =
                        read_text(reader, prompt.facts.metadata.id, head, &mut allowance, None)
                            .await?;
                    raw.payload =
                        Some(serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?);
                }
            }
            // Attachments are delivered through provider media blocks; their
            // base64 bytes never appear in the model-facing text renderer.
            raw.attachments.clear();
            let rendered = executor.render_tool_result(&invocation, &raw).await;
            if let Some(text) = rendered.model {
                let marker = "[Tool result preview limited; read the recorded content references for the complete retained output.]";
                let text =
                    crate::tool::bounded_model_output_preview(&text, marker, 2000, 50 * 1024);
                let mut envelope: Value =
                    serde_json::from_str(output_json).map_err(|e| invalid(e.to_string()))?;
                envelope["result"] = json!({"model":text});
                let next = serde_json::to_string(&envelope).map_err(|e| invalid(e.to_string()))?;
                *output_json = next;
            }
        }
        if modeled_read_bytes(&prompt.turns)?
            .saturating_add(modeled_read_bytes(&prompt.compaction)?)
            > MAX_MODEL_BYTES.saturating_add(prompt.media_modeled)
        {
            return Err(invalid(
                "rendered tool inputs exceed the provider input budget",
            ));
        }
        Ok(prompt)
    }

    pub fn budget(&self) -> ReadBudgetSnapshot {
        self.budget.snapshot()
    }

    #[cfg(test)]
    pub(in crate::session) fn isolated() -> Self {
        Self {
            budget: ReadBudget::default(),
        }
    }

    pub async fn hydrate(
        &self,
        reader: &KernelReader,
        facts: ReadView<PromptView>,
        model: &ModelRef,
        native_compaction: bool,
    ) -> Result<KernelPrompt> {
        self.hydrate_conversation(reader, facts, model, native_compaction, None)
            .await
    }

    async fn hydrate_conversation(
        &self,
        reader: &KernelReader,
        facts: ReadView<PromptView>,
        model: &ModelRef,
        native_compaction: bool,
        conversation: Option<&agena_domain::SessionConversation>,
    ) -> Result<KernelPrompt> {
        let _lease = self
            .budget
            .reserve_view(facts.metadata.id, MAX_MODEL_BYTES)?;
        let mut remaining = MAX_MODEL_BYTES;
        let mut media_leases = Vec::new();
        let mut media_modeled = 0usize;
        charge(&mut remaining, modeled_read_bytes(&*facts)?)?;
        let sources = facts
            .sources
            .iter()
            .map(|head| (head.id, head))
            .collect::<BTreeMap<_, _>>();
        let replays = facts
            .replays
            .iter()
            .filter(|r| r.phase == ProviderRoundPhase::Succeeded)
            .map(|r| (r.round_id, r.provider_state.as_ref()))
            .collect::<BTreeMap<_, _>>();
        let mut by_entry = BTreeMap::<EntryId, Vec<&ContentStreamView>>::new();
        for head in &facts.sources {
            by_entry.entry(head.entry).or_default().push(head);
        }
        let mut reasoning =
            BTreeMap::<RoundId, BTreeMap<ContentId, (ContentStreamView, String)>>::new();
        let mut turns = Vec::<CompletionInputRun>::new();
        let mut run_rounds = Vec::<Option<RoundId>>::new();
        let mut active_round = None;
        let mut compaction = None;
        let mut boundary = conversation.map(agena_domain::SessionConversation::boundary_notice);
        for item in &facts.entries {
            if item.entry.origin_session_id == facts.metadata.id
                && matches!(item.entry.payload, EntryPayload::User { .. })
                && let Some(text) = boundary.take()
            {
                charge(&mut remaining, text.len().saturating_mul(2) + 256)?;
                turns.push(CompletionInputRun {
                    role: Role::System,
                    parts: vec![CompletionInputPart::SystemMessage { text }],
                    provider_state: CompletionInputProviderState::default(),
                });
                run_rounds.push(None);
                active_round = None;
            }
            let mut parts = Vec::new();
            let role = match &item.entry.payload {
                EntryPayload::User { document } => {
                    for node in &document.0 {
                        match node {
                            ComposerNode::Text { text } => {
                                parts.push(CompletionInputPart::Text { text: text.clone() })
                            }
                            ComposerNode::Activity { activity } => match &activity.payload {
                                ActivityPayload::TextArtifact(artifact) => {
                                    parts.push(CompletionInputPart::Text {
                                        text: artifact.text.clone(),
                                    })
                                }
                                ActivityPayload::CommandReference(command) => {
                                    let text = command.model_context_text();
                                    charge(&mut remaining, text.len().saturating_mul(2) + 256)?;
                                    parts.push(CompletionInputPart::Text { text });
                                }
                                ActivityPayload::Resource(resource) => {
                                    if resource.delivery == ResourceDelivery::ModelInput {
                                        // History keeps its attachments across
                                        // provider and model switches: bytes
                                        // that can no longer be delivered
                                        // degrade to an explicit notice.
                                        let hydrated = super::media::hydrate(
                                            reader,
                                            facts.metadata.id,
                                            item.entry.id,
                                            resource,
                                            model,
                                            &self.budget,
                                        )
                                        .await?;
                                        media_modeled =
                                            media_modeled.saturating_add(hydrated.modeled);
                                        media_leases.extend(hydrated.lease);
                                        parts.push(CompletionInputPart::Attachment {
                                            attachment: hydrated.attachment,
                                        });
                                        continue;
                                    }
                                    let encoded = encoded_json(resource, &mut remaining)?;
                                    parts.push(CompletionInputPart::Text {text:format!("<agena_resource_reference>\n{encoded}\nThis is a resource reference. Media contents were not included.\n</agena_resource_reference>")});
                                }
                                _ => {
                                    return Err(KernelError::Invalid(
                                        "user input contains a runtime-only activity".into(),
                                    ));
                                }
                            },
                        }
                    }
                    Role::User
                }
                EntryPayload::Text { content_id } | EntryPayload::Reasoning { content_id } => {
                    let head = sources
                        .get(content_id)
                        .ok_or_else(|| invalid("input text has no frozen source head"))?;
                    let text =
                        read_text(reader, facts.metadata.id, head, &mut remaining, None).await?;
                    if matches!(item.entry.payload, EntryPayload::Reasoning { .. }) {
                        let round = item
                            .round_id
                            .ok_or_else(|| invalid("reasoning input has no source round"))?;
                        charge(&mut remaining, text.len().saturating_mul(2) + 256)?;
                        reasoning
                            .entry(round)
                            .or_default()
                            .insert(*content_id, ((*head).clone(), text.clone()));
                        parts.push(CompletionInputPart::Reasoning { text });
                    } else {
                        parts.push(CompletionInputPart::Text { text });
                    }
                    Role::Assistant
                }
                EntryPayload::Tool { .. } => {
                    let tool = item
                        .tool
                        .as_ref()
                        .ok_or_else(|| invalid("input tool has no recorded outcome"))?;
                    if tool.phase == ToolPhase::Preparing {
                        continue;
                    }
                    if tool.execution_kind
                        == agena_storage::kernel::ToolExecutionKind::ProviderHosted
                    {
                        if item.round_id != active_round || turns.is_empty() {
                            turns.push(CompletionInputRun {
                                role: Role::Assistant,
                                parts: Vec::new(),
                                provider_state: CompletionInputProviderState::default(),
                            });
                            run_rounds.push(item.round_id);
                        }
                        active_round = item.round_id;
                        continue;
                    }
                    let function = ModelToolFunction::new(tool.call.name.clone());
                    let arguments_json = encoded_json(&tool.call.input, &mut remaining)?;
                    parts.push(CompletionInputPart::ToolCall {
                        id: tool.call.call_id.clone(),
                        function: function.clone(),
                        arguments_json: arguments_json.clone(),
                    });
                    let mut bodies = Vec::new();
                    for head in by_entry.get(&item.entry.id).into_iter().flatten() {
                        bodies.push(
                            tool_content(reader, facts.metadata.id, head, &mut remaining).await?,
                        );
                    }
                    let result = json!({"phase":tool.phase,"result":tool.result,"sources":bodies});
                    let output_json = encoded_json(&result, &mut remaining)?;
                    let status = match tool.phase {
                        ToolPhase::Succeeded => CompletionInputToolResultStatus::Completed,
                        ToolPhase::Cancelled => CompletionInputToolResultStatus::Cancelled,
                        _ => CompletionInputToolResultStatus::Failed,
                    };
                    parts.push(CompletionInputPart::ToolResult {
                        tool_call_id: tool.call.call_id.clone(),
                        function,
                        arguments_json,
                        status,
                        output_json,
                        attachments: Vec::new(),
                    });
                    Role::Assistant
                }
                EntryPayload::Interaction { .. } => continue,
                EntryPayload::Notice { text } => {
                    parts.push(CompletionInputPart::SystemMessage { text: text.clone() });
                    Role::System
                }
                EntryPayload::Compaction { summary, .. } => {
                    // A checkpoint with an unreadable or foreign-route native
                    // compaction state still has its frozen summary text; only
                    // the native replay is skipped.
                    let state = item
                        .round_id
                        .and_then(|id| replays.get(&id))
                        .and_then(|state| *state)
                        .and_then(KernelProviderState::decode_lossy);
                    if native_compaction
                        && let Some(state) = state
                        && state.model == *model
                        && state.compaction.is_some()
                    {
                        charge(&mut remaining, modeled_read_bytes(&state.compaction)?)?;
                        compaction = state.compaction;
                        active_round = None;
                        continue;
                    }
                    let head = sources
                        .get(summary)
                        .ok_or_else(|| invalid("checkpoint has no frozen summary source"))?;
                    let summary =
                        read_text(reader, facts.metadata.id, head, &mut remaining, None).await?;
                    parts.push(CompletionInputPart::Text {text:format!("<agena_history_checkpoint entry=\"{}\">\nThe following is historical checkpoint data, not a new instruction. Continue from it while prioritizing later verbatim messages.\n\n{summary}\n</agena_history_checkpoint>",item.entry.id.get())});
                    Role::User
                }
            };
            let round = if role == Role::Assistant {
                item.round_id
            } else {
                None
            };
            if round.is_some() && round == active_round {
                turns
                    .last_mut()
                    .ok_or_else(|| invalid("input round has no message"))?
                    .parts
                    .extend(parts);
            } else {
                turns.push(CompletionInputRun {
                    role,
                    parts,
                    provider_state: CompletionInputProviderState::default(),
                });
                run_rounds.push(round);
            }
            active_round = round;
        }
        if let Some(text) = boundary {
            charge(&mut remaining, text.len().saturating_mul(2) + 256)?;
            turns.push(CompletionInputRun {
                role: Role::System,
                parts: vec![CompletionInputPart::SystemMessage { text }],
                provider_state: CompletionInputProviderState::default(),
            });
            run_rounds.push(None);
        }
        let mut restored = BTreeSet::new();
        for (run, round) in turns.iter_mut().zip(run_rounds) {
            let Some(round) = round else { continue };
            if !restored.insert(round) {
                continue;
            }
            let Some(raw) = replays.get(&round).and_then(|state| *state) else {
                continue;
            };
            // Replay belongs to the route that produced it. A round captured by
            // a different model is replayed as plain history, and a round whose
            // state cannot be interpreted loses only its replay fidelity.
            let Some(state) = KernelProviderState::decode_lossy(raw) else {
                continue;
            };
            if state.model != *model {
                continue;
            }
            let empty = BTreeMap::new();
            let value = match restore_replay_text(
                &state.replay,
                reasoning.get(&round).unwrap_or(&empty),
                &mut remaining,
            ) {
                Ok(value) => value,
                // A content epoch reset invalidates the whole replay tree, not
                // one round; it is a structural condition of the session.
                Err(KernelError::ContentReset) => return Err(KernelError::ContentReset),
                Err(error) => {
                    tracing::warn!(%error, "skipping unreplayable provider state");
                    continue;
                }
            };
            match serde_json::from_value(value) {
                Ok(state) => run.provider_state = state,
                Err(error) => {
                    tracing::warn!(%error, "skipping malformed provider replay");
                }
            }
        }
        turns.retain(|turn| !turn.parts.is_empty() || !turn.provider_state.is_empty());
        if modeled_read_bytes(&turns)?.saturating_add(modeled_read_bytes(&compaction)?)
            > MAX_MODEL_BYTES.saturating_add(media_modeled)
        {
            return Err(invalid(
                "hydrated provider input exceeds its byte budget; compact a smaller prefix",
            ));
        }
        Ok(KernelPrompt {
            facts,
            turns,
            compaction,
            model: model.clone(),
            _lease,
            _media_leases: media_leases,
            media_modeled,
        })
    }
}

pub(super) fn charge(remaining: &mut usize, bytes: usize) -> Result<()> {
    *remaining = remaining.checked_sub(bytes).ok_or_else(|| {
        invalid("hydrated provider input exceeds its byte budget; compact a smaller prefix")
    })?;
    Ok(())
}

fn invalid(message: impl Into<String>) -> KernelError {
    KernelError::Invalid(message.into())
}

fn encoded_json(value: &impl Serialize, remaining: &mut usize) -> Result<String> {
    struct Count {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes = self.bytes.saturating_add(buf.len());
            if self.bytes > self.limit {
                return Err(std::io::Error::other("provider JSON exceeds byte budget"));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count {
        bytes: 0,
        limit: *remaining / 2,
    };
    serde_json::to_writer(&mut count, value).map_err(|e| invalid(e.to_string()))?;
    charge(remaining, count.bytes.saturating_mul(2) + 256)?;
    serde_json::to_string(value).map_err(|e| invalid(e.to_string()))
}

pub(crate) async fn read_text(
    reader: &KernelReader,
    session: SessionId,
    head: &ContentStreamView,
    remaining: &mut usize,
    preview: Option<usize>,
) -> Result<String> {
    let start = if preview.is_some() {
        head.retained_start
    } else {
        0
    };
    let length = usize::try_from(head.committed.saturating_sub(start))
        .map_err(|e| invalid(e.to_string()))?;
    let count = length.min(preview.unwrap_or(length));
    charge(remaining, count.saturating_mul(2) + 512)?;
    let mut bytes = Vec::with_capacity(count);
    let mut cursor = TextCursor {
        content: head.id,
        epoch: head.epoch,
        offset: start,
    };
    let mut first = true;
    while first || bytes.len() < count {
        first = false;
        let page = reader
            .text_range_at(
                session,
                head,
                Some(cursor),
                (count - bytes.len()).clamp(1, 64 * 1024),
            )
            .await?;
        if count == 0 {
            break;
        }
        if page.next.offset <= cursor.offset {
            return Err(invalid("frozen text read made no progress"));
        }
        bytes.extend_from_slice(&page.bytes);
        cursor = page.next;
    }
    if count < length {
        while std::str::from_utf8(&bytes).is_err_and(|e| e.error_len().is_none()) {
            bytes.pop();
        }
    }
    let mut text = String::from_utf8(bytes)
        .map_err(|e| invalid(format!("source contains invalid UTF-8: {e}")))?;
    if start > 0 {
        text.insert_str(0, "[Earlier tool output is outside the retained range.]\n");
    }
    if count < length {
        text.push_str("\n[Tool output preview limited; read the content source for the complete retained range.]");
    }
    Ok(text)
}

async fn tool_content(
    reader: &KernelReader,
    session: SessionId,
    head: &ContentStreamView,
    remaining: &mut usize,
) -> Result<Value> {
    let body = match head.kind {
        ContentKind::Text => {
            json!({"text":read_text(reader,session,head,remaining,Some(TOOL_PREVIEW_BYTES)).await?})
        }
        ContentKind::Log => {
            let mut cursor = EventCursor {
                content: head.id,
                epoch: head.epoch,
                sequence: head.retained_start,
            };
            let mut records = Vec::new();
            let mut preview = TOOL_PREVIEW_BYTES;
            let mut limited = false;
            let mut partial = None;
            let mut first = true;
            while first || cursor.sequence < head.committed {
                first = false;
                let page = reader
                    .event_page_at(session, head, Some(cursor), 200)
                    .await?;
                if cursor.sequence == head.committed {
                    break;
                }
                if page.next.sequence <= cursor.sequence {
                    return Err(invalid("frozen log read made no progress"));
                }
                for record in &page.records {
                    let EventPayload::Log { channel, text } = &record.payload else {
                        return Err(invalid("log input contains a non-log record"));
                    };
                    let mut end = text.len().min(preview);
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    charge(remaining, end.saturating_mul(2) + 512)?;
                    records.push(json!({"sequence":record.sequence,"timestamp_ms":record.timestamp_ms,"channel":channel,"text":&text[..end]}));
                    preview -= end;
                    if end == text.len() {
                        cursor.sequence = record.sequence;
                    } else {
                        partial = Some(json!({"sequence":record.sequence,"offset_bytes":end}));
                    }
                    if end < text.len() || preview == 0 || records.len() == 2000 {
                        limited = end < text.len() || record.sequence < head.committed;
                        break;
                    }
                }
                if limited || preview == 0 {
                    break;
                }
            }
            json!({"records":records,"retained_start":head.retained_start,"next_cursor":cursor,"partial_event":partial,"preview_limited":limited})
        }
        ContentKind::Terminal => {
            let snapshot = reader.event_snapshot_at(session, head).await?;
            if let Some(screen) = &snapshot.terminal {
                let bytes = screen.cells.iter().map(|run| run.text.len()).sum::<usize>()
                    + usize::from(screen.rows) * (usize::from(screen.cols) + 1);
                charge(remaining, bytes.saturating_mul(2) + 1024)?;
                let mut text = screen.plain_text();
                let limited = text.len() > TOOL_PREVIEW_BYTES;
                if limited {
                    let mut end = TOOL_PREVIEW_BYTES;
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    text.truncate(end);
                }
                json!({"terminal":{"rows":screen.rows,"cols":screen.cols,"cursor_row":screen.cursor_row,"cursor_col":screen.cursor_col,"text":text,"preview_limited":limited}})
            } else {
                json!({"terminal":null})
            }
        }
        ContentKind::Document => {
            let mut cursor = EventCursor {
                content: head.id,
                epoch: head.epoch,
                sequence: head.retained_start,
            };
            let mut events = Vec::new();
            let mut preview = TOOL_PREVIEW_BYTES;
            let mut limited = false;
            let mut partial = None;
            while cursor.sequence < head.committed && preview > 0 && events.len() < 2000 {
                let page = reader
                    .event_page_at(session, head, Some(cursor), 128)
                    .await?;
                if page.next.sequence <= cursor.sequence {
                    return Err(invalid("mixed output read made no progress"));
                }
                for record in &page.records {
                    let EventPayload::Mixed { input, .. } = &record.payload else {
                        return Err(invalid("mixed output contains an unexpected event"));
                    };
                    let bytes = modeled_read_bytes(&**input)?;
                    if bytes > preview {
                        let available = preview.saturating_sub(1024) / 2;
                        match &**input {
                            agena_domain::ContentInput::Text { text }
                            | agena_domain::ContentInput::Log { text, .. } => {
                                let mut end = text.len().min(available);
                                while !text.is_char_boundary(end) {
                                    end -= 1;
                                }
                                let value = match &**input {
                                    agena_domain::ContentInput::Log { stream, .. } => {
                                        json!({"type":"log","stream":stream,"text":&text[..end]})
                                    }
                                    _ => json!({"type":"text","text":&text[..end]}),
                                };
                                charge(remaining, end.saturating_mul(2) + 1024)?;
                                events.push(json!({"sequence":record.sequence,"timestamp_ms":record.timestamp_ms,"input":value,"preview_limited":true}));
                                partial = Some(
                                    json!({"sequence":record.sequence,"offset_bytes":end,"resume":"read this event at next_cursor and skip offset_bytes of its text"}),
                                );
                            }
                            _ => {
                                events.push(json!({"sequence":record.sequence,"timestamp_ms":record.timestamp_ms,"preview_limited":true,"note":"This event exceeds the preview budget. Read the retained event using next_cursor; the latest structured checkpoint is included below."}));
                            }
                        }
                        limited = true;
                        break;
                    }
                    charge(remaining, bytes)?;
                    preview -= bytes;
                    events.push(json!({"sequence":record.sequence,"timestamp_ms":record.timestamp_ms,"input":input}));
                    cursor.sequence = record.sequence;
                    if events.len() >= 2000 {
                        limited = true;
                        break;
                    }
                }
                if limited {
                    break;
                }
            }
            let snapshot = reader.event_snapshot_at(session, head).await?;
            charge(remaining, modeled_read_bytes(&snapshot.document)?)?;
            json!({"events":events,"document":snapshot.document,"retained_start":head.retained_start,"next_cursor":cursor,"partial_event":partial,"preview_limited":limited||cursor.sequence<head.committed})
        }
        ContentKind::Structured => {
            let snapshot = reader.event_snapshot_at(session, head).await?;
            charge(remaining, modeled_read_bytes(&*snapshot)?)?;
            json!({"document":snapshot.document,"terminal":snapshot.terminal})
        }
    };
    Ok(
        json!({"content_id":head.id,"epoch":head.epoch,"kind":head.kind,"committed_head":head.committed,"capture_state":head.state,"capture_error":head.capture_error,"capture_loss":head.capture_loss,"body":body}),
    )
}
