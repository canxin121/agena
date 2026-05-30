use super::*;
use agena_tui_components::{
    BoundedListPanelHeight, ComposerEditorSurfaceSpec, DashboardDetailOverlaySpec,
    DashboardLeadPanelSpec, DashboardListPanelHeight, DashboardListPanelState,
    DashboardSplitPanelsSpec, DashboardTextPanelHeight, DashboardTextSection,
    DashboardWorkbenchOverlaySpec, DashboardWorkbenchSpec, DecisionDialogSpec,
    DetailTextDialogSpec, DetailTextLine, DetailTextSpec, EditorDialogSpec,
    EditorPreviewDialogSpec, EditorPreviewHelpSpec, FramedSurfaceSpec,
    HeaderBodyFooterTextSurfaceSpec, HeaderRowSpec, LineTextDialogSpec, ListPanelSpec,
    ListWorkbenchDialogSpec, ListWorkbenchPanelState, QuerySuggestionPopupSpec,
    QuestionFlowCustomInputSpec, QuestionFlowDialogMode, QuestionFlowDialogSpec,
    SearchListDialogSpec, SearchPanelsDialogSpec, SearchPanelsDialogState, SuggestionPopupItem,
    SuggestionPopupSpec, SurfaceMode, TextDialogLine, TextPanelSpec, VerticalSectionSize,
    WorkbenchTextSection, WrappedTextSpec, adaptive_detail_split, adaptive_modal_width,
    build_accented_two_line_list_item, build_detail_two_line_list_item, build_wrapped_text_lines,
    inset_rect, join_inline_segments, layout_composer_surface, layout_header_body_footer_surface,
    pane_header_height, render_composer_editor_surface, render_dashboard_workbench_dialog,
    render_decision_dialog, render_editor_dialog, render_editor_preview_dialog,
    render_framed_surface, render_header_body_footer_text_surface, render_header_row,
    render_line_text_dialog, render_list_panel, render_list_workbench_dialog,
    render_overlay_line_input_dialog, render_query_suggestion_popup, render_question_flow_dialog,
    render_search_list_dialog, render_search_panels_dialog, render_suggestion_popup,
    render_text_panel, render_wrapped_text, split_vertical_sections, truncate_display_text,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, ListItem, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

impl App {
    pub(super) fn draw(&mut self, frame: &mut Frame) {
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

        let composer_height = self.composer_height();
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
        let layout = layout_header_body_footer_surface(
            area,
            pane_header_height(area.height),
            footer_height,
            1,
        );

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
                right: Some(right.into()),
                body: Text::from(lines),
                body_scroll: (min(self.transcript.scroll, u16::MAX as usize) as u16, 0),
                body_wrap: false,
                footer,
                title_style: Style::default().add_modifier(Modifier::BOLD),
                right_style: Style::default().fg(Color::DarkGray),
            },
        );
    }

    fn transcript_surface_top_right(&self) -> Vec<String> {
        vec![self.main_surface_mode_label()]
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
        let item_rows = u16::from(!self.composer_items.is_empty());
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

        let editor_view = self.composer.render_view(
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
                Style::default().fg(Color::DarkGray),
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
                    .fg(self.theme_color("accent", Color::Cyan))
                    .add_modifier(Modifier::BOLD),
                detail_style: Style::default().fg(Color::DarkGray),
                pad_selected_row: true,
            },
        );
    }

    fn render_file_mention_suggestions(&self, frame: &mut Frame, area: Rect) {
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
                    .fg(self.theme_color("flash_info", Color::Cyan))
                    .add_modifier(Modifier::BOLD),
                detail_style: Style::default().fg(Color::DarkGray),
                pad_selected_row: true,
            },
        );
    }

    fn render_prompt_history_search(&self, frame: &mut Frame, area: Rect) {
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
                    .fg(self.theme_color("accent", Color::Cyan))
                    .add_modifier(Modifier::BOLD),
                query_style: Style::default(),
                empty_style: Style::default().fg(Color::DarkGray),
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
                    prefix_style: Style::default().fg(Color::DarkGray),
                    selected_prefix_style: Some(Style::default().fg(Color::DarkGray)),
                    label_style: Style::default(),
                    detail_style: Style::default().fg(Color::DarkGray),
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
        let Some(spec) = self.transcript_footer_spec() else {
            return;
        };
        render_wrapped_text(frame, area, &spec);
    }

    fn transcript_footer_spec(&self) -> Option<WrappedTextSpec<'static>> {
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
            style: Style::default().fg(Color::DarkGray),
        })
    }

    fn transcript_footer_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.transcript_footer_spec()
            .map(|spec| build_wrapped_text_lines(&spec, width))
            .unwrap_or_default()
    }

    fn transcript_footer_text(&self) -> String {
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

    fn transcript_footer_height(&self, width: u16, total_height: u16) -> u16 {
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

    fn composer_popup_rows(&self) -> u16 {
        if self.overlay.is_some() || self.focus != Focus::Composer {
            return 0;
        }
        if let Some(search) = self.prompt_history_search.as_ref() {
            let result_rows = if search.items.is_empty() {
                1
            } else {
                min(MAX_PROMPT_HISTORY_SEARCH_RESULTS, search.items.len())
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

    fn render_composer_status_row(&self, frame: &mut Frame, area: Rect) {
        let text = self.composer_status_parts().join("  |  ");
        if text.trim().is_empty() {
            return;
        }
        render_wrapped_text(
            frame,
            area,
            &WrappedTextSpec {
                text: sanitize_display_text(text.as_str()).into(),
                style: Style::default().fg(Color::DarkGray),
            },
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
                        "total" => search.items.len() as i64,
                    ),
                )
            } else {
                self.i18n.text_args(
                    "composer-status-history-query",
                    &crate::fl_args!(
                        "current" => selection as i64,
                        "total" => search.items.len() as i64,
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

    fn main_surface_mode_label(&self) -> String {
        if self.focus == Focus::Composer {
            ui_text::t(&self.i18n, "surface-mode-insert")
        } else {
            ui_text::t(&self.i18n, "surface-mode-view")
        }
    }

    fn theme_color(&self, key: &str, fallback: Color) -> Color {
        self.plugin_theme
            .as_ref()
            .and_then(|theme| theme.colors.get(key))
            .and_then(|value| parse_tui_color(value))
            .unwrap_or(fallback)
    }

    fn render_overlay(&self, frame: &mut Frame, area: Rect) {
        let Some(overlay) = &self.overlay else {
            return;
        };

        match overlay {
            Overlay::TranscriptSearch(dialog) | Overlay::SessionRename(dialog) => {
                render_overlay_line_input_dialog(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    sanitize_display_text(dialog.title.as_str()).into(),
                    sanitize_display_text(dialog.prompt.as_str()).into(),
                    sanitize_display_text(ui_text::t(&self.i18n, "overlay-line-footer")).into(),
                    &dialog.input,
                );
            }
            Overlay::SettingsValueEdit(dialog) => {
                render_overlay_line_input_dialog(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    sanitize_display_text(dialog.title.as_str()).into(),
                    sanitize_display_text(dialog.prompt.as_str()).into(),
                    sanitize_display_text(ui_text::t(&self.i18n, "overlay-line-footer")).into(),
                    &dialog.input,
                );
            }
            Overlay::RuntimeSettingEdit(dialog) => {
                render_overlay_line_input_dialog(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    sanitize_display_text(dialog.title.as_str()).into(),
                    sanitize_display_text(dialog.prompt.as_str()).into(),
                    sanitize_display_text(ui_text::t(&self.i18n, "overlay-line-footer")).into(),
                    &dialog.input,
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
            Overlay::PathBrowser(dialog) => {
                self.render_path_browser_overlay(frame, area, dialog);
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
            Route::PermissionStudio(dialog) => {
                self.render_permission_studio_overlay(frame, area, dialog, SurfaceMode::Route);
            }
            Route::PermissionRuleStudio(dialog) => {
                self.render_permission_rule_studio_overlay(frame, area, dialog, SurfaceMode::Route);
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
            Route::PluginWorkbench(dialog) => {
                self.render_plugin_workbench(frame, area, dialog, SurfaceMode::Route);
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

    fn render_permission_rule_edit_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionRuleEditOverlay,
    ) {
        let prompt_body = Text::from(sanitize_display_text(dialog.state.prompt.as_str()));
        let help_body = Text::from(permission_rule_edit_help());
        let preview_body = Text::from(render_permission_rule_preview(
            &self.i18n,
            dialog.state.input.text(),
        ));
        let result = render_editor_preview_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            &EditorPreviewDialogSpec::new(
                sanitize_display_text(dialog.state.title.as_str()).into(),
                82,
                &prompt_body,
                (1, 2),
                &dialog.state.input,
                &preview_body,
                (2, 8),
            )
            .with_help(
                EditorPreviewHelpSpec::new(&help_body, (3, 6))
                    .with_wrap(true)
                    .with_borders(Borders::BOTTOM),
            )
            .with_input_borders(Borders::BOTTOM)
            .with_cursor(true),
        );
        if let Some(cursor) = result.cursor {
            frame.set_cursor_position(cursor);
        }
    }

    fn render_file_attach_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &FileAttachOverlay,
    ) {
        render_search_list_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            dialog,
            &SearchListDialogSpec::new(
                ui_text::t(&self.i18n, "overlay-picker-loading").into(),
                selection_highlight_style(),
                ">> ".into(),
            )
            .with_list_title(ui_text::t(&self.i18n, "overlay-attach-matches").into()),
            sanitize_display_str,
        );
    }

    fn render_path_browser_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PathBrowserOverlay,
    ) {
        render_search_list_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            dialog,
            &SearchListDialogSpec::new(
                ui_text::t(&self.i18n, "overlay-picker-loading").into(),
                selection_highlight_style(),
                ">> ".into(),
            )
            .with_list_title(ui_text::t(&self.i18n, "overlay-path-browser-list-title").into()),
            sanitize_display_str,
        );
    }

    fn render_permission_overlay(&self, frame: &mut Frame, area: Rect, dialog: &PermissionOverlay) {
        let body_lines = permission_overlay_body_lines(&self.i18n, dialog);
        let choices = permission_overlay_choices(&self.i18n);
        let items = choices
            .iter()
            .map(|label| ListItem::new(label.clone()))
            .collect::<Vec<_>>();
        let body = Text::from(body_lines);
        let footer = Text::from(self.i18n.text_args(
            "overlay-permission-footer-edit-rule",
            &crate::fl_args!(
                "footer" => ui_text::t(&self.i18n, "overlay-permission-footer")
            ),
        ));
        render_decision_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            &DecisionDialogSpec::new(
                ui_text::t(&self.i18n, "overlay-permission-title").into(),
                &body,
                items.as_slice(),
                &footer,
                84,
                selection_highlight_style(),
                ">> ".into(),
            )
            .with_body_height_bounds((4, 14))
            .with_list_state(Some(dialog.selection.selected), 1, (4, 8))
            .with_footer_height_bounds((1, 1)),
        );
    }

    fn render_user_input_overlay(&self, frame: &mut Frame, area: Rect, dialog: &UserInputOverlay) {
        let target_width = adaptive_modal_width(area.width, 92);
        let footer_review = ui_text::t(&self.i18n, "overlay-user-input-footer-review");
        let footer_question = ui_text::t(&self.i18n, "overlay-user-input-footer-question");
        let nav_color = self.theme_color("flash_info", Color::Cyan);
        let title = ui_text::t(&self.i18n, "overlay-user-input-title");
        if dialog.state.screen() == QuestionFlowScreen::Review {
            let nav_body = Text::from(vec![
                Line::from(Span::styled(
                    sanitize_display_text(self.i18n.text_args(
                        "overlay-user-input-request-id",
                        &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
                    )),
                    Style::default().fg(Color::DarkGray),
                )),
                user_input_nav_line(&self.i18n, dialog, nav_color),
            ]);

            let mut review_lines = vec![Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-user-input-review-intro"),
                Style::default().add_modifier(Modifier::BOLD),
            ))];
            for (index, question) in dialog.request.questions.iter().enumerate() {
                let values = dialog
                    .answers
                    .get(&question.id)
                    .map(|draft| user_input_answer_values(question, draft))
                    .unwrap_or_default();
                let answered = !values.is_empty();
                let style = if index == dialog.state.selected_question() {
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
                        user_input_review_answer_preview(&self.i18n, values.as_slice())
                    ),
                    if answered {
                        Style::default().fg(nav_color)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                )));
            }
            let review_body = Text::from(review_lines);
            let footer = Text::from(footer_review);
            render_question_flow_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                &QuestionFlowDialogSpec::new(
                    title.clone().into(),
                    92,
                    ui_text::t(&self.i18n, "overlay-user-input-questions").into(),
                    QuestionFlowDialogMode::review(
                        ui_text::t(&self.i18n, "overlay-user-input-summary").into(),
                        &review_body,
                        &footer,
                    ),
                )
                .with_nav_body(&nav_body),
            );
            return;
        }

        let Some(question) = dialog
            .request
            .questions
            .get(dialog.state.selected_question())
        else {
            let detail_body = Text::from(sanitize_display_text(ui_text::t(
                &self.i18n,
                "overlay-user-input-no-questions",
            )));
            render_question_flow_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                &QuestionFlowDialogSpec::new(
                    title.clone().into(),
                    92,
                    ui_text::t(&self.i18n, "overlay-user-input-questions").into(),
                    QuestionFlowDialogMode::empty(
                        ui_text::t(&self.i18n, "overlay-user-input-detail").into(),
                        &detail_body,
                        12,
                    ),
                ),
            );
            return;
        };

        let nav_body = Text::from(vec![
            Line::from(Span::styled(
                sanitize_display_text(self.i18n.text_args(
                    "overlay-user-input-request-id",
                    &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
                )),
                Style::default().fg(Color::DarkGray),
            )),
            user_input_nav_line(&self.i18n, dialog, nav_color),
        ]);

        let draft = dialog
            .answers
            .get(&question.id)
            .cloned()
            .unwrap_or_default();
        let answer_summary = user_input_answer_summary(&self.i18n, question, &draft);
        let unanswered = ui_text::t(&self.i18n, "overlay-user-input-unanswered");
        let prompt_body = Text::from(vec![
            Line::from(Span::styled(
                sanitize_display_text(question.question.as_str()),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                sanitize_display_text(format!(
                    "{} · id={}",
                    ui_text::t(
                        &self.i18n,
                        if question.multiple {
                            "overlay-user-input-choice-multiple"
                        } else {
                            "overlay-user-input-choice-single"
                        },
                    ),
                    question.id
                )),
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(vec![
                Span::styled(
                    format!(
                        "{} ",
                        ui_text::t(&self.i18n, "overlay-user-input-current-answer")
                    ),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    sanitize_display_text(answer_summary.as_str()),
                    if answer_summary == unanswered {
                        Style::default().fg(Color::DarkGray)
                    } else {
                        Style::default().fg(nav_color)
                    },
                ),
            ]),
        ]);

        let mut option_lines = Vec::new();
        let choice_preview_width = target_width.saturating_sub(8);
        for (index, option) in question.options.iter().enumerate() {
            let picked = draft.option_indexes.contains(&index);
            let focused = index == dialog.state.selected_option() && !dialog.editing_custom;
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
                            choice_preview_width,
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
                &self.i18n,
                &draft.custom_values,
                choice_preview_width,
            );
            let custom_selected =
                custom_row == dialog.state.selected_option() && !dialog.editing_custom;
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
                Span::styled(
                    ui_text::t(&self.i18n, "overlay-user-input-other"),
                    custom_style.add_modifier(Modifier::BOLD),
                ),
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
        let choices_body = Text::from(option_lines);
        let footer = Text::from(footer_question);
        let custom_input = question.allow_custom.then(|| {
            QuestionFlowCustomInputSpec::new(
                ui_text::t(
                    &self.i18n,
                    if dialog.editing_custom {
                        "overlay-user-input-custom-input"
                    } else {
                        "overlay-user-input-custom-input-hint"
                    },
                )
                .into(),
                &dialog.custom_input,
                dialog.editing_custom,
            )
        });
        let result = render_question_flow_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            &QuestionFlowDialogSpec::new(
                title.into(),
                92,
                ui_text::t(&self.i18n, "overlay-user-input-questions").into(),
                QuestionFlowDialogMode::question(
                    ui_text::t(&self.i18n, "overlay-user-input-prompt-panel").into(),
                    &prompt_body,
                    ui_text::t(&self.i18n, "overlay-user-input-choices").into(),
                    &choices_body,
                    custom_input,
                    &footer,
                ),
            )
            .with_nav_body(&nav_body),
        );
        if let Some(cursor) = result.cursor {
            frame.set_cursor_position(cursor);
        }
    }

    fn render_confirm_overlay(&self, frame: &mut Frame, area: Rect, dialog: &ConfirmOverlay) {
        let lines = dialog
            .body_lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    TextDialogLine::styled(
                        sanitize_display_text(line.as_str()),
                        Style::default().add_modifier(Modifier::BOLD),
                    )
                } else {
                    TextDialogLine::plain(sanitize_display_text(line.as_str()))
                }
            })
            .collect::<Vec<_>>();
        let spec = LineTextDialogSpec::new(
            sanitize_display_text(dialog.title.as_str()).into(),
            lines.as_slice(),
            76,
        )
        .with_body_wrap(true)
        .with_body_height_bounds((2, 10))
        .with_footer(
            sanitize_display_text(dialog.footer.as_str()).into(),
            (1, 1),
            Some(Alignment::Right),
            Style::default(),
        );
        render_line_text_dialog(frame, area, SurfaceMode::Overlay, &spec);
    }

    fn render_help_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &HelpOverlay,
        surface: SurfaceMode,
    ) {
        let lines = ui_text::help_lines(&self.i18n)
            .into_iter()
            .map(|line| match line.kind {
                ui_text::HelpLineKind::Header => TextDialogLine::styled(
                    sanitize_display_text(line.text),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                ui_text::HelpLineKind::Section => TextDialogLine::styled(
                    sanitize_display_text(line.text),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                ui_text::HelpLineKind::Body => {
                    TextDialogLine::plain(sanitize_display_text(line.text))
                }
                ui_text::HelpLineKind::Spacer => TextDialogLine::plain(""),
            })
            .collect::<Vec<_>>();
        let spec = LineTextDialogSpec::new(
            sanitize_display_text(ui_text::t(&self.i18n, "help-title")).into(),
            lines.as_slice(),
            132,
        )
        .with_body_wrap(true)
        .with_body_height_bounds((8, 22))
        .with_body_scroll((dialog.scroll, 0))
        .with_footer(
            ui_text::t(&self.i18n, "overlay-help-footer").into(),
            (1, 2),
            Some(Alignment::Right),
            Style::default().fg(Color::DarkGray),
        );
        render_line_text_dialog(frame, area, surface, &spec);
    }

    fn render_session_search_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SessionSearchOverlay,
        surface: SurfaceMode,
    ) {
        render_search_list_dialog(
            frame,
            area,
            surface,
            dialog,
            &SearchListDialogSpec::new(
                ui_text::t(&self.i18n, "overlay-picker-loading").into(),
                selection_highlight_style(),
                ">> ".into(),
            ),
            sanitize_display_str,
        );
    }

    fn render_picker_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PickerOverlay,
        surface: SurfaceMode,
    ) {
        render_search_list_dialog(
            frame,
            area,
            surface,
            dialog,
            &SearchListDialogSpec::new(
                ui_text::t(&self.i18n, "overlay-picker-loading").into(),
                selection_highlight_style(),
                ">> ".into(),
            ),
            sanitize_display_str,
        );
    }

    fn render_session_model_chooser_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SessionModelChooserOverlay,
        surface: SurfaceMode,
    ) {
        render_search_list_dialog(
            frame,
            area,
            surface,
            dialog,
            &SearchListDialogSpec::new(
                ui_text::t(&self.i18n, "overlay-picker-loading").into(),
                selection_highlight_style(),
                ">> ".into(),
            )
            .with_list_title(ui_text::t(&self.i18n, "overlay-session-model-list-title").into()),
            sanitize_display_str,
        );
    }

    fn render_choice_overlay(&self, frame: &mut Frame, area: Rect, dialog: &ChoiceOverlay) {
        render_search_list_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            dialog,
            &SearchListDialogSpec::new(
                ui_text::t(&self.i18n, "overlay-picker-loading").into(),
                selection_highlight_style(),
                ">> ".into(),
            ),
            sanitize_display_str,
        );
    }

    fn render_timeline_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &TimelineOverlay,
        surface: SurfaceMode,
    ) {
        let loading_message =
            sanitize_display_text(ui_text::t(&self.i18n, "overlay-picker-loading"));
        let spec = SearchPanelsDialogSpec::new(
            122,
            ui_text::t(&self.i18n, "overlay-timeline-events").into(),
            1,
            (5, 10),
            40,
            46,
            loading_message.clone().into(),
            selection_highlight_style(),
            ">> ".into(),
        );
        render_search_panels_dialog(
            frame,
            area,
            surface,
            dialog,
            &spec,
            |item| ListItem::new(sanitize_display_text(item.summary.as_str())),
            |state| {
                let detail = match state {
                    SearchPanelsDialogState::Loading { message }
                    | SearchPanelsDialogState::Empty { message } => {
                        Text::from(sanitize_display_text(message))
                    }
                    SearchPanelsDialogState::Selected(item) => item.detail_body.clone(),
                };
                vec![WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-timeline-detail").into(),
                    detail,
                    4,
                    12,
                )]
            },
        );
    }

    fn render_settings_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SettingsStudioOverlay,
        surface: SurfaceMode,
    ) {
        let current_section = dialog.state.selected_section();
        let section_title = current_section
            .map(|section| section.label.clone())
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-settings-default-section-title"));

        let framed = render_framed_surface(
            frame,
            area,
            surface,
            &FramedSurfaceSpec {
                title: sanitize_display_text(dialog.title.as_str()).into(),
                target_width: 150,
                target_height: 42,
            },
        );
        let page_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(1)])
            .split(framed.inner);
        let block = Block::default()
            .title(format!(
                " {} / {} ",
                sanitize_display_text(dialog.title.as_str()),
                sanitize_display_text(section_title.as_str())
            ))
            .borders(Borders::ALL);
        let inner = block.inner(page_rows[0]);
        frame.render_widget(block, page_rows[0]);

        let nav_width = inner
            .width
            .saturating_mul(3)
            .saturating_div(10)
            .clamp(18, 30)
            .min(inner.width.saturating_sub(1));
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(nav_width),
                Constraint::Length(u16::from(inner.width > 0)),
                Constraint::Min(24),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(settings_compact_sections_text(
                &self.i18n,
                dialog,
                body[0].width,
            ))
            .wrap(Wrap { trim: false }),
            body[0],
        );
        frame.render_widget(
            Paragraph::new(settings_compact_vertical_divider(body[1].height))
                .wrap(Wrap { trim: false }),
            body[1],
        );
        frame.render_widget(
            Paragraph::new(settings_compact_editor_text(
                &self.i18n,
                dialog,
                current_section,
                body[2].width,
                body[2].height,
            ))
            .wrap(Wrap { trim: false }),
            body[2],
        );
        frame.render_widget(
            Paragraph::new(sanitize_display_text(dialog.footer.as_str()))
                .wrap(Wrap { trim: false }),
            page_rows[1],
        );
    }

    fn render_agent_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &AgentStudioOverlay,
        surface: SurfaceMode,
    ) {
        let title_summary = format!(
            "{} · {}",
            dialog.profile.scope.as_str(),
            if dialog.editable {
                ui_text::t(&self.i18n, "value-config-owned")
            } else {
                ui_text::t(&self.i18n, "value-file-backed")
            }
        );
        let overview_body = agent_studio_overview_text(
            &self.i18n,
            &dialog.profile,
            dialog.default_agent_name.as_deref(),
            dialog.editable,
        );
        let selected_item = dialog.workbench.list.selected_item();
        let detail_body = selected_item
            .map(|item| {
                agent_studio_item_detail_text(
                    &self.i18n,
                    &dialog.profile,
                    item,
                    dialog.editable,
                    dialog.default_agent_name.as_deref(),
                )
            })
            .unwrap_or_else(|| {
                Text::from(ui_text::t(&self.i18n, "overlay-agent-studio-empty-detail"))
            });

        let list_items = dialog
            .workbench
            .list
            .items
            .iter()
            .map(|item| {
                build_accented_two_line_list_item(
                    sanitize_display_text(item.label.as_str()).into(),
                    Some(sanitize_display_text(item.value.as_str()).into()),
                    Some(sanitize_display_text(item.detail.as_str()).into()),
                )
            })
            .collect::<Vec<_>>();
        let spec = ListWorkbenchDialogSpec::new(
            sanitize_display_text(dialog.workbench.title.as_str()).into(),
            sanitize_display_text(dialog.workbench.footer.as_str()).into(),
            138,
            36,
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 2,
                    min_body_height: 6,
                    max_body_height: 16,
                },
                Some(ui_text::t(&self.i18n, "overlay-agent-studio-fields").into()),
                list_items.as_slice(),
                (!dialog.workbench.list.items.is_empty()).then_some(dialog.workbench.list.selected),
                selection_highlight_style(),
                ">> ".into(),
            ),
            vec![
                WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-workbench-overview").into(),
                    overview_body,
                    2,
                    5,
                ),
                WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-workbench-details").into(),
                    detail_body,
                    3,
                    10,
                ),
            ],
        )
        .with_summary(sanitize_display_text(title_summary.as_str()).into());
        let spec = spec.with_optional_overlay_source(dialog.workbench.editor.as_ref());
        render_list_workbench_dialog(frame, area, surface, &spec);
    }

    fn render_permission_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionStudioOverlay,
        surface: SurfaceMode,
    ) {
        let framed = render_framed_surface(
            frame,
            area,
            surface,
            &FramedSurfaceSpec {
                title: sanitize_display_text(dialog.title.as_str()).into(),
                target_width: 140,
                target_height: area.height,
            },
        );
        if framed.inner.width == 0 || framed.inner.height == 0 {
            return;
        }

        let header_height = if framed.inner.height >= 4 { 2 } else { 1 };
        let footer_height = if framed.inner.height >= 3 { 1 } else { 0 };
        let body_height = framed
            .inner
            .height
            .saturating_sub(header_height)
            .saturating_sub(footer_height);
        let sections = split_vertical_sections(
            framed.inner,
            &[
                VerticalSectionSize::Fixed(header_height),
                VerticalSectionSize::Flexible(body_height.max(1)),
                VerticalSectionSize::Fixed(footer_height),
            ],
        );
        let header_area = sections.first().copied().unwrap_or(framed.inner);
        let body_area = sections.get(1).copied().unwrap_or(framed.inner);
        let footer_area = sections.get(2).copied().unwrap_or(Rect {
            x: framed.inner.x,
            y: framed
                .inner
                .y
                .saturating_add(framed.inner.height.saturating_sub(1)),
            width: framed.inner.width,
            height: 1,
        });

        render_header_row(
            frame,
            header_area,
            &HeaderRowSpec {
                left: sanitize_display_text(dialog.title_context.as_str()).into(),
                right: Some(
                    sanitize_display_text(format!(
                        "{} · {} · {}",
                        dialog.source_label.as_str(),
                        dialog.scope_label.as_str(),
                        if dialog.editable {
                            ui_text::t(&self.i18n, "value-editable")
                        } else {
                            ui_text::t(&self.i18n, "value-read-only")
                        }
                    ))
                    .into(),
                ),
                left_style: Style::default()
                    .fg(self.theme_color("accent", Color::Cyan))
                    .add_modifier(Modifier::BOLD),
                right_style: Style::default().fg(Color::DarkGray),
            },
        );

        let body_constraints = adaptive_detail_split(body_area.width, 26, 40);
        let body_sections = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(body_constraints)
            .split(body_area);
        let nav_area = body_sections.first().copied().unwrap_or(body_area);
        let table_area = body_sections.get(1).copied().unwrap_or(body_area);

        let nav_items = dialog
            .nav
            .items
            .iter()
            .map(|item| {
                build_accented_two_line_list_item(
                    sanitize_display_text(format!(
                        "{}{}",
                        "  ".repeat(item.level),
                        item.label.as_str()
                    ))
                    .into(),
                    None,
                    None,
                )
            })
            .collect::<Vec<_>>();
        let nav_spec = ListPanelSpec::new(
            Some(ui_text::t(&self.i18n, "overlay-permission-studio-nav").into()),
            nav_items.as_slice(),
            Some(dialog.nav.selected),
            if dialog.pane_focus == PermissionStudioPaneFocus::Navigation {
                selection_highlight_style()
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            },
            ">> ".into(),
        );
        render_list_panel(frame, nav_area, &nav_spec);

        let Some(selected_section) = dialog.state.selected_section() else {
            let empty_text = Text::from(ui_text::t(&self.i18n, "overlay-settings-empty-section"));
            render_text_panel(
                frame,
                table_area,
                &TextPanelSpec {
                    title: Some(ui_text::t(&self.i18n, "overlay-workbench-overview").into()),
                    body: &empty_text,
                    wrap: true,
                    scroll: None,
                    alignment: None,
                },
            );
            if footer_area.height > 0 {
                self.render_permission_studio_footer_row(frame, footer_area, dialog);
            }
            return;
        };
        let table_text =
            self.permission_studio_table_text(dialog, selected_section, table_area.width);
        render_text_panel(
            frame,
            table_area,
            &TextPanelSpec {
                title: Some(sanitize_display_text(selected_section.label.as_str()).into()),
                body: &table_text,
                wrap: false,
                scroll: None,
                alignment: None,
            },
        );

        if footer_area.height > 0 {
            self.render_permission_studio_footer_row(frame, footer_area, dialog);
        }

        if let Some(editor) = dialog.editor.as_ref() {
            render_editor_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                &EditorDialogSpec {
                    title: editor.title.clone().into(),
                    prompt: editor.prompt.clone().into(),
                    footer: editor.footer.clone().into(),
                    target_width: if editor.multiline { 96 } else { 78 },
                    multiline: editor.multiline,
                    prompt_height_bounds: (1, 3),
                    footer_height_bounds: (1, 2),
                },
                &editor.input,
            );
        }
    }

    fn permission_studio_table_text(
        &self,
        dialog: &PermissionStudioOverlay,
        section: &PermissionStudioSection,
        width: u16,
    ) -> Text<'static> {
        let (left_header, right_header) = Self::permission_studio_table_headers(section.id);
        let width = width.max(24);
        let left_width = width
            .saturating_mul(45)
            .saturating_div(100)
            .clamp(12, width - 12);
        let right_width = width.saturating_sub(left_width).saturating_sub(3).max(8);
        let mut lines = Vec::new();
        let header_style = Style::default()
            .fg(self.theme_color("accent", Color::Cyan))
            .add_modifier(Modifier::BOLD);
        let row_style = Style::default();
        let selected_style = selection_highlight_style();
        let selected_index = dialog.state.selected_item_index();
        let selected_focus = dialog.state.focus() == PermissionStudioFocus::Items;

        lines.push(Line::from(Span::styled(
            settings_compact_fixed_columns(
                &[
                    (left_header.as_str(), left_width as usize),
                    (right_header.as_str(), right_width as usize),
                ],
                width,
            ),
            header_style,
        )));
        lines.push(Line::from(Span::styled(
            "─".repeat(width as usize),
            Style::default().fg(Color::DarkGray),
        )));

        if section.items.is_empty() {
            lines.push(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-settings-empty-items"),
                Style::default().fg(Color::DarkGray),
            )));
            return Text::from(lines);
        }

        for (index, item) in section.items.iter().enumerate() {
            let is_selected = index == selected_index;
            let style = if is_selected && selected_focus {
                selected_style
            } else if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                row_style
            };
            let marker = if is_selected { ">> " } else { "   " };
            let left = format!("{marker}{}", item.label);
            lines.push(Line::from(Span::styled(
                settings_compact_fixed_columns(
                    &[
                        (left.as_str(), left_width as usize),
                        (item.value.as_str(), right_width as usize),
                    ],
                    width,
                ),
                style,
            )));
        }

        Text::from(lines)
    }

    fn render_permission_studio_footer_row(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionStudioOverlay,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let Some(section) = dialog.state.selected_section() else {
            return;
        };
        let buttons = self.permission_studio_footer_buttons(dialog, section.id);
        if buttons.is_empty() {
            return;
        }

        let spans =
            buttons
                .into_iter()
                .enumerate()
                .fold(Vec::new(), |mut acc, (index, (label, style))| {
                    if index > 0 {
                        acc.push(Span::styled("  ", Style::default().fg(Color::DarkGray)));
                    }
                    acc.push(Span::styled(format!("[ {label} ]"), style));
                    acc
                });
        frame.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
            area,
        );
    }

    fn permission_studio_table_headers(section_id: PermissionStudioSectionId) -> (String, String) {
        match section_id {
            PermissionStudioSectionId::PathDefaults => ("Setting".to_string(), "Value".to_string()),
            PermissionStudioSectionId::PathRules => ("Path".to_string(), "Access".to_string()),
            PermissionStudioSectionId::NetworkZones => ("Zone".to_string(), "Connect".to_string()),
            PermissionStudioSectionId::NetworkRules => {
                ("Domain".to_string(), "Connect".to_string())
            }
            PermissionStudioSectionId::ToolTags => ("Tag".to_string(), "Access".to_string()),
            PermissionStudioSectionId::ToolNames => ("Name".to_string(), "Access".to_string()),
            PermissionStudioSectionId::ToolCommandRules => {
                ("Tool".to_string(), "Access".to_string())
            }
            PermissionStudioSectionId::RootPath
            | PermissionStudioSectionId::RootNetwork
            | PermissionStudioSectionId::RootTools => ("Item".to_string(), "Summary".to_string()),
        }
    }

    fn permission_studio_footer_buttons(
        &self,
        dialog: &PermissionStudioOverlay,
        section_id: PermissionStudioSectionId,
    ) -> Vec<(String, Style)> {
        if !dialog.editable {
            return Vec::new();
        }
        let accent = Style::default()
            .fg(self.theme_color("accent", Color::Cyan))
            .add_modifier(Modifier::BOLD);
        let danger = Style::default()
            .fg(self.theme_color("danger", Color::Red))
            .add_modifier(Modifier::BOLD);
        match section_id {
            PermissionStudioSectionId::PathDefaults
            | PermissionStudioSectionId::NetworkZones
            | PermissionStudioSectionId::RootPath
            | PermissionStudioSectionId::RootNetwork
            | PermissionStudioSectionId::RootTools => Vec::new(),
            PermissionStudioSectionId::PathRules
            | PermissionStudioSectionId::NetworkRules
            | PermissionStudioSectionId::ToolTags
            | PermissionStudioSectionId::ToolNames
            | PermissionStudioSectionId::ToolCommandRules => vec![
                (ui_text::t(&self.i18n, "value-add"), accent),
                (ui_text::t(&self.i18n, "value-edit"), accent),
                (ui_text::t(&self.i18n, "value-duplicate"), accent),
                (ui_text::t(&self.i18n, "value-delete"), danger),
            ],
        }
    }

    fn render_permission_rule_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionRuleStudioOverlay,
        surface: SurfaceMode,
    ) {
        let overview_body = build_detail_text(
            vec![
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-draft"),
                    sanitize_display_text(permission_rule_draft_label(&self.i18n, &dialog.draft)),
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-mode"),
                    sanitize_display_text(permission_rule_mode_label(dialog.draft.mode)),
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-scope"),
                    sanitize_display_text(dialog.draft.scope.as_str()),
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-session"),
                    sanitize_display_text(if dialog.draft.session_id.trim().is_empty() {
                        ui_text::t(&self.i18n, "value-unset")
                    } else {
                        dialog.draft.session_id.clone()
                    }),
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(
                        &self.i18n,
                        "overlay-permission-rule-overview-workspace-root",
                    ),
                    sanitize_display_text(if dialog.draft.workspace_root.trim().is_empty() {
                        ui_text::t(&self.i18n, "value-runtime-default")
                    } else {
                        dialog.draft.workspace_root.clone()
                    }),
                    Style::default().fg(Color::DarkGray),
                    Style::default(),
                ),
            ],
            &DetailTextSpec::with_label_width(14),
        );
        let detail_text = dialog
            .workbench
            .list
            .selected_item()
            .map(|item| permission_rule_studio_detail_text(&self.i18n, &dialog.draft, item))
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-permission-rule-empty-detail"));

        let list_items = dialog
            .workbench
            .list
            .items
            .iter()
            .map(|item| {
                build_accented_two_line_list_item(
                    sanitize_display_text(item.label.as_str()).into(),
                    Some(sanitize_display_text(item.value.as_str()).into()),
                    Some(sanitize_display_text(item.detail.as_str()).into()),
                )
            })
            .collect::<Vec<_>>();
        let spec = ListWorkbenchDialogSpec::new(
            sanitize_display_text(dialog.workbench.title.as_str()).into(),
            sanitize_display_text(dialog.workbench.footer.as_str()).into(),
            132,
            40,
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 2,
                    min_body_height: 8,
                    max_body_height: 18,
                },
                Some(ui_text::t(&self.i18n, "overlay-permission-rule-fields").into()),
                list_items.as_slice(),
                (!dialog.workbench.list.items.is_empty()).then_some(dialog.workbench.list.selected),
                selection_highlight_style(),
                ">> ".into(),
            ),
            vec![
                WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-workbench-overview").into(),
                    overview_body,
                    3,
                    8,
                ),
                WorkbenchTextSection::new(
                    ui_text::t(&self.i18n, "overlay-workbench-details").into(),
                    Text::from(sanitize_display_text(detail_text)),
                    4,
                    14,
                ),
            ],
        )
        .with_summary(
            sanitize_display_text(permission_rule_subject_kind_name(dialog.draft.subject_kind))
                .into(),
        );
        let spec = spec.with_optional_overlay_source(dialog.workbench.editor.as_ref());
        render_list_workbench_dialog(frame, area, surface, &spec);
    }

    fn render_provider_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &ProviderStudioOverlay,
        surface: SurfaceMode,
    ) {
        let draft_fields = provider_studio_visible_fields(dialog);
        let detail_text_spec = provider_studio_detail_text_spec();
        let draft_text = build_detail_text(
            draft_fields.iter().enumerate().map(|(index, field)| {
                let display = provider_studio_main_field_display(&self.i18n, dialog, *field);
                let selected = dialog.detail_page.is_none()
                    && dialog.selection.focus() == ProviderStudioFocus::Fields
                    && dialog.selection.top_selected() == index;
                let label_style = if selected {
                    selection_highlight_style()
                } else {
                    Style::default()
                };
                DetailTextLine::labeled(
                    provider_studio_field_label(&self.i18n, *field),
                    sanitize_display_text(display),
                    label_style,
                    label_style,
                )
            }),
            &detail_text_spec,
        );
        let provider_items = dialog
            .providers
            .items
            .iter()
            .map(|row| {
                build_detail_two_line_list_item(
                    sanitize_display_text(row.label.as_str()).into(),
                    Some(sanitize_display_text(row.detail.as_str()).into()),
                    Style::default().fg(Color::DarkGray),
                )
            })
            .collect::<Vec<_>>();
        let adapter_items = dialog
            .adapter_candidate_ids
            .iter()
            .map(|adapter_id| {
                let adapter_models = dialog
                    .adapter_models
                    .iter()
                    .find(|adapter_models| adapter_models.adapter_id == *adapter_id);
                let enabled = if !provider_studio_adapter_selectable(dialog, adapter_id.as_str()) {
                    "[-]"
                } else if dialog.selected_adapter_ids.contains(adapter_id.as_str()) {
                    "[x]"
                } else {
                    "[ ]"
                };
                let detail = truncate_display_text(
                    provider_studio_adapter_list_detail(&self.i18n, dialog, adapter_id.as_str())
                        .as_str(),
                    48,
                );
                let detail_style = match adapter_models {
                    Some(adapter) if adapter.error.is_none() => {
                        Style::default().fg(Color::DarkGray)
                    }
                    Some(_) => Style::default().fg(Color::Red),
                    None => Style::default().fg(Color::DarkGray),
                };
                build_detail_two_line_list_item(
                    sanitize_display_text(format!("{enabled} {}", adapter_id)).into(),
                    Some(sanitize_display_text(detail).into()),
                    detail_style,
                )
            })
            .collect::<Vec<_>>();

        let model_items = provider_studio_selected_adapter_models(dialog)
            .map(|adapter_models| {
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
                        build_detail_two_line_list_item(
                            sanitize_display_text(format!("{selected} {}", model.id.as_str()))
                                .into(),
                            Some(
                                sanitize_display_text(provider_studio_model_list_detail(
                                    &self.i18n,
                                    dialog,
                                    adapter_models.adapter_id.as_str(),
                                    model.id.as_str(),
                                ))
                                .into(),
                            ),
                            Style::default().fg(Color::DarkGray),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let has_models = provider_studio_selected_adapter_models(dialog)
            .map(|adapter_models| !adapter_models.models.is_empty())
            .unwrap_or(false);
        let dashboard_spec = DashboardWorkbenchSpec::new(
            sanitize_display_text(dialog.title.as_str()).into(),
            sanitize_display_text(dialog.footer.as_str()).into(),
            122,
            DashboardTextSection::new(
                None,
                draft_text,
                DashboardTextPanelHeight::AutoBody {
                    min_body_height: 4,
                    max_body_height: 16,
                },
            ),
            DashboardSplitPanelsSpec::new(
                24,
                28,
                if dialog.adapter_candidate_ids.is_empty() {
                    DashboardListPanelState::empty(
                        Some(ui_text::t(&self.i18n, "overlay-provider-studio-adapters").into()),
                        ui_text::t(&self.i18n, "overlay-provider-studio-adapter-models-empty")
                            .into(),
                        DashboardListPanelHeight::AutoBody {
                            lines_per_item: 1,
                            min_body_height: 4,
                            max_body_height: 10,
                        },
                    )
                } else {
                    DashboardListPanelState::items(
                        DashboardListPanelHeight::AutoBody {
                            lines_per_item: 2,
                            min_body_height: 4,
                            max_body_height: 10,
                        },
                        Some(ui_text::t(&self.i18n, "overlay-provider-studio-adapters").into()),
                        adapter_items.as_slice(),
                        Some(dialog.selection.left_selected()),
                        if dialog.selection.focus() == ProviderStudioFocus::Adapters {
                            selection_highlight_style()
                        } else {
                            Style::default().add_modifier(Modifier::BOLD)
                        },
                        ">> ".into(),
                    )
                },
                if has_models {
                    DashboardListPanelState::items(
                        DashboardListPanelHeight::AutoBody {
                            lines_per_item: 2,
                            min_body_height: 4,
                            max_body_height: 10,
                        },
                        Some(ui_text::t(&self.i18n, "overlay-provider-studio-models").into()),
                        model_items.as_slice(),
                        Some(dialog.selection.right_selected()),
                        if dialog.selection.focus() == ProviderStudioFocus::Models {
                            selection_highlight_style()
                        } else {
                            Style::default().add_modifier(Modifier::BOLD)
                        },
                        ">> ".into(),
                    )
                } else {
                    DashboardListPanelState::empty(
                        Some(ui_text::t(&self.i18n, "overlay-provider-studio-models").into()),
                        ui_text::t(&self.i18n, "overlay-provider-studio-models-empty").into(),
                        DashboardListPanelHeight::AutoBody {
                            lines_per_item: 1,
                            min_body_height: 4,
                            max_body_height: 10,
                        },
                    )
                },
            ),
        );
        let dashboard_spec = if dialog.show_provider_list {
            dashboard_spec.with_lead_panel(DashboardLeadPanelSpec::new(
                28,
                54,
                DashboardListPanelState::items(
                    DashboardListPanelHeight::AutoBody {
                        lines_per_item: 2,
                        min_body_height: 4,
                        max_body_height: 12,
                    },
                    Some(ui_text::t(&self.i18n, "overlay-provider-studio-providers").into()),
                    provider_items.as_slice(),
                    (!dialog.providers.items.is_empty()).then_some(dialog.providers.selected),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                    ">> ".into(),
                ),
            ))
        } else {
            dashboard_spec
        };

        let detail_overlay = if let Some(model_page) = dialog.model_page.as_ref() {
            let lines = provider_model_config_fields()
                .iter()
                .enumerate()
                .map(|(field_index, field)| {
                    let field = *field;
                    let selected =
                        dialog.editor.is_none() && model_page.selection.selected == field_index;
                    let editable = provider_model_config_field_editable(field);
                    let label_style = if selected {
                        selection_highlight_style()
                    } else if editable {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    let value_style = if selected {
                        selection_highlight_style()
                    } else if editable {
                        Style::default()
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };
                    DetailTextLine::labeled(
                        provider_model_config_field_label(&self.i18n, field),
                        sanitize_display_text(provider_model_config_field_display(
                            &self.i18n,
                            &model_page.draft,
                            field,
                        )),
                        label_style,
                        value_style,
                    )
                })
                .collect::<Vec<_>>();
            Some(DashboardDetailOverlaySpec::new(
                DetailTextDialogSpec::new(
                    sanitize_display_text(model_page.title.as_str()).into(),
                    96,
                    DetailTextSpec::with_label_width(18),
                    (6, 20),
                )
                .with_footer(
                    sanitize_display_text(model_page.footer.as_str()).into(),
                    (1, 2),
                    None,
                    Style::default(),
                ),
                lines,
            ))
        } else if let Some(detail_page) = dialog.detail_page.as_ref() {
            let detail_fields = provider_studio_detail_fields(dialog);
            let auth_state_lines = provider_studio_auth_state_lines(&self.i18n, dialog);
            let mut lines = Vec::with_capacity(1 + auth_state_lines.len() + detail_fields.len());
            lines.push(DetailTextLine::labeled(
                provider_studio_field_label(&self.i18n, ProviderStudioField::AuthStatus),
                sanitize_display_text(provider_studio_auth_status_summary(&self.i18n, dialog)),
                Style::default().fg(Color::DarkGray),
                Style::default().fg(Color::DarkGray),
            ));
            lines.extend(auth_state_lines.into_iter().map(|line| {
                DetailTextLine::plain(
                    sanitize_display_text(line),
                    Style::default().fg(Color::DarkGray),
                )
            }));
            lines.extend(
                detail_fields
                    .iter()
                    .enumerate()
                    .map(|(field_index, field)| {
                        let field = *field;
                        let selected = dialog.editor.is_none()
                            && detail_page.selection.selected == field_index;
                        let label_style = if selected {
                            selection_highlight_style()
                        } else if provider_studio_field_editable(dialog, field) {
                            Style::default().add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        let value_style = if selected {
                            selection_highlight_style()
                        } else if provider_studio_field_editable(dialog, field) {
                            Style::default()
                        } else {
                            Style::default().fg(Color::DarkGray)
                        };
                        DetailTextLine::labeled(
                            provider_studio_field_label(&self.i18n, field),
                            sanitize_display_text(provider_studio_main_field_display(
                                &self.i18n, dialog, field,
                            )),
                            label_style,
                            value_style,
                        )
                    }),
            );
            Some(DashboardDetailOverlaySpec::new(
                DetailTextDialogSpec::new(
                    sanitize_display_text(detail_page.title.as_str()).into(),
                    92,
                    detail_text_spec,
                    (4, 20),
                )
                .with_footer(
                    sanitize_display_text(detail_page.footer.as_str()).into(),
                    (1, 2),
                    None,
                    Style::default(),
                ),
                lines,
            ))
        } else {
            None
        };

        render_dashboard_workbench_dialog(
            frame,
            area,
            surface,
            &dashboard_spec,
            Some(
                DashboardWorkbenchOverlaySpec::new(detail_overlay, None)
                    .with_optional_editor_source(dialog.editor.as_ref()),
            ),
        );
    }

    fn render_model_catalog_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &ModelCatalogStudioOverlay,
        surface: SurfaceMode,
    ) {
        let query_label = if dialog.query.trim().is_empty() {
            ui_text::t(&self.i18n, "overlay-model-catalog-query-all")
        } else {
            dialog.query.clone()
        };
        let summary = self.i18n.text_args(
            "overlay-model-catalog-summary",
            &crate::fl_args!(
                "query" => query_label,
                "start" => dialog.offset.saturating_add(1) as i64,
                "end" => dialog.offset.saturating_add(dialog.workbench.list.items.len()) as i64,
                "total" => dialog.total as i64,
                "count" => dialog.summary.model_count as i64,
            ),
        );

        let list_items = dialog
            .workbench
            .list
            .items
            .iter()
            .map(|entry| {
                build_detail_two_line_list_item(
                    sanitize_display_text(entry.model_id.as_str()).into(),
                    Some(
                        sanitize_display_text(entry.display_name.clone().unwrap_or_else(|| {
                            entry
                                .origin
                                .clone()
                                .unwrap_or_else(|| ui_text::t(&self.i18n, "value-unknown"))
                        }))
                        .into(),
                    ),
                    Style::default().fg(Color::DarkGray),
                )
            })
            .collect::<Vec<_>>();
        let detail_spec = DetailTextSpec::with_label_width(9);
        let detail_body = dialog
            .workbench
            .list
            .selected_item()
            .map(|entry| {
                let mut lines = vec![
                    DetailTextLine::labeled(
                        ui_text::t(&self.i18n, "overlay-model-catalog-field-model-id"),
                        sanitize_display_text(entry.model_id.as_str()),
                        Style::default().fg(Color::DarkGray),
                        Style::default(),
                    ),
                    DetailTextLine::labeled(
                        ui_text::t(&self.i18n, "overlay-model-catalog-field-display"),
                        sanitize_display_text(
                            entry
                                .display_name
                                .clone()
                                .unwrap_or_else(|| ui_text::t(&self.i18n, "value-unset")),
                        ),
                        Style::default().fg(Color::DarkGray),
                        Style::default(),
                    ),
                    DetailTextLine::labeled(
                        ui_text::t(&self.i18n, "overlay-model-catalog-field-origin"),
                        sanitize_display_text(
                            entry
                                .origin
                                .clone()
                                .unwrap_or_else(|| ui_text::t(&self.i18n, "value-unset")),
                        ),
                        Style::default().fg(Color::DarkGray),
                        Style::default(),
                    ),
                    DetailTextLine::labeled(
                        ui_text::t(&self.i18n, "overlay-model-catalog-field-limits"),
                        sanitize_display_text(self.i18n.text_args(
                            "overlay-model-catalog-limits",
                            &crate::fl_args!(
                                "context" => entry
                                    .context_window_tokens
                                    .map(|value| value.to_string())
                                    .unwrap_or_else(|| "?".to_owned()),
                                "output" => entry
                                    .max_output_tokens
                                    .map(|value| value.to_string())
                                    .unwrap_or_else(|| "?".to_owned())
                            ),
                        )),
                        Style::default().fg(Color::DarkGray),
                        Style::default(),
                    ),
                    DetailTextLine::labeled(
                        ui_text::t(&self.i18n, "overlay-model-catalog-field-source"),
                        sanitize_display_text(format!("{:?}", entry.source)),
                        Style::default().fg(Color::DarkGray),
                        Style::default(),
                    ),
                ];
                if let Some(description) = entry.description.as_deref()
                    && !description.trim().is_empty()
                {
                    lines.push(DetailTextLine::plain(String::new(), Style::default()));
                    lines.push(DetailTextLine::plain(
                        sanitize_display_text(description),
                        Style::default(),
                    ));
                }
                build_detail_text(lines, &detail_spec)
            })
            .unwrap_or_else(|| {
                Text::from(sanitize_display_text(
                    dialog.summary.last_error.clone().unwrap_or_else(|| {
                        ui_text::t(&self.i18n, "overlay-provider-studio-catalog-empty")
                    }),
                ))
            });
        let spec = ListWorkbenchDialogSpec::new(
            sanitize_display_text(dialog.workbench.title.as_str()).into(),
            sanitize_display_text(dialog.workbench.footer.as_str()).into(),
            136,
            48,
            if dialog.loading {
                ListWorkbenchPanelState::loading(
                    Some(ui_text::t(&self.i18n, "overlay-model-catalog-entries").into()),
                    ui_text::t(&self.i18n, "overlay-picker-loading").into(),
                    BoundedListPanelHeight {
                        lines_per_item: 2,
                        min_body_height: 5,
                        max_body_height: 14,
                    },
                )
            } else if dialog.workbench.list.items.is_empty() {
                ListWorkbenchPanelState::empty(
                    Some(ui_text::t(&self.i18n, "overlay-model-catalog-entries").into()),
                    ui_text::t(&self.i18n, "overlay-provider-studio-catalog-empty").into(),
                    BoundedListPanelHeight {
                        lines_per_item: 2,
                        min_body_height: 5,
                        max_body_height: 14,
                    },
                )
            } else {
                ListWorkbenchPanelState::items(
                    BoundedListPanelHeight {
                        lines_per_item: 2,
                        min_body_height: 5,
                        max_body_height: 14,
                    },
                    Some(ui_text::t(&self.i18n, "overlay-model-catalog-entries").into()),
                    list_items.as_slice(),
                    Some(dialog.workbench.list.selected),
                    selection_highlight_style(),
                    ">> ".into(),
                )
            },
            vec![WorkbenchTextSection::new(
                ui_text::t(&self.i18n, "overlay-model-catalog-detail").into(),
                detail_body,
                4,
                12,
            )],
        )
        .with_summary(sanitize_display_text(summary.as_str()).into())
        .with_min_widths(48, 34);
        let spec = spec.with_optional_overlay_source(dialog.workbench.editor.as_ref());
        render_list_workbench_dialog(frame, area, surface, &spec);
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

fn user_input_nav_line(
    i18n: &I18n,
    dialog: &UserInputOverlay,
    answered_color: Color,
) -> Line<'static> {
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
        let selected = dialog.state.selected_question() == index
            && dialog.state.screen() == QuestionFlowScreen::Question;
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
            format!(" [>] {} ", ui_text::t(i18n, "overlay-user-input-submit")),
            if dialog.state.screen() == QuestionFlowScreen::Review {
                selection_highlight_style()
            } else {
                Style::default()
            },
        ));
    }
    Line::from(spans)
}

fn user_input_answer_summary(
    i18n: &I18n,
    question: &UserInputQuestion,
    draft: &UserInputAnswerDraft,
) -> String {
    let values = user_input_answer_values(question, draft);
    if values.is_empty() {
        ui_text::t(i18n, "overlay-user-input-unanswered")
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

fn sanitize_display_text(text: impl AsRef<str>) -> String {
    sanitize_terminal_text(text.as_ref())
}

fn sanitize_display_str(text: &str) -> String {
    sanitize_display_text(text)
}

fn settings_compact_sections_text(
    i18n: &I18n,
    dialog: &SettingsStudioOverlay,
    width: u16,
) -> Text<'static> {
    let mut lines = vec![Line::from(Span::styled(
        sanitize_display_text(ui_text::t(i18n, "overlay-settings-sections")),
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(""));
    if dialog.state.sections().is_empty() {
        lines.push(Line::from(sanitize_display_text(ui_text::t(
            i18n,
            "overlay-settings-empty-section",
        ))));
        return Text::from(lines);
    }
    let content_width = width.max(1) as usize;
    let mut previous_group = String::new();
    for (index, section) in dialog.state.sections().iter().enumerate() {
        let group = settings_section_group_label(i18n, section.id);
        if group != previous_group {
            if !previous_group.is_empty() {
                lines.push(Line::from(""));
            }
            previous_group = group.clone();
            lines.push(Line::from(Span::styled(
                sanitize_display_text(settings_compact_pad_to_width(group.as_str(), content_width)),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let selected = index == dialog.state.selected_section_index();
        let marker = if selected { "> " } else { "  " };
        let label = format!("{marker}{}  {}", section.label, section.items.len());
        let line = settings_compact_pad_to_width(label.as_str(), content_width);
        let style = if selected && dialog.state.focus() == SettingsStudioFocus::Navigation {
            selection_highlight_style()
        } else if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(sanitize_display_text(line), style)));
    }
    Text::from(lines)
}

fn settings_compact_editor_text(
    i18n: &I18n,
    dialog: &SettingsStudioOverlay,
    current_section: Option<&SettingsStudioSection>,
    width: u16,
    height: u16,
) -> Text<'static> {
    let mut lines = Vec::new();
    let Some(section) = current_section else {
        lines.push(Line::from(sanitize_display_text(ui_text::t(
            i18n,
            "overlay-settings-empty-section",
        ))));
        return Text::from(lines);
    };

    lines.push(Line::from(Span::styled(
        sanitize_display_text(section.label.as_str()),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        sanitize_display_text(section.description.as_str()),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(""));

    let detail = settings_compact_item_detail_text(i18n, dialog, section);
    let fixed_rows = 4usize
        .saturating_add(2)
        .saturating_add(detail.lines.len().max(1));
    let visible_item_count = height.saturating_sub(fixed_rows as u16).max(1) as usize;

    if section.items.is_empty() {
        lines.push(Line::from(sanitize_display_text(ui_text::t(
            i18n,
            "overlay-settings-empty-items",
        ))));
    } else {
        let option_header = ui_text::t(i18n, "overlay-settings-compact-column-option");
        let value_header = ui_text::t(i18n, "overlay-settings-compact-column-value");
        let detail_header = ui_text::t(i18n, "overlay-settings-compact-column-detail");
        lines.push(Line::from(Span::styled(
            settings_compact_fixed_columns(
                &[
                    (option_header.as_str(), 34),
                    (value_header.as_str(), 26),
                    (detail_header.as_str(), 58),
                ],
                width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        let (start, end) = settings_compact_visible_range(
            section.items.len(),
            dialog.state.selected_item_index(),
            visible_item_count,
        );
        for (index, item) in section.items[start..end].iter().enumerate() {
            let index = start + index;
            let selected = index == dialog.state.selected_item_index();
            let marker = if selected { ">> " } else { "   " };
            let label = format!("{marker}{}", item.label);
            let line = settings_compact_fixed_columns(
                &[
                    (label.as_str(), 34),
                    (item.value.as_str(), 26),
                    (item.detail.as_str(), 58),
                ],
                width,
            );
            let style = if selected && dialog.state.focus() == SettingsStudioFocus::Items {
                selection_highlight_style()
            } else if selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(line, style)));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        sanitize_display_text(ui_text::t(i18n, "overlay-workbench-details")),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.extend(detail.lines);
    Text::from(lines)
}

fn settings_compact_item_detail_text(
    i18n: &I18n,
    dialog: &SettingsStudioOverlay,
    _section: &SettingsStudioSection,
) -> Text<'static> {
    dialog
        .state
        .selected_item()
        .map(|item| {
            let mut lines = vec![Line::from(sanitize_display_text(item.detail.as_str()))];
            if let SettingsPickerAction::EditField(field) = &item.action {
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-detail-path",
                    &crate::fl_args!("path" => field.path),
                ))));
            }
            if !item.value.trim().is_empty() {
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-detail-current",
                    &crate::fl_args!("value" => item.value.clone()),
                ))));
            }
            lines.push(Line::from(sanitize_display_text(ui_text::t(
                i18n,
                "overlay-settings-detail-action",
            ))));
            Text::from(lines)
        })
        .unwrap_or_else(|| Text::from(ui_text::t(i18n, "overlay-settings-empty-detail")))
}

fn settings_section_group_label(i18n: &I18n, section: SettingsStudioSectionId) -> String {
    let key = match section {
        SettingsStudioSectionId::ConfigProviders
        | SettingsStudioSectionId::ConfigAgents
        | SettingsStudioSectionId::ConfigPermission
        | SettingsStudioSectionId::ConfigPlugins => "overlay-settings-group-core",
        SettingsStudioSectionId::ConfigRuntime
        | SettingsStudioSectionId::ConfigSession
        | SettingsStudioSectionId::ConfigHarnesses
        | SettingsStudioSectionId::ConfigTracing
        | SettingsStudioSectionId::ConfigUi => "overlay-settings-group-application",
        SettingsStudioSectionId::RuntimeOverrides | SettingsStudioSectionId::RuntimeRules => {
            "overlay-settings-group-session"
        }
        SettingsStudioSectionId::Catalogs | SettingsStudioSectionId::Files => {
            "overlay-settings-group-system"
        }
    };
    ui_text::t(i18n, key)
}

fn settings_compact_visible_range(
    item_count: usize,
    selected_index: usize,
    max_visible: usize,
) -> (usize, usize) {
    if item_count == 0 {
        return (0, 0);
    }
    let max_visible = max_visible.max(1).min(item_count);
    let selected_index = selected_index.min(item_count.saturating_sub(1));
    let start = selected_index
        .saturating_sub(max_visible / 2)
        .min(item_count.saturating_sub(max_visible));
    (start, start + max_visible)
}

fn settings_compact_vertical_divider(height: u16) -> Text<'static> {
    Text::from((0..height).map(|_| Line::from("│")).collect::<Vec<_>>())
}

fn settings_compact_fixed_columns(columns: &[(&str, usize)], width: u16) -> String {
    let mut out = String::new();
    for (index, (text, size)) in columns.iter().enumerate() {
        if index > 0 {
            out.push_str("  ");
        }
        let remaining = width.saturating_sub(out.width() as u16) as usize;
        if remaining == 0 {
            break;
        }
        let size = (*size).min(remaining);
        let cleaned = sanitize_display_text(text);
        let clipped = truncate_display_text(cleaned.as_str(), size);
        out.push_str(clipped.as_str());
        let padding = size.saturating_sub(clipped.width());
        out.push_str(" ".repeat(padding).as_str());
    }
    out
}

fn settings_compact_pad_to_width(text: &str, width: usize) -> String {
    let cleaned = sanitize_display_text(text);
    let clipped = truncate_display_text(cleaned.as_str(), width);
    let padding = width.saturating_sub(clipped.width());
    format!("{clipped}{}", " ".repeat(padding))
}

fn user_input_review_answer_preview(i18n: &I18n, values: &[String]) -> String {
    if values.is_empty() {
        ui_text::t(i18n, "overlay-user-input-unanswered")
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

fn user_input_custom_values_preview(i18n: &I18n, values: &[String], width: u16) -> String {
    if values.is_empty() {
        ui_text::t(i18n, "overlay-user-input-custom-empty")
    } else {
        truncate_display_text(values.join(", ").as_str(), width.max(1) as usize)
    }
}

fn provider_studio_main_field_display(
    i18n: &I18n,
    dialog: &ProviderStudioOverlay,
    field: ProviderStudioField,
) -> String {
    let value = match field {
        ProviderStudioField::AuthMode => {
            provider_draft_auth_mode_label(i18n, &dialog.draft.auth_kind)
        }
        ProviderStudioField::CredentialIssuer => dialog
            .draft
            .auth_kind
            .credential_issuer()
            .map(|issuer| provider_credential_issuer_label_localized(i18n, issuer))
            .unwrap_or_else(|| provider_studio_main_field_value(i18n, dialog, field)),
        _ => provider_studio_main_field_value(i18n, dialog, field),
    };
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
        _ if value.trim().is_empty() => ui_text::t(i18n, "value-unset"),
        _ => value,
    }
}

fn provider_studio_detail_text_spec() -> DetailTextSpec<'static> {
    DetailTextSpec::with_label_width(16)
}

fn permission_overlay_body_lines(i18n: &I18n, dialog: &PermissionOverlay) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        sanitize_display_text(i18n.text_args(
            "overlay-permission-request-id",
            &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
        )),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(permission_action_label(
        i18n,
        &dialog.request.action,
    )));
    lines.push(Line::from(sanitize_display_text(i18n.text_args(
        "overlay-permission-reason",
        &crate::fl_args!("reason" => sanitize_display_text(dialog.request.reason.as_str())),
    ))));
    if !dialog.request.explanation.trim().is_empty() {
        lines.push(Line::from(sanitize_display_text(i18n.text_args(
            "overlay-permission-explanation",
            &crate::fl_args!(
                "explanation" => sanitize_display_text(dialog.request.explanation.as_str())
            ),
        ))));
    }
    let mut facts = Vec::new();
    facts.push(i18n.text_args(
        "overlay-permission-fact-risk",
        &crate::fl_args!("value" => permission_risk_label(i18n, dialog.request.risk)),
    ));
    if let Some(source) = dialog.request.source.as_deref() {
        facts.push(i18n.text_args(
            "overlay-permission-fact-source",
            &crate::fl_args!("value" => sanitize_display_text(source)),
        ));
    }
    if let Some(scope) = dialog.request.scope {
        facts.push(i18n.text_args(
            "overlay-permission-fact-scope",
            &crate::fl_args!("value" => permission_request_scope_label(i18n, scope)),
        ));
    }
    if let Some(operator) = dialog.request.operator.as_deref() {
        facts.push(i18n.text_args(
            "overlay-permission-fact-operator",
            &crate::fl_args!("value" => sanitize_display_text(operator)),
        ));
    }
    if !facts.is_empty() {
        lines.push(Line::from(join_inline_segments(facts)));
    }
    if let Some(session_id) = dialog.request.session_id {
        lines.push(Line::from(sanitize_display_text(i18n.text_args(
            "overlay-permission-session",
            &crate::fl_args!("session" => session_id),
        ))));
    }
    append_permission_trace_lines(i18n, &mut lines, &dialog.request.trace);
    lines
}

fn permission_request_scope_label(i18n: &I18n, scope: PermissionScope) -> String {
    match scope {
        PermissionScope::Session => ui_text::t(i18n, "value-session"),
        PermissionScope::Workspace => ui_text::t(i18n, "value-workspace"),
        PermissionScope::Global => ui_text::t(i18n, "value-global"),
    }
}
