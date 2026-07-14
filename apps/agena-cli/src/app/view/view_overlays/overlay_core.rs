use super::super::{
    App, Borders, ChoiceOverlay, ConfirmOverlay, FileAttachOverlay, Frame, Line, ListItem,
    ListPanelSection, ListPanelSpec, Modifier, Overlay, ParagraphSection, PathBrowserOverlay,
    PermissionOverlay, PermissionOverlayPage, PickerOverlay, QuestionFlowCustomInputSpec,
    QuestionFlowDialogMode, QuestionFlowDialogSpec, QuestionFlowScreen, Rect, Route,
    SearchPickerViewState, SessionModelChooserOverlay, SessionSearchOverlay, Span,
    StackedDialogSection, StackedDialogSectionHeight, StackedDialogSpec, Style, SurfaceMode, Text,
    TextPanelSection, TextPanelSpec, TimelineOverlay, UserInputOverlay, WorkbenchTextSection,
    adaptive_modal_width, build_detail_two_line_list_item, list_panel_height,
    permission_overlay_body_lines, permission_overlay_choice_lines, permission_overlay_footer,
    permission_overlay_title, render_confirm_dialog, render_overlay_line_input_dialog,
    render_question_flow_dialog, render_search_picker_dialog,
    render_search_picker_dialog_with_preview, render_stacked_dialog, review_request_body_markdown,
    sanitize_display_str, sanitize_display_text, selection_highlight_style, ui_text,
    user_input_answer_summary, user_input_answer_values, user_input_body_markdown_lines,
    user_input_custom_values_preview, user_input_footer_text, user_input_markdown_text,
    user_input_nav_line, user_input_option_description_preview, user_input_overlay_title,
    user_input_question_label, user_input_request_is_review, user_input_review_answer_preview,
    user_input_review_question, user_input_submit_label, user_input_timeout_line,
    user_input_timeout_text, wrapped_text_height_for_text,
};

impl App {
    pub(in crate::app) fn render_overlay(&self, frame: &mut Frame, area: Rect) {
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
            Overlay::Choice(dialog) => {
                self.render_choice_overlay(frame, area, dialog);
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
                self.render_session_search_overlay(frame, area, dialog);
            }
            Overlay::Picker(dialog) => {
                self.render_picker_overlay(frame, area, dialog);
            }
            Overlay::Timeline(dialog) => {
                self.render_timeline_overlay(frame, area, dialog);
            }
            Overlay::ProviderStudio(dialog) => {
                self.render_provider_studio_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
            Overlay::ModelCatalogStudio(dialog) => {
                self.render_model_catalog_studio_overlay(frame, area, dialog, SurfaceMode::Overlay);
            }
        }
    }

    pub(in crate::app) fn render_route(&self, frame: &mut Frame, area: Rect) {
        self.render_route_content(frame, area, &self.current_route);
        self.render_overlay(frame, area);
    }

    pub(in crate::app) fn render_route_content(
        &self,
        frame: &mut Frame,
        area: Rect,
        route: &Route,
    ) {
        match route {
            Route::Main => {}
            Route::Usage(dialog) => {
                self.render_usage_dashboard(frame, area, dialog, SurfaceMode::Route)
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
                self.render_session_search_overlay(frame, area, dialog);
            }
            Route::Picker(dialog) => {
                self.render_picker_overlay(frame, area, dialog);
            }
            Route::SessionModelChooser(dialog) => {
                self.render_session_model_chooser_overlay(frame, area, dialog);
            }
            Route::Timeline(dialog) => {
                self.render_timeline_overlay(frame, area, dialog);
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
    }

    pub(in crate::app) fn render_file_attach_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &FileAttachOverlay,
    ) {
        render_search_picker_dialog(
            frame,
            area,
            dialog,
            &self.standard_search_picker_dialog_spec("overlay-attach-matches"),
            sanitize_display_str,
        );
    }

    pub(in crate::app) fn render_path_browser_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PathBrowserOverlay,
    ) {
        render_search_picker_dialog(
            frame,
            area,
            dialog,
            &self.standard_search_picker_dialog_spec("overlay-path-browser-list-title"),
            sanitize_display_str,
        );
    }

