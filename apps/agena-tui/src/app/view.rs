use super::*;

impl App {
    pub(super) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let composer_height = self.composer_height();
        let status_height = self.status_row_height(area.width);
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(8),
                Constraint::Length(composer_height),
                Constraint::Length(status_height),
            ])
            .split(area);

        let main = vertical[0];
        let composer = vertical[1];
        let status = vertical[2];

        let stacked_sessions = should_stack_sessions_layout(main.width);
        let sessions_area;
        let transcript_host_area;
        if stacked_sessions {
            let stacked = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(adaptive_sessions_height(main.height)),
                    Constraint::Min(12),
                ])
                .split(main);
            sessions_area = stacked[0];
            transcript_host_area = stacked[1];
        } else {
            let sessions_width = adaptive_sessions_width(main.width);
            let horizontal = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(sessions_width), Constraint::Min(24)])
                .split(main);
            sessions_area = horizontal[0];
            transcript_host_area = horizontal[1];
        }

        let transcript_layout = transcript_surface_layout(transcript_host_area);
        self.layout = LayoutCache {
            transcript_body: transcript_layout.body,
        };

        self.transcript.clamp_scroll(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
        );

        self.render_sessions(frame, sessions_area, stacked_sessions);
        self.render_transcript_surface(frame, transcript_host_area);
        self.render_composer(frame, composer);
        self.render_status(frame, status);
        self.render_overlay(frame, area);
    }

    fn render_sessions(&mut self, frame: &mut Frame, area: Rect, stacked: bool) {
        let frame_block = Block::default().borders(if stacked {
            Borders::BOTTOM
        } else {
            Borders::RIGHT
        });
        let inner = inset_rect(frame_block.inner(area), 1, 1);
        frame.render_widget(frame_block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let header_height = session_sidebar_header_height(inner.height);
        let footer_height = u16::from(inner.height > header_height.saturating_add(2));
        let mut constraints = vec![Constraint::Length(header_height), Constraint::Min(1)];
        if footer_height > 0 {
            constraints.push(Constraint::Length(footer_height));
        }
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);
        let header_area = sections[0];
        let list_area = sections[1];
        let footer_area = (footer_height > 0).then_some(sections[2]);

        let header_frame = Block::default().borders(Borders::BOTTOM);
        let header_inner = inset_rect(header_frame.inner(header_area), 0, 0);
        frame.render_widget(header_frame, header_area);

        if header_inner.width > 0 && header_inner.height > 0 {
            let mut header_constraints = vec![Constraint::Length(1)];
            if header_inner.height >= 2 {
                header_constraints.push(Constraint::Length(1));
            }
            if header_inner.height >= 3 {
                header_constraints.push(Constraint::Length(1));
            }
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints(header_constraints)
                .split(header_inner);

            let selected_title = self
                .current_or_selected_session_title()
                .unwrap_or_else(|| "new session".to_string());
            let selected_id = self
                .current_or_selected_session_id()
                .map(|id| format!("#{id}"))
                .unwrap_or_else(|| "new".to_string());
            let mut selected_right = vec![selected_id];
            if let Some(session_id) = self.current_or_selected_session_id()
                && self.session_is_busy(session_id)
            {
                selected_right.push(ui_text::t(&self.i18n, "transcript-header-busy"));
            }

            let mut summary_right = vec![format!("{} listed", self.sessions.items.len())];
            if self.sessions.loading {
                summary_right.push(ui_text::t(&self.i18n, "transcript-header-loading"));
            } else if self.sessions.loading_more {
                summary_right.push(ui_text::t(&self.i18n, "sessions-loading-more"));
            }

            self.render_header_row(
                frame,
                rows[0],
                ui_text::t(&self.i18n, "pane-sessions"),
                summary_right.join("  ·  "),
                Style::default().add_modifier(Modifier::BOLD),
                Style::default().fg(Color::DarkGray),
            );
            if rows.len() > 1 {
                self.render_header_row(
                    frame,
                    rows[1],
                    selected_title,
                    selected_right.join("  ·  "),
                    Style::default(),
                    Style::default().fg(Color::DarkGray),
                );
            }

            if rows.len() > 2 {
                let selected_meta = self
                    .current_or_selected_session_summary()
                    .map(|session| {
                        let mut parts = vec![ui_text::session_meta(
                            &self.i18n,
                            session.id,
                            session.message_count,
                            session.updated_at,
                        )];
                        if let Some(parent_id) = session.parent_id {
                            parts.push(self.i18n.text_args(
                                "session-summary-parent",
                                &crate::fl_args!("id" => parent_id),
                            ));
                        }
                        if session.child_session_count > 0 {
                            parts.push(self.i18n.text_args(
                                "session-summary-children",
                                &crate::fl_args!("count" => session.child_session_count as i64),
                            ));
                        }
                        parts.join("  ·  ")
                    })
                    .unwrap_or_else(|| self.current_session_view_summary());

                let mut scope_parts = vec![self.current_session_view_summary()];
                if !self.sessions.search_query.trim().is_empty() {
                    scope_parts.push(format!("find={}", self.sessions.search_query.trim()));
                }
                self.render_header_row(
                    frame,
                    rows[2],
                    selected_meta,
                    scope_parts.join("  ·  "),
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                );
            }
        }

        let current_session_id = self.transcript.session_id;
        let current_parent_id = self.current_parent_session_id();
        let session_depths = session_depth_map(self.sessions.items.as_slice());

        let mut items = self
            .sessions
            .items
            .iter()
            .map(|session| {
                let is_open = self.transcript.session_id == Some(session.id);
                let is_busy = self.session_is_busy(session.id);
                let lineage_relation = self
                    .current_lineage_item(session.id)
                    .map(|item| item.relation);
                let is_current_child =
                    current_session_id.is_some_and(|id| session.parent_id == Some(id));
                let is_current_parent = current_parent_id == Some(session.id);
                let depth = session_depths.get(&session.id).copied().unwrap_or_default();
                let mut title_style = Style::default().add_modifier(Modifier::BOLD);
                if is_open {
                    title_style = title_style.fg(Color::Cyan).add_modifier(Modifier::BOLD);
                } else if is_busy {
                    title_style = title_style.fg(Color::Magenta);
                }

                let marker_style = if is_open {
                    Style::default().fg(Color::Cyan)
                } else if matches!(lineage_relation, Some(LineageRelation::Ancestor)) || is_current_parent {
                    Style::default().fg(Color::Magenta)
                } else if matches!(lineage_relation, Some(LineageRelation::Child)) || is_current_child {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let mut title_spans = vec![Span::styled(
                    format!(
                        "{}{}",
                        "  ".repeat(depth),
                        if depth == 0 { "◆ " } else { "↳ " }
                    ),
                    marker_style,
                )];
                title_spans.push(Span::styled(session.title.clone(), title_style));
                let badge = if is_open {
                    Some((
                        ui_text::t(&self.i18n, "session-tag-current"),
                        Style::default().fg(Color::Cyan),
                    ))
                } else if is_busy {
                    Some((
                        ui_text::t(&self.i18n, "transcript-header-busy"),
                        Style::default().fg(Color::Magenta),
                    ))
                } else if is_current_parent {
                    Some((
                        ui_text::t(&self.i18n, "session-tag-parent"),
                        Style::default().fg(Color::Magenta),
                    ))
                } else if matches!(lineage_relation, Some(LineageRelation::Ancestor)) {
                    Some((
                        ui_text::t(&self.i18n, "session-tag-ancestor"),
                        Style::default().fg(Color::Magenta),
                    ))
                } else if is_current_child || matches!(lineage_relation, Some(LineageRelation::Child)) {
                    Some((
                        ui_text::t(&self.i18n, "session-tag-child"),
                        Style::default().fg(Color::Green),
                    ))
                } else if matches!(lineage_relation, Some(LineageRelation::Sibling)) {
                    Some((
                        ui_text::t(&self.i18n, "session-tag-sibling"),
                        Style::default().fg(Color::DarkGray),
                    ))
                } else {
                    None
                };
                if let Some((label, style)) = badge {
                    title_spans.push(Span::styled("  ", Style::default()));
                    title_spans.push(Span::styled(label, style));
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
                if session.child_session_count > 0 {
                    meta_parts.push(self.i18n.text_args(
                        "session-summary-children",
                        &crate::fl_args!("count" => session.child_session_count as i64),
                    ));
                }
                ListItem::new(vec![
                    Line::from(title_spans),
                    Line::from(Span::styled(
                        meta_parts.join("  ·  "),
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

        if self.sessions.items.is_empty() && self.sessions.initialized {
            frame.render_widget(
                Paragraph::new(ui_text::t(&self.i18n, "sessions-empty")).alignment(Alignment::Center),
                list_area,
            );
        } else {
            let list = List::new(items)
                .highlight_style(selection_highlight_style())
                .highlight_symbol("> ");

            let mut state = ListState::default();
            state.select(self.sessions.selection_for_render());
            frame.render_stateful_widget(list, list_area, &mut state);
        }

        if let Some(footer_area) = footer_area {
            let footer_frame = Block::default().borders(Borders::TOP);
            let footer_inner = inset_rect(footer_frame.inner(footer_area), 0, 0);
            frame.render_widget(footer_frame, footer_area);
            if footer_inner.width > 0 && footer_inner.height > 0 {
                let left = if self.sessions.items.is_empty() {
                    self.current_session_view_summary()
                } else {
                    format!(
                        "{}  ·  {}/{}",
                        self.current_session_view_summary(),
                        self.sessions.selected.saturating_add(1),
                        self.sessions.items.len(),
                    )
                };
                let mut right_parts = Vec::new();
                if !self.sessions.search_query.trim().is_empty() {
                    right_parts.push(format!("find={}", self.sessions.search_query.trim()));
                }
                if self.sessions.has_more {
                    right_parts.push(ui_text::t(&self.i18n, "sessions-more"));
                }
                self.render_header_row(
                    frame,
                    footer_inner,
                    left,
                    right_parts.join("  ·  "),
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                );
            }
        }
    }

    fn render_transcript_surface(&mut self, frame: &mut Frame, area: Rect) {
        let layout = transcript_surface_layout(area);
        let header_frame = Block::default().borders(Borders::BOTTOM);
        let header_inner = inset_rect(header_frame.inner(layout.header), 1, 0);
        frame.render_widget(header_frame, layout.header);

        if header_inner.width > 0 && header_inner.height > 0 {
            let mut constraints = vec![Constraint::Length(1)];
            if header_inner.height >= 2 {
                constraints.push(Constraint::Length(1));
            }
            if header_inner.height >= 3 {
                constraints.push(Constraint::Length(1));
            }
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints(constraints)
                .split(header_inner);

            let top_right = self.transcript_surface_top_right();
            let title = self.transcript_surface_title();
            self.render_header_row(
                frame,
                rows[0],
                title,
                top_right.join("  ·  "),
                Style::default().add_modifier(Modifier::BOLD),
                Style::default().fg(Color::DarkGray),
            );

            if rows.len() > 1 {
                let primary_left = self.transcript_surface_primary_left();
                let primary_right = self.transcript_surface_primary_right();
                self.render_header_row(
                    frame,
                    rows[1],
                    primary_left,
                    primary_right,
                    Style::default().fg(Color::DarkGray),
                    Style::default().fg(Color::DarkGray),
                );
            }
            if rows.len() > 2 {
                let secondary_left = self.transcript_surface_secondary_left();
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        secondary_left,
                        Style::default().fg(Color::DarkGray),
                    ))),
                    rows[2],
                );
            }
        }

        if layout.body.width == 0 || layout.body.height == 0 {
            return;
        }

        let lines = if self.transcript.session_id.is_none() {
            vec![
                Line::from(ui_text::t(&self.i18n, "no-session-selected")),
                Line::from(ui_text::t(&self.i18n, "no-session-selected-hint")),
            ]
        } else {
            let rendered = self.transcript.rendered(layout.body.width).clone();
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
        frame.render_widget(paragraph, layout.body);
    }

    fn transcript_surface_top_right(&self) -> Vec<String> {
        let is_running = self.transcript.execution.as_ref().is_some_and(|execution| {
            execution.run_state != SessionRunState::Idle || execution.blocked
        });
        let mut top_right = Vec::new();
        if is_running && !self.transcript.submitting {
            top_right.push(ui_text::t(&self.i18n, "session-running"));
        }
        if self.transcript.submitting {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-busy"));
        }
        if self.transcript.loading_initial {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-loading"));
        } else if self.transcript.loading_older {
            top_right.push(ui_text::t(&self.i18n, "transcript-header-loading-older"));
        }
        top_right
    }

    fn transcript_surface_title(&self) -> String {
        let is_running = self.transcript.execution.as_ref().is_some_and(|execution| {
            execution.run_state != SessionRunState::Idle || execution.blocked
        });
        ui_text::transcript_header_title(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
            is_running,
        )
    }

    fn transcript_surface_primary_left(&self) -> String {
        let mut parts = Vec::new();
        if let Some(execution) = self.transcript.execution.as_ref() {
            parts.push(ui_text::session_meta(
                &self.i18n,
                execution.session.id,
                execution.session.message_count,
                execution.session.updated_at,
            ));
            if let Some(parent_id) = execution.session.parent_id {
                parts.push(self.i18n.text_args(
                    "session-summary-parent",
                    &crate::fl_args!("id" => parent_id),
                ));
            }
            if execution.session.child_session_count > 0 {
                parts.push(self.i18n.text_args(
                    "session-summary-children",
                    &crate::fl_args!("count" => execution.session.child_session_count as i64),
                ));
            }
        }
        if parts.is_empty() {
            if let Some(session_id) = self.current_or_selected_session_id() {
                parts.push(format!("#{session_id}"));
            }
            parts.push(self.current_session_view_summary());
        }
        parts.join("  ·  ")
    }

    fn transcript_surface_primary_right(&mut self) -> String {
        let mut parts = Vec::new();
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
            parts.push(ui_text::transcript_lines_summary(
                &self.i18n,
                first_line,
                last_line,
                total_lines,
                percent,
            ));
        }
        if self.transcript.follow_tail {
            parts.push(ui_text::t(&self.i18n, "transcript-header-tail"));
        }
        if !self.transcript.search_query.trim().is_empty() {
            parts.push(ui_text::transcript_search_summary(
                &self.i18n,
                self.transcript.search_query.as_str(),
                self.transcript.current_search_match_number(),
                self.transcript.current_search_match_count(),
            ));
        }
        parts.join("  ·  ")
    }

    fn transcript_surface_secondary_left(&self) -> String {
        let mut parts = self.current_lineage_context_parts();
        parts.extend(
            self.current_execution_context_parts()
                .into_iter()
                .filter(|part| !part.starts_with("cwd=")),
        );
        parts.push(self.current_session_view_summary());
        if let Some(summary) = self.run_options.summary() {
            parts.push(summary);
        }
        parts.join("  ·  ")
    }

    fn render_composer(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(self.composer_panel_title())
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let item_count = self.composer_items.len();
        let item_rows = u16::from(item_count > 0);
        let header_rows = 1_u16;
        let footer_rows = if inner.height >= 4 { 1 } else { 0 };
        let editor_rows = inner
            .height
            .saturating_sub(header_rows)
            .saturating_sub(item_rows)
            .saturating_sub(footer_rows)
            .max(1);

        let mut constraints = vec![Constraint::Length(header_rows)];
        if item_rows > 0 {
            constraints.push(Constraint::Length(item_rows));
        }
        constraints.push(Constraint::Length(editor_rows));
        if footer_rows > 0 {
            constraints.push(Constraint::Length(footer_rows));
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let context_row = rows[0];
        let mut next_row = 1;
        let item_row = if item_rows > 0 {
            let row = Some(rows[next_row]);
            next_row += 1;
            row
        } else {
            None
        };
        let editor_row = rows[next_row];
        let footer_row = if footer_rows > 0 {
            Some(rows[next_row + 1])
        } else {
            None
        };

        self.render_composer_context_row(frame, context_row);
        if let Some(item_row) = item_row {
            self.render_composer_items_row(frame, item_row);
        }

        let editor_width = editor_row.width.saturating_sub(2);
        let editor_x = editor_row.x.saturating_add(1);
        let editor_view = self
            .composer
            .render_view(editor_width.max(1), editor_row.height.max(1));

        frame.render_widget(
            Paragraph::new(Text::from(editor_view.lines.clone())).alignment(Alignment::Left),
            Rect {
                x: editor_x,
                y: editor_row.y,
                width: editor_width.max(1),
                height: editor_row.height,
            },
        );

        if self.composer.text().is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    ui_text::t(&self.i18n, "composer-placeholder"),
                    Style::default().fg(Color::DarkGray),
                ))),
                Rect {
                    x: editor_x,
                    y: editor_row.y,
                    width: editor_width.max(1),
                    height: 1,
                },
            );
        }

        if let Some(footer_row) = footer_row {
            self.render_composer_footer_row(frame, footer_row);
        }

        if self.overlay.is_none() && self.focus == Focus::Composer {
            frame.set_cursor_position((
                editor_x.saturating_add(editor_view.cursor_x),
                editor_row.y.saturating_add(editor_view.cursor_y),
            ));
        }
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let lines = self.status_lines(area.width);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            area,
        );
    }

    fn composer_panel_title(&self) -> String {
        let mut title = ui_text::composer_title(&self.i18n, self.transcript.session_id);
        if self.transcript.submitting {
            title.push_str("running");
        } else {
            title.push_str(self.focus.label());
        }
        title.push(' ');
        title
    }

    fn render_composer_context_row(&self, frame: &mut Frame, area: Rect) {
        let left = self
            .status_context_summary()
            .unwrap_or_else(|| self.default_status_hint());
        let right = self.composer_context_right();
        self.render_header_row(
            frame,
            area,
            left,
            right,
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::DarkGray),
        );
    }

    fn render_composer_items_row(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut spans = Vec::new();
        for (index, item) in self.composer_items.iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
            }
            spans.push(Span::styled(
                format!("[{}]", item.short_label()),
                self.composer_item_style(item),
            ));
        }

        frame.render_widget(
            Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_composer_footer_row(&self, frame: &mut Frame, area: Rect) {
        let lines = self.composer_footer_lines(area.width);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            area,
        );
    }

    fn composer_footer_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if let Some(flash) = &self.flash {
            lines.extend(self.wrap_styled_text(
                flash.text.as_str(),
                width,
                self.flash_style(flash.level),
            ));
            return lines;
        }

        let left = self.composer_primary_footer_text();
        let right = self.composer_secondary_footer_text();
        let combined = if right.is_empty() {
            left
        } else if width >= 88 {
            format!("{left}  |  {right}")
        } else {
            format!("{left}\n{right}")
        };
        lines.extend(self.wrap_styled_text(
            combined.as_str(),
            width,
            Style::default().fg(Color::DarkGray),
        ));
        lines
    }

    fn composer_primary_footer_text(&self) -> String {
        let mut parts = Vec::new();
        if self.transcript.submitting {
            parts.push("esc interrupt".to_string());
            if !self.queue.is_empty() {
                parts.push(format!("tab queue [{}]", self.queue.len()));
            }
        } else {
            parts.push("enter send".to_string());
            parts.push("tab focus".to_string());
            parts.push("/ search".to_string());
        }
        if self.focus == Focus::Composer {
            parts.push("ctrl+f transcript-find".to_string());
        }
        parts.join("  ·  ")
    }

    fn composer_secondary_footer_text(&self) -> String {
        let mut parts = Vec::new();
        if !self.queue.is_empty() {
            let preview = self.queue.first_preview(28).unwrap_or_default();
            if preview.is_empty() {
                parts.push(format!("queued {}", self.queue.len()));
            } else {
                parts.push(format!("queued {} {}", self.queue.len(), preview));
            }
        }
        if let Some(summary) = self.run_options.summary() {
            parts.push(summary);
        }
        if let Some(status_line) = self
            .status_line
            .as_ref()
            .and_then(|status_line| status_line.text.as_ref())
            .map(String::as_str)
        {
            if !status_line.trim().is_empty() {
                parts.push(status_line.trim().to_string());
            }
        }
        parts.join("  |  ")
    }

    fn composer_context_right(&self) -> String {
        let mut parts = Vec::new();
        if let Some(session_id) = self.transcript.session_id {
            parts.push(format!("#{session_id}"));
        } else {
            parts.push("new session".to_string());
        }
        if self.transcript.submitting {
            parts.push(ui_text::t(&self.i18n, "transcript-header-busy"));
        } else {
            parts.push(match self.focus {
                Focus::Sessions => ui_text::t(&self.i18n, "status-sessions"),
                Focus::Transcript => ui_text::t(&self.i18n, "status-transcript"),
                Focus::Composer => ui_text::t(&self.i18n, "status-composer"),
            });
        }
        parts.join("  |  ")
    }

    fn default_status_hint(&self) -> String {
        match self.focus {
            Focus::Sessions => ui_text::t(&self.i18n, "status-sessions"),
            Focus::Transcript => ui_text::t(&self.i18n, "status-transcript"),
            Focus::Composer => ui_text::t(&self.i18n, "status-composer"),
        }
    }

    fn status_lines(&self, width: u16) -> Vec<Line<'static>> {
        let style = Style::default().fg(self.theme_color("status", Color::DarkGray));
        if let Some(flash) = &self.flash {
            return self.wrap_styled_text(
                flash.text.as_str(),
                width,
                self.flash_style(flash.level),
            );
        }

        let mut segments = Vec::new();
        if let Some(text) = self
            .status_line
            .as_ref()
            .and_then(|status_line| status_line.text.clone())
        {
            if !text.trim().is_empty() {
                segments.push(text);
            }
        } else if let Some(context) = self.status_context_summary() {
            segments.push(context);
        } else {
            segments.push(self.default_status_hint());
        }

        for segment in self.backend.plugin_statusline_segments() {
            if segment.content.trim().is_empty() {
                continue;
            }
            segments.push(segment.content.clone());
        }

        self.wrap_styled_text(segments.join("  |  ").as_str(), width, style)
    }

    fn status_row_height(&self, width: u16) -> u16 {
        self.status_lines(width).len().max(1) as u16
    }

    fn composer_item_style(&self, item: &ComposerItem) -> Style {
        match item {
            ComposerItem::Attachment(_) => Style::default()
                .fg(self.theme_color("flash_info", Color::Cyan))
                .add_modifier(Modifier::BOLD),
            ComposerItem::LargePaste(_) => Style::default()
                .fg(self.theme_color("flash_warning", Color::Magenta))
                .add_modifier(Modifier::BOLD),
        }
    }

    fn flash_style(&self, level: FlashLevel) -> Style {
        match level {
            FlashLevel::Success => {
                Style::default().fg(self.theme_color("flash_success", Color::Green))
            }
            FlashLevel::Warning => {
                Style::default().fg(self.theme_color("flash_warning", Color::Magenta))
            }
            FlashLevel::Error => Style::default().fg(self.theme_color("flash_error", Color::Red)),
            FlashLevel::Info => Style::default().fg(self.theme_color("flash_info", Color::Cyan)),
        }
    }

    fn wrap_styled_text(&self, text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
        let sanitized = sanitize_terminal_text(text);
        let available = width.max(1) as usize;
        let options = textwrap::Options::new(available)
            .break_words(false)
            .word_splitter(textwrap::WordSplitter::NoHyphenation);

        sanitized
            .split('\n')
            .flat_map(|line| {
                let wrapped = textwrap::wrap(line, options.clone());
                if wrapped.is_empty() {
                    vec![Line::from(Span::styled(String::new(), style))]
                } else {
                    wrapped
                        .into_iter()
                        .map(|segment| Line::from(Span::styled(segment.into_owned(), style)))
                        .collect::<Vec<_>>()
                }
            })
            .collect()
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
        let item_rows = u16::from(!self.composer_items.is_empty());
        let chrome_rows = 2_u16 + item_rows;
        min(14, line_count as u16 + chrome_rows)
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

#[derive(Debug, Clone, Copy)]
struct TranscriptSurfaceLayout {
    header: Rect,
    body: Rect,
}

fn inset_rect(area: Rect, horizontal: u16, vertical: u16) -> Rect {
    Rect {
        x: area.x.saturating_add(horizontal),
        y: area.y.saturating_add(vertical),
        width: area.width.saturating_sub(horizontal.saturating_mul(2)),
        height: area.height.saturating_sub(vertical.saturating_mul(2)),
    }
}

pub(super) fn transcript_surface_header_height(total_height: u16) -> u16 {
    if total_height <= 10 {
        2
    } else if total_height <= 18 {
        3
    } else {
        4
    }
}

fn transcript_surface_layout(area: Rect) -> TranscriptSurfaceLayout {
    let header_height = min(
        transcript_surface_header_height(area.height),
        area.height.saturating_sub(1),
    );
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_height), Constraint::Min(1)])
        .split(area);

    TranscriptSurfaceLayout {
        header: split[0],
        body: inset_rect(split[1], 1, 0),
    }
}

pub(super) fn session_sidebar_header_height(total_height: u16) -> u16 {
    if total_height <= 4 {
        2
    } else if total_height <= 8 {
        3
    } else {
        4
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

pub(super) fn should_stack_sessions_layout(total_width: u16) -> bool {
    total_width < 92
}

pub(super) fn adaptive_sessions_height(total_height: u16) -> u16 {
    min(10, max(6, total_height.saturating_div(3)))
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
