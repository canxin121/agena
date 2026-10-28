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
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::watch;

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
    reference: String,
    panel: usize,
    focused: String,
    scroll: u16,
    pub(crate) editing: Option<String>,
    error: Option<String>,
}
#[derive(Debug, Clone)]
struct Control {
    id: String,
    node: String,
    item: Option<String>,
    label: String,
    value: String,
    kind: UiKind,
    disabled: bool,
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
fn controls(view: &UiView) -> Vec<Control> {
    let mut out = Vec::new();
    for node in view.root.controls() {
        let disabled = node.props.get("disabled") == Some(&Value::Bool(true));
        if node.kind == UiKind::Tasks {
            for row in node
                .props
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let id = row["id"].as_str().unwrap_or("");
                let selected = node
                    .value
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(id)));
                out.push(Control {
                    id: format!("{}:{id}", node.id),
                    node: node.id.clone(),
                    item: Some(id.into()),
                    label: row["title"].as_str().unwrap_or("").into(),
                    value: if selected { "☑" } else { "☐" }.into(),
                    kind: node.kind,
                    disabled,
                });
            }
        } else {
            let value = match node.kind {
                UiKind::Tabs | UiKind::Select => node
                    .field
                    .as_ref()
                    .and_then(|f| {
                        f.options
                            .iter()
                            .find(|o| Some(o.value.as_str()) == node.value.as_str())
                    })
                    .map(|o| o.label.clone())
                    .unwrap_or_default(),
                UiKind::Checkbox | UiKind::Switch => if node.value.as_bool() == Some(true) {
                    "☑"
                } else {
                    "☐"
                }
                .into(),
                UiKind::Button => "↵".into(),
                _ => value_text(&node.value),
            };
            out.push(Control {
                id: node.id.clone(),
                node: node.id.clone(),
                item: None,
                label: field(node, "label"),
                value,
                kind: node.kind,
                disabled,
            });
        }
    }
    out
}
fn panel(read: &InlineUiRead, index: usize) -> Option<(String, UiProgram, UiView)> {
    let (id, program) = read
        .document()?
        .blocks
        .iter()
        .filter_map(|b| match b {
            InlineBlock::Panel { id, program } => Some((id, program)),
            _ => None,
        })
        .nth(index)?;
    let view = program
        .render(read.states.get(id).unwrap_or(&UiState::new()))
        .ok()?;
    Some((id.clone(), program.clone(), view))
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
        if part.kind != "text" || part.role != "assistant" {
            return part.clone();
        }
        let Some(text) = part.content.get("text").and_then(Value::as_str) else {
            return part.clone();
        };
        if !text.contains("inline-ui") {
            return part.clone();
        }
        let docs = self.inline_documents(part.inline_ui.as_ref());
        let text = agena_domain::expand_inline_references_with(
            text,
            &docs,
            part.state != "completed",
            |reference, document| {
                self.inline_reads
                    .get(reference)
                    .and_then(InlineUiRead::markdown)
                    .unwrap_or_else(|| document.to_markdown())
            },
        );
        let mut projected = part.clone();
        projected.content["text"] = json!(text);
        projected.inline_ui = None;
        projected
    }
    pub(crate) fn inline_ui_frame(
        &self,
        frame: Arc<agena_tui_transcript::content::ContentFrame>,
    ) -> Arc<agena_tui_transcript::content::ContentFrame> {
        if frame.inline_ui.is_none() || !frame.source_text.contains("inline-ui") {
            return frame;
        }
        let docs = self.inline_documents(frame.inline_ui.as_ref());
        let text = agena_domain::expand_inline_references_with(
            &frame.source_text,
            &docs,
            frame.resource.state == agena_domain::ContentState::Active
                || frame.resource.cursor.sequence > frame.resource.committed_cursor.sequence,
            |reference, document| {
                self.inline_reads
                    .get(reference)
                    .and_then(InlineUiRead::markdown)
                    .unwrap_or_else(|| document.to_markdown())
            },
        );
        if text.as_str() == frame.text.as_ref() {
            return frame;
        }
        let mut projected = (*frame).clone();
        projected.text = Arc::from(text);
        Arc::new(projected)
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
            for reference in references.into_iter().take(16) {
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
        let mut changed = false;
        for read in self.transcript.inline_reads.values_mut() {
            changed |= read.sync();
        }
        if changed {
            self.transcript.invalidate_render();
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
        let effect = handle_dialog_key(dialog, read, key, &self.i18n);
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
    let margin = if area.width > 50 { 2 } else { 0 };
    let surface = Rect {
        x: area.x + margin,
        y: area.y + 1,
        width: area.width.saturating_sub(margin * 2),
        height: area.height.saturating_sub(2),
    };
    frame.render_widget(Clear, surface);
    let border = Block::default()
        .borders(Borders::ALL)
        .title(i18n.text("inline-ui-title"))
        .border_style(Style::default().fg(agena_tui_components::theme::accent_color()));
    let inner = border.inner(surface);
    frame.render_widget(border, surface);
    let Some(read) = read else {
        frame.render_widget(Paragraph::new(i18n.text("inline-ui-loading")), inner);
        return;
    };
    let status = read
        .snapshot
        .as_ref()
        .map(|s| {
            format!(
                "{} · r{} · {}",
                s.document.title,
                s.revision,
                i18n.text(match s.phase {
                    agena_domain::InlinePhase::Working => "inline-ui-working",
                    agena_domain::InlinePhase::Complete => "inline-ui-complete",
                    agena_domain::InlinePhase::Error => "inline-ui-error",
                })
            )
        })
        .unwrap_or_else(|| i18n.text("inline-ui-loading"));
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(if dialog.editing.is_some() || dialog.error.is_some() {
                4
            } else {
                2
            }),
        ])
        .split(inner);
    frame.render_widget(
        Paragraph::new(status)
            .style(Style::default().add_modifier(Modifier::BOLD))
            .wrap(Wrap { trim: false }),
        layout[0],
    );
    let Some((_, _, view)) = panel(read, dialog.panel) else {
        frame.render_widget(
            Paragraph::new(format!(
                "{}\n\n{}",
                read.error
                    .clone()
                    .unwrap_or_else(|| i18n.text("inline-ui-loading")),
                i18n.text("inline-ui-reset-help")
            ))
            .wrap(Wrap { trim: false }),
            layout[1],
        );
        return;
    };
    let all = controls(&view);
    let focused = all.iter().position(|c| c.id == dialog.focused).unwrap_or(0);
    let split = if inner.width >= 90 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(61), Constraint::Percentage(39)])
            .split(layout[1])
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(layout[1])
    };
    let mut markdown = Vec::new();
    agena_tui_transcript::push_markdown(
        &mut markdown,
        "",
        &view.to_markdown(),
        split[0].width.saturating_sub(2),
    );
    let lines = markdown
        .into_iter()
        .map(|line| {
            line.rich_line
                .unwrap_or_else(|| Line::styled(line.text, line.style))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(if inner.width >= 90 {
                Borders::RIGHT
            } else {
                Borders::NONE
            }))
            .scroll((dialog.scroll, 0))
            .wrap(Wrap { trim: false }),
        split[0],
    );
    let height = usize::from(split[1].height.saturating_sub(2));
    let start = if focused >= height {
        focused + 1 - height
    } else {
        0
    };
    let rows = all
        .iter()
        .enumerate()
        .skip(start)
        .take(height.max(1))
        .map(|(i, c)| {
            let label = format!(
                "{} {}  {}",
                if i == focused { "›" } else { " " },
                c.label,
                c.value
            );
            let style = if c.disabled {
                Style::default().fg(agena_tui_components::theme::muted_color())
            } else if i == focused {
                Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(label, style)))
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(rows).block(
            Block::default()
                .title(i18n.text("inline-ui-controls"))
                .borders(if inner.width >= 90 {
                    Borders::NONE
                } else {
                    Borders::TOP
                }),
        ),
        split[1],
    );
    let mut footer = Vec::new();
    if let Some(value) = &dialog.editing {
        footer.push(Line::raw(format!(
            "{}: {value}▏",
            i18n.text("inline-ui-edit")
        )));
    }
    if let Some(error) = &dialog.error {
        footer.push(Line::styled(
            error.clone(),
            Style::default().fg(agena_tui_components::theme::danger_color()),
        ));
    }
    if let Some(error) = &read.error {
        footer.push(Line::raw(error.clone()));
    }
    footer.push(Line::raw(i18n.text(if inner.width < 65 {
        "inline-ui-keys-short"
    } else {
        "inline-ui-keys"
    })));
    frame.render_widget(
        Paragraph::new(footer)
            .style(Style::default().fg(agena_tui_components::theme::muted_color()))
            .wrap(Wrap { trim: false }),
        layout[2],
    );
}

