use super::*;
use agena_tui_components::{
    BoundedListPanelHeight, ComposerEditorSurfaceSpec, DashboardDetailOverlaySpec,
    DashboardLeadPanelSpec, DashboardListPanelHeight, DashboardListPanelState,
    DashboardSplitPanelsSpec, DashboardTextPanelHeight, DashboardTextSection,
    DashboardWorkbenchOverlaySpec, DashboardWorkbenchSpec, DecisionDialogSpec,
    DetailTextDialogSpec, DetailTextLine, DetailTextSpec, EditorDialogSpec,
    EditorPreviewDialogSpec, EditorPreviewHelpSpec, FramedSurfaceSpec,
    HeaderBodyFooterTextSurfaceSpec, HeaderRowSpec, LineTextDialogSpec, ListPanelSection,
    ListPanelSpec, ListWorkbenchDialogSpec, ListWorkbenchPanelState, ParagraphSection,
    QuerySuggestionPopupSpec, QuestionFlowCustomInputSpec, QuestionFlowDialogMode,
    QuestionFlowDialogSpec, SearchListDialogSpec, SearchPanelsDialogSpec, SearchPanelsDialogState,
    StackedDialogSection, StackedDialogSectionHeight, StackedDialogSpec, SuggestionPopupItem,
    SuggestionPopupSpec, SurfaceMode, TextDialogLine, TextPanelSection, TextPanelSpec,
    VerticalSectionSize, WorkbenchTextSection, WrappedTextSpec, adaptive_detail_split,
    adaptive_modal_width, build_accented_two_line_list_item, build_detail_two_line_list_item,
    build_wrapped_text_lines, format_key_value_segment, inset_rect, join_inline_segments,
    layout_composer_surface, layout_header_body_footer_surface, list_panel_height,
    pane_header_height, render_composer_editor_surface, render_dashboard_workbench_dialog,
    render_decision_dialog, render_editor_dialog, render_editor_preview_dialog,
    render_framed_surface, render_header_body_footer_text_surface, render_header_row,
    render_line_text_dialog, render_list_panel, render_list_workbench_dialog,
    render_overlay_line_input_dialog, render_query_suggestion_popup, render_question_flow_dialog,
    render_search_list_dialog, render_search_panels_dialog, render_stacked_dialog,
    render_suggestion_popup, render_text_panel, render_wrapped_text, split_vertical_sections,
    truncate_display_text, wrapped_text_height_for_text,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Borders, ListItem, Paragraph, Wrap},
};
use tui_markdown::from_str as markdown_to_text;
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
            Overlay::TranscriptSearch(dialog)
            | Overlay::SessionRename(dialog)
            | Overlay::AgentCreate(dialog) => {
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
            Route::PluginPolicyStudio(dialog) => {
                self.render_plugin_policy_studio(frame, area, dialog, SurfaceMode::Route);
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
        if user_input_request_is_review(&dialog.request) {
            self.render_user_input_review_overlay(frame, area, dialog);
            return;
        }
        let target_width = adaptive_modal_width(area.width, 92);
        let footer_review = user_input_footer_text(
            &self.i18n,
            &dialog.request,
            "overlay-user-input-footer-review",
        );
        let footer_question = user_input_footer_text(
            &self.i18n,
            &dialog.request,
            "overlay-user-input-footer-question",
        );
        let nav_color = self.theme_color("flash_info", Color::Cyan);
        let title = user_input_overlay_title(&self.i18n, &dialog.request);
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

            let mut review_lines =
                user_input_body_markdown_lines(dialog.request.body_markdown.as_str(), None);
            if !review_lines.is_empty() {
                review_lines.push(Line::default());
            }
            review_lines.push(Line::from(Span::styled(
                ui_text::t(&self.i18n, "overlay-user-input-review-intro"),
                Style::default().add_modifier(Modifier::BOLD),
            )));
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
            let detail_body = Text::from(if dialog.request.body_markdown.trim().is_empty() {
                vec![Line::from(sanitize_display_text(ui_text::t(
                    &self.i18n,
                    "overlay-user-input-no-questions",
                )))]
            } else {
                user_input_body_markdown_lines(dialog.request.body_markdown.as_str(), None)
            });
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
        let mut prompt_lines =
            user_input_body_markdown_lines(dialog.request.body_markdown.as_str(), None);
        if !prompt_lines.is_empty() {
            prompt_lines.push(Line::default());
        }
        prompt_lines.extend(vec![
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
        let prompt_body = Text::from(prompt_lines);

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

    fn render_user_input_review_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &UserInputOverlay,
    ) {
        let Some(question) = user_input_review_question(&dialog.request) else {
            return;
        };
        let target_width = area.width;
        let content_width = SurfaceMode::Route
            .content_width(area, target_width)
            .saturating_sub(2)
            .max(1);
        let title = user_input_overlay_title(&self.i18n, &dialog.request);
        let plan_body = review_request_body_markdown(dialog.request.body_markdown.as_str());
        let body_content_width = content_width.saturating_sub(2).max(1);
        let natural_body_height = wrapped_text_height_for_text(&plan_body, body_content_width);
        let decision_items = question
            .options
            .iter()
            .map(|option| {
                build_detail_two_line_list_item(
                    option.label.clone().into(),
                    (!option.description.trim().is_empty())
                        .then_some(option.description.clone().into()),
                    Style::default().fg(Color::DarkGray),
                )
            })
            .collect::<Vec<ListItem<'static>>>();
        let cancel_label = {
            let label = dialog.request.cancel_label.trim();
            if label.is_empty() { "cancel" } else { label }
        };
        let footer = Text::from(vec![Line::from(Span::styled(
            sanitize_display_text(format!(
                "Enter {} · Ctrl+D {} · ↑/↓ choose · PgUp/PgDn scroll · Home/End jump",
                user_input_submit_label(&self.i18n, &dialog.request),
                cancel_label
            )),
            Style::default().fg(Color::DarkGray),
        ))]);
        let decision_height = list_panel_height(question.options.len(), 2, 4, 10);
        let footer_height = wrapped_text_height_for_text(&footer, content_width).clamp(1, 2);
        let body_height = area
            .height
            .saturating_sub(2)
            .saturating_sub(decision_height)
            .saturating_sub(footer_height)
            .max(1);
        let max_scroll = natural_body_height.saturating_sub(body_height.saturating_sub(2));
        let scroll = dialog.review_scroll.min(max_scroll);

        render_stacked_dialog(
            frame,
            area,
            SurfaceMode::Route,
            &StackedDialogSpec {
                title: title.into(),
                target_width,
                sections: vec![
                    StackedDialogSection::ListPanel(ListPanelSection {
                        height: StackedDialogSectionHeight::AutoList {
                            lines_per_item: 2,
                            min_body: 4,
                            max_body: 10,
                        },
                        spec: ListPanelSpec::new(
                            Some("Decisions".into()),
                            decision_items.as_slice(),
                            Some(dialog.review_option),
                            selection_highlight_style(),
                            ">> ".into(),
                        ),
                    }),
                    StackedDialogSection::TextPanel(TextPanelSection {
                        height: StackedDialogSectionHeight::Fixed(body_height),
                        spec: TextPanelSpec {
                            title: Some("Plan".into()),
                            body: &plan_body,
                            wrap: true,
                            scroll: Some((scroll, 0)),
                            alignment: None,
                        },
                    }),
                    StackedDialogSection::Paragraph(ParagraphSection {
                        height: StackedDialogSectionHeight::AutoText { min: 1, max: 2 },
                        title: None,
                        borders: Borders::NONE,
                        body: footer,
                        wrap: true,
                        scroll: None,
                        alignment: None,
                    }),
                ],
            },
        );
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
        let frame_title = format!("{} / {}", dialog.title.as_str(), section_title.as_str());

        let framed = render_framed_surface(
            frame,
            area,
            surface,
            &FramedSurfaceSpec {
                title: sanitize_display_text(frame_title).into(),
                target_width: 150,
                target_height: 42,
            },
        );
        let page_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(12), Constraint::Length(1)])
            .split(framed.inner);
        let inner = page_rows[0];

        let nav_width = inner
            .width
            .saturating_mul(22)
            .saturating_div(100)
            .clamp(18, 28)
            .min(inner.width.saturating_sub(1));
        let wide_inspector = inner.width >= 100 && inner.height >= 12;
        let inspector_title = settings_compact_item_detail_title(&self.i18n, dialog);
        let inspector_text = settings_compact_item_detail_text(&self.i18n, dialog);

        if wide_inspector {
            let available_after_nav = inner.width.saturating_sub(nav_width).saturating_sub(2);
            let inspector_width = inner
                .width
                .saturating_mul(30)
                .saturating_div(100)
                .clamp(30, 44)
                .min(available_after_nav.saturating_sub(32))
                .max(28);
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(nav_width),
                    Constraint::Length(1),
                    Constraint::Min(32),
                    Constraint::Length(1),
                    Constraint::Length(inspector_width),
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
                Paragraph::new(settings_compact_vertical_divider(body[3].height))
                    .wrap(Wrap { trim: false }),
                body[3],
            );
            render_text_panel(
                frame,
                body[4],
                &TextPanelSpec {
                    title: Some(inspector_title.into()),
                    body: &inspector_text,
                    wrap: true,
                    scroll: None,
                    alignment: None,
                },
            );
        } else {
            let body = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(nav_width),
                    Constraint::Length(u16::from(inner.width > 0)),
                    Constraint::Min(24),
                ])
                .split(inner);
            let detail_height = body[2]
                .height
                .saturating_mul(35)
                .saturating_div(100)
                .clamp(7, 14)
                .min(body[2].height.saturating_sub(4));
            let editor_rows = if detail_height > 0 {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(4), Constraint::Length(detail_height)])
                    .split(body[2])
            } else {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(0)])
                    .split(body[2])
            };
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
                    editor_rows[0].width,
                    editor_rows[0].height,
                ))
                .wrap(Wrap { trim: false }),
                editor_rows[0],
            );
            if editor_rows[1].height > 0 {
                render_text_panel(
                    frame,
                    editor_rows[1],
                    &TextPanelSpec {
                        title: Some(inspector_title.into()),
                        body: &inspector_text,
                        wrap: true,
                        scroll: None,
                        alignment: None,
                    },
                );
            }
        }
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
            "{} · {} · {}",
            agent_profile_scope_label_localized(&self.i18n, &dialog.profile),
            agent_profile_storage_label_localized(&self.i18n, dialog.storage),
            if dialog.editable {
                ui_text::t(&self.i18n, "value-editable")
            } else {
                ui_text::t(&self.i18n, "value-read-only")
            }
        );
        let overview_body = agent_studio_overview_text(
            &self.i18n,
            &dialog.profile,
            dialog.default_agent_name.as_deref(),
            dialog.storage,
        );
        let selected_item = dialog.workbench.list.selected_item();
        let detail_body = selected_item
            .map(|item| {
                agent_studio_item_detail_text(&self.i18n, &dialog.profile, item, dialog.storage)
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
                        sanitize_display_text(model_catalog_list_subtitle(&self.i18n, entry))
                            .into(),
                    ),
                    Style::default().fg(Color::DarkGray),
                )
            })
            .collect::<Vec<_>>();
        let detail_body = dialog
            .workbench
            .list
            .selected_item()
            .map(|entry| model_catalog_detail_text(&self.i18n, entry))
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
                30,
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

