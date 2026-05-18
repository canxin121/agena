use super::*;

impl App {
    pub(super) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let composer_height = self.composer_height();
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(composer_height)])
            .split(area);

        let main = vertical[0];
        let composer = vertical[1];

        let stacked_sessions = should_stack_sessions_layout(main.width);
        let sessions_area;
        let transcript_host_area;
        if stacked_sessions {
            let stacked = Layout::default()
                .direction(Direction::Vertical)
                .constraints(adaptive_sessions_split(main.height))
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
        self.render_overlay(frame, area);
    }

    fn render_sessions(&mut self, frame: &mut Frame, area: Rect, stacked: bool) {
        let frame_block = Block::default().borders(if stacked {
            Borders::BOTTOM
        } else {
            Borders::RIGHT
        });
        let horizontal_padding = u16::from(area.width > 24);
        let vertical_padding = u16::from(area.height > 6);
        let inner = inset_rect(
            frame_block.inner(area),
            horizontal_padding,
            vertical_padding,
        );
        frame.render_widget(frame_block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let header_height = session_sidebar_header_height(inner.height);
        let constraints = vec![Constraint::Length(header_height), Constraint::Min(1)];
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);
        let header_area = sections[0];
        let list_area = sections[1];

        let header_frame = Block::default().borders(Borders::BOTTOM);
        let header_inner = inset_rect(header_frame.inner(header_area), 0, 0);
        frame.render_widget(header_frame, header_area);

        if header_inner.width > 0 && header_inner.height > 0 {
            let header_constraints = vec![Constraint::Length(1)];
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints(header_constraints)
                .split(header_inner);

            let mut summary_right = vec![
                self.workspace_context_label(),
                self.sessions.items.len().to_string(),
            ];
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
                } else if matches!(lineage_relation, Some(LineageRelation::Ancestor))
                    || is_current_parent
                {
                    Style::default().fg(Color::Magenta)
                } else if matches!(lineage_relation, Some(LineageRelation::Child))
                    || is_current_child
                {
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
                title_spans.push(Span::styled(
                    sanitize_display_text(session.title.as_str()),
                    title_style,
                ));
                if is_busy {
                    title_spans.push(Span::styled(" *", Style::default().fg(Color::Magenta)));
                }

                ListItem::new(Line::from(title_spans))
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
                Paragraph::new(ui_text::t(&self.i18n, "sessions-empty"))
                    .alignment(Alignment::Center),
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
    }

    fn render_transcript_surface(&mut self, frame: &mut Frame, area: Rect) {
        let layout = transcript_surface_layout(area);
        let header_frame = Block::default().borders(Borders::BOTTOM);
        let header_inner = inset_rect(header_frame.inner(layout.header), 1, 0);
        frame.render_widget(header_frame, layout.header);

        if header_inner.width > 0 && header_inner.height > 0 {
            let constraints = vec![Constraint::Length(1)];
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
        }

        if layout.body.width == 0 || layout.body.height == 0 {
            return;
        }

        let lines = if self.transcript.session_id.is_none() {
            vec![Line::from(ui_text::t(&self.i18n, "no-session-selected"))]
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
        if !self.transcript.search_query.trim().is_empty() {
            top_right.push(ui_text::transcript_search_summary(
                &self.i18n,
                self.transcript.search_query.as_str(),
                self.transcript.current_search_match_number(),
                self.transcript.current_search_match_count(),
            ));
        }
        top_right
    }

    fn transcript_surface_title(&self) -> String {
        let is_running = self.transcript.execution.as_ref().is_some_and(|execution| {
            execution.run_state != SessionRunState::Idle || execution.blocked
        });
        let title = ui_text::transcript_header_title(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
            is_running,
        );
        if self.transcript.session_id.is_some() {
            title
        } else {
            format!(
                " {} · {} ",
                ui_text::t(&self.i18n, "pane-transcript"),
                self.workspace_context_label()
            )
        }
    }

    fn render_composer(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(sanitize_display_text(self.composer_panel_title()))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let item_count = self.composer_items.len();
        let item_rows = u16::from(item_count > 0);
        let footer_rows = u16::from(self.should_render_composer_footer() && inner.height >= 2);
        let suggestion_rows = min(
            self.slash_command_suggestion_rows(),
            inner
                .height
                .saturating_sub(item_rows)
                .saturating_sub(footer_rows)
                .saturating_sub(1),
        );
        let editor_rows = inner
            .height
            .saturating_sub(item_rows)
            .saturating_sub(suggestion_rows)
            .saturating_sub(footer_rows)
            .max(1);

        let mut constraints = Vec::new();
        if item_rows > 0 {
            constraints.push(Constraint::Length(item_rows));
        }
        if suggestion_rows > 0 {
            constraints.push(Constraint::Length(suggestion_rows));
        }
        constraints.push(Constraint::Length(editor_rows));
        if footer_rows > 0 {
            constraints.push(Constraint::Length(footer_rows));
        }
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut next_row = 0;
        let item_row = if item_rows > 0 {
            let row = Some(rows[next_row]);
            next_row += 1;
            row
        } else {
            None
        };
        let suggestion_row = if suggestion_rows > 0 {
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

        if let Some(item_row) = item_row {
            self.render_composer_items_row(frame, item_row);
        }
        if let Some(suggestion_row) = suggestion_row {
            self.render_slash_command_suggestions(frame, suggestion_row);
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

    fn render_slash_command_suggestions(&self, frame: &mut Frame, area: Rect) {
        let Some(state) = self.slash_command_suggestions.as_ref() else {
            return;
        };
        if area.width == 0 || area.height == 0 || state.items.is_empty() {
            return;
        }

        let selected = min(state.selected, state.items.len().saturating_sub(1));
        let visible_rows = min(
            area.height as usize,
            min(MAX_SLASH_COMMAND_SUGGESTIONS, state.items.len()),
        );
        let start = selected.saturating_add(1).saturating_sub(visible_rows);
        let width = area.width as usize;
        let lines = state
            .items
            .iter()
            .enumerate()
            .skip(start)
            .take(visible_rows)
            .map(|(index, item)| {
                let is_selected = index == selected;
                let base_style = if is_selected {
                    selection_highlight_style()
                } else {
                    Style::default()
                };
                let name_style = if is_selected {
                    base_style
                } else {
                    Style::default()
                        .fg(self.theme_color("accent", Color::Cyan))
                        .add_modifier(Modifier::BOLD)
                };
                let detail_style = if is_selected {
                    base_style
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let marker = if is_selected { "> " } else { "  " };
                let marker_width = UnicodeWidthStr::width(marker);
                let name_max = min(24, width.saturating_sub(marker_width));
                let name = truncate_display_text(
                    sanitize_display_text(item.label.as_str()).as_str(),
                    name_max,
                );
                let name_width = UnicodeWidthStr::width(name.as_str());
                let detail_max = width
                    .saturating_sub(marker_width)
                    .saturating_sub(name_width)
                    .saturating_sub(2);

                let mut spans = vec![
                    Span::styled(marker, base_style),
                    Span::styled(name, name_style),
                ];
                let mut used_width = marker_width.saturating_add(name_width);
                if detail_max > 0 {
                    let detail = truncate_display_text(
                        sanitize_display_text(item.detail.as_str()).as_str(),
                        detail_max,
                    );
                    if !detail.is_empty() {
                        let detail_width = UnicodeWidthStr::width(detail.as_str());
                        spans.push(Span::styled("  ", base_style));
                        spans.push(Span::styled(detail, detail_style));
                        used_width = used_width.saturating_add(2).saturating_add(detail_width);
                    }
                }
                if is_selected && used_width < width {
                    spans.push(Span::styled(" ".repeat(width - used_width), base_style));
                }

                Line::from(spans)
            })
            .collect::<Vec<_>>();

        frame.render_widget(Paragraph::new(Text::from(lines)), area);
    }

    fn composer_panel_title(&self) -> String {
        let mut title = ui_text::composer_title(&self.i18n, self.transcript.session_id);
        if self.transcript.submitting {
            title.push_str(ui_text::t(&self.i18n, "transcript-header-busy").as_str());
        }
        title.push(' ');
        title
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

        let combined = self.composer_footer_text();
        lines.extend(self.wrap_styled_text(
            combined.as_str(),
            width,
            Style::default().fg(Color::DarkGray),
        ));
        lines
    }

    fn composer_footer_text(&self) -> String {
        let mut parts = Vec::new();
        if self.transcript.submitting {
            parts.push("Esc cancel".to_string());
        }
        if !self.queue.is_empty() {
            let preview = self.queue.first_preview(28).unwrap_or_default();
            if preview.is_empty() {
                parts.push(format!("queue {}", self.queue.len()));
            } else {
                parts.push(format!("queue {} {}", self.queue.len(), preview));
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
        for segment in self.backend.plugin_statusline_segments() {
            if segment.content.trim().is_empty() {
                continue;
            }
            parts.push(segment.content.clone());
        }

        parts.join("  |  ")
    }

    fn should_render_composer_footer(&self) -> bool {
        self.flash.is_some()
            || self.transcript.submitting
            || !self.queue.is_empty()
            || self.run_options.summary().is_some()
            || self
                .status_line
                .as_ref()
                .and_then(|status_line| status_line.text.as_ref())
                .is_some_and(|text| !text.trim().is_empty())
            || self
                .backend
                .plugin_statusline_segments()
                .iter()
                .any(|segment| !segment.content.trim().is_empty())
    }

    fn slash_command_suggestion_rows(&self) -> u16 {
        if self.overlay.is_some() || self.focus != Focus::Composer {
            return 0;
        }
        self.slash_command_suggestions
            .as_ref()
            .map(|state| min(MAX_SLASH_COMMAND_SUGGESTIONS, state.items.len()) as u16)
            .unwrap_or(0)
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

        let left = sanitize_display_text(left);
        let right = sanitize_display_text(right);

        if right.trim().is_empty() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    truncate_display_text(left.as_str(), area.width as usize),
                    left_style,
                ))),
                area,
            );
            return;
        }

        let max_right_width = if area.width < 52 {
            area.width.saturating_div(2).max(8)
        } else {
            area.width.saturating_mul(2).saturating_div(5).max(16)
        };
        let truncated_right = truncate_display_text(right.as_str(), max_right_width as usize);
        let right_width = UnicodeWidthStr::width(truncated_right.as_str()).saturating_add(1) as u16;
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(min(area.width, right_width)),
            ])
            .split(area);
        let truncated_left = truncate_display_text(
            left.as_str(),
            columns[0].width.saturating_sub(1).max(1) as usize,
        );

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(truncated_left, left_style))),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(truncated_right, right_style)))
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
                let area = centered_rect(area, 72, 8);
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
                            .title(sanitize_display_text(format!(
                                " {} ",
                                ui_text::t(&self.i18n, "help-title")
                            )))
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
            Overlay::ProviderStudio(dialog) => {
                self.render_provider_studio_overlay(frame, area, dialog);
            }
        }
    }

    fn render_line_overlay(&self, frame: &mut Frame, area: Rect, dialog: &LineInputOverlay) {
        let area = centered_rect(area, 70, 7);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
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

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.prompt.as_str())),
            rows[0],
        );
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
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
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

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.prompt.as_str())),
            rows[0],
        );
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
            .title(sanitize_display_text(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-attach-title")
            )))
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
                .map(|path| ListItem::new(sanitize_display_text(path.to_string_lossy().as_ref())))
                .collect::<Vec<_>>()
        };
        let list = List::new(result_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(sanitize_display_text(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-attach-matches")
                    ))),
            )
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
            .title(sanitize_display_text(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-permission-title")
            )))
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
            sanitize_display_text(self.i18n.text_args(
                "overlay-permission-request-id",
                &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
            )),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(permission_action_label(
            &self.i18n,
            &dialog.request.action,
        )));
        lines.push(Line::from(sanitize_display_text(self.i18n.text_args(
            "overlay-permission-reason",
            &crate::fl_args!("reason" => sanitize_display_text(dialog.request.reason.as_str())),
        ))));
        if !dialog.request.explanation.trim().is_empty() {
            lines.push(Line::from(format!(
                "Explanation: {}",
                sanitize_display_text(dialog.request.explanation.as_str())
            )));
        }
        let mut facts = Vec::new();
        facts.push(format!(
            "risk={}",
            permission_risk_label(dialog.request.risk)
        ));
        if let Some(source) = dialog.request.source.as_deref() {
            facts.push(format!("source={}", sanitize_display_text(source)));
        }
        if let Some(scope) = dialog.request.scope {
            facts.push(format!("scope={scope}"));
        }
        if let Some(operator) = dialog.request.operator.as_deref() {
            facts.push(format!("operator={}", sanitize_display_text(operator)));
        }
        if !facts.is_empty() {
            lines.push(Line::from(facts.join(" · ")));
        }
        if let Some(session_id) = dialog.request.session_id {
            lines.push(Line::from(sanitize_display_text(self.i18n.text_args(
                "overlay-permission-session",
                &crate::fl_args!("session" => session_id),
            ))));
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
            .title(sanitize_display_text(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-user-input-title")
            )))
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
            sanitize_display_text(self.i18n.text_args(
                "overlay-user-input-request-id",
                &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
            )),
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        for question in &dialog.request.questions {
            lines.push(Line::from(Span::styled(
                sanitize_display_text(format!("{} ({})", question.question, question.id)),
                Style::default().add_modifier(Modifier::BOLD),
            )));
            for option in &question.options {
                let mut text = format!("  - {}", sanitize_display_text(option.label.as_str()));
                if !option.description.trim().is_empty() {
                    text.push_str(
                        format!(" | {}", sanitize_display_text(option.description.as_str()))
                            .as_str(),
                    );
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
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
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
                        sanitize_display_text(line.as_str()),
                        Style::default().add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(sanitize_display_text(line.as_str()))
                }
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Text::from(body)).wrap(Wrap { trim: false }),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str()))
                .alignment(Alignment::Right),
            rows[1],
        );
    }

    fn render_picker_overlay(&self, frame: &mut Frame, area: Rect, dialog: &PickerOverlay) {
        let area = centered_rect(area, 88, 18);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
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
            Paragraph::new(sanitize_display_text(dialog.prompt.as_str())),
            rows[0],
        );

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
                sanitize_display_text(dialog.empty_message.as_str()),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|item| {
                    ListItem::new(vec![
                        Line::from(sanitize_display_text(item.label.as_str())),
                        Line::from(Span::styled(
                            sanitize_display_text(item.detail.as_str()),
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

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
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

    fn render_timeline_overlay(&self, frame: &mut Frame, area: Rect, dialog: &TimelineOverlay) {
        let area = centered_rect(area, 94, 24);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
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

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.prompt.as_str())),
            rows[0],
        );

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
                sanitize_display_text(dialog.empty_message.as_str()),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|item| ListItem::new(sanitize_display_text(item.summary.as_str())))
                .collect::<Vec<_>>()
        };
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-timeline-events")
                    )))
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
                .map(|item| sanitize_display_text(item.detail.as_str()))
                .unwrap_or_else(|| sanitize_display_text(dialog.empty_message.as_str()))
        };
        frame.render_widget(
            Paragraph::new(detail)
                .block(
                    Block::default()
                        .title(sanitize_display_text(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-timeline-detail")
                        )))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            content[1],
        );

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
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

    fn render_plugin_inspector_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PluginInspectorOverlay,
    ) {
        let area = centered_rect(area, 96, 28);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
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

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.prompt.as_str())),
            rows[0],
        );

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
                sanitize_display_text(dialog.empty_message.as_str()),
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
                    ListItem::new(Line::from(Span::styled(
                        sanitize_display_text(item.summary.as_str()),
                        style,
                    )))
                })
                .collect::<Vec<_>>()
        };
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-plugins-list")
                    )))
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
            .map(|item| sanitize_display_text(item.detail.as_str()))
            .unwrap_or_else(|| sanitize_display_text(dialog.empty_message.as_str()));
        frame.render_widget(
            Paragraph::new(detail)
                .block(
                    Block::default()
                        .title(sanitize_display_text(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-plugins-detail")
                        )))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            detail_area,
        );

        let logs = dialog
            .items
            .get(dialog.selected)
            .map(|item| sanitize_display_text(item.logs.as_str()))
            .unwrap_or_else(|| sanitize_display_text(dialog.empty_message.as_str()));
        frame.render_widget(
            Paragraph::new(logs)
                .block(
                    Block::default()
                        .title(sanitize_display_text(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-plugins-logs")
                        )))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            logs_area,
        );

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
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

    fn render_provider_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &ProviderStudioOverlay,
    ) {
        let area = centered_rect(area, 112, 34);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(18),
                Constraint::Length(1),
            ])
            .split(inner);

        let summary = format!(
            "draft {}  ·  auth {}  ·  adapters {}  ·  catalog {}  ·  {}{}",
            if dialog.draft.provider_id.trim().is_empty() {
                "<new>"
            } else {
                dialog.draft.provider_id.trim()
            },
            dialog.draft.auth_kind.label(),
            dialog.selected_adapter_ids.len(),
            dialog.catalog_total,
            if dialog.listing_adapter_models {
                "listing "
            } else {
                ""
            },
            if dialog.saving { "saving" } else { "" }
        );
        self.render_header_row(
            frame,
            rows[0],
            ui_text::t(&self.i18n, "overlay-provider-studio-header"),
            summary,
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        );

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(54)])
            .split(rows[1]);
        let providers_area = content[0];
        let right_area = content[1];
        let right_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Min(8),
            ])
            .split(right_area);

        let provider_items = dialog
            .providers
            .iter()
            .map(|row| {
                ListItem::new(vec![
                    Line::from(sanitize_display_text(row.label.as_str())),
                    Line::from(Span::styled(
                        sanitize_display_text(row.detail.as_str()),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
            })
            .collect::<Vec<_>>();
        let provider_list = List::new(provider_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-provider-studio-providers")
                    )))
                    .borders(Borders::ALL),
            )
            .highlight_style(if dialog.focus == ProviderStudioFocus::Providers {
                selection_highlight_style()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol(">> ");
        let mut provider_state = ListState::default();
        provider_state.select((!dialog.providers.is_empty()).then_some(dialog.selected_provider));
        frame.render_stateful_widget(provider_list, providers_area, &mut provider_state);

        let draft_lines = ProviderStudioField::ALL
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let value = provider_studio_field_value(&dialog.draft, *field);
                let display = match field {
                    ProviderStudioField::ApiKey if !value.trim().is_empty() => {
                        "********".to_owned()
                    }
                    _ if value.trim().is_empty() => "unset".to_owned(),
                    _ => value,
                };
                let label_style = if dialog.focus == ProviderStudioFocus::Fields
                    && dialog.selected_field == index
                {
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else if provider_studio_field_editable(dialog, *field) {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                Line::from(vec![
                    Span::styled(
                        format!("{:>16}", provider_studio_field_label(*field)),
                        label_style,
                    ),
                    Span::raw("  "),
                    Span::styled(
                        sanitize_display_text(display),
                        if provider_studio_field_editable(dialog, *field) {
                            Style::default()
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let mut draft_header = vec![Line::from(Span::styled(
            format!("auth_kind  {}", dialog.draft.auth_kind.label()),
            Style::default().fg(Color::DarkGray),
        ))];
        draft_header.extend(draft_lines);
        frame.render_widget(
            Paragraph::new(Text::from(draft_header))
                .block(
                    Block::default()
                        .title(sanitize_display_text(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-provider-studio-draft")
                        )))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right_rows[0],
        );

        let adapter_models_split = if should_stack_detail_layout(right_rows[1].width, 24, 28) {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(right_rows[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints(adaptive_detail_split(right_rows[1].width, 24, 28))
                .split(right_rows[1])
        };
        let adapters_area = adapter_models_split[0];
        let models_area = adapter_models_split[1];

        let adapter_items = if dialog.adapter_candidate_ids.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-provider-studio-adapter-models-empty"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .adapter_candidate_ids
                .iter()
                .map(|adapter_id| {
                    let adapter_models = dialog
                        .adapter_models
                        .iter()
                        .find(|adapter_models| adapter_models.adapter_id == *adapter_id);
                    let enabled = if dialog.selected_adapter_ids.contains(adapter_id.as_str()) {
                        "[x]"
                    } else {
                        "[ ]"
                    };
                    let detail = if let Some(adapter) = adapter_models {
                        if adapter.error.is_none() {
                            format!(
                                "{} models · {}",
                                adapter.models.len(),
                                adapter
                                    .resolved_base_url
                                    .as_deref()
                                    .map(|value| truncate_display_text(value, 28))
                                    .unwrap_or_else(|| "loaded".to_owned())
                            )
                        } else {
                            truncate_display_text(adapter.error.as_deref().unwrap_or("error"), 32)
                        }
                    } else {
                        "not listed".to_owned()
                    };
                    let detail_style = match adapter_models {
                        Some(adapter) if adapter.error.is_none() => {
                            Style::default().fg(Color::DarkGray)
                        }
                        Some(_) => Style::default().fg(Color::Red),
                        None => Style::default().fg(Color::DarkGray),
                    };
                    ListItem::new(vec![
                        Line::from(sanitize_display_text(format!("{enabled} {}", adapter_id))),
                        Line::from(Span::styled(sanitize_display_text(detail), detail_style)),
                    ])
                })
                .collect::<Vec<_>>()
        };
        let adapter_list = List::new(adapter_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-provider-studio-adapters")
                    )))
                    .borders(Borders::ALL),
            )
            .highlight_style(if dialog.focus == ProviderStudioFocus::Adapters {
                selection_highlight_style()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol(">> ");
        let mut adapter_state = ListState::default();
        adapter_state
            .select((!dialog.adapter_candidate_ids.is_empty()).then_some(dialog.selected_adapter));
        frame.render_stateful_widget(adapter_list, adapters_area, &mut adapter_state);

        let model_items = provider_studio_selected_adapter_models(dialog)
            .map(|adapter_models| {
                if adapter_models.models.is_empty() {
                    vec![ListItem::new(Line::from(Span::styled(
                        ui_text::t(&self.i18n, "overlay-provider-studio-models-empty"),
                        Style::default().fg(Color::DarkGray),
                    )))]
                } else {
                    adapter_models
                        .models
                        .iter()
                        .map(|model| {
                            let detail = format!(
                                "{}{}",
                                model
                                    .display_name
                                    .clone()
                                    .unwrap_or_else(|| model.id.to_string()),
                                if dialog
                                    .catalog_resolved_model_ids
                                    .contains(model.id.as_str())
                                {
                                    " · catalog"
                                } else {
                                    ""
                                }
                            );
                            ListItem::new(vec![
                                Line::from(sanitize_display_text(model.id.to_string())),
                                Line::from(Span::styled(
                                    sanitize_display_text(detail),
                                    Style::default().fg(Color::DarkGray),
                                )),
                            ])
                        })
                        .collect::<Vec<_>>()
                }
            })
            .unwrap_or_else(|| {
                vec![ListItem::new(Line::from(Span::styled(
                    ui_text::t(&self.i18n, "overlay-provider-studio-models-empty"),
                    Style::default().fg(Color::DarkGray),
                )))]
            });
        let model_list = List::new(model_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-provider-studio-models")
                    )))
                    .borders(Borders::ALL),
            )
            .highlight_style(if dialog.focus == ProviderStudioFocus::Models {
                selection_highlight_style()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol(">> ");
        let mut model_state = ListState::default();
        let has_models = provider_studio_selected_adapter_models(dialog)
            .map(|adapter_models| !adapter_models.models.is_empty())
            .unwrap_or(false);
        model_state.select(has_models.then_some(dialog.selected_model));
        frame.render_stateful_widget(model_list, models_area, &mut model_state);

        let catalog_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(5)])
            .split(right_rows[2]);
        frame.render_widget(
            Paragraph::new(sanitize_display_text(format!(
                "query: {}  ·  page {}-{} / {}",
                if dialog.catalog_query.trim().is_empty() {
                    "<all>".to_owned()
                } else {
                    dialog.catalog_query.clone()
                },
                dialog.catalog_offset.saturating_add(1),
                dialog
                    .catalog_offset
                    .saturating_add(dialog.catalog_items.len()),
                dialog.catalog_total
            )))
            .block(
                Block::default()
                    .title(sanitize_display_text(format!(
                        " {} ",
                        ui_text::t(&self.i18n, "overlay-provider-studio-catalog")
                    )))
                    .borders(Borders::ALL),
            ),
            catalog_rows[0],
        );

        let catalog_split = if should_stack_detail_layout(catalog_rows[1].width, 34, 28) {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
                .split(catalog_rows[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints(adaptive_detail_split(catalog_rows[1].width, 34, 28))
                .split(catalog_rows[1])
        };
        let catalog_list_items = if dialog.catalog_loading {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-picker-loading"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else if dialog.catalog_items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-provider-studio-catalog-empty"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .catalog_items
                .iter()
                .map(|entry| {
                    ListItem::new(vec![
                        Line::from(sanitize_display_text(entry.model_id.as_str())),
                        Line::from(Span::styled(
                            sanitize_display_text(format!(
                                "{}{}",
                                entry.origin.clone().unwrap_or_else(|| "unknown".to_owned()),
                                if entry.kind
                                    == agena_api_server::local_api::ModelCatalogEntryKind::Custom
                                {
                                    " · custom"
                                } else {
                                    ""
                                }
                            )),
                            Style::default().fg(Color::DarkGray),
                        )),
                    ])
                })
                .collect::<Vec<_>>()
        };
        let catalog_list = List::new(catalog_list_items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(if dialog.focus == ProviderStudioFocus::Catalog {
                selection_highlight_style()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol(">> ");
        let mut catalog_state = ListState::default();
        catalog_state.select(
            (!dialog.catalog_loading && !dialog.catalog_items.is_empty())
                .then_some(dialog.selected_catalog),
        );
        frame.render_stateful_widget(catalog_list, catalog_split[0], &mut catalog_state);

        let detail = dialog
            .catalog_items
            .get(dialog.selected_catalog)
            .map(|entry| {
                [
                    format!("model_id: {}", entry.model_id),
                    format!(
                        "display: {}",
                        entry
                            .display_name
                            .clone()
                            .unwrap_or_else(|| "unset".to_owned())
                    ),
                    format!(
                        "origin: {}",
                        entry.origin.clone().unwrap_or_else(|| "unset".to_owned())
                    ),
                    format!(
                        "limits: ctx {} · out {}",
                        entry
                            .context_window_tokens
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "?".to_owned()),
                        entry
                            .max_output_tokens
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "?".to_owned())
                    ),
                    entry.description.clone().unwrap_or_default(),
                ]
                .join("\n")
            })
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-provider-studio-catalog-empty"));
        frame.render_widget(
            Paragraph::new(sanitize_display_text(detail))
                .block(
                    Block::default()
                        .title(sanitize_display_text(format!(
                            " {} ",
                            ui_text::t(&self.i18n, "overlay-provider-studio-detail")
                        )))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            catalog_split[1],
        );

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
            rows[2],
        );

        if let Some(editor) = dialog.editor.as_ref() {
            let area = centered_rect(area, 78, 7);
            frame.render_widget(Clear, area);
            let block = Block::default()
                .title(sanitize_display_text(format!(" {} ", editor.title)))
                .borders(Borders::ALL);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let editor_rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(sanitize_display_text(editor.prompt.as_str())),
                editor_rows[0],
            );
            let input_view = editor
                .input
                .render_view(editor_rows[1].width.saturating_sub(2), 1);
            frame.render_widget(
                Paragraph::new(Text::from(input_view.lines.clone()))
                    .block(Block::default().borders(Borders::ALL)),
                editor_rows[1],
            );
            frame.render_widget(
                Paragraph::new(sanitize_display_text(ui_text::t(
                    &self.i18n,
                    "overlay-provider-studio-edit-footer",
                ))),
                editor_rows[2],
            );
            frame.set_cursor_position((
                editor_rows[1]
                    .x
                    .saturating_add(1)
                    .saturating_add(input_view.cursor_x),
                editor_rows[1]
                    .y
                    .saturating_add(1)
                    .saturating_add(input_view.cursor_y),
            ));
        }
    }

    fn composer_height(&self) -> u16 {
        let line_count = max(1, self.composer.logical_line_count());
        let item_rows = u16::from(!self.composer_items.is_empty());
        let suggestion_rows = self.slash_command_suggestion_rows();
        let footer_rows = u16::from(self.should_render_composer_footer());
        let chrome_rows = 2_u16 + item_rows + suggestion_rows + footer_rows;
        min(12, line_count as u16 + chrome_rows)
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
    min(2, total_height)
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
    min(2, total_height)
}

pub(super) fn truncate_display_text(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return ".".repeat(max_width);
    }

    let target = max_width.saturating_sub(3);
    let mut width = 0_usize;
    let mut output = String::new();
    for grapheme in text.graphemes(true) {
        let grapheme_width = UnicodeWidthStr::width(grapheme);
        if width.saturating_add(grapheme_width) > target {
            break;
        }
        output.push_str(grapheme);
        width = width.saturating_add(grapheme_width);
    }
    output.push_str("...");
    output
}

fn sanitize_display_text(text: impl AsRef<str>) -> String {
    sanitize_terminal_text(text.as_ref())
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

pub(super) fn adaptive_sessions_split(total_height: u16) -> [Constraint; 2] {
    let sessions_height = adaptive_sessions_height(total_height);
    let available = total_height.saturating_sub(2);
    if available < sessions_height.saturating_add(12).saturating_add(3) {
        [Constraint::Percentage(50), Constraint::Percentage(50)]
    } else {
        [Constraint::Length(sessions_height), Constraint::Min(12)]
    }
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
