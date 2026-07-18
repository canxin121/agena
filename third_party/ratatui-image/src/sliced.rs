//! Sliced image widget and protocol wrapper.
use std::sync::Mutex;

use crate::{
    FontSize, Resize,
    errors::Errors,
    picker::{Picker, ProtocolType},
    protocol::{Protocol, ProtocolTrait, halfblocks::Halfblocks, kitty::Kitty, sixel::Sixel},
    sliced::sixel_slice::SlicedSixel,
};
use image::DynamicImage;
use ratatui::{
    layout::{Rect, Size},
    widgets::Widget,
};

/// An image "sliced" into rows for partially displaying, for example in vertical scrolling.
///
/// Uses a specialized [`SlicedProtocol`] with specialized operations based on the protocol.
pub struct SlicedImage<'a> {
    sliced_protocol: &'a SlicedProtocol,
    position: SignedPosition,
}

impl<'a> SlicedImage<'a> {
    /// Create a sliced image that will render with the given size at the given position.
    ///
    /// The position is relative to the `area` parameter of [`SlicedImage::render`], which is
    /// either a direct argument or stems from `frame.render_widget(w, area)`.
    ///
    /// Example that renders an image as if starting at 3 lines *above* the terminal viewport:
    ///
    /// ```rust
    /// # use ratatui_image::picker::Picker;
    /// # use ratatui::layout::Size;
    /// # use ratatui_image::sliced::{SignedPosition, SlicedProtocol, SlicedImage};
    /// # let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24))?;
    /// # let picker = Picker::halfblocks();
    /// let dyn_img = image::DynamicImage::new_rgb8(320, 180);
    ///
    /// // This example would render the image at its actual pixel size.
    /// let sliced = SlicedProtocol::new(&picker, dyn_img, None)?;
    ///
    /// terminal.draw(|f| {
    ///     let position = SignedPosition::from((0, -3));
    ///     f.render_widget(SlicedImage::new(&sliced, position), f.area());
    /// });
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// The same works for e.g. ending N lines below viewport, or within any other inner area of
    /// the TUI.
    pub fn new(sliced_protocol: &'a SlicedProtocol, position: SignedPosition) -> SlicedImage<'a> {
        SlicedImage {
            sliced_protocol,
            position,
        }
    }

    fn skip_and_drop(size: Size, position: SignedPosition, area: Rect) -> Option<(usize, usize)> {
        if area.height == 0 || area.width == 0 {
            return None;
        }
        let top = i32::from(position.y);
        let bottom = top + i32::from(size.height);
        let area_top = 0;
        let area_bottom = i32::from(area.height);

        if top >= area_bottom || bottom <= area_top {
            return None;
        }

        let skip = (area_top - top).max(0) as usize;
        let drop = (bottom - area_bottom).max(0) as usize;

        Some((skip, drop))
    }
}

impl Widget for SlicedImage<'_> {
    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let size = self.sliced_protocol.size();
        let Some((skip_line_count, drop_line_count)) =
            Self::skip_and_drop(size, self.position, area)
        else {
            return;
        };

        let x = self.position.x.max(0) as u16;
        let y = self.position.y.max(0) as u16;
        if x >= area.width {
            return;
        }
        let hidden_rows =
            u16::try_from(skip_line_count.saturating_add(drop_line_count)).unwrap_or(u16::MAX);
        let image_area = Rect::new(
            area.x.saturating_add(x),
            area.y.saturating_add(y.min(area.height)),
            size.width.min(area.width.saturating_sub(x)),
            size.height.saturating_sub(hidden_rows),
        );

