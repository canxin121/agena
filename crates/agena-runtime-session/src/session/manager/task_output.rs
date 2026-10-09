//! A delegated run owns one parent Part log, just like a shell process.
//! Semantic child changes discover sources; independent content cursors carry
//! bytes without reloading a session or updating its Part for every token.

use std::{collections::HashMap, sync::Arc};

use agena_domain::{
    CommandOutputStream, ContentCursor, ContentInput, ContentKind, ContentPayload, ContentRef,
    ContentState, SubtaskStatus,
};
use agena_storage::{
    content::{ContentHub, ContentWriter},
    store::{Part, PartDelta, PartRole, PartVisibility, SessionChange, SessionStore, StoreError},
};
use tokio::{
    sync::{mpsc, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use super::{
    AppError, SessionManager,
    replies::{operation_content_value, operation_from_part},
};

const MAX_CAPTURE_PARTS: usize = 512;
const MAX_ACTIVE_RELAYS: usize = 32;

pub(super) struct SubtaskOutput {
    stop: CancellationToken,
    terminal: watch::Sender<(SubtaskStatus, Option<String>)>,
    worker: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for SubtaskOutput {
    fn drop(&mut self) {
        // Cancellation of the caller must also seal the log and remove every
        // subscription, including when child preparation or persistence fails.
        self.stop.cancel();
    }
}

impl SubtaskOutput {
    pub(super) async fn finish(
        mut self,
        status: SubtaskStatus,
        failure: Option<&agena_failure::Failure>,
    ) {
        self.terminal
            .send_replace((status, failure.map(|failure| failure.user.fallback.clone())));
        self.stop.cancel();
        if let Some(worker) = self.worker.take()
            && let Err(error) = worker.await
        {
            tracing::warn!(%error, "delegated task output capture failed");
        }
    }

    fn start(
        store: Arc<dyn SessionStore>,
        writer: ContentWriter,
        child_id: i64,
        baseline: i64,
    ) -> Self {
        let stop = CancellationToken::new();
        let capture_stop = stop.clone();
        let (terminal, terminal_rx) = watch::channel((SubtaskStatus::Interrupted, None));
        let (tx, rx) = mpsc::channel(256);
        let overflow = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observer_overflow = overflow.clone();
        // Subscribe before execution creates its first Part/resource. No
        // history bootstrap is needed: this log covers only the new run.
        let subscription = store.subscribe(
            child_id,
            Arc::new(move |change| {
                let (SessionChange::PartAdded { part, .. }
                | SessionChange::PartUpdated { part, .. }) = change
                else {
                    return;
                };
                if part.part_id <= baseline
                    || part.role != PartRole::Assistant
                    || part.visibility == PartVisibility::Ai
                {
                    return;
                }
                if matches!(tx.try_send(part), Err(mpsc::error::TrySendError::Full(_))) {
                    observer_overflow.store(true, std::sync::atomic::Ordering::Release);
                }
            }),
        );
        let worker = tokio::spawn(async move {
            let result = capture(
                store.contents().clone(),
                writer.clone(),
                rx,
                capture_stop,
                terminal_rx,
                overflow,
            )
            .await;
            drop(subscription);
            if let Err(error) = result {
                writer.record_loss(0, &error);
                let _ = writer.finalize(ContentState::Interrupted).await;
                tracing::warn!(%error, child_id, "delegated task output capture interrupted");
            }
        });
        Self {
            stop,
            terminal,
            worker: Some(worker),
        }
    }
}

impl SessionManager {
    pub(super) async fn start_subtask_output(
        &self,
        parent_id: i64,
        call_id: Option<i64>,
        child_id: i64,
        baseline: i64,
        task_id: &str,
        description: &str,
    ) -> Result<Option<SubtaskOutput>, AppError> {
        let Some(call_id) = call_id else {
            return Ok(None);
        };
        let writer = self
            .session_mutations
            .run(parent_id, async {
                let parent = self.store.load_session(parent_id).await?;
                let Some((part, mut operation)) = parent.parts().iter().rev().find_map(|part| {
                    let operation = operation_from_part(part)?;
                    (part.role == PartRole::Assistant && operation.call_id == call_id)
                        .then_some((part, operation))
                }) else {
                    return Ok(None);
                };
                let writer = self
                    .store
                    .facade
                    .contents()
                    .open(parent_id, part.part_id, ContentKind::Log)
                    .await
                    .map_err(crate::session::store::store_error)?;
                operation.resources.push(writer.resource().reference());
                operation
                    .metadata
                    .insert("child_session_id".into(), serde_json::json!(child_id));
                operation
                    .metadata
                    .insert("task_id".into(), serde_json::json!(task_id));
                self.store
                    .update_part(
                        parent_id,
                        part.part_id,
                        PartDelta {
                            content: Some(operation_content_value(&operation)?),
                            ..Default::default()
                        },
                    )
                    .await?;
                append(
                    &writer,
                    CommandOutputStream::Stdout,
                    format!("Task: {description}\nSession: {child_id} · {task_id}\nRunning\n"),
                )
                .await
                .map_err(crate::session::store::store_error)?;
                Ok(Some(writer))
            })
            .await?;
        Ok(writer.map(|writer| {
            SubtaskOutput::start(self.store.facade.clone(), writer, child_id, baseline)
        }))
    }
}

async fn append(
    writer: &ContentWriter,
    stream: CommandOutputStream,
    text: String,
) -> Result<(), StoreError> {
    if !text.is_empty() {
        writer.append(ContentInput::Log { stream, text }).await?;
    }
    Ok(())
}

async fn capture(
    hub: ContentHub,
    writer: ContentWriter,
    mut parts: mpsc::Receiver<Part>,
    stop: CancellationToken,
    terminal: watch::Receiver<(SubtaskStatus, Option<String>)>,
    overflow: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), StoreError> {
    let relay_stop = CancellationToken::new();
    let mut relays = JoinSet::new();
    let mut seen = HashMap::<i64, (String, bool)>::new();
    let mut sources = std::collections::HashSet::new();
    let mut limited = false;
    loop {
        let part = tokio::select! {
            biased;
            _ = stop.cancelled() => { parts.close(); parts.recv().await },
            part = parts.recv() => part,
            result = relays.join_next(), if !relays.is_empty() => {
                if let Some(result) = result { report_relay(result, &writer).await?; }
                continue;
            },
        };
        let Some(part) = part else {
            break;
        };
        if overflow.swap(false, std::sync::atomic::Ordering::AcqRel) {
            append(
                &writer,
                CommandOutputStream::Stderr,
                "\n[Some task updates were omitted; open the child session for details.]\n".into(),
            )
            .await?;
        }
        if !seen.contains_key(&part.part_id) && seen.len() >= MAX_CAPTURE_PARTS {
            if !limited {
                limited = true;
                append(
                    &writer,
                    CommandOutputStream::Stderr,
                    "\n[Task preview limit reached; open the child session for further output.]\n"
                        .into(),
                )
                .await?;
            }
            continue;
        }
        let operation = operation_from_part(&part);
        let references = part
            .content
            .get("resources")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| serde_json::from_value::<ContentRef>(value.clone()).ok())
            .filter(|reference| {
                matches!(
                    reference.kind,
                    ContentKind::Text | ContentKind::Log | ContentKind::Terminal
                )
            })
            .collect::<Vec<_>>();
        let entry = seen
            .entry(part.part_id)
            .or_insert_with(|| (String::new(), false));
        if let Some(operation) = &operation
            && entry.0 != part.state.as_str()
        {
            append(
                &writer,
                CommandOutputStream::Stdout,
                format!(
                    "\n[{} · {}]\n",
                    agena_tool::tool_title_for_state(&operation.invocation, operation.state),
                    part.state.as_str()
                ),
            )
            .await?;
        } else if entry.0.is_empty() && matches!(part.kind.as_str(), "text" | "think") {
            append(
                &writer,
                CommandOutputStream::Stdout,
                if part.kind == "think" {
                    "\n[Thinking]\n"
                } else {
                    "\n[Assistant]\n"
                }
                .into(),
            )
            .await?;
        }
        entry.0 = part.state.as_str().into();
        if !references.is_empty() {
            entry.1 = true;
            for reference in references {
                if !sources.insert(reference.resource_id) {
                    continue;
                }
                // Bound simultaneously active subscriptions, including tools
                // that keep a background source open after returning.
                while let Some(result) = relays.try_join_next() {
                    report_relay(result, &writer).await?;
                }
                if relays.len() >= MAX_ACTIVE_RELAYS {
                    append(&writer, CommandOutputStream::Stderr, "\n[Task has too many live sources; open the child session for this output.]\n".into()).await?;
                    continue;
                }
                let hub = hub.clone();
                let writer = writer.clone();
                let stop = relay_stop.clone();
                let child_id = part.origin_session_id;
                let child_part_id = part.part_id;
                relays.spawn(async move {
                    relay(hub, writer, reference, child_id, child_part_id, stop).await
                });
            }
        } else if !entry.1 && part.state.is_terminal() {
            let text = operation
                .as_ref()
                .and_then(|operation| {
                    operation
                        .output_text()
                        .or_else(|| operation.error_message())
                })
                .map(str::to_owned)
                .or_else(|| {
                    part.content
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            if !text.is_empty() {
                append(&writer, CommandOutputStream::Stdout, format!("{text}\n")).await?;
                entry.1 = true;
            }
        }
    }
    // Producers may have finished while their final records are still unread.
    // Stop means drain the current cursor before leaving, never abort a relay.
    relay_stop.cancel();
    while let Some(result) = relays.join_next().await {
        report_relay(result, &writer).await?;
    }
    let (status, error) = terminal.borrow().clone();
    let stream = if status == SubtaskStatus::Completed {
        CommandOutputStream::Stdout
    } else {
        CommandOutputStream::Stderr
    };
    append(
        &writer,
        stream,
        format!(
            "\n[Task {}]{}\n",
            status.as_ref(),
            error.map(|error| format!(": {error}")).unwrap_or_default()
        ),
    )
    .await?;
    writer
        .finalize(
            if matches!(
                status,
                SubtaskStatus::Cancelled | SubtaskStatus::TimedOut | SubtaskStatus::Interrupted
            ) {
                ContentState::Interrupted
            } else {
                ContentState::Complete
            },
        )
        .await?;
    Ok(())
}

async fn report_relay(
    result: Result<Result<(), StoreError>, tokio::task::JoinError>,
    writer: &ContentWriter,
) -> Result<(), StoreError> {
    let error = match result {
        Ok(Ok(())) => return Ok(()),
        Ok(Err(error)) => error.to_string(),
        Err(error) => error.to_string(),
    };
    append(
        writer,
        CommandOutputStream::Stderr,
        format!("\n[Task output unavailable: {error}]\n"),
    )
    .await
}

async fn relay(
    hub: ContentHub,
    writer: ContentWriter,
    reference: ContentRef,
    child_id: i64,
    child_part_id: i64,
    stop: CancellationToken,
) -> Result<(), StoreError> {
    let mut changes = hub.subscribe(reference.resource_id);
    let resource = hub.describe(reference.resource_id).await?;
    if resource.owner_session_id != child_id || resource.part_id != child_part_id {
        return Err(StoreError::InvalidState(
            "task output does not belong to its child Part".into(),
        ));
    }
    let mut cursor = ContentCursor {
        sequence: 0,
        ..resource.cursor
    };
    let mut drain_until = None;
    loop {
        if drain_until.is_none() && stop.is_cancelled() {
            // A dropped caller may leave the supervised child running. Freeze
            // the final cursor so a busy producer cannot prolong cleanup.
            drain_until = Some(hub.describe(reference.resource_id).await?.cursor);
        }
        let page = hub
            .read(reference.resource_id, Some(cursor), 64 * 1024)
            .await?;
        if page.gap {
            append(
                &writer,
                CommandOutputStream::Stderr,
                "\n[Earlier child output is no longer retained.]\n".into(),
            )
            .await?;
        }
        for chunk in page.chunks {
            if drain_until.is_some_and(|limit| chunk.cursor.sequence > limit.sequence) {
                break;
            }
            match chunk.payload {
                ContentPayload::Text { text } => {
                    append(&writer, CommandOutputStream::Stdout, text).await?
                }
                ContentPayload::Log { stream, text } => append(&writer, stream, text).await?,
                ContentPayload::Terminal { screen } => {
                    append(&writer, CommandOutputStream::Stdout, screen.formatted()).await?
                }
                ContentPayload::TerminalPatch {
                    screen,
                    rows_changed,
                    ..
                } => {
                    append(
                        &writer,
                        CommandOutputStream::Stdout,
                        screen.formatted_patch(&rows_changed),
                    )
                    .await?
                }
                _ => {}
            }
        }
        let advanced = page.next_cursor != cursor;
        cursor = page.next_cursor;
        if drain_until.is_some_and(|limit| cursor.sequence >= limit.sequence) {
            break;
        }
        if page.has_more && advanced {
            continue;
        }
        if page.resource.state != ContentState::Active {
            break;
        }
        if stop.is_cancelled() {
            continue;
        }
        let Some(receiver) = changes.as_mut() else {
            break;
        };
        let closed = tokio::select! {
            _ = stop.cancelled() => false,
            result = receiver.changed() => result.is_err(),
        };
        if closed {
            changes = None;
        }
    }
    Ok(())
}
