use super::*;

impl App {
    pub(super) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let composer_height = self.composer_height();
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(composer_height),
                Constraint::Length(1),
            ])
            .split(area);

        let main = vertical[0];
        let composer = vertical[1];
        let status = vertical[2];

        let sessions_width = adaptive_sessions_width(main.width);
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(sessions_width), Constraint::Min(24)])
            .split(main);

        let transcript = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(4), Constraint::Min(4)])
            .split(horizontal[1]);

        self.layout = LayoutCache {
            transcript_body: inner_rect(transcript[1]),
        };

        self.transcript.clamp_scroll(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
        );

        self.render_sessions(frame, horizontal[0]);
        self.render_transcript_header(frame, transcript[0]);
        self.render_transcript(frame, transcript[1]);
        self.render_composer(frame, composer);
        self.render_status(frame, status);
        self.render_overlay(frame, area);
    }

    fn render_sessions(&mut self, frame: &mut Frame, area: Rect) {
        let title = ui_text::sessions_title(
            &self.i18n,
            self.current_session_view_summary().as_str(),
            self.sessions.search_query.as_str(),
        );
        let current_session_id = self.transcript.session_id;
        let current_parent_id = self.current_parent_session_id();
        let session_depths = session_depth_map(self.sessions.items.as_slice());

        if self.sessions.items.is_empty() && self.sessions.initialized {
            let empty = Paragraph::new(ui_text::t(&self.i18n, "sessions-empty"))
                .block(Block::default().title(title).borders(Borders::ALL))
                .alignment(Alignment::Center);
            frame.render_widget(empty, area);
            return;
        }

        let mut items = self
            .sessions
            .items
            .iter()
            .map(|session| {
                let is_open = self.transcript.session_id == Some(session.id);
                let lineage_relation = self
                    .current_lineage_item(session.id)
                    .map(|item| item.relation);
                let is_current_child =
                    current_session_id.is_some_and(|id| session.parent_id == Some(id));
                let is_current_parent = current_parent_id == Some(session.id);
                let depth = session_depths.get(&session.id).copied().unwrap_or_default();
                let mut title_style = Style::default();
                if is_open {
                    title_style = title_style.fg(Color::Cyan).add_modifier(Modifier::BOLD);
                }

                let mut title_spans = vec![Span::styled(
                    format!(
                        "{}{}",
                        "  ".repeat(depth),
                        if depth == 0 { "◆ " } else { "↳ " }
                    ),
                    Style::default().fg(Color::DarkGray),
                )];
                title_spans.push(Span::styled(session.title.clone(), title_style));
                if is_open {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-current")),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                if is_current_parent {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-parent")),
                        Style::default().fg(Color::Magenta),
                    ));
                } else if matches!(lineage_relation, Some(LineageRelation::Ancestor)) {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-ancestor")),
                        Style::default().fg(Color::Magenta),
                    ));
                }
                if is_current_child || matches!(lineage_relation, Some(LineageRelation::Child)) {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-child")),
                        Style::default().fg(Color::Green),
                    ));
                } else if matches!(lineage_relation, Some(LineageRelation::Sibling)) {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!("[{}]", ui_text::t(&self.i18n, "session-tag-sibling")),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                if session.child_session_count > 0 {
                    title_spans.push(Span::raw(" "));
                    title_spans.push(Span::styled(
                        format!(
                            "[{}]",
                            self.i18n.text_args(
                                "session-summary-children",
                                &crate::fl_args!("count" => session.child_session_count as i64),
                            )
                        ),
                        Style::default().fg(Color::DarkGray),
                    ));
                }

                let meta = ui_text::session_meta(
                    &self.i18n,
                    session.id,
                    session.message_count,
                    session.updated_at,
                );
                let mut meta_parts = vec![meta];
                if let Some(parent_id) = session.parent_id {
                    meta_parts.push(self.i18n.text_args(
                        "session-summary-parent",
                        &crate::fl_args!("id" => parent_id),
                    ));
                }
                ListItem::new(vec![
                    Line::from(title_spans),
                    Line::from(Span::styled(
                        meta_parts.join(" | "),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
            })
            .collect::<Vec<_>>();

        if self.sessions.loading_more {
            items.push(ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "sessions-loading-more"),
                Style::default().fg(Color::DarkGray),
            ))));
        } else if self.sessions.has_more {
            items.push(ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "sessions-more"),
                Style::default().fg(Color::DarkGray),
            ))));
        }

        let list = List::new(items)
            .block(Block::default().title(title).borders(Borders::ALL))
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");

        let mut state = ListState::default();
        state.select(self.sessions.selection_for_render());
        frame.render_stateful_widget(list, area, &mut state);
    }

    fn render_transcript_header(&mut self, frame: &mut Frame, area: Rect) {
        let is_running = self.transcript.execution.as_ref().is_some_and(|execution| {
            execution.run_state != SessionRunState::Idle || execution.blocked
        });
        let title = ui_text::transcript_header_title(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
            is_running,
        );
        let mut top_right = Vec::new();
        if self.transcript.submitting {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-busy"));
        }
        if self.transcript.loading_initial {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-loading"));
        } else if self.transcript.loading_older {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-loading-older"));
        }

        let mut bottom_left = Vec::new();
        if let Some(execution) = self.transcript.execution.as_ref() {
            bottom_left.push(ui_text::session_meta(
                &self.i18n,
                execution.session.id,
                execution.session.message_count,
                execution.session.updated_at,
            ));
            if let Some(parent_id) = execution.session.parent_id {
                bottom_left.push(self.i18n.text_args(
                    "session-summary-parent",
                    &crate::fl_args!("id" => parent_id),
                ));
            }
            if execution.session.child_session_count > 0 {
                bottom_left.push(self.i18n.text_args(
                    "session-summary-children",
                    &crate::fl_args!("count" => execution.session.child_session_count as i64),
                ));
            }
        }
        bottom_left.extend(self.current_lineage_context_parts());
        bottom_left.extend(self.current_execution_context_parts());
        bottom_left.push(self.current_session_view_summary());
        if let Some(summary) = self.run_options.summary() {
            bottom_left.push(summary);
        }

        let mut bottom_right = Vec::new();
        let total_lines = self
            .transcript
            .rendered(self.layout.transcript_body.width.max(1))
            .lines
            .len();
        if total_lines > 0 {
            let first_line = min(self.transcript.scroll.saturating_add(1), total_lines);
            let last_line = min(
                self.transcript
                    .scroll
                    .saturating_add(self.layout.transcript_body.height.max(1) as usize),
                total_lines,
            );
            let percent = ((last_line as f64 / total_lines as f64) * 100.0).round() as u16;
            bottom_right.push(ui_text::transcript_lines_summary(
                &self.i18n,
                first_line,
                last_line,
                total_lines,
                percent,
            ));
        }
        if self.transcript.follow_tail {
            bottom_right.push(ui_text::t(&self.i18n, "transcript-header-tail"));
        }
        if !self.transcript.search_query.trim().is_empty() {
            bottom_right.push(ui_text::transcript_search_summary(
                &self.i18n,
                self.transcript.search_query.as_str(),
                self.transcript.current_search_match_number(),
                self.transcript.current_search_match_count(),
            ));
        }

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(inner);
        self.render_header_row(
            frame,
            rows[0],
            title,
            top_right.join(" | "),
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        );
        self.render_header_row(
            frame,
            rows[1],
            bottom_left.join(" | "),
            bottom_right.join(" | "),
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::DarkGray),
        );
    }

    fn render_transcript(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(ui_text::transcript_panel_title(&self.i18n))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let lines = if self.transcript.session_id.is_none() {
            vec![
                Line::from(ui_text::t(&self.i18n, "no-session-selected")),
                Line::from(ui_text::t(&self.i18n, "no-session-selected-hint")),
            ]
        } else {
            let rendered = self.transcript.rendered(inner.width).clone();
            let matches = rendered.search_matches.clone();
            let active_match = self
                .transcript
                .search_match_index
                .and_then(|index| matches.get(index).copied());
            rendered
                .lines
                .iter()
                .enumerate()
                .map(|(idx, line)| {
                    let line_is_active = active_match == Some(idx);
                    let line_has_match = matches.contains(&idx);
                    highlight_search_line(
                        line.text.as_str(),
                        line.style,
                        self.transcript.search_query.as_str(),
                        line_is_active,
                        line_has_match,
                    )
                })
                .collect::<Vec<_>>()
        };

        let paragraph = Paragraph::new(Text::from(lines))
            .scroll((min(self.transcript.scroll, u16::MAX as usize) as u16, 0))
            .wrap(Wrap { trim: false });
        frame.render_widget(paragraph, inner);
    }

    fn render_composer(&self, frame: &mut Frame, area: Rect) {
        let mut title = ui_text::composer_title(&self.i18n, self.transcript.session_id);
        if let Some(summary) = self.run_options.summary() {
            title = format!("{title}[{summary}] ");
        }
        if !self.queue.is_empty() {
            let preview = self.queue.first_preview(40).unwrap_or_default();
            if preview.is_empty() {
                title = format!("{title}· {} queued ", self.queue.len());
            } else {
                title = format!("{title}· {} queued · {preview} ", self.queue.len());
            }
        }
        if self.transcript.submitting {
            title = format!("{title}· esc to interrupt ");
        }
        let block = Block::default().title(title).borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let view = self.composer.render_view(inner.width, inner.height);
        let content = if self.composer.text().is_empty() {
            Text::from(Line::from(Span::styled(
                ui_text::t(&self.i18n, "composer-placeholder"),
                Style::default().fg(Color::DarkGray),
            )))
        } else {
            Text::from(view.lines.clone())
        };

        frame.render_widget(Paragraph::new(content), inner);

        if self.overlay.is_none() && self.focus == Focus::Composer {
            frame.set_cursor_position((
                inner.x.saturating_add(view.cursor_x),
                inner.y.saturating_add(view.cursor_y),
            ));
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        if let Some(flash) = &self.flash {
            let style = match flash.level {
                FlashLevel::Success => {
                    Style::default().fg(self.theme_color("flash_success", Color::Green))
                }
                FlashLevel::Warning => {
                    Style::default().fg(self.theme_color("flash_warning", Color::Magenta))
                }
                FlashLevel::Error => {
                    Style::default().fg(self.theme_color("flash_error", Color::Red))
                }
                FlashLevel::Info => {
                    Style::default().fg(self.theme_color("flash_info", Color::Cyan))
                }
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(flash.text.clone(), style))),
                area,
            );
            return;
        }

        let base_style = Style::default().fg(self.theme_color("status", Color::DarkGray));
        let mut spans = Vec::new();
        let text = if let Some(text) = self
            .status_line
            .as_ref()
            .and_then(|status_line| status_line.text.clone())
        {
            text
        } else {
            let default_hint = match self.focus {
                Focus::Sessions => ui_text::t(&self.i18n, "status-sessions"),
                Focus::Transcript => ui_text::t(&self.i18n, "status-transcript"),
                Focus::Composer => ui_text::t(&self.i18n, "status-composer"),
            };
            self.status_context_summary()
                .map(|context| format!("{context}  |  {default_hint}"))
                .unwrap_or(default_hint)
        };
        spans.push(Span::styled(text, base_style));

        for segment in self.backend.plugin_statusline_segments() {
            if segment.content.trim().is_empty() {
                continue;
            }
            spans.push(Span::styled("  |  ", base_style));
            let style = segment
                .color
                .as_deref()
                .and_then(parse_tui_color)
                .map(|color| Style::default().fg(color))
                .unwrap_or(base_style);
            spans.push(Span::styled(segment.content, style));
        }

        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn theme_color(&self, key: &str, fallback: Color) -> Color {
        self.plugin_theme
            .as_ref()
            .and_then(|theme| theme.colors.get(key))
            .and_then(|value| parse_tui_color(value))
            .unwrap_or(fallback)
    }

    fn render_header_row(
        &self,
        frame: &mut Frame,
        area: Rect,
        left: String,
        right: String,
        left_style: Style,
        right_style: Style,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        if right.trim().is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(left, left_style))),
                area,
            );
            return;
        }

        let right_width = UnicodeWidthStr::width(right.as_str()).saturating_add(1) as u16;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(min(area.width, right_width)),
            ])
            .split(area);

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(left, left_style))),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(right, right_style)))
                .alignment(Alignment::Right),
            columns[1],
        );
    }

    fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        let Some(overlay) = &self.overlay else {
            return;
        };

        match overlay {
            Overlay::Help => {
                let area = centered_rect(area, 92, 36);
                frame.render_widget(Clear, area);
                let help_lines = ui_text::help_lines(&self.i18n);
                let text = help_lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if index == 0 {
                            Line::from(Span::styled(
                                value,
                                Style::default().add_modifier(Modifier::BOLD),
                            ))
                        } else {
                            Line::from(value)
                        }
                    })
                    .collect::<Vec<_>>();
                let widget = Paragraph::new(Text::from(text))
                    .block(
                        Block::default()
                            .title(format!(" {} ", ui_text::t(&self.i18n, "help-title")))
                            .borders(Borders::ALL),
                    )
                    .wrap(Wrap { trim: false });
                frame.render_widget(widget, area);
            }
            Overlay::SessionSearch(dialog)
            | Overlay::TranscriptSearch(dialog)
            | Overlay::SessionRename(dialog) => {
                self.render_line_overlay(frame, area, dialog);
            }
            Overlay::PermissionRuleEdit(dialog) => {
                self.render_permission_rule_edit_overlay(frame, area, dialog);
            }
            Overlay::FileAttach(dialog) => {
                self.render_file_attach_overlay(frame, area, dialog);
            }
            Overlay::Permission(dialog) => {
                self.render_permission_overlay(frame, area, dialog);
            }
            Overlay::UserInputReply(dialog) => {
                self.render_user_input_overlay(frame, area, dialog);
            }
            Overlay::Confirm(dialog) => {
                self.render_confirm_overlay(frame, area, dialog);
            }
            Overlay::Picker(dialog) => {
                self.render_picker_overlay(frame, area, dialog);
            }
            Overlay::Timeline(dialog) => {
                self.render_timeline_overlay(frame, area, dialog);
            }
            Overlay::PluginInspector(dialog) => {
                self.render_plugin_inspector_overlay(frame, area, dialog);
            }
        }
    }

    fn render_line_overlay(&self, frame: &mut Frame, area: Rect, dialog: &LineInputOverlay) {
        let area = centered_rect(area, 70, 7);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);
        let view = dialog.input.render_view(rows[1].width, 1);
        frame.render_widget(
            Paragraph::new(Text::from(view.lines.clone()))
                .block(Block::default().borders(Borders::BOTTOM)),
            rows[1],
        );
        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-line-footer")),
            rows[2],
        );
        frame.set_cursor_position((
            rows[1].x.saturating_add(view.cursor_x),
            rows[1].y.saturating_add(view.cursor_y),
        ));
    }

    fn render_permission_rule_edit_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionRuleEditOverlay,
    ) {
        let area = centered_rect(area, 82, 11);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(2),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);
        frame.render_widget(
            Paragraph::new(permission_rule_edit_help())
                .block(Block::default().borders(Borders::BOTTOM))
                .wrap(Wrap { trim: false }),
            rows[1],
        );
        let input_view = dialog.input.render_view(rows[2].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::BOTTOM)),
            rows[2],
        );
        frame.render_widget(
            Paragraph::new(render_permission_rule_preview(dialog.input.text()))
                .wrap(Wrap { trim: false }),
            rows[3],
        );
        frame.set_cursor_position((
            rows[2].x.saturating_add(input_view.cursor_x),
            rows[2].y.saturating_add(input_view.cursor_y),
        ));
    }

    fn render_file_attach_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &FileAttachOverlay,
    ) {
        let area = centered_rect(area, 88, 18);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-attach-title")
            ))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-attach-prompt")),
            rows[0],
        );

        let input_view = dialog.input.render_view(rows[1].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );

        let result_items = if dialog.results.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-attach-no-match"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .results
                .iter()
                .map(|path| ListItem::new(path.to_string_lossy().to_string()))
                .collect::<Vec<_>>()
        };
        let list = List::new(result_items)
            .block(Block::default().borders(Borders::ALL).title(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-attach-matches")
            )))
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.results.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, rows[2], &mut state);

        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-attach-footer")),
            rows[3],
        );

        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[1]
                .y
                .saturating_add(1)
                .saturating_add(input_view.cursor_y),
        ));
    }

    fn render_permission_overlay(&self, frame: &mut Frame, area: Rect, dialog: &PermissionOverlay) {
        let area = centered_rect(area, 84, 15);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-permission-title")
            ))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(7),
                Constraint::Length(4),
                Constraint::Length(1),
            ])
            .split(inner);

        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            self.i18n.text_args(
                "overlay-permission-request-id",
                &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
            ),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(permission_action_label(
            &self.i18n,
            &dialog.request.action,
        )));
        lines.push(Line::from(self.i18n.text_args(
            "overlay-permission-reason",
            &crate::fl_args!("reason" => dialog.request.reason.clone()),
        )));
        if !dialog.request.explanation.trim().is_empty() {
            lines.push(Line::from(format!(
                "Explanation: {}",
                dialog.request.explanation
            )));
        }
        let mut facts = Vec::new();
        facts.push(format!(
            "risk={}",
            permission_risk_label(dialog.request.risk)
        ));
        if let Some(source) = dialog.request.source.as_deref() {
            facts.push(format!("source={source}"));
        }
        if let Some(scope) = dialog.request.scope {
            facts.push(format!("scope={scope}"));
        }
        if let Some(operator) = dialog.request.operator.as_deref() {
            facts.push(format!("operator={operator}"));
        }
        if !facts.is_empty() {
            lines.push(Line::from(facts.join(" · ")));
        }
        if let Some(session_id) = dialog.request.session_id {
            lines.push(Line::from(self.i18n.text_args(
                "overlay-permission-session",
                &crate::fl_args!("session" => session_id),
            )));
        }
        append_permission_trace_lines(&mut lines, &dialog.request.trace);

        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            rows[0],
        );

        let choices = permission_overlay_choices(&self.i18n);
        let items = choices
            .iter()
            .map(|label| ListItem::new(label.clone()))
            .collect::<Vec<_>>();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select(Some(dialog.selected));
        frame.render_stateful_widget(list, rows[1], &mut state);

        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-permission-footer")),
            rows[2],
        );
    }

    fn render_user_input_overlay(&self, frame: &mut Frame, area: Rect, dialog: &UserInputOverlay) {
        let height = min(18, area.height.saturating_sub(4));
        let area = centered_rect(area, 84, height);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-user-input-title")
            ))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(inner);

        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            self.i18n.text_args(
                "overlay-user-input-request-id",
                &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
            ),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        for question in &dialog.request.questions {
            lines.push(Line::from(Span::styled(
                format!("{} ({})", question.question, question.id),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for option in &question.options {
                let mut text = format!("  - {}", option.label);
                if !option.description.trim().is_empty() {
                    text.push_str(format!(" | {}", option.description).as_str());
                }
                lines.push(Line::from(text));
            }
            if question.allow_custom {
                lines.push(Line::from(format!(
                    "  - {}",
                    ui_text::t(&self.i18n, "overlay-user-input-custom-allowed")
                )));
            }
            lines.push(Line::from(""));
        }
        lines.push(Line::from(ui_text::t(
            &self.i18n,
            "overlay-user-input-reply-format",
        )));
        lines.push(Line::from(ui_text::t(
            &self.i18n,
            "overlay-user-input-cancel-hint",
        )));

        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            rows[0],
        );

        let view = dialog.input.render_view(rows[1].width, rows[1].height);
        frame.render_widget(
            Paragraph::new(Text::from(view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );
        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-user-input-footer")),
            rows[2],
        );
        frame.set_cursor_position((
            rows[1].x.saturating_add(1).saturating_add(view.cursor_x),
            rows[1].y.saturating_add(1).saturating_add(view.cursor_y),
        ));
    }

    fn render_confirm_overlay(&self, frame: &mut Frame, area: Rect, dialog: &ConfirmOverlay) {
        let body_height = dialog.body_lines.len() as u16;
        let area = centered_rect(area, 76, max(8, body_height.saturating_add(4)));
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(body_height), Constraint::Length(1)])
            .split(inner);

        let body = dialog
            .body_lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(line.clone())
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Text::from(body)).wrap(Wrap { trim: false }),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(dialog.footer.clone()).alignment(Alignment::Right),
            rows[1],
        );
    }

    fn render_picker_overlay(&self, frame: &mut Frame, area: Rect, dialog: &PickerOverlay) {
        let area = centered_rect(area, 88, 18);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(6),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);

        let input_view = dialog.input.render_view(rows[1].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );

        let result_items = if dialog.loading {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-picker-loading"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else if dialog.items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                dialog.empty_message.clone(),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|item| {
                    ListItem::new(vec![
                        Line::from(item.label.clone()),
                        Line::from(Span::styled(
                            item.detail.clone(),
                            Style::default().fg(Color::DarkGray),
                        )),
                    ])
                })
                .collect::<Vec<_>>()
        };

        let list = List::new(result_items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.loading && !dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, rows[2], &mut state);

        frame.render_widget(Paragraph::new(dialog.footer.clone()), rows[3]);
        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[1]
                .y
                .saturating_add(1)
                .saturating_add(input_view.cursor_y),
        ));
    }

    fn render_timeline_overlay(&self, frame: &mut Frame, area: Rect, dialog: &TimelineOverlay) {
        let area = centered_rect(area, 94, 24);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);

        let input_view = dialog.input.render_view(rows[1].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );

        let stacked = should_stack_detail_layout(rows[2].width, 40, 46);
        let stacked_constraints = [Constraint::Percentage(42), Constraint::Percentage(58)];
        let split_constraints = adaptive_detail_split(rows[2].width, 40, 46);
        let content = Layout::default()
            .direction(if stacked {
                Direction::Vertical
            } else {
                Direction::Horizontal
            })
            .constraints(if stacked {
                stacked_constraints.as_ref()
            } else {
                split_constraints.as_ref()
            })
            .split(rows[2]);

        let list_items = if dialog.loading {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-picker-loading"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else if dialog.items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                dialog.empty_message.clone(),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|item| ListItem::new(item.summary.clone()))
                .collect::<Vec<_>>()
        };
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-timeline-events")
                    ))
                    .borders(Borders::ALL),
            )
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.loading && !dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, content[0], &mut state);

        let detail = if dialog.loading {
            ui_text::t(&self.i18n, "overlay-picker-loading")
        } else {
            dialog
                .items
                .get(dialog.selected)
                .map(|item| item.detail.clone())
                .unwrap_or_else(|| dialog.empty_message.clone())
        };
        frame.render_widget(
            Paragraph::new(detail)
                .block(
                    Block::default()
                        .title(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-timeline-detail")
                        ))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            content[1],
        );

        frame.render_widget(Paragraph::new(dialog.footer.clone()), rows[3]);
        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[1]
                .y
                .saturating_add(1)
                .saturating_add(input_view.cursor_y),
        ));
    }

    fn render_plugin_inspector_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PluginInspectorOverlay,
    ) {
        let area = centered_rect(area, 96, 28);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(format!(" {} ", dialog.title))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Min(12),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(Paragraph::new(dialog.prompt.clone()), rows[0]);

        let input_view = dialog.input.render_view(rows[1].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );

        let stacked = should_stack_detail_layout(rows[2].width, 34, 48);
        let (list_area, detail_area, logs_area) = if stacked {
            let content = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(6), Constraint::Min(5), Constraint::Min(5)])
                .split(rows[2]);
            (content[0], content[1], content[2])
        } else {
            let content = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(adaptive_detail_split(rows[2].width, 34, 48))
                .split(rows[2]);
            let right = Layout::default()
                .direction(Direction::Vertical)
                .constraints(adaptive_vertical_split(content[1].height, 7, 9))
                .split(content[1]);
            (content[0], right[0], right[1])
        };

        let list_items = if dialog.items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                dialog.empty_message.clone(),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|item| {
                    let style = match item.state {
                        agena::plugin::status::PluginRunState::Running => {
                            Style::default().fg(Color::Green)
                        }
                        agena::plugin::status::PluginRunState::Restarting => {
                            Style::default().fg(Color::Magenta)
                        }
                        agena::plugin::status::PluginRunState::Failed => {
                            Style::default().fg(Color::Red)
                        }
                        agena::plugin::status::PluginRunState::Stopped => {
                            Style::default().fg(Color::DarkGray)
                        }
                    };
                    ListItem::new(Line::from(Span::styled(item.summary.clone(), style)))
                })
                .collect::<Vec<_>>()
        };
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-plugins-list")
                    ))
                    .borders(Borders::ALL),
            )
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, list_area, &mut state);

        let detail = dialog
            .items
            .get(dialog.selected)
            .map(|item| item.detail.clone())
            .unwrap_or_else(|| dialog.empty_message.clone());
        frame.render_widget(
            Paragraph::new(detail)
                .block(
                    Block::default()
                        .title(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-plugins-detail")
                        ))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            detail_area,
        );

        let logs = dialog
            .items
            .get(dialog.selected)
            .map(|item| item.logs.clone())
            .unwrap_or_else(|| dialog.empty_message.clone());
        frame.render_widget(
            Paragraph::new(logs)
                .block(
                    Block::default()
                        .title(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-plugins-logs")
                        ))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            logs_area,
        );

        frame.render_widget(Paragraph::new(dialog.footer.clone()), rows[3]);
        frame.set_cursor_position((
            rows[1]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[1]
                .y
                .saturating_add(1)
                .saturating_add(input_view.cursor_y),
        ));
    }

    fn composer_height(&self) -> u16 {
        let line_count = max(1, self.composer.logical_line_count());
        min(12, line_count as u16 + 2)
    }
}