fn model_catalog_list_subtitle(i18n: &I18n, entry: &CatalogModelResource) -> String {
    let mut parts = Vec::new();
    if let Some(display_name) = entry
        .display_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(display_name.to_owned());
    }
    if let Some(origin) = entry
        .origin
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(origin.to_owned());
    }
    if let Some(lifecycle) = entry.lifecycle {
        parts.push(model_catalog_lifecycle_label(i18n, lifecycle));
    }
    if model_catalog_has_token_limits(entry) {
        parts.push(model_catalog_limits_summary(i18n, entry));
    }
    let feature_summary = model_catalog_supported_feature_summary(&entry.capabilities);
    if !feature_summary.is_empty() {
        parts.push(feature_summary);
    }
    let pricing = model_catalog_pricing_summary(i18n, entry.pricing.as_ref());
    if pricing != ui_text::t(i18n, "value-unset") {
        parts.push(pricing);
    }
    join_inline_segments(parts)
}

fn model_catalog_detail_text(i18n: &I18n, entry: &CatalogModelResource) -> Text<'static> {
    let mut lines = vec![
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-model-id",
            entry.model_id.as_str(),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-display",
            model_catalog_optional_string(i18n, entry.display_name.as_deref()),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-origin",
            model_catalog_optional_string(i18n, entry.origin.as_deref()),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-lifecycle",
            entry
                .lifecycle
                .map(|lifecycle| model_catalog_lifecycle_label(i18n, lifecycle))
                .unwrap_or_else(|| ui_text::t(i18n, "value-unset")),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-dates",
            model_catalog_dates_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-limits",
            model_catalog_limits_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-inputs",
            model_catalog_input_capability_summary(i18n, &entry.capabilities),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-output",
            model_catalog_string_list_summary(i18n, &entry.output_modalities),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-features",
            model_catalog_feature_capability_summary(i18n, &entry.capabilities),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-modes",
            model_catalog_modes_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-defaults",
            model_catalog_defaults_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-runtime",
            model_catalog_runtime_summary(i18n, entry),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-pricing",
            model_catalog_pricing_summary(i18n, entry.pricing.as_ref()),
        ),
        model_catalog_detail_labeled_line(
            i18n,
            "overlay-model-catalog-field-source",
            model_catalog_source_summary(entry),
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
    build_detail_text(lines, &DetailTextSpec::with_label_width(14))
}

