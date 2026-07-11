impl App {
    pub(in crate::app) fn composer_height(&self, width: u16, total_height: u16) -> u16 {
        // The composer surface spends two columns on its border and another
        // two on its inner text inset. Keep this calculation aligned with
        // `render_composer` so wrapped text gets enough vertical room.
        let editor_width = width.saturating_sub(4).max(1);
        let line_count = max(1, self.composer.wrapped_line_count(editor_width));
        let item_rows = u16::from(self.has_composer_item_summary_row());
        let status_rows = u16::from(!self.composer_status_parts().is_empty());
        let popup_rows = self.composer_popup_rows();
        let chrome_rows = 2_u16 + item_rows + popup_rows + status_rows;
        let minimum_height = chrome_rows.saturating_add(1);
        let available_height = total_height.saturating_sub(4).max(minimum_height);
        min(12, available_height).min(
            u16::try_from(line_count)
                .unwrap_or(u16::MAX)
                .saturating_add(chrome_rows)
                .max(minimum_height),
        )
    }
}
use super::{App, max, min};
