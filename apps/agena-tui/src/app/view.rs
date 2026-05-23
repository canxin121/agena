use super::*;

#[derive(Clone, Copy)]
enum SurfaceMode {
    Overlay,
    Route,
}

impl SurfaceMode {
    fn outer_width(self, area: Rect, target_width: u16) -> u16 {
        match self {
            Self::Overlay => adaptive_modal_width(area.width, target_width),
            Self::Route => area.width,
        }
    }

    fn content_width(self, area: Rect, target_width: u16) -> u16 {
        self.outer_width(area, target_width).saturating_sub(2)
    }

    fn outer_rect(self, area: Rect, target_width: u16, target_height: u16) -> Rect {
        match self {
            Self::Overlay => preferred_overlay_rect(area, target_width, target_height),
            Self::Route => area,
        }
    }
}

impl App {
    pub(super) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if !matches!(self.current_route, Route::Main) {
            let footer_height = self.route_footer_height(area.width, area.height);
            let (body, footer) = if footer_height > 0 && area.height > footer_height {
                let rows = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(area.height.saturating_sub(footer_height)),
                        Constraint::Length(footer_height),
                    ])
                    .split(area);
                (rows[0], Some(rows[1]))
            } else {
                (area, None)
            };
            self.layout = LayoutCache::default();
            self.render_route(frame, body);
            if let Some(footer) = footer {
                self.render_transcript_footer_row(frame, footer);
            }
            return;
        }

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

    fn route_footer_height(&self, width: u16, total_height: u16) -> u16 {
        if total_height <= 1 {
            return 0;
        }
        let line_count = self.transcript_footer_lines(width).len();
        if line_count == 0 {
            0
        } else {
            min(2, line_count as u16)
        }
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
                    let mut rendered_line =
                        if !line_has_match && self.transcript.search_query.trim().is_empty() {
                            line.rich_line.clone().unwrap_or_else(|| {
                                Line::from(Span::styled(line.text.clone(), line.style))
                            })
                        } else {
                            highlight_search_line(
                                line.text.as_str(),
                                line.style,
                                self.transcript.search_query.as_str(),
                                line_is_active,
                                line_has_match,
                            )
                        };
                    if self.focus == Focus::Transcript && idx == self.transcript.cursor_line {
                        rendered_line = apply_line_highlight(rendered_line);
                    }
                    rendered_line
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
        vec![self.main_surface_mode_label().to_string()]
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
        let (status_area, composer_area) = if status_rows > 0 && area.height > status_rows {
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(status_rows),
                    Constraint::Length(area.height.saturating_sub(status_rows)),
                ])
                .split(area);
            (Some(rows[0]), rows[1])
        } else {
            (None, area)
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
        let normalized = trim_empty_line_edges(sanitized.as_str());
        if normalized.is_empty() {
            return Vec::new();
        }
        let available = width.max(1) as usize;
        let options = textwrap::Options::new(available)
            .break_words(false)
            .word_splitter(textwrap::WordSplitter::NoHyphenation);

        normalized
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
        if self.transcript.loading_initial {
            parts.push(ui_text::t(&self.i18n, "transcript-header-loading"));
        } else if self.transcript.loading_older {
            parts.push(ui_text::t(&self.i18n, "transcript-header-loading-older"));
        }
        if !self.transcript.search_query.trim().is_empty() {
            parts.push(ui_text::transcript_search_summary(
                &self.i18n,
                self.transcript.search_query.as_str(),
                self.transcript.current_search_match_number(),
                self.transcript.current_search_match_count(),
            ));
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
            let (permission_count, user_input_count) =
                pending_interactive_counts_for_execution(execution);
            if user_input_count > 0 {
                parts.push(self.i18n.text_args(
                    "composer-status-pending-user-input",
                    &crate::fl_args!(
                        "count" => user_input_count as i64,
                    ),
                ));
            }
            if permission_count > 0 {
                parts.push(self.i18n.text_args(
                    "composer-status-pending-approval",
                    &crate::fl_args!(
                        "count" => permission_count as i64,
                    ),
                ));
            }
        }
        if self.has_suppressed_pending_interactive_overlay() {
            parts.push(ui_text::t(
                &self.i18n,
                "composer-status-hidden-pending-dialog",
            ));
        }
        parts
    }

    fn main_surface_mode_label(&self) -> &'static str {
        if self.focus == Focus::Composer {
            "INSERT"
        } else {
            "VIEW"
        }
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
            Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                self.render_line_overlay(frame, area, dialog);
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
                self.render_session_search_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
            Overlay::Picker(dialog) => {
                self.render_picker_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
            Overlay::SessionModelChooser(dialog) => {
                self.render_session_model_chooser_overlay(
                    frame,
                    area,
                    dialog,
                    SurfaceMode::Overlay,
                );
            }
            Overlay::Timeline(dialog) => {
                self.render_timeline_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
            Overlay::ProviderStudio(dialog) => {
                self.render_provider_studio_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
            Overlay::ModelCatalogStudio(dialog) => {
                self.render_model_catalog_studio_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
        }
    }

    fn render_route(&self, frame: &mut Frame, area: Rect) {
        match &self.current_route {
            Route::Main => {}
            Route::Help(dialog) => {
                self.render_help_overlay(frame, area, dialog, SurfaceMode::Route)
            }
            Route::SettingsStudio(dialog) => {
                self.render_settings_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::AgentStudio(dialog) => {
                self.render_agent_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::AgentPermissionStudio(dialog) => {
                self.render_agent_permission_studio_overlay(
                    frame,
                    area,
                    dialog,
                    SurfaceMode::Route,
                );
            }
            Route::SessionSearch(dialog) => {
                self.render_session_search_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::Picker(dialog) => {
                self.render_picker_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::SessionModelChooser(dialog) => {
                self.render_session_model_chooser_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::Timeline(dialog) => {
                self.render_timeline_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::PluginInspector(dialog) => {
                self.render_plugin_inspector_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::ProviderStudio(dialog) => {
                self.render_provider_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::ModelCatalogStudio(dialog) => {
                self.render_model_catalog_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
        }
        self.render_overlay(frame, area);
    }

    fn render_line_overlay(&self, frame: &mut Frame, area: Rect, dialog: &LineInputOverlay) {
        let target_width = adaptive_modal_width(area.width, 88);
        let prompt_height =
            wrapped_text_height(dialog.prompt.as_str(), target_width.saturating_sub(2)).clamp(1, 2);
        let area = preferred_overlay_rect(area, 88, prompt_height.saturating_add(6));
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(prompt_height),
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
        let target_width = adaptive_modal_width(area.width, 82);
        let prompt_height =
            overlay_text_height(dialog.prompt.as_str(), target_width.saturating_sub(2), 1, 2);
        let help_height = overlay_text_height(
            permission_rule_edit_help().as_str(),
            target_width.saturating_sub(2),
            3,
            6,
        );
        let preview_height = overlay_text_height(
            render_permission_rule_preview(dialog.input.text()).as_str(),
            target_width.saturating_sub(2),
            2,
            8,
        );
        let area = preferred_overlay_rect(
            area,
            82,
            prompt_height
                .saturating_add(help_height)
                .saturating_add(3)
                .saturating_add(preview_height),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(prompt_height),
                Constraint::Length(help_height),
                Constraint::Length(3),
                Constraint::Min(preview_height),
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
        let target_width = adaptive_modal_width(area.width, 88);
        let prompt_height = overlay_text_height(
            ui_text::t(&self.i18n, "overlay-attach-prompt").as_str(),
            target_width.saturating_sub(2),
            1,
            2,
        );
        let footer_height = overlay_text_height(
            ui_text::t(&self.i18n, "overlay-attach-footer").as_str(),
            target_width.saturating_sub(2),
            1,
            2,
        );
        let list_height = list_panel_height(dialog.results.len().max(1), 1, 4, 10);
        let area = preferred_overlay_rect(
            area,
            88,
            prompt_height
                .saturating_add(3)
                .saturating_add(list_height)
                .saturating_add(footer_height),
        );
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
                Constraint::Length(prompt_height),
                Constraint::Length(3),
                Constraint::Min(list_height),
                Constraint::Length(footer_height),
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
        let body_height =
            permission_overlay_body_height(dialog, adaptive_modal_width(area.width, 84))
                .clamp(4, 14);
        let choices = permission_overlay_choices(&self.i18n);
        let choices_height = list_panel_height(choices.len(), 1, 4, 8);
        let area = preferred_overlay_rect(
            area,
            84,
            body_height.saturating_add(choices_height).saturating_add(1),
        );
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
                Constraint::Min(body_height),
                Constraint::Length(choices_height),
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
            Paragraph::new(format!(
                "{} · e edit rule",
                ui_text::t(&self.i18n, "overlay-permission-footer")
            )),
            rows[2],
        );
    }

    fn render_user_input_overlay(&self, frame: &mut Frame, area: Rect, dialog: &UserInputOverlay) {
        let target_width = adaptive_modal_width(area.width, 92);
        let nav_panel_height = user_input_nav_panel_height();
        let footer_review = "j/k choose question  e edit  enter submit  esc close  ctrl+d cancel";
        let footer_question = "j/k move  tab next  shift+tab prev  space select/toggle  enter choose/next  ctrl+d cancel";
        let height = if dialog.screen == UserInputOverlayScreen::Review {
            let summary_height = user_input_review_summary_height(dialog, target_width);
            nav_panel_height
                .saturating_add(summary_height)
                .saturating_add(wrapped_text_height(
                    footer_review,
                    target_width.saturating_sub(2),
                ))
        } else if let Some(question) = dialog.request.questions.get(dialog.selected_question) {
            let prompt_height = user_input_prompt_panel_height(question, target_width);
            let choices_height = user_input_choices_panel_height(question, target_width);
            let custom_height = if question.allow_custom { 3 } else { 0 };
            nav_panel_height
                .saturating_add(prompt_height)
                .saturating_add(choices_height)
                .saturating_add(custom_height)
                .saturating_add(wrapped_text_height(
                    footer_question,
                    target_width.saturating_sub(2),
                ))
        } else {
            12
        };
        let area = preferred_overlay_rect(area, 92, height);
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
            let summary_height = user_input_review_summary_height(dialog, inner.width);
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(nav_panel_height),
                    Constraint::Length(summary_height),
                    Constraint::Length(wrapped_text_height(
                        footer_review,
                        inner.width.saturating_sub(2),
                    )),
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
                        user_input_review_answer_preview(values.as_slice())
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
        let prompt_panel_height = user_input_prompt_panel_height(question, inner.width);
        let choices_panel_height = user_input_choices_panel_height(question, inner.width);
        let custom_height = if question.allow_custom { 3 } else { 0 };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(nav_panel_height),
                Constraint::Length(prompt_panel_height),
                Constraint::Length(choices_panel_height),
                Constraint::Length(custom_height),
                Constraint::Length(wrapped_text_height(
                    footer_question,
                    inner.width.saturating_sub(2),
                )),
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
                        user_input_option_description_preview(
                            option.description.as_str(),
                            rows[2].width.saturating_sub(6),
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
            let custom_values = user_input_custom_values_preview(
                &draft.custom_values,
                rows[2].width.saturating_sub(6),
            );
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
            option_lines.push(Line::from(vec![
                Span::styled(format!("{prefix} "), custom_style),
                Span::styled("Other", custom_style.add_modifier(Modifier::BOLD)),
            ]));
            option_lines.push(Line::from(Span::styled(
                format!("    {}", custom_values),
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
        let body_width = adaptive_modal_width(area.width, 76).saturating_sub(2);
        let body_height = dialog
            .body_lines
            .iter()
            .map(|line| wrapped_text_height(line.as_str(), body_width))
            .sum::<u16>()
            .clamp(2, 10);
        let area = preferred_overlay_rect(area, 76, body_height.saturating_add(4));
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

    fn render_help_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &HelpOverlay,
        surface: SurfaceMode,
    ) {
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
        let content_width = surface.content_width(area, 132);
        let body_height =
            help_overlay_body_height(text.as_slice(), content_width.saturating_add(2));
        let footer_height = overlay_text_height(
            ui_text::t(&self.i18n, "overlay-help-footer").as_str(),
            content_width,
            1,
            2,
        );
        let area = surface.outer_rect(
            area,
            132,
            body_height.saturating_add(footer_height).saturating_add(2),
        );
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
            .constraints([
                Constraint::Min(body_height),
                Constraint::Length(footer_height),
            ])
            .split(inner);
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
        surface: SurfaceMode,
    ) {
        let content_width = surface.content_width(area, 128);
        let prompt_height = overlay_text_height(dialog.prompt.as_str(), content_width, 1, 2);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let list_height = list_panel_height(
            if dialog.loading || dialog.items.is_empty() {
                1
            } else {
                dialog.items.len()
            },
            if dialog.loading || dialog.items.is_empty() {
                1
            } else {
                2
            },
            5,
            12,
        );
        let area = surface.outer_rect(
            area,
            128,
            prompt_height
                .saturating_add(3)
                .saturating_add(list_height)
                .saturating_add(footer_height),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(prompt_height),
                Constraint::Length(3),
                Constraint::Min(list_height),
                Constraint::Length(footer_height),
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

    fn render_picker_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PickerOverlay,
        surface: SurfaceMode,
    ) {
        let content_width = surface.content_width(area, 120);
        let prompt_height = overlay_text_height(dialog.prompt.as_str(), content_width, 1, 2);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let list_height = list_panel_height(
            if dialog.loading || dialog.items.is_empty() {
                1
            } else {
                dialog.items.len()
            },
            if dialog.loading || dialog.items.is_empty() {
                1
            } else {
                2
            },
            4,
            10,
        );
        let area = surface.outer_rect(
            area,
            120,
            prompt_height
                .saturating_add(3)
                .saturating_add(list_height)
                .saturating_add(footer_height),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(prompt_height),
                Constraint::Length(3),
                Constraint::Min(list_height),
                Constraint::Length(footer_height),
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
        surface: SurfaceMode,
    ) {
        let content_width = surface.content_width(area, 128);
        let prompt_height = overlay_text_height(dialog.prompt.as_str(), content_width, 1, 2);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let visible_items = if dialog.loading || dialog.items.is_empty() {
            1
        } else {
            dialog.page_size.max(1).min(dialog.items.len())
        };
        let list_height = list_panel_height(
            visible_items,
            if dialog.loading || dialog.items.is_empty() {
                1
            } else {
                2
            },
            5,
            12,
        );
        let area = surface.outer_rect(
            area,
            128,
            prompt_height
                .saturating_add(3)
                .saturating_add(list_height)
                .saturating_add(footer_height),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(prompt_height),
                Constraint::Length(3),
                Constraint::Min(list_height),
                Constraint::Length(footer_height),
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
        let target_width = adaptive_modal_width(area.width, 96);
        let prompt_height =
            wrapped_text_height(dialog.prompt.as_str(), target_width.saturating_sub(2)).clamp(1, 3);
        let footer_height =
            wrapped_text_height(dialog.footer.as_str(), target_width.saturating_sub(2)).clamp(1, 2);
        let choice_rows = Self::choice_overlay_rows(dialog);
        let list_height = choice_overlay_rows_height(choice_rows.as_slice());
        let area = preferred_overlay_rect(
            area,
            96,
            3_u16
                .saturating_add(prompt_height)
                .saturating_add(list_height)
                .saturating_add(footer_height),
        );
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
                Constraint::Length(prompt_height),
                Constraint::Min(list_height),
                Constraint::Length(footer_height),
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

    fn render_timeline_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &TimelineOverlay,
        surface: SurfaceMode,
    ) {
        let content_width = surface.content_width(area, 122);
        let prompt_height = overlay_text_height(dialog.prompt.as_str(), content_width, 1, 2);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let content_height = timeline_overlay_content_height(dialog, content_width);
        let area = surface.outer_rect(
            area,
            122,
            prompt_height
                .saturating_add(3)
                .saturating_add(content_height)
                .saturating_add(footer_height),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(prompt_height),
                Constraint::Length(3),
                Constraint::Min(content_height),
                Constraint::Length(footer_height),
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
        let (list_height, detail_height) =
            timeline_overlay_panel_heights(dialog, rows[2].width.saturating_sub(2));
        let split_constraints = adaptive_detail_split(rows[2].width, 40, 46);
        let stacked_constraints = [
            Constraint::Length(list_height),
            Constraint::Length(detail_height),
        ];
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
        let (list_area, detail_area) = if stacked {
            (content[0], content[1])
        } else {
            (
                top_aligned_panel_rect(content[0], list_height),
                top_aligned_panel_rect(content[1], detail_height),
            )
        };

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
        frame.render_stateful_widget(list, list_area, &mut state);

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
            detail_area,
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
        surface: SurfaceMode,
    ) {
        let content_width = surface.content_width(area, 122);
        let prompt_height = overlay_text_height(dialog.prompt.as_str(), content_width, 1, 2);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let content_height = plugin_inspector_content_height(dialog, content_width);
        let area = surface.outer_rect(
            area,
            122,
            prompt_height
                .saturating_add(3)
                .saturating_add(content_height)
                .saturating_add(footer_height),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(prompt_height),
                Constraint::Length(3),
                Constraint::Min(content_height),
                Constraint::Length(footer_height),
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
        let (list_height, detail_height, logs_height) =
            plugin_inspector_panel_heights(dialog, rows[2].width.saturating_sub(2));
        let (list_area, detail_area, logs_area) = if stacked {
            let content =
                top_aligned_vertical_areas(rows[2], &[list_height, detail_height, logs_height]);
            (content[0], content[1], content[2])
        } else {
            let content = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(adaptive_detail_split(rows[2].width, 34, 48))
                .split(rows[2]);
            let right = top_aligned_vertical_areas(content[1], &[detail_height, logs_height]);
            (
                top_aligned_panel_rect(content[0], list_height),
                right[0],
                right[1],
            )
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
        surface: SurfaceMode,
    ) {
        let title_width = surface.outer_width(area, 136);
        let content_width = surface.content_width(area, 136);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let content_height = settings_studio_content_height(dialog, content_width);
        let current_section = dialog.sections.get(dialog.selected_section);
        let section_summary = current_section
            .map(|section| format!("{}  ·  {} item(s)", section.summary, section.items.len()))
            .unwrap_or_else(|| "no settings available".to_string());
        let area = surface.outer_rect(
            area,
            136,
            content_height
                .saturating_add(footer_height)
                .saturating_add(2),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(overlay_title_with_summary(
                dialog.title.as_str(),
                section_summary.as_str(),
                title_width,
            )))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(content_height),
                Constraint::Length(footer_height),
            ])
            .split(inner);

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(60)])
            .split(rows[0]);
        let nav_area = content[0];
        let right_area = content[1];
        let nav_panel_height = list_panel_height(dialog.sections.len().max(1), 2, 4, 12);
        let nav_panel_area = top_aligned_panel_rect(nav_area, nav_panel_height);

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
        frame.render_stateful_widget(nav_list, nav_panel_area, &mut nav_state);

        let section_title = current_section
            .map(|section| section.label.clone())
            .unwrap_or_else(|| "Section".to_string());
        let section_description = current_section
            .map(|section| section.description.clone())
            .unwrap_or_else(|| "No section selected.".to_string());
        let selected_item =
            current_section.and_then(|section| section.items.get(dialog.selected_item));
        let detail_text = match current_section.map(|section| section.id) {
            Some(SettingsStudioSectionId::Agents) => selected_item
                .and_then(|item| match &item.action {
                    SettingsPickerAction::OpenAgent(agent) => {
                        Some(settings_studio_agent_detail_text(
                            agent,
                            dialog.default_agent_name.as_deref(),
                        ))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "Select an agent to inspect or edit it.".to_string()),
            _ => selected_item
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
                .unwrap_or_else(|| {
                    "Select a section and an option to inspect or edit it.".to_string()
                }),
        };
        let section_panel_height =
            bordered_paragraph_height(section_description.as_str(), right_area.width, 1, 3);
        let item_list_height = list_panel_height(
            current_section
                .map(|section| section.items.len())
                .unwrap_or(0)
                .max(1),
            if current_section
                .map(|section| section.items.is_empty())
                .unwrap_or(true)
            {
                1
            } else {
                2
            },
            4,
            12,
        );
        let detail_panel_height =
            bordered_paragraph_height(detail_text.as_str(), right_area.width, 3, 6);
        let right_rows = top_aligned_vertical_areas(
            right_area,
            &[section_panel_height, item_list_height, detail_panel_height],
        );

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
            rows[1],
        );
    }

    fn render_agent_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &AgentStudioOverlay,
        surface: SurfaceMode,
    ) {
        let title_width = surface.outer_width(area, 138);
        let content_width = surface.content_width(area, 138);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let content_height = agent_studio_content_height(dialog, content_width);
        let title_summary = format!(
            "{} · {}",
            dialog.profile.scope.as_str(),
            if dialog.editable {
                "config-owned"
            } else {
                "file-backed"
            }
        );
        let area = surface.outer_rect(
            area,
            138,
            content_height
                .saturating_add(footer_height)
                .saturating_add(2),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(overlay_title_with_summary(
                dialog.title.as_str(),
                title_summary.as_str(),
                title_width,
            )))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(content_height),
                Constraint::Length(footer_height),
            ])
            .split(inner);

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(36), Constraint::Min(60)])
            .split(rows[0]);
        let list_area = top_aligned_panel_rect(
            content[0],
            list_panel_height(dialog.items.len().max(1), 2, 6, 16),
        );
        let right_area = content[1];
        let overview_text = agent_studio_overview_text(
            &dialog.profile,
            dialog.default_agent_name.as_deref(),
            dialog.editable,
        );
        let selected_item = dialog.items.get(dialog.selected);
        let detail_text = selected_item
            .map(|item| {
                agent_studio_item_detail_text(
                    &dialog.profile,
                    item,
                    dialog.editable,
                    dialog.default_agent_name.as_deref(),
                )
            })
            .unwrap_or_else(|| "Select a field to inspect or edit it.".to_string());
        let overview_height =
            bordered_paragraph_height(overview_text.as_str(), right_area.width, 2, 5);
        let detail_height =
            bordered_paragraph_height(detail_text.as_str(), right_area.width, 3, 10);
        let right_rows = top_aligned_vertical_areas(right_area, &[overview_height, detail_height]);

        let list_items = dialog
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
            .collect::<Vec<_>>();
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(" Agent Fields "))
                    .borders(Borders::ALL),
            )
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, list_area, &mut state);

        frame.render_widget(
            Paragraph::new(sanitize_display_text(overview_text))
                .block(
                    Block::default()
                        .title(sanitize_display_text(" Overview "))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right_rows[0],
        );
        frame.render_widget(
            Paragraph::new(sanitize_display_text(detail_text))
                .block(
                    Block::default()
                        .title(sanitize_display_text(" Details "))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right_rows[1],
        );
        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
            rows[1],
        );

        if let Some(editor) = dialog.editor.as_ref() {
            self.render_workbench_editor(
                frame,
                area,
                &editor.title,
                &editor.prompt,
                &editor.footer,
                &editor.input,
                editor.multiline,
            );
        }
    }

    fn render_agent_permission_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &AgentPermissionStudioOverlay,
        surface: SurfaceMode,
    ) {
        let title_width = surface.outer_width(area, 132);
        let content_width = surface.content_width(area, 132);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let content_height = agent_permission_studio_content_height(dialog, content_width);
        let title_summary = format!(
            "{} · {}",
            dialog.profile.scope.as_str(),
            if dialog.editable {
                "editable"
            } else {
                "read-only"
            }
        );
        let area = surface.outer_rect(
            area,
            132,
            content_height
                .saturating_add(footer_height)
                .saturating_add(2),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(overlay_title_with_summary(
                dialog.title.as_str(),
                title_summary.as_str(),
                title_width,
            )))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(content_height),
                Constraint::Length(footer_height),
            ])
            .split(inner);

        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(34), Constraint::Min(58)])
            .split(rows[0]);
        let list_area = top_aligned_panel_rect(
            content[0],
            list_panel_height(dialog.items.len().max(1), 2, 6, 16),
        );
        let right_area = content[1];
        let overview_text = format!(
            "Agent: {}\nSource: {}\nSummary: {}",
            dialog.profile.name,
            agent_profile_source_label(&dialog.profile),
            agent_permission_summary(&dialog.profile.frontmatter.permission),
        );
        let selected_item = dialog.items.get(dialog.selected);
        let detail_text = selected_item
            .map(|item| {
                agent_permission_studio_item_detail_text(&dialog.profile, item, dialog.editable)
            })
            .unwrap_or_else(|| "Select a permission section to inspect or edit it.".to_string());
        let overview_height =
            bordered_paragraph_height(overview_text.as_str(), right_area.width, 2, 5);
        let detail_height =
            bordered_paragraph_height(detail_text.as_str(), right_area.width, 4, 12);
        let right_rows = top_aligned_vertical_areas(right_area, &[overview_height, detail_height]);

        let list_items = dialog
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
            .collect::<Vec<_>>();
        let list = List::new(list_items)
            .block(
                Block::default()
                    .title(sanitize_display_text(" Permission Sections "))
                    .borders(Borders::ALL),
            )
            .highlight_style(selection_highlight_style())
            .highlight_symbol(">> ");
        let mut state = ListState::default();
        state.select((!dialog.items.is_empty()).then_some(dialog.selected));
        frame.render_stateful_widget(list, list_area, &mut state);

        frame.render_widget(
            Paragraph::new(sanitize_display_text(overview_text))
                .block(
                    Block::default()
                        .title(sanitize_display_text(" Overview "))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right_rows[0],
        );
        frame.render_widget(
            Paragraph::new(sanitize_display_text(detail_text))
                .block(
                    Block::default()
                        .title(sanitize_display_text(" Details "))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            right_rows[1],
        );
        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
            rows[1],
        );

        if let Some(editor) = dialog.editor.as_ref() {
            self.render_workbench_editor(
                frame,
                area,
                &editor.title,
                &editor.prompt,
                &editor.footer,
                &editor.input,
                editor.multiline,
            );
        }
    }

    fn render_workbench_editor(
        &self,
        frame: &mut Frame,
        area: Rect,
        title: &str,
        prompt: &str,
        footer: &str,
        input: &Editor,
        multiline: bool,
    ) {
        let target_width = if multiline { 96 } else { 78 };
        let prompt_height = wrapped_text_height(
            prompt,
            adaptive_modal_width(area.width, target_width).saturating_sub(2),
        )
        .clamp(1, 3);
        let footer_height = wrapped_text_height(
            footer,
            adaptive_modal_width(area.width, target_width).saturating_sub(2),
        )
        .clamp(1, 2);
        let input_height = editor_input_panel_height(input, multiline);
        let editor_height = prompt_height
            .saturating_add(footer_height)
            .saturating_add(input_height)
            .saturating_add(2);
        let area = preferred_overlay_rect(area, target_width, editor_height);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(prompt_height),
                Constraint::Length(input_height),
                Constraint::Length(footer_height),
            ])
            .split(inner);
        frame.render_widget(Paragraph::new(sanitize_display_text(prompt)), rows[0]);
        let input_view = input.render_view(
            rows[1].width.saturating_sub(2),
            rows[1].height.saturating_sub(2).max(1),
        );
        frame.render_widget(
            Paragraph::new(Text::from(input_view.lines.clone()))
                .block(Block::default().borders(Borders::ALL)),
            rows[1],
        );
        frame.render_widget(Paragraph::new(sanitize_display_text(footer)), rows[2]);
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
        surface: SurfaceMode,
    ) {
        let content_width = surface.content_width(area, 140);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let content_height = provider_studio_content_height(dialog, content_width);
        let area = surface.outer_rect(
            area,
            122,
            content_height
                .saturating_add(footer_height)
                .saturating_add(2),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(format!(" {} ", dialog.title)))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(content_height),
                Constraint::Length(footer_height),
            ])
            .split(inner);

        let right_area = if dialog.show_provider_list {
            let content = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(28), Constraint::Min(54)])
                .split(rows[0]);
            let providers_area = content[0];
            let providers_panel_area = top_aligned_panel_rect(
                providers_area,
                list_panel_height(dialog.providers.len().max(1), 2, 4, 12),
            );
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
            frame.render_stateful_widget(provider_list, providers_panel_area, &mut provider_state);
            content[1]
        } else {
            rows[0]
        };
        let draft_fields = provider_studio_visible_fields(dialog);
        let draft_panel_height = provider_studio_draft_panel_height(dialog, right_area.width);
        let lower_panel_height = provider_studio_lower_content_height(dialog, right_area.width);
        let right_rows =
            top_aligned_vertical_areas(right_area, &[draft_panel_height, lower_panel_height]);

        let draft_lines = draft_fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                let display = provider_studio_main_field_display(dialog, *field);
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

        let (adapter_height, model_height) =
            provider_studio_adapter_model_heights(dialog, right_rows[1].width);
        let adapter_models_split = if should_stack_detail_layout(right_rows[1].width, 24, 28) {
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(adapter_height),
                    Constraint::Length(model_height),
                ])
                .split(right_rows[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints(adaptive_detail_split(right_rows[1].width, 24, 28))
                .split(right_rows[1])
        };
        let (adapters_area, models_area) =
            if should_stack_detail_layout(right_rows[1].width, 24, 28) {
                (adapter_models_split[0], adapter_models_split[1])
            } else {
                (
                    top_aligned_panel_rect(adapter_models_split[0], adapter_height),
                    top_aligned_panel_rect(adapter_models_split[1], model_height),
                )
            };

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
            rows[1],
        );

        if let Some(detail_page) = dialog.detail_page.as_ref() {
            let detail_lines = provider_studio_detail_page_lines(dialog);
            let detail_height =
                provider_studio_detail_page_height(dialog, adaptive_modal_width(area.width, 92));
            let detail_area = preferred_overlay_rect(area, 92, detail_height);
            frame.render_widget(Clear, detail_area);
            let detail_block = Block::default()
                .title(sanitize_display_text(format!(" {} ", detail_page.title)))
                .borders(Borders::ALL);
            let detail_inner = detail_block.inner(detail_area);
            frame.render_widget(detail_block, detail_area);
            let footer_height =
                wrapped_text_height(detail_page.footer.as_str(), detail_inner.width.max(1))
                    .clamp(1, 2);
            let detail_rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(detail_inner.height.saturating_sub(footer_height)),
                    Constraint::Length(footer_height),
                ])
                .split(detail_inner);
            let detail_fields = provider_studio_detail_fields(dialog);
            let auth_state_line_count = provider_studio_auth_state_lines(dialog).len();
            let lines = detail_lines
                .into_iter()
                .enumerate()
                .map(|(index, line)| {
                    if index == 0 {
                        return Line::from(vec![
                            Span::styled(
                                format!(
                                    "{:>16}",
                                    provider_studio_field_label(ProviderStudioField::AuthStatus)
                                ),
                                Style::default().fg(Color::DarkGray),
                            ),
                            Span::raw("  "),
                            Span::styled(
                                sanitize_display_text(line),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]);
                    }
                    if index <= auth_state_line_count {
                        return Line::from(Span::styled(
                            sanitize_display_text(line),
                            Style::default().fg(Color::DarkGray),
                        ));
                    }
                    let field_index = index.saturating_sub(auth_state_line_count + 1);
                    let Some(field) = detail_fields.get(field_index).copied() else {
                        return Line::from(sanitize_display_text(line));
                    };
                    let selected =
                        dialog.editor.is_none() && detail_page.selected_field == field_index;
                    let label_style = if selected {
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD)
                    } else if provider_studio_field_editable(dialog, field) {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    Line::from(vec![
                        Span::styled(
                            format!("{:>16}", provider_studio_field_label(field)),
                            label_style,
                        ),
                        Span::raw("  "),
                        Span::styled(
                            sanitize_display_text(line),
                            if selected {
                                Style::default()
                                    .fg(Color::Cyan)
                                    .add_modifier(Modifier::BOLD)
                            } else if provider_studio_field_editable(dialog, field) {
                                Style::default()
                            } else {
                                Style::default().fg(Color::DarkGray)
                            },
                        ),
                    ])
                })
                .collect::<Vec<_>>();
            frame.render_widget(
                Paragraph::new(Text::from(lines)).wrap(Wrap { trim: false }),
                detail_rows[0],
            );
            frame.render_widget(
                Paragraph::new(sanitize_display_text(detail_page.footer.as_str())),
                detail_rows[1],
            );
        }

        if let Some(editor) = dialog.editor.as_ref() {
            let target_width = if editor.multiline { 92 } else { 78 };
            let prompt_height = wrapped_text_height(
                editor.prompt.as_str(),
                adaptive_modal_width(area.width, target_width).saturating_sub(2),
            )
            .clamp(1, 3);
            let footer_height = wrapped_text_height(
                editor.footer.as_str(),
                adaptive_modal_width(area.width, target_width).saturating_sub(2),
            )
            .clamp(1, 2);
            let input_height = editor_input_panel_height(&editor.input, editor.multiline);
            let editor_height = prompt_height
                .saturating_add(footer_height)
                .saturating_add(input_height)
                .saturating_add(2);
            let area = preferred_overlay_rect(area, target_width, editor_height);
            frame.render_widget(Clear, area);
            let block = Block::default()
                .title(sanitize_display_text(format!(" {} ", editor.title)))
                .borders(Borders::ALL);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let editor_rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(prompt_height),
                    Constraint::Length(input_height),
                    Constraint::Length(footer_height),
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
        surface: SurfaceMode,
    ) {
        let title_width = surface.outer_width(area, 136);
        let content_width = surface.content_width(area, 136);
        let footer_height = overlay_text_height(dialog.footer.as_str(), content_width, 1, 2);
        let content_height = model_catalog_content_height(dialog, content_width);
        let summary = format!(
            "query {}  ·  page {}-{} / {}  ·  {} entries",
            if dialog.query.trim().is_empty() {
                "<all>".to_owned()
            } else {
                dialog.query.clone()
            },
            dialog.offset.saturating_add(1),
            dialog.offset.saturating_add(dialog.items.len()),
            dialog.total,
            dialog.summary.entry_count,
        );
        let area = surface.outer_rect(
            area,
            136,
            content_height
                .saturating_add(footer_height)
                .saturating_add(2),
        );
        frame.render_widget(Clear, area);
        let block = Block::default()
            .title(sanitize_display_text(overlay_title_with_summary(
                dialog.title.as_str(),
                summary.as_str(),
                title_width,
            )))
            .borders(Borders::ALL);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(content_height),
                Constraint::Length(footer_height),
            ])
            .split(inner);

        let stacked = should_stack_detail_layout(rows[0].width, 48, 34);
        let (list_height, detail_height) =
            model_catalog_panel_heights(dialog, rows[0].width.saturating_sub(2));
        let (list_area, detail_area) = if stacked {
            let (list_height, detail_height) =
                model_catalog_panel_heights(dialog, rows[0].width.saturating_sub(2));
            let content = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(list_height),
                    Constraint::Length(detail_height),
                ])
                .split(rows[0]);
            (content[0], content[1])
        } else {
            let content = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(adaptive_detail_split(rows[0].width, 48, 34))
                .split(rows[0]);
            (
                top_aligned_panel_rect(content[0], list_height),
                top_aligned_panel_rect(content[1], detail_height),
            )
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
                            sanitize_display_text(entry.display_name.clone().unwrap_or_else(
                                || entry.origin.clone().unwrap_or_else(|| "unknown".to_owned()),
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
        frame.render_stateful_widget(catalog_list, list_area, &mut list_state);

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
                    format!("source: {:?}", entry.source),
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
            detail_area,
        );

        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str())),
            rows[1],
        );

        if let Some(editor) = dialog.editor.as_ref() {
            let prompt_height = wrapped_text_height(
                editor.prompt.as_str(),
                adaptive_modal_width(area.width, 78).saturating_sub(2),
            )
            .clamp(1, 2);
            let footer_height = wrapped_text_height(
                ui_text::t(&self.i18n, "overlay-provider-studio-edit-footer").as_str(),
                adaptive_modal_width(area.width, 78).saturating_sub(2),
            )
            .clamp(1, 2);
            let area = preferred_overlay_rect(
                area,
                78,
                prompt_height
                    .saturating_add(footer_height)
                    .saturating_add(5),
            );
            frame.render_widget(Clear, area);
            let block = Block::default()
                .title(sanitize_display_text(format!(" {} ", editor.title)))
                .borders(Borders::ALL);
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let editor_rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(prompt_height),
                    Constraint::Length(3),
                    Constraint::Length(footer_height),
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

fn apply_line_highlight(line: Line<'static>) -> Line<'static> {
    let style = selection_highlight_style();
    let spans = line
        .spans
        .into_iter()
        .map(|span| {
            let mut span_style = span.style;
            if style.fg.is_some() {
                span_style.fg = style.fg;
            }
            if style.bg.is_some() {
                span_style.bg = style.bg;
            }
            span_style = span_style.add_modifier(style.add_modifier);
            span_style = span_style.remove_modifier(style.sub_modifier);
            Span::styled(span.content, span_style)
        })
        .collect::<Vec<_>>();
    Line::from(spans)
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

fn preferred_overlay_rect(area: Rect, width: u16, height: u16) -> Rect {
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

fn top_aligned_vertical_areas(area: Rect, heights: &[u16]) -> Vec<Rect> {
    let mut constraints = heights
        .iter()
        .copied()
        .map(Constraint::Length)
        .collect::<Vec<_>>();
    constraints.push(Constraint::Min(0));
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .iter()
        .take(heights.len())
        .copied()
        .collect()
}

fn top_aligned_panel_rect(area: Rect, panel_height: u16) -> Rect {
    top_aligned_vertical_areas(area, &[panel_height])
        .into_iter()
        .next()
        .unwrap_or(area)
}

fn overlay_title_with_summary(title: &str, summary: &str, width: u16) -> String {
    let title = sanitize_display_text(title).trim().to_string();
    let summary = sanitize_display_text(summary).trim().to_string();
    if summary.is_empty() {
        return format!(" {title} ");
    }

    let max_summary_width = width
        .saturating_sub(UnicodeWidthStr::width(title.as_str()) as u16)
        .saturating_sub(7) as usize;
    if max_summary_width < 8 {
        format!(" {title} ")
    } else {
        format!(
            " {} · {} ",
            title,
            truncate_display_text(summary.as_str(), max_summary_width)
        )
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

fn trim_empty_line_edges(text: &str) -> String {
    let lines = text.split('\n').collect::<Vec<_>>();
    let Some(first_non_empty) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let last_non_empty = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .unwrap_or(first_non_empty);
    lines[first_non_empty..=last_non_empty].join("\n")
}

pub(super) fn adaptive_modal_width(total_width: u16, target: u16) -> u16 {
    let max_width = total_width.saturating_sub(2).max(1);
    if total_width <= 72 {
        min(max_width, max(36, target))
    } else if total_width <= 96 {
        min(max_width, max(44, target.saturating_sub(10)))
    } else {
        min(target, max_width)
    }
}

pub(super) fn adaptive_modal_height(total_height: u16, target: u16) -> u16 {
    let max_height = total_height.saturating_sub(2).max(1);
    if total_height <= 18 {
        min(max_height, max(6, target))
    } else if total_height <= 28 {
        min(max_height, max(8, target))
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
        let [left_pct, right_pct] = proportional_percentages(left_min, right_min);
        [
            Constraint::Percentage(left_pct),
            Constraint::Percentage(right_pct),
        ]
    }
}

pub(super) fn should_stack_detail_layout(total_width: u16, left_min: u16, right_min: u16) -> bool {
    let available = total_width.saturating_sub(2);
    available < left_min.saturating_add(right_min).saturating_add(8)
}

fn estimated_horizontal_panel_widths(
    total_width: u16,
    left_min: u16,
    right_min: u16,
) -> (u16, u16) {
    if should_stack_detail_layout(total_width, left_min, right_min) {
        return (total_width.max(1), total_width.max(1));
    }

    let [left_pct, _right_pct] = proportional_percentages(left_min, right_min);
    let left_width = ((u32::from(total_width) * u32::from(left_pct)) / 100) as u16;
    let right_width = total_width.saturating_sub(left_width);
    (left_width.max(1), right_width.max(1))
}

fn proportional_percentages(first: u16, second: u16) -> [u16; 2] {
    let total = u32::from(first.max(1)).saturating_add(u32::from(second.max(1)));
    let first_pct = ((u32::from(first.max(1)) * 100) / total).clamp(1, 99) as u16;
    [first_pct, 100_u16.saturating_sub(first_pct)]
}

fn wrapped_text_height(text: &str, width: u16) -> u16 {
    let usable_width = usize::from(width.max(1));
    text.lines()
        .map(|line| {
            let display_width = UnicodeWidthStr::width(line);
            let rows = display_width.max(1).div_ceil(usable_width);
            u16::try_from(rows).unwrap_or(u16::MAX)
        })
        .sum::<u16>()
        .max(1)
}

fn bordered_paragraph_height(text: &str, width: u16, min_body: u16, max_body: u16) -> u16 {
    wrapped_text_height(text, width.saturating_sub(2))
        .clamp(min_body, max_body)
        .saturating_add(2)
}

fn overlay_text_height(text: &str, width: u16, min_body: u16, max_body: u16) -> u16 {
    wrapped_text_height(text, width).clamp(min_body, max_body)
}

fn list_panel_height(
    entry_count: usize,
    lines_per_entry: u16,
    min_body: u16,
    max_body: u16,
) -> u16 {
    let natural_lines = u16::try_from(entry_count)
        .unwrap_or(u16::MAX)
        .saturating_mul(lines_per_entry)
        .max(1);
    let relaxed_min_body =
        min_body.min(natural_lines.saturating_add(lines_per_entry.saturating_sub(1)));
    let lines = natural_lines.clamp(relaxed_min_body, max_body);
    lines.saturating_add(2)
}

fn help_overlay_body_height(lines: &[Line<'static>], width: u16) -> u16 {
    let body_width = width.saturating_sub(2);
    lines
        .iter()
        .map(|line| {
            let text = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            wrapped_text_height(text.as_str(), body_width)
        })
        .sum::<u16>()
        .clamp(8, 22)
}

fn settings_studio_content_height(dialog: &SettingsStudioOverlay, width: u16) -> u16 {
    let nav_height = list_panel_height(dialog.sections.len().max(1), 2, 4, 12);
    let right_width = width.saturating_sub(28).max(1);
    let current_section = dialog.sections.get(dialog.selected_section);
    let selected_item = current_section.and_then(|section| section.items.get(dialog.selected_item));
    let section_description = current_section
        .map(|section| section.description.as_str())
        .unwrap_or("No section selected.");
    let detail_text = match current_section.map(|section| section.id) {
        Some(SettingsStudioSectionId::Agents) => selected_item
            .and_then(|item| match &item.action {
                SettingsPickerAction::OpenAgent(agent) => Some(settings_studio_agent_detail_text(
                    agent,
                    dialog.default_agent_name.as_deref(),
                )),
                _ => None,
            })
            .unwrap_or_else(|| "Select an agent to inspect or edit it.".to_string()),
        _ => selected_item
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
            .unwrap_or_else(|| "Select a section and an option to inspect or edit it.".to_string()),
    };
    let section_panel_height = bordered_paragraph_height(section_description, right_width, 1, 3);
    let item_list_height = list_panel_height(
        current_section
            .map(|section| section.items.len())
            .unwrap_or(0)
            .max(1),
        if current_section
            .map(|section| section.items.is_empty())
            .unwrap_or(true)
        {
            1
        } else {
            2
        },
        4,
        12,
    );
    let detail_panel_height = bordered_paragraph_height(detail_text.as_str(), right_width, 3, 6);
    max(
        nav_height,
        section_panel_height
            .saturating_add(item_list_height)
            .saturating_add(detail_panel_height),
    )
}

fn agent_studio_content_height(dialog: &AgentStudioOverlay, width: u16) -> u16 {
    let left_height = list_panel_height(dialog.items.len().max(1), 2, 6, 16);
    let right_width = width.saturating_sub(36).max(1);
    let overview_height = bordered_paragraph_height(
        agent_studio_overview_text(
            &dialog.profile,
            dialog.default_agent_name.as_deref(),
            dialog.editable,
        )
        .as_str(),
        right_width,
        2,
        5,
    );
    let detail_text = dialog
        .items
        .get(dialog.selected)
        .map(|item| {
            agent_studio_item_detail_text(
                &dialog.profile,
                item,
                dialog.editable,
                dialog.default_agent_name.as_deref(),
            )
        })
        .unwrap_or_else(|| "Select a field to inspect or edit it.".to_string());
    let detail_height = bordered_paragraph_height(detail_text.as_str(), right_width, 3, 10);
    max(left_height, overview_height.saturating_add(detail_height))
}

fn agent_permission_studio_content_height(
    dialog: &AgentPermissionStudioOverlay,
    width: u16,
) -> u16 {
    let left_height = list_panel_height(dialog.items.len().max(1), 2, 6, 16);
    let right_width = width.saturating_sub(34).max(1);
    let overview_text = format!(
        "Agent: {}\nSource: {}\nSummary: {}",
        dialog.profile.name,
        agent_profile_source_label(&dialog.profile),
        agent_permission_summary(&dialog.profile.frontmatter.permission),
    );
    let overview_height = bordered_paragraph_height(overview_text.as_str(), right_width, 2, 5);
    let detail_text = dialog
        .items
        .get(dialog.selected)
        .map(|item| {
            agent_permission_studio_item_detail_text(&dialog.profile, item, dialog.editable)
        })
        .unwrap_or_else(|| "Select a permission section to inspect or edit it.".to_string());
    let detail_height = bordered_paragraph_height(detail_text.as_str(), right_width, 4, 12);
    max(left_height, overview_height.saturating_add(detail_height))
}

fn provider_studio_content_height(dialog: &ProviderStudioOverlay, width: u16) -> u16 {
    let right_width = if dialog.show_provider_list {
        width.saturating_sub(28).max(44)
    } else {
        width.max(44)
    };
    let draft_panel_height = provider_studio_draft_panel_height(dialog, right_width);
    let lower_height = provider_studio_lower_content_height(dialog, right_width);
    let right_total = draft_panel_height.saturating_add(lower_height);
    if dialog.show_provider_list {
        max(
            list_panel_height(dialog.providers.len().max(1), 2, 4, 12),
            right_total,
        )
    } else {
        right_total
    }
}

fn provider_studio_lower_content_height(dialog: &ProviderStudioOverlay, width: u16) -> u16 {
    let (adapter_height, model_height) = provider_studio_adapter_model_heights(dialog, width);
    if should_stack_detail_layout(width, 24, 28) {
        adapter_height.saturating_add(model_height)
    } else {
        max(adapter_height, model_height)
    }
}

fn model_catalog_content_height(dialog: &ModelCatalogStudioOverlay, width: u16) -> u16 {
    let (list_height, detail_height) = model_catalog_panel_heights(dialog, width);
    let stacked = should_stack_detail_layout(width, 48, 34);
    if stacked {
        list_height.saturating_add(detail_height)
    } else {
        max(list_height, detail_height)
    }
}

fn user_input_nav_panel_height() -> u16 {
    4
}

fn user_input_review_summary_height(dialog: &UserInputOverlay, width: u16) -> u16 {
    let content_width = width.saturating_sub(4).max(1);
    let mut lines = wrapped_text_height("Review your answers before submitting.", content_width);
    for question in &dialog.request.questions {
        let values = dialog
            .answers
            .get(&question.id)
            .map(|draft| user_input_answer_values(question, draft))
            .unwrap_or_default();
        let answered = !values.is_empty();
        let question_line = format!(
            "{} {}",
            if answered { "[x]" } else { "[ ]" },
            sanitize_display_text(user_input_question_label(question))
        );
        let answer_line = format!(
            "    {}",
            user_input_review_answer_preview(values.as_slice())
        );
        lines = lines.saturating_add(wrapped_text_height(question_line.as_str(), content_width));
        lines = lines.saturating_add(wrapped_text_height(answer_line.as_str(), content_width));
    }
    lines.clamp(6, 14).saturating_add(2)
}

fn user_input_prompt_panel_height(question: &UserInputQuestion, width: u16) -> u16 {
    let answer_hint = "Current answer:";
    let content_width = width.saturating_sub(4);
    let body_height = wrapped_text_height(question.question.as_str(), content_width)
        .saturating_add(wrapped_text_height(
            format!(
                "{} · id={}",
                if question.multiple {
                    "Choose one or more"
                } else {
                    "Choose one"
                },
                question.id
            )
            .as_str(),
            content_width,
        ))
        .saturating_add(wrapped_text_height(answer_hint, content_width));
    body_height.clamp(3, 6).saturating_add(2)
}

fn user_input_choices_panel_height(question: &UserInputQuestion, width: u16) -> u16 {
    let line_width = width.saturating_sub(4).max(1);
    let description_width = width.saturating_sub(6);
    let mut lines = 0_u16;
    for option in &question.options {
        let prefix = if question.multiple { "[ ]" } else { "( )" };
        let label_line = format!("{prefix} {}", sanitize_display_text(option.label.as_str()));
        lines = lines.saturating_add(wrapped_text_height(label_line.as_str(), line_width));
        if !option.description.trim().is_empty() {
            let description_line = format!(
                "    {}",
                user_input_option_description_preview(
                    option.description.as_str(),
                    description_width,
                )
            );
            lines =
                lines.saturating_add(wrapped_text_height(description_line.as_str(), line_width));
        }
    }
    if question.allow_custom {
        lines = lines
            .saturating_add(wrapped_text_height("Other", line_width))
            .saturating_add(wrapped_text_height(
                format!(
                    "    {}",
                    user_input_custom_values_preview(&[], description_width)
                )
                .as_str(),
                line_width,
            ));
    }
    lines.clamp(4, 12).saturating_add(2)
}

fn user_input_review_answer_preview(values: &[String]) -> String {
    if values.is_empty() {
        "unanswered".to_string()
    } else {
        truncate_display_text(values.join(", ").as_str(), 72)
    }
}

fn user_input_option_description_preview(description: &str, width: u16) -> String {
    truncate_display_text(
        sanitize_display_text(description).as_str(),
        width.max(1) as usize,
    )
}

fn user_input_custom_values_preview(values: &[String], width: u16) -> String {
    if values.is_empty() {
        "Press Enter or e to type a custom answer".to_string()
    } else {
        truncate_display_text(values.join(", ").as_str(), width.max(1) as usize)
    }
}

fn choice_overlay_rows_height(rows: &[ChoiceRow]) -> u16 {
    if rows.is_empty() {
        return 5;
    }
    let line_count = rows.len().saturating_mul(2);
    u16::try_from(line_count)
        .unwrap_or(u16::MAX)
        .clamp(3, 12)
        .saturating_add(2)
}

fn timeline_overlay_content_height(dialog: &TimelineOverlay, width: u16) -> u16 {
    let (list_height, detail_height) = timeline_overlay_panel_heights(dialog, width);
    let stacked = should_stack_detail_layout(width, 40, 46);
    if stacked {
        list_height.saturating_add(detail_height)
    } else {
        max(list_height, detail_height)
    }
}

fn plugin_inspector_content_height(dialog: &PluginInspectorOverlay, width: u16) -> u16 {
    let (list_height, detail_height, logs_height) = plugin_inspector_panel_heights(dialog, width);
    let stacked = should_stack_detail_layout(width, 34, 48);
    if stacked {
        list_height
            .saturating_add(detail_height)
            .saturating_add(logs_height)
    } else {
        max(list_height, detail_height.saturating_add(logs_height))
    }
}

fn timeline_overlay_panel_heights(dialog: &TimelineOverlay, width: u16) -> (u16, u16) {
    let list_height = list_panel_height(
        if dialog.loading || dialog.items.is_empty() {
            1
        } else {
            dialog.items.len()
        },
        1,
        5,
        10,
    );
    let detail_text = if dialog.loading {
        "loading"
    } else {
        dialog
            .items
            .get(dialog.selected)
            .map(|item| item.detail.as_str())
            .unwrap_or(dialog.empty_message.as_str())
    };
    let detail_width = if should_stack_detail_layout(width, 40, 46) {
        width.saturating_sub(2).max(1)
    } else {
        estimated_horizontal_panel_widths(width, 40, 46)
            .1
            .saturating_sub(2)
            .max(1)
    };
    let detail_height = bordered_paragraph_height(detail_text, detail_width, 4, 12);
    (list_height, detail_height)
}

fn plugin_inspector_panel_heights(dialog: &PluginInspectorOverlay, width: u16) -> (u16, u16, u16) {
    let list_height = list_panel_height(
        if dialog.items.is_empty() {
            1
        } else {
            dialog.items.len()
        },
        1,
        4,
        10,
    );
    let detail_text = dialog
        .items
        .get(dialog.selected)
        .map(|item| item.detail.as_str())
        .unwrap_or(dialog.empty_message.as_str());
    let logs_text = dialog
        .items
        .get(dialog.selected)
        .map(|item| item.logs.as_str())
        .unwrap_or(dialog.empty_message.as_str());
    if should_stack_detail_layout(width, 34, 48) {
        let detail_width = width.saturating_sub(2).max(1);
        let detail_height = bordered_paragraph_height(detail_text, detail_width, 3, 8);
        let logs_height = bordered_paragraph_height(logs_text, detail_width, 3, 8);
        (list_height, detail_height, logs_height)
    } else {
        let right_width = estimated_horizontal_panel_widths(width, 34, 48)
            .1
            .saturating_sub(2)
            .max(1);
        let detail_height = bordered_paragraph_height(detail_text, right_width, 3, 7);
        let logs_height = bordered_paragraph_height(logs_text, right_width, 3, 7);
        (list_height, detail_height, logs_height)
    }
}

fn provider_studio_main_field_display(
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> String {
    let value = provider_studio_main_field_value(dialog, field);
    match field {
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
    }
}

fn provider_studio_draft_panel_height(dialog: &ProviderStudioOverlay, width: u16) -> u16 {
    let content_width = width.saturating_sub(4).max(1);
    let lines = provider_studio_visible_fields(dialog)
        .iter()
        .map(|field| {
            wrapped_text_height(
                format!(
                    "{:>16}  {}",
                    provider_studio_field_label(*field),
                    provider_studio_main_field_display(dialog, *field),
                )
                .as_str(),
                content_width,
            )
        })
        .sum::<u16>();
    lines.clamp(4, 16).saturating_add(2)
}

fn provider_studio_adapter_model_heights(
    dialog: &ProviderStudioOverlay,
    _width: u16,
) -> (u16, u16) {
    let adapter_height = list_panel_height(
        dialog.adapter_candidate_ids.len().max(1),
        if dialog.adapter_candidate_ids.is_empty() {
            1
        } else {
            2
        },
        4,
        10,
    );
    let model_height = provider_studio_selected_adapter_models(dialog)
        .map(|adapter_models| {
            list_panel_height(
                adapter_models.models.len().max(1),
                if adapter_models.models.is_empty() {
                    1
                } else {
                    2
                },
                4,
                10,
            )
        })
        .unwrap_or_else(|| list_panel_height(1, 1, 4, 10));
    (adapter_height, model_height)
}

fn provider_studio_detail_page_lines(dialog: &ProviderStudioOverlay) -> Vec<String> {
    let mut lines = vec![provider_studio_auth_status_summary(dialog).to_owned()];
    lines.extend(provider_studio_auth_state_lines(dialog));
    lines.extend(
        provider_studio_detail_fields(dialog)
            .iter()
            .map(|field| provider_studio_main_field_display(dialog, *field)),
    );
    lines
}

fn provider_studio_detail_page_height(dialog: &ProviderStudioOverlay, modal_width: u16) -> u16 {
    let Some(detail_page) = dialog.detail_page.as_ref() else {
        return 8;
    };
    let inner_width = modal_width.saturating_sub(2).max(1);
    let body_height = provider_studio_detail_page_lines(dialog)
        .iter()
        .map(|line| wrapped_text_height(line.as_str(), inner_width))
        .sum::<u16>()
        .clamp(4, 20);
    let footer_height = wrapped_text_height(detail_page.footer.as_str(), inner_width).clamp(1, 2);
    body_height.saturating_add(footer_height).saturating_add(2)
}

fn model_catalog_panel_heights(dialog: &ModelCatalogStudioOverlay, width: u16) -> (u16, u16) {
    let list_height = list_panel_height(
        dialog.items.len().max(1),
        if dialog.loading || dialog.items.is_empty() {
            1
        } else {
            2
        },
        5,
        14,
    );
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
                format!("source: {:?}", entry.source),
                entry.description.clone().unwrap_or_default(),
            ]
            .into_iter()
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
        })
        .unwrap_or_else(|| {
            dialog
                .summary
                .last_error
                .clone()
                .unwrap_or_else(|| "No catalog entries.".to_string())
        });
    let detail_width = if should_stack_detail_layout(width, 48, 34) {
        width.saturating_sub(2).max(1)
    } else {
        estimated_horizontal_panel_widths(width, 48, 34)
            .1
            .saturating_sub(2)
            .max(1)
    };
    let detail_height = bordered_paragraph_height(detail.as_str(), detail_width, 4, 12);
    (list_height, detail_height)
}

fn editor_input_panel_height(editor: &Editor, multiline: bool) -> u16 {
    if !multiline {
        return 3;
    }
    u16::try_from(max(1, editor.logical_line_count()))
        .unwrap_or(u16::MAX)
        .clamp(4, 8)
        .saturating_add(2)
}

fn permission_overlay_body_height(dialog: &PermissionOverlay, width: u16) -> u16 {
    let content_width = width.saturating_sub(2);
    let mut lines = 2_u16;
    lines = lines.saturating_add(wrapped_text_height(
        format!(
            "Reason: {}",
            sanitize_display_text(dialog.request.reason.as_str())
        )
        .as_str(),
        content_width,
    ));
    if !dialog.request.explanation.trim().is_empty() {
        lines = lines.saturating_add(wrapped_text_height(
            format!(
                "Explanation: {}",
                sanitize_display_text(dialog.request.explanation.as_str())
            )
            .as_str(),
            content_width,
        ));
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
        lines = lines.saturating_add(wrapped_text_height(
            facts.join(" · ").as_str(),
            content_width,
        ));
    }
    if dialog.request.session_id.is_some() {
        lines = lines.saturating_add(1);
    }
    if !dialog.request.trace.is_empty() {
        lines = lines.saturating_add(1);
        for step in &dialog.request.trace {
            lines = lines.saturating_add(wrapped_text_height(
                permission_trace_step_label(step).as_str(),
                content_width,
            ));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_modal_width_never_exceeds_available_width() {
        assert_eq!(adaptive_modal_width(30, 96), 28);
    }

    #[test]
    fn adaptive_modal_height_preserves_requested_height_when_space_is_available() {
        assert_eq!(adaptive_modal_height(24, 19), 19);
    }

    #[test]
    fn adaptive_modal_height_clamps_to_available_height() {
        assert_eq!(adaptive_modal_height(24, 40), 22);
    }

    #[test]
    fn estimated_horizontal_panel_widths_follow_stacked_layout() {
        let (left, right) = estimated_horizontal_panel_widths(40, 34, 48);
        assert_eq!((left, right), (40, 40));
    }

    #[test]
    fn estimated_horizontal_panel_widths_preserve_total_width() {
        let (left, right) = estimated_horizontal_panel_widths(114, 48, 34);
        assert_eq!(left + right, 114);
        assert!(left > right);
    }
}
