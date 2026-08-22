//! # agena-tui-media
//!
//! Terminal media rendering for Agena.
//!
//! Renders images, markdown images, and LaTeX formulas for the terminal UI:
//!
//! - [`render_formula`] — render a LaTeX formula to a graphics artifact.
//! - [`render_markdown_image`] — render a markdown image reference.
//! - [`with_math_render_context`] / [`with_text_math_rendering`] — scoped
//!   rendering contexts used by the transcript.
//! - [`MathLayoutConfig`], [`MathRenderContext`], [`MathGraphicsConfig`] —
//!   layout, graphics configuration, and per-render state.
//!
//! Rendering targets the kitty/iterm2 graphics protocols through
//! [`agena_tui`] terminal graphics and falls back to unicode math when no
//! graphics backend is available.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use agena_tui::terminal_graphics::GraphicsProtocolHint;
use agena_tui_components::TerminalRgb;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use image::{DynamicImage, Rgba};
use ratatui::{
    Frame,
    layout::{Rect, Size},
};
use ratatui_image::{
    FontSize, Resize,
    picker::{Picker, ProtocolType, cap_parser::QueryStdioOptions},
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};
use ratex_layout::{
    LayoutBox, LayoutOptions, layout,
    layout_box::{BoxContent, VBoxChildKind},
    to_display_list,
};
use ratex_parser::parser::parse;

mod render;
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::{color::Color, math_style::MathStyle};
pub use render::*;
use rust_latex_parser::EqNode;

mod remote_image;
mod unicode_math;

const MAX_FORMULA_BYTES: usize = 16 * 1024;
const MAX_MARKDOWN_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_SVG_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_ARTIFACTS: usize = 128;
const MAX_CACHED_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_PROTOCOLS: usize = 256;
const MAX_MATH_AST_NODES: usize = 4_096;
const MAX_MATH_NESTING: usize = 128;
const MAX_UNICODE_GRID_WIDTH: usize = 4_096;
const MAX_UNICODE_GRID_HEIGHT: usize = 1_024;
const MAX_UNICODE_GRID_CELLS: usize = 1_048_576;
const MAX_UNICODE_ARTIFACTS: usize = 256;
const MAX_CACHED_UNICODE_CELLS: usize = 2_097_152;
const MAX_EXPLICIT_FALLBACK_CHARS: usize = 512;
const DEFAULT_DARK_BACKGROUND: TerminalRgb = TerminalRgb::new(24, 24, 27);

#[derive(Debug, Clone, Copy)]
/// Layout configuration for math rendering.
pub struct MathLayoutConfig {
    pub native_graphics: bool,
    pub cell_width: u16,
    pub cell_height: u16,
    pub foreground: [u8; 3],
}

impl Default for MathLayoutConfig {
    fn default() -> Self {
        Self {
            native_graphics: false,
            cell_width: 10,
            cell_height: 20,
            foreground: [235, 235, 235],
        }
    }
}

thread_local! {
    static RENDER_CONTEXT_STACK: RefCell<Vec<MathRenderContext>> = const { RefCell::new(Vec::new()) };
    /// Export and pager rendering cannot serialize terminal image placements.
    /// Keep that decision local to the rendering thread so generating a text
    /// transcript never changes the live terminal's negotiated protocol.
    static TEXT_MATH_RENDER_DEPTH: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug, Clone, Default)]
/// Context of a math render.
pub struct MathRenderContext {
    layout: MathLayoutConfig,
    workspace: Option<Arc<PathBuf>>,
}

impl MathRenderContext {
    pub fn new(graphics: Option<&MathGraphicsConfig>, workspace: &Path) -> Self {
        let workspace = match fs::canonicalize(workspace) {
            Ok(workspace) => workspace,
            Err(error) => {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "canonicalize the TUI media workspace root",
                        &error,
                    ),
                    "TUI media workspace containment will use the unresolved root"
                );
                workspace.to_path_buf()
            }
        };
        Self {
            layout: graphics.map_or_else(MathLayoutConfig::default, |graphics| graphics.layout),
            workspace: Some(Arc::new(workspace)),
        }
    }
}

pub fn with_math_render_context<T>(context: &MathRenderContext, render: impl FnOnce() -> T) -> T {
    struct ContextGuard;

    impl Drop for ContextGuard {
        fn drop(&mut self) {
            RENDER_CONTEXT_STACK.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }

    RENDER_CONTEXT_STACK.with(|stack| stack.borrow_mut().push(context.clone()));
    let _guard = ContextGuard;
    render()
}

#[derive(Clone, Debug)]
/// Graphics configuration for math rendering.
pub struct MathGraphicsConfig {
    picker: Picker,
    layout: MathLayoutConfig,
}

impl MathGraphicsConfig {
    pub fn query(
        background: Option<TerminalRgb>,
        allow_native_graphics: bool,
        through_tmux: bool,
        protocol_hint: Option<GraphicsProtocolHint>,
    ) -> Self {
        // Background detection is a separate terminal-owned transaction and
        // therefore still runs when native graphics are disabled. This query
        // is limited to graphics capabilities and cell geometry.
        let mut picker = picker_for_graphics_policy(allow_native_graphics, through_tmux, || {
            Picker::from_query_stdio_with_options_and_tmux_in_raw_mode(
                QueryStdioOptions::default(),
                through_tmux,
            )
            .unwrap_or_else(|_| unicode_picker(through_tmux))
        });
        apply_protocol_hint(&mut picker, allow_native_graphics, protocol_hint);
        let resolved_background = background.unwrap_or(DEFAULT_DARK_BACKGROUND);
        let font = picker.font_size();
        let layout = MathLayoutConfig {
            native_graphics: picker.protocol_type() != ProtocolType::Halfblocks,
            cell_width: font.width.max(1),
            cell_height: font.height.max(1),
            ..MathLayoutConfig::default()
        };
        let mut config = Self { picker, layout };
        config.apply_terminal_appearance(resolved_background);
        config
    }

    pub fn is_native(&self) -> bool {
        self.picker.protocol_type() != ProtocolType::Halfblocks
    }

    pub fn protocol_name(&self) -> &'static str {
        protocol_name(self.picker.protocol_type())
    }

    /// Retheme generated graphics without repeating terminal capability
    /// negotiation. Detection evidence stays in `TerminalContext`; only the
    /// generated glyph color follows the effective configured appearance, and
    /// the protocol compositor stays alpha-preserving.
    pub fn apply_terminal_appearance(&mut self, background: TerminalRgb) {
        // Preserve source alpha during protocol resizing/padding. Opaque
        // Markdown images remain opaque, while formula PNGs and authored
        // transparent images allow the terminal background to show through.
        self.picker.set_background_color(Some(Rgba([0, 0, 0, 0])));
        self.layout.foreground = foreground_for_background(background);
    }
}

