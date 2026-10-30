use super::*;
use ratatui::{
    Terminal,
    backend::TestBackend,
    buffer::{Buffer, CellDiffOption},
    style::{Color, Modifier, Style},
};

fn fixture() -> (MathGraphicsRenderer, TranscriptMathPlacement) {
    let picker = Picker::from_parts(FontSize::new(2, 2), ProtocolType::Iterm2, false, Vec::new());
    let config = MathGraphicsConfig {
        picker,
        image_background: [24, 24, 27],
        layout: MathLayoutConfig {
            native_graphics: true,
            cell_width: 2,
            cell_height: 2,
            ..Default::default()
        },
    };
    let mut image = DynamicImage::new_rgba8(4, 6).to_rgba8();
    for y in 0..6 {
        image.put_pixel(1, y, Rgba([235, 235, 235, 255]));
    }
    let artifact = Arc::new(MathArtifact {
        id: 123,
        image: DynamicImage::ImageRgba8(image),
        size: Size::new(2, 3),
        foreground: Some([235, 235, 235]),
        row_layout_weights: Vec::new(),
    });
    (
        MathGraphicsRenderer::new(config).unwrap(),
        TranscriptMathPlacement {
            line: 1,
            column: 2,
            size: artifact.size,
            artifact,
        },
    )
}

fn draw(
    terminal: &mut Terminal<TestBackend>,
    renderer: &mut MathGraphicsRenderer,
    placement: &TranscriptMathPlacement,
    selected: Option<Rect>,
    scroll: usize,
) -> Buffer {
    terminal
        .draw(|frame| {
            if let Some(selected) = selected {
                frame.buffer_mut().set_style(
                    selected,
                    Style::default()
                        .bg(Color::Rgb(130, 80, 223))
                        .fg(Color::Rgb(250, 250, 250))
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            renderer.render(frame, frame.area(), scroll, std::slice::from_ref(placement));
        })
        .unwrap()
        .buffer
        .clone()
}

fn decoded_image(buffer: &Buffer, x: u16, y: u16) -> image::RgbaImage {
    let payload = buffer[(x, y)]
        .symbol()
        .rsplit_once(':')
        .unwrap()
        .1
        .trim_end_matches('\x07');
    let bytes = BASE64_STANDARD.decode(payload).unwrap();
    image::load_from_memory(&bytes).unwrap().to_rgba8()
}

#[test]
fn halfblock_previews_keep_two_pixel_colors_and_composite_source_alpha() {
    let mut pixels = image::RgbaImage::new(1, 2);
    pixels.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
    pixels.put_pixel(0, 1, Rgba([0, 0, 255, 0]));
    let artifact = MathArtifact {
        id: 999,
        image: DynamicImage::ImageRgba8(pixels),
        size: Size::new(1, 1),
        foreground: None,
        row_layout_weights: Vec::new(),
    };
    let context = MathRenderContext {
        images_enabled: true,
        image_background: [240, 241, 242],
        ..Default::default()
    };
    let lines = with_math_render_context(&context, || {
        assert!(image_rendering_enabled());
        with_text_math_rendering(|| assert!(!image_rendering_enabled()));
        assert!(image_rendering_enabled());
        image_halfblock_lines(&artifact, artifact.size)
    });
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].spans[0].content, "▀");
    assert_eq!(lines[0].spans[0].style.fg, Some(Color::Rgb(255, 0, 0)));
    assert_eq!(lines[0].spans[0].style.bg, Some(Color::Rgb(240, 241, 242)));
    assert!(!image_rendering_enabled());
}

