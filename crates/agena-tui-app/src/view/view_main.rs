pub(crate) fn highlight_search_line(
    text: &str,
    base_style: Style,
    query: &str,
    active_match: bool,
    has_match: bool,
) -> Line<'static> {
    let line_style = if active_match {
        base_style.patch(agena_tui_components::theme::selection_style())
    } else if has_match {
        base_style
            .fg(agena_tui_components::theme::accent_color())
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
            line_style
        } else {
            line_style
                .fg(agena_tui_components::theme::accent_color())
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

impl App {
    pub(crate) fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        if matches!(
            self.current_route,
            Route::SessionSearch(_)
                | Route::CommandPalette(_)
                | Route::SkillPicker(_)
                | Route::SkillStudio(_)
                | Route::SessionNavigation(_)
                | Route::SelectionPicker(_)
                | Route::SessionModelChooser(_)
                | Route::Timeline(_)
        ) {
            self.render_search_picker_route_background(frame, area);
            self.render_route(frame, area);
            self.render_context_help(frame, area);
            return;
        }
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
            self.render_context_help(frame, area);
            return;
        }

        self.render_main_content(frame, area);
        self.render_overlay(frame, area);
        self.render_context_help(frame, area);
    }

    fn render_search_picker_route_background(&mut self, frame: &mut Frame, area: Rect) {
        if let Some(parent) = self.route_stack.last().cloned()
            && !matches!(parent, Route::Main)
        {
            self.layout = LayoutCache::default();
            self.render_route_content(frame, area, &parent);
        } else {
            self.render_main_content(frame, area);
        }
    }

    fn render_main_content(&mut self, frame: &mut Frame, area: Rect) {
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
            transcript_scrollbar: agena_tui_transcript::scrollbar_area(
                transcript_host_area,
                transcript_layout.body,
            ),
        };

        self.transcript.clamp_scroll(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
        );

        self.render_transcript_surface(frame, transcript_host_area);
        self.render_composer(frame, composer);
    }

    pub(crate) fn route_footer_height(&self, width: u16, total_height: u16) -> u16 {
        if matches!(
            &self.current_route,
            Route::SessionSearch(_)
                | Route::CommandPalette(_)
                | Route::SkillPicker(_)
                | Route::SkillStudio(_)
                | Route::SessionNavigation(_)
                | Route::SelectionPicker(_)
                | Route::SessionModelChooser(_)
                | Route::Timeline(_)
        ) {
            return 0;
        }
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

    pub(crate) fn render_transcript_surface(&mut self, frame: &mut Frame, area: Rect) {
        let footer_height = self.transcript_footer_height(area.width, area.height);
        let layout = layout_header_body_footer_surface(
            area,
            pane_header_height(area.height),
            footer_height,
            1,
        );

        let has_transcript_content = self.transcript.session_id.is_some()
            || !self.transcript.pending_user_messages.is_empty();
        if has_transcript_content {
            self.transcript
                .ensure_visual_focus(layout.body.width, layout.body.height);
        }
        let scroll = self.transcript.viewport_top();
        let viewport_height = usize::from(layout.body.height);
        let mut transcript_line_count = 0;
        let spinner = spinner_frame(current_spinner_millis());
        let mut math = Vec::new();
        let lines = if !has_transcript_content {
            vec![
                Line::from(ui_text::t(&self.i18n, "no-session-selected")),
                Line::from(Span::styled(
                    ui_text::t(&self.i18n, "no-session-selected-hint"),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )),
            ]
        } else {
            let highlighted_block = self.transcript.highlighted_block_range(layout.body.width);
            let search_match_index = self.transcript.search_match_index;
            let search_query = self.transcript.search_query.clone();
            let cursor_cell = self.transcript.cursor_cell_range(layout.body.width);
            let selection_ranges = self.transcript.selection_cell_ranges(layout.body.width);
            let rendered = self.transcript.rendered(layout.body.width);
            transcript_line_count = rendered.lines.len();
            let active_match =
                search_match_index.and_then(|index| rendered.search_matches.get(index).copied());
            let visible = transcript_visible_range(rendered.lines.len(), scroll, viewport_height);
            let visible_start = visible.start;
            let visible_end = visible.end;
            math = rendered
                .math
                .iter()
                .filter(|placement| {
                    let placement_end = placement
                        .line
                        .saturating_add(usize::from(placement.size.height));
                    placement.line < visible_end && placement_end > visible_start
                })
                .cloned()
                .collect();
            rendered
                .lines
                .get(visible_start..visible_end)
                .unwrap_or_default()
                .iter()
                .enumerate()
                .map(|(offset, line)| {
                    let idx = visible_start.saturating_add(offset);
                    let line_is_active = active_match == Some(idx);
                    let line_has_match = rendered.search_matches.binary_search(&idx).is_ok();
                    let mut rendered_line = if !line_has_match && search_query.trim().is_empty() {
                        line.rich_line.clone().unwrap_or_else(|| {
                            Line::from(Span::styled(line.text.clone(), line.style))
                        })
                    } else {
                        highlight_search_line(
                            line.text.as_str(),
                            line.style,
                            search_query.as_str(),
                            line_is_active,
                            line_has_match,
                        )
                    };
                    if transcript_line_is_in_block(idx, highlighted_block.as_ref()) {
                        rendered_line = apply_block_highlight(rendered_line, layout.body.width);
                    }
                    if let Some((cursor_line, range)) = cursor_cell.as_ref()
                        && *cursor_line == idx
                    {
                        rendered_line = apply_cursor_cell_highlight(rendered_line, range.clone());
                    }
                    if let Some(range) = selection_ranges.get(idx).and_then(Clone::clone) {
                        rendered_line = apply_line_cell_highlight(rendered_line, range);
                    }
                    refresh_spinner_line(rendered_line, spinner)
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
                body_scroll: (0, 0),
                body_wrap: false,
                footer,
                title_style: Style::default().add_modifier(Modifier::BOLD),
                subtitle_style: Style::default()
                    .fg(agena_tui_components::theme::muted_color())
                    .add_modifier(Modifier::DIM | Modifier::ITALIC),
                right_style: Style::default().fg(agena_tui_components::theme::muted_color()),
            },
        );
        let scrollbar_area = agena_tui_transcript::scrollbar_area(area, layout.body);
        if let Some(metrics) = agena_tui_transcript::scrollbar_metrics(
            transcript_line_count,
            viewport_height,
            usize::from(scrollbar_area.height),
            scroll,
        ) {
            let palette = agena_tui_components::theme::active_palette();
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(Style::default().fg(palette.muted))
                .thumb_symbol("█")
                .thumb_style(Style::default().fg(palette.accent));
            // With an explicit viewport length, represent every valid scroll
            // offset (zero through max_scroll) as one state position.
            let mut state = ScrollbarState::new(metrics.max_scroll.saturating_add(1))
                .position(scroll.min(metrics.max_scroll))
                .viewport_content_length(viewport_height);
            frame.render_stateful_widget(scrollbar, scrollbar_area, &mut state);
        }
        if let Some(renderer) = self.math_renderer.as_mut() {
            renderer.render(
                frame,
                layout.body,
                self.transcript.viewport_top(),
                math.as_slice(),
            );
        }
    }

    pub(crate) fn transcript_surface_top_right(&self) -> Vec<String> {
        transcript_surface_top_right_parts(
            self.current_session_activity_indicator(),
            self.main_surface_mode_label(),
        )
    }

    pub(crate) fn transcript_surface_title(&self) -> String {
        ui_text::transcript_header_title(
            &self.i18n,
            self.transcript.session_id,
            self.transcript.session_title.as_str(),
        )
    }

    pub(crate) fn render_composer(&self, frame: &mut Frame, area: Rect) {
        let status_rows = u16::from(!self.composer_status_parts().is_empty());
        let item_rows = u16::from(self.has_composer_item_summary_row());
        let layout = layout_composer_surface(area, status_rows, item_rows, 0);

        if layout.inner.width == 0 || layout.inner.height == 0 {
            if let Some(status_area) = layout.status {
                self.render_composer_status_row(frame, inset_rect(status_area, 1, 0));
            }
            return;
        }

        if let Some(item_row) = layout.items {
            self.render_composer_items_row(frame, item_row);
        }
        let editor_view = self.composer.render_wrapped_view(
            layout.editor.width.saturating_sub(2).max(1),
            layout.editor.height.max(1),
        );
        let cursor = if self.overlay.is_none()
            && self.focus == Focus::Composer
            && self.prompt_history_search.is_none()
            && !self.composer_item_selection.is_active()
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
        if let Some(search) = self.prompt_history_search.as_ref() {
            agena_tui::prompt_history::render_overlay(frame, frame.area(), search, &self.i18n);
        }
        if self.file_mention_suggestions.is_none()
            && let Some(state) = self.slash_command_suggestions.as_ref()
        {
            agena_tui::slash_commands::render_overlay(frame, frame.area(), state, &self.i18n);
        }
        if let Some(state) = self.file_mention_suggestions.as_ref() {
            agena_tui::file_mentions::render_overlay(frame, frame.area(), state, &self.i18n);
        }
    }

    pub(crate) fn render_composer_items_row(&self, frame: &mut Frame, area: Rect) {
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
            let style = if self.composer_item_selection.is_selected(index) {
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

    pub(crate) fn render_transcript_footer_row(&self, frame: &mut Frame, area: Rect) {
        let Some(spec) = self.transcript_footer_spec() else {
            return;
        };
        render_wrapped_text(frame, area, &spec);
    }

    pub(crate) fn transcript_footer_spec(&self) -> Option<WrappedTextSpec<'static>> {
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

    pub(crate) fn transcript_footer_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.transcript_footer_spec()
            .map(|spec| build_wrapped_text_lines(&spec, width))
            .unwrap_or_default()
    }

    pub(crate) fn transcript_footer_text(&self) -> String {
        let mut parts = Vec::new();
        if !self.queue.is_empty() {
            let preview = self.queue.first_preview(28).unwrap_or_default();
            if preview.is_empty() {
                parts.push(self.i18n.text_args(
                    "transcript-footer-queue",
                    &agena_tui::fl_args!("count" => self.queue.len() as i64),
                ));
            } else {
                parts.push(self.i18n.text_args(
                    "transcript-footer-queue-preview",
                    &agena_tui::fl_args!(
                        "count" => self.queue.len() as i64,
                        "preview" => preview,
                    ),
                ));
            }
        }
        if let Some(status_line) = self
            .status_line
            .as_ref()
            .and_then(|status_line| status_line.text())
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

    pub(crate) fn transcript_footer_height(&self, width: u16, total_height: u16) -> u16 {
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

    pub(crate) fn composer_item_style(&self, item: &ComposerItem) -> Style {
        match item {
            ComposerItem::Attachment(_) => Style::default()
                .fg(agena_tui_components::theme::info_color())
                .add_modifier(Modifier::BOLD),
            ComposerItem::LargePaste(_) => Style::default()
                .fg(agena_tui_components::theme::warning_color())
                .add_modifier(Modifier::BOLD),
            ComposerItem::SkillReference(_) => Style::default()
                .fg(agena_tui_components::theme::accent_color())
                .add_modifier(Modifier::BOLD),
        }
    }

    /// Large pastes are already represented by an inline, atomic editor
    /// element. Rendering another chip above the editor repeats the same
    /// character count on a second line, so reserve the summary row for file
    /// attachments only.
    pub(crate) fn has_composer_item_summary_row(&self) -> bool {
        self.composer_items
            .iter()
            .any(composer_item_needs_summary_chip)
    }

    pub(crate) fn flash_style(&self, level: FlashLevel) -> Style {
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

    pub(crate) fn render_composer_status_row(&self, frame: &mut Frame, area: Rect) {
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

    pub(crate) fn composer_status_parts(&self) -> Vec<String> {
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
            .composer_item_selection
            .selected()
            .and_then(|index| self.composer_items.get(index).map(|item| (index, item)))
        {
            parts.push(self.i18n.text_args(
                "composer-status-selected-item",
                &agena_tui::fl_args!(
                    "current" => selected.0.saturating_add(1) as i64,
                    "total" => self.composer_items.len() as i64,
                    "label" => selected.1.short_label(),
                ),
            ));
        }
        if let Some(search) = self.prompt_history_search.as_ref() {
            let query = search.input.text().trim();
            let selection = min(search.selected + 1, search.row_count().max(1));
            parts.push(if query.is_empty() {
                self.i18n.text_args(
                    "composer-status-history",
                    &agena_tui::fl_args!(
                        "current" => selection as i64,
                        "total" => search.result_count() as i64,
                    ),
                )
            } else {
                self.i18n.text_args(
                    "composer-status-history-query",
                    &agena_tui::fl_args!(
                        "current" => selection as i64,
                        "total" => search.result_count() as i64,
                        "query" => query,
                    ),
                )
            });
        } else if let Some(state) = self.file_mention_suggestions.as_ref() {
            parts.push(self.i18n.text_args(
                "composer-status-mention",
                &agena_tui::fl_args!("query" => ui_text::prefixed_query("@", state.input.text())),
            ));
        } else if let Some(state) = self.slash_command_suggestions.as_ref() {
            parts.push(self.i18n.text_args(
                "composer-status-slash",
                &agena_tui::fl_args!("query" => ui_text::prefixed_query("/", state.input.text())),
            ));
        }
        if let Some(execution) = self.transcript.execution.as_ref() {
            let (permission_count, user_input_count) =
                pending_interactive_counts_for_execution(execution);
            if user_input_count > 0 {
                parts.push(self.i18n.text_args(
                    "composer-status-pending-user-input",
                    &agena_tui::fl_args!(
                        "count" => user_input_count as i64,
                    ),
                ));
            }
            if permission_count > 0 {
                parts.push(self.i18n.text_args(
                    "composer-status-pending-approval",
                    &agena_tui::fl_args!(
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

    pub(crate) fn main_surface_mode_label(&self) -> String {
        let key = if self.focus == Focus::Composer {
            "surface-mode-insert"
        } else if self.transcript.has_active_text_selection() {
            "surface-mode-select"
        } else {
            "surface-mode-navigate"
        };
        ui_text::t(&self.i18n, key)
    }
}

fn transcript_line_is_in_block(line: usize, block: Option<&std::ops::Range<usize>>) -> bool {
    block.is_some_and(|range| line >= range.start && line < range.end)
}

fn transcript_visible_range(
    line_count: usize,
    scroll: usize,
    viewport_height: usize,
) -> std::ops::Range<usize> {
    let start = scroll.min(line_count);
    start..start.saturating_add(viewport_height).min(line_count)
}

fn transcript_surface_top_right_parts(activity: Option<String>, mode: String) -> Vec<String> {
    activity.into_iter().chain(std::iter::once(mode)).collect()
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        transcript_line_is_in_block, transcript_surface_top_right_parts, transcript_visible_range,
    };

    #[test]
    fn block_selection_range_excludes_rows_outside_the_block() {
        let message_body = 4..8;

        assert!(!transcript_line_is_in_block(3, Some(&message_body)));
        assert!(transcript_line_is_in_block(4, Some(&message_body)));
        assert!(!transcript_line_is_in_block(3, None));
    }

    #[test]
    fn activity_indicator_is_rendered_immediately_left_of_insert_mode() {
        assert_eq!(
            transcript_surface_top_right_parts(Some("⠋".to_string()), "INSERT".to_string()),
            vec!["⠋", "INSERT"]
        );
        assert_eq!(
            transcript_surface_top_right_parts(None, "INSERT".to_string()),
            vec!["INSERT"]
        );
    }

    #[test]
    fn transcript_rendering_materializes_only_the_visible_viewport() {
        assert_eq!(
            transcript_visible_range(100_000, 50_000, 40),
            50_000..50_040
        );
        assert_eq!(transcript_visible_range(25, 20, 40), 20..25);
    }
}
use super::{
    App, ComposerEditorSurfaceSpec, ComposerItem, FlashLevel, Frame,
    HeaderBodyFooterTextSurfaceSpec, LayoutCache, Line, Modifier, Paragraph, Rect, Route, Span,
    Style, Text, VerticalSectionSize, Wrap, WrappedTextSpec, apply_block_highlight,
    apply_cursor_cell_highlight, apply_line_cell_highlight, build_wrapped_text_lines,
    composer_item_needs_summary_chip, find_search_ranges, inset_rect, layout_composer_surface,
    layout_header_body_footer_surface, min, pane_header_height,
    pending_interactive_counts_for_execution, render_composer_editor_surface,
    render_header_body_footer_text_surface, render_wrapped_text, sanitize_display_text,
    selection_highlight_style, split_vertical_sections,
};
use crate::ui_text;
use crate::{current_spinner_millis, refresh_spinner_line, spinner_frame};
use agena_tui::main_focus::Focus;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
