impl App {
    pub(in crate::app) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if !matches!(self.current_route, Route::Main) {
            let footer_height = self.route_footer_height(area.width, area.height);
            let (body, footer) = if footer_height > 0 && area.height > footer_height {
                let rows = split_vertical_sections(
                    area,
                    &[
                        VerticalSectionSize::Flexible(1),
                        VerticalSectionSize::Fixed(footer_height),
                    ],
                );
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

        let composer_height = self.composer_height(area.width, area.height);
        let vertical = split_vertical_sections(
            area,
            &[
                VerticalSectionSize::Flexible(8),
                VerticalSectionSize::Fixed(composer_height),
            ],
        );

        let transcript_host_area = vertical[0];
        let composer = vertical[1];
        let transcript_footer_height =
            self.transcript_footer_height(transcript_host_area.width, transcript_host_area.height);

        let transcript_layout = layout_header_body_footer_surface(
            transcript_host_area,
            pane_header_height(transcript_host_area.height),
            transcript_footer_height,
            1,
        );
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

    pub(in crate::app) fn route_footer_height(&self, width: u16, total_height: u16) -> u16 {
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

    pub(in crate::app) fn render_transcript_surface(&mut self, frame: &mut Frame, area: Rect) {
        let footer_height = self.transcript_footer_height(area.width, area.height);
        let layout = layout_header_body_footer_surface(
            area,
            pane_header_height(area.height),
            footer_height,
            1,
        );

        let lines = if self.transcript.session_id.is_none()
            && self.transcript.pending_user_messages.is_empty()
        {
            vec![
                Line::from(ui_text::t(&self.i18n, "no-session-selected")),
                Line::from(Span::styled(
                    ui_text::t(&self.i18n, "no-session-selected-hint"),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )),
            ]
        } else {
            let rendered = self.transcript.rendered(layout.body.width).clone();
            let matches = rendered.search_matches.clone();
            let active_match = self
                .transcript
                .search_match_index
                .and_then(|index| matches.get(index).copied());
            let highlighted_block = self.transcript.highlighted_block_range(layout.body.width);
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
                    let block_selected = highlighted_block
                        .as_ref()
                        .is_some_and(|range| idx >= range.start && idx < range.end);
                    if self.focus == Focus::Transcript
                        && (idx == self.transcript.cursor_line || block_selected)
                    {
                        rendered_line = apply_line_highlight(rendered_line);
                    }
                    rendered_line
                })
                .collect::<Vec<_>>()
        };
        let title = sanitize_display_text(self.transcript_surface_title());
        let subtitle = self
            .current_session_path_label()
            .map(|path| sanitize_display_text(path.as_str()));
        let right = sanitize_display_text(self.transcript_surface_top_right().join("  ·  "));
        let footer = if layout.footer.height > 0 {
            let footer_lines = self.transcript_footer_lines(layout.footer.width);
            (!footer_lines.is_empty()).then(|| Text::from(footer_lines))
        } else {
            None
        };
        render_header_body_footer_text_surface(
            frame,
            layout,
            &HeaderBodyFooterTextSurfaceSpec {
                title: title.into(),
                subtitle: subtitle.map(Into::into),
                right: Some(right.into()),
                body: Text::from(lines),
                body_scroll: (min(self.transcript.scroll, u16::MAX as usize) as u16, 0),
                body_wrap: false,
                footer,
                title_style: Style::default().add_modifier(Modifier::BOLD),
                subtitle_style: Style::default()
                    .fg(agena_tui_components::theme::muted_color())
                    .add_modifier(Modifier::DIM | Modifier::ITALIC),
                right_style: Style::default().fg(agena_tui_components::theme::muted_color()),
            },
        );
    }

    pub(in crate::app) fn transcript_surface_top_right(&self) -> Vec<String> {
        vec![self.main_surface_mode_label()]
    }

    pub(in crate::app) fn transcript_surface_title(&self) -> String {
        ui_text::transcript_header_title(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
        )
    }

    pub(in crate::app) fn render_composer(&self, frame: &mut Frame, area: Rect) {
        let status_rows = u16::from(!self.composer_status_parts().is_empty());
        let item_rows = u16::from(self.has_composer_item_summary_row());
        let layout =
            layout_composer_surface(area, status_rows, item_rows, self.composer_popup_rows());

        if layout.inner.width == 0 || layout.inner.height == 0 {
            if let Some(status_area) = layout.status {
                self.render_composer_status_row(frame, inset_rect(status_area, 1, 0));
            }
            return;
        }

        if let Some(item_row) = layout.items {
            self.render_composer_items_row(frame, item_row);
        }
        if let Some(popup_row) = layout.popup {
            self.render_active_composer_popup(frame, popup_row);
        }

        let editor_view = self.composer.render_wrapped_view(
            layout.editor.width.saturating_sub(2).max(1),
            layout.editor.height.max(1),
        );
        let cursor = if self.overlay.is_none()
            && self.focus == Focus::Composer
            && self.prompt_history_search.is_none()
            && self.selected_composer_item.is_none()
        {
            Some((editor_view.cursor_x, editor_view.cursor_y))
        } else {
            None
        };
        let placeholder = self.composer.text().is_empty().then(|| {
            Line::from(Span::styled(
                ui_text::t(&self.i18n, "composer-placeholder"),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            ))
        });
        render_composer_editor_surface(
            frame,
            layout,
            &ComposerEditorSurfaceSpec {
                editor_lines: Text::from(editor_view.lines.clone()),
                placeholder,
                cursor,
            },
        );

        if let Some(status_area) = layout.status {
            self.render_composer_status_row(frame, inset_rect(status_area, 1, 0));
        }
        self.render_prompt_history_floating_window(frame, area);
    }

    pub(in crate::app) fn render_prompt_history_floating_window(
        &self,
        frame: &mut Frame,
        composer_area: Rect,
    ) {
        let Some(search) = self.prompt_history_search.as_ref() else {
            return;
        };
        let result_rows = if search.items.is_empty() {
            1
        } else {
            min(MAX_PROMPT_HISTORY_SEARCH_RESULTS, search.items.len())
        };
        let height = (result_rows as u16).saturating_add(3);
        let available_above = composer_area.y.saturating_sub(frame.area().y);
        let height = min(height, available_above);
        if height < 3 || composer_area.width < 8 {
            return;
        }
        let width = min(composer_area.width.saturating_sub(2), 100);
        let area = Rect {
            x: composer_area
                .x
                .saturating_add(composer_area.width.saturating_sub(width) / 2),
            y: composer_area.y.saturating_sub(height),
            width,
            height,
        };
        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .title(ui_text::t(&self.i18n, "composer-prompt-history-title"));
        let inner = block.inner(area);
        frame.render_widget(ratatui::widgets::Clear, area);
        frame.render_widget(block, area);
        self.render_prompt_history_search(frame, inner);
    }

    pub(in crate::app) fn render_active_composer_popup(&self, frame: &mut Frame, area: Rect) {
        if self.prompt_history_search.is_some() {
            self.render_prompt_history_search(frame, area);
        } else if self.file_mention_suggestions.is_some() {
            self.render_file_mention_suggestions(frame, area);
        } else {
            self.render_slash_command_suggestions(frame, area);
        }
    }

    pub(in crate::app) fn render_slash_command_suggestions(&self, frame: &mut Frame, area: Rect) {
        let Some(state) = self.slash_command_suggestions.as_ref() else {
            return;
        };
        if area.width == 0 || area.height == 0 || state.items.is_empty() {
            return;
        }

        let items = state
            .items
            .iter()
            .map(|item| SuggestionPopupItem {
                prefix: None,
                label: sanitize_display_text(item.label.as_str()).into(),
                detail: Some(sanitize_display_text(item.detail.as_str()).into()),
            })
            .collect::<Vec<_>>();
        render_suggestion_popup(
            frame,
            area,
            &SuggestionPopupSpec {
                items: items.as_slice(),
                selected: state.selected,
                max_visible_rows: MAX_SLASH_COMMAND_SUGGESTIONS,
                selected_marker: "> ".into(),
                unselected_marker: "  ".into(),
                max_label_width: 24,
                detail_gap: 2,
                base_style: Style::default(),
                selected_style: selection_highlight_style(),
                prefix_style: Style::default(),
                selected_prefix_style: None,
                label_style: Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD),
                detail_style: Style::default().fg(agena_tui_components::theme::muted_color()),
                pad_selected_row: true,
            },
        );
    }

