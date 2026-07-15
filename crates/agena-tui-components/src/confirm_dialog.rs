use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
};

use crate::text_dialog::{line_text_dialog_section_heights, render_modal_line_text_dialog};
use crate::{
    ConfirmDialogState, LineTextDialogSpec, SurfaceMode, TextDialogLine, framed_overlay_height,
    search_picker_dialog_area,
};

const BODY_HEIGHT_BOUNDS: (u16, u16) = (2, 10);
const FOOTER_HEIGHT_BOUNDS: (u16, u16) = (1, 1);

/// Renders the shared secondary-confirmation surface.
///
/// Confirmation dialogs use the same canonical width as search pickers, while
/// their height follows the amount of confirmation copy they contain.
pub fn render_confirm_dialog<TAction, F>(
    frame: &mut Frame,
    area: Rect,
    dialog: &ConfirmDialogState<TAction>,
    normalize_text: F,
) where
    F: Fn(&str) -> String,
{
    let lines = dialog
        .body_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let text = normalize_text(line);
            if index == 0 {
                TextDialogLine::styled(text, Style::default().add_modifier(Modifier::BOLD))
            } else {
                TextDialogLine::plain(text)
            }
        })
        .collect::<Vec<_>>();
    let picker_area = search_picker_dialog_area(area);
    let spec = LineTextDialogSpec::new(
        normalize_text(&dialog.title).into(),
        lines.as_slice(),
        Some(normalize_text(&dialog.footer).into()),
        picker_area.width,
        true,
        None,
        None,
        BODY_HEIGHT_BOUNDS,
        FOOTER_HEIGHT_BOUNDS,
        Some(Alignment::Right),
        Style::default(),
    );
    let content_width = picker_area.width.saturating_sub(2);
    let (body_height, footer_height) = line_text_dialog_section_heights(&spec, content_width);
    let target_height = framed_overlay_height(body_height.saturating_add(footer_height));
    let dialog_area = confirm_dialog_area(area, target_height);

    render_modal_line_text_dialog(frame, dialog_area, SurfaceMode::Route, &spec);
}

/// Returns a centered confirmation window with the canonical search-picker
/// width and a content-driven, typically much shorter height.
pub fn confirm_dialog_area(area: Rect, target_height: u16) -> Rect {
    let picker_area = search_picker_dialog_area(area);
    if picker_area.height == 0 {
        return picker_area;
    }

    let height = target_height.clamp(1, picker_area.height);
    Rect {
        y: picker_area
            .y
            .saturating_add(picker_area.height.saturating_sub(height) / 2),
        height,
        ..picker_area
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn confirmation_window_matches_search_picker_width_with_a_shorter_height() {
        let terminal_area = Rect::new(0, 0, 120, 30);
        let picker_area = search_picker_dialog_area(terminal_area);
        let confirm_area = confirm_dialog_area(terminal_area, 7);

        assert_eq!(confirm_area.x, picker_area.x);
        assert_eq!(confirm_area.width, picker_area.width);
        assert_eq!(confirm_area.height, 7);
        assert_eq!(confirm_area.y, 11);
        assert!(confirm_area.height < picker_area.height);
    }

    #[test]
    fn tiny_terminals_keep_the_shared_full_width() {
        let terminal_area = Rect::new(0, 0, 40, 12);

        assert_eq!(
            confirm_dialog_area(terminal_area, 5),
            Rect::new(0, 3, 40, 5)
        );
    }

    #[test]
    fn shared_renderer_uses_the_canonical_geometry() {
        let dialog = ConfirmDialogState::new(
            "Delete provider".to_owned(),
            vec!["Delete provider openai?".to_owned()],
            "Y confirm · N/Esc cancel".to_owned(),
            (),
        );
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| {
                render_confirm_dialog(frame, frame.area(), &dialog, str::to_owned);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Delete provider"));
        assert!(rendered.contains("Delete provider openai?"));
        assert_eq!(buffer[(4, 12)].symbol(), "╔");
        assert_eq!(buffer[(115, 12)].symbol(), "╗");
        assert_eq!(buffer[(4, 16)].symbol(), "╚");
        assert_eq!(buffer[(115, 16)].symbol(), "╝");
        let palette = crate::theme::active_palette();
        assert_eq!(buffer[(5, 13)].bg, palette.modal_bg);
        assert_eq!(buffer[(4, 12)].fg, palette.modal_border);
    }
}