    pub(in crate::app) fn render_permission_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionOverlay,
    ) {
        let body = Text::from(permission_overlay_body_lines(&self.i18n, dialog));
        let footer = Text::from(permission_overlay_footer(&self.i18n, dialog.page));
        let mut sections = vec![StackedDialogSection::Paragraph(ParagraphSection {
            height: StackedDialogSectionHeight::AutoText { min: 4, max: 18 },
            title: None,
            borders: Borders::NONE,
            body,
            wrap: true,
            scroll: None,
            alignment: None,
        })];
        if !matches!(dialog.page, PermissionOverlayPage::Details(_)) {
            let choices = Text::from(permission_overlay_choice_lines(&self.i18n, dialog));
            sections.push(StackedDialogSection::Paragraph(ParagraphSection {
                height: StackedDialogSectionHeight::AutoText { min: 3, max: 6 },
                title: None,
                borders: Borders::NONE,
                body: choices,
                wrap: true,
                scroll: None,
                alignment: None,
            }));
        }
        sections.push(StackedDialogSection::Paragraph(ParagraphSection {
            height: StackedDialogSectionHeight::AutoText { min: 1, max: 2 },
            title: None,
            borders: Borders::NONE,
            body: footer,
            wrap: true,
            scroll: None,
            alignment: None,
        }));
        render_stacked_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            &StackedDialogSpec {
                title: permission_overlay_title(&self.i18n, dialog.page).into(),
                target_width: 84,
                sections,
            },
        );
    }

    pub(in crate::app) fn render_user_input_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &UserInputOverlay,
    ) {
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
        let nav_color = agena_tui_components::theme::info_color();
        let title = user_input_overlay_title(&self.i18n, &dialog.request);
        if dialog.state.screen() == QuestionFlowScreen::Review {
            let mut nav_lines = vec![
                Line::from(Span::styled(
                    sanitize_display_text(self.i18n.text_args(
                        "overlay-user-input-request-id",
                        &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
                    )),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )),
                user_input_nav_line(&self.i18n, dialog, nav_color),
            ];
            if let Some(timeout) = user_input_timeout_line(&self.i18n, &dialog.request) {
                nav_lines.push(timeout);
            }
            let nav_body = Text::from(nav_lines);

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
                        Style::default().fg(agena_tui_components::theme::muted_color())
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
                    Some(&nav_body),
                    QuestionFlowDialogMode::review(
                        ui_text::t(&self.i18n, "overlay-user-input-summary").into(),
                        &review_body,
                        &footer,
                    ),
                ),
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
                    None,
                    QuestionFlowDialogMode::empty(
                        ui_text::t(&self.i18n, "overlay-user-input-detail").into(),
                        &detail_body,
                        12,
                    ),
                ),
            );
            return;
        };

        let mut nav_lines = vec![
            Line::from(Span::styled(
                sanitize_display_text(self.i18n.text_args(
                    "overlay-user-input-request-id",
                    &crate::fl_args!("request_id" => dialog.request.request_id.clone()),
                )),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )),
            user_input_nav_line(&self.i18n, dialog, nav_color),
        ];
        if let Some(timeout) = user_input_timeout_line(&self.i18n, &dialog.request) {
            nav_lines.push(timeout);
        }
        let nav_body = Text::from(nav_lines);

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
                Style::default().fg(agena_tui_components::theme::muted_color()),
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
                        Style::default().fg(agena_tui_components::theme::muted_color())
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
                        Style::default().fg(agena_tui_components::theme::muted_color())
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
                        Style::default().fg(agena_tui_components::theme::muted_color())
                    }
                } else if custom_selected {
                    custom_style
                } else {
                    Style::default().fg(nav_color)
                },
            )));
        }
        let choices_body = Text::from(option_lines);
        let focused_preview = question
            .options
            .get(dialog.state.selected_option())
            .filter(|option| !option.preview_markdown.trim().is_empty());
        let preview_title = focused_preview.map(|option| {
            self.i18n.text_args(
                "overlay-user-input-preview",
                &crate::fl_args!("label" => sanitize_display_text(option.label.as_str())),
            )
        });
        let preview_body = focused_preview
            .map(|option| user_input_markdown_text(option.preview_markdown.as_str(), None));
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
                Some(&nav_body),
                QuestionFlowDialogMode::question(
                    ui_text::t(&self.i18n, "overlay-user-input-prompt-panel").into(),
                    &prompt_body,
                    ui_text::t(&self.i18n, "overlay-user-input-choices").into(),
                    &choices_body,
                    preview_title.map(Into::into),
                    preview_body.as_ref(),
                    custom_input,
                    &footer,
                ),
            ),
        );
        if let Some(cursor) = result.cursor {
            frame.set_cursor_position(cursor);
        }
    }

    pub(in crate::app) fn render_user_input_review_overlay(
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
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                )
            })
            .collect::<Vec<ListItem<'static>>>();
        let cancel_label = {
            let label = dialog.request.cancel_label.trim();
            if label.is_empty() { "cancel" } else { label }
        };
        let mut footer_lines = vec![Line::from(Span::styled(
            sanitize_display_text(format!(
                "Enter {} · Ctrl+X {} · ↑/↓ choose · PgUp/PgDn scroll",
                user_input_submit_label(&self.i18n, &dialog.request),
                cancel_label
            )),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        ))];
        if let Some(timeout) = user_input_timeout_text(&self.i18n, &dialog.request) {
            footer_lines.push(Line::from(Span::styled(
                format!("◷ {timeout}"),
                Style::default().fg(agena_tui_components::theme::warning_color()),
            )));
        }
        let footer = Text::from(footer_lines);
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

    pub(in crate::app) fn render_confirm_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &ConfirmOverlay,
    ) {
        render_confirm_dialog(frame, area, dialog, |text| sanitize_display_text(text));
    }

    pub(in crate::app) fn render_session_search_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SessionSearchOverlay,
    ) {
        render_search_picker_dialog(
            frame,
            area,
            dialog,
            &self.standard_search_picker_dialog_spec("overlay-attach-matches"),
            sanitize_display_str,
        );
    }

    pub(in crate::app) fn render_picker_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PickerOverlay,
    ) {
        render_search_picker_dialog(
            frame,
            area,
            dialog,
            &self.standard_search_picker_dialog_spec("overlay-attach-matches"),
            sanitize_display_str,
        );
    }

    pub(in crate::app) fn render_session_model_chooser_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SessionModelChooserOverlay,
    ) {
        render_search_picker_dialog(
            frame,
            area,
            dialog,
            &self.standard_search_picker_dialog_spec("overlay-session-model-list-title"),
            sanitize_display_str,
        );
    }

    pub(in crate::app) fn render_choice_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &ChoiceOverlay,
    ) {
        render_search_picker_dialog(
            frame,
            area,
            dialog,
            &self.standard_search_picker_dialog_spec("overlay-attach-matches"),
            sanitize_display_str,
        );
    }

    pub(in crate::app) fn render_timeline_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &TimelineOverlay,
    ) {
        let spec = self.standard_search_picker_dialog_spec("overlay-timeline-events");
        render_search_picker_dialog_with_preview(
            frame,
            area,
            dialog,
            &spec,
            sanitize_display_str,
            |state| {
                let detail = match state {
                    SearchPickerViewState::Loading { message }
                    | SearchPickerViewState::Empty { message }
                    | SearchPickerViewState::Error { message } => {
                        Text::from(sanitize_display_text(message))
                    }
                    SearchPickerViewState::Selected(item) => item.detail_body.clone(),
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
}