        match &self.sliced_protocol {
            SlicedProtocol::Kitty(kitty) => {
                kitty.render_with_skip(image_area, buf, skip_line_count);
            }
            SlicedProtocol::Iterm2(iterm2) => iterm2.render(
                image_area,
                buf,
                skip_line_count,
                drop_line_count,
            ),
            SlicedProtocol::Sixel(sliced_sixel) => {
                let sliced = sliced_sixel.borrow_dependent();
                sliced.render(image_area, buf, skip_line_count, drop_line_count);
            }
            SlicedProtocol::Halfblocks(halfblocks) => {
                halfblocks.render_with_skip(image_area, buf, skip_line_count, drop_line_count);
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SignedPosition {
    pub x: i16,
    pub y: i16,
}

impl From<(i16, i16)> for SignedPosition {
    fn from((x, y): (i16, i16)) -> Self {
        Self { x, y }
    }
}

/// The sliced image for [`SlicedImage`].
///
/// Contains the sliced data specialized for the protocol.
pub enum SlicedProtocol {
    /// iTerm2 has no viewport-cropping command. Keep one full protocol for the
    /// normal case and lazily encode one contiguous cropped image for a
    /// viewport boundary. A displayed image is therefore never assembled from
    /// independent terminal rows that expose custom line spacing as seams.
    Iterm2(SlicedIterm2),
    /// Takes full advantage of the unicode-placeholder mechanism.
    Kitty(Kitty),
    /// Strips sixel "bands" at render time to display only relevant parts, since the sixel format
    /// already is row based. Not pixel accurate, but good enough. Stores font-height to match
    /// against sixel "bands" height.
    ///
    /// TODO: deconstruct at encode-time instead of render-time.
    Sixel(SlicedSixel),
    /// Renders the full image (with chafa if available) for best ASCII art results, then just
    /// renders the relevant rows.
    Halfblocks(Halfblocks),
}

impl SlicedProtocol {
    /// Create a `SlicedProtocol` for the target [`ratatui::layout::Size`].
    ///
    /// If `size` is omitted, it will be calculated based on `dyn_img`'s image-pixel-size and
    /// `picker.font_size()`.
    pub fn new(
        picker: &Picker,
        dyn_img: DynamicImage,
        size: Option<Size>,
    ) -> Result<SlicedProtocol, Errors> {
        let size = size.unwrap_or_else(|| Resize::natural_size(&dyn_img, picker.font_size()));
        SlicedProtocol::new_with_resize(picker, dyn_img, size, Resize::Fit(None))
    }

    /// Create a `SlicedProtocol` for the target [`ratatui::layout::Size`] with the given
    /// [`Resize`] option.
    pub fn new_with_resize(
        picker: &Picker,
        dyn_img: DynamicImage,
        size: Size,
        resize: Resize,
    ) -> Result<SlicedProtocol, Errors> {
        match picker.protocol_type() {
            ProtocolType::Kitty => {
                let Protocol::Kitty(kitty) = picker.new_protocol(dyn_img, size, resize)? else {
                    unreachable!("ProtocolType::Kitty must produce Protocol::Kitty");
                };
                Ok(SlicedProtocol::Kitty(kitty))
            }
            ProtocolType::Sixel => {
                let font_size = picker.font_size();

                let dyn_img = resize.resize(&dyn_img, font_size, size, None);

                let sixel = Sixel::new(dyn_img, size, picker.is_tmux)?;

                let sliced = SlicedSixel::from_sixel(sixel, font_size.height, picker.is_tmux);

                Ok(SlicedProtocol::Sixel(sliced))
            }
            ProtocolType::Halfblocks => {
                let Protocol::Halfblocks(halfblocks) =
                    picker.new_protocol(dyn_img, size, resize)?
                else {
                    unreachable!("ProtocolType::Halfblocks must produce Protocol::Halfblocks");
                };
                Ok(SlicedProtocol::Halfblocks(halfblocks))
            }
            _ => {
                Ok(SlicedProtocol::Iterm2(SlicedIterm2::new(
                    picker,
                    dyn_img,
                    size,
                )?))
            }
        }
    }

    pub fn size(&self) -> Size {
        match self {
            SlicedProtocol::Iterm2(iterm2) => iterm2.size,
            SlicedProtocol::Halfblocks(hb) => hb.size(),
            SlicedProtocol::Kitty(kitty) => kitty.size(),
            SlicedProtocol::Sixel(sixel_slice) => sixel_slice.borrow_owner().size(),
        }
    }
}

pub struct SlicedIterm2 {
    image: DynamicImage,
    picker: Picker,
    full: Protocol,
    size: Size,
    clipped: Mutex<Option<ClippedIterm2>>,
}

struct ClippedIterm2 {
    key: (usize, usize),
    protocol: Protocol,
}

impl SlicedIterm2 {
    fn new(picker: &Picker, image: DynamicImage, size: Size) -> Result<Self, Errors> {
        let image = iterm2_cell_image(image, picker.font_size(), size);
        let full = picker.new_protocol_raw(image.clone(), size)?;
        Ok(Self {
            image,
            picker: picker.clone(),
            full,
            size,
            clipped: Mutex::new(None),
        })
    }

    fn render(
        &self,
        area: Rect,
        buf: &mut ratatui::prelude::Buffer,
        skip_line_count: usize,
        drop_line_count: usize,
    ) {
        if skip_line_count == 0 && drop_line_count == 0 {
            self.full.render(area, buf);
            return;
        }
        let hidden = skip_line_count.saturating_add(drop_line_count);
        let visible_rows = usize::from(self.size.height).saturating_sub(hidden);
        let Ok(visible_height) = u16::try_from(visible_rows) else {
            return;
        };
        if visible_height == 0 {
            return;
        }

        let key = (skip_line_count, drop_line_count);
        let mut clipped = self
            .clipped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if clipped.as_ref().is_none_or(|cached| cached.key != key) {
            let font_height = u32::from(self.picker.font_size().height);
            let Ok(skip_rows) = u32::try_from(skip_line_count) else {
                return;
            };
            let y = skip_rows.saturating_mul(font_height);
            let pixel_height = u32::from(visible_height).saturating_mul(font_height);
            if y.saturating_add(pixel_height) > self.image.height() {
                return;
            }
            let image = self.image.crop_imm(0, y, self.image.width(), pixel_height);
            let size = Size::new(self.size.width, visible_height);
            let Ok(protocol) = self.picker.new_protocol_raw(image, size) else {
                return;
            };
            *clipped = Some(ClippedIterm2 { key, protocol });
        }
        if let Some(cached) = clipped.as_ref() {
            cached.protocol.render(area, buf);
        }
    }
}

/// Normalize the source to an exact iTerm2 cell rectangle once. Full and
/// viewport-cropped variants are encoded from this same continuous raster.
fn iterm2_cell_image(image: DynamicImage, font_size: FontSize, size: Size) -> DynamicImage {
    let width = u32::from(size.width) * u32::from(font_size.width);
    let height = u32::from(size.height) * u32::from(font_size.height);
    let resized = image.resize(width, height, image::imageops::FilterType::Nearest);
    let mut image = DynamicImage::new_rgba8(width, height);
    image::imageops::overlay(&mut image, &resized, 0, 0);
    image
}

/// Sixel "slicing" functions
///
/// Generated with an LLM, seems to work, it's just an implementation detail.
/// Sixel data consists of some start and end data, and in between are "bands" of sixels, which are
/// six pixel columns of data. Therefore it's easy to remove some sixel bands anywhere in the
/// image, for vertical clipping.
mod sixel_slice {
    use ratatui::layout::Size;
    use self_cell::self_cell;

    use crate::{
        picker::cap_parser::Parser,
        protocol::{
            clear_area,
            sixel::{self, Sixel},
        },
    };

    self_cell!(
        pub struct SlicedSixel {
            owner: Sixel,
            #[covariant]
            dependent: SlicedSixelData,
        }
    );

    pub struct SlicedSixelData<'a> {
        size: Size,
        font_height: u16,
        is_tmux: bool,
        header: &'a str,
        bands: Vec<&'a str>,
    }
    impl<'a> SlicedSixelData<'a> {
        pub fn render(
            &self,
            area: ratatui::prelude::Rect,
            buf: &mut ratatui::prelude::Buffer,
            skip_line_count: usize,
            drop_line_count: usize,
        ) {
            if self.size.width > area.width {
                return;
            }

            let data = self.to_sequence(skip_line_count, drop_line_count, area.width, area.height);
            sixel::render(&data, area, buf);
        }

        fn bands(&self, skip_line_count: usize, drop_line_count: usize) -> Vec<&str> {
            let font_height = usize::from(self.font_height);
            let skip_bands = skip_line_count.saturating_mul(font_height).div_ceil(6);
            let visible_end_rows = usize::from(self.size.height).saturating_sub(drop_line_count);
            let visible_end_band = if drop_line_count == 0 {
                // Preserve the encoder's partial final band when the image is not clipped at
                // the bottom. Dropping it would remove up to five legitimate pixel rows.
                self.bands.len()
            } else {
                // A cropped partial band can contain pixels belonging to the hidden terminal
                // row, so stop at the last complete band before the viewport boundary.
                visible_end_rows.saturating_mul(font_height) / 6
            };
            let take_bands = visible_end_band.saturating_sub(skip_bands);

            let sliced_bands: Vec<&str> = self
                .bands
                .iter()
                .skip(skip_bands)
                .take(take_bands)
                .copied()
                .collect();

            let trimmed = &sliced_bands[..sliced_bands
                .iter()
                .rposition(|s| !s.is_empty())
                .map(|i| i + 1)
                .unwrap_or(0)];

            trimmed.into()
        }

        pub fn to_sequence(
            &self,
            skip_line_count: usize,
            drop_line_count: usize,
            width: u16,
            height: u16,
        ) -> String {
            let (start, escape, end) = Parser::tmux_start_escape_end(self.is_tmux);

            let mut data = String::from(start);
            clear_area(&mut data, escape, width, height);
            data.push_str(self.header);

            let sliced_bands = self.bands(skip_line_count, drop_line_count);

            data.push_str(&sliced_bands.join("-"));

            if !sliced_bands.is_empty() {
                data.push('-');
            }
            data.push_str(escape);
            data.push('\\');
            data.push_str(end);

            data
        }
    }

    impl SlicedSixel {
        pub fn from_sixel(sixel: Sixel, font_height: u16, is_tmux: bool) -> SlicedSixel {
            SlicedSixel::new(sixel, |s| {
                let size = s.size;
                let dcs_start = if is_tmux {
                    s.data.find("\u{1b}\u{1b}P")
                } else {
                    s.data.find("\u{1b}P")
                }
                .unwrap_or(0);
                let data = &s.data[dcs_start..];
                let header_end = find_sixel_data_start(data);
                let (header, body) = data.split_at(header_end);
                let mut bands: Vec<&str> = body.split('-').collect();
                bands.pop();
                SlicedSixelData {
                    size,
                    font_height,
                    is_tmux,
                    header,
                    bands,
                }
            })
        }
    }

    fn find_sixel_data_start(data: &str) -> usize {
        let bytes = data.as_bytes();
        let mut i = 0;

        // Step 1: find ESC P
        while i + 1 < bytes.len() {
            if bytes[i] == 0x1B && bytes[i + 1] == b'P' {
                break;
            }
            i += 1;
        }

        // Step 2: skip past `q`
        while i < bytes.len() && bytes[i] != b'q' {
            i += 1;
        }
        if i < bytes.len() {
            i += 1;
        }

        // Step 3: skip raster attrs and color *definitions* only
        while i < bytes.len() {
            match bytes[i] {
                b'"' => {
                    // raster attribute line, skip to next `#` or sixel data char
                    i += 1;
                    while i < bytes.len()
                        && bytes[i] != b'#'
                        && bytes[i] != b'-'
                        && !(63..=126).contains(&bytes[i])
                    {
                        i += 1;
                    }
                }
                b'-' => break,
                b'#' => {
                    // peek ahead: is this `#digits;` (color def) or `#digits` followed by data?
                    let start = i;
                    i += 1;
                    // skip digits
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    if i < bytes.len() && bytes[i] == b';' {
                        // it's a color definition — skip the rest of it
                        while i < bytes.len()
                            && bytes[i] != b'#'
                            && bytes[i] != b'-'
                            && !(63..=126).contains(&bytes[i])
                        {
                            i += 1;
                        }
                    } else {
                        // it's a color selector in band data — rewind to the `#`, we're done
                        i = start;
                        break;
                    }
                }
                63..=126 => break, // sixel data character
                _ => i += 1,
            }
        }

        i
    }

    #[cfg(test)]
    mod tests {
        use image::{DynamicImage, Rgb, RgbImage};
        use ratatui::layout::Size;

        use crate::{FontSize, Resize, protocol::sixel::Sixel, sliced::sixel_slice::SlicedSixel};

        fn fixture_image(width: u32, height: u32) -> DynamicImage {
            DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
                Rgb([
                    x.wrapping_mul(17) as u8,
                    y.wrapping_mul(29) as u8,
                    x.wrapping_add(y).wrapping_mul(11) as u8,
                ])
            }))
        }

