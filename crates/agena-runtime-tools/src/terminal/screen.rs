use agena_domain::{TerminalAttributes, TerminalCellRun, TerminalColor, TerminalSnapshot};

fn color(value: vt100::Color) -> Option<TerminalColor> {
    match value {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(TerminalColor::Indexed { index }),
        vt100::Color::Rgb(red, green, blue) => Some(TerminalColor::Rgb { red, green, blue }),
    }
}

pub(super) fn snapshot(screen: &vt100::Screen) -> TerminalSnapshot {
    let (rows, cols) = screen.size();
    let (cursor_row, cursor_col) = screen.cursor_position();
    let mut cells = Vec::<TerminalCellRun>::new();
    for row in 0..rows {
        let end = (0..cols)
            .rev()
            .find(|col| {
                screen.cell(row, *col).is_some_and(|cell| {
                    cell.has_contents() || cell.bgcolor() != vt100::Color::Default
                })
            })
            .map_or(0, |col| col + 1);
        for col in 0..end {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let attributes = TerminalAttributes {
                foreground: color(cell.fgcolor()),
                background: color(cell.bgcolor()),
                bold: cell.bold(),
                dim: cell.dim(),
                italic: cell.italic(),
                underline: cell.underline(),
                inverse: cell.inverse(),
            };
            let text = if cell.has_contents() {
                cell.contents()
            } else {
                " "
            };
            if let Some(last) = cells.last_mut()
                && last.row == row
                && last.attributes == attributes
            {
                last.text.push_str(text);
            } else {
                cells.push(TerminalCellRun {
                    row,
                    col,
                    text: text.into(),
                    attributes,
                });
            }
        }
    }
    TerminalSnapshot {
        rows,
        cols,
        cursor_row,
        cursor_col,
        cursor_visible: !screen.hide_cursor(),
        alternate_screen: screen.alternate_screen(),
        bracketed_paste: screen.bracketed_paste(),
        application_cursor: screen.application_cursor(),
        cells,
    }
}