    pub(in crate::app) fn render_file_mention_suggestions(&self, frame: &mut Frame, area: Rect) {
        let Some(state) = self.file_mention_suggestions.as_ref() else {
            return;
        };
        if area.width == 0 || area.height == 0 || state.items.is_empty() {
            return;
        }

        let items = state
            .items
            .iter()
            .map(|item| SuggestionPopupItem {
                prefix: None,
                label: sanitize_display_text(item.label.as_str()).into(),
                detail: Some(sanitize_display_text(item.detail.as_str()).into()),
            })
            .collect::<Vec<_>>();
        render_suggestion_popup(
            frame,
            area,
            &SuggestionPopupSpec {
                items: items.as_slice(),
                selected: state.selected,
                max_visible_rows: MAX_FILE_MENTION_SUGGESTIONS,
                selected_marker: "@ ".into(),
                unselected_marker: "  ".into(),
                max_label_width: 28,
                detail_gap: 2,
                base_style: Style::default(),
                selected_style: selection_highlight_style(),
                prefix_style: Style::default(),
                selected_prefix_style: None,
                label_style: Style::default()
                    .fg(agena_tui_components::theme::info_color())
                    .add_modifier(Modifier::BOLD),
                detail_style: Style::default().fg(agena_tui_components::theme::muted_color()),
                pad_selected_row: true,
            },
        );
    }

