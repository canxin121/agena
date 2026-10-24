use crate::{App, TranscriptState, TuiBackend};
use agena_domain::{ContentId, ContentState};
use agena_tui_transcript::content::{ContentFrame, ContentView};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

const MAX_ACTIVE_CONTENT_READS: usize = 16;

#[derive(Clone, Default)]
struct ContentUpdate {
    frame: Option<Arc<ContentFrame>>,
    error: Option<String>,
    complete: bool,
}

pub(crate) struct ContentRead {
    receiver: watch::Receiver<ContentUpdate>,
    interested: watch::Sender<bool>,
    task: tokio::task::AbortHandle,
    pub(crate) displayed: Option<Arc<ContentFrame>>,
    pub(crate) displayed_error: Option<String>,
}

impl ContentRead {
    fn start(
        application: TuiBackend,
        session_id: i64,
        id: ContentId,
        reconnecting: String,
    ) -> Self {
        let (tx, receiver) = watch::channel(ContentUpdate::default());
        let (interested, mut interest) = watch::channel(true);
        let task = tokio::spawn(async move {
            let mut view = ContentView::default();
            let mut failures = 0u32;
            'observe: loop {
                // Keep the accepted cursor, parser and displayed frame while
                // offscreen, but drop the network subscription immediately.
                if interest.wait_for(|active| *active).await.is_err() {
                    return;
                }
                let result = tokio::select! {
                    biased;
                    _ = interest.changed() => continue,
                    result = application.stream_content(session_id, id, view.cursor) => result,
                };
                match result {
                    Ok(mut subscription) => {
                        failures = 0;
                        loop {
                            let result = tokio::select! {
                                biased;
                                _ = interest.changed() => continue 'observe,
                                result = subscription.recv() => result,
                            };
                            match result {
                                Some(Ok(page)) => {
                                    let complete = page.resource.state != ContentState::Active
                                        && !page.has_more;
                                    if !view.apply(page) {
                                        break;
                                    }
                                    tx.send_replace(ContentUpdate {
                                        frame: view.frame.clone(),
                                        error: None,
                                        complete,
                                    });
                                    if complete {
                                        return;
                                    }
                                }
                                Some(Err(error)) => {
                                    tracing::debug!(%id, diagnostic = %error.operator_diagnostic(), "content subscription reconnecting");
                                    tx.send_replace(ContentUpdate {
                                        frame: view.frame.clone(),
                                        error: Some(error.user_message()),
                                        complete: false,
                                    });
                                    break;
                                }
                                None => break,
                            }
                        }
                    }
                    Err(error) => {
                        tracing::debug!(%id, %error, "content subscription unavailable");
                        tx.send_replace(ContentUpdate {
                            frame: view.frame.clone(),
                            error: Some(error.to_string()),
                            complete: false,
                        });
                    }
                }
                if tx.borrow().error.is_none() {
                    tx.send_replace(ContentUpdate {
                        frame: view.frame.clone(),
                        error: Some(reconnecting.clone()),
                        complete: false,
                    });
                }
                failures = failures.saturating_add(1);
                tokio::select! {
                    _ = tx.closed() => return,
                    _ = interest.changed() => {},
                    _ = tokio::time::sleep(Duration::from_millis(250 * 2u64.pow(failures.min(5)))) => {},
                }
            }
        });
        Self {
            receiver,
            interested,
            task: task.abort_handle(),
            displayed: None,
            displayed_error: None,
        }
    }

    fn is_complete(&self) -> bool {
        self.receiver.borrow().complete
    }

    fn has_headline_preview(&self) -> bool {
        let latest = self.receiver.borrow();
        if latest.complete {
            return true;
        }
        let Some(frame) = latest.frame.as_ref() else {
            return false;
        };
        let text = frame.text.as_ref();
        // A finished first line gives a stable summary while the remainder
        // keeps streaming. Bound one-line streams as well; expanding resumes
        // the same reader/cursor instead of replaying the accepted prefix.
        text.rfind('\n').is_some_and(|end| {
            agena_tui_transcript::content_headline_preview(&text[..=end]).is_some()
        }) || (text.chars().take(256).count() == 256
            && agena_tui_transcript::content_headline_preview(text).is_some())
    }

    fn set_interested(&self, interested: bool) {
        self.interested.send_if_modified(|current| {
            if *current == interested {
                return false;
            }
            *current = interested;
            true
        });
    }

    fn sync_display(&mut self, paused: bool) -> bool {
        let latest = self.receiver.borrow_and_update();
        let mut changed = false;
        // Selection and scrolling freeze displayed cells. An unloaded source
        // may still publish its first frame so history can be read in place.
        if (!paused || self.displayed.is_none())
            && let Some(frame) = &latest.frame
            && self
                .displayed
                .as_ref()
                .is_none_or(|old| !Arc::ptr_eq(frame, old))
        {
            self.displayed = Some(frame.clone());
            changed = true;
        }
        if self.displayed_error != latest.error {
            self.displayed_error = latest.error.clone();
            changed = true;
        }
        changed
    }
}

