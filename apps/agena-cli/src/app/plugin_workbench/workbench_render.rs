impl App {
    pub(in crate::app) fn render_plugin_workbench(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PluginWorkbenchOverlay,
        surface: SurfaceMode,
    ) {
        let surface = render_framed_surface(
            frame,
            area,
            surface,
            &FramedSurfaceSpec {
                title: clean(format!(
                    "{} · {}",
                    dialog.title,
                    plugin_workbench_summary(dialog)
                ))
                .into(),
                target_width: 150,
                target_height: 42,
            },
        );
        match dialog.mode {
            PluginWorkbenchMode::List => render_plugin_list_page(frame, surface.inner, dialog),
            PluginWorkbenchMode::Detail => render_plugin_detail_page(frame, surface.inner, dialog),
        }
        render_plugin_workbench_editor_overlay(frame, area, surface.outer, dialog);
    }

    pub(in crate::app) fn render_plugin_policy_studio(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PluginPolicyStudioOverlay,
        surface: SurfaceMode,
    ) {
        let section_title = dialog
            .selected_section()
            .map(|section| section.label.clone())
            .unwrap_or_else(|| "Plugins".to_owned());
        let surface = render_framed_surface(
            frame,
            area,
            surface,
            &FramedSurfaceSpec {
                title: clean(format!("{} / {}", dialog.title, section_title)).into(),
                target_width: 152,
                target_height: 42,
            },
        );
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(1)])
            .split(surface.inner);
        let inner = rows[0];
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(
                    inner
                        .width
                        .saturating_mul(24)
                        .saturating_div(100)
                        .clamp(22, 34),
                ),
                Constraint::Length(1),
                Constraint::Min(36),
                Constraint::Length(1),
                Constraint::Length(
                    inner
                        .width
                        .saturating_mul(28)
                        .saturating_div(100)
                        .clamp(28, 44),
                ),
            ])
            .split(inner);

        frame.render_widget(
            Paragraph::new(plugin_policy_sections_text(
                dialog,
                columns[0].width,
                columns[0].height,
            ))
            .wrap(Wrap { trim: false }),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new(compact_vertical_divider(columns[1].height)).wrap(Wrap { trim: false }),
            columns[1],
        );
        frame.render_widget(
            Paragraph::new(plugin_policy_table_text(
                dialog,
                columns[2].width,
                columns[2].height,
            ))
            .wrap(Wrap { trim: false }),
            columns[2],
        );
        frame.render_widget(
            Paragraph::new(compact_vertical_divider(columns[3].height)).wrap(Wrap { trim: false }),
            columns[3],
        );
        render_text_panel(
            frame,
            columns[4],
            &TextPanelSpec {
                title: Some(plugin_policy_detail_title(dialog).into()),
                body: &plugin_policy_detail_text(dialog),
                wrap: true,
                scroll: None,
                alignment: None,
            },
        );
        frame.render_widget(
            Paragraph::new(clean(dialog.footer.as_str())).wrap(Wrap { trim: false }),
            rows[1],
        );
    }

    pub(in crate::app) fn paste_plugin_workbench(dialog: &mut PluginWorkbenchOverlay, text: &str) {
        if let Some(editor) = dialog.editor.as_mut() {
            editor.input.flush_all_pending_input();
            editor.input.insert_str(text);
            return;
        }
        if dialog.mode == PluginWorkbenchMode::List {
            dialog.query.flush_all_pending_input();
            dialog.query.insert_str(text);
            refresh_plugin_workbench_filter(dialog);
        }
    }

    pub(in crate::app) fn flush_plugin_workbench_input(
        dialog: &mut PluginWorkbenchOverlay,
        now: Instant,
    ) {
        if let Some(editor) = dialog.editor.as_mut() {
            editor.input.flush_pending_input_if_due(now);
        }
        dialog.query.flush_pending_input_if_due(now);
    }
}
use super::{
    App, Constraint, Direction, Frame, FramedSurfaceSpec, Instant, Layout, Paragraph,
    PluginPolicyStudioOverlay, PluginWorkbenchMode, PluginWorkbenchOverlay, Rect, SurfaceMode,
    TextPanelSpec, Wrap, clean, compact_vertical_divider, plugin_policy_detail_text,
    plugin_policy_detail_title, plugin_policy_sections_text, plugin_policy_table_text,
    plugin_workbench_summary, refresh_plugin_workbench_filter, render_framed_surface,
    render_plugin_detail_page, render_plugin_list_page, render_plugin_workbench_editor_overlay,
    render_text_panel,
};
