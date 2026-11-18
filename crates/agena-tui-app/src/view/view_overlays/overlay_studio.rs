use super::super::{
    App, BoundedListPanelHeight, DetailTextLine, DetailTextSpec, Frame, Line,
    ListWorkbenchDialogSpec, ListWorkbenchPanelState, Modifier, PermissionRuleStudioOverlay,
    PermissionStudioOverlay, Rect, SettingsStudioOverlay, Span, Style, SurfaceMode, Text,
    WorkbenchOverlayDialogSpec, WorkbenchTextSection, build_accented_two_line_list_item,
    build_detail_text, permission_rule_draft_label, permission_rule_mode_label,
    permission_rule_studio_detail_text, permission_rule_subject_kind_name,
    render_list_workbench_dialog, sanitize_display_text, selection_highlight_style,
    settings_item_detail_text, settings_item_detail_title,
};
use crate::ui_text;

impl App {
    pub(crate) fn render_settings_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &SettingsStudioOverlay,
        surface: SurfaceMode,
    ) {
        let (detail_title, detail) = if dialog.state.details_open() {
            (
                settings_item_detail_title(&self.i18n, dialog),
                settings_item_detail_text(&self.i18n, dialog),
            )
        } else {
            (String::new(), Text::default())
        };
        agena_tui_settings::render_settings(
            frame,
            area,
            surface,
            &dialog.state,
            &self.i18n,
            &agena_tui_settings::SettingsViewSpec {
                title: &dialog.title,
                detail_title: &detail_title,
                detail,
            },
        );
    }

    pub(crate) fn render_permission_studio_overlay(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PermissionStudioOverlay,
        surface: SurfaceMode,
    ) {
        use agena_tui_components::{WorkbenchFrameSpec, pointer, render_workbench_frame};
        use ratatui::widgets::{List, ListItem, ListState};

        let rows = crate::permission_studio_content_rows(dialog);
        let selected = crate::selected_content_row(
            &rows,
            dialog.state.selected_section_index(),
            dialog.state.selected_item_index(),
        );
        let spec = WorkbenchFrameSpec::new(
            sanitize_display_text(dialog.title.as_str()).into(),
            sanitize_display_text(dialog.footer.as_str()).into(),
            140,
            u16::try_from(rows.len()).unwrap_or(u16::MAX),
        )
        .summary((!dialog.editable).then(|| self.i18n.text("value-read-only").into()))
        .footer_max_height(area.height.saturating_sub(6).max(2));
        let workbench = render_workbench_frame(frame, area, surface, &spec);
        let width = workbench.body.width.saturating_sub(2) as usize;
        let columns = super::super::permission_studio_columns(&self.i18n, dialog, width);
        let items = rows
            .iter()
            .map(|row| match *row {
                crate::PermissionStudioRow::Section(index) => {
                    let section = &dialog.state.sections()[index];
                    ListItem::new(Line::from(Span::styled(
                        sanitize_display_text(section.label.as_str()),
                        Style::default()
                            .fg(agena_tui_components::theme::accent_color())
                            .add_modifier(Modifier::BOLD),
                    )))
                }
                crate::PermissionStudioRow::Item { section, item } => {
                    let section = &dialog.state.sections()[section];
                    ListItem::new(super::super::permission_studio_dense_line(
                        &self.i18n,
                        &section.items[item],
                        section.id,
                        &columns,
                        width,
                    ))
                }
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default().with_selected(selected);
        frame.render_stateful_widget(
            List::new(items)
                .highlight_style(selection_highlight_style())
                .highlight_symbol("› "),
            workbench.body,
            &mut state,
        );

        // Use the same flattened rows as keyboard navigation. All rows occupy
        // one terminal line, so a scrolled click retains its section ownership.
        let panel = pointer::next_list();
        pointer::register(
            workbench.body,
            Some(pointer::PointerAction::FocusList(panel)),
            Some((
                pointer::key(crossterm::event::KeyCode::Up),
                pointer::key(crossterm::event::KeyCode::Down),
            )),
        );
        for index in state.offset()
            ..rows
                .len()
                .min(state.offset() + workbench.body.height as usize)
        {
            pointer::register(
                Rect::new(
                    workbench.body.x,
                    workbench.body.y + (index - state.offset()) as u16,
                    workbench.body.width,
                    1,
                ),
                Some(pointer::PointerAction::List {
                    panel,
                    index,
                    selected: selected.unwrap_or(0),
                }),
                None,
            );
        }
        if let Some(editor) = &dialog.editor {
            agena_tui_components::render_workbench_editor_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                editor.title.as_str().into(),
                editor.prompt.as_str().into(),
                editor.footer.as_str().into(),
                editor.multiline,
                &editor.input,
            );
        }
    }

    pub(crate) fn render_permission_rule_studio_overlay(
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
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-mode"),
                    sanitize_display_text(permission_rule_mode_label(dialog.draft.mode)),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-scope"),
                    sanitize_display_text(dialog.draft.scope.as_str()),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
                DetailTextLine::labeled(
                    ui_text::t(&self.i18n, "overlay-permission-rule-overview-session"),
                    sanitize_display_text(if dialog.draft.session_id.trim().is_empty() {
                        ui_text::t(&self.i18n, "value-unset")
                    } else {
                        dialog.draft.session_id.clone()
                    }),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
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
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                    Style::default(),
                ),
            ],
            &DetailTextSpec::label_width(14),
        );
        let detail_text = dialog
            .presentation
            .list
            .selected_item()
            .map(|item| permission_rule_studio_detail_text(&self.i18n, &dialog.draft, item))
            .unwrap_or_else(|| ui_text::t(&self.i18n, "overlay-permission-rule-empty-detail"));

        let list_items = dialog
            .presentation
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
            sanitize_display_text(dialog.presentation.title.as_str()).into(),
            sanitize_display_text(dialog.presentation.footer.as_str()).into(),
            ListWorkbenchPanelState::items(
                BoundedListPanelHeight {
                    lines_per_item: 2,
                    min_body_height: 8,
                    max_body_height: 18,
                },
                Some(ui_text::t(&self.i18n, "overlay-permission-rule-fields").into()),
                list_items.as_slice(),
                (!dialog.presentation.list.items.is_empty())
                    .then_some(dialog.presentation.list.selected),
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
        .summary(Some(
            sanitize_display_text(permission_rule_subject_kind_name(dialog.draft.subject_kind))
                .into(),
        ))
        .target_width(132)
        .left_panel_width(40)
        .overlay(
            dialog
                .editor
                .as_ref()
                .map(WorkbenchOverlayDialogSpec::from_source),
        );
        let spec = spec.footer_max_height(area.height.saturating_sub(6).max(2));
        render_list_workbench_dialog(frame, area, surface, &spec);
    }
}