impl Drop for ContentRead {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TranscriptState {
    fn content_read_interest(
        &mut self,
        width: u16,
        height: u16,
        visible: bool,
    ) -> BTreeSet<ContentId> {
        let resources = |part: &agena_api::part::PartResource| {
            part.content
                .get("resources")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|value| value.get("resource_id").cloned())
                .filter_map(|value| serde_json::from_value::<ContentId>(value).ok())
                .collect::<BTreeSet<_>>()
        };
        let retained = self
            .parts
            .iter()
            .flat_map(resources)
            .collect::<BTreeSet<_>>();
        // Viewport changes never unload observed content. Membership removal
        // (or a session reset) owns eviction of its last displayed frame.
        self.content_reads.retain(|id, _| retained.contains(id));
        let interested = if visible {
            let top = self.viewport_top();
            let end = top.saturating_add(usize::from(height));
            // Activities own only their headline row. A visible Markdown
            // block or tool section keeps its source subscribed even after
            // that headline has scrolled out of view.
            let visible_parts = self
                .rendered(width)
                .nodes
                .iter()
                .filter(|node| node.expanded && node.start_line < end && node.end_line > top)
                .filter_map(|node| match node.key {
                    agena_tui_transcript::TranscriptNodeKey::Activity {
                        content_id: agena_tui_transcript::TranscriptContentId::StoredPart(id),
                        ..
                    }
                    | agena_tui_transcript::TranscriptNodeKey::ActivitySection {
                        content_id: agena_tui_transcript::TranscriptContentId::StoredPart(id),
                        ..
                    }
                    | agena_tui_transcript::TranscriptNodeKey::MarkdownBlock {
                        content_id: agena_tui_transcript::TranscriptContentId::StoredPart(id),
                        ..
                    }
                    | agena_tui_transcript::TranscriptNodeKey::Content {
                        content_id: Some(agena_tui_transcript::TranscriptContentId::StoredPart(id)),
                        ..
                    } => Some(id),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            let visible_headlines = self
                .rendered(width)
                .nodes
                .iter()
                .filter(|node| node.start_line < end && node.end_line > top)
                .filter_map(|node| match node.key {
                    agena_tui_transcript::TranscriptNodeKey::Activity {
                        content_id: agena_tui_transcript::TranscriptContentId::StoredPart(id),
                        ..
                    } => Some(id),
                    _ => None,
                })
                .collect::<BTreeSet<_>>();
            let mut interested = self.parts
                .iter()
                .filter(|part| visible_parts.contains(&part.part_id))
                .flat_map(resources)
                .collect::<BTreeSet<_>>()
                .into_iter()
                // Completed observations keep their frame without consuming
                // a subscription slot or starving later visible resources.
                .filter(|id| !self.content_reads.get(id).is_some_and(ContentRead::is_complete))
                .take(MAX_ACTIVE_CONTENT_READS)
                .collect::<BTreeSet<_>>();
            // Collapsed text-backed parts still need a useful headline. Read
            // only until a preview exists, after scheduling visible bodies.
            // Terminal/log output remains governed by detail visibility.
            for part in self.parts.iter().filter(|part| {
                visible_headlines.contains(&part.part_id)
                    && !visible_parts.contains(&part.part_id)
                    && (matches!(part.kind.as_str(), "think" | "compaction")
                        || (part.kind == "text" && part.role == "assistant"))
                    && !part
                        .summary
                        .as_ref()
                        .is_some_and(|text| !text.trim().is_empty())
                    && !part
                        .presentation
                        .as_ref()
                        .is_some_and(|view| !view.summary.trim().is_empty())
            }) {
                for id in part
                    .content
                    .get("resources")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|resource| {
                        resource.get("kind").and_then(serde_json::Value::as_str) == Some("text")
                    })
                    .filter_map(|resource| resource.get("resource_id").cloned())
                    .filter_map(|value| serde_json::from_value::<ContentId>(value).ok())
                {
                    if interested.len() >= MAX_ACTIVE_CONTENT_READS {
                        break;
                    }
                    if !self
                        .content_reads
                        .get(&id)
                        .is_some_and(ContentRead::has_headline_preview)
                    {
                        interested.insert(id);
                    }
                }
            }
            interested
        } else {
            BTreeSet::new()
        };
        for (id, read) in &self.content_reads {
            read.set_interested(interested.contains(id));
        }
        interested
    }
}

impl App {
    /// Visibility schedules subscriptions; loaded membership retains frames.
    /// Each source coalesces arbitrarily fast output into one watch slot.
    pub(crate) fn heal_content_reads(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.transcript.content_reads.clear();
            return;
        };
        let interested = self.transcript.content_read_interest(
            self.layout.transcript_body.width.max(1),
            self.layout.transcript_body.height,
            self.current_route_is_main(),
        );
        for id in interested {
            self.transcript.content_reads.entry(id).or_insert_with(|| {
                ContentRead::start(
                    self.application.clone(),
                    session_id,
                    id,
                    self.i18n.text("content-reconnecting"),
                )
            });
        }
        let paused =
            self.transcript.has_active_text_selection() || !self.transcript.viewport.follow_tail;
        let mut changed = false;
        for read in self.transcript.content_reads.values_mut() {
            changed |= read.sync_display(paused);
        }
        if changed {
            self.transcript.invalidate_content_render();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_api::part::PartResource;
    use agena_domain::{
        ContentChunk, ContentCursor, ContentKind, ContentPage, ContentPayload, ContentRef,
        ContentResource, PartDocument, TerminalAttributes, TerminalCellRun, TerminalSnapshot,
        ViewBlock,
    };
    use agena_tui_transcript::{TranscriptContentId, TranscriptNodeKey};
    use serde_json::json;

    fn terminal_part(part_id: i64) -> (PartResource, ContentRef) {
        let resource = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Terminal,
        };
        let part = PartResource {
            part_id,
            kind: "tool_call".into(),
            role: "assistant".into(),
            state: "completed".into(),
            message_boundary: true,
            run_id: Some(part_id),
            message_id: Some(part_id),
            revision: 1,
            content: json!({
                "name": "shell.open", "input": {}, "call_id": part_id,
                "state": "completed", "lifecycle": {"start_ms": 0}, "resources": [resource]
            }),
            presentation: Some(PartDocument {
                title: "Historical terminal".into(),
                summary: String::new(),
                blocks: vec![ViewBlock::Content {
                    id: "output".into(),
                    resource: resource.clone(),
                    format: Default::default(),
                }],
            }),
            ..Default::default()
        };
        (part, resource)
    }