    pub(in crate::app) fn render_prompt_history_search(&self, frame: &mut Frame, area: Rect) {
        let Some(search) = self.prompt_history_search.as_ref() else {
            return;
        };
        if area.width == 0 || area.height == 0 {
            return;
        }

        let query = sanitize_display_text(search.query.text());
        let items = search
            .items
            .iter()
            .map(|result| SuggestionPopupItem {
                prefix: Some(format!("#{:<3} ", result.history_index + 1).into()),
                label: sanitize_display_text(result.text.as_str()).into(),
                detail: None,
            })
            .collect::<Vec<_>>();
        let cursor = render_query_suggestion_popup(
            frame,
            area,
            &QuerySuggestionPopupSpec {
                prompt_label: ui_text::t(&self.i18n, "composer-prompt-history-label").into(),
                query: query.clone().into(),
                empty_message: ui_text::t(&self.i18n, "composer-prompt-history-no-matches").into(),
                prompt_style: Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD),
                query_style: Style::default(),
                empty_style: Style::default().fg(agena_tui_components::theme::muted_color()),
                results: SuggestionPopupSpec {
                    items: items.as_slice(),
                    selected: search.selected,
                    max_visible_rows: area.height.saturating_sub(1) as usize,
                    selected_marker: "> ".into(),
                    unselected_marker: "  ".into(),
                    max_label_width: area.width.saturating_sub(7) as usize,
                    detail_gap: 2,
                    base_style: Style::default(),
                    selected_style: selection_highlight_style(),
                    prefix_style: Style::default().fg(agena_tui_components::theme::muted_color()),
                    selected_prefix_style: Some(
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                    ),
                    label_style: Style::default(),
                    detail_style: Style::default().fg(agena_tui_components::theme::muted_color()),
                    pad_selected_row: false,
                },
            },
        );