#[derive(Default)]
struct InlineKeyEffect {
    close: bool,
    changed: bool,
    copy: Option<String>,
}
fn handle_dialog_key(
    dialog: &mut InlineUiDialog,
    read: &mut InlineUiRead,
    key: KeyEvent,
    i18n: &agena_tui::i18n::I18n,
) -> InlineKeyEffect {
    let mut effect = InlineKeyEffect::default();
    if key.code == KeyCode::Esc {
        if dialog.editing.take().is_none() {
            effect.close = true;
        }
        dialog.error = None;
        return effect;
    }
    match key.code {
        KeyCode::PageDown => {
            dialog.scroll = dialog.scroll.saturating_add(8);
            return effect;
        }
        KeyCode::PageUp => {
            dialog.scroll = dialog.scroll.saturating_sub(8);
            return effect;
        }
        _ => {}
    }
    if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
        let initial = read.document().and_then(|document| {
            document
                .blocks
                .iter()
                .filter_map(|block| match block {
                    InlineBlock::Panel { id, program } => {
                        Some((id.clone(), program.reconcile_state(&UiState::new())))
                    }
                    _ => None,
                })
                .nth(dialog.panel)
        });
        if let Some((id, state)) = initial {
            read.states.insert(id, state);
            read.error = None;
            dialog.error = None;
            dialog.editing = None;
            effect.changed = true;
        }
        return effect;
    }
    let Some((block_id, program, view)) = panel(read, dialog.panel) else {
        return effect;
    };
    let all = controls(&view);
    if all.is_empty() {
        return effect;
    }
    let selected = all.iter().position(|c| c.id == dialog.focused);
    if selected.is_none() && dialog.editing.is_some() {
        dialog.editing = None;
        dialog.error = Some(i18n.text("inline-ui-control-changed"));
        return effect;
    }
    let index = selected.unwrap_or(0);
    let control = all[index].clone();
    let mut input = None;
    if let Some(buffer) = &mut dialog.editing {
        match key.code {
            KeyCode::Enter => {
                let raw = buffer.clone();
                dialog.editing = None;
                input = if matches!(control.kind, UiKind::Number | UiKind::Slider) {
                    match raw.trim().parse::<f64>() {
                        Ok(value) if value.is_finite() => Some(json!(value)),
                        _ => {
                            dialog.error = Some(i18n.text("inline-ui-invalid-number"));
                            return effect;
                        }
                    }
                } else {
                    Some(json!(raw))
                };
            }
            KeyCode::Backspace => {
                buffer.pop();
                return effect;
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && buffer.chars().count() < 6000 =>
            {
                buffer.push(c);
                return effect;
            }
            _ => return effect,
        }
    } else {
        match key.code {
            KeyCode::Tab | KeyCode::BackTab => {
                let reverse =
                    key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                let next = if reverse {
                    (index + all.len() - 1) % all.len()
                } else {
                    (index + 1) % all.len()
                };
                dialog.focused = all[next].id.clone();
                dialog.error = None;
                return effect;
            }
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let count = read
                    .document()
                    .map(|d| {
                        d.blocks
                            .iter()
                            .filter(|b| matches!(b, InlineBlock::Panel { .. }))
                            .count()
                    })
                    .unwrap_or_default();
                if count > 0 {
                    dialog.panel = if key.code == KeyCode::Char(']') {
                        (dialog.panel + 1) % count
                    } else {
                        (dialog.panel + count - 1) % count
                    };
                    dialog.focused.clear();
                    dialog.scroll = 0;
                }
                return effect;
            }
            _ => {}
        }
        if control.disabled {
            return effect;
        }
        dialog.focused = control.id.clone();
        dialog.error = None;
        let Some(node) = view.root.find(&control.node) else {
            return effect;
        };
        match control.kind {
            UiKind::Tabs | UiKind::Select
                if matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                ) =>
            {
                let options = node
                    .field
                    .as_ref()
                    .map(|f| f.options.as_slice())
                    .unwrap_or(&[]);
                if options.is_empty() {
                    return effect;
                }
                let current = options
                    .iter()
                    .position(|o| Some(o.value.as_str()) == node.value.as_str())
                    .unwrap_or_default();
                let next = if key.code == KeyCode::Left {
                    (current + options.len() - 1) % options.len()
                } else {
                    (current + 1) % options.len()
                };
                input = Some(json!(options[next].value));
            }
            UiKind::Checkbox | UiKind::Switch
                if matches!(key.code, KeyCode::Char(' ') | KeyCode::Enter) =>
            {
                input = Some(json!(!node.value.as_bool().unwrap_or(false)))
            }
            UiKind::Tasks if matches!(key.code, KeyCode::Char(' ') | KeyCode::Enter) => {
                let item = json!(control.item.unwrap_or_default());
                let mut values = node.value.as_array().cloned().unwrap_or_default();
                if values.contains(&item) {
                    values.retain(|value| value != &item);
                } else {
                    values.push(item);
                }
                input = Some(json!(values));
            }
            UiKind::Slider | UiKind::Number => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                    let Some(field) = &node.field else {
                        return effect;
                    };
                    let min = field
                        .min
                        .as_ref()
                        .and_then(|n| n.as_f64())
                        .unwrap_or_default();
                    let max = field.max.as_ref().and_then(|n| n.as_f64()).unwrap_or(100.0);
                    let step = field.step.as_ref().and_then(|n| n.as_f64()).unwrap_or(1.0);
                    let value = match key.code {
                        KeyCode::Home => min,
                        KeyCode::End => max,
                        KeyCode::Left => node.value.as_f64().unwrap_or(min) - step,
                        _ => node.value.as_f64().unwrap_or(min) + step,
                    }
                    .clamp(min, max);
                    input = Some(json!(value));
                }
                KeyCode::Enter => dialog.editing = Some(value_text(&node.value)),
                _ => {}
            },
            UiKind::Input if key.code == KeyCode::Enter => {
                dialog.editing = Some(node.value.as_str().unwrap_or("").into())
            }
            UiKind::Button if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                match program.activate(&view.state, &node.id) {
                    Ok(result) => {
                        effect.changed = result.state != view.state;
                        read.states.insert(block_id.clone(), result.state);
                        effect.copy = result.copy;
                    }
                    Err(error) => dialog.error = Some(error),
                }
            }
            _ => {}
        }
    }
    if let Some(value) = input {
        match program.input(&view.state, &control.node, value) {
            Ok(result) => {
                effect.changed = result.state != view.state;
                read.states.insert(block_id, result.state);
            }
            Err(error) => dialog.error = Some(error),
        }
    }
    effect
}

#[cfg(test)]
mod tests;