fn model_catalog_detail_labeled_line(
    i18n: &I18n,
    label_key: &str,
    value: impl Into<String>,
) -> DetailTextLine<'static> {
    let value = value.into();
    DetailTextLine::labeled(
        ui_text::t(i18n, label_key),
        sanitize_display_text(value.as_str()),
        Style::default().fg(Color::DarkGray),
        Style::default(),
    )
}

fn model_catalog_optional_string(i18n: &I18n, value: Option<&str>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
}

fn model_catalog_lifecycle_label(i18n: &I18n, value: agena::model::ModelLifecycle) -> String {
    let key = match value {
        agena::model::ModelLifecycle::Active => "overlay-model-catalog-lifecycle-active",
        agena::model::ModelLifecycle::Preview => "overlay-model-catalog-lifecycle-preview",
        agena::model::ModelLifecycle::Beta => "overlay-model-catalog-lifecycle-beta",
        agena::model::ModelLifecycle::Alpha => "overlay-model-catalog-lifecycle-alpha",
        agena::model::ModelLifecycle::Experimental => {
            "overlay-model-catalog-lifecycle-experimental"
        }
        agena::model::ModelLifecycle::Deprecated => "overlay-model-catalog-lifecycle-deprecated",
    };
    ui_text::t(i18n, key)
}

