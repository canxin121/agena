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
            // The search-picker background renderer resets the layout cache
            // for non-main parents; keep the live frame area for handlers.
            self.layout.overlay_area = area;
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
            self.layout.overlay_area = body;
            self.render_route(frame, body);
            if let Some(footer) = footer {
                self.render_transcript_footer_row(frame, footer);
            }
            self.render_context_help(frame, area);
            return;
        }

        self.render_main_content(frame, area);
        self.layout.overlay_area = area;
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
            overlay_area: area,
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
                | Route::PlanViewer(_)
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
        let header_inner = inset_rect(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::BOTTOM)
                .inner(layout.header),
            1,
            0,
        );
        self.surface_layout.header_title = Rect {
            height: 1,
            ..header_inner
        };
        self.surface_layout.header_subtitle = Rect {
            y: header_inner.y.saturating_add(1),
            height: 1,
            ..header_inner
        };

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
        self.render_surface_selection_highlight(frame, crate::SurfaceSelectionKind::HeaderTitle);
        self.render_surface_selection_highlight(frame, crate::SurfaceSelectionKind::HeaderSubtitle);
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

    pub(crate) fn render_composer(&mut self, frame: &mut Frame, area: Rect) {
        // Status chips live on the border rows: the main status sits at the
        // top border's left corner, the top-right corner holds
        // history/approval, the bottom-left corner holds background activity,
        // and the bottom-right corner holds plan progress. The layout gets no
        // dedicated row.
        let layout = layout_composer_surface(area);
        let texts = self.composer_chip_texts();
        let placements = composer_chip_placements(layout.outer, &texts);
        self.surface_layout.composer_status = placements
            .status
            .as_ref()
            .map(|placement| Rect {
                x: placement.column,
                y: layout.outer.y,
                width: placement.chip_width,
                height: 1,
            })
            .unwrap_or_default();
        self.surface_layout.composer_outer = layout.outer;
        self.surface_layout.composer_editor = Rect {
            x: layout.editor.x.saturating_add(1),
            y: layout.editor.y,
            width: layout.editor.width.saturating_sub(2).max(1),
            height: layout.editor.height,
        };
        let chip = |placement: &agena_tui_components::ComposerStatusPlacement| {
            Line::from(Span::styled(
                placement.text.clone(),
                agena_tui_components::theme::status_chip_style(),
            ))
        };
        let status = placements.status.as_ref().map(chip);
        let status_top_right = placements.top_right.as_ref().map(chip);
        let status_bottom_left = placements.bottom_left.as_ref().map(chip);
        let status_bottom_right = placements.bottom_right.as_ref().map(chip);

        // The border and its chips are drawn by the surface renderer even
        // when the composer is too small to host editor content, so render
        // the empty surface first and bail out.
        if layout.inner.width == 0 || layout.inner.height == 0 {
            render_composer_editor_surface(
                frame,
                layout,
                &ComposerEditorSurfaceSpec {
                    editor_lines: Text::default(),
                    placeholder: None,
                    cursor: None,
                    status,
                    status_top_right,
                    status_bottom_left,
                    status_bottom_right,
                },
            );
            self.render_surface_selection_highlight(
                frame,
                crate::SurfaceSelectionKind::ComposerStatus,
            );
            return;
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
                status,
                status_top_right,
                status_bottom_left,
                status_bottom_right,
            },
        );

        self.render_surface_selection_highlight(frame, crate::SurfaceSelectionKind::ComposerEditor);
        self.render_surface_selection_highlight(frame, crate::SurfaceSelectionKind::ComposerStatus);
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

    pub(crate) fn render_transcript_footer_row(&self, frame: &mut Frame, area: Rect) {
        let Some(spec) = self.transcript_footer_spec() else {
            return;
        };
        render_wrapped_text(frame, area, &spec);
    }

    pub(crate) fn transcript_footer_spec(&self) -> Option<WrappedTextSpec<'static>> {
        if let Some(notice) = self.notifications.banner(crate::notifications::now_ms()) {
            return Some(WrappedTextSpec {
                text: sanitize_display_text(notice.summary.as_str()).into(),
                style: self.notice_style(notice.severity),
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
            let preview = self.queue.preview(28).unwrap_or_default();
            if preview.is_empty() {
                parts.push(ui_text::t(&self.i18n, "transcript-footer-pending"));
            } else {
                parts.push(self.i18n.text_args(
                    "transcript-footer-pending-preview",
                    &agena_tui::fl_args!("preview" => preview),
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
            // The `agena.terminal.*` segments are internal terminal-integration
            // signals (window-title/notification activity like `blocked`,
            // `running`, `idle`). They are not user-facing footer content and
            // their raw payloads must not be rendered above the composer; the
            // terminal integration consumes them separately.
            if segment.segment_id.starts_with("agena.terminal.") {
                continue;
            }
            // Plan progress has a dedicated composer bottom-right chip; keeping
            // it in the footer would duplicate it above the input box. The plan
            // segment is qualified by its contributing session (`plan:{id}`) so
            // stale segments from other sessions are never rendered either.
            if segment.segment_id == "plan" || segment.segment_id.starts_with("plan:") {
                continue;
            }
            if segment.content.trim().is_empty() {
                continue;
            }
            parts.push(segment.content.clone());
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

    pub(crate) fn notice_style(&self, severity: NoticeSeverity) -> Style {
        match severity {
            NoticeSeverity::Success => {
                Style::default().fg(agena_tui_components::theme::success_color())
            }
            NoticeSeverity::Warning => {
                Style::default().fg(agena_tui_components::theme::warning_color())
            }
            NoticeSeverity::Error => {
                Style::default().fg(agena_tui_components::theme::danger_color())
            }
            NoticeSeverity::Info => Style::default().fg(agena_tui_components::theme::info_color()),
        }
    }

    pub(crate) fn composer_status_parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        parts.extend(self.current_session_status_parts());
        if self.transcript.state_loading {
            parts.push(ui_text::t(&self.i18n, "transcript-header-loading"));
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
        if let Some(state) = self.file_mention_suggestions.as_ref() {
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
            let (_, user_input_count) = pending_interactive_counts_for_execution(execution);
            if user_input_count > 0 {
                parts.push(self.i18n.text_args(
                    "composer-status-pending-user-input",
                    &agena_tui::fl_args!(
                        "count" => user_input_count as i64,
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

    /// Text for the composer's bottom-left chip: background-activity count.
    fn composer_background_activity_part(&self) -> Option<String> {
        self.background_activity_summary
            .filter(|(count, _)| *count > 0)
            .map(|(count, _)| format!("● {count} background"))
    }

    /// Text for the composer's top-right chip while history search is active.
    fn composer_history_search_part(&self) -> Option<String> {
        let search = self.prompt_history_search.as_ref()?;
        let query = search.input.text().trim();
        let selection = min(search.selected + 1, search.row_count().max(1));
        Some(if query.is_empty() {
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
        })
    }

    /// Text for the composer's top-right chip while executions await approval.
    fn composer_pending_approval_part(&self) -> Option<String> {
        let execution = self.transcript.execution.as_ref()?;
        let (permission_count, _) = pending_interactive_counts_for_execution(execution);
        (permission_count > 0).then(|| {
            self.i18n.text_args(
                "composer-status-pending-approval",
                &agena_tui::fl_args!("count" => permission_count as i64),
            )
        })
    }

    /// Text for the composer's bottom-right chip: plan progress contributed
    /// by the planning plugin's statusline segment.
    fn composer_plan_progress_part(&self) -> Option<String> {
        let session_id = self.transcript.session_id?;
        let expected_segment = format!("plan:{session_id}");
        self.backend
            .plugin_statusline_segments()
            .into_iter()
            .find(|segment| segment.segment_id == expected_segment)
            .map(|segment| segment.content.trim().to_string())
            .filter(|content| !content.is_empty())
    }

    /// The four status-chip texts for the current frame. The top-left border
    /// chip keeps the remaining session status parts; the corner chips carry
    /// background activity (bottom-left), history search or pending approval
    /// (top-right), and plan progress (bottom-right).
    fn composer_chip_texts(&self) -> ComposerChipTexts {
        ComposerChipTexts {
            status: sanitize_display_text(self.composer_status_parts().join("  |  ").as_str()),
            top_right: self
                .composer_history_search_part()
                .or_else(|| self.composer_pending_approval_part())
                .map(|text| sanitize_display_text(text.as_str())),
            bottom_left: self
                .composer_background_activity_part()
                .map(|text| sanitize_display_text(text.as_str())),
            bottom_right: self
                .composer_plan_progress_part()
                .map(|text| sanitize_display_text(text.as_str())),
        }
    }

    /// Project the currently displayed text of a selectable chat surface into
    /// absolute-cell display lines. The projection must match the renderers
    /// exactly so mouse selection copies what the user sees.
    pub(crate) fn surface_display_lines(
        &self,
        kind: crate::SurfaceSelectionKind,
    ) -> Vec<crate::SurfaceDisplayLine> {
        let layout = self.surface_layout;
        match kind {
            crate::SurfaceSelectionKind::HeaderTitle => {
                let title = sanitize_display_text(self.transcript_surface_title());
                let right =
                    sanitize_display_text(self.transcript_surface_top_right().join("  ·  "));
                vec![header_row_display_line(layout.header_title, title, right)]
            }
            crate::SurfaceSelectionKind::HeaderSubtitle => {
                let text = self
                    .current_session_path_label()
                    .map(|path| sanitize_display_text(path.as_str()))
                    .unwrap_or_default();
                let displayed = truncate_display_text_middle(
                    text.as_str(),
                    layout.header_subtitle.width as usize,
                );
                vec![crate::SurfaceDisplayLine {
                    text: displayed,
                    row: layout.header_subtitle.y,
                    column: layout.header_subtitle.x,
                }]
            }
            crate::SurfaceSelectionKind::ComposerStatus => {
                let texts = self.composer_chip_texts();
                composer_chip_placements(layout.composer_outer, &texts)
                    .status
                    .map(|placement| {
                        vec![crate::SurfaceDisplayLine {
                            text: placement.text,
                            row: layout.composer_outer.y,
                            column: placement.text_column,
                        }]
                    })
                    .unwrap_or_default()
            }
            crate::SurfaceSelectionKind::ComposerEditor => {
                let area = layout.composer_editor;
                let view = self
                    .composer
                    .render_wrapped_view(area.width.max(1), area.height.max(1));
                view.lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, line)| crate::SurfaceDisplayLine {
                        text: line_plain_text(&line),
                        row: area.y.saturating_add(index as u16),
                        column: area.x,
                    })
                    .collect()
            }
        }
    }

    /// Re-render the selected cells of one chat surface with an inverse-video
    /// highlight. The base render already drew the text; this overlay draws the
    /// same glyphs with the selected spans reversed so the selection is visible
    /// without duplicating or restyling the rest of the surface.
    fn render_surface_selection_highlight(
        &self,
        frame: &mut Frame,
        kind: crate::SurfaceSelectionKind,
    ) {
        let Some(selection) = self
            .surface_selection
            .filter(|selection| selection.kind == kind)
        else {
            return;
        };
        let area = self.surface_layout.rect_for(kind);
        if area.width == 0 || area.height == 0 {
            return;
        }
        match kind {
            crate::SurfaceSelectionKind::HeaderTitle
            | crate::SurfaceSelectionKind::HeaderSubtitle => {
                let lines = self.surface_display_lines(kind);
                let ranges = crate::surface_selection_ranges(&lines, &selection);
                let base_style = match kind {
                    crate::SurfaceSelectionKind::HeaderTitle => {
                        Style::default().add_modifier(Modifier::BOLD)
                    }
                    crate::SurfaceSelectionKind::HeaderSubtitle => Style::default()
                        .fg(agena_tui_components::theme::muted_color())
                        .add_modifier(Modifier::DIM | Modifier::ITALIC),
                    _ => Style::default(),
                };
                for (line, range) in lines.iter().zip(ranges) {
                    if let Some(range) = range {
                        let rendered = crate::apply_cell_range_highlight(
                            Line::from(Span::styled(line.text.clone(), base_style)),
                            Some(range),
                        );
                        frame.render_widget(
                            Paragraph::new(rendered),
                            Rect {
                                x: area.x,
                                y: line.row,
                                width: area.width,
                                height: 1,
                            },
                        );
                    }
                }
            }
            crate::SurfaceSelectionKind::ComposerStatus => {
                let lines = self.surface_display_lines(kind);
                let ranges = crate::surface_selection_ranges(&lines, &selection);
                let base_style = agena_tui_components::theme::status_chip_style();
                for (line, range) in lines.iter().zip(ranges) {
                    if let Some(range) = range {
                        let rendered = crate::apply_cell_range_highlight(
                            Line::from(Span::styled(line.text.clone(), base_style)),
                            Some(range),
                        );
                        frame.render_widget(
                            Paragraph::new(rendered),
                            Rect {
                                x: line.column,
                                y: line.row,
                                width: line.width(),
                                height: 1,
                            },
                        );
                    }
                }
            }
            crate::SurfaceSelectionKind::ComposerEditor => {
                let view = self
                    .composer
                    .render_wrapped_view(area.width.max(1), area.height.max(1));
                let lines = self.surface_display_lines(kind);
                let ranges = crate::surface_selection_ranges(&lines, &selection);
                let highlighted = view
                    .lines
                    .into_iter()
                    .enumerate()
                    .map(|(index, line)| {
                        if let Some(range) = ranges.get(index).and_then(Clone::clone) {
                            crate::apply_cell_range_highlight(line, Some(range))
                        } else {
                            line
                        }
                    })
                    .collect::<Vec<_>>();
                frame.render_widget(
                    Paragraph::new(Text::from(highlighted)),
                    Rect {
                        x: area.x,
                        y: area.y,
                        width: area.width,
                        height: area.height,
                    },
                );
            }
        }
    }

    pub(crate) fn main_surface_mode_label(&self) -> String {
        let key = if self.focus == Focus::Composer {
            "surface-mode-insert"
        } else if self.transcript.has_active_text_selection() || self.surface_selection.is_some() {
            "surface-mode-select"
        } else {
            "surface-mode-navigate"
        };
        ui_text::t(&self.i18n, key)
    }
}

/// The four status-chip texts for the current composer frame. The centered
/// top-border chip keeps the session-status parts; the corner chips carry
/// background activity (bottom-left), history search or pending approval
/// (top-right), and plan progress (bottom-right).
#[derive(Debug, Clone, Default)]
struct ComposerChipTexts {
    status: String,
    top_right: Option<String>,
    bottom_left: Option<String>,
    bottom_right: Option<String>,
}

/// Geometry of the four composer chips for the current frame. Produced by
/// [`composer_chip_placements`] with the same placement functions the surface
/// renderer uses, so the selection/copy projection matches the drawn pixels.
#[derive(Debug, Clone, Default)]
struct ComposerChipPlacements {
    status: Option<ComposerStatusPlacement>,
    top_right: Option<ComposerStatusPlacement>,
    bottom_left: Option<ComposerStatusPlacement>,
    bottom_right: Option<ComposerStatusPlacement>,
}

/// Computes where each composer chip sits on its border row. Order matters:
/// the top-right chip is placed first so the left status chip can reserve its
/// width, mirroring `render_composer_editor_surface`.
fn composer_chip_placements(outer: Rect, texts: &ComposerChipTexts) -> ComposerChipPlacements {
    let top_right = texts
        .top_right
        .as_deref()
        .and_then(|text| composer_corner_placement_right(outer, text));
    let status = composer_status_placement_left(
        outer,
        texts.status.as_str(),
        top_right
            .as_ref()
            .map(|placement| placement.chip_width)
            .unwrap_or(0),
    );
    let bottom_left = texts
        .bottom_left
        .as_deref()
        .and_then(|text| composer_corner_placement_left(outer, text));
    let bottom_right = texts
        .bottom_right
        .as_deref()
        .and_then(|text| composer_corner_placement_right(outer, text));
    ComposerChipPlacements {
        status,
        top_right,
        bottom_left,
        bottom_right,
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

/// Reconstruct the visible header row exactly as `render_header_row` lays it
/// out: the title (truncated to the left budget) followed by the right-side
/// text right-aligned in its reserved column. Used both to copy mouse
/// selections and to render their highlight.
fn header_row_display_line(area: Rect, left: String, right: String) -> crate::SurfaceDisplayLine {
    let text = if right.trim().is_empty() {
        agena_tui_components::text::truncate_display_text(left.as_str(), area.width as usize)
    } else {
        let max_right_width = if area.width < 52 {
            area.width.saturating_div(2).max(8)
        } else {
            area.width.saturating_mul(2).saturating_div(5).max(16)
        };
        let truncated_right = agena_tui_components::text::truncate_display_text(
            right.as_str(),
            max_right_width as usize,
        );
        let right_width = UnicodeWidthStr::width(truncated_right.as_str()).saturating_add(1) as u16;
        let left_budget = area
            .width
            .saturating_sub(right_width)
            .saturating_sub(1)
            .max(1);
        let truncated_left =
            agena_tui_components::text::truncate_display_text(left.as_str(), left_budget as usize);
        let left_text_width = UnicodeWidthStr::width(truncated_left.as_str()) as u16;
        let right_text_width = UnicodeWidthStr::width(truncated_right.as_str()) as u16;
        let gap = area
            .width
            .saturating_sub(left_text_width)
            .saturating_sub(right_text_width);
        format!(
            "{truncated_left}{}{truncated_right}",
            " ".repeat(gap as usize)
        )
    };
    crate::SurfaceDisplayLine {
        text,
        row: area.y,
        column: area.x,
    }
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
    App, ComposerEditorSurfaceSpec, ComposerStatusPlacement, Frame,
    HeaderBodyFooterTextSurfaceSpec, LayoutCache, Line, Modifier, Paragraph, Rect, Route, Span,
    Style, Text, VerticalSectionSize, WrappedTextSpec, apply_block_highlight,
    apply_cursor_cell_highlight, apply_line_cell_highlight, build_wrapped_text_lines,
    composer_corner_placement_left, composer_corner_placement_right,
    composer_status_placement_left, find_search_ranges, inset_rect, layout_composer_surface,
    layout_header_body_footer_surface, min, pane_header_height,
    pending_interactive_counts_for_execution, render_composer_editor_surface,
    render_header_body_footer_text_surface, render_wrapped_text, sanitize_display_text,
    split_vertical_sections,
};
use crate::NoticeSeverity;
use crate::ui_text;
use crate::{current_spinner_millis, refresh_spinner_line, spinner_frame};
use agena_tui::main_focus::Focus;
use agena_tui_components::text::{line_plain_text, truncate_display_text_middle};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};
use unicode_width::UnicodeWidthStr;
