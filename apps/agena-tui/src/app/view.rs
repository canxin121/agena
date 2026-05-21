use super::*;

impl App {
    pub(super) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let composer_height = self.composer_height();
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(composer_height)])
            .split(area);

        let transcript_host_area = vertical[0];
        let composer = vertical[1];
        let transcript_footer_height =
            self.transcript_footer_height(transcript_host_area.width, transcript_host_area.height);

        let transcript_layout =
            transcript_surface_layout(transcript_host_area, transcript_footer_height);
        self.layout = LayoutCache {
            transcript_body: transcript_layout.body,
        };

        self.transcript.clamp_scroll(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
        );

        self.render_transcript_surface(frame, transcript_host_area);
        self.render_composer(frame, composer);
        self.render_overlay(frame, area);
    }

    fn render_transcript_surface(&mut self, frame: &mut Frame, area: Rect) {
        let footer_height = self.transcript_footer_height(area.width, area.height);
        let layout = transcript_surface_layout(area, footer_height);
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
            vec![
                Line::from(ui_text::t(&self.i18n, "no-session-selected")),
                Line::from(Span::styled(
                    ui_text::t(&self.i18n, "no-session-selected-hint"),
                    Style::default().fg(Color::DarkGray),
                )),
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

        if layout.footer.height > 0 {
            self.render_transcript_footer_row(frame, layout.footer);
        }
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
            if let Some(path) = self.current_session_path_label() {
                format!("{}  {}", title.trim_end(), path)
            } else {
                title
            }
        } else {
            format!(" {} ", ui_text::t(&self.i18n, "pane-transcript"))
        }
    }

    fn render_composer(&self, frame: &mut Frame, area: Rect) {
        let status_rows = u16::from(!self.composer_status_parts().is_empty());
        let (composer_area, status_area) = if status_rows > 0 && area.height > status_rows {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(area.height.saturating_sub(status_rows)),
                    Constraint::Length(status_rows),
                ])
                .split(area);
            (rows[0], Some(rows[1]))
        } else {
            (area, None)
        };

        let block = Block::default().borders(Borders::ALL);
        let inner = block.inner(composer_area);
        frame.render_widget(block, composer_area);

        if inner.width == 0 || inner.height == 0 {
            if let Some(status_area) = status_area {
                self.render_composer_status_row(frame, inset_rect(status_area, 1, 0));
            }
            return;
        }

        let item_count = self.composer_items.len();
        let item_rows = u16::from(item_count > 0);
        let popup_rows = min(
            self.composer_popup_rows(),
            inner.height.saturating_sub(item_rows).saturating_sub(1),
        );
        let editor_rows = inner
            .height
            .saturating_sub(item_rows)
            .saturating_sub(popup_rows)
            .max(1);

        let mut constraints = Vec::new();
        if item_rows > 0 {
            constraints.push(Constraint::Length(item_rows));
        }
        if popup_rows > 0 {
            constraints.push(Constraint::Length(popup_rows));
        }
        constraints.push(Constraint::Length(editor_rows));
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
        let popup_row = if popup_rows > 0 {
            let row = Some(rows[next_row]);
            next_row += 1;
            row
        } else {
            None
        };
        let editor_row = rows[next_row];

        if let Some(item_row) = item_row {
            self.render_composer_items_row(frame, item_row);
        }
        if let Some(popup_row) = popup_row {
            self.render_active_composer_popup(frame, popup_row);
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

        if self.overlay.is_none()
            && self.focus == Focus::Composer
            && self.prompt_history_search.is_none()
            && self.selected_composer_item.is_none()
        {
            frame.set_cursor_position((
                editor_x.saturating_add(editor_view.cursor_x),
                editor_row.y.saturating_add(editor_view.cursor_y),
            ));
        }

        if let Some(status_area) = status_area {
            self.render_composer_status_row(frame, inset_rect(status_area, 1, 0));
        }
    }

    fn render_active_composer_popup(&self, frame: &mut Frame, area: Rect) {
        if self.prompt_history_search.is_some() {
            self.render_prompt_history_search(frame, area);
        } else if self.file_mention_suggestions.is_some() {
            self.render_file_mention_suggestions(frame, area);
        } else {
            self.render_slash_command_suggestions(frame, area);
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

    fn render_file_mention_suggestions(&self, frame: &mut Frame, area: Rect) {
        let Some(state) = self.file_mention_suggestions.as_ref() else {
            return;
        };
        if area.width == 0 || area.height == 0 || state.items.is_empty() {
            return;
        }

        let selected = min(state.selected, state.items.len().saturating_sub(1));
        let visible_rows = min(
            area.height as usize,
            min(MAX_FILE_MENTION_SUGGESTIONS, state.items.len()),
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
                        .fg(self.theme_color("flash_info", Color::Cyan))
                        .add_modifier(Modifier::BOLD)
                };
                let detail_style = if is_selected {
                    base_style
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let marker = if is_selected { "@ " } else { "  " };
                let marker_width = UnicodeWidthStr::width(marker);
                let name_max = min(28, width.saturating_sub(marker_width));
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

    fn render_prompt_history_search(&self, frame: &mut Frame, area: Rect) {
        let Some(search) = self.prompt_history_search.as_ref() else {
            return;
        };
        if area.width == 0 || area.height == 0 {
            return;
        }

        let row_count = max(1, area.height as usize);
        let result_rows = row_count.saturating_sub(1);
        let query = sanitize_display_text(search.query.text());
        let mut lines = vec![Line::from(vec![
            Span::styled(
                "history> ",
                Style::default()
                    .fg(self.theme_color("accent", Color::Cyan))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(query.clone()),
        ])];

        if result_rows > 0 {
            if search.results.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  no prompt history matches",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                let selected = min(search.selected, search.results.len().saturating_sub(1));
                let visible_rows = min(result_rows, search.results.len());
                let start = selected.saturating_add(1).saturating_sub(visible_rows);
                for (index, result) in search
                    .results
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(visible_rows)
                {
                    let is_selected = index == selected;
                    let base_style = if is_selected {
                        selection_highlight_style()
                    } else {
                        Style::default()
                    };
                    let marker = if is_selected { "> " } else { "  " };
                    let prefix = format!("#{:<3} ", result.history_index + 1);
                    lines.push(Line::from(vec![
                        Span::styled(marker, base_style),
                        Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            truncate_display_text(
                                sanitize_display_text(result.text.as_str()).as_str(),
                                area.width.saturating_sub(7) as usize,
                            ),
                            base_style,
                        ),
                    ]));
                }
            }
        }

        frame.render_widget(Paragraph::new(Text::from(lines.clone())), area);

        if self.overlay.is_none() && self.focus == Focus::Composer {
            let query_width = UnicodeWidthStr::width(query.as_str()) as u16;
            frame.set_cursor_position((
                area.x.saturating_add(9).saturating_add(query_width),
                area.y,
            ));
        }
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
            let style = if self.selected_composer_item == Some(index) {
                selection_highlight_style().add_modifier(Modifier::BOLD)
            } else {
                self.composer_item_style(item)
            };
            spans.push(Span::styled(format!("[{}]", item.short_label()), style));
        }

        frame.render_widget(
            Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
            area,
        );
    }

    fn render_transcript_footer_row(&self, frame: &mut Frame, area: Rect) {
        let lines = self.transcript_footer_lines(area.width);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            area,
        );
    }

    fn transcript_footer_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if let Some(flash) = &self.flash {
            lines.extend(self.wrap_styled_text(
                flash.text.as_str(),
                width,
                self.flash_style(flash.level),
            ));
            return lines;
        }

        let combined = self.transcript_footer_text();
        if combined.trim().is_empty() {
            return lines;
        }
        lines.extend(self.wrap_styled_text(
            combined.as_str(),
            width,
            Style::default().fg(Color::DarkGray),
        ));
        lines
    }

    fn transcript_footer_text(&self) -> String {
        let mut parts = Vec::new();
        if !self.queue.is_empty() {
            let preview = self.queue.first_preview(28).unwrap_or_default();
            if preview.is_empty() {
                parts.push(format!("queue {}", self.queue.len()));
            } else {
                parts.push(format!("queue {} {}", self.queue.len(), preview));
            }
        }
        if let Some(status_line) = self
            .status_line
            .as_ref()
            .and_then(|status_line| status_line.text.as_ref())
            .map(String::as_str)
            && !status_line.trim().is_empty()
        {
            parts.push(status_line.trim().to_string());
        }
        for segment in self.backend.plugin_statusline_segments() {
            if segment.content.trim().is_empty() {
                continue;
            }
            parts.push(segment.content.clone());
        }
        for block in self.backend.plugin_tui_content_blocks() {
            if block.block.location != "composer_footer" || block.block.body.trim().is_empty() {
                continue;
            }
            let label = block.block.title.trim();
            if label.is_empty() {
                parts.push(block.block.body.trim().to_string());
            } else {
                parts.push(format!("{label}: {}", block.block.body.trim()));
            }
        }

        parts.join("  |  ")
    }

    fn transcript_footer_height(&self, width: u16, total_height: u16) -> u16 {
        if total_height <= transcript_surface_header_height(total_height).saturating_add(1) {
            return 0;
        }
        let line_count = self.transcript_footer_lines(width).len();
        if line_count == 0 {
            0
        } else {
            min(2, line_count as u16)
        }
    }

    fn composer_popup_rows(&self) -> u16 {
        if self.overlay.is_some() || self.focus != Focus::Composer {
            return 0;
        }
        if let Some(search) = self.prompt_history_search.as_ref() {
            let result_rows = if search.results.is_empty() {
                1
            } else {
                min(MAX_PROMPT_HISTORY_SEARCH_RESULTS, search.results.len())
            };
            return (result_rows + 1) as u16;
        }
        if let Some(state) = self.file_mention_suggestions.as_ref() {
            return min(MAX_FILE_MENTION_SUGGESTIONS, state.items.len()) as u16;
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

    fn render_composer_status_row(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let text = self.composer_status_parts().join("  |  ");
        let lines = self.wrap_styled_text(
            text.as_str(),
            area.width,
            Style::default().fg(Color::DarkGray),
        );
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
            area,
        );
    }

    fn composer_status_parts(&self) -> Vec<String> {
        let mut parts = self.current_session_status_parts();
        if self.composer_mode.is_vim() {
            parts.push(format!("mode {}", self.composer_mode.status_label()));
        }
        if let Some(selected) = self
            .selected_composer_item
            .and_then(|index| self.composer_items.get(index).map(|item| (index, item)))
        {
            parts.push(format!(
                "item {}/{} {}",
                selected.0 + 1,
                self.composer_items.len(),
                selected.1.short_label()
            ));
        }
        if let Some(search) = self.prompt_history_search.as_ref() {
            let query = search.query.text().trim();
            let selection = min(search.selected + 1, search.results.len().max(1));
            parts.push(format!(
                "history {selection}/{}{}",
                search.results.len(),
                if query.is_empty() {
                    String::new()
                } else {
                    format!(" query={query}")
                }
            ));
        } else if let Some(state) = self.file_mention_suggestions.as_ref() {
            let query = state.query.trim();
            let suffix = if query.is_empty() {
                "@".to_string()
            } else {
                format!("@{query}")
            };
            parts.push(format!("mention {suffix}"));
        } else if let Some(state) = self.slash_command_suggestions.as_ref() {
            let query = state.query.trim();
            let suffix = if query.is_empty() {
                "/".to_string()
            } else {
                format!("/{query}")
            };
            parts.push(format!("slash {suffix}"));
        }
        if let Some(execution) = self.transcript.execution.as_ref() {
            if !execution.pending_user_input_requests.is_empty() {
                parts.push(format!(
                    "input {} pending (Alt+U)",
                    execution.pending_user_input_requests.len()
                ));
            }
            if !execution.pending_permission_requests.is_empty() {
                parts.push(format!(
                    "approval {} pending (Alt+P)",
                    execution.pending_permission_requests.len()
                ));
            }
        }
        parts
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
            Overlay::Help(dialog) => {
                self.render_help_overlay(frame, area, dialog);
            }
            Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                self.render_line_overlay(frame, area, dialog);
            }
            Overlay::SettingsStudio(dialog) => {
                self.render_settings_studio_overlay(frame, area, dialog);
            }
            Overlay::SettingsValueEdit(dialog) => {
                self.render_line_overlay(
                    frame,
                    area,
                    &LineInputOverlay {
                        title: dialog.title.clone(),
                        prompt: dialog.prompt.clone(),
                        input: dialog.input.clone(),
                    },
                );
            }
            Overlay::RuntimeSettingEdit(dialog) => {
                self.render_line_overlay(
                    frame,
                    area,
                    &LineInputOverlay {
                        title: dialog.title.clone(),
                        prompt: dialog.prompt.clone(),
                        input: dialog.input.clone(),
                    },
                );
            }
            Overlay::Choice(dialog) => {
                self.render_choice_overlay(frame, area, dialog);
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
            Overlay::SessionSearch(dialog) => {
                self.render_session_search_overlay(frame, area, dialog);
            }
            Overlay::Picker(dialog) => {
                self.render_picker_overlay(frame, area, dialog);
            }
            Overlay::SessionModelChooser(dialog) => {
                self.render_session_model_chooser_overlay(frame, area, dialog);
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
            Overlay::ModelCatalogStudio(dialog) => {
                self.render_model_catalog_studio_overlay(frame, area, dialog);
            }
        }
    }

    fn render_line_overlay(&self, frame: &mut Frame, area: Rect, dialog: &LineInputOverlay) {
        let area = centered_rect(area, 88, 10);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(2),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.prompt.as_str()))
                .wrap(Wrap { trim: false }),
            rows[0],
        );
        let input_block = Block::default().borders(Borders::ALL);
        let input_inner = input_block.inner(rows[1]);
        frame.render_widget(input_block, rows[1]);
        let view = dialog
            .input
            .render_view(input_inner.width.max(1), input_inner.height.max(1));
        frame.render_widget(Paragraph::new(Text::from(view.lines.clone())), input_inner);
        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-line-footer")),
            rows[2],
        );
        frame.set_cursor_position((
            input_inner.x.saturating_add(view.cursor_x),
            input_inner.y.saturating_add(view.cursor_y),
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
        let height = min(24, area.height.saturating_sub(4));
        let area = centered_rect(area, 92, height);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(
                " {} ",
                ui_text::t(&self.i18n, "overlay-user-input-title")
            )))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let nav_color = self.theme_color("flash_info", Color::Cyan);
        if dialog.screen == UserInputOverlayScreen::Review {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Min(10),
                    Constraint::Length(2),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(Text::from(vec![
                    Line::from(Span::styled(
                        sanitize_display_text(self.i18n.text_args(
                            "overlay-user-input-request-id",
                            &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
                        )),
                        Style::default().fg(Color::DarkGray),
                    )),
                    user_input_nav_line(dialog, nav_color),
                ]))
                .block(Block::default().borders(Borders::ALL).title(" Questions ")),
                rows[0],
            );

            let mut review_lines = vec![Line::from(Span::styled(
                "Review your answers before submitting.",
                Style::default().add_modifier(Modifier::BOLD),
            ))];
            review_lines.push(Line::from(""));
            for (index, question) in dialog.request.questions.iter().enumerate() {
                let values = dialog
                    .answers
                    .get(&question.id)
                    .map(|draft| user_input_answer_values(question, draft))
                    .unwrap_or_default();
                let answered = !values.is_empty();
                let style = if index == dialog.selected_question {
                    selection_highlight_style()
                } else {
                    Style::default()
                };
                review_lines.push(Line::from(vec![
                    Span::styled(format!("{} ", if answered { "[x]" } else { "[ ]" }), style),
                    Span::styled(
                        sanitize_display_text(user_input_question_label(question)),
                        style.add_modifier(Modifier::BOLD),
                    ),
                ]));
                review_lines.push(Line::from(Span::styled(
                    format!(
                        "    {}",
                        if answered {
                            truncate_display_text(values.join(", ").as_str(), 72)
                        } else {
                            "unanswered".to_string()
                        }
                    ),
                    if answered {
                        Style::default().fg(nav_color)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                )));
            }
            frame.render_widget(
                Paragraph::new(Text::from(review_lines))
                    .wrap(Wrap { trim: false })
                    .block(Block::default().borders(Borders::ALL).title(" Summary ")),
                rows[1],
            );
            frame.render_widget(
                Paragraph::new(
                    "j/k choose question  e edit  enter submit  esc close  ctrl+d cancel",
                )
                .wrap(Wrap { trim: false }),
                rows[2],
            );
            return;
        }

        let Some(question) = dialog.request.questions.get(dialog.selected_question) else {
            frame.render_widget(
                Paragraph::new("No questions.")
                    .block(Block::default().borders(Borders::ALL).title(" Detail ")),
                inner,
            );
            return;
        };
        let custom_height = if question.allow_custom { 3 } else { 0 };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(5),
                Constraint::Min(6),
                Constraint::Length(custom_height),
                Constraint::Length(2),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(Span::styled(
                    sanitize_display_text(self.i18n.text_args(
                        "overlay-user-input-request-id",
                        &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
                    )),
                    Style::default().fg(Color::DarkGray),
                )),
                user_input_nav_line(dialog, nav_color),
            ]))
            .block(Block::default().borders(Borders::ALL).title(" Questions ")),
            rows[0],
        );

        let draft = dialog
            .answers
            .get(&question.id)
            .cloned()
            .unwrap_or_default();
        let answer_summary = user_input_answer_summary(question, &draft);
        frame.render_widget(
            Paragraph::new(Text::from(vec![
                Line::from(Span::styled(
                    sanitize_display_text(question.question.as_str()),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    sanitize_display_text(format!(
                        "{} · id={}",
                        if question.multiple {
                            "Choose one or more"
                        } else {
                            "Choose one"
                        },
                        question.id
                    )),
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "Current answer: ",
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        sanitize_display_text(answer_summary.as_str()),
                        if answer_summary == "unanswered" {
                            Style::default().fg(Color::DarkGray)
                        } else {
                            Style::default().fg(nav_color)
                        },
                    ),
                ]),
            ]))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(" Prompt ")),
            rows[1],
        );

        let mut option_lines = Vec::new();
        for (index, option) in question.options.iter().enumerate() {
            let picked = draft.option_indexes.contains(&index);
            let focused = index == dialog.selected_option && !dialog.editing_custom;
            let style = if focused {
                selection_highlight_style()
            } else {
                Style::default()
            };
            let prefix = if question.multiple {
                if picked { "[x]" } else { "[ ]" }
            } else if picked {
                "(x)"
            } else {
                "( )"
            };
            option_lines.push(Line::from(vec![
                Span::styled(format!("{prefix} "), style),
                Span::styled(
                    sanitize_display_text(option.label.as_str()),
                    style.add_modifier(Modifier::BOLD),
                ),
            ]));
            if !option.description.trim().is_empty() {
                option_lines.push(Line::from(Span::styled(
                    format!(
                        "    {}",
                        truncate_display_text(
                            sanitize_display_text(option.description.as_str()).as_str(),
                            rows[2].width.saturating_sub(6) as usize,
                        )
                    ),
                    if focused {
                        style
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                )));
            }
        }
        if question.allow_custom {
            let custom_row = question.options.len();
            let custom_values = if draft.custom_values.is_empty() {
                "Press Enter or e to type a custom answer".to_string()
            } else {
                draft.custom_values.join(", ")
            };
            let custom_selected = custom_row == dialog.selected_option && !dialog.editing_custom;
            let custom_style = if custom_selected {
                selection_highlight_style()
            } else {
                Style::default()
            };
            let custom_picked = !draft.custom_values.is_empty();
            let prefix = if question.multiple {
                if custom_picked { "[x]" } else { "[ ]" }
            } else if custom_picked {
                "(x)"
            } else {
                "( )"
            };
            option_lines.push(Line::from(""));
            option_lines.push(Line::from(vec![
                Span::styled(format!("{prefix} "), custom_style),
                Span::styled("Other", custom_style.add_modifier(Modifier::BOLD)),
            ]));
            option_lines.push(Line::from(Span::styled(
                format!(
                    "    {}",
                    truncate_display_text(
                        sanitize_display_text(custom_values.as_str()).as_str(),
                        rows[2].width.saturating_sub(6) as usize,
                    )
                ),
                if draft.custom_values.is_empty() {
                    if custom_selected {
                        custom_style
                    } else {
                        Style::default().fg(Color::DarkGray)
                    }
                } else if custom_selected {
                    custom_style
                } else {
                    Style::default().fg(nav_color)
                },
            )));
        }
        frame.render_widget(
            Paragraph::new(Text::from(option_lines))
                .wrap(Wrap { trim: false })
                .block(Block::default().borders(Borders::ALL).title(" Choices ")),
            rows[2],
        );

        if question.allow_custom {
            let custom_view = dialog
                .custom_input
                .render_view(rows[3].width.saturating_sub(2), 1);
            frame.render_widget(
                Paragraph::new(Text::from(custom_view.lines.clone())).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(if dialog.editing_custom {
                            " Custom Input "
                        } else {
                            " Custom Input (e / paste) "
                        }),
                ),
                rows[3],
            );
            if dialog.editing_custom {
                frame.set_cursor_position((
                    rows[3]
                        .x
                        .saturating_add(1)
                        .saturating_add(custom_view.cursor_x),
                    rows[3]
                        .y
                        .saturating_add(1)
                        .saturating_add(custom_view.cursor_y),
                ));
            }
        }

        frame.render_widget(
            Paragraph::new(
                "j/k move  tab next  shift+tab prev  space select/toggle  enter choose/next  ctrl+d cancel",
            )
            .wrap(Wrap { trim: false }),
            rows[4],
        );
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

    fn render_help_overlay(&self, frame: &mut Frame, area: Rect, dialog: &HelpOverlay) {
        let area = centered_rect(area, 108, 28);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(
                " {} ",
                ui_text::t(&self.i18n, "help-title")
            )))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(1)])
            .split(inner);

        let text = ui_text::help_lines(&self.i18n)
            .into_iter()
            .map(|line| match line.kind {
                ui_text::HelpLineKind::Header => Line::from(Span::styled(
                    sanitize_display_text(line.text),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
                ui_text::HelpLineKind::Section => Line::from(Span::styled(
                    sanitize_display_text(line.text),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                ui_text::HelpLineKind::Body => Line::from(sanitize_display_text(line.text)),
                ui_text::HelpLineKind::Spacer => Line::from(""),
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Text::from(text))
                .scroll((dialog.scroll, 0))
                .wrap(Wrap { trim: false }),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(ui_text::t(&self.i18n, "overlay-help-footer"))
                .alignment(Alignment::Right)
                .style(Style::default().fg(Color::DarkGray)),
            rows[1],
        );
    }

    fn render_session_search_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SessionSearchOverlay,
    ) {
        let area = centered_rect(area, 108, 24);
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
                .map(|session| {
                    let mut detail_parts = vec![ui_text::session_meta(
                        &self.i18n,
                        session.id,
                        session.message_count,
                        session.updated_at,
                    )];
                    if self.transcript.session_id == Some(session.id) {
                        detail_parts.push(ui_text::t(&self.i18n, "session-tag-current"));
                    }
                    if let Some(parent_id) = session.parent_id {
                        detail_parts.push(self.i18n.text_args(
                            "session-summary-parent",
                            &crate::fl_args!("id" => parent_id),
                        ));
                    }
                    if session.child_session_count > 0 {
                        detail_parts.push(self.i18n.text_args(
                            "session-summary-children",
                            &crate::fl_args!("count" => session.child_session_count as i64),
                        ));
                    }
                    ListItem::new(vec![
                        Line::from(sanitize_display_text(session.title.as_str())),
                        Line::from(Span::styled(
                            sanitize_display_text(detail_parts.join(" | ")),
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
            Paragraph::new(sanitize_display_text(dialog.footer.as_str()))
                .style(Style::default().fg(Color::DarkGray)),
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

    fn render_session_model_chooser_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SessionModelChooserOverlay,
    ) {
        let area = centered_rect(area, 108, 26);
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

        let page_size = dialog.page_size.max(1);
        let page_start = (dialog.selected / page_size) * page_size;
        let page_items = if dialog.loading {
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
                .skip(page_start)
                .take(page_size)
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
        let list_title = if dialog.items.is_empty() {
            " Models ".to_string()
        } else {
            let page = (dialog.selected / page_size) + 1;
            let page_count = dialog.items.len().div_ceil(page_size);
            format!(
                " Models  ·  page {page}/{page_count}  ·  {} match(es) ",
                dialog.items.len()
            )
        };
        let list = List::new(page_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(list_title))
                    .borders(Borders::ALL),
            )
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select(
            (!dialog.loading && !dialog.items.is_empty()).then_some(dialog.selected - page_start),
        );
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

    fn render_choice_overlay(&self, frame: &mut Frame, area: Rect, dialog: &ChoiceOverlay) {
        let area = top_centered_rect(area, 96, 24, 1);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .split(inner);

        let input_view = dialog.input.render_view(rows[0].width.saturating_sub(2), 1);
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[0],
        );
        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.prompt.as_str()))
                .wrap(Wrap { trim: false }),
            rows[1],
        );

        let choice_rows = Self::choice_overlay_rows(dialog);
        let has_rows = !choice_rows.is_empty();
        let result_items = if !has_rows {
            vec![ListItem::new(Line::from(Span::styled(
                sanitize_display_text(dialog.empty_message.as_str()),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            choice_rows
                .into_iter()
                .map(|row| {
                    let (label, detail, style) = match row {
                        ChoiceRow::Clear => (
                            "Clear value".to_string(),
                            choice_overlay_clear_detail(&dialog.action),
                            Style::default().fg(Color::Yellow),
                        ),
                        ChoiceRow::Custom(value) => (
                            "Use typed value".to_string(),
                            format!(
                                "Apply exactly {}",
                                format_setting_value_inline(&JsonValue::String(value))
                            ),
                            Style::default().fg(Color::Cyan),
                        ),
                        ChoiceRow::Item(item) => (item.label, item.detail, Style::default()),
                    };
                    ListItem::new(vec![
                        Line::from(Span::styled(sanitize_display_text(label.as_str()), style)),
                        Line::from(Span::styled(
                            sanitize_display_text(detail.as_str()),
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
        state.select(has_rows.then_some(dialog.selected));
        frame.render_stateful_widget(list, rows[2], &mut state);

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
            rows[3],
        );
        frame.set_cursor_position((
            rows[0]
                .x
                .saturating_add(1)
                .saturating_add(input_view.cursor_x),
            rows[0]
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

    fn render_settings_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SettingsStudioOverlay,
    ) {
        let area = centered_rect(area, 122, 36);
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
        let current_section = dialog.sections.get(dialog.selected_section);
        let section_summary = current_section
            .map(|section| format!("{}  ·  {} item(s)", section.summary, section.items.len()))
            .unwrap_or_else(|| "no settings available".to_string());
        self.render_header_row(
            frame,
            rows[0],
            "Settings".to_string(),
            section_summary,
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        );

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(60)])
            .split(rows[1]);
        let nav_area = content[0];
        let right_area = content[1];
        let right_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(10),
                Constraint::Length(7),
            ])
            .split(right_area);

        let nav_items = dialog
            .sections
            .iter()
            .map(|section| {
                ListItem::new(vec![
                    Line::from(sanitize_display_text(section.label.as_str())),
                    Line::from(Span::styled(
                        sanitize_display_text(section.summary.as_str()),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
            })
            .collect::<Vec<_>>();
        let nav_list = List::new(nav_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(" Sections "))
                    .borders(Borders::ALL),
            )
            .highlight_style(if dialog.focus == SettingsStudioFocus::Navigation {
                selection_highlight_style()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol(">> ");
        let mut nav_state = ListState::default();
        nav_state.select((!dialog.sections.is_empty()).then_some(dialog.selected_section));
        frame.render_stateful_widget(nav_list, nav_area, &mut nav_state);

        let section_title = current_section
            .map(|section| section.label.clone())
            .unwrap_or_else(|| "Section".to_string());
        let section_description = current_section
            .map(|section| section.description.clone())
            .unwrap_or_else(|| "No section selected.".to_string());
        frame.render_widget(
            Paragraph::new(sanitize_display_text(section_description))
                .block(
                    Block::default()
                        .title(sanitize_display_text(format!(" {} ", section_title)))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right_rows[0],
        );

        let selected_item =
            current_section.and_then(|section| section.items.get(dialog.selected_item));
        let item_rows = current_section
            .map(|section| {
                if section.items.is_empty() {
                    vec![ListItem::new(Line::from(Span::styled(
                        "No settings in this section.",
                        Style::default().fg(Color::DarkGray),
                    )))]
                } else {
                    section
                        .items
                        .iter()
                        .map(|item| {
                            let mut first_line = vec![Span::styled(
                                sanitize_display_text(item.label.as_str()),
                                Style::default().add_modifier(Modifier::BOLD),
                            )];
                            if !item.value.trim().is_empty() {
                                first_line.push(Span::raw("  "));
                                first_line.push(Span::styled(
                                    sanitize_display_text(item.value.as_str()),
                                    Style::default().fg(Color::Cyan),
                                ));
                            }
                            ListItem::new(vec![
                                Line::from(first_line),
                                Line::from(Span::styled(
                                    sanitize_display_text(item.detail.as_str()),
                                    Style::default().fg(Color::DarkGray),
                                )),
                            ])
                        })
                        .collect::<Vec<_>>()
                }
            })
            .unwrap_or_default();
        let item_list = List::new(item_rows)
            .block(
                Block::default()
                    .title(sanitize_display_text(" Options "))
                    .borders(Borders::ALL),
            )
            .highlight_style(if dialog.focus == SettingsStudioFocus::Items {
                selection_highlight_style()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol(">> ");
        let mut item_state = ListState::default();
        let has_items = current_section
            .map(|section| !section.items.is_empty())
            .unwrap_or(false);
        item_state.select(has_items.then_some(dialog.selected_item));
        frame.render_stateful_widget(item_list, right_rows[1], &mut item_state);

        let detail_text = selected_item
            .map(|item| {
                if item.value.trim().is_empty() {
                    format!("{}\nEnter opens or edits this setting.", item.detail)
                } else {
                    format!(
                        "{}\nCurrent value: {}\nEnter opens or edits this setting.",
                        item.detail, item.value
                    )
                }
            })
            .unwrap_or_else(|| "Select a section and an option to inspect or edit it.".to_string());
        frame.render_widget(
            Paragraph::new(sanitize_display_text(detail_text))
                .block(
                    Block::default()
                        .title(sanitize_display_text(" Details "))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right_rows[2],
        );

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
            rows[2],
        );
    }

    fn render_provider_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &ProviderStudioOverlay,
    ) {
        let area = centered_rect(area, 122, 38);
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

        self.render_header_row(
            frame,
            rows[0],
            ui_text::t(&self.i18n, "overlay-provider-studio-header"),
            String::new(),
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        );

        let right_area = if dialog.show_provider_list {
            let content = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(28), Constraint::Min(54)])
                .split(rows[1]);
            let providers_area = content[0];
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
                .highlight_style(Style::default().add_modifier(Modifier::BOLD))
                .highlight_symbol(">> ");
            let mut provider_state = ListState::default();
            provider_state
                .select((!dialog.providers.is_empty()).then_some(dialog.selected_provider));
            frame.render_stateful_widget(provider_list, providers_area, &mut provider_state);
            content[1]
        } else {
            rows[1]
        };
        let draft_fields = provider_studio_visible_fields(dialog);
        let draft_panel_height = u16::try_from(draft_fields.len().saturating_add(3))
            .unwrap_or(u16::MAX)
            .clamp(8, 18);
        let right_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(draft_panel_height), Constraint::Min(10)])
            .split(right_area);

        let draft_lines = draft_fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let value = provider_studio_main_field_value(dialog, *field);
                let display = match field {
                    ProviderStudioField::ApiKey
                    | ProviderStudioField::RefreshToken
                    | ProviderStudioField::AccessToken
                    | ProviderStudioField::AccessKeyId
                    | ProviderStudioField::SecretAccessKey
                    | ProviderStudioField::SessionToken
                        if !value.trim().is_empty() =>
                    {
                        "********".to_owned()
                    }
                    _ if value.trim().is_empty() => "unset".to_owned(),
                    _ => value,
                };
                let selected = dialog.detail_page.is_none()
                    && dialog.focus == ProviderStudioFocus::Fields
                    && dialog.selected_field == index;
                let label_style = if selected {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let value_style = label_style;
                Line::from(vec![
                    Span::styled(
                        format!("{:>16}", provider_studio_field_label(*field)),
                        label_style,
                    ),
                    Span::raw("  "),
                    Span::styled(sanitize_display_text(display), value_style),
                ])
            })
            .collect::<Vec<_>>();
        frame.render_widget(
            Paragraph::new(Text::from(draft_lines))
                .block(Block::default().borders(Borders::ALL))
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
                    let enabled =
                        if !provider_studio_adapter_selectable(dialog, adapter_id.as_str()) {
                            "[-]"
                        } else if dialog.selected_adapter_ids.contains(adapter_id.as_str()) {
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
                    } else if let Some(rule) =
                        provider_studio_adapter_rule(dialog, adapter_id.as_str())
                    {
                        let mut parts = vec![rule.detail.to_owned()];
                        if rule.supports_draft_model_listing {
                            parts.push("live list".to_owned());
                        }
                        if dialog.configured_adapter_ids.contains(adapter_id) {
                            parts.push("configured".to_owned());
                        }
                        truncate_display_text(parts.join(" · ").as_str(), 48)
                    } else if dialog.configured_adapter_ids.contains(adapter_id) {
                        "configured on disk; not in current auth contract".to_owned()
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
                            let selected = if provider_studio_model_selected(
                                dialog,
                                adapter_models.adapter_id.as_str(),
                                model.id.as_str(),
                            ) {
                                "[x]"
                            } else {
                                "[ ]"
                            };
                            let match_entry =
                                dialog.catalog_matches.get(&provider_studio_model_key(
                                    adapter_models.adapter_id.as_str(),
                                    model.id.as_str(),
                                ));
                            let detail = format!(
                                "{}{}",
                                match_entry
                                    .map(|entry| entry.model_id.to_string())
                                    .unwrap_or_else(|| "catalog unmatched".to_owned()),
                                if dialog.draft.default_adapter == adapter_models.adapter_id
                                    && dialog.draft.default_model == model.id.as_str()
                                {
                                    "  ·  default"
                                } else {
                                    ""
                                },
                            );
                            ListItem::new(vec![
                                Line::from(sanitize_display_text(format!(
                                    "{selected} {}",
                                    model.id.as_str()
                                ))),
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

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
            rows[2],
        );

        if let Some(detail_page) = dialog.detail_page.as_ref() {
            let detail_fields = provider_studio_detail_fields(dialog);
            let auth_state_lines = provider_studio_auth_state_lines(dialog);
            let detail_height = u16::try_from(
                detail_fields
                    .len()
                    .saturating_add(auth_state_lines.len())
                    .saturating_add(4),
            )
            .unwrap_or(u16::MAX)
            .clamp(10, 24);
            let detail_area = centered_rect(area, 92, detail_height);
            frame.render_widget(Clear, detail_area);
            let detail_block = Block::default()
                .title(sanitize_display_text(format!(" {} ", detail_page.title)))
                .borders(Borders::ALL);
            let detail_inner = detail_block.inner(detail_area);
            frame.render_widget(detail_block, detail_area);
            let detail_rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(6), Constraint::Length(1)])
                .split(detail_inner);

            let mut lines = auth_state_lines
                .into_iter()
                .map(|line| {
                    Line::from(Span::styled(
                        sanitize_display_text(line),
                        Style::default().fg(Color::DarkGray),
                    ))
                })
                .collect::<Vec<_>>();
            lines.insert(
                0,
                Line::from(vec![
                    Span::styled(
                        format!(
                            "{:>16}",
                            provider_studio_field_label(ProviderStudioField::AuthStatus)
                        ),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        provider_studio_auth_status_summary(dialog),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
            );
            lines.extend(detail_fields.iter().enumerate().map(|(index, field)| {
                let value = provider_studio_field_value(&dialog.draft, *field);
                let display = match field {
                    ProviderStudioField::ApiKey
                    | ProviderStudioField::RefreshToken
                    | ProviderStudioField::AccessToken
                    | ProviderStudioField::AccessKeyId
                    | ProviderStudioField::SecretAccessKey
                    | ProviderStudioField::SessionToken
                        if !value.trim().is_empty() =>
                    {
                        "********".to_owned()
                    }
                    _ if value.trim().is_empty() => "unset".to_owned(),
                    _ => value,
                };
                let selected = dialog.editor.is_none() && detail_page.selected_field == index;
                let label_style = if selected {
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
                        if selected {
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD)
                        } else if provider_studio_field_editable(dialog, *field) {
                            Style::default()
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ),
                ])
            }));
            frame.render_widget(
                Paragraph::new(Text::from(lines))
                    .wrap(Wrap { trim: false })
                    .block(
                        Block::default()
                            .title(sanitize_display_text(format!(
                                " {} ",
                                ui_text::t(&self.i18n, "overlay-provider-studio-detail")
                            )))
                            .borders(Borders::NONE),
                    ),
                detail_rows[0],
            );
            frame.render_widget(
                Paragraph::new(sanitize_display_text(detail_page.footer.as_str())),
                detail_rows[1],
            );
        }

        if let Some(editor) = dialog.editor.as_ref() {
            let area = if editor.multiline {
                centered_rect(area, 92, 24)
            } else {
                centered_rect(area, 78, 7)
            };
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
                    Constraint::Min(if editor.multiline { 8 } else { 3 }),
                    Constraint::Length(1),
                ])
                .split(inner);
            frame.render_widget(
                Paragraph::new(sanitize_display_text(editor.prompt.as_str())),
                editor_rows[0],
            );
            let input_view = editor.input.render_view(
                editor_rows[1].width.saturating_sub(2),
                editor_rows[1].height.saturating_sub(2).max(1),
            );
            frame.render_widget(
                Paragraph::new(Text::from(input_view.lines.clone()))
                    .block(Block::default().borders(Borders::ALL)),
                editor_rows[1],
            );
            frame.render_widget(
                Paragraph::new(sanitize_display_text(editor.footer.as_str())),
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

    fn render_model_catalog_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &ModelCatalogStudioOverlay,
    ) {
        let area = centered_rect(area, 116, 36);
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
                Constraint::Min(16),
                Constraint::Length(1),
            ])
            .split(inner);
        let summary = format!(
            "query {}  ·  page {}-{} / {}  ·  {} official / {} custom",
            if dialog.query.trim().is_empty() {
                "<all>".to_owned()
            } else {
                dialog.query.clone()
            },
            dialog.offset.saturating_add(1),
            dialog.offset.saturating_add(dialog.items.len()),
            dialog.total,
            dialog.summary.official_entry_count,
            dialog.summary.custom_entry_count,
        );
        self.render_header_row(
            frame,
            rows[0],
            "Model Catalog".to_string(),
            summary,
            Style::default().add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        );

        let content = if should_stack_detail_layout(rows[1].width, 48, 34) {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
                .split(rows[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints(adaptive_detail_split(rows[1].width, 48, 34))
                .split(rows[1])
        };

        let list_items = if dialog.loading {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-picker-loading"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else if dialog.items.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-provider-studio-catalog-empty"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            dialog
                .items
                .iter()
                .map(|entry| {
                    ListItem::new(vec![
                        Line::from(sanitize_display_text(entry.model_id.as_str())),
                        Line::from(Span::styled(
                            sanitize_display_text(format!(
                                "{}{}",
                                entry.display_name.clone().unwrap_or_else(|| {
                                    entry.origin.clone().unwrap_or_else(|| "unknown".to_owned())
                                }),
                                if entry.kind
                                    == agena_api_server::local_api::ModelCatalogEntryKind::Custom
                                {
                                    "  ·  custom"
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
        let catalog_list = List::new(list_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(" Entries "))
                    .borders(Borders::ALL),
            )
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut list_state = ListState::default();
        list_state.select((!dialog.loading && !dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(catalog_list, content[0], &mut list_state);

        let detail = dialog
            .items
            .get(dialog.selected)
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
                    format!("source: {:?}  ·  kind: {:?}", entry.source, entry.kind),
                    entry.description.clone().unwrap_or_default(),
                ]
                .into_iter()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
            })
            .unwrap_or_else(|| {
                dialog.summary.last_error.clone().unwrap_or_else(|| {
                    ui_text::t(&self.i18n, "overlay-provider-studio-catalog-empty")
                })
            });
        frame.render_widget(
            Paragraph::new(sanitize_display_text(detail))
                .block(
                    Block::default()
                        .title(sanitize_display_text(" Detail "))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            content[1],
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
        let status_rows = u16::from(!self.composer_status_parts().is_empty());
        let popup_rows = self.composer_popup_rows();
        let chrome_rows = 2_u16 + item_rows + popup_rows + status_rows;
        min(12, line_count as u16 + chrome_rows)
    }
}

fn user_input_nav_line(dialog: &UserInputOverlay, answered_color: Color) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, question) in dialog.request.questions.iter().enumerate() {
        let answered = dialog
            .answers
            .get(&question.id)
            .map(|draft| !user_input_answer_values(question, draft).is_empty())
            .unwrap_or(false);
        let label = if question.header.trim().is_empty() {
            format!("Q{}", index + 1)
        } else {
            question.header.clone()
        };
        let text = format!(
            " {} {} ",
            if answered { "[x]" } else { "[ ]" },
            truncate_display_text(sanitize_display_text(label.as_str()).as_str(), 12)
        );
        let selected =
            dialog.selected_question == index && dialog.screen == UserInputOverlayScreen::Question;
        let style = if selected {
            selection_highlight_style()
        } else if answered {
            Style::default()
                .fg(answered_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        spans.push(Span::styled(text, style));
        spans.push(Span::raw(" "));
    }
    if !App::user_input_review_hidden(dialog) {
        spans.push(Span::styled(
            " [>] Submit ",
            if dialog.screen == UserInputOverlayScreen::Review {
                selection_highlight_style()
            } else {
                Style::default()
            },
        ));
    }
    Line::from(spans)
}

fn user_input_answer_summary(question: &UserInputQuestion, draft: &UserInputAnswerDraft) -> String {
    let values = user_input_answer_values(question, draft);
    if values.is_empty() {
        "unanswered".to_string()
    } else {
        values.join(", ")
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

fn top_centered_rect(area: Rect, width: u16, height: u16, top_margin: u16) -> Rect {
    let width = adaptive_modal_width(area.width, width);
    let height = adaptive_modal_height(area.height, height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let max_margin = area.height.saturating_sub(height);
    let y = area.y + min(top_margin, max_margin);
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
    footer: Rect,
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

fn transcript_surface_layout(area: Rect, footer_height: u16) -> TranscriptSurfaceLayout {
    let header_height = min(
        transcript_surface_header_height(area.height),
        area.height.saturating_sub(1),
    );
    let footer_height = min(
        footer_height,
        area.height.saturating_sub(header_height).saturating_sub(1),
    );
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(1),
            Constraint::Length(footer_height),
        ])
        .split(area);

    TranscriptSurfaceLayout {
        header: split[0],
        body: inset_rect(split[1], 1, 0),
        footer: inset_rect(split[2], 1, 0),
    }
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