fn model_catalog_dates_summary(i18n: &I18n, entry: &CatalogModelResource) -> String {
    let mut parts = Vec::new();
    if let Some(value) = entry
        .release_date
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "overlay-model-catalog-date-release",
            &crate::fl_args!("value" => value),
        ));
    }
    if let Some(value) = entry
        .last_updated
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "overlay-model-catalog-date-updated",
            &crate::fl_args!("value" => value),
        ));
    }
    if let Some(value) = entry
        .knowledge_cutoff
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(i18n.text_args(
            "overlay-model-catalog-date-cutoff",
            &crate::fl_args!("value" => value),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

fn model_catalog_limits_summary(i18n: &I18n, entry: &CatalogModelResource) -> String {
    sanitize_display_text(i18n.text_args(
        "overlay-model-catalog-limits",
        &crate::fl_args!(
            "context" => model_catalog_token_count(i18n, entry.context_window_tokens),
            "input" => model_catalog_token_count(i18n, entry.max_input_tokens),
            "output" => model_catalog_token_count(i18n, entry.max_output_tokens),
        ),
    ))
}

fn model_catalog_has_token_limits(entry: &CatalogModelResource) -> bool {
    entry.context_window_tokens.is_some()
        || entry.max_input_tokens.is_some()
        || entry.max_output_tokens.is_some()
}

fn model_catalog_token_count(i18n: &I18n, value: Option<u32>) -> String {
    value
        .map(|value| format_tokens_k(value as u64))
        .unwrap_or_else(|| ui_text::t(i18n, "value-unset"))
}

fn model_catalog_input_capability_summary(
    i18n: &I18n,
    capabilities: &agena::provider::ModelCapabilityPatch,
) -> String {
    let mut supported = Vec::new();
    let mut unsupported = Vec::new();
    for modality in [
        agena::model::ModelInputModality::Text,
        agena::model::ModelInputModality::Image,
        agena::model::ModelInputModality::Document,
        agena::model::ModelInputModality::Audio,
        agena::model::ModelInputModality::Video,
        agena::model::ModelInputModality::File,
    ] {
        match capabilities.input_support(modality) {
            Some(agena::model::CapabilitySupport::Supported) => {
                supported.push(modality.as_str().to_owned())
            }
            Some(agena::model::CapabilitySupport::Unsupported) => {
                unsupported.push(modality.as_str().to_owned())
            }
            Some(agena::model::CapabilitySupport::Unknown) | None => {}
        }
    }
    model_catalog_support_summary(i18n, supported, unsupported)
}

fn model_catalog_feature_capability_summary(
    i18n: &I18n,
    capabilities: &agena::provider::ModelCapabilityPatch,
) -> String {
    let mut supported = Vec::new();
    let mut unsupported = Vec::new();
    for feature in [
        agena::provider::ModelCapabilityFeature::ToolCalling,
        agena::provider::ModelCapabilityFeature::Streaming,
        agena::provider::ModelCapabilityFeature::Reasoning,
        agena::provider::ModelCapabilityFeature::StructuredOutput,
        agena::provider::ModelCapabilityFeature::Temperature,
    ] {
        match capabilities.feature_support(feature) {
            Some(agena::model::CapabilitySupport::Supported) => {
                supported.push(model_catalog_feature_label(feature).to_owned())
            }
            Some(agena::model::CapabilitySupport::Unsupported) => {
                unsupported.push(model_catalog_feature_label(feature).to_owned())
            }
            Some(agena::model::CapabilitySupport::Unknown) | None => {}
        }
    }
    model_catalog_support_summary(i18n, supported, unsupported)
}

fn model_catalog_supported_feature_summary(
    capabilities: &agena::provider::ModelCapabilityPatch,
) -> String {
    [
        agena::provider::ModelCapabilityFeature::ToolCalling,
        agena::provider::ModelCapabilityFeature::Streaming,
        agena::provider::ModelCapabilityFeature::Reasoning,
        agena::provider::ModelCapabilityFeature::StructuredOutput,
        agena::provider::ModelCapabilityFeature::Temperature,
    ]
    .into_iter()
    .filter(|feature| {
        matches!(
            capabilities.feature_support(*feature),
            Some(agena::model::CapabilitySupport::Supported)
        )
    })
    .map(model_catalog_feature_label)
    .collect::<Vec<_>>()
    .join(", ")
}

fn model_catalog_feature_label(feature: agena::provider::ModelCapabilityFeature) -> &'static str {
    match feature {
        agena::provider::ModelCapabilityFeature::ToolCalling => "tools",
        agena::provider::ModelCapabilityFeature::Streaming => "stream",
        agena::provider::ModelCapabilityFeature::Reasoning => "reasoning",
        agena::provider::ModelCapabilityFeature::StructuredOutput => "structured",
        agena::provider::ModelCapabilityFeature::Temperature => "temperature",
    }
}

fn model_catalog_support_summary(
    i18n: &I18n,
    supported: Vec<String>,
    unsupported: Vec<String>,
) -> String {
    let mut parts = Vec::new();
    if !supported.is_empty() {
        parts.push(format_key_value_segment("+", supported.join(", ").as_str()));
    }
    if !unsupported.is_empty() {
        parts.push(format_key_value_segment(
            "-",
            unsupported.join(", ").as_str(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

fn model_catalog_string_list_summary(i18n: &I18n, values: &[String]) -> String {
    if values.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        values.join(", ")
    }
}

fn model_catalog_modes_summary(i18n: &I18n, entry: &CatalogModelResource) -> String {
    let mut parts = Vec::new();
    if let Some(default) = entry
        .default_thinking_mode
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-thinking").as_str(),
            default,
        ));
    }
    let thinking_modes = model_catalog_thinking_mode_names(entry);
    if !thinking_modes.is_empty() {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-thinking-modes").as_str(),
            thinking_modes.as_str(),
        ));
    }
    let speed_modes = model_catalog_speed_mode_names(entry);
    if !speed_modes.is_empty() {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-speed-modes").as_str(),
            speed_modes.as_str(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

fn model_catalog_thinking_mode_names(entry: &CatalogModelResource) -> String {
    entry
        .thinking_modes
        .iter()
        .filter(|(_, mode)| !mode.disabled)
        .map(|(name, mode)| {
            mode.display_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(name.as_str())
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn model_catalog_speed_mode_names(entry: &CatalogModelResource) -> String {
    entry
        .speed_modes
        .iter()
        .filter(|(_, mode)| !mode.disabled)
        .map(|(name, mode)| {
            mode.display_name
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(name.as_str())
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn model_catalog_defaults_summary(i18n: &I18n, entry: &CatalogModelResource) -> String {
    let mut parts = Vec::new();
    if let Some(value) = entry
        .default_verbosity
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-verbosity").as_str(),
            value,
        ));
    }
    if let Some(value) = entry
        .default_temperature
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-temperature").as_str(),
            value,
        ));
    }
    if let Some(value) = entry
        .default_top_p
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-top-p").as_str(),
            value,
        ));
    }
    if let Some(value) = entry.default_top_k {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-default-top-k").as_str(),
            value.to_string().as_str(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

fn model_catalog_runtime_summary(i18n: &I18n, entry: &CatalogModelResource) -> String {
    let mut parts = Vec::new();
    if let Some(value) = entry.supports_parallel_tool_calls {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-parallel-tools").as_str(),
            model_catalog_bool_label(i18n, value).as_str(),
        ));
    }
    if let Some(value) = entry.supports_verbosity {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-supports-verbosity").as_str(),
            model_catalog_bool_label(i18n, value).as_str(),
        ));
    }
    if let Some(value) = entry.assistant_reasoning_interleaved {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-reasoning-interleaved").as_str(),
            model_catalog_bool_label(i18n, value).as_str(),
        ));
    }
    if let Some(value) = entry
        .assistant_reasoning_field
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-reasoning-field").as_str(),
            value,
        ));
    }
    if let Some(value) = entry.open_weights {
        parts.push(format_key_value_segment(
            ui_text::t(i18n, "overlay-model-catalog-open-weights").as_str(),
            model_catalog_bool_label(i18n, value).as_str(),
        ));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

fn model_catalog_bool_label(i18n: &I18n, value: bool) -> String {
    ui_text::t(i18n, if value { "value-yes" } else { "value-no" })
}

fn model_catalog_pricing_summary(
    i18n: &I18n,
    pricing: Option<&agena::model::ModelPricing>,
) -> String {
    let Some(pricing) = pricing.filter(|pricing| !pricing.is_empty()) else {
        return ui_text::t(i18n, "value-unset");
    };
    let mut parts = Vec::new();
    if let Some(value) = pricing.input_usd_per_million_tokens.as_deref() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-price-input",
            &crate::fl_args!("value" => value),
        )));
    }
    if let Some(value) = pricing.output_usd_per_million_tokens.as_deref() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-price-output",
            &crate::fl_args!("value" => value),
        )));
    }
    if let Some(value) = pricing.cache_read_usd_per_million_tokens.as_deref() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-price-cache-read",
            &crate::fl_args!("value" => value),
        )));
    }
    if let Some(value) = pricing.cache_write_usd_per_million_tokens.as_deref() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-price-cache-write",
            &crate::fl_args!("value" => value),
        )));
    }
    if !pricing.tiers.is_empty() {
        parts.push(sanitize_display_text(i18n.text_args(
            "overlay-model-catalog-tier-count",
            &crate::fl_args!("count" => pricing.tiers.len() as i64),
        )));
    }
    if parts.is_empty() {
        ui_text::t(i18n, "value-unset")
    } else {
        join_inline_segments(parts)
    }
}

