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

    pub(in crate::app) fn paste_plugin_workbench(dialog: &mut PluginWorkbenchOverlay, text: &str) {
        if let Some(editor) = dialog.editor.as_mut() {
            editor.input.insert_str(text);
            return;
        }
        if dialog.mode == PluginWorkbenchMode::List {
            dialog.query.insert_str(text);
            refresh_plugin_workbench_filter(dialog);
        }
    }
}
use super::{
    App, Frame, FramedSurfaceSpec, PluginWorkbenchMode, PluginWorkbenchOverlay, Rect, SurfaceMode,
    clean, plugin_workbench_summary, refresh_plugin_workbench_filter, render_framed_surface,
    render_plugin_detail_page, render_plugin_list_page, render_plugin_workbench_editor_overlay,
};
