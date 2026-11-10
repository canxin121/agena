//! Concise tool tests that still execute the complete user/model/tool round trip.
//!
//! This module only scripts AI decisions. It never invokes a plugin directly or
//! changes the session's permissions. Use `start_tool` for interactive tools, or
//! the lower-level model API when a scenario needs multiple model decisions.

use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context as TaskContext, Poll},
    time::Duration,
};

use agena_api::{live::ToolDetailSection, part::PartResource, resource::RunOptions};
use agena_client::AgenaClient;
use agena_domain::{AttachmentItem, ContentFormat, RawOutput};
use agena_failure::{Failure, ModelFeedbackKind};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{
    ModelHandle, PendingInteraction, Reply, RequestMatch, RunHandle, RunResult, UserSession,
};

const MAX_RAW_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// An in-flight scripted tool call. Its run exposes the normal permission,
/// user-input, cancellation and exact-attempt APIs.
pub struct ToolRun {
    run: RunHandle,
    client: AgenaClient,
    model: ModelHandle,
    timeout: Duration,
    tool: String,
    call_id: String,
    completion: String,
    captured: Arc<Mutex<Option<Value>>>,
}

pub(crate) async fn start(
    user: &UserSession,
    model: ModelHandle,
    timeout: Duration,
    tool: String,
    input: Value,
    options: RunOptions,
) -> Result<ToolRun> {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let marker = format!("agena-testkit-tool-request-{id}");
    let call_id = format!("call_testkit_{id}");
    let completion = format!("agena-testkit-tool-complete-{id}");
    let captured = Arc::new(Mutex::new(None));
    // Reply::tool_call teaches the scripted model to read the tool's help in
    // the same run, because the runtime refuses a call whose contract is not in
    // the current context.
    let first = model
        .expect(RequestMatch::user_text(&marker).declares_function("tools_call"))
        .named(format!("ask AI to call {tool}"))
        .reply(Reply::tool_call(&call_id, &tool, input));
    let capture = captured.clone();
    let output_id = call_id.clone();
    model
        .expect(RequestMatch::tool_output(&call_id))
        .named(format!("AI receives {tool} output"))
        .after(&first)
        .check(move |request| {
            // Validate identity before using the compatible text projection,
            // which also handles text blocks beside multimodal attachments.
            request.require_tool_output(&output_id)?;
            let text = request
                .tool_output_text(&output_id)
                .context("matched model request did not contain its tool output")?;
            let output: Value =
                serde_json::from_str(&text).context("real tool output is not a JSON envelope")?;
            ensure!(
                output.get("phase").and_then(Value::as_str).is_some(),
                "real tool output omitted its phase: {output}"
            );
            let mut slot = capture.lock().unwrap_or_else(|error| error.into_inner());
            ensure!(slot.is_none(), "tool output was captured more than once");
            *slot = Some(output);
            Ok(())
        })
        .reply(Reply::text(&completion));
    let run = user.send_with(marker, options).await?;
    Ok(ToolRun {
        run,
        client: user.client().clone(),
        model,
        timeout,
        tool,
        call_id,
        completion,
        captured,
    })
}

impl ToolRun {
    pub fn run(&self) -> &RunHandle {
        &self.run
    }

    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// Wait for this exact attempt's permission request. If the tool instead
    /// finishes with a structured failure, keep the original wait error and add
    /// the failure that the real model observed, without dumping invocation data.
    pub async fn wait_permission(&self) -> Result<PendingInteraction> {
        self.run
            .wait_permission()
            .await
            .map_err(|error| self.interaction_error("permission", error))
    }

    pub async fn wait_user_input(&self) -> Result<PendingInteraction> {
        self.run
            .wait_user_input()
            .await
            .map_err(|error| self.interaction_error("user input", error))
    }

    fn interaction_error(&self, expected: &str, error: anyhow::Error) -> anyhow::Error {
        let captured = self
            .captured
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let failure = captured
            .as_ref()
            .filter(|output| output.get("phase").and_then(Value::as_str) == Some("failed"))
            .and_then(|output| output.pointer("/result/failure"))
            .and_then(|value| serde_json::from_value::<Failure>(value.clone()).ok());
        match failure {
            Some(failure) => error.context(format!(
                "waiting for {expected} from tool {:?} (call {}); observed tool failure {:?}, code {}: {}",
                self.tool, self.call_id, failure.model.as_ref().map(|model| model.kind),
                failure.code.as_str(), failure.user.fallback.chars().take(240).collect::<String>(),
            )),
            None => error,
        }
    }