        #[test]
        fn test_sixel_slice_bands() {
            // TODO: is there always a `-` before `<esc>\`?
            let data = String::from("\x1b[6X\x1bPq\"1;1;8;16#0band1-band2-band3-\x1b\\");
            let sixel = Sixel {
                data,
                size: Size::default(),
                is_tmux: false,
            };
            let sliced = SlicedSixel::from_sixel(sixel, 6, false);
            let sliced = sliced.borrow_dependent();
            // band1 should be skipped, band2 should be present
            assert_eq!(sliced.bands, vec!["#0band1", "band2", "band3"]);
        }

        #[test]
        fn test_sixel_slice_drops_bands_above_and_below_the_viewport() {
            let data = String::from("\x1bPq#0a-b-c-d-e-f-g-h-i-j-\x1b\\");
            let sixel = Sixel {
                data,
                size: Size::new(1, 10),
                is_tmux: false,
            };
            let sliced = SlicedSixel::from_sixel(sixel, 6, false);
            let sliced = sliced.borrow_dependent();

            assert_eq!(sliced.bands(2, 3), vec!["c", "d", "e", "f", "g"]);
        }

        #[test]
        fn test_tmux_sixel_slice_keeps_one_outer_wrapper_and_escaped_inner_st() {
            let sixel = Sixel::new(image::DynamicImage::new_rgba8(2, 12), Size::new(2, 2), true)
                .expect("Sixel should encode");
            let sliced = SlicedSixel::from_sixel(sixel, 6, true);
            let encoded = sliced.borrow_dependent().to_sequence(0, 0, 2, 2);

            assert_eq!(encoded.matches("\x1bPtmux;").count(), 1);
            assert!(encoded.contains("\x1b\x1bP"));
            assert!(encoded.ends_with("\x1b\x1b\\\x1b\\"));
        }