    fn terminal_frame(resource: &ContentRef) -> Arc<ContentFrame> {
        let cursor = ContentCursor {
            epoch: agena_domain::ContentEpoch::new().into(),
            sequence: 1,
        };
        let mut view = ContentView::default();
        assert!(view.apply(ContentPage {
            resource: ContentResource {
                resource_id: resource.resource_id,
                owner_session_id: 1,
                part_id: 1,
                kind: ContentKind::Terminal,
                state: ContentState::Complete,
                cursor,
                committed_cursor: cursor,
                total_bytes: None,
                dropped_bytes: 0,
                dropped_packets: 0,
                committed_dropped_bytes: 0,
                committed_dropped_packets: 0,
                retained_ranges: Vec::new(),
                capture_error: None,
            },
            chunks: vec![ContentChunk {
                cursor,
                captured_at_ms: 0,
                payload: ContentPayload::Terminal {
                    screen: TerminalSnapshot {
                        rows: 24,
                        cols: 80,
                        cursor_row: 12,
                        cursor_col: 0,
                        cursor_visible: true,
                        alternate_screen: false,
                        bracketed_paste: false,
                        application_cursor: false,
                        cells: (0..12)
                            .map(|row| TerminalCellRun {
                                row,
                                col: 0,
                                text: format!("HISTORY_{}", row + 1),
                                attributes: TerminalAttributes::default(),
                            })
                            .collect(),
                    },
                },
            }],
            next_cursor: cursor,
            has_more: false,
            gap: false,
        }));
        view.frame.unwrap()
    }