    /// Require both the model round trip and this exact execution's settlement.
    /// Tool failures remain observable in `ToolResult`; a failed surrounding run
    /// or missing reply is an error rather than an invented tool result.
    pub async fn wait_settled(&self) -> Result<ToolResult> {
        let run = self.run.wait_settled().await?;
        run.assert_succeeded()?;
        run.assert_assistant_text(&self.completion)?;
        let output = self
            .captured
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
            .context("the model did not receive the tool result")?;
        let mut matches = run.parts.iter().filter(|part| {
            part.kind == "tool_call"
                && part
                    .content
                    .get("tool_api_call")
                    .and_then(|call| {
                        serde_json::from_value::<agena_domain::ToolApiCall>(call.clone()).ok()
                    })
                    .is_some_and(|call| {
                        // A tools_help call names the same tool in its arguments;
                        // only the execution call is this run's durable part.
                        call.function == agena_domain::ToolApiFunction::Call
                            && Value::from(call.arguments)
                                .get("tool")
                                .and_then(Value::as_str)
                                == Some(self.tool.as_str())
                    })
        });
        let part = matches.next().cloned().with_context(|| {
            format!(
                "no durable tool part for {:?} in this exact attempt; parts: {}",
                self.tool,
                serde_json::to_string(&run.parts)
                    .unwrap_or_default()
                    .chars()
                    .take(8000)
                    .collect::<String>()
            )
        })?;
        ensure!(
            matches.next().is_none(),
            "multiple durable parts match {:?}; use explicit model scripts for multi-tool runs",
            self.tool
        );
        let operation = async {
            let detail = self
                .client
                .session_tool_detail(
                    self.run.session_id(),
                    part.part_id,
                    ToolDetailSection::Output,
                )
                .await?;
            ensure!(
                detail.part_id == part.part_id && detail.section == ToolDetailSection::Output,
                "tool output detail belongs to a different part or section"
            );
            let mut raw: Option<RawOutput> =
                serde_json::from_value(detail.value).context("invalid public raw tool output")?;
            if let Some(raw) = raw.as_mut() {
                hydrate_fields(&self.client, self.run.session_id(), raw).await?;
            }
            Ok::<_, anyhow::Error>(raw)
        };
        let raw_output = tokio::select! {
            error = self.model.failed() => return Err(error),
            result = tokio::time::timeout(self.timeout, operation) => {
                result.context("timed out reading durable tool output")??
            }
        };
        Ok(ToolResult {
            tool: self.tool.clone(),
            call_id: self.call_id.clone(),
            output,
            raw_output,
            part,
            run,
        })
    }
}

/// Both what the AI actually received and the product's durable, structured
/// result. These are separate views and deliberately remain separate fields.
pub struct ToolResult {
    pub tool: String,
    /// The provider protocol's call ID, not the integer durable part ID.
    pub call_id: String,
    /// The actual `function_call_output` JSON envelope sent back to the AI.
    pub output: Value,
    /// The raw output obtained through the public tool detail/content APIs.
    /// Referenced payload fields are hydrated with a 4 MiB aggregate limit.
    pub raw_output: Option<RawOutput>,
    pub part: PartResource,
    pub run: RunResult,
}

impl ToolResult {
    pub fn assert_succeeded(&self) -> Result<()> {
        ensure!(
            self.output.get("phase").and_then(Value::as_str) == Some("succeeded"),
            "tool {:?} failed: {}",
            self.tool,
            self.output
        );
        Ok(())
    }

    pub fn assert_failed(&self) -> Result<()> {
        ensure!(
            self.output.get("phase").and_then(Value::as_str) == Some("failed"),
            "expected tool {:?} to fail, got: {}",
            self.tool,
            self.output
        );
        Ok(())
    }

    /// The structured failure actually sent to the model. A missing or malformed
    /// failure is an error; a generic tool failure is never treated as a specific
    /// permission, validation, or transport failure.
    pub fn failure(&self) -> Result<Failure> {
        self.assert_failed()?;
        let value = self.output.pointer("/result/failure").with_context(|| {
            format!("tool {:?} omitted its structured model failure", self.tool)
        })?;
        serde_json::from_value(value.clone()).with_context(|| {
            format!(
                "tool {:?} returned an invalid structured failure",
                self.tool
            )
        })
    }

