use std::fmt::Write as _;

use agena_domain::{
    ExternalAgent, ExternalAgentHistoryPreview, ExternalAgentSessionInfo,
    ExternalAgentWorkspaceAction as WorkspaceAction,
    ExternalAgentWorkspaceResult as WorkspaceResult,
};
use agena_tui_components::{
    FramedSurfaceSpec, ScrollState, SurfaceMode, layout_header_body_footer_surface, pointer,
    render_framed_surface,
};
use anyhow::bail;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::NativeAgentPage;
use crate::{App, I18n, Overlay, view::sanitize_display_text};

const HISTORY_WIDTH: u16 = 92;

#[derive(Clone, Debug)]
struct HistoryTarget {
    workspace: i64,
    agent: ExternalAgent,
    session: ExternalAgentSessionInfo,
}

#[derive(Clone, Debug)]
pub(crate) struct NativeAgentHistory {
    target: HistoryTarget,
    title: String,
    body: String,
    cursors: Vec<u64>,
    earlier: Option<u64>,
    scroll: ScrollState,
}

impl NativeAgentHistory {
    fn new(
        target: HistoryTarget,
        cursors: Vec<u64>,
        preview: ExternalAgentHistoryPreview,
        i18n: &I18n,
    ) -> Self {
        let name = target
            .session
            .title
            .as_deref()
            .unwrap_or(&target.session.native_id);
        let title = sanitize_display_text(format!(
            "{} · {name}",
            i18n.text("external-agent-preview-history")
        ));
        let mut body = String::new();
        if preview.omitted_content {
            let _ = writeln!(body, "{}\n", i18n.text("external-agent-preview-omitted"));
        }
        if preview.turns.is_empty() {
            let _ = writeln!(body, "{}", i18n.text("external-agent-preview-empty"));
        }
        for turn in preview.turns {
            for (role, text) in [
                ("message-role-user", turn.user),
                ("message-role-assistant", turn.reply),
            ] {
                if !text.is_empty() {
                    let _ = writeln!(
                        body,
                        "{}\n{}\n",
                        i18n.text(role),
                        sanitize_display_text(text)
                    );
                }
            }
        }
        Self {
            target,
            title,
            body,
            cursors,
            earlier: preview.next_before_turn,
            scroll: ScrollState::default(),
        }
    }

    fn paragraph(&self) -> Paragraph<'_> {
        Paragraph::new(self.body.as_str()).wrap(Wrap { trim: false })
    }

    fn scroll_limit(&self, body: Rect) -> u16 {
        let lines = self.paragraph().line_count(body.width.max(1));
        u16::try_from(lines.saturating_sub(usize::from(body.height))).unwrap_or(u16::MAX)
    }
}

impl App {
    pub(super) fn open_native_history(
        &mut self,
        page: NativeAgentPage,
        session: ExternalAgentSessionInfo,
    ) {
        let target = HistoryTarget {
            workspace: page.workspace_id,
            agent: page.agent,
            session,
        };
        self.load_native_history(target, Vec::new(), None);
    }

    fn load_native_history(
        &mut self,
        target: HistoryTarget,
        cursors: Vec<u64>,
        fallback: Option<Overlay>,
    ) {
        let native_id = target.session.native_id.clone();
        let workspace = target.workspace;
        let agent = target.agent;
        let before_turn = cursors.last().copied();
        self.native_agent_request(
            "external-agent-preview-history",
            fallback,
            move |backend| async move {
                let result = backend
                    .client()
                    .external_agent_workspace(
                        workspace,
                        WorkspaceAction::PreviewHistory {
                            agent,
                            native_id,
                            before_turn,
                        },
                    )
                    .await?;
                let WorkspaceResult::HistoryPreview { preview } = result else {
                    bail!("expected a native history preview")
                };
                Ok(preview)
            },
            move |app, preview| {
                app.overlay = Some(Overlay::NativeAgentHistory(Box::new(
                    NativeAgentHistory::new(target, cursors, preview, &app.i18n),
                )));
            },
        );
    }

    pub(crate) fn handle_native_history_key(
        &mut self,
        key: KeyEvent,
        history: &mut NativeAgentHistory,
    ) -> bool {
        if key.modifiers.is_empty() {
            let mut cursors = history.cursors.clone();
            match key.code {
                KeyCode::Esc => return true,
                KeyCode::Left if history.earlier.is_some() => cursors.extend(history.earlier),
                KeyCode::Right if !cursors.is_empty() => {
                    cursors.pop();
                }
                _ => {
                    let area = self.layout.overlay_area;
                    let outer = SurfaceMode::Overlay.outer_rect(area, HISTORY_WIDTH, area.height);
                    let inner = Block::default().borders(Borders::ALL).inner(outer);
                    let body = layout_header_body_footer_surface(inner, 0, 1, 0).body;
                    history.scroll.handle_navigation_key(
                        key,
                        history.scroll_limit(body),
                        body.height,
                    );
                    return false;
                }
            }
            let fallback = Overlay::NativeAgentHistory(Box::new(history.clone()));
            self.load_native_history(history.target.clone(), cursors, Some(fallback));
        }
        false
    }

    pub(crate) fn render_native_history(
        &self,
        frame: &mut Frame,
        area: Rect,
        history: &NativeAgentHistory,
    ) {
        let surface = render_framed_surface(
            frame,
            area,
            SurfaceMode::Overlay,
            &FramedSurfaceSpec {
                title: history.title.as_str().into(),
                target_width: HISTORY_WIDTH,
                target_height: area.height,
            },
        );
        let layout = layout_header_body_footer_surface(surface.inner, 0, 1, 0);
        let scroll = history.scroll.scroll.min(history.scroll_limit(layout.body));
        frame.render_widget(history.paragraph().scroll((scroll, 0)), layout.body);
        pointer::register(
            layout.body,
            None,
            Some((pointer::key(KeyCode::Up), pointer::key(KeyCode::Down))),
        );
        let earlier = self.i18n.text("external-agent-preview-earlier");
        let newer = self.i18n.text("external-agent-preview-newer");
        let mut buttons = Vec::new();
        if history.earlier.is_some() {
            buttons.push((earlier.as_str(), pointer::key(KeyCode::Left)));
        }
        if !history.cursors.is_empty() {
            buttons.push((newer.as_str(), pointer::key(KeyCode::Right)));
        }
        if buttons.is_empty() {
            frame.render_widget(
                Paragraph::new(self.i18n.text("external-agent-preview-scroll")),
                layout.footer,
            );
        } else {
            pointer::render_buttons(frame, layout.footer, &buttons);
        }
    }
}