fn model_catalog_source_summary(entry: &CatalogModelResource) -> String {
    entry
        .source_label
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{:?}", entry.source).to_ascii_lowercase())
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
            format!(" [>] {} ", user_input_submit_label(i18n, &dialog.request)),
            if dialog.state.screen() == QuestionFlowScreen::Review {
                selection_highlight_style()
            } else {
                Style::default()
            },
        ));
    }
    Line::from(spans)
}

fn user_input_overlay_title(i18n: &I18n, request: &UserInputRequest) -> String {
    let title = request.title.trim();
    if title.is_empty() {
        ui_text::t(i18n, "overlay-user-input-title")
    } else {
        sanitize_display_text(title)
    }
}

fn user_input_review_question(request: &UserInputRequest) -> Option<&UserInputQuestion> {
    let question = request.questions.first()?;
    if request.kind.trim() != "review" || request.questions.len() != 1 || question.multiple {
        return None;
    }
    (!question.options.is_empty()).then_some(question)
}

fn user_input_request_is_review(request: &UserInputRequest) -> bool {
    user_input_review_question(request).is_some()
}

fn user_input_submit_label(i18n: &I18n, request: &UserInputRequest) -> String {
    let label = request.submit_label.trim();
    if label.is_empty() {
        ui_text::t(i18n, "overlay-user-input-submit")
    } else {
        sanitize_display_text(label)
    }
}

fn user_input_footer_text(i18n: &I18n, request: &UserInputRequest, key: &str) -> String {
    let mut footer = ui_text::t(i18n, key);
    let cancel = request.cancel_label.trim();
    if !cancel.is_empty() {
        footer.push_str(" · Esc ");
        footer.push_str(sanitize_display_text(cancel).as_str());
    }
    footer
}

fn review_request_body_markdown(body_markdown: &str) -> Text<'static> {
    let markdown = body_markdown.trim();
    if markdown.is_empty() {
        return Text::from(vec![Line::from("")]);
    }
    let rendered = markdown_to_text(markdown);
    Text::from(
        rendered
            .lines
            .into_iter()
            .map(|line| {
                let spans = line
                    .spans
                    .into_iter()
                    .map(|span| Span::styled(sanitize_display_text(span.content), span.style))
                    .collect::<Vec<_>>();
                Line::from(spans)
            })
            .collect::<Vec<_>>(),
    )
}