    fn observed(
        frame: Arc<ContentFrame>,
        complete: bool,
    ) -> (
        ContentRead,
        watch::Sender<ContentUpdate>,
        watch::Receiver<bool>,
    ) {
        let (sender, receiver) = watch::channel(ContentUpdate {
            frame: Some(frame.clone()),
            error: None,
            complete,
        });
        let (interested, interest) = watch::channel(true);
        let task = tokio::spawn(std::future::pending::<()>());
        (
            ContentRead {
                receiver,
                interested,
                task: task.abort_handle(),
                displayed: Some(frame),
                displayed_error: None,
            },
            sender,
            interest,
        )
    }

    fn transcript(parts: Vec<PartResource>) -> TranscriptState {
        let mut state = TranscriptState::default();
        state.session_id = Some(1);
        state.detail_expanded_by_default.activity_default_expanded = true;
        state.merge_parts(parts);
        state
    }

    #[tokio::test]
    async fn collapsed_reasoning_reads_a_stable_preview_then_resumes_its_body_on_expand() {
        let resource = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Text,
        };
        let part = PartResource {
            part_id: 1,
            kind: "think".into(),
            role: "assistant".into(),
            state: "in_progress".into(),
            message_boundary: true,
            run_id: Some(1),
            content: json!({"resources": [resource]}),
            ..Default::default()
        };
        let mut state = transcript(vec![part]);
        state.detail_expanded_by_default.activity_default_expanded = false;
        let initial = state.content_read_interest(100, 100, true);
        assert_eq!(initial, BTreeSet::from([resource.resource_id]));
        let key = state
            .rendered(100)
            .nodes
            .iter()
            .find(|node| matches!(node.key, TranscriptNodeKey::Activity { .. }))
            .unwrap()
            .key
            .clone();
        let mut partial = (*terminal_frame(&resource)).clone();
        partial.resource.kind = ContentKind::Text;
        partial.resource.state = ContentState::Active;
        partial.terminal_size = None;
        partial.lines.clear();
        partial.wrapped.clear();
        partial.text = Arc::from("\n\n**Inspect stream");
        let (read, sender, interest) = observed(Arc::new(partial.clone()), false);
        state.content_reads.insert(resource.resource_id, read);
        state.invalidate_content_render();
        assert_eq!(state.content_read_interest(100, 100, true), initial);
        partial.text = Arc::from("\n\n**Inspect streaming**\nBODY_ONLY_DETAIL");
        let frame = Arc::new(partial);
        sender.send_replace(ContentUpdate {
            frame: Some(frame.clone()),
            complete: false,
            error: None,
        });
        assert!(
            state
                .content_reads
                .get_mut(&resource.resource_id)
                .unwrap()
                .sync_display(false)
        );
        state.invalidate_content_render();
        assert!(state.content_read_interest(100, 100, true).is_empty());
        assert!(!*interest.borrow());
        let rendered = state.rendered(100);
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.text.contains("Thinking · Inspect streaming"))
        );
        assert!(
            !rendered
                .lines
                .iter()
                .any(|line| line.text.contains("BODY_ONLY_DETAIL"))
        );
        state.node_expansions.insert(key.clone(), true);
        state.invalidate_render();
        assert_eq!(state.content_read_interest(100, 100, true), initial);
        assert!(*interest.borrow());
        assert!(
            state
                .rendered(100)
                .lines
                .iter()
                .any(|line| line.text.contains("BODY_ONLY_DETAIL"))
        );
        state.node_expansions.insert(key, false);
        state.invalidate_render();
        assert!(state.content_read_interest(100, 100, true).is_empty());
        assert!(Arc::ptr_eq(
            state.content_reads[&resource.resource_id]
                .displayed
                .as_ref()
                .unwrap(),
            &frame
        ));
    }

    #[tokio::test]
    async fn compaction_expansion_loads_its_summary_once_and_retains_it_after_collapse() {
        let resource = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Text,
        };
        let source = PartResource {
            part_id: 1,
            kind: "text".into(),
            role: "assistant".into(),
            state: "completed".into(),
            message_boundary: true,
            run_id: Some(1),
            content: json!({"text": "", "resources": [resource]}),
            ..Default::default()
        };
        let checkpoint = PartResource {
            part_id: 2,
            kind: "compaction".into(),
            role: "assistant".into(),
            state: "completed".into(),
            run_id: Some(1),
            content: json!({"resources": [resource], "compaction_through": 1}),
            ..Default::default()
        };
        let mut state = transcript(vec![source, checkpoint]);
        state.detail_expanded_by_default.activity_default_expanded = false;
        let node = state
            .rendered(120)
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.key,
                    TranscriptNodeKey::Activity {
                        content_id: TranscriptContentId::StoredPart(2),
                        ..
                    }
                )
            })
            .unwrap()
            .clone();
        assert!(node.toggleable);
        assert_eq!(
            state.content_read_interest(120, 100, true),
            BTreeSet::from([resource.resource_id])
        );
        state.node_expansions.insert(node.key.clone(), true);
        state.invalidate_render();
        assert_eq!(
            state.content_read_interest(120, 100, true),
            BTreeSet::from([resource.resource_id])
        );
        let mut frame = (*terminal_frame(&resource)).clone();
        frame.resource.kind = ContentKind::Text;
        frame.terminal_size = None;
        frame.lines.clear();
        frame.wrapped.clear();
        frame.text = Arc::from("COMPACT_SUMMARY_BODY");
        let (read, _sender, _interest) = observed(Arc::new(frame), true);
        state.content_reads.insert(resource.resource_id, read);
        state.invalidate_content_render();
        let rendered = state.rendered(120);
        // A compact headline and its expanded body may both mention the
        // summary. There is only one content owner, and no assistant answer.
        assert_eq!(
            rendered
                .nodes
                .iter()
                .filter(|node| matches!(
                    node.key,
                    TranscriptNodeKey::Activity {
                        content_id: TranscriptContentId::StoredPart(2),
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(
            rendered
                .lines
                .iter()
                .any(|line| line.text.contains("COMPACT_SUMMARY_BODY"))
        );
        assert!(
            !rendered
                .lines
                .iter()
                .any(|line| line.text.contains("Answer"))
        );
        state.node_expansions.insert(node.key.clone(), false);
        state.invalidate_render();
        state.content_read_interest(120, 100, true);
        state.node_expansions.insert(node.key, true);
        state.invalidate_render();
        assert!(
            state
                .rendered(120)
                .lines
                .iter()
                .any(|line| line.text.contains("COMPACT_SUMMARY_BODY"))
        );
    }

    #[tokio::test]
    async fn historical_output_survives_offscreen_streaming_repaints_and_collapse() {
        let (history, resource) = terminal_part(1);
        let answer_resource = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Text,
        };
        let answer = PartResource {
            part_id: 2,
            kind: "text".into(),
            role: "assistant".into(),
            state: "completed".into(),
            run_id: Some(1),
            message_id: Some(1),
            revision: 1,
            content: json!({"text": "", "resources": [answer_resource]}),
            ..Default::default()
        };
        let mut live = PartResource {
            part_id: 3,
            kind: "text".into(),
            role: "assistant".into(),
            state: "running".into(),
            message_boundary: true,
            run_id: Some(3),
            message_id: Some(3),
            revision: 1,
            content: json!({"text": "stream starts"}),
            ..Default::default()
        };
        let mut state = transcript(vec![history, answer, live.clone()]);
        let frame = terminal_frame(&resource);
        let (read, _sender, interest) = observed(frame.clone(), true);
        state.content_reads.insert(resource.resource_id, read);
        let mut answer_frame = (*terminal_frame(&answer_resource)).clone();
        answer_frame.resource.kind = ContentKind::Text;
        answer_frame.terminal_size = None;
        answer_frame.lines.clear();
        answer_frame.wrapped.clear();
        answer_frame.text = Arc::from("HISTORICAL_ANSWER");
        let (answer_read, _answer_sender, _) = observed(Arc::new(answer_frame), true);
        state
            .content_reads
            .insert(answer_resource.resource_id, answer_read);
        let rendered = state.rendered(100);
        let history_end = rendered
            .lines
            .iter()
            .position(|line| line.text.contains("HISTORICAL_ANSWER"))
            .unwrap()
            + 1;
        let historical_lines = rendered.lines[..history_end].to_vec();
        let key = rendered
            .nodes
            .iter()
            .find(|node| {
                matches!(
                    node.key,
                    TranscriptNodeKey::Activity {
                        content_id: TranscriptContentId::StoredPart(1),
                        ..
                    }
                )
            })
            .unwrap()
            .key
            .clone();
        for revision in 2..30 {
            state.viewport.top = state.rendered(100).lines.len().saturating_sub(2);
            assert!(state.content_read_interest(100, 2, true).is_empty());
            assert!(!*interest.borrow());
            live.revision = revision;
            live.content = json!({"text": format!("{}stream {revision}", "line\n\n".repeat(revision as usize))});
            assert!(state.merge_part(live.clone()));
            let lines = &state.rendered(100).lines;
            assert!(
                historical_lines
                    .iter()
                    .zip(&lines[..history_end])
                    .all(|(old, new)| Arc::ptr_eq(old, new)),
                "streaming rebuilt or unloaded historical output"
            );
            assert!(
                !lines
                    .iter()
                    .any(|line| line.text.contains("Loading output"))
            );
        }
        state.node_expansions.insert(key.clone(), false);
        state.invalidate_render();
        state.content_read_interest(100, 1000, true);
        state.node_expansions.insert(key, true);
        state.invalidate_render();
        assert!(
            state
                .rendered(100)
                .lines
                .iter()
                .any(|line| line.text.contains("HISTORY_12"))
        );
        assert!(Arc::ptr_eq(
            state.content_reads[&resource.resource_id]
                .displayed
                .as_ref()
                .unwrap(),
            &frame
        ));
    }

    #[tokio::test]
    async fn route_and_viewport_suspend_reads_without_unloading_or_replaying() {
        let (part, resource) = terminal_part(1);
        let mut state = transcript(vec![part]);
        let frame = terminal_frame(&resource);
        let (read, sender, interest) = observed(frame.clone(), false);
        state.content_reads.insert(resource.resource_id, read);
        assert_eq!(
            state.content_read_interest(100, 100, true),
            BTreeSet::from([resource.resource_id])
        );
        assert!(*interest.borrow());
        assert!(state.content_read_interest(100, 100, false).is_empty());
        assert!(!*interest.borrow());
        assert!(
            state
                .rendered(100)
                .lines
                .iter()
                .any(|line| line.text.contains("HISTORY_12"))
        );
        let mut updated = (*frame).clone();
        updated.text = Arc::from("newer observation");
        let updated = Arc::new(updated);
        sender.send_replace(ContentUpdate {
            frame: Some(updated.clone()),
            ..Default::default()
        });
        let read = state.content_reads.get_mut(&resource.resource_id).unwrap();
        assert!(!read.sync_display(true));
        assert!(Arc::ptr_eq(read.displayed.as_ref().unwrap(), &frame));
        assert!(read.sync_display(false));
        assert!(Arc::ptr_eq(read.displayed.as_ref().unwrap(), &updated));
        assert_eq!(
            state.content_read_interest(100, 100, true),
            BTreeSet::from([resource.resource_id])
        );
        assert!(*interest.borrow());
        // Membership removal is the event that actually retires the reader.
        state.merge_parts(Vec::new());
        state.content_read_interest(100, 100, true);
        assert!(state.content_reads.is_empty());
        assert!(interest.has_changed().is_err());
    }

    #[tokio::test]
    async fn completed_sources_release_subscription_slots_for_later_visible_output() {
        let fixtures = (1..=20).map(terminal_part).collect::<Vec<_>>();
        let mut state = transcript(fixtures.iter().map(|(part, _)| part.clone()).collect());
        let first = state.content_read_interest(100, 1000, true);
        assert_eq!(first.len(), MAX_ACTIVE_CONTENT_READS);
        for (_, resource) in &fixtures {
            if first.contains(&resource.resource_id) {
                let (read, _, _) = observed(terminal_frame(resource), true);
                state.content_reads.insert(resource.resource_id, read);
            }
        }
        let next = state.content_read_interest(100, 1000, true);
        assert_eq!(next.len(), 4);
        assert!(next.is_disjoint(&first));
        assert_eq!(state.content_reads.len(), MAX_ACTIVE_CONTENT_READS);
    }

    #[tokio::test]
    async fn visible_answer_and_tool_bodies_keep_reading_after_headlines_scroll_away() {
        let (terminal, terminal_resource) = terminal_part(1);
        let terminal_observation = terminal_frame(&terminal_resource);
        let text_resource = ContentRef {
            resource_id: ContentId::new(),
            kind: ContentKind::Text,
        };
        let answer = PartResource {
            part_id: 2,
            kind: "text".into(),
            role: "assistant".into(),
            state: "running".into(),
            message_boundary: true,
            run_id: Some(2),
            message_id: Some(2),
            content: json!({"text": "", "resources": [text_resource]}),
            ..Default::default()
        };
        let mut text_observation = (*terminal_frame(&text_resource)).clone();
        text_observation.resource.kind = ContentKind::Text;
        text_observation.terminal_size = None;
        text_observation.lines.clear();
        text_observation.wrapped.clear();
        text_observation.text = Arc::from("a visible answer paragraph\n\n".repeat(60));
        for (part, resource, frame) in [
            (terminal, terminal_resource, terminal_observation),
            (answer, text_resource, Arc::new(text_observation)),
        ] {
            let part_id = part.part_id;
            let mut state = transcript(vec![part]);
            let (read, _sender, interest) = observed(frame, false);
            state.content_reads.insert(resource.resource_id, read);
            let rendered = state.rendered(100);
            let headline = rendered
                .nodes
                .iter()
                .find(|node| {
                    matches!(node.key, TranscriptNodeKey::Activity {
                    content_id: TranscriptContentId::StoredPart(id), ..
                } if id == part_id)
                })
                .unwrap();
            let key = headline.key.clone();
            let headline_end = headline.end_line;
            let body = rendered.nodes.iter().find(|node| {
                matches!(node.key,
                    TranscriptNodeKey::MarkdownBlock { content_id: TranscriptContentId::StoredPart(id), .. }
                    | TranscriptNodeKey::ActivitySection { content_id: TranscriptContentId::StoredPart(id), .. }
                    if id == part_id
                ) && node.end_line > headline_end && node.expanded
            }).unwrap();
            state.viewport.top = body.end_line - 1;
            assert!(state.viewport.top >= headline_end);
            assert_eq!(
                state.content_read_interest(100, 1, true),
                BTreeSet::from([resource.resource_id])
            );
            assert!(
                *interest.borrow(),
                "visible body was mistaken for an offscreen source"
            );
            state.node_expansions.insert(key, false);
            state.invalidate_render();
            state.viewport.top = 0;
            assert!(state.content_read_interest(100, 100, true).is_empty());
            assert!(!*interest.borrow());
            assert!(
                state.content_reads[&resource.resource_id]
                    .displayed
                    .is_some()
            );
        }
    }

    #[tokio::test]
    async fn terminal_head_is_not_complete_until_its_final_page_is_consumed() {
        let (_, resource) = terminal_part(1);
        let frame = terminal_frame(&resource);
        let (read, sender, _) = observed(frame.clone(), false);
        // A closed resource may still have unread pages. It must retain a
        // subscription slot until the final page, rather than freeze early.
        assert_eq!(frame.resource.state, ContentState::Complete);
        assert!(!read.is_complete());
        sender.send_replace(ContentUpdate {
            frame: Some(frame),
            error: None,
            complete: true,
        });
        assert!(read.is_complete());
    }
}
