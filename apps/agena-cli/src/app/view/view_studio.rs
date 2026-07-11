impl App {
    pub(in crate::app) fn render_provider_studio_overlay(
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
                            model.id.as_ref(),
                        ) {
                            "[x]"
                        } else {
                            "[ ]"
                        };
                        build_detail_two_line_list_item(
                            sanitize_display_text(format!("{selected} {}", model.id)).into(),
                            Some(
                                sanitize_display_text(provider_studio_model_list_detail(
                                    &self.i18n,
                                    dialog,
                                    adapter_models.adapter_id.as_str(),
                                    model.id.as_ref(),
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
        let lead_panel = dialog.show_provider_list.then(|| {
            DashboardLeadPanelSpec::new(
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
            )
        });
        let dashboard_spec = DashboardWorkbenchSpec::new(
            sanitize_display_text(dialog.title.as_str()).into(),
            sanitize_display_text(dialog.footer.as_str()).into(),
            122,
            lead_panel,
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
                    Some(sanitize_display_text(model_page.footer.as_str()).into()),
                    96,
                    DetailTextSpec::label_width(18),
                    (6, 20),
                    (1, 2),
                    None,
                    Style::default(),
                ),
                lines,
            ))
        } else if let Some(detail_page) = dialog.detail_page.as_ref() {
            let detail_fields = provider_studio_detail_fields(dialog);
            let auth_state_lines = provider_studio_auth_state_lines(&self.i18n, dialog);
            let mut lines = Vec::with_capacity(auth_state_lines.len() + detail_fields.len());
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
                    Some(sanitize_display_text(detail_page.footer.as_str()).into()),
                    92,
                    detail_text_spec,
                    (4, 20),
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
            Some(DashboardWorkbenchOverlaySpec::from_sources(
                detail_overlay,
                dialog.editor.as_ref(),
            )),
        );
    }

    pub(in crate::app) fn render_model_catalog_studio_overlay(
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
            Some(sanitize_display_text(summary.as_str()).into()),
            sanitize_display_text(dialog.workbench.footer.as_str()).into(),
            136,
            48,
            Some(48),
            Some(34),
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
            dialog
                .workbench
                .editor
                .as_ref()
                .map(WorkbenchOverlayDialogSpec::from_source),
        );
        render_list_workbench_dialog(frame, area, surface, &spec);
    }
}
use super::{
    App, BoundedListPanelHeight, Color, DashboardDetailOverlaySpec, DashboardLeadPanelSpec,
    DashboardListPanelHeight, DashboardListPanelState, DashboardSplitPanelsSpec,
    DashboardTextPanelHeight, DashboardTextSection, DashboardWorkbenchOverlaySpec,
    DashboardWorkbenchSpec, DetailTextDialogSpec, DetailTextLine, DetailTextSpec, Frame,
    ListWorkbenchDialogSpec, ListWorkbenchPanelState, ModelCatalogStudioOverlay, Modifier,
    ProviderStudioFocus, ProviderStudioOverlay, Rect, Style, SurfaceMode, Text,
    WorkbenchOverlayDialogSpec, WorkbenchTextSection, build_detail_text,
    build_detail_two_line_list_item, model_catalog_detail_text, model_catalog_list_subtitle,
    provider_model_config_field_display, provider_model_config_field_editable,
    provider_model_config_field_label, provider_model_config_fields,
    provider_studio_adapter_list_detail, provider_studio_adapter_selectable,
    provider_studio_auth_state_lines, provider_studio_detail_fields,
    provider_studio_detail_text_spec, provider_studio_field_editable, provider_studio_field_label,
    provider_studio_main_field_display, provider_studio_model_list_detail,
    provider_studio_model_selected, provider_studio_selected_adapter_models,
    provider_studio_visible_fields, render_dashboard_workbench_dialog,
    render_list_workbench_dialog, sanitize_display_text, selection_highlight_style,
    truncate_display_text, ui_text,
};
