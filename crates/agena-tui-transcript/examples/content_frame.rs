//! Export actual resource-renderer cells for deterministic visual inspection.
use agena_domain::{
    CommandOutputStream, ContentChunk, ContentCursor, ContentFormat, ContentId, ContentKind,
    ContentPage, ContentPayload, ContentResource, ContentState, TerminalAttributes,
    TerminalCellRun, TerminalSnapshot,
};
use agena_tui_transcript::content::{ContentView, render_content};
use ratatui::{
    Terminal,
    backend::TestBackend,
    style::{Color, Modifier},
    text::{Line, Text},
    widgets::Paragraph,
};
use std::error::Error;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn color(color: Color, default: &str) -> String {
    let palette = [
        "#45475a", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#bac2de",
        "#7f849c", "#f38ba8", "#a6e3a1", "#f9e2af", "#89b4fa", "#f5c2e7", "#94e2d5", "#cdd6f4",
    ];
    let index = match color {
        Color::Reset => return default.into(),
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Rgb(r, g, b) => return format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(index) => usize::from(index),
    };
    if index < 16 {
        return palette[index].into();
    }
    if index >= 232 {
        let gray = 8 + (index - 232) * 10;
        return format!("#{gray:02x}{gray:02x}{gray:02x}");
    }
    let cube = [0, 95, 135, 175, 215, 255];
    let index = index - 16;
    format!(
        "#{:02x}{:02x}{:02x}",
        cube[index / 36],
        cube[(index / 6) % 6],
        cube[index % 6]
    )
}
fn frame(kind: ContentKind, payloads: Vec<ContentPayload>) -> ContentView {
    let id = ContentId::new();
    let cursor = ContentCursor {
        epoch: id.0,
        sequence: payloads.len() as u64,
    };
    let resource = ContentResource {
        resource_id: id,
        owner_session_id: 1,
        part_id: 2,
        kind,
        state: ContentState::Active,
        cursor,
        committed_cursor: ContentCursor {
            sequence: 0,
            ..cursor
        },
        total_bytes: 0,
        dropped_bytes: 0,
        retained_ranges: vec![],
        capture_error: None,
    };
    let mut view = ContentView::default();
    assert!(
        view.apply(ContentPage {
            resource,
            chunks: payloads
                .into_iter()
                .enumerate()
                .map(|(index, payload)| ContentChunk {
                    cursor: ContentCursor {
                        sequence: index as u64 + 1,
                        ..cursor
                    },
                    captured_at_ms: 1000,
                    payload,
                })
                .collect(),
            next_cursor: cursor,
            has_more: false,
            gap: false,
        })
    );
    view
}

fn main() -> Result<(), Box<dyn Error>> {
    let log = frame(ContentKind::Log,vec![
        ContentPayload::Log {stream:CommandOutputStream::Stdout,text:"  Building…\n\x1b[32mPASS\x1b[0m  module 1\n\n中文与 emoji 🧪\nprogress 10%\rprogress 90%\n".into()},
        ContentPayload::Log {stream:CommandOutputStream::Stderr,text:"warning: diagnostic channel\n".into()},
    ]);
    let pty = frame(
        ContentKind::Terminal,
        vec![ContentPayload::Terminal {
            screen: TerminalSnapshot {
                rows: 6,
                cols: 60,
                cursor_row: 3,
                cursor_col: 2,
                cursor_visible: true,
                alternate_screen: false,
                bracketed_paste: false,
                application_cursor: false,
                cells: vec![
                    TerminalCellRun {
                        row: 0,
                        col: 0,
                        text: "Agena terminal · 实时输出".into(),
                        attributes: TerminalAttributes {
                            bold: true,
                            ..Default::default()
                        },
                    },
                    TerminalCellRun {
                        row: 2,
                        col: 0,
                        text: "$ build running…".into(),
                        attributes: Default::default(),
                    },
                ],
            },
        }],
    );
    let mut svg = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1040\" height=\"500\" viewBox=\"0 0 1040 500\"><rect width=\"1040\" height=\"500\" fill=\"#1e1e2e\"/><g font-family=\"monospace\" font-size=\"14\">",
    );
    for (panel, (name, view)) in [
        ("Pipe output · running", log),
        ("PTY observation · 60 × 6", pty),
    ]
    .into_iter()
    .enumerate()
    {
        let mut rendered = Vec::new();
        render_content(
            view.frame.as_deref(),
            &mut rendered,
            96,
            &agena_tui::i18n::I18n::english(),
            None,
            ContentFormat::Plain,
        );
        let mut terminal = Terminal::new(TestBackend::new(96, 12))?;
        terminal.draw(|frame| {
            frame.render_widget(
                Paragraph::new(Text::from(
                    rendered
                        .iter()
                        .map(|line| {
                            line.rich_line
                                .clone()
                                .unwrap_or_else(|| Line::raw(line.text.clone()).style(line.style))
                        })
                        .collect::<Vec<_>>(),
                )),
                frame.area(),
            );
        })?;
        let top = panel * 230 + 30;
        svg.push_str(&format!(
            "<text x=\"24\" y=\"{top}\" fill=\"#89b4fa\">{name}</text>"
        ));
        for y in 0..12 {
            for x in 0..96 {
                let cell = &terminal.backend().buffer()[(x, y)];
                let mut fg = color(cell.fg, "#cdd6f4");
                let mut bg = color(cell.bg, "#1e1e2e");
                if cell.modifier.contains(Modifier::REVERSED) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let x = 24 + usize::from(x) * 10;
                let y = top + 24 + usize::from(y) * 17;
                if bg != "#1e1e2e" {
                    svg.push_str(&format!(
                        "<rect x=\"{x}\" y=\"{}\" width=\"10\" height=\"17\" fill=\"{bg}\"/>",
                        y - 14
                    ));
                }
                if cell.symbol().trim().is_empty() {
                    continue;
                }
                let weight = if cell.modifier.contains(Modifier::BOLD) {
                    "bold"
                } else {
                    "normal"
                };
                let decoration = if cell.modifier.contains(Modifier::UNDERLINED) {
                    "underline"
                } else {
                    "none"
                };
                svg.push_str(&format!("<text x=\"{x}\" y=\"{y}\" fill=\"{fg}\" font-weight=\"{weight}\" text-decoration=\"{decoration}\">{}</text>",escape(cell.symbol())));
            }
        }
    }
    svg.push_str("</g></svg>\n");
    print!("{svg}");
    Ok(())
}