fn picker_for_graphics_policy(
    allow_native_graphics: bool,
    through_tmux: bool,
    query: impl FnOnce() -> Picker,
) -> Picker {
    if allow_native_graphics {
        query()
    } else {
        unicode_picker(through_tmux)
    }
}

fn unicode_picker(through_tmux: bool) -> Picker {
    Picker::from_parts(
        FontSize::new(10, 20),
        ProtocolType::Halfblocks,
        through_tmux,
        Vec::new(),
    )
}

fn apply_protocol_hint(
    picker: &mut Picker,
    allow_native_graphics: bool,
    protocol_hint: Option<GraphicsProtocolHint>,
) {
    if !allow_native_graphics || picker.protocol_type() != ProtocolType::Halfblocks {
        return;
    }
    if let Some(protocol) = protocol_hint {
        picker.set_protocol_type(match protocol {
            GraphicsProtocolHint::Iterm2 => ProtocolType::Iterm2,
            GraphicsProtocolHint::Kitty => ProtocolType::Kitty,
        });
    }
}

const fn protocol_name(protocol: ProtocolType) -> &'static str {
    match protocol {
        ProtocolType::Kitty => "kitty",
        ProtocolType::Sixel => "sixel",
        ProtocolType::Iterm2 => "iterm2",
        ProtocolType::Halfblocks => "unicode",
    }
}

pub fn layout_config() -> MathLayoutConfig {
    let config = RENDER_CONTEXT_STACK.with(|stack| {
        stack
            .borrow()
            .last()
            .map_or_else(MathLayoutConfig::default, |context| context.layout)
    });
    apply_text_math_rendering_policy(config)
}

fn apply_text_math_rendering_policy(mut config: MathLayoutConfig) -> MathLayoutConfig {
    if TEXT_MATH_RENDER_DEPTH.with(|depth| depth.get() > 0) {
        config.native_graphics = false;
    }
    config
}

pub fn with_text_math_rendering<T>(render: impl FnOnce() -> T) -> T {
    struct TextMathRenderGuard {
        previous_depth: usize,
    }

    impl Drop for TextMathRenderGuard {
        fn drop(&mut self) {
            TEXT_MATH_RENDER_DEPTH.with(|depth| depth.set(self.previous_depth));
        }
    }

    let previous_depth = TEXT_MATH_RENDER_DEPTH.with(|depth| {
        let previous = depth.get();
        depth.set(previous.saturating_add(1));
        previous
    });
    let _guard = TextMathRenderGuard { previous_depth };
    render()
}

#[derive(Debug)]
/// A math rendering artifact.
pub struct MathArtifact {
    pub id: u64,
    pub image: DynamicImage,
    pub size: Size,
    /// Relative heights of the outermost native array/alignment rows. Image
    /// artifacts leave this empty; transcript math uses it only when its
    /// independently scanned structural row count matches.
    pub row_layout_weights: Vec<usize>,
}

#[derive(Debug, Clone)]
/// Placement of a math line.
pub struct MathLinePlacement {
    pub column: u16,
    pub artifact: Arc<MathArtifact>,
    pub size: Size,
}

#[derive(Debug, Clone)]
/// Placement of math within a transcript.
pub struct TranscriptMathPlacement {
    pub line: usize,
    pub column: u16,
    pub artifact: Arc<MathArtifact>,
    pub size: Size,
}

#[derive(Default)]
struct ArtifactCache {
    entries: HashMap<u64, Arc<MathArtifact>>,
    recency: VecDeque<u64>,
    pixels: u64,
}

impl ArtifactCache {
    fn get(&mut self, id: u64) -> Option<Arc<MathArtifact>> {
        let artifact = self.entries.get(&id).cloned()?;
        self.recency.retain(|candidate| *candidate != id);
        self.recency.push_back(id);
        Some(artifact)
    }

    fn insert(&mut self, artifact: Arc<MathArtifact>) {
        let id = artifact.id;
        if let Some(previous) = self.entries.insert(id, Arc::clone(&artifact)) {
            self.pixels = self.pixels.saturating_sub(image_pixels(&previous.image));
        }
        self.pixels = self.pixels.saturating_add(image_pixels(&artifact.image));
        self.recency.retain(|candidate| *candidate != id);
        self.recency.push_back(id);
        while self.entries.len() > MAX_ARTIFACTS || self.pixels > MAX_CACHED_PIXELS {
            let Some(expired) = self.recency.pop_front() else {
                break;
            };
            if let Some(previous) = self.entries.remove(&expired) {
                self.pixels = self.pixels.saturating_sub(image_pixels(&previous.image));
            }
        }
    }
}

fn image_pixels(image: &DynamicImage) -> u64 {
    u64::from(image.width()).saturating_mul(u64::from(image.height()))
}

static ARTIFACT_CACHE: LazyLock<Mutex<ArtifactCache>> =
    LazyLock::new(|| Mutex::new(ArtifactCache::default()));

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct UnicodeMathCacheKey {
    source: String,
    display: bool,
    cell_width: u16,
    cell_height: u16,
    foreground: [u8; 3],
}

#[derive(Default)]
struct UnicodeMathCache {
    entries: HashMap<UnicodeMathCacheKey, Arc<Vec<String>>>,
    recency: VecDeque<UnicodeMathCacheKey>,
    cells: usize,
}

impl UnicodeMathCache {
    fn get(&mut self, key: &UnicodeMathCacheKey) -> Option<Arc<Vec<String>>> {
        let lines = self.entries.get(key).cloned()?;
        self.recency.retain(|candidate| candidate != key);
        self.recency.push_back(key.clone());
        Some(lines)
    }

    fn insert(&mut self, key: UnicodeMathCacheKey, lines: Arc<Vec<String>>) {
        if let Some(previous) = self.entries.insert(key.clone(), Arc::clone(&lines)) {
            self.cells = self.cells.saturating_sub(unicode_grid_cells(&previous));
        }
        self.cells = self.cells.saturating_add(unicode_grid_cells(&lines));
        self.recency.retain(|candidate| candidate != &key);
        self.recency.push_back(key);
        while self.entries.len() > MAX_UNICODE_ARTIFACTS || self.cells > MAX_CACHED_UNICODE_CELLS {
            let Some(expired) = self.recency.pop_front() else {
                break;
            };
            if let Some(previous) = self.entries.remove(&expired) {
                self.cells = self.cells.saturating_sub(unicode_grid_cells(&previous));
            }
        }
    }
}