        if self.overlay.is_none() && self.focus == Focus::Composer {
            if let Some(cursor) = cursor {
                frame.set_cursor_position(cursor);
            }
        }
    }

    pub(in crate::app) fn render_composer_items_row(&self, frame: &mut Frame, area: Rect) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let mut spans = Vec::new();
        let mut visible_items = 0;
        for (index, item) in self.composer_items.iter().enumerate() {
            if !composer_item_needs_summary_chip(item) {
                continue;
            }
            if visible_items > 0 {
                spans.push(Span::styled(
                    "  ",
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                ));
            }
            let style = if self.selected_composer_item == Some(index) {
                selection_highlight_style().add_modifier(Modifier::BOLD)
            } else {
                self.composer_item_style(item)
            };
            spans.push(Span::styled(format!("[{}]", item.short_label()), style));
            visible_items += 1;
        }

        frame.render_widget(
            Paragraph::new(Line::from(spans)).wrap(Wrap { trim: false }),
            area,
        );
    }

    pub(in crate::app) fn render_transcript_footer_row(&self, frame: &mut Frame, area: Rect) {
        let Some(spec) = self.transcript_footer_spec() else {
            return;
        };
        render_wrapped_text(frame, area, &spec);
    }

    pub(in crate::app) fn transcript_footer_spec(&self) -> Option<WrappedTextSpec<'static>> {
        if let Some(flash) = &self.flash {
            return Some(WrappedTextSpec {
                text: sanitize_display_text(flash.text.as_str()).into(),
                style: self.flash_style(flash.level),
            });
        }

        let combined = self.transcript_footer_text();
        if combined.trim().is_empty() {
            return None;
        }
        Some(WrappedTextSpec {
            text: sanitize_display_text(combined.as_str()).into(),
            style: Style::default().fg(agena_tui_components::theme::muted_color()),
        })
    }

    pub(in crate::app) fn transcript_footer_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.transcript_footer_spec()
            .map(|spec| build_wrapped_text_lines(&spec, width))
            .unwrap_or_default()
    }

    pub(in crate::app) fn transcript_footer_text(&self) -> String {
        let mut parts = Vec::new();
        if !self.queue.is_empty() {
            let preview = self.queue.first_preview(28).unwrap_or_default();
            if preview.is_empty() {
                parts.push(self.i18n.text_args(
                    "transcript-footer-queue",
                    &crate::fl_args!("count" => self.queue.len() as i64),
                ));
            } else {
                parts.push(self.i18n.text_args(
                    "transcript-footer-queue-preview",
                    &crate::fl_args!(
                        "count" => self.queue.len() as i64,
                        "preview" => preview,
                    ),
                ));
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
            parts.push(ui_text::transcript_footer_plugin_block(
                &self.i18n,
                block.block.title.as_str(),
                block.block.body.as_str(),
            ));
        }

        parts.join("  |  ")
    }

    pub(in crate::app) fn transcript_footer_height(&self, width: u16, total_height: u16) -> u16 {
        if total_height <= pane_header_height(total_height).saturating_add(1) {
            return 0;
        }
        let line_count = self.transcript_footer_lines(width).len();
        if line_count == 0 {
            0
        } else {
            min(2, line_count as u16)
        }
    }

    pub(in crate::app) fn composer_popup_rows(&self) -> u16 {
        if self.overlay.is_some() || self.focus != Focus::Composer {
            return 0;
        }
        if self.prompt_history_search.is_some() {
            return 0;
        }
        if let Some(state) = self.file_mention_suggestions.as_ref() {
            return min(MAX_FILE_MENTION_SUGGESTIONS, state.items.len()) as u16;
        }
        self.slash_command_suggestions
            .as_ref()
            .map(|state| min(MAX_SLASH_COMMAND_SUGGESTIONS, state.items.len()) as u16)
            .unwrap_or(0)
    }

    pub(in crate::app) fn composer_item_style(&self, item: &ComposerItem) -> Style {
        match item {
            ComposerItem::Attachment(_) => Style::default()
                .fg(agena_tui_components::theme::info_color())
                .add_modifier(Modifier::BOLD),
            ComposerItem::LargePaste(_) => Style::default()
                .fg(agena_tui_components::theme::warning_color())
                .add_modifier(Modifier::BOLD),
        }
    }

    /// Large pastes are already represented by an inline, atomic editor
    /// element. Rendering another chip above the editor repeats the same
    /// character count on a second line, so reserve the summary row for file
    /// attachments only.
    pub(in crate::app) fn has_composer_item_summary_row(&self) -> bool {
        self.composer_items
            .iter()
            .any(composer_item_needs_summary_chip)
    }

    pub(in crate::app) fn flash_style(&self, level: FlashLevel) -> Style {
        match level {
            FlashLevel::Success => {
                Style::default().fg(agena_tui_components::theme::success_color())
            }
            FlashLevel::Warning => {
                Style::default().fg(agena_tui_components::theme::warning_color())
            }
            FlashLevel::Error => Style::default().fg(agena_tui_components::theme::danger_color()),
            FlashLevel::Info => Style::default().fg(agena_tui_components::theme::info_color()),
        }
    }

    pub(in crate::app) fn render_composer_status_row(&self, frame: &mut Frame, area: Rect) {
        let text = self.composer_status_parts().join("  |  ");
        if text.trim().is_empty() {
            return;
        }
        render_wrapped_text(
            frame,
            area,
            &WrappedTextSpec {
                text: sanitize_display_text(text.as_str()).into(),
                style: Style::default().fg(agena_tui_components::theme::muted_color()),
            },
        );
    }

    pub(in crate::app) fn composer_status_parts(&self) -> Vec<String> {
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
            parts.push(self.i18n.text_args(
                "composer-status-selected-item",
                &crate::fl_args!(
                    "current" => selected.0.saturating_add(1) as i64,
                    "total" => self.composer_items.len() as i64,
                    "label" => selected.1.short_label(),
                ),
            ));
        }
        if let Some(search) = self.prompt_history_search.as_ref() {
            let query = search.query.text().trim();
            let selection = min(search.selected + 1, search.items.len().max(1));
            parts.push(if query.is_empty() {
                self.i18n.text_args(
                    "composer-status-history",
                    &crate::fl_args!(
                        "current" => selection as i64,
                        "total" => search.meta.total_matches as i64,
                    ),
                )
            } else {
                self.i18n.text_args(
                    "composer-status-history-query",
                    &crate::fl_args!(
                        "current" => selection as i64,
                        "total" => search.meta.total_matches as i64,
                        "query" => query,
                    ),
                )
            });
        } else if let Some(state) = self.file_mention_suggestions.as_ref() {
            parts.push(self.i18n.text_args(
                "composer-status-mention",
                &crate::fl_args!("query" => ui_text::prefixed_query("@", state.query.as_str())),
            ));
        } else if let Some(state) = self.slash_command_suggestions.as_ref() {
            parts.push(self.i18n.text_args(
                "composer-status-slash",
                &crate::fl_args!("query" => ui_text::prefixed_query("/", state.query.as_str())),
            ));
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

    pub(in crate::app) fn main_surface_mode_label(&self) -> String {
        if self.focus == Focus::Composer {
            ui_text::t(&self.i18n, "surface-mode-insert")
        } else {
            ui_text::t(&self.i18n, "surface-mode-view")
        }
    }
}
use super::{
    App, ComposerEditorSurfaceSpec, ComposerItem, FlashLevel, Focus, Frame,
    HeaderBodyFooterTextSurfaceSpec, LayoutCache, Line, MAX_FILE_MENTION_SUGGESTIONS,
    MAX_PROMPT_HISTORY_SEARCH_RESULTS, MAX_SLASH_COMMAND_SUGGESTIONS, Modifier, Paragraph,
    QuerySuggestionPopupSpec, Rect, Route, Span, Style, SuggestionPopupItem, SuggestionPopupSpec,
    Text, VerticalSectionSize, Wrap, WrappedTextSpec, apply_line_highlight,
    build_wrapped_text_lines, composer_item_needs_summary_chip, highlight_search_line, inset_rect,
    layout_composer_surface, layout_header_body_footer_surface, min, pane_header_height,
    pending_interactive_counts_for_execution, render_composer_editor_surface,
    render_header_body_footer_text_surface, render_query_suggestion_popup, render_suggestion_popup,
    render_wrapped_text, sanitize_display_text, selection_highlight_style, split_vertical_sections,
    ui_text,
};
