//! Stateful, live inline reply UI. The terminal owns local input, while the
//! server owns revisioned data/spec. Neither rendering nor reconnection executes
//! actions. /ui opens a keyboard work surface for the selected reply content.
use crate::{App, KeyEvent, Overlay, TranscriptState, TuiBackend};
use agena_domain::{
    InlineBlock, InlineDocument, InlineSnapshot, UiKind, UiProgram, UiResolvedNode, UiState, UiView,
};
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Line,
    widgets::Paragraph,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};
use tokio::sync::watch;
pub(crate) mod native;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Default)]
struct LiveUpdate {
    snapshot: Option<InlineSnapshot>,
    error: Option<String>,
    unavailable: bool,
}
pub(crate) struct InlineUiRead {
    receiver: watch::Receiver<LiveUpdate>,
    task: tokio::task::AbortHandle,
    pub(crate) snapshot: Option<InlineSnapshot>,
    seed: Option<InlineDocument>,
    pub(crate) states: BTreeMap<String, UiState>,
    pub(crate) presentation: BTreeMap<String, agena_tui_transcript::inline_ui::InlinePresentation>,
    error: Option<String>,
    unavailable: bool,
}
impl InlineUiRead {
    fn start(
        application: TuiBackend,
        session: i64,
        reference: String,
        seed: Option<InlineDocument>,
    ) -> Self {
        let (tx, receiver) = watch::channel(LiveUpdate::default());
        let task = tokio::spawn(async move {
            let mut latest = LiveUpdate::default();
            let mut delay = 200;
            loop {
                match application
                    .client()
                    .stream_inline_ui(session, &reference)
                    .await
                {
                    Ok(mut subscription) => {
                        while let Some(result) = subscription.recv().await {
                            match result {
                                Ok(snapshot) => {
                                    if latest
                                        .snapshot
                                        .as_ref()
                                        .is_none_or(|old| snapshot.revision >= old.revision)
                                    {
                                        latest.snapshot = Some(snapshot);
                                        latest.error = None;
                                        latest.unavailable = false;
                                        tx.send_replace(latest.clone());
                                        delay = 200;
                                    }
                                }
                                Err(error) => {
                                    latest.unavailable = matches!(&error, agena_client::ClientError::Api(api) if matches!(api.problem.category, agena_failure::FailureCategory::NotFound | agena_failure::FailureCategory::PermissionDenied | agena_failure::FailureCategory::AuthenticationRequired));
                                    latest.error = Some(error.user_message());
                                    if latest.unavailable {
                                        latest.snapshot = None;
                                    }
                                    tx.send_replace(latest.clone());
                                    if latest.unavailable {
                                        return;
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => {
                        latest.unavailable = matches!(&error, agena_client::ClientError::Api(api) if matches!(api.problem.category, agena_failure::FailureCategory::NotFound | agena_failure::FailureCategory::PermissionDenied | agena_failure::FailureCategory::AuthenticationRequired));
                        latest.error = Some(error.user_message());
                        if latest.unavailable {
                            latest.snapshot = None;
                        }
                        tx.send_replace(latest.clone());
                        if latest.unavailable {
                            return;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(delay)).await;
                delay = (delay * 2).min(5000);
            }
        });
        Self {
            receiver,
            task: task.abort_handle(),
            snapshot: None,
            seed,
            states: BTreeMap::new(),
            presentation: BTreeMap::new(),
            error: None,
            unavailable: false,
        }
    }
    pub(crate) fn document(&self) -> Option<&InlineDocument> {
        if self.unavailable {
            None
        } else {
            self.snapshot
                .as_ref()
                .map(|s| &s.document)
                .or(self.seed.as_ref())
        }
    }
    fn sync(&mut self) -> bool {
        if !self.receiver.has_changed().unwrap_or(false) {
            return false;
        }
        let latest = self.receiver.borrow_and_update().clone();
        let changed = self.snapshot != latest.snapshot
            || self.error != latest.error
            || self.unavailable != latest.unavailable;
        self.snapshot = latest.snapshot;
        self.error = latest.error;
        self.unavailable = latest.unavailable;
        if changed {
            let fields = self
                .document()
                .map(|d| {
                    d.blocks
                        .iter()
                        .filter_map(|b| match b {
                            InlineBlock::Panel { id, program } => Some((
                                id.clone(),
                                program.reconcile_state(
                                    self.states.get(id).unwrap_or(&UiState::new()),
                                ),
                            )),
                            _ => None,
                        })
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            self.states = fields;
            let schema = self
                .document()
                .map(|document| {
                    document
                        .blocks
                        .iter()
                        .map(|block| {
                            let nodes = match block {
                                InlineBlock::Panel { program, .. } => program
                                    .nodes
                                    .iter()
                                    .map(|node| node.id.clone())
                                    .collect::<BTreeSet<_>>(),
                                _ => BTreeSet::from([block.id().to_owned()]),
                            };
                            (block.id().to_owned(), nodes)
                        })
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            self.presentation.retain(|block, presentation| {
                let Some(nodes) = schema.get(block) else {
                    return false;
                };
                presentation.open.retain(|id| nodes.contains(id));
                presentation.data.retain(|id| nodes.contains(id));
                presentation.hidden_series.retain(|id, series| {
                    series.retain(|index| *index < 6);
                    nodes.contains(id) && !series.is_empty()
                });
                true
            });
            if self.error.is_none() {
                self.error = self.document().and_then(|document| {
                    document.blocks.iter().find_map(|block| match block {
                        InlineBlock::Panel { id, program } => program
                            .render(self.states.get(id).unwrap_or(&UiState::new()))
                            .err(),
                        _ => None,
                    })
                });
            }
        }
        changed
    }
    #[cfg(test)]
    fn markdown(&self) -> Option<String> {
        self.document()
            .map(|doc| doc.to_markdown_with_states(&self.states))
    }
}
impl Drop for InlineUiRead {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InlineUiDialog {
    pub(crate) editor_only: bool,
    pub(crate) choice: Option<String>,
    pub(crate) edit_cursor: Option<usize>,
    pub(crate) pointer: Option<native::InlinePointer>,
    pub(crate) picker: bool,
    pub(crate) native_focus: Option<agena_tui_transcript::inline_ui::InlineControl>,
    pub(crate) native_layout: std::cell::RefCell<native::NativeLayout>,
    pub(crate) reveal_native: std::cell::Cell<bool>,

    reference: String,
    focused: String,
    scroll: u16,
    // Derived viewport limit: rendering clamps it after live content shrinks.
    scroll_limit: std::cell::Cell<u16>,
    pub(crate) editing: Option<String>,
    error: Option<String>,
}
fn field(node: &UiResolvedNode, key: &str) -> String {
    node.props.get(key).map(value_text).unwrap_or_default()
}
fn value_text(v: &Value) -> String {
    match v {
        Value::Null => "—".into(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map(|v| v.to_string()).unwrap_or_default(),
        _ => v.to_string(),
    }
}
pub(crate) type InlineUiCopyState = BTreeMap<String, (InlineDocument, BTreeMap<String, UiState>)>;

impl TranscriptState {
    pub(crate) fn inline_ui_copy_state(&self) -> InlineUiCopyState {
        self.inline_reads
            .iter()
            .filter_map(|(reference, read)| {
                read.document()
                    .map(|document| (reference.clone(), (document.clone(), read.states.clone())))
            })
            .collect()
    }

    fn inline_documents(
        &self,
        seed: Option<&agena_domain::InlineUiReferences>,
    ) -> agena_domain::InlineUiReferences {
        let mut docs = seed.cloned().unwrap_or_default();
        for (reference, read) in &self.inline_reads {
            if let Some(doc) = read.document() {
                docs.insert(reference.clone(), doc.clone());
            } else if read.unavailable {
                docs.remove(reference);
            }
        }
        docs
    }
    pub(crate) fn inline_ui_part(
        &self,
        part: &agena_api::part::PartResource,
    ) -> agena_api::part::PartResource {
        let mut projected = part.clone();
        if part.kind == "text" && part.role == "assistant" && part.inline_ui.is_some() {
            projected.inline_ui = Some(self.inline_documents(part.inline_ui.as_ref()));
        }
        projected
    }
    pub(crate) fn native_inline_reply(
        &self,
        source: String,
        seed: Option<&agena_domain::InlineUiReferences>,
        streaming: bool,
    ) -> agena_tui_transcript::inline_ui::InlineReply {
        agena_tui_transcript::inline_ui::InlineReply {
            source,
            documents: self.inline_documents(seed),
            streaming,
            focused: false,
            states: self
                .inline_reads
                .iter()
                .map(|(reference, read)| (reference.clone(), read.states.clone()))
                .collect(),
            presentation: self
                .inline_reads
                .iter()
                .map(|(reference, read)| (reference.clone(), read.presentation.clone()))
                .collect(),
            phases: self
                .inline_reads
                .iter()
                .filter_map(|(reference, read)| {
                    read.snapshot
                        .as_ref()
                        .map(|snapshot| (reference.clone(), (snapshot.revision, snapshot.phase)))
                })
                .collect(),
        }
    }
}
impl App {
    pub(crate) fn heal_inline_ui_reads(&mut self) {
        let Some(session) = self.transcript.session_id else {
            self.transcript.inline_reads.clear();
            return;
        };
        let mut references = BTreeSet::new();
        let mut seeds = BTreeMap::new();
        for part in &self.transcript.parts {
            if part.role != "assistant" || part.kind != "text" {
                continue;
            }
            if let Some(text) = part.content.get("text").and_then(Value::as_str) {
                for span in agena_domain::inline_reference_spans(text, true) {
                    if let Some(reference) = span.reference {
                        references.insert(reference);
                    }
                }
            }
            if let Some(docs) = &part.inline_ui {
                seeds.extend(docs.clone());
            }
        }
        for read in self.transcript.content_reads.values() {
            if let Some(frame) = &read.displayed
                && let Some(docs) = &frame.inline_ui
            {
                seeds.extend(docs.clone());
                for span in agena_domain::inline_reference_spans(&frame.source_text, true) {
                    if let Some(reference) = span.reference {
                        references.insert(reference);
                    }
                }
            }
        }
        // A dialog can be opened for an authorized reference not in the loaded
        // history window. Keep its subscription until the dialog is closed.
        if let Some(Overlay::InlineUi(dialog)) = &self.overlay {
            references.insert(dialog.reference.clone());
        }
        self.transcript
            .inline_reads
            .retain(|reference, _| references.contains(reference));
        if self.current_route_is_main() {
            for reference in references
                .into_iter()
                .take(agena_domain::INLINE_UI_MAX_REFERENCES)
            {
                self.transcript
                    .inline_reads
                    .entry(reference.clone())
                    .or_insert_with(|| {
                        InlineUiRead::start(
                            self.application.clone(),
                            session,
                            reference.clone(),
                            seeds.remove(&reference),
                        )
                    });
            }
        }
        let pending = self
            .transcript
            .inline_reads
            .values()
            .any(|read| read.receiver.has_changed().unwrap_or(false));
        let focus = if pending {
            self.transcript
                .control_at_cursor(self.layout.transcript_body.width)
                .and_then(|control| match control.action {
                    agena_tui_transcript::TranscriptControlAction::InlineUi(target) => Some(target),
                    _ => None,
                })
        } else {
            None
        };
        let mut changed = false;
        for read in self.transcript.inline_reads.values_mut() {
            changed |= read.sync();
        }
        if changed {
            // Old pointer columns can no longer be trusted after a layout revision.
            self.transcript.inline_pointer = None;
            if let Some(Overlay::InlineUi(dialog)) = &mut self.overlay {
                dialog.pointer = None;
            }
            self.transcript.invalidate_render();
            if let Some(target) = focus {
                self.transcript.restore_inline_focus(
                    self.layout.transcript_body.width,
                    self.layout.transcript_body.height,
                    &target,
                );
            }
        }
    }
    pub(crate) fn open_inline_ui(&mut self, args: &str) {
        let Some(session) = self.transcript.session_id else {
            self.flash_warning(self.i18n.text("inline-ui-no-content"));
            return;
        };
        self.heal_inline_ui_reads();
        let reference = if args.trim().is_empty() {
            self.transcript
                .inline_reads
                .keys()
                .max_by_key(|reference| {
                    agena_domain::inline_reference_tool_id(reference).unwrap_or_default()
                })
                .cloned()
        } else {
            Some(args.trim().to_owned())
        };
        let Some(reference) =
            reference.filter(|s| agena_domain::inline_reference_tool_id(s).is_some())
        else {
            self.flash_warning(self.i18n.text("inline-ui-no-content"));
            return;
        };
        self.transcript
            .inline_reads
            .entry(reference.clone())
            .or_insert_with(|| {
                InlineUiRead::start(self.application.clone(), session, reference.clone(), None)
            });
        self.overlay = Some(Overlay::InlineUi(InlineUiDialog {
            reference,
            ..Default::default()
        }));
    }
    pub(crate) fn handle_inline_ui_key(
        &mut self,
        key: KeyEvent,
        dialog: &mut InlineUiDialog,
    ) -> bool {
        let Some(read) = self.transcript.inline_reads.get_mut(&dialog.reference) else {
            return key.code == KeyCode::Esc;
        };
        let effect = native::handle_modal_key(dialog, read, key, &self.i18n);
        if effect.changed {
            self.transcript.invalidate_render();
        }
        if let Some(text) = effect.copy {
            self.request_clipboard_copy(text, self.i18n.text("inline-ui-copied"));
        }
        effect.close
    }
    pub(crate) fn render_inline_ui_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &InlineUiDialog,
    ) {
        render_dialog(
            frame,
            area,
            dialog,
            self.transcript.inline_reads.get(&dialog.reference),
            &self.i18n,
        );
    }
}

fn render_dialog(
    frame: &mut Frame,
    area: Rect,
    dialog: &InlineUiDialog,
    read: Option<&InlineUiRead>,
    i18n: &agena_tui::i18n::I18n,
) {
    native::render_modal(frame, area, dialog, read, i18n);
}

#[derive(Default)]
struct InlineKeyEffect {
    close: bool,
    changed: bool,
    copy: Option<String>,
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod native_tests;
