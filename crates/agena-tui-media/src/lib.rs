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

mod graphics_selection;
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
    static DEFER_REMOTE_IMAGES: Cell<bool> = const { Cell::new(false) };
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
    images_enabled: bool,
    image_background: [u8; 3],
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
            images_enabled: true,
            image_background: graphics.map_or(
                [
                    DEFAULT_DARK_BACKGROUND.red,
                    DEFAULT_DARK_BACKGROUND.green,
                    DEFAULT_DARK_BACKGROUND.blue,
                ],
                |graphics| graphics.image_background,
            ),
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
    image_background: [u8; 3],
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
        let mut config = Self {
            picker,
            layout,
            image_background: [0; 3],
        };
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
        self.image_background = [background.red, background.green, background.blue];
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

/// A terminal can display raster previews with colored half blocks even when
/// it has no native graphics protocol. Text exports still keep captions only.
pub fn image_rendering_enabled() -> bool {
    TEXT_MATH_RENDER_DEPTH.with(|depth| depth.get() == 0)
        && RENDER_CONTEXT_STACK.with(|stack| {
            stack
                .borrow()
                .last()
                .is_some_and(|context| context.images_enabled)
        })
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
    /// Default formula ink, used to recolor selected glyphs while preserving
    /// authored colors. Ordinary image/SVG artifacts leave this unset.
    pub foreground: Option<[u8; 3]>,
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
        foreground: Some(config.foreground),
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

/// Layout offscreen transcript rows without starting network downloads.
/// The viewport requests only the sources it actually displays.
pub fn with_deferred_remote_images<T>(render: impl FnOnce() -> T) -> T {
    struct Guard(bool);
    impl Drop for Guard {
        fn drop(&mut self) {
            DEFER_REMOTE_IMAGES.set(self.0);
        }
    }
    let _guard = Guard(DEFER_REMOTE_IMAGES.replace(true));
    render()
}

pub fn request_remote_image(source: &str) {
    if let Ok(url) = url::Url::parse(source)
        && matches!(url.scheme(), "http" | "https")
    {
        let _ = remote_image::load(&url);
    }
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
        if DEFER_REMOTE_IMAGES.get() {
            remote_image::cached(&url)?
        } else {
            remote_image::load(&url)?
        }
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