fn unicode_grid_cells(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| unicode_width::UnicodeWidthStr::width(line.as_str()))
        .max()
        .unwrap_or(0)
        .saturating_mul(lines.len())
}

static UNICODE_MATH_CACHE: LazyLock<Mutex<UnicodeMathCache>> =
    LazyLock::new(|| Mutex::new(UnicodeMathCache::default()));

pub fn render_formula(source: &str, display: bool) -> Result<Arc<MathArtifact>, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("empty formula".to_string());
    }
    if source.len() > MAX_FORMULA_BYTES {
        return Err("formula is too large".to_string());
    }

    let config = layout_config();
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    display.hash(&mut hasher);
    config.cell_width.hash(&mut hasher);
    config.cell_height.hash(&mut hasher);
    config.foreground.hash(&mut hasher);
    let id = hasher.finish();
    if let Some(artifact) = ARTIFACT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
    {
        return Ok(artifact);
    }

    let ast = parse(source).map_err(|error| format!("failed to parse formula: {error}"))?;
    let foreground = Color::new(
        f32::from(config.foreground[0]) / 255.0,
        f32::from(config.foreground[1]) / 255.0,
        f32::from(config.foreground[2]) / 255.0,
        1.0,
    );
    let options = LayoutOptions::default()
        .with_style(if display {
            MathStyle::Display
        } else {
            MathStyle::Text
        })
        .with_color(foreground);
    let layout_box = layout(&ast, &options);
    let row_layout_weights = ratex_row_layout_weights(&layout_box).unwrap_or_default();
    let display_list = to_display_list(&layout_box);
    let font_size = f32::from(config.cell_height) * if display { 1.15 } else { 0.95 };
    let padding = f32::from(config.cell_height) * 0.12;
    let projected_width = display_list.width as f32 * font_size + 2.0 * padding;
    let projected_height =
        (display_list.height + display_list.depth) as f32 * font_size + 2.0 * padding;
    if !projected_width.is_finite()
        || !projected_height.is_finite()
        || projected_width > MAX_IMAGE_DIMENSION as f32
        || projected_height > MAX_IMAGE_DIMENSION as f32
    {
        return Err("rendered formula exceeds the image safety limit".to_string());
    }
    let png = render_to_png(
        &display_list,
        &RenderOptions {
            font_size,
            padding,
            // Native terminal image protocols preserve alpha. Keep the
            // formula canvas transparent so terminal colors, transparency,
            // and background images remain visible; only glyph color follows
            // the detected/configured appearance.
            background_color: Color::new(0.0, 0.0, 0.0, 0.0),
            font_dir: String::new(),
            device_pixel_ratio: 1.0,
        },
    )?;
    let image = image::load_from_memory(&png).map_err(|error| {
        agena_failure::diagnostic::format_error_chain_with_context(
            "failed to decode the rendered formula image",
            &error,
        )
    })?;
    if image.width() > MAX_IMAGE_DIMENSION || image.height() > MAX_IMAGE_DIMENSION {
        return Err("rendered formula exceeds the image safety limit".to_string());
    }
    let (image, size) = align_formula_raster_to_cells(image, config, display)?;
    let artifact = Arc::new(MathArtifact {
        id,
        image,
        size,
        row_layout_weights,
    });
    ARTIFACT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(Arc::clone(&artifact));
    Ok(artifact)
}

fn ratex_row_layout_weights(layout: &LayoutBox) -> Option<Vec<usize>> {
    if let BoxContent::Array {
        row_heights,
        row_depths,
        ..
    } = &layout.content
        && row_heights.len() > 1
        && row_heights.len() == row_depths.len()
    {
        return Some(
            row_heights
                .iter()
                .zip(row_depths)
                .map(|(height, depth)| {
                    ((height + depth) * 1024.0).ceil().clamp(1.0, 1_000_000.0) as usize
                })
                .collect(),
        );
    }

    let find = |child: &LayoutBox| ratex_row_layout_weights(child);
    match &layout.content {
        BoxContent::HBox(children) => children.iter().find_map(find),
        BoxContent::VBox(children) => children.iter().find_map(|child| match &child.kind {
            VBoxChildKind::Box(child) => find(child),
            VBoxChildKind::Kern(_) => None,
        }),
        BoxContent::Fraction { numer, denom, .. } => find(numer).or_else(|| find(denom)),
        BoxContent::SupSub { base, sup, sub, .. } | BoxContent::OpLimits { base, sup, sub, .. } => {
            find(base)
                .or_else(|| sup.as_deref().and_then(find))
                .or_else(|| sub.as_deref().and_then(find))
        }
        BoxContent::Radical { body, index, .. } => {
            find(body).or_else(|| index.as_deref().and_then(find))
        }
        BoxContent::Accent { base, accent, .. } => find(base).or_else(|| find(accent)),
        BoxContent::LeftRight { left, right, inner } => {
            find(inner).or_else(|| find(left)).or_else(|| find(right))
        }
        BoxContent::Array {
            cells, row_tags, ..
        } => cells
            .iter()
            .flatten()
            .find_map(find)
            .or_else(|| row_tags.iter().flatten().find_map(find)),
        BoxContent::Framed { body, .. }
        | BoxContent::RaiseBox { body, .. }
        | BoxContent::Scaled { body, .. }
        | BoxContent::Angl { body, .. }
        | BoxContent::Overline { body, .. }
        | BoxContent::Underline { body, .. } => find(body),
        BoxContent::ProofTree { children, .. } => {
            children.iter().find_map(|child| find(&child.box_))
        }
        BoxContent::Glyph { .. }
        | BoxContent::Rule { .. }
        | BoxContent::Kern
        | BoxContent::SvgPath { .. }
        | BoxContent::Empty => None,
    }
}