        #[test]
        fn test_idempotence() {
            let images = [
                ("wide", fixture_image(31, 17)),
                ("tall", fixture_image(13, 29)),
                ("square", fixture_image(19, 19)),
            ];
            let size = Size::new(10, 10);
            let font_size = FontSize::new(8, 16);
            let sliced_sixels = images.map(|(name, dyn_img)| {
                let dyn_img = Resize::Fit(None).resize(&dyn_img, font_size, size, None);
                let sixel = Sixel::new(dyn_img, size, false).unwrap();
                (
                    name,
                    SlicedSixel::from_sixel(sixel, font_size.height, false),
                )
            });
            for (name, sliced_sixel) in sliced_sixels {
                let mut source = sliced_sixel.borrow_owner().data.as_str();
                source = source.strip_suffix("\x1b\\").unwrap();
                source = source.trim_end_matches('-');
                let source = format!("{source}-\x1b\\");

                let sliced_sixel_data = sliced_sixel.borrow_dependent();
                let sliced = sliced_sixel_data.to_sequence(0, 0, size.height, size.width);
                if sliced != source {
                    let mut surrounding = String::new();
                    for (i, char) in source.chars().enumerate() {
                        if surrounding.len() > 20 {
                            surrounding = surrounding.split_off(19);
                        }
                        surrounding.push(char);

                        let Some(sliced_char) = sliced.chars().nth(i) else {
                            panic!("sliced is shorter after {i}");
                        };
                        assert_eq!(
                            char,
                            sliced_char,
                            "{name} index #{i} (surrounding: \"{}\")",
                            surrounding.replace('\x1b', "<esc>")
                        );
                    }
                    panic!("should have found the first different char");
                }
            }
        }