fn user_input_body_markdown_lines(body_markdown: &str, style: Option<Style>) -> Vec<Line<'static>> {
    let style = style.unwrap_or_default();
    body_markdown
        .lines()
        .map(sanitize_display_text)
        .map(|line| Line::from(Span::styled(line, style)))
        .collect()
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
        let focused = dialog.state.focus() == SettingsStudioFocus::Navigation;
        let selected = index == dialog.state.selected_section_index();
        let marker = if selected && focused { "> " } else { "  " };
        let label = format!("{marker}{}  {}", section.label, section.items.len());
        let line = settings_compact_pad_to_width(label.as_str(), content_width);
        let style = if selected && focused {
            selection_highlight_style()
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

    let fixed_rows = 3usize;
    let visible_item_count = height
        .saturating_sub(fixed_rows as u16)
        .saturating_div(2)
        .max(1) as usize;

    if section.items.is_empty() {
        lines.push(Line::from(sanitize_display_text(ui_text::t(
            i18n,
            "overlay-settings-empty-items",
        ))));
    } else {
        let (start, end) = settings_compact_visible_range(
            section.items.len(),
            dialog.state.selected_item_index(),
            visible_item_count,
        );
        for (index, item) in section.items[start..end].iter().enumerate() {
            let index = start + index;
            let focused = dialog.state.focus() == SettingsStudioFocus::Items;
            let selected = index == dialog.state.selected_item_index();
            let marker = if selected && focused { ">> " } else { "   " };
            let style = if selected && focused {
                selection_highlight_style()
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                settings_compact_item_title_line(item, marker, width),
                style,
            )));
            lines.push(Line::from(Span::styled(
                settings_compact_item_subtitle_line(item, "   ", width),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    Text::from(lines)
}

fn settings_compact_item_title_line(item: &SettingsStudioItem, marker: &str, width: u16) -> String {
    let width = width.max(1) as usize;
    let marker = sanitize_display_text(marker);
    let label = sanitize_display_text(item.label.as_str());
    let value = sanitize_display_text(item.value.as_str());
    if value.trim().is_empty() || width <= UnicodeWidthStr::width(marker.as_str()) + 4 {
        return truncate_display_text(format!("{marker}{label}").as_str(), width);
    }

    let marker_width = UnicodeWidthStr::width(marker.as_str());
    let label_width = UnicodeWidthStr::width(label.as_str());
    let value_width = UnicodeWidthStr::width(value.as_str());
    let full_width = marker_width
        .saturating_add(label_width)
        .saturating_add(value_width)
        .saturating_add(2);
    if full_width <= width {
        let gap = width
            .saturating_sub(marker_width)
            .saturating_sub(label_width)
            .saturating_sub(value_width)
            .max(1);
        return format!("{marker}{label}{}{}", " ".repeat(gap), value);
    }

    let available = width.saturating_sub(marker_width);
    let gap_width = if available > 2 { 2 } else { 1 };
    let content_budget = available.saturating_sub(gap_width);
    if content_budget <= 1 {
        return truncate_display_text(format!("{marker}{label}").as_str(), width);
    }

    let min_value_budget = value_width.min(16).min(content_budget.saturating_sub(1));
    let preferred_label_budget = label_width.min(24);
    let mut label_budget = preferred_label_budget.min(content_budget.saturating_sub(1));
    let mut value_budget = content_budget.saturating_sub(label_budget);
    if value_budget < min_value_budget {
        let min_label_budget = label_width.min(12).min(content_budget.saturating_sub(1));
        let reclaim = min_value_budget
            .saturating_sub(value_budget)
            .min(label_budget.saturating_sub(min_label_budget));
        label_budget = label_budget.saturating_sub(reclaim);
        value_budget = value_budget.saturating_add(reclaim);
    }

    let label = truncate_display_text(label.as_str(), label_budget.max(1));
    let value = truncate_display_text(value.as_str(), value_budget.max(1));
    let label_width = UnicodeWidthStr::width(label.as_str());
    let value_width = UnicodeWidthStr::width(value.as_str());
    let gap = width
        .saturating_sub(marker_width)
        .saturating_sub(value_width)
        .saturating_sub(label_width)
        .max(1);
    format!("{marker}{label}{}{}", " ".repeat(gap), value)
}

fn settings_compact_item_subtitle_line(
    item: &SettingsStudioItem,
    indent: &str,
    width: u16,
) -> String {
    let width = width.max(1) as usize;
    let indent = sanitize_display_text(indent);
    let indent_width = UnicodeWidthStr::width(indent.as_str()).min(width);
    let budget = width.saturating_sub(indent_width).max(1);
    format!(
        "{indent}{}",
        truncate_display_text(sanitize_display_text(item.detail.as_str()).as_str(), budget)
    )
}

fn settings_compact_item_detail_title(i18n: &I18n, dialog: &SettingsStudioOverlay) -> String {
    let detail_label = ui_text::t(i18n, "overlay-workbench-details");
    dialog
        .state
        .selected_item()
        .map(|item| format!("{detail_label}: {}", item.label))
        .unwrap_or(detail_label)
}

fn settings_compact_item_detail_text(i18n: &I18n, dialog: &SettingsStudioOverlay) -> Text<'static> {
    dialog
        .state
        .selected_item()
        .map(|item| {
            let mut lines = vec![Line::from(Span::styled(
                sanitize_display_text(item.detail.as_str()),
                Style::default(),
            ))];
            if let Some(current_value) = item.current_value.as_deref() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    sanitize_display_text(ui_text::t(i18n, "settings-detail-values-heading")),
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-detail-current",
                    &crate::fl_args!("value" => current_value.to_string()),
                ))));
            }
            if let Some(effective_value) = item.effective_value.as_deref() {
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-edit-effective-value",
                    &crate::fl_args!("value" => effective_value.to_string()),
                ))));
            }
            if !item.source_rows.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    sanitize_display_text(ui_text::t(i18n, "settings-detail-sources-heading")),
                    Style::default().add_modifier(Modifier::BOLD),
                )));
                for row in &item.source_rows {
                    lines.push(Line::from(sanitize_display_text(format!(
                        "{}: {}",
                        row.label, row.value
                    ))));
                }
            }
            if let Some(path) = item.path.as_deref() {
                lines.push(Line::from(""));
                lines.push(Line::from(sanitize_display_text(i18n.text_args(
                    "overlay-settings-detail-path",
                    &crate::fl_args!("path" => path.to_string()),
                ))));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(sanitize_display_text(
                settings_item_action_hint(i18n, item),
            )));
            Text::from(lines)
        })
        .unwrap_or_else(|| Text::from(ui_text::t(i18n, "overlay-settings-empty-detail")))
}

