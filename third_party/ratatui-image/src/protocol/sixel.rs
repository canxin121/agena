//! Sixel protocol implementations.
//! Uses [`icy_sixel`] to draw image pixels, if the terminal [supports] the [Sixel] protocol.
//!
//! Delivers the image on each render as [Sixel]s.
//!
//! [`icy_sixel`]: https://github.com/mkrueger/icy_sixel
//! [supports]: https://arewesixelyet.com
//! [Sixel]: https://en.wikipedia.org/wiki/Sixel
use icy_sixel::{EncodeOptions, sixel_encode};
use image::DynamicImage;
use ratatui::{
    buffer::{Buffer, CellDiffOption},
    layout::{Position, Rect, Size},
};

use super::{ProtocolTrait, StatefulProtocolTrait, clear_area};
use crate::{Result, errors::Errors, picker::cap_parser::Parser, protocol::UNIT_WIDTH};

#[derive(Clone, Default)]
pub struct Sixel {
    pub data: String,
    pub size: Size,
    pub is_tmux: bool,
}

impl Sixel {
    pub fn new(image: DynamicImage, size: Size, is_tmux: bool) -> Result<Self> {
        let data = encode(&image, size, is_tmux)?;
        Ok(Self {
            data,
            size,
            is_tmux,
        })
    }
}

// TODO: change E to sixel_rs::status::Error and map when calling
fn encode(img: &DynamicImage, size: Size, is_tmux: bool) -> Result<String> {
    let (w, h) = (img.width(), img.height());
    let img_rgba8 = img.to_rgba8();
    let bytes = img_rgba8.as_raw();
    let (start, escape, end) = Parser::tmux_start_escape_end(is_tmux);

    let width = size.width;
    let height = size.height;

    let sixel_data = sixel_encode(bytes, w as usize, h as usize, &EncodeOptions::default())
        .map_err(|err| Errors::Sixel(format!("sixel encoding error: {err}")))?;

    let mut data = String::new();
    if is_tmux {
        if !sixel_data.starts_with('\x1b') {
            return Err(Errors::Tmux("sixel string did not start with escape"));
        }
        // The clear sequence and the complete Sixel DCS must be inside the
        // tmux passthrough. Every ESC in the nested protocol, including its
        // terminating ST, has to be doubled for tmux.
        data.push_str(start);
        clear_area(&mut data, escape, width, height);
        for character in sixel_data.chars() {
            if character == '\x1b' {
                data.push_str(escape);
            } else {
                data.push(character);
            }
        }
        data.push_str(end);
    } else {
        clear_area(&mut data, escape, width, height);
        data.push_str(&sixel_data);
    }

    Ok(data)
}

impl ProtocolTrait for Sixel {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if self.size.width > area.width || self.size.height > area.height {
            return;
        }
        let render_area = Rect::new(area.x, area.y, self.size.width, self.size.height);

        render(&self.data, render_area, buf)
    }

    fn size(&self) -> Size {
        self.size
    }
}

pub(crate) fn render(data: &str, area: Rect, buf: &mut Buffer) {
    buf.cell_mut(Into::<Position>::into(area))
        .map(|cell| cell.set_symbol(data).set_diff_option(UNIT_WIDTH));

    let mut skip_first = false;

    // Skip entire area
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if !skip_first {
                skip_first = true;
                continue;
            }
            buf.cell_mut((x, y))
                .map(|cell| cell.set_diff_option(CellDiffOption::Skip));
        }
    }
}

impl StatefulProtocolTrait for Sixel {
    fn resize_encode(&mut self, img: DynamicImage, size: Size) -> Result<()> {
        let data = encode(&img, size, self.is_tmux)?;
        *self = Sixel {
            data,
            size,
            ..*self
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_passthrough_escapes_the_complete_nested_sixel_dcs() {
        let image = DynamicImage::new_rgba8(2, 2);
        let encoded = encode(&image, Size::new(2, 1), true).expect("Sixel should encode");

        assert!(encoded.starts_with("\x1bPtmux;"));
        assert!(encoded.contains("\x1b\x1bP"));
        assert!(encoded.ends_with("\x1b\x1b\\\x1b\\"));
    }
}
