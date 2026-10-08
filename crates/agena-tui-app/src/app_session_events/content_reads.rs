use crate::App;
use agena_domain::{ContentId, ContentState};
use agena_tui_transcript::content::{ContentFrame, ContentView};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

pub(crate) struct ContentRead {
    receiver: watch::Receiver<(Option<Arc<ContentFrame>>, Option<String>)>,
    task: tokio::task::AbortHandle,
    pub(crate) displayed: Option<Arc<ContentFrame>>,
    pub(crate) displayed_error: Option<String>,
}

impl Drop for ContentRead {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
impl ContentRead {
    pub(crate) fn fixture(frame: Arc<ContentFrame>) -> Self {
        let (_, receiver) = watch::channel((Some(frame.clone()), None));
        let task = tokio::spawn(std::future::pending::<()>());
        Self {
            receiver,
            task: task.abort_handle(),
            displayed: Some(frame),
            displayed_error: None,
        }
    }
}

impl App {
    /// A local interest set and frame scheduler. Collapsed/offscreen resources
    /// perform no ongoing work; a watch slot coalesces arbitrarily fast output.
    pub(crate) fn heal_content_reads(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.transcript.content_reads.clear();
            return;
        };
        if !self.current_route_is_main() {
            self.transcript.content_reads.clear();
            return;
        }
        let width = self.layout.transcript_body.width.max(1);
        let top = self.transcript.viewport_top();
        let end = top.saturating_add(self.layout.transcript_body.height as usize);
        let expanded = self
            .transcript
            .rendered(width)
            .nodes
            .iter()
            .filter(|node| node.expanded && node.start_line < end && node.end_line > top)
            .filter_map(|node| match node.key {
                agena_tui_transcript::TranscriptNodeKey::Activity {
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
        let interested = self
            .transcript
            .parts
            .iter()
            .filter(|part| expanded.contains(&part.part_id))
            .filter_map(|part| {
                part.content
                    .get("resources")
                    .and_then(serde_json::Value::as_array)
            })
            .flatten()
            .filter_map(|value| value.get("resource_id").cloned())
            .filter_map(|value| serde_json::from_value::<ContentId>(value).ok())
            .take(16)
            .collect::<BTreeSet<_>>();
        self.transcript
            .content_reads
            .retain(|id, _| interested.contains(id));
        let paused =
            self.transcript.has_active_text_selection() || !self.transcript.viewport.follow_tail;
        let mut changed = false;
        for id in interested {
            let application = self.application.clone();
            let reconnecting = self.i18n.text("content-reconnecting");
            let read = self.transcript.content_reads.entry(id).or_insert_with(|| {
                let (tx, receiver) = watch::channel((None, None));
                let task = tokio::spawn(async move {
                    let mut view = ContentView::default();
                    let mut failures = 0u32;
                    loop {
                        if tx.is_closed() { break; }
                        match application.stream_content(session_id, id, view.cursor).await {
                            Ok(mut subscription) => {
                                failures = 0;
                                while let Some(result) = subscription.recv().await {
                                    match result {
                                        Ok(page) => {
                                            let done = page.resource.state != ContentState::Active && !page.has_more;
                                            if !view.apply(page) { break; }
                                            tx.send_replace((view.frame.clone(), None));
                                            if done { return; }
                                        }
                                        Err(error) => {
                                            tracing::debug!(%id, diagnostic = %error.operator_diagnostic(), "content subscription reconnecting");
                                            tx.send_replace((view.frame.clone(), Some(error.user_message())));
                                            break;
                                        }
                                    }
                                }
                            }
                            Err(error) => {
                                tracing::debug!(%id, %error, "content subscription unavailable");
                                tx.send_replace((view.frame.clone(), Some(error.to_string())));
                            },
                        }
                        if tx.borrow().1.is_none() { tx.send_replace((view.frame.clone(), Some(reconnecting.clone()))); }
                        failures = failures.saturating_add(1);
                        tokio::select! {
                            _ = tx.closed() => return,
                            _ = tokio::time::sleep(Duration::from_millis(250 * 2u64.pow(failures.min(5)))) => {},
                        }
                    }
                });
                ContentRead { receiver, task: task.abort_handle(), displayed: None, displayed_error: None }
            });
            // Selection and scrolling freeze the displayed cells, while the
            // source cursor continues advancing in the single bounded slot.
            if !paused || read.displayed.is_none() {
                let (latest, _) = read.receiver.borrow_and_update().clone();
                if latest
                    .as_ref()
                    .zip(read.displayed.as_ref())
                    .is_none_or(|(new, old)| !Arc::ptr_eq(new, old))
                {
                    changed |= latest.is_some();
                    read.displayed = latest;
                }
            }
            let error = read.receiver.borrow().1.clone();
            if read.displayed_error != error {
                changed = true;
                read.displayed_error = error;
            }
        }
        if changed {
            self.transcript.invalidate_content_render();
        }
    }
}
