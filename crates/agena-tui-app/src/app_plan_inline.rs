//! Session-owned plan previews share the chat surface with BTW and the editor.
use std::time::{Duration, Instant};

use agena_tui::main_focus::Focus;
use agena_tui_components::{
    pointer::{self, PointerAction},
    theme,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};
use serde_json::Value;

use crate::{App, AppMessage, Route};

#[cfg(test)]
mod tests;

pub(super) fn plan_document(output: &str) -> String {
    if output.starts_with("Revision:") {
        output
            .split_once('\n')
            .map_or("", |(_, body)| body)
            .trim()
            .to_owned()
    } else {
        output.trim().to_owned()
    }
}

pub(super) fn render_plan_document(markdown: &str, width: u16) -> Vec<Line<'static>> {
    agena_tui_transcript::render_markdown_document(markdown, width)
        .into_iter()
        .map(|line| {
            line.rich_line
                .unwrap_or_else(|| Line::from(Span::styled(line.text, line.style)))
        })
        .collect()
}

#[derive(Debug)]
struct PlanTask(tokio::task::JoinHandle<()>);
impl Drop for PlanTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InlinePlanData {
    pub(crate) summary: String,
    title: String,
    current_step: String,
    markdown: String,
    autorun: Option<bool>,
}

impl InlinePlanData {
    fn from_response(output: String, payload: Option<&Value>) -> Self {
        let Some(plan) = payload
            .and_then(|p| p.get("plan"))
            .filter(|p| p.is_object())
        else {
            return Self::default();
        };
        let steps = plan.get("steps").and_then(Value::as_array);
        let total = steps.map_or(0, Vec::len);
        let completed = steps.map_or(0, |steps| {
            steps
                .iter()
                .filter(|step| {
                    matches!(
                        step.get("status").and_then(Value::as_str),
                        Some("completed" | "skipped")
                    )
                })
                .count()
        });
        let phase = plan
            .get("phase")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let symbol = match phase {
            "completed" => "✓",
            "blocked" => "⚠",
            "cancelled" => "✕",
            "planning" => "⏳",
            _ => "▶",
        };
        let autorun = plan.get("autorun").and_then(Value::as_bool);
        let mut summary = symbol.to_owned();
        if total > 0 {
            summary.push_str(&format!(" {completed}/{total}"));
        }
        if autorun == Some(true) {
            summary.push_str(" ↻");
        }
        let markdown = plan_document(&output);
        Self {
            summary,
            title: plan
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            current_step: if matches!(phase, "completed" | "cancelled") {
                String::new()
            } else {
                payload
                    .and_then(|p| p.get("current_step"))
                    .and_then(|s| s.get("title"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            },
            markdown,
            autorun,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct InlinePlanState {
    pub(crate) expanded: bool,
    pub(crate) data: Option<InlinePlanData>,
    error: Option<String>,
    request: Option<PlanTask>,
    request_id: u64,
    mutating: bool,
    refreshed_at: Option<Instant>,
    scroll: u16,
    max_scroll: u16,
    render_key: Option<(String, u16, ratatui::style::Color)>,
    rendered: Vec<Line<'static>>,
}

impl InlinePlanState {
    fn visible(&self) -> bool {
        self.expanded
            || self
                .data
                .as_ref()
                .is_some_and(|data| !data.summary.is_empty())
    }

    fn refresh_due(&self, busy: bool, now: Instant) -> bool {
        if self.request.is_some() {
            return false;
        }
        let seconds = if busy || self.expanded || self.error.is_some() {
            5
        } else if self.visible() {
            30
        } else {
            60
        };
        self.refreshed_at
            .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(seconds))
    }
}

impl App {
    pub(crate) fn open_plan_viewer(&mut self) {
        let Some(id) = self.current_or_selected_session_id() else {
            self.flash_warning(self.i18n.text("flash-plan-viewer-requires-session"));
            return;
        };
        if self.transcript.session_id != Some(id) {
            let title = self.current_or_selected_session_title().unwrap_or_default();
            self.open_session(id, title);
        }
        self.request_inline_plan(id, None);
        self.inline_plans.entry(id).or_default().expanded = true;
        self.collapse_btw_for_plan(id);
        self.current_route = Route::Main;
        self.route_stack.clear();
        self.plan_focus = Some(id);
        self.focus = Focus::Transcript;
    }

    fn collapse_btw_for_plan(&mut self, id: i64) {
        self.collapse_session_work(id);
        self.collapse_btw(id);
        self.btw_focus = None;
    }

    pub(crate) fn collapse_inline_plan(&mut self, id: i64) {
        if let Some(state) = self.inline_plans.get_mut(&id) {
            state.expanded = false;
        }
        self.plan_focus = None;
    }

    // The same read restores the plugin's durable status chip and fills the
    // inline document. No second summary poll is needed.
    pub(crate) fn request_plan_display_refresh(&mut self, id: i64) {
        self.request_inline_plan(id, None);
    }

    pub(crate) fn heal_plan_display_refresh(&mut self) {
        let selected = self.transcript.session_id;
        for (id, state) in &mut self.inline_plans {
            if Some(*id) != selected && !state.mutating && state.request.take().is_some() {
                state.request_id = 0;
                state.refreshed_at = None;
            }
        }
        if !matches!(self.current_route, Route::Main) {
            return;
        }
        let Some(id) = selected else {
            return;
        };
        let busy = self.active_run_session_id() == Some(id);
        if self
            .inline_plans
            .get(&id)
            .is_none_or(|state| state.refresh_due(busy, Instant::now()))
        {
            self.request_inline_plan(id, None);
        }
    }

    fn request_inline_plan(&mut self, id: i64, autorun: Option<bool>) {
        if self
            .inline_plans
            .get(&id)
            .is_some_and(|state| state.request.is_some())
        {
            return;
        }
        if !self.inline_plans.contains_key(&id) && self.inline_plans.len() >= 32 {
            let oldest = self
                .inline_plans
                .iter()
                .filter(|(_, state)| state.request.is_none())
                .min_by_key(|(_, state)| state.refreshed_at)
                .map(|(id, _)| *id);
            if let Some(oldest) = oldest {
                self.inline_plans.remove(&oldest);
            }
        }
        let request_id = self.next_usage_request_id.saturating_add(1);
        self.next_usage_request_id = request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        let state = self.inline_plans.entry(id).or_default();
        state.request_id = request_id;
        state.mutating = autorun.is_some();
        state.error = None;
        state.refreshed_at = Some(Instant::now());
        state.request = Some(PlanTask(tokio::spawn(async move {
            let operation = async {
                if let Some(autorun) = autorun {
                    application
                        .invoke_plugin_tool(
                            "agena.plan",
                            "phase",
                            serde_json::json!({"autorun": autorun}),
                            Some(id),
                        )
                        .await?;
                }
                let response = application
                    .invoke_plugin_tool(
                        "agena.plan",
                        "get",
                        serde_json::json!({"view": "full"}),
                        Some(id),
                    )
                    .await?;
                Ok::<_, anyhow::Error>(InlinePlanData::from_response(
                    response.output_text,
                    response.payload.as_ref(),
                ))
            };
            let result = tokio::time::timeout(Duration::from_secs(30), operation)
                .await
                .map_err(|_| anyhow::anyhow!("Plan request timed out"))
                .and_then(|result| result)
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::InlinePlanLoaded {
                    session_id: id,
                    request_id,
                    result,
                })
                .await;
        })));
    }

    pub(crate) fn handle_inline_plan_loaded(
        &mut self,
        id: i64,
        request_id: u64,
        result: crate::UiResult<InlinePlanData>,
    ) {
        let Some(state) = self.inline_plans.get_mut(&id) else {
            return;
        };
        if state.request_id != request_id || request_id == 0 {
            return;
        }
        state.request = None;
        state.mutating = false;
        state.refreshed_at = Some(Instant::now());
        match result {
            Ok(data) => {
                state.data = Some(data);
                state.error = None;
            }
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    fn inline_plan_has_focus(&self) -> bool {
        self.current_route_is_main()
            && self.overlay.is_none()
            && self.context_help.is_none()
            && self.prompt_history_search.is_none()
            && self.slash_command_suggestions.is_none()
            && self.file_mention_suggestions.is_none()
            && self.plan_focus.is_some()
            && self.plan_focus == self.transcript.session_id
            && self
                .plan_focus
                .and_then(|id| self.inline_plans.get(&id))
                .is_some_and(|s| s.expanded)
    }

    pub(crate) fn handle_inline_plan_key(&mut self, key: KeyEvent) -> bool {
        if !self.inline_plan_has_focus()
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return false;
        }
        let action = match key.code {
            KeyCode::Esc | KeyCode::Tab => "plan-leave",
            KeyCode::Char('q') => "plan-toggle",
            KeyCode::Char('r' | 'R') => "plan-refresh",
            KeyCode::Char('a' | 'A') => "plan-autorun",
            KeyCode::Char('f') => "plan-fullscreen",
            KeyCode::Up | KeyCode::Char('k') | KeyCode::PageUp => "plan-scroll-up",
            KeyCode::Down | KeyCode::Char('j') | KeyCode::PageDown => "plan-scroll-down",
            _ => return false,
        };
        self.handle_inline_plan_action(action);
        true
    }

    pub(crate) fn handle_inline_plan_action(&mut self, action: &str) {
        let Some(id) = self.transcript.session_id else {
            return;
        };
        let Some(state) = self.inline_plans.get_mut(&id) else {
            return;
        };
        match action {
            "plan-toggle" => {
                state.expanded = !state.expanded;
                if state.expanded {
                    self.collapse_btw_for_plan(id);
                    self.plan_focus = Some(id);
                } else {
                    self.plan_focus = None;
                }
            }
            "plan-focus" => {
                self.plan_focus = Some(id);
                self.focus = Focus::Transcript;
            }
            "plan-leave" => {
                self.plan_focus = None;
                self.focus = Focus::Composer;
            }
            "plan-scroll-up" => state.scroll = state.scroll.saturating_sub(3),
            "plan-scroll-down" => {
                state.scroll = state.scroll.saturating_add(3).min(state.max_scroll)
            }
            "plan-refresh" => self.request_inline_plan(id, None),
            "plan-autorun" => {
                let autorun = state.data.as_ref().and_then(|data| data.autorun);
                if let Some(current) = autorun {
                    self.request_inline_plan(id, Some(!current));
                }
            }
            "plan-fullscreen" => self.open_plan_viewer_fullscreen(),
            _ => {}
        }
    }

    pub(crate) fn inline_plan_height(&self, available: u16) -> u16 {
        let Some(state) = self
            .transcript
            .session_id
            .and_then(|id| self.inline_plans.get(&id))
            .filter(|s| s.visible())
        else {
            return 0;
        };
        if state.expanded && available >= 5 {
            available.min(12)
        } else {
            available.min(1)
        }
    }

    pub(crate) fn render_inline_plan(&mut self, frame: &mut Frame, area: Rect) {
        self.plan_area = area;
        if area.is_empty() {
            return;
        }
        let Some(state) = self
            .transcript
            .session_id
            .and_then(|id| self.inline_plans.get_mut(&id))
        else {
            return;
        };
        let expanded = state.expanded && area.height >= 5 && area.width > 2;
        let title = format!(
            "{} {} {}",
            if expanded { "▾" } else { "▸" },
            self.i18n.text("plan-viewer-title"),
            state.data.as_ref().map_or("", |data| data.summary.as_str())
        );
        pointer::render_buttons(
            frame,
            Rect { height: 1, ..area },
            &[(&title, PointerAction::Named("plan-toggle"))],
        );
        let caption = state.data.as_ref().map(|data| {
            if data.current_step.is_empty() {
                &data.title
            } else {
                &data.current_step
            }
        });
        if let Some(caption) = caption {
            let offset = unicode_width::UnicodeWidthStr::width(title.as_str())
                .saturating_add(4)
                .min(usize::from(area.width)) as u16;
            frame.render_widget(
                Paragraph::new(crate::view::sanitize_display_text(caption))
                    .style(theme::muted_style()),
                Rect::new(area.x + offset, area.y, area.width - offset, 1),
            );
        }
        if !expanded {
            return;
        }
        let refresh = self.i18n.text(if state.request.is_some() {
            "plan-viewer-loading"
        } else {
            "plan-viewer-refresh"
        });
        let autorun = self.i18n.text(
            if state.data.as_ref().and_then(|data| data.autorun) == Some(true) {
                "plan-viewer-autorun-on"
            } else {
                "plan-viewer-autorun-off"
            },
        );
        let mut buttons = vec![(refresh.as_str(), PointerAction::Named("plan-refresh"))];
        if state
            .data
            .as_ref()
            .is_some_and(|data| data.autorun.is_some())
        {
            buttons.push((autorun.as_str(), PointerAction::Named("plan-autorun")));
        }
        buttons.push(("↗", PointerAction::Named("plan-fullscreen")));
        pointer::render_buttons(
            frame,
            Rect::new(area.x, area.y + 1, area.width, 1),
            &buttons,
        );
        let body = Rect::new(
            area.x + 1,
            area.y + 2,
            area.width.saturating_sub(2),
            area.height.saturating_sub(3),
        );
        let markdown = state
            .data
            .as_ref()
            .map_or("", |data| data.markdown.as_str());
        if state
            .render_key
            .as_ref()
            .is_none_or(|(text, width, color)| {
                text != markdown || *width != body.width || *color != theme::accent_color()
            })
        {
            state.rendered = render_plan_document(markdown, body.width);
            state.render_key = Some((markdown.to_owned(), body.width, theme::accent_color()));
        }
        let mut lines = state.rendered.clone();
        if let Some(error) = &state.error {
            lines.insert(
                0,
                Line::from(error.clone())
                    .style(ratatui::style::Style::default().fg(theme::danger_color())),
            );
        }
        if lines.is_empty() {
            lines.push(
                Line::from(self.i18n.text(if state.request.is_some() {
                    "plan-viewer-loading"
                } else {
                    "plan-viewer-empty"
                }))
                .style(theme::muted_style()),
            );
        }
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
        state.max_scroll = paragraph
            .line_count(body.width)
            .saturating_sub(usize::from(body.height))
            .min(usize::from(u16::MAX)) as u16;
        state.scroll = state.scroll.min(state.max_scroll);
        frame.render_widget(paragraph.scroll((state.scroll, 0)), body);
        pointer::register(
            body,
            Some(PointerAction::Named("plan-focus")),
            Some((
                PointerAction::Named("plan-scroll-up"),
                PointerAction::Named("plan-scroll-down"),
            )),
        );
        let footer = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        frame.render_widget(
            Paragraph::new(self.i18n.text("plan-inline-footer")).style(theme::muted_style()),
            footer,
        );
        pointer::register(footer, Some(PointerAction::Named("plan-leave")), None);
    }
}