pub(super) fn highlight_search_line(
    text: &str,
    base_style: Style,
    query: &str,
    active_match: bool,
    has_match: bool,
) -> Line<'static> {
    let line_style = if active_match {
        base_style
            .fg(Color::Cyan)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else if has_match {
        base_style
            .fg(Color::Cyan)
            .add_modifier(Modifier::UNDERLINED)
    } else {
        base_style
    };

    if !has_match || query.trim().is_empty() {
        return Line::from(Span::styled(text.to_string(), line_style));
    }

    let ranges = find_search_ranges(text, query);
    if ranges.is_empty() {
        return Line::from(Span::styled(text.to_string(), line_style));
    }

    let mut spans = Vec::new();
    let mut cursor = 0;
    for range in ranges {
        if cursor < range.start {
            spans.push(Span::styled(
                text[cursor..range.start].to_string(),
                line_style,
            ));
        }
        let match_style = if active_match {
            line_style.add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            line_style
                .fg(Color::Cyan)
                .add_modifier(Modifier::UNDERLINED)
        };
        spans.push(Span::styled(text[range.clone()].to_string(), match_style));
        cursor = range.end;
    }
    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), line_style));
    }

    Line::from(spans)
}

fn selection_highlight_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

fn parse_tui_color(value: &str) -> Option<Color> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    match lower.as_str() {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "magenta" => Some(Color::Magenta),
        "cyan" => Some(Color::Cyan),
        "gray" | "grey" => Some(Color::Gray),
        "darkgray" | "dark_gray" | "dark-grey" | "darkgrey" => Some(Color::DarkGray),
        "lightred" | "light_red" | "light-red" => Some(Color::LightRed),
        "lightgreen" | "light_green" | "light-green" => Some(Color::LightGreen),
        "lightyellow" | "light_yellow" | "light-yellow" => Some(Color::LightYellow),
        "lightblue" | "light_blue" | "light-blue" => Some(Color::LightBlue),
        "lightmagenta" | "light_magenta" | "light-magenta" => Some(Color::LightMagenta),
        "lightcyan" | "light_cyan" | "light-cyan" => Some(Color::LightCyan),
        "white" => Some(Color::White),
        _ => parse_hex_color(value),
    }
}

