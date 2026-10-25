//! Composite terminal selection colors into native graphics. Text modifiers
//! must never be attached to the control sequence that transports an image.

use ratatui::{buffer::Buffer, style::Color};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct GraphicHighlight {
    cells: Rect,
    foreground: Option<[u8; 3]>,
    background: [u8; 3],
}

pub(super) fn take_graphic_highlights(
    buffer: &mut Buffer,
    area: Rect,
    scroll: usize,
    placement: &TranscriptMathPlacement,
) -> Vec<GraphicHighlight> {
    let mut highlights: Vec<GraphicHighlight> = Vec::new();
    let visible_width = placement
        .size
        .width
        .min(area.width.saturating_sub(placement.column));
    for row in 0..placement.size.height {
        let line = placement.line.saturating_add(usize::from(row));
        if line < scroll {
            continue;
        }
        let screen_row = line - scroll;
        if screen_row >= usize::from(area.height) {
            break;
        }
        for column in 0..visible_width {
            let position = (
                area.x + placement.column + column,
                area.y + screen_row as u16,
            );
            let Some(cell) = buffer.cell_mut(position) else {
                continue;
            };
            let background = color_rgb(cell.bg);
            let foreground = placement
                .artifact
                .foreground
                .and_then(|_| color_rgb(cell.fg));
            // Clear both the anchor and every skipped image cell. Otherwise a
            // style change below the anchor can overwrite terminal image cells
            // or leave stale underlines after the selection has moved away.
            cell.reset();
            let Some(background) = background else {
                continue;
            };
            if let Some(last) = highlights.last_mut()
                && last.cells.y == row
                && last.cells.right() == column
                && last.foreground == foreground
                && last.background == background
            {
                last.cells.width += 1;
            } else {
                highlights.push(GraphicHighlight {
                    cells: Rect::new(column, row, 1, 1),
                    foreground,
                    background,
                });
            }
        }
    }
    highlights
}

pub(super) fn highlighted_graphic(
    artifact: &MathArtifact,
    size: Size,
    font: FontSize,
    highlights: &[GraphicHighlight],
) -> Option<DynamicImage> {
    if highlights.is_empty() {
        return Some(artifact.image.clone());
    }
    let width = u32::from(size.width).checked_mul(u32::from(font.width))?;
    let height = u32::from(size.height).checked_mul(u32::from(font.height))?;
    if width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return None;
    }
    let mut image = Resize::Scale(None)
        .resize(&artifact.image, font, size, Some(Rgba([0, 0, 0, 0])))
        .to_rgba8();
    for highlight in highlights {
        let left = u32::from(highlight.cells.x) * u32::from(font.width);
        let right = u32::from(highlight.cells.right()) * u32::from(font.width);
        let top = u32::from(highlight.cells.y) * u32::from(font.height);
        let bottom = u32::from(highlight.cells.bottom()) * u32::from(font.height);
        for y in top..bottom.min(height) {
            for x in left..right.min(width) {
                let pixel = image.get_pixel_mut(x, y);
                if artifact.foreground.is_some_and(|ink| pixel.0[..3] == ink)
                    && let Some(foreground) = highlight.foreground
                {
                    pixel.0[..3].copy_from_slice(&foreground);
                }
                let mut alpha = u16::from(pixel.0[3]);
                if artifact.foreground.is_none() {
                    // An opaque photograph also needs visible selection
                    // feedback. Keep 75% of its original coverage and tint
                    // with the same selection color used by surrounding text.
                    alpha = alpha * 192 / 255;
                }
                for (channel, background) in pixel.0[..3].iter_mut().zip(highlight.background) {
                    *channel = ((u16::from(*channel) * alpha
                        + u16::from(background) * (255 - alpha)
                        + 127)
                        / 255) as u8;
                }
                pixel.0[3] = 255;
            }
        }
    }
    Some(DynamicImage::ImageRgba8(image))
}

fn color_rgb(color: Color) -> Option<[u8; 3]> {
    let index = match color {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => return Some([r, g, b]),
        Color::Indexed(index) => index,
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
    };
    const BASIC: [[u8; 3]; 16] = [
        [0, 0, 0],
        [128, 0, 0],
        [0, 128, 0],
        [128, 128, 0],
        [0, 0, 128],
        [128, 0, 128],
        [0, 128, 128],
        [192, 192, 192],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [0, 0, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    Some(match index {
        0..=15 => BASIC[usize::from(index)],
        16..=231 => {
            let cube = index - 16;
            let level = |n| if n == 0 { 0 } else { 55 + n * 40 };
            [level(cube / 36), level(cube / 6 % 6), level(cube % 6)]
        }
        _ => [8 + (index - 232) * 10; 3],
    })
}