    pub fn assert_failure_kind(&self, expected: ModelFeedbackKind) -> Result<()> {
        let failure = self.failure()?;
        let actual = failure
            .model
            .as_ref()
            .with_context(|| format!("tool {:?} omitted model failure feedback", self.tool))?;
        ensure!(
            actual.kind == expected,
            "tool {:?} failed with {:?}, expected {expected:?}: {}",
            self.tool,
            actual.kind,
            self.output
        );
        Ok(())
    }

    /// The actual model-facing text, or the complete envelope when the product
    /// did not render a text projection (for example, a protocol-level error).
    pub fn text(&self) -> String {
        self.output
            .pointer("/result/model")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| self.output.to_string())
    }

    pub fn assert_text_contains(&self, expected: &str) -> Result<()> {
        let text = self.text();
        ensure!(
            text.contains(expected),
            "tool {:?} model text does not contain {expected:?}: {text}",
            self.tool
        );
        Ok(())
    }

    /// Parse the model-facing text as JSON. Use `data` for the durable payload,
    /// which need not have the same shape as the model's rendered text.
    pub fn json(&self) -> Result<Value> {
        serde_json::from_str(&self.text())
            .with_context(|| format!("tool {:?} model text is not JSON", self.tool))
    }

    pub fn data(&self) -> Result<&Value> {
        self.raw_output
            .as_ref()
            .and_then(|raw| raw.payload.as_ref())
            .with_context(|| format!("tool {:?} has no durable output payload", self.tool))
    }

    /// Attachments from the durable output, not from the model's text rendering.
    /// Use `UserSession::download_attachment` to read server-owned bytes.
    pub fn attachments(&self) -> Result<&[AttachmentItem]> {
        self.raw_output
            .as_ref()
            .map(|raw| raw.attachments.as_slice())
            .with_context(|| format!("tool {:?} has no durable raw output", self.tool))
    }

    pub fn attachment(&self, index: usize) -> Result<&AttachmentItem> {
        let attachments = self.attachments()?;
        attachments.get(index).with_context(|| {
            format!(
                "tool {:?} has {} durable attachments; index {index} is out of range",
                self.tool,
                attachments.len(),
            )
        })
    }
}

async fn hydrate_fields(client: &AgenaClient, session: i64, raw: &mut RawOutput) -> Result<()> {
    let mut remaining = MAX_RAW_OUTPUT_BYTES;
    if let Some(payload) = &raw.payload {
        let length = serde_json::to_vec(payload)?.len();
        ensure!(
            length <= remaining,
            "raw tool payload exceeds the 4 MiB test helper limit"
        );
        remaining -= length;
    }
    // A referenced field is a product-owned source. The client verifies source
    // identity, epoch and complete transfer; the sink bounds aggregate memory.
    for field in &raw.fields {
        let mut sink = LimitedBytes {
            bytes: Vec::new(),
            limit: remaining,
        };
        client
            .write_content_text(session, &field.resource, &mut sink)
            .await?;
        remaining -= sink.bytes.len();
        let text = String::from_utf8(sink.bytes).context("tool content is not UTF-8")?;
        let value = if field.format == ContentFormat::Json {
            serde_json::from_str(&text).context("tool JSON content is invalid")?
        } else {
            Value::String(text)
        };
        if field.pointer.is_empty() {
            raw.payload = Some(value);
        } else {
            let target = raw
                .payload
                .as_mut()
                .and_then(|value| value.pointer_mut(&field.pointer))
                .ok_or_else(|| {
                    anyhow!(
                        "referenced tool output field {:?} has no payload position",
                        field.pointer
                    )
                })?;
            *target = value;
        }
    }
    Ok(())
}

struct LimitedBytes {
    bytes: Vec<u8>,
    limit: usize,
}

impl AsyncWrite for LimitedBytes {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut TaskContext<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Poll::Ready(Err(std::io::Error::other(
                "raw tool content exceeds the 4 MiB test helper limit",
            )));
        }
        self.bytes.extend_from_slice(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