        #[test]
        fn test_bands_from_cell_geometry() {
            let body = (0..38)
                .map(|index| format!("#0?{index}"))
                .collect::<Vec<_>>()
                .join("-");
            let sixel = Sixel {
                data: format!("\x1bPq{body}-\x1b\\"),
                size: Size::new(4, 12),
                is_tmux: false,
            };
            let proto = SlicedSixel::from_sixel(sixel, 20, false);
            let sliced = proto.borrow_dependent();

            // 38 source bands fit within a 12-row image at 20 pixels per row.
            assert_eq!(38, sliced.bands(0, 0).len());

            // one row is 20px, so 3 bands make 18px
            assert_eq!(3, sliced.bands(0, 11).len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iterm_image_sequences(buffer: &ratatui::buffer::Buffer) -> Vec<&str> {
        buffer
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .filter(|symbol| symbol.contains("]1337;File="))
            .collect()
    }

    #[test]
    fn fully_visible_iterm_image_is_encoded_once_without_row_seams() {
        let picker = Picker::from_parts(
            FontSize::new(2, 2),
            ProtocolType::Iterm2,
            false,
            Vec::new(),
        );
        let protocol = SlicedProtocol::new_with_resize(
            &picker,
            DynamicImage::new_rgba8(8, 8),
            Size::new(4, 4),
            Resize::Scale(None),
        )
        .expect("iTerm2 protocol should encode");

        let area = Rect::new(0, 0, 4, 4);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        SlicedImage::new(&protocol, SignedPosition::from((0, 0))).render(area, &mut buffer);
        let sequences = iterm_image_sequences(&buffer);
        assert_eq!(sequences.len(), 1);
        assert!(sequences[0].contains(";width=4;height=4;"));
    }

    #[test]
    fn clipped_iterm_image_is_one_contiguous_visible_raster() {
        let picker = Picker::from_parts(
            FontSize::new(2, 2),
            ProtocolType::Iterm2,
            false,
            Vec::new(),
        );
        let protocol = SlicedProtocol::new_with_resize(
            &picker,
            DynamicImage::new_rgba8(8, 8),
            Size::new(4, 4),
            Resize::Scale(None),
        )
        .expect("iTerm2 protocol should encode");

        let area = Rect::new(0, 0, 4, 2);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        SlicedImage::new(&protocol, SignedPosition::from((0, -1))).render(area, &mut buffer);
        let sequences = iterm_image_sequences(&buffer);
        assert_eq!(sequences.len(), 1);
        assert!(sequences[0].contains(";width=4;height=2;"));

        let SlicedProtocol::Iterm2(iterm2) = &protocol else {
            panic!("expected iTerm2 protocol");
        };
        assert!(iterm2.clipped.lock().expect("clipped cache").is_some());

        let mut second_buffer = ratatui::buffer::Buffer::empty(area);
        SlicedImage::new(&protocol, SignedPosition::from((0, -1)))
            .render(area, &mut second_buffer);
        assert_eq!(iterm_image_sequences(&second_buffer).len(), 1);
        assert!(
            iterm2.clipped.lock().expect("clipped cache").is_some(),
            "rendering the same viewport should retain its encoded raster"
        );
    }

    #[test]
    fn proportional_scaling_has_consistent_cell_geometry_across_protocols() {
        let font_size = FontSize::new(8, 16);
        let target = Size::new(10, 2);
        for protocol_type in [
            ProtocolType::Kitty,
            ProtocolType::Sixel,
            ProtocolType::Iterm2,
            ProtocolType::Halfblocks,
        ] {
            let picker = Picker::from_parts(font_size, protocol_type, false, Vec::new());
            let protocol = SlicedProtocol::new_with_resize(
                &picker,
                DynamicImage::new_rgba8(250, 100),
                target,
                Resize::Scale(None),
            )
            .expect("protocol should encode a proportional target");

            assert_eq!(protocol.size(), target, "{protocol_type:?}");
        }
    }

    #[test]
    fn test_skip_and_drop() {
        struct TCase {
            y: i16,
            size: u16,                    // height
            area: u16,                    // height
            want: Option<(usize, usize)>, // (skip, drop)
        }
        for TCase {
            y,
            size,
            area,
            want,
        } in [
            TCase {
                y: -1,
                size: 12,
                area: 10,
                want: Some((1, 1)),
            },
            TCase {
                y: 0,
                size: 10,
                area: 10,
                want: Some((0, 0)),
            },
            TCase {
                y: 0,
                size: 5,
                area: 10,
                want: Some((0, 0)),
            },
            TCase {
                y: 2,
                size: 5,
                area: 10,
                want: Some((0, 0)),
            },
            TCase {
                y: -1,
                size: 10,
                area: 10,
                want: Some((1, 0)),
            },
            TCase {
                y: -5,
                size: 10,
                area: 10,
                want: Some((5, 0)),
            },
            TCase {
                y: 0,
                size: 20,
                area: 10,
                want: Some((0, 10)),
            },
            TCase {
                y: 5,
                size: 10,
                area: 10,
                want: Some((0, 5)),
            },
            TCase {
                y: 9,
                size: 1,
                area: 10,
                want: Some((0, 0)),
            },
            TCase {
                y: -2,
                size: 14,
                area: 10,
                want: Some((2, 2)),
            },
            TCase {
                y: -10,
                size: 10,
                area: 10,
                want: None,
            },
            TCase {
                y: 10,
                size: 10,
                area: 10,
                want: None,
            },
            TCase {
                y: 11,
                size: 10,
                area: 10,
                want: None,
            },
            TCase {
                y: 0,
                size: 1,
                area: 10,
                want: Some((0, 0)),
            },
            TCase {
                y: -1,
                size: 1,
                area: 10,
                want: None,
            },
            TCase {
                y: 10,
                size: 1,
                area: 10,
                want: None,
            },
        ] {
            assert_eq!(
                want,
                SlicedImage::skip_and_drop(
                    (100, size).into(),
                    (0, y).into(),
                    Rect::new(0, 0, 100, area),
                ),
                "position.y:{y}, size.y:{size}, area.height:{area}",
            );
        }

        assert_eq!(
            Some((32_768, 32_757)),
            SlicedImage::skip_and_drop(
                Size::new(1, u16::MAX),
                SignedPosition::from((0, i16::MIN)),
                Rect::new(0, 0, 1, 10),
            ),
            "large images and signed viewport offsets must not overflow"
        );
    }

    #[test]
    fn image_entirely_right_of_viewport_does_not_underflow() {
        let picker = Picker::halfblocks();
        let protocol = SlicedProtocol::new(
            &picker,
            DynamicImage::new_rgba8(2, 2),
            Some(Size::new(2, 2)),
        )
        .expect("halfblock protocol");
        let area = Rect::new(0, 0, 10, 4);
        let mut buffer = ratatui::buffer::Buffer::empty(area);

        SlicedImage::new(&protocol, SignedPosition::from((11, 0))).render(area, &mut buffer);

        assert!(buffer.content().iter().all(|cell| cell.symbol() == " "));
    }

    #[test]
    fn iterm_cell_image_has_exact_target_geometry() {
        use image::RgbaImage;

        let img = RgbaImage::from_pixel(4, 3, image::Rgba([12, 34, 56, 255]));
        let image = iterm2_cell_image(
            DynamicImage::ImageRgba8(img),
            FontSize::new(1, 2),
            Size::new(4, 2),
        );

        assert_eq!((image.width(), image.height()), (4, 4));
        let image = image.to_rgba8();
        assert_eq!(image.get_pixel(0, 2).0, [12, 34, 56, 255]);
        assert_eq!(image.get_pixel(0, 3).0, [0, 0, 0, 0]);
    }
}