/// Put a formula on an exact terminal-cell canvas before handing it to a
/// graphics protocol. Protocol implementations otherwise independently round
/// and pad the raster, and several of them put the remainder below the image.
/// For inline math that makes the formula visibly sit above the surrounding
/// text. An even-height image has its center on a cell boundary, while text is
/// drawn around the center of one cell, so inline formulas always use an odd
/// number of rows. Their raster center can then coincide exactly with the
/// surrounding text row instead of approximating it with the lower middle row.
fn align_formula_raster_to_cells(
    image: DynamicImage,
    config: MathLayoutConfig,
    display: bool,
) -> Result<(DynamicImage, Size), String> {
    let cell_width = u32::from(config.cell_width.max(1));
    let cell_height = u32::from(config.cell_height.max(1));
    let width = image
        .width()
        .div_ceil(cell_width)
        .clamp(1, u32::from(u16::MAX));
    let natural_height = image
        .height()
        .div_ceil(cell_height)
        .clamp(1, u32::from(u16::MAX));
    let height = if !display && natural_height.is_multiple_of(2) {
        natural_height.saturating_add(1)
    } else {
        natural_height
    };
    let canvas_width = width.saturating_mul(cell_width);
    let canvas_height = height.saturating_mul(cell_height);
    if canvas_width > MAX_IMAGE_DIMENSION
        || canvas_height > MAX_IMAGE_DIMENSION
        || u64::from(canvas_width).saturating_mul(u64::from(canvas_height)) > MAX_IMAGE_PIXELS
    {
        return Err("rendered formula exceeds the image safety limit".to_string());
    }

    let x = canvas_width.saturating_sub(image.width()) / 2;
    let anchor_center = canvas_height / 2;
    let y = anchor_center
        .saturating_sub(image.height() / 2)
        .min(canvas_height.saturating_sub(image.height()));

    if image.width() == canvas_width && image.height() == canvas_height && x == 0 && y == 0 {
        return Ok((image, Size::new(width as u16, height as u16)));
    }

    let mut canvas = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        canvas_width,
        canvas_height,
        Rgba([0, 0, 0, 0]),
    ));
    image::imageops::overlay(&mut canvas, &image, i64::from(x), i64::from(y));
    Ok((canvas, Size::new(width as u16, height as u16)))
}

/// Decodes a Markdown image. Relative and `file:` URLs are confined to the
/// active workspace; `data:image/*;base64` and asynchronously cached public
/// HTTP(S) URLs use the same byte, dimension, and decoded-pixel limits.
pub fn render_markdown_image(source: &str) -> Result<Arc<MathArtifact>, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("image URL is empty".to_string());
    }
    let bytes = if source.starts_with("data:") {
        Arc::new(decode_data_image(source)?)
    } else if let Ok(url) = url::Url::parse(source)
        && matches!(url.scheme(), "http" | "https")
    {
        remote_image::load(&url)?
    } else {
        Arc::new(read_workspace_image(source)?)
    };
    if looks_like_svg(&bytes) {
        svg_artifact(&bytes)
    } else {
        image_artifact(&bytes)
    }
}

pub fn remote_image_generation() -> u64 {
    remote_image::generation()
}