fn parse_hex_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(red, green, blue))
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = adaptive_modal_width(area.width, width);
    let height = adaptive_modal_height(area.height, height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn inner_rect(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

pub(super) fn adaptive_modal_width(total_width: u16, target: u16) -> u16 {
    let max_width = total_width.saturating_sub(2);
    if total_width <= 72 {
        max(36, max_width)
    } else if total_width <= 96 {
        min(max_width, max(44, target.saturating_sub(10)))
    } else {
        min(target, max_width)
    }
}

pub(super) fn adaptive_modal_height(total_height: u16, target: u16) -> u16 {
    let max_height = total_height.saturating_sub(2);
    if total_height <= 18 {
        max(6, max_height)
    } else if total_height <= 28 {
        min(max_height, max(8, target.saturating_sub(6)))
    } else {
        min(target, max_height)
    }
}

pub(super) fn adaptive_detail_split(
    total_width: u16,
    left_min: u16,
    right_min: u16,
) -> [Constraint; 2] {
    if should_stack_detail_layout(total_width, left_min, right_min) {
        [Constraint::Percentage(50), Constraint::Percentage(50)]
    } else {
        [Constraint::Min(left_min), Constraint::Min(right_min)]
    }
}

pub(super) fn should_stack_detail_layout(total_width: u16, left_min: u16, right_min: u16) -> bool {
    let available = total_width.saturating_sub(2);
    available < left_min.saturating_add(right_min).saturating_add(8)
}

pub(super) fn adaptive_vertical_split(
    total_height: u16,
    top_min: u16,
    bottom_min: u16,
) -> [Constraint; 2] {
    let available = total_height.saturating_sub(2);
    if available < top_min.saturating_add(bottom_min).saturating_add(3) {
        [Constraint::Percentage(50), Constraint::Percentage(50)]
    } else {
        [Constraint::Min(top_min), Constraint::Min(bottom_min)]
    }
}