fn settings_item_action_hint(i18n: &I18n, item: &SettingsStudioItem) -> String {
    match &item.action {
        SettingsPickerAction::OpenPluginPolicyStudio
        | SettingsPickerAction::OpenPluginWorkbench => {
            ui_text::t(i18n, "settings-detail-action-screen")
        }
        SettingsPickerAction::OpenSessionEffectivePermissionView(_) => {
            ui_text::t(i18n, "settings-detail-action-readonly")
        }
        SettingsPickerAction::OpenConfigFile => ui_text::t(i18n, "settings-detail-action-file"),
        _ => ui_text::t(i18n, "overlay-settings-detail-action"),
    }
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

fn append_permission_action_lines(
    i18n: &I18n,
    lines: &mut Vec<Line<'static>>,
    heading_key: &str,
    actions: &[&PermissionAction],
) {
    if actions.is_empty() {
        return;
    }
    lines.push(Line::from(Span::styled(
        sanitize_display_text(ui_text::t(i18n, heading_key)),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for action in actions {
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                sanitize_display_text(permission_action_label(i18n, action))
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }
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
    lines.push(Line::from(sanitize_display_text(permission_action_label(
        i18n,
        &dialog.request.action,
    ))));
    let requested_actions = permission_requested_actions_for_display(
        Some(&dialog.request.action),
        dialog.request.requested_actions.as_slice(),
    );
    append_permission_action_lines(
        i18n,
        &mut lines,
        "overlay-permission-requested-actions",
        requested_actions.as_slice(),
    );
    let related_actions = permission_related_actions_for_display(
        Some(&dialog.request.action),
        dialog.request.related_actions.as_slice(),
        dialog.request.requested_actions.as_slice(),
    );
    append_permission_action_lines(
        i18n,
        &mut lines,
        "overlay-permission-related-actions",
        related_actions.as_slice(),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_dialog_with_focus(focus: SettingsStudioFocus) -> SettingsStudioOverlay {
        SettingsStudioOverlay {
            title: "Settings".to_string(),
            footer: String::new(),
            state: SectionedListState::new(
                vec![
                    SettingsStudioSection {
                        id: SettingsStudioSectionId::ConfigProviders,
                        label: "Providers".to_string(),
                        summary: String::new(),
                        description: "Provider settings".to_string(),
                        items: vec![
                            SettingsStudioItem::new(
                                "Default Provider",
                                "github",
                                "Provider used by default.",
                                SettingsPickerAction::OpenProviderList,
                            ),
                            SettingsStudioItem::new(
                                "Provider List",
                                "1 provider",
                                "Open provider list.",
                                SettingsPickerAction::OpenProviderList,
                            ),
                        ],
                    },
                    SettingsStudioSection {
                        id: SettingsStudioSectionId::ConfigAgents,
                        label: "Agents".to_string(),
                        summary: String::new(),
                        description: "Agent settings".to_string(),
                        items: vec![SettingsStudioItem::new(
                            "Default Agent",
                            "build",
                            "Agent used by default.",
                            SettingsPickerAction::OpenAgentList,
                        )],
                    },
                ],
                0,
                0,
                focus,
            ),
        }
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn find_line<'a>(text: &'a Text<'_>, needle: &str) -> &'a Line<'a> {
        text.lines
            .iter()
            .find(|line| line_text(line).contains(needle))
            .unwrap_or_else(|| panic!("missing line containing {needle:?}: {text:?}"))
    }

    fn text_plain(text: &Text<'_>) -> String {
        text.lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn settings_item_title_uses_available_width_for_value() {
        let item = SettingsStudioItem::new(
            "Default",
            "github / openai / gpt-5.2-codex",
            "Default provider route",
            SettingsPickerAction::OpenProviderDefaultWizard,
        );

        let line = settings_compact_item_title_line(&item, ">> ", 96);

        assert!(line.contains("github / openai / gpt-5.2-codex"));
        assert!(
            !line.contains("..."),
            "wide value should not truncate: {line}"
        );
        assert!(UnicodeWidthStr::width(line.as_str()) <= 96);
    }

    #[test]
    fn settings_item_title_truncates_only_when_constrained() {
        let item = SettingsStudioItem::new(
            "Very Long Setting Name",
            "github / openai / gpt-5.2-codex",
            "Default provider route",
            SettingsPickerAction::OpenProviderDefaultWizard,
        );

        let line = settings_compact_item_title_line(&item, ">> ", 32);

        assert!(line.contains("..."), "narrow value should truncate: {line}");
        assert!(UnicodeWidthStr::width(line.as_str()) <= 32);
    }

    fn sample_catalog_model() -> CatalogModelResource {
        CatalogModelResource {
            model_id: "gpt-test".to_string(),
            source: agena_api_server::local_api::ModelCatalogSourceKind::Generated,
            source_label: Some("generated catalog".to_string()),
            display_name: Some("GPT Test".to_string()),
            origin: Some("OpenAI".to_string()),
            lifecycle: Some(agena::model::ModelLifecycle::Preview),
            context_window_tokens: Some(128_000),
            max_input_tokens: Some(64_000),
            max_output_tokens: Some(16_000),
            description: Some("A detailed test model.".to_string()),
            knowledge_cutoff: Some("2025-10".to_string()),
            release_date: Some("2026-01-15".to_string()),
            last_updated: Some("2026-02-01".to_string()),
            open_weights: Some(false),
            default_thinking_mode: Some("medium".to_string()),
            supports_parallel_tool_calls: Some(true),
            supports_verbosity: Some(true),
            default_verbosity: Some("medium".to_string()),
            default_temperature: Some("0.2".to_string()),
            default_top_p: Some("0.95".to_string()),
            default_top_k: Some(40),
            assistant_reasoning_interleaved: Some(true),
            assistant_reasoning_field: Some("reasoning".to_string()),
            output_modalities: vec!["text".to_string(), "image".to_string()],
            pricing: Some(agena::model::ModelPricing {
                input_usd_per_million_tokens: Some("1.25".to_string()),
                output_usd_per_million_tokens: Some("10.00".to_string()),
                cache_read_usd_per_million_tokens: Some("0.10".to_string()),
                cache_write_usd_per_million_tokens: None,
                tiers: Vec::new(),
            }),
            thinking_modes: std::collections::BTreeMap::from([(
                "medium".to_string(),
                agena::provider::ConfiguredModelThinkingMode {
                    display_name: Some("Medium".to_string()),
                    ..Default::default()
                },
            )]),
            speed_modes: std::collections::BTreeMap::from([(
                "fast".to_string(),
                agena::provider::ConfiguredModelSpeedMode {
                    display_name: Some("Fast".to_string()),
                    ..Default::default()
                },
            )]),
            capabilities: agena::provider::ModelCapabilityPatch {
                input: Some(
                    agena::provider::CapabilitySelectionPatch::from_supported_unsupported(
                        vec![
                            agena::model::ModelInputModality::Text,
                            agena::model::ModelInputModality::Image,
                        ],
                        vec![agena::model::ModelInputModality::Audio],
                    ),
                ),
                features: Some(
                    agena::provider::CapabilitySelectionPatch::from_supported_unsupported(
                        vec![
                            agena::provider::ModelCapabilityFeature::ToolCalling,
                            agena::provider::ModelCapabilityFeature::Reasoning,
                        ],
                        vec![agena::provider::ModelCapabilityFeature::Temperature],
                    ),
                ),
            },
        }
    }

    #[test]
    fn model_catalog_list_subtitle_includes_core_metadata() {
        let i18n = I18n::english();
        let entry = sample_catalog_model();
        let subtitle = model_catalog_list_subtitle(&i18n, &entry);

        assert!(subtitle.contains("GPT Test"));
        assert!(subtitle.contains("OpenAI"));
        assert!(subtitle.contains("preview"));
        assert!(subtitle.contains("ctx 128k"));
        assert!(subtitle.contains("tools"));
        assert!(subtitle.contains("reasoning"));
        assert!(subtitle.contains("in $1.25/M"), "subtitle was: {subtitle}");
    }

    #[test]
    fn model_catalog_detail_text_includes_extended_metadata() {
        let i18n = I18n::english();
        let entry = sample_catalog_model();
        let detail = model_catalog_detail_text(&i18n, &entry);
        let plain = text_plain(&detail);

        assert!(plain.contains("Lifecycle"));
        assert!(plain.contains("release 2026-01-15"));
        assert!(plain.contains("+=text, image"));
        assert!(plain.contains("-=audio"));
        assert!(plain.contains("+=tools, reasoning"));
        assert!(plain.contains("-=temperature"));
        assert!(plain.contains("thinking=medium"));
        assert!(plain.contains("thinking modes=Medium"));
        assert!(plain.contains("speed modes=Fast"));
        assert!(plain.contains("verbosity=medium"));
        assert!(plain.contains("parallel tools=yes"));
        assert!(plain.contains("in $1.25/M"));
        assert!(plain.contains("generated catalog"));
        assert!(plain.contains("A detailed test model."));
    }

    #[test]
    fn settings_compact_view_highlights_only_navigation_when_navigation_has_focus() {
        let i18n = I18n::english();
        let dialog = settings_dialog_with_focus(SettingsStudioFocus::Navigation);
        let section_text = settings_compact_sections_text(&i18n, &dialog, 28);
        let editor_text =
            settings_compact_editor_text(&i18n, &dialog, dialog.state.selected_section(), 48, 12);

        let provider_line = find_line(&section_text, "Providers");
        let default_provider_line = find_line(&editor_text, "Default Provider");

        assert!(line_text(provider_line).starts_with("> Providers"));
        assert_eq!(provider_line.spans[0].style, selection_highlight_style());
        assert!(line_text(default_provider_line).starts_with("   Default Provider"));
        assert_eq!(default_provider_line.spans[0].style, Style::default());
    }

    #[test]
    fn settings_compact_view_highlights_only_items_when_items_have_focus() {
        let i18n = I18n::english();
        let dialog = settings_dialog_with_focus(SettingsStudioFocus::Items);
        let section_text = settings_compact_sections_text(&i18n, &dialog, 28);
        let editor_text =
            settings_compact_editor_text(&i18n, &dialog, dialog.state.selected_section(), 48, 12);

        let provider_line = find_line(&section_text, "Providers");
        let default_provider_line = find_line(&editor_text, "Default Provider");

        assert!(line_text(provider_line).starts_with("  Providers"));
        assert_eq!(provider_line.spans[0].style, Style::default());
        assert!(line_text(default_provider_line).starts_with(">> Default Provider"));
        assert_eq!(
            default_provider_line.spans[0].style,
            selection_highlight_style()
        );
    }

    #[test]
    fn permission_overlay_body_lists_requested_and_related_actions() {
        let i18n = I18n::english();
        let lines = permission_overlay_body_lines(
            &i18n,
            &PermissionOverlay {
                session_id: 1,
                request: PermissionRequest {
                    request_id: "perm-1".to_string(),
                    session_id: Some(1),
                    action: PermissionAction::Tool {
                        tool_name: "bash".to_string(),
                        qualifier: Some("npm test".to_string()),
                    },
                    related_actions: vec![
                        PermissionAction::Tool {
                            tool_name: "bash".to_string(),
                            qualifier: Some("npm test".to_string()),
                        },
                        PermissionAction::PathAccess {
                            access_kind: "read".to_string(),
                            workspace_root: "/workspace".to_string(),
                            target_path: "/workspace/notes.txt".to_string(),
                        },
                        PermissionAction::NetworkAccess {
                            target: "https://api.example.com/health".to_string(),
                            host: "api.example.com".to_string(),
                            port: Some(443),
                        },
                    ],
                    requested_actions: vec![
                        PermissionAction::Tool {
                            tool_name: "bash".to_string(),
                            qualifier: Some("npm test".to_string()),
                        },
                        PermissionAction::PathAccess {
                            access_kind: "read".to_string(),
                            workspace_root: "/workspace".to_string(),
                            target_path: "/workspace/notes.txt".to_string(),
                        },
                    ],
                    reason: "needs approval".to_string(),
                    explanation: String::new(),
                    source: None,
                    scope: None,
                    operator: None,
                    risk: PermissionRiskLevel::Medium,
                    trace: Vec::new(),
                    created_at: chrono::Utc::now(),
                },
                selection: SelectionCursor::default(),
            },
        );
        let plain = lines.iter().map(line_text).collect::<Vec<_>>().join("\n");

        assert!(plain.contains("tool: bash · npm test"), "body was: {plain}");
        assert!(plain.contains("Requested Actions"), "body was: {plain}");
        assert!(
            plain.contains("path read: /workspace/notes.txt"),
            "body was: {plain}"
        );
        assert!(plain.contains("Related Actions"), "body was: {plain}");
        assert!(
            plain.contains("network: api.example.com:443"),
            "body was: {plain}"
        );
    }
}