#[test]
fn native_canvas_does_not_inherit_text_selection_modifiers() {
    let (mut renderer, placement) = fixture();
    let mut terminal = Terminal::new(TestBackend::new(8, 6)).unwrap();
    let selected = draw(
        &mut terminal,
        &mut renderer,
        &placement,
        Some(Rect::new(0, 1, 8, 3)),
        0,
    );
    for y in 1..4 {
        for x in 2..4 {
            assert_eq!(selected[(x, y)].modifier, Modifier::empty());
            assert_eq!(selected[(x, y)].bg, Color::Reset);
            if x != 2 || y != 1 {
                assert_eq!(selected[(x, y)].diff_option, CellDiffOption::Skip);
            }
        }
    }
    let png = decoded_image(&selected, 2, 1);
    assert_eq!(png.get_pixel(0, 5), &Rgba([130, 80, 223, 255]));
    assert!(selected[(0, 3)].modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn selecting_lower_rows_repaints_the_image_and_clearing_restores_alpha() {
    let (mut renderer, placement) = fixture();
    let mut terminal = Terminal::new(TestBackend::new(8, 6)).unwrap();
    let plain = draw(&mut terminal, &mut renderer, &placement, None, 0);
    let selected = draw(
        &mut terminal,
        &mut renderer,
        &placement,
        Some(Rect::new(0, 3, 8, 1)),
        0,
    );
    assert_ne!(plain[(2, 1)].symbol(), selected[(2, 1)].symbol());
    let png = decoded_image(&selected, 2, 1);
    assert_eq!(png.get_pixel(0, 0).0[3], 0);
    assert_eq!(png.get_pixel(0, 5), &Rgba([130, 80, 223, 255]));
    let cleared = draw(&mut terminal, &mut renderer, &placement, None, 0);
    assert_eq!(plain, cleared);
}

#[test]
fn viewport_clipping_keeps_selection_on_the_correct_source_rows() {
    let (mut renderer, placement) = fixture();
    let mut terminal = Terminal::new(TestBackend::new(8, 2)).unwrap();
    // The first formula row is above the viewport; the last row is selected.
    let clipped = draw(
        &mut terminal,
        &mut renderer,
        &placement,
        Some(Rect::new(0, 1, 8, 1)),
        2,
    );
    let png = decoded_image(&clipped, 2, 0);
    assert_eq!(png.dimensions(), (4, 4));
    assert_eq!(png.get_pixel(0, 0).0[3], 0);
    assert_eq!(png.get_pixel(0, 3), &Rgba([130, 80, 223, 255]));
    assert_eq!(png.get_pixel(1, 3), &Rgba([250, 250, 250, 255]));
    assert_eq!(placement.artifact.image.to_rgba8().get_pixel(0, 5).0[3], 0);

    // Bottom clipping must also preserve the selected row's relative position.
    let clipped = draw(
        &mut terminal,
        &mut renderer,
        &placement,
        Some(Rect::new(0, 1, 8, 1)),
        0,
    );
    let png = decoded_image(&clipped, 2, 1);
    assert_eq!(png.dimensions(), (4, 2));
    assert_eq!(png.get_pixel(0, 1), &Rgba([130, 80, 223, 255]));
}

#[test]
fn selection_preserves_authored_colors_and_original_image_pixels() {
    let (mut renderer, mut placement) = fixture();
    let artifact = Arc::get_mut(&mut placement.artifact).unwrap();
    let image = artifact.image.as_mut_rgba8().unwrap();
    image.put_pixel(2, 5, Rgba([255, 0, 0, 255]));
    image.put_pixel(3, 5, Rgba([235, 235, 235, 128]));
    let mut terminal = Terminal::new(TestBackend::new(8, 6)).unwrap();
    let selected = draw(
        &mut terminal,
        &mut renderer,
        &placement,
        Some(Rect::new(0, 1, 8, 3)),
        0,
    );
    let png = decoded_image(&selected, 2, 1);
    assert_eq!(png.get_pixel(1, 5), &Rgba([250, 250, 250, 255]));
    assert_eq!(png.get_pixel(2, 5), &Rgba([255, 0, 0, 255]));
    assert_eq!(png.get_pixel(3, 5), &Rgba([190, 165, 237, 255]));

    // An authored image is tinted, rather than replacing its colors as ink.
    Arc::get_mut(&mut placement.artifact).unwrap().foreground = None;
    renderer.protocols.clear();
    let selected = draw(
        &mut terminal,
        &mut renderer,
        &placement,
        Some(Rect::new(0, 1, 8, 3)),
        0,
    );
    let png = decoded_image(&selected, 2, 1);
    assert_eq!(png.get_pixel(1, 5), &Rgba([209, 197, 232, 255]));
    assert_eq!(png.get_pixel(2, 5), &Rgba([224, 20, 55, 255]));
}

#[test]
fn opaque_image_selection_repaints_lower_rows_and_preserves_the_cached_source() {
    let (mut renderer, mut placement) = fixture();
    let artifact = Arc::get_mut(&mut placement.artifact).unwrap();
    artifact.foreground = None;
    artifact.image = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 6, |x, y| {
        Rgba([40 + x as u8 * 30, 60 + y as u8 * 20, 180, 255])
    }));
    let original = artifact.image.to_rgba8();
    let mut terminal = Terminal::new(TestBackend::new(8, 6)).unwrap();
    let plain = draw(&mut terminal, &mut renderer, &placement, None, 0);
    let selected = draw(
        &mut terminal,
        &mut renderer,
        &placement,
        Some(Rect::new(0, 3, 8, 1)),
        0,
    );
    assert_ne!(plain[(2, 1)].symbol(), selected[(2, 1)].symbol());
    let png = decoded_image(&selected, 2, 1);
    assert_eq!(png.dimensions(), original.dimensions());
    assert_eq!(png.get_pixel(1, 0), original.get_pixel(1, 0));
    assert_ne!(png.get_pixel(1, 5), original.get_pixel(1, 5));
    for y in 1..4 {
        for x in 2..4 {
            assert!(selected[(x, y)].modifier.is_empty());
        }
    }
    let cleared = draw(&mut terminal, &mut renderer, &placement, None, 0);
    assert_eq!(plain, cleared);
    assert_eq!(placement.artifact.image.to_rgba8(), original);
    let clipped = draw(
        &mut terminal,
        &mut renderer,
        &placement,
        Some(Rect::new(0, 1, 8, 1)),
        2,
    );
    let png = decoded_image(&clipped, 2, 0);
    assert_eq!(png.dimensions(), (4, 4));
    assert_eq!(png.get_pixel(0, 0), original.get_pixel(0, 2));
    assert_ne!(png.get_pixel(0, 3), original.get_pixel(0, 5));
}

#[test]
fn pointer_motion_replaces_the_cached_highlight_instead_of_accumulating_variants() {
    let (mut renderer, placement) = fixture();
    let mut terminal = Terminal::new(TestBackend::new(8, 6)).unwrap();
    for row in [1, 2, 3, 2, 1] {
        let selected = draw(
            &mut terminal,
            &mut renderer,
            &placement,
            Some(Rect::new(0, row, 8, 1)),
            0,
        );
        let same = draw(
            &mut terminal,
            &mut renderer,
            &placement,
            Some(Rect::new(0, row, 8, 1)),
            0,
        );
        assert!(selected.diff(&same).is_empty());
        assert_eq!(renderer.protocols.len(), 1);
    }
}
