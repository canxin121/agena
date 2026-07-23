impl App {
    pub(in crate::app) fn render_plugin_workbench(
        &self,
        frame: &mut Frame,
        area: Rect,
        dialog: &PluginWorkbenchOverlay,
        surface: SurfaceMode,
    ) {
        let surface = agena_tui_components::render_workbench_frame(
            frame,
            area,
            surface,
            &agena_tui_components::WorkbenchFrameSpec::new(
                clean(format!(
                    "{} · {}",
                    dialog.title,
                    plugin_workbench_summary(dialog)
                ))
                .into(),
                "".into(),
                150,
                40,
            ),
        );
        match dialog.navigation.mode {
            PluginWorkbenchMode::List => render_plugin_list_page(frame, surface.body, dialog),
            PluginWorkbenchMode::Detail => render_plugin_detail_page(frame, surface.body, dialog),
        }
        render_plugin_workbench_editor_overlay(frame, area, surface.outer, dialog);
    }

    pub(in crate::app) fn paste_plugin_workbench(dialog: &mut PluginWorkbenchOverlay, text: &str) {
        if let Some(editor) = dialog.editor.as_mut() {
            editor.input.insert_str(text);
            return;
        }
        if dialog.navigation.mode == PluginWorkbenchMode::List {
            dialog.list.append_query_text(text);
        }
    }
}
use super::{
    App, Frame, PluginWorkbenchMode, PluginWorkbenchOverlay, Rect, SurfaceMode, clean,
    plugin_workbench_summary, render_plugin_detail_page, render_plugin_list_page,
    render_plugin_workbench_editor_overlay,
};