#[cfg(feature = "test-support")]
pub(crate) fn seed_remote_image(source: &str, bytes: Vec<u8>) {
    remote_image::seed(source, bytes);
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn test_math_render_context(layout: MathLayoutConfig) -> MathRenderContext {
    MathRenderContext {
        layout,
        workspace: None,
    }
}

#[cfg(feature = "test-support")]
pub mod test_support {
    use super::{MathLayoutConfig, MathRenderContext};

    pub fn seed_remote_image(source: &str, bytes: Vec<u8>) {
        super::seed_remote_image(source, bytes);
    }

    pub fn test_math_render_context(layout: MathLayoutConfig) -> MathRenderContext {
        super::test_math_render_context(layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    fn solid_image(width: u32, height: u32, color: Rgba<u8>) -> DynamicImage {
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(width, height, color))
    }

    fn most_opaque_pixel(image: &DynamicImage) -> Rgba<u8> {
        image
            .to_rgba8()
            .pixels()
            .copied()
            .max_by_key(|pixel| pixel[3])
            .unwrap_or(Rgba([0, 0, 0, 0]))
    }

    #[test]
    fn inline_formula_rasters_use_an_odd_canvas_with_an_exact_center_cell() {
        let config = MathLayoutConfig {
            cell_width: 10,
            cell_height: 20,
            ..MathLayoutConfig::default()
        };
        let red = Rgba([255, 0, 0, 255]);
        let transparent = Rgba([0, 0, 0, 0]);

        let (single, single_size) =
            align_formula_raster_to_cells(solid_image(10, 10, red), config, false)
                .expect("one-row formula should align");
        assert_eq!(single_size, Size::new(1, 1));
        assert_eq!((single.width(), single.height()), (10, 20));
        assert_eq!(single.get_pixel(0, 4), transparent);
        assert_eq!(single.get_pixel(0, 5), red);
        assert_eq!(single.get_pixel(0, 14), red);
        assert_eq!(single.get_pixel(0, 15), transparent);

        let (even, even_size) =
            align_formula_raster_to_cells(solid_image(10, 30, red), config, false)
                .expect("formula that naturally needs two rows should align");
        assert_eq!(even_size, Size::new(1, 3));
        assert_eq!((even.width(), even.height()), (10, 60));
        assert_eq!(even.get_pixel(0, 14), transparent);
        assert_eq!(even.get_pixel(0, 15), red);
        assert_eq!(even.get_pixel(0, 44), red);
        assert_eq!(even.get_pixel(0, 45), transparent);

        let (odd, odd_size) =
            align_formula_raster_to_cells(solid_image(10, 44, red), config, false)
                .expect("three-row formula should align");
        assert_eq!(odd_size, Size::new(1, 3));
        assert_eq!((odd.width(), odd.height()), (10, 60));
        assert_eq!(odd.get_pixel(0, 7), transparent);
        assert_eq!(odd.get_pixel(0, 8), red);
        assert_eq!(odd.get_pixel(0, 51), red);
        assert_eq!(odd.get_pixel(0, 52), transparent);
    }

    #[test]
    fn rendered_formula_rasters_exactly_match_their_terminal_cell_geometry() {
        let config = MathLayoutConfig {
            native_graphics: true,
            cell_width: 9,
            cell_height: 18,
            ..MathLayoutConfig::default()
        };
        let context = test_math_render_context(config);
        let artifact =
            with_math_render_context(&context, || render_formula(r"\frac{a+b}{c+d}", false))
                .expect("inline fraction should render");
        let simple = with_math_render_context(&context, || render_formula("x", false))
            .expect("simple inline formula should render");
        assert_eq!(
            simple.size.height, 1,
            "ordinary inline symbols should stay on the surrounding text row"
        );

        assert_eq!(
            artifact.image.width(),
            u32::from(artifact.size.width) * u32::from(config.cell_width)
        );
        assert_eq!(
            artifact.image.height(),
            u32::from(artifact.size.height) * u32::from(config.cell_height)
        );
        assert_eq!(
            artifact.size.height % 2,
            1,
            "inline formula center must occupy a real terminal row"
        );
    }

    #[test]
    fn ratex_renders_a_matrix_to_a_nonempty_image() {
        let artifact = render_formula(r"\begin{bmatrix}1&2\\3&4\end{bmatrix}", true)
            .expect("matrix should render");
        assert!(artifact.image.width() > 1);
        assert!(artifact.image.height() > 1);
        assert!(artifact.size.width >= 1);
        assert!(artifact.size.height >= 1);
        let pixels = artifact.image.to_rgba8();
        assert!(
            pixels.pixels().any(|pixel| pixel[3] == 0),
            "formula padding must remain transparent"
        );
        assert!(
            pixels.pixels().any(|pixel| pixel[3] > 0),
            "formula glyphs must remain visible"
        );
    }

    #[test]
    fn resolved_terminal_background_drives_formula_contrast() {
        let background = TerminalRgb::new(248, 249, 250);
        assert_eq!(background, TerminalRgb::new(248, 249, 250));
        assert_eq!(foreground_for_background(background), [28, 28, 28]);
        assert_eq!(
            foreground_for_background(TerminalRgb::new(18, 18, 20)),
            [235, 235, 235]
        );
    }

    #[test]
    fn configured_appearance_rethemes_the_graphics_layout() {
        let mut config = MathGraphicsConfig {
            picker: unicode_picker(false),
            layout: MathLayoutConfig::default(),
        };

        let light = TerminalRgb::new(250, 250, 250);
        config.apply_terminal_appearance(light);
        assert_eq!(config.layout.foreground, [28, 28, 28]);

        let dark = TerminalRgb::new(24, 24, 27);
        config.apply_terminal_appearance(dark);
        assert_eq!(config.layout.foreground, [235, 235, 235]);
    }

    #[test]
    fn formula_artifacts_are_rebuilt_with_light_and_dark_contrast() {
        let dark = MathRenderContext {
            layout: MathLayoutConfig {
                foreground: [235, 235, 235],
                ..MathLayoutConfig::default()
            },
            workspace: None,
        };
        let light = MathRenderContext {
            layout: MathLayoutConfig {
                foreground: [28, 28, 28],
                ..MathLayoutConfig::default()
            },
            workspace: None,
        };

        let dark_artifact = with_math_render_context(&dark, || render_formula("x^2+1", true))
            .expect("dark formula should render");
        let light_artifact = with_math_render_context(&light, || render_formula("x^2+1", true))
            .expect("light formula should render");

        assert_ne!(dark_artifact.id, light_artifact.id);
        assert_eq!(dark_artifact.image.to_rgba8().get_pixel(0, 0)[3], 0);
        assert_eq!(light_artifact.image.to_rgba8().get_pixel(0, 0)[3], 0);
        assert_eq!(
            &most_opaque_pixel(&dark_artifact.image).0[..3],
            &[235, 235, 235]
        );
        assert_eq!(
            &most_opaque_pixel(&light_artifact.image).0[..3],
            &[28, 28, 28]
        );
    }

    #[test]
    fn svg_current_color_tracks_appearance_and_invalidates_the_cache() {
        let source = concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2">"#,
            r#"<rect width="2" height="2" fill="currentColor"/>"#,
            "</svg>"
        );
        let dark = MathRenderContext {
            layout: MathLayoutConfig {
                foreground: [235, 235, 235],
                ..MathLayoutConfig::default()
            },
            workspace: None,
        };
        let light = MathRenderContext {
            layout: MathLayoutConfig {
                foreground: [28, 28, 28],
                ..MathLayoutConfig::default()
            },
            workspace: None,
        };

        let dark_artifact = with_math_render_context(&dark, || render_markdown_svg(source))
            .expect("dark SVG should render");
        let light_artifact = with_math_render_context(&light, || render_markdown_svg(source))
            .expect("light SVG should render");

        assert_ne!(dark_artifact.id, light_artifact.id);
        assert_eq!(
            &dark_artifact.image.to_rgba8().get_pixel(0, 0).0[..3],
            &[235, 235, 235]
        );
        assert_eq!(
            &light_artifact.image.to_rgba8().get_pixel(0, 0).0[..3],
            &[28, 28, 28]
        );

        let authored = concat!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2" color="#c32148">"##,
            r#"<rect width="2" height="2" fill="currentColor"/>"#,
            "</svg>"
        );
        let authored_artifact = with_math_render_context(&dark, || render_markdown_svg(authored))
            .expect("SVG with an authored color should render");
        let authored_light = with_math_render_context(&light, || render_markdown_svg(authored))
            .expect("authored SVG color should remain valid in light mode");
        assert_eq!(authored_artifact.id, authored_light.id);
        assert_eq!(
            &authored_artifact.image.to_rgba8().get_pixel(0, 0).0[..3],
            &[0xc3, 0x21, 0x48],
            "theme defaults must not override SVG-authored colors"
        );
    }

    #[test]
    fn disabled_native_graphics_never_queries_the_terminal() {
        let queried = std::cell::Cell::new(false);
        let picker = picker_for_graphics_policy(false, true, || {
            queried.set(true);
            unicode_picker(true)
        });

        assert!(!queried.get());
        assert_eq!(picker.protocol_type(), ProtocolType::Halfblocks);

        let picker = picker_for_graphics_policy(true, false, || {
            queried.set(true);
            unicode_picker(false)
        });
        assert!(queried.get());
        assert_eq!(picker.protocol_type(), ProtocolType::Halfblocks);
    }

    #[test]
    fn explicit_endpoint_hint_never_overrides_a_blocked_transport_policy() {
        let mut blocked = unicode_picker(true);
        apply_protocol_hint(&mut blocked, false, Some(GraphicsProtocolHint::Iterm2));
        assert_eq!(blocked.protocol_type(), ProtocolType::Halfblocks);

        let mut verified = unicode_picker(true);
        apply_protocol_hint(&mut verified, true, Some(GraphicsProtocolHint::Iterm2));
        assert_eq!(verified.protocol_type(), ProtocolType::Iterm2);
    }

    #[test]
    fn text_math_rendering_scope_disables_images_without_mutating_terminal_policy() {
        let native = MathLayoutConfig {
            native_graphics: true,
            ..MathLayoutConfig::default()
        };

        assert!(apply_text_math_rendering_policy(native).native_graphics);
        with_text_math_rendering(|| {
            assert!(!apply_text_math_rendering_policy(native).native_graphics);
            with_text_math_rendering(|| {
                assert!(!apply_text_math_rendering_policy(native).native_graphics);
            });
            assert!(!apply_text_math_rendering_policy(native).native_graphics);
        });
        assert!(apply_text_math_rendering_policy(native).native_graphics);
    }

    #[test]
    fn render_configuration_is_scoped_and_nested_instead_of_process_global() {
        let first_layout = MathLayoutConfig {
            cell_width: 7,
            cell_height: 14,
            ..MathLayoutConfig::default()
        };
        let second_layout = MathLayoutConfig {
            cell_width: 11,
            cell_height: 22,
            ..MathLayoutConfig::default()
        };
        let first = MathRenderContext {
            layout: first_layout,
            workspace: None,
        };
        let second = MathRenderContext {
            layout: second_layout,
            workspace: None,
        };

        with_math_render_context(&first, || {
            assert_eq!(layout_config().cell_width, 7);
            with_math_render_context(&second, || {
                assert_eq!(layout_config().cell_width, 11);
            });
            assert_eq!(layout_config().cell_width, 7);
        });
        assert_eq!(
            layout_config().cell_width,
            MathLayoutConfig::default().cell_width
        );
    }

    #[test]
    fn unicode_fallback_is_two_dimensional_for_a_fraction() {
        let lines = unicode_formula(r"\frac{a}{b}", true);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn unicode_inline_math_uses_a_compact_text_style() {
        let inline = unicode_formula(r"x_i=\frac{a+b}{c}", false);
        assert_eq!(
            inline.len(),
            1,
            "inline formula should not expand the paragraph"
        );
        assert!(inline[0].contains('⁄'));
        assert!(inline[0].contains('ᵢ'));

        let display = unicode_formula(r"x_i=\frac{a+b}{c}", true);
        assert!(
            display.len() >= 2,
            "display formula should retain 2D fractions"
        );
    }

    #[test]
    fn unicode_fallback_normalizes_styled_fractions_and_row_spacing() {
        let lines = unicode_formula(
            concat!(
                r"\begin{cases}",
                "\n",
                r"x_1 + x_2 = -\dfrac{b}{a} \\[8pt]",
                "\n",
                r"x_1 \cdot x_2 = \dfrac{c}{a}",
                "\n",
                r"\end{cases}",
            ),
            true,
        );
        let rendered = lines.join("\n");

        assert!(
            lines
                .iter()
                .all(|line| matches!(line.chars().next(), Some('⎧' | '⎨' | '⎩'))),
            "multiline source must not split rows away from the cases brace:\n{rendered}"
        );
        assert!(rendered.contains('⎧'), "missing cases brace:\n{rendered}");
        assert!(
            rendered.contains("x₁ + x₂"),
            "missing first row:\n{rendered}"
        );
        assert!(rendered.contains("x₁"), "missing second x₁:\n{rendered}");
        assert!(
            rendered.contains('·'),
            "missing product operator:\n{rendered}"
        );
        assert!(
            rendered.matches("x₂").count() >= 2,
            "missing x₂:\n{rendered}"
        );
        assert!(
            rendered.contains('─'),
            "fractions should have bars:\n{rendered}"
        );
        assert!(
            !rendered.contains("dfrac"),
            "raw command leaked:\n{rendered}"
        );
        assert!(!rendered.contains("8pt"), "row spacing leaked:\n{rendered}");
    }

    #[test]
    fn unicode_fallback_renders_extended_big_operators_with_limits() {
        let rendered = unicode_formula(
            r"\left|\bigcup_{i=1}^{n} A_i\right| = \sum_{i=1}^{n} |A_i|",
            true,
        )
        .join("\n");

        assert!(
            rendered.contains('⋃'),
            "missing union operator:\n{rendered}"
        );
        assert!(
            rendered.contains('∑'),
            "missing summation operator:\n{rendered}"
        );
        assert!(
            rendered.contains("i = 1"),
            "missing lower limit:\n{rendered}"
        );
        assert!(rendered.contains('n'), "missing upper limit:\n{rendered}");
        assert!(
            !rendered.contains(r"\bigcup"),
            "raw big operator leaked:\n{rendered}"
        );

        for (command, symbol) in [
            (r"\bigcap", '⋂'),
            (r"\bigwedge", '⋀'),
            (r"\bigvee", '⋁'),
            (r"\bigsqcup", '⨆'),
            (r"\bigodot", '⨀'),
            (r"\bigoplus", '⨁'),
            (r"\bigotimes", '⨂'),
            (r"\biguplus", '⨄'),
        ] {
            let rendered = unicode_formula(&format!(r"{command}_{{i=1}}^n A_i"), true).join("\n");
            assert!(
                rendered.contains(symbol),
                "{command} did not render as {symbol}:\n{rendered}"
            );
            assert!(
                !rendered.contains(command),
                "raw command leaked for {command}:\n{rendered}"
            );
        }
    }

    #[test]
    fn unicode_fallback_keeps_binomial_arguments_inside_parentheses() {
        let lines = unicode_formula(r"\binom{n}{k} = \frac{n!}{k!(n-k)!}", true);
        let rendered = lines.join("\n");
        let binomial_rows = lines
            .iter()
            .filter(|line| line.contains('n') || line.contains('k'))
            .take(2)
            .collect::<Vec<_>>();

        assert_eq!(binomial_rows.len(), 2, "missing binomial rows:\n{rendered}");
        assert!(
            binomial_rows.iter().all(|line| {
                let left = line.find(['⎛', '⎝']);
                let value = line.find(['n', 'k']);
                let right = line.find(['⎞', '⎠']);
                matches!((left, value, right), (Some(left), Some(value), Some(right)) if left < value && value < right)
            }),
            "both binomial arguments must stay inside the parentheses:\n{rendered}"
        );
        assert!(rendered.contains('─'), "fraction bar missing:\n{rendered}");
    }

    #[test]
    fn unicode_fallback_renders_aligned_equation_systems_as_grids() {
        let rendered = unicode_formula(
            concat!(
                r"\begin{aligned}",
                r"\nabla \cdot \mathbf{E} &= \frac{\rho}{\varepsilon_0} \\",
                r"\nabla \cdot \mathbf{B} &= 0 \\",
                r"\nabla \times \mathbf{E} &= -\frac{\partial \mathbf{B}}{\partial t} \\",
                r"\nabla \times \mathbf{B} &= \mu_0 \mathbf{J}",
                r"\end{aligned}",
            ),
            true,
        )
        .join("\n");

        assert_eq!(
            rendered.matches('∇').count(),
            4,
            "aligned rows were lost:\n{rendered}"
        );
        assert!(
            rendered.contains('ρ'),
            "fraction numerator missing:\n{rendered}"
        );
        assert!(
            rendered.contains('ε'),
            "fraction denominator missing:\n{rendered}"
        );
        assert!(
            rendered.contains('∂'),
            "partial derivative missing:\n{rendered}"
        );
        assert!(
            !rendered.contains(r"\begin") && !rendered.contains(r"\end"),
            "raw environment leaked:\n{rendered}"
        );

        for source in [
            r"\begin{alignedat}{2}a&=b\\c&=d\end{alignedat}",
            r"\begin{array}[t]{cc}a&b\\c&d\end{array}",
            r"\begin{gathered}a\\b\end{gathered}",
            r"\begin{split}a&=b\\c&=d\end{split}",
        ] {
            let rendered = unicode_formula(source, true).join("\n");
            assert!(rendered.contains('a'), "first row missing:\n{rendered}");
            assert!(rendered.contains('d') || rendered.contains('b'));
            assert!(
                !rendered.contains(r"\begin") && !rendered.contains(r"\end"),
                "raw matrix-like environment leaked:\n{rendered}"
            );
        }
    }

    #[test]
    fn unicode_fallback_handles_common_ams_commands_without_leaking_latex() {
        let rendered = unicode_formula(
            concat!(
                r"X_n \xrightarrow{P} \mu, ",
                r"a^{p-1} \equiv 1 \pmod{p} \quad \blacksquare",
            ),
            true,
        )
        .join("\n");

        assert!(
            rendered.contains('→'),
            "annotated arrow missing:\n{rendered}"
        );
        assert!(
            rendered.contains('P'),
            "arrow annotation missing:\n{rendered}"
        );
        assert!(
            rendered.contains("mod"),
            "modulus notation missing:\n{rendered}"
        );
        assert!(
            rendered.contains('∎'),
            "proof terminator missing:\n{rendered}"
        );
        assert!(!rendered.contains('\\'), "raw LaTeX leaked:\n{rendered}");
    }

    #[test]
    fn unicode_fallback_renders_common_symbol_and_style_aliases_semantically() {
        let rendered = unicode_formula(
            concat!(
                r"a \odot b, A^{\complement}, x \mid y, x \doteq y, ",
                r"\nearrow \nwarrow \searrow \swarrow \updownarrow \longmapsto, ",
                r"\llbracket x \rrbracket, \dbinom{n}{k}, ",
                r"\operatorname{rank}(A), \mathscr{L}, \Big( x \Big)",
            ),
            true,
        )
        .join("\n");

        for symbol in ['⊙', '∁', '∣', '≐', '↗', '↖', '↘', '↙', '↕', '⟼', '⟦', '⟧']
        {
            assert!(
                rendered.contains(symbol),
                "missing compatibility symbol {symbol}:\n{rendered}"
            );
        }
        assert!(
            rendered.contains("rank"),
            "operator name disappeared:\n{rendered}"
        );
        assert!(
            rendered.contains('ℒ'),
            "script font alias disappeared:\n{rendered}"
        );
        assert!(
            !rendered.contains("⟦LaTeX:"),
            "supported aliases unexpectedly fell back to source:\n{rendered}"
        );
        assert!(
            !rendered
                .chars()
                .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
            "compatibility aliases emitted Braille cells:\n{rendered}"
        );
    }

    #[test]
    fn unicode_fallback_renders_session_1_extended_structures_without_source_markers() {
        let formulas = [
            r"\sum_{\substack{0 \le i \le n \\ i \ne j}} x_i",
            r"\sqrt[3]{x}",
            r"\sqrt[n]{x}",
            r"\left( \frac{a}{b} \right) \quad \left[ \frac{a}{b} \right] \quad \left\{ \frac{a}{b} \right\}",
            r"\left\langle \frac{a}{b} \right\rangle \quad \left| \frac{a}{b} \right| \quad \left\| \frac{a}{b} \right\|",
            r"\overleftarrow{AB} \quad \overleftrightarrow{AB} \quad \overrightarrow{AB}",
            r"\underleftarrow{AB} \quad \underleftrightarrow{AB} \quad \underrightarrow{AB}",
            r"\widetilde{abc} \quad \widehat{abc} \quad \overline{abc} \quad \underline{abc}",
            r"\ce{H2O} \quad \ce{CO2} \quad \ce{CH3COOH}",
            r"\ce{2H2 + O2 -> 2H2O}",
            r"\ce{NaOH + HCl -> NaCl + H2O}",
            r"\ce{CH4 + 2O2 -> CO2 + 2H2O}",
            r"\ce{^{227}_{90}Th -> _{88}^{223}Ra + _{2}^{4}He}",
        ];

        for source in formulas {
            let rendered = unicode_formula(source, true).join("\n");
            assert!(
                !rendered.contains("⟦LaTeX:"),
                "session formula unexpectedly fell back to source: {source}\n{rendered}"
            );
            assert!(
                rendered.chars().any(|ch| !ch.is_whitespace()),
                "session formula disappeared: {source}"
            );
            assert!(
                !rendered
                    .chars()
                    .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
                "session formula emitted Braille raster cells: {source}\n{rendered}"
            );
        }

        let roots = unicode_formula(r"\sqrt[3]{x} \quad \sqrt[n]{y}", true).join("\n");
        assert!(roots.contains('√') && roots.contains('3') && roots.contains('n'));

        let chemistry = unicode_formula(
            r"\ce{H2O + CO2 -> H2CO3} \quad \ce{^{227}_{90}Th -> _{88}^{223}Ra}",
            true,
        )
        .join("\n");
        for expected in ["H₂O", "CO₂", "→", "²²⁷₉₀Th", "²²³₈₈Ra"] {
            assert!(
                chemistry.contains(expected),
                "missing normalized chemistry {expected}:\n{chemistry}"
            );
        }
    }

    #[test]
    fn unsupported_semantic_math_uses_a_bounded_source_fallback() {
        let source = r"\definitelyunsupported{x}";
        let rendered = unicode_formula(source, true).join("\n");
        assert!(
            rendered.starts_with("⟦LaTeX: "),
            "unsupported semantic nodes should retain readable source:\n{rendered}"
        );
        assert!(
            rendered.contains(source),
            "fallback lost the original formula:\n{rendered}"
        );
        assert!(
            !rendered
                .chars()
                .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
            "math fallback must never emit Braille raster cells:\n{rendered}"
        );

        let repeated = unicode_formula(source, true).join("\n");
        assert_eq!(
            rendered, repeated,
            "cached layout must remain deterministic"
        );
    }

    #[test]
    fn session_709_math_compatibility_corpus_never_disappears_or_emits_braille() {
        for source in [
            r"\log_a b = \frac{\log_c b}{\log_c a}",
            r"\begin{cases}x_1+x_2=-\dfrac{b}{a}\\[8pt]x_1x_2=\dfrac{c}{a}\end{cases}",
            r"\begin{vmatrix}\mathbf{i}&\mathbf{j}&\mathbf{k}\\\partial_x&\partial_y&\partial_z\\F_x&F_y&F_z\end{vmatrix}",
            r"\begin{bmatrix}a_{11}&a_{12}\\a_{21}&a_{22}\end{bmatrix}\begin{bmatrix}b_{11}&b_{12}\\b_{21}&b_{22}\end{bmatrix}",
            r"\binom{n}{k}=\frac{n!}{k!(n-k)!}",
            r"\left|\bigcup_{i=1}^{n}A_i\right|=\sum_{i=1}^{n}|A_i|",
            r"\begin{aligned}\nabla\cdot\mathbf E&=\frac{\rho}{\varepsilon_0}\\\nabla\times\mathbf B&=\mu_0\mathbf J\end{aligned}",
            r"\operatorname{rank}(A)=\sqrt[3]{8}",
            r"X_n\xrightarrow{P}\mu,\quad a^{p-1}\equiv1\pmod p\quad\blacksquare",
            r"\left\langle x,y\right\rangle",
        ] {
            let lines = unicode_formula(source, true);
            let rendered = lines.join("\n");
            assert!(
                lines.iter().any(|line| !line.trim().is_empty()),
                "formula disappeared: {source}"
            );
            assert!(
                !rendered
                    .chars()
                    .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
                "formula emitted unreadable Braille raster cells for {source}:\n{rendered}"
            );
            assert!(
                !rendered.contains("⟦LaTeX:"),
                "compatibility formula unexpectedly fell back to source for {source}:\n{rendered}"
            );
            assert!(
                lines.len() <= MAX_UNICODE_GRID_HEIGHT
                    && unicode_grid_cells(&lines) <= MAX_UNICODE_GRID_CELLS,
                "formula exceeded its terminal grid budget: {source}"
            );
        }
    }

    #[test]
    fn malformed_math_has_an_explicit_visible_failure_mode() {
        let rendered = unicode_formula(r"\begin{aligned}\nabla\cdot\mathbf E", true).join("\n");
        assert!(!rendered.trim().is_empty());
        assert!(
            rendered.chars().any(|ch| !ch.is_whitespace()),
            "malformed formulas must never reserve invisible rows"
        );
    }

    #[test]
    fn oversized_unicode_math_fails_explicitly_and_stays_bounded() {
        let source = "x".repeat(MAX_FORMULA_BYTES + 1);
        let rendered = unicode_formula(&source, true).join("\n");
        assert!(rendered.starts_with("⟦LaTeX: "));
        assert!(rendered.chars().count() <= MAX_EXPLICIT_FALLBACK_CHARS + 12);
    }

    #[test]
    fn cases_do_not_invent_an_english_condition_keyword() {
        let rendered =
            unicode_formula(r"\begin{cases}x^2 & x \ge 0 \\ -x & x < 0\end{cases}", true)
                .join("\n");
        assert!(
            !rendered.contains(" if "),
            "source did not contain 'if':\n{rendered}"
        );
        assert!(rendered.contains('⎧'));
    }

    #[test]
    fn markdown_data_images_decode_into_bounded_graphics_artifacts() {
        let source = concat!(
            "data:image/png;base64,",
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk",
            "+A8AAQUBAScY42YAAAAASUVORK5CYII="
        );
        let artifact = render_markdown_image(source).expect("tiny PNG should decode");
        assert_eq!(artifact.image.width(), 1);
        assert_eq!(artifact.image.height(), 1);
    }

    #[test]
    fn attachment_data_urls_are_bounded_before_allocation() {
        let oversized = "A".repeat(MAX_MARKDOWN_IMAGE_BYTES.saturating_mul(4).div_ceil(3) + 4);
        let error = bounded_image_data_url("image/png", &oversized)
            .expect_err("oversized base64 attachment must be rejected before copying");
        assert!(error.contains("safety limit"));
    }

    #[test]
    fn attachment_data_urls_keep_only_a_safe_image_mime_essence() {
        assert_eq!(
            bounded_image_data_url("DATA:IMAGE/SVG+XML;charset=utf-8", "AA==")
                .expect("safe SVG MIME"),
            "data:image/SVG+XML;base64,AA=="
        );
        assert_eq!(
            bounded_image_data_url("text/plain\nimage/svg+xml", "AA==")
                .expect("invalid MIME falls back safely"),
            "data:image/png;base64,AA=="
        );
    }

    #[test]
    fn remote_images_never_block_without_a_tui_runtime() {
        let error = render_markdown_image("https://example.com/tracker.png")
            .expect_err("a synchronous caller must not perform network I/O");
        assert!(error.contains("TUI runtime"));
    }

    #[test]
    fn cached_remote_images_enter_the_bounded_graphics_pipeline() {
        let bytes = BASE64_STANDARD
            .decode(concat!(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk",
                "+A8AAQUBAScY42YAAAAASUVORK5CYII="
            ))
            .expect("test PNG");
        remote_image::seed("https://images.example.test/pixel.png", bytes);
        let artifact = render_markdown_image("https://images.example.test/pixel.png")
            .expect("cached remote image should decode");
        assert_eq!((artifact.image.width(), artifact.image.height()), (1, 1));
    }

    #[test]
    fn markdown_svg_images_are_safely_rasterized() {
        let svg = concat!(
            "data:image/svg+xml;base64,",
            "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMiIg",
            "aGVpZ2h0PSI4Ij48cmVjdCB3aWR0aD0iMTIiIGhlaWdodD0iOCIgZmlsbD0iI2ZmMDAw",
            "MCIvPjwvc3ZnPg=="
        );
        let artifact = render_markdown_image(svg).expect("small SVG should rasterize");
        assert_eq!(artifact.image.width(), 12);
        assert_eq!(artifact.image.height(), 8);
    }
}
