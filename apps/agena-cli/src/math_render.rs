use std::{
    cell::Cell,
    collections::{HashMap, VecDeque, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, RwLock},
};

use agena_tui_components::TerminalRgb;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use image::{DynamicImage, Rgba};
use ratatui::{
    Frame,
    layout::{Rect, Size},
};
use ratatui_image::{
    picker::{Capability, Picker, ProtocolType, cap_parser::QueryStdioOptions},
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::{color::Color, math_style::MathStyle};
use regex::Regex;
use rust_latex_parser::{EqNode, MatrixKind, parse_equation};

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
pub(crate) struct MathLayoutConfig {
    pub(crate) native_graphics: bool,
    pub(crate) cell_width: u16,
    pub(crate) cell_height: u16,
    pub(crate) foreground: [u8; 3],
    pub(crate) background: [u8; 3],
}

impl Default for MathLayoutConfig {
    fn default() -> Self {
        Self {
            native_graphics: false,
            cell_width: 10,
            cell_height: 20,
            foreground: [235, 235, 235],
            background: terminal_rgb_array(DEFAULT_DARK_BACKGROUND),
        }
    }
}

static LAYOUT_CONFIG: LazyLock<RwLock<MathLayoutConfig>> =
    LazyLock::new(|| RwLock::new(MathLayoutConfig::default()));
static GRAPHICS_CONFIG: LazyLock<RwLock<Option<MathGraphicsConfig>>> =
    LazyLock::new(|| RwLock::new(None));
static MARKDOWN_WORKSPACE: LazyLock<RwLock<Option<PathBuf>>> = LazyLock::new(|| RwLock::new(None));
thread_local! {
    /// Export and pager rendering cannot serialize terminal image placements.
    /// Keep that decision local to the rendering thread so generating a text
    /// transcript never changes the live terminal's negotiated protocol.
    static TEXT_MATH_RENDER_DEPTH: Cell<usize> = const { Cell::new(0) };
}
static LATEX_FRACTION_ALIAS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\(?:dfrac|tfrac|cfrac)\b").expect("valid LaTeX fraction alias regex")
});
static LATEX_UNICODE_ALIAS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\\(",
        r"odot|complement|mid|doteq|nearrow|nwarrow|searrow|swarrow|",
        r"updownarrow|longmapsto|llbracket|rrbracket|",
        r"dbinom|tbinom|operatorname|mathit|mathscr|widehat|widetilde|",
        r"bmod|pmod|lg|big|Big|bigg|Bigg|bigl|bigr|Bigl|Bigr|biggl|biggr|Biggl|Biggr",
        r")\b",
    ))
    .expect("valid LaTeX Unicode compatibility alias regex")
});
static LATEX_ROW_SPACING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\\\\\s*\[\s*[+-]?(?:\d+(?:\.\d*)?|\.\d+)\s*(?:pt|pc|in|bp|cm|mm|dd|cc|sp|em|ex|mu)\s*\]",
    )
    .expect("valid LaTeX row spacing regex")
});
static LATEX_ALIGNED_AT_BEGIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\begin\{alignedat\*?\}(?:\s*\{[^{}\r\n]*\})?")
        .expect("valid alignedat environment regex")
});
static LATEX_ARRAY_BEGIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\begin\{array\}(?:\s*\[[^\]\r\n]*\])?\s*\{[^{}\r\n]*\}")
        .expect("valid array environment regex")
});
static LATEX_MATRIX_LIKE_BEGIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\\begin\{(?:aligned|align\*?|gathered|gather\*?|split|multline\*?|equation\*?|smallmatrix)\}(?:\s*\[[^\]\r\n]*\])?",
    )
    .expect("valid matrix-like environment regex")
});
static LATEX_MATRIX_LIKE_END: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\\end\{(?:alignedat\*?|array|aligned|align\*?|gathered|gather\*?|split|multline\*?|equation\*?|smallmatrix)\}",
    )
    .expect("valid matrix-like environment end regex")
});
static LATEX_INDEXED_ROOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\sqrt\s*\[").expect("valid indexed root regex"));
static LATEX_COMMAND_DELIMITER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\(?:left|right|middle)\s*\\").expect("valid command delimiter regex")
});
static LATEX_ARROW_WITH_LOWER_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\\x(?:left|right)arrow\s*\[").expect("valid annotated arrow regex")
});

#[derive(Clone, Debug)]
pub(crate) struct MathGraphicsConfig {
    picker: Picker,
    background: Option<TerminalRgb>,
    background_was_reported: bool,
    downgraded_protocol: Option<ProtocolType>,
}

impl MathGraphicsConfig {
    pub(crate) fn query(background_hint: Option<TerminalRgb>, allow_native_graphics: bool) -> Self {
        // Capability probing sends cursor-position, device-attribute, graphics,
        // cell-size, and OSC colour queries. Through tmux, screen, SSH, or Mosh
        // those replies can be delayed until an external pager/editor owns the
        // tty, where they become visible input such as a stray trailing `R`.
        // TerminalRuntime has already made the transport policy decision, so
        // do not perform a query whose result cannot be used.
        let mut picker = picker_for_graphics_policy(allow_native_graphics, || {
            Picker::from_query_stdio_with_options(QueryStdioOptions {
                terminal_background_color_osc: true,
                ..QueryStdioOptions::default()
            })
            .unwrap_or_else(|_| Picker::halfblocks())
        });
        let reported_background = terminal_background_from_capabilities(picker.capabilities());
        let background = reported_background.or(background_hint);
        let resolved_background = background.unwrap_or(DEFAULT_DARK_BACKGROUND);
        let foreground = foreground_for_background(resolved_background);
        let selected_protocol = picker.protocol_type();
        let downgraded_protocol =
            downgraded_native_protocol(selected_protocol, allow_native_graphics);
        if downgraded_protocol.is_some() {
            picker.set_protocol_type(ProtocolType::Halfblocks);
        }
        picker.set_background_color(Some(Rgba([
            resolved_background.red,
            resolved_background.green,
            resolved_background.blue,
            255,
        ])));
        let font = picker.font_size();
        let config = MathLayoutConfig {
            native_graphics: picker.protocol_type() != ProtocolType::Halfblocks,
            cell_width: font.width.max(1),
            cell_height: font.height.max(1),
            foreground,
            background: terminal_rgb_array(resolved_background),
        };
        *LAYOUT_CONFIG
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
        let graphics = Self {
            picker,
            background,
            background_was_reported: reported_background.is_some(),
            downgraded_protocol,
        };
        *GRAPHICS_CONFIG
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(graphics.clone());
        graphics
    }

    pub(crate) fn is_native(&self) -> bool {
        self.picker.protocol_type() != ProtocolType::Halfblocks
    }

    pub(crate) fn protocol_name(&self) -> &'static str {
        protocol_name(self.picker.protocol_type())
    }

    pub(crate) const fn background(&self) -> Option<TerminalRgb> {
        self.background
    }

    pub(crate) const fn background_was_reported(&self) -> bool {
        self.background_was_reported
    }

    pub(crate) fn downgraded_protocol_name(&self) -> Option<&'static str> {
        self.downgraded_protocol.map(protocol_name)
    }
}

fn picker_for_graphics_policy(
    allow_native_graphics: bool,
    query: impl FnOnce() -> Picker,
) -> Picker {
    if allow_native_graphics {
        query()
    } else {
        Picker::halfblocks()
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

fn downgraded_native_protocol(
    selected_protocol: ProtocolType,
    allow_native_graphics: bool,
) -> Option<ProtocolType> {
    if selected_protocol != ProtocolType::Halfblocks && !allow_native_graphics {
        Some(selected_protocol)
    } else {
        None
    }
}

fn terminal_background_from_capabilities(capabilities: &[Capability]) -> Option<TerminalRgb> {
    capabilities.iter().find_map(|capability| match capability {
        Capability::Background(red, green, blue) => Some(TerminalRgb::new(*red, *green, *blue)),
        _ => None,
    })
}

const fn terminal_rgb_array(color: TerminalRgb) -> [u8; 3] {
    [color.red, color.green, color.blue]
}

pub(crate) fn configured_graphics() -> Option<MathGraphicsConfig> {
    GRAPHICS_CONFIG
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

pub(crate) fn layout_config() -> MathLayoutConfig {
    let config = *LAYOUT_CONFIG
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    apply_text_math_rendering_policy(config)
}

fn apply_text_math_rendering_policy(mut config: MathLayoutConfig) -> MathLayoutConfig {
    if TEXT_MATH_RENDER_DEPTH.with(|depth| depth.get() > 0) {
        config.native_graphics = false;
    }
    config
}

pub(crate) fn with_text_math_rendering<T>(render: impl FnOnce() -> T) -> T {
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

pub(crate) fn configure_markdown_workspace(workspace: &Path) {
    let workspace = fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    *MARKDOWN_WORKSPACE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(workspace);
}

#[derive(Debug)]
pub(crate) struct MathArtifact {
    pub(crate) id: u64,
    pub(crate) image: DynamicImage,
    pub(crate) size: Size,
}

#[derive(Debug, Clone)]
pub(crate) struct MathLinePlacement {
    pub(crate) column: u16,
    pub(crate) artifact: Arc<MathArtifact>,
    pub(crate) size: Size,
}

#[derive(Debug, Clone)]
pub(crate) struct TranscriptMathPlacement {
    pub(crate) line: usize,
    pub(crate) column: u16,
    pub(crate) artifact: Arc<MathArtifact>,
    pub(crate) size: Size,
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
    background: [u8; 3],
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

pub(crate) fn render_formula(source: &str, display: bool) -> Result<Arc<MathArtifact>, String> {
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
    config.background.hash(&mut hasher);
    let id = hasher.finish();
    if let Some(artifact) = ARTIFACT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
    {
        return Ok(artifact);
    }

    let ast = parse(source).map_err(|error| error.to_string())?;
    let foreground = Color::new(
        f32::from(config.foreground[0]) / 255.0,
        f32::from(config.foreground[1]) / 255.0,
        f32::from(config.foreground[2]) / 255.0,
        1.0,
    );
    let background = Color::new(
        f32::from(config.background[0]) / 255.0,
        f32::from(config.background[1]) / 255.0,
        f32::from(config.background[2]) / 255.0,
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
            // Give every formula a self-contained contrast pair. When OSC 11
            // succeeds this blends into the terminal; when all color evidence
            // is wrong, the formula remains readable on its own backing color.
            background_color: background,
            font_dir: String::new(),
            device_pixel_ratio: 1.0,
        },
    )?;
    let image = image::load_from_memory(&png).map_err(|error| error.to_string())?;
    if image.width() > MAX_IMAGE_DIMENSION || image.height() > MAX_IMAGE_DIMENSION {
        return Err("rendered formula exceeds the image safety limit".to_string());
    }
    let width = image
        .width()
        .div_ceil(u32::from(config.cell_width))
        .clamp(1, u32::from(u16::MAX)) as u16;
    let height = image
        .height()
        .div_ceil(u32::from(config.cell_height))
        .clamp(1, u32::from(u16::MAX)) as u16;
    let artifact = Arc::new(MathArtifact {
        id,
        image,
        size: Size::new(width, height),
    });
    ARTIFACT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(Arc::clone(&artifact));
    Ok(artifact)
}

/// Decodes a Markdown image without performing network I/O. Relative and `file:` URLs are
/// confined to the active workspace; `data:image/*;base64` URLs are accepted within the same
/// byte, dimension, and decoded-pixel limits used by the transcript graphics cache.
pub(crate) fn render_markdown_image(source: &str) -> Result<Arc<MathArtifact>, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err("image URL is empty".to_string());
    }
    let bytes = if source.starts_with("data:") {
        decode_data_image(source)?
    } else {
        read_workspace_image(source)?
    };
    if looks_like_svg(&bytes) {
        svg_artifact(&bytes)
    } else {
        image_artifact(&bytes)
    }
}

pub(crate) fn render_markdown_svg(source: &str) -> Result<Arc<MathArtifact>, String> {
    svg_artifact(source.as_bytes())
}

fn decode_data_image(source: &str) -> Result<Vec<u8>, String> {
    let (metadata, payload) = source
        .split_once(',')
        .ok_or_else(|| "invalid image data URL".to_string())?;
    if !metadata
        .get("data:".len()..)
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("image/"))
        || !metadata.to_ascii_lowercase().ends_with(";base64")
    {
        return Err("only base64-encoded image data URLs are supported".to_string());
    }
    let estimated_len = payload.len().saturating_mul(3).div_ceil(4);
    if estimated_len > MAX_MARKDOWN_IMAGE_BYTES {
        return Err("image exceeds the encoded byte safety limit".to_string());
    }
    let bytes = BASE64_STANDARD
        .decode(payload)
        .map_err(|_| "invalid base64 image data".to_string())?;
    Ok(bytes)
}

fn read_workspace_image(source: &str) -> Result<Vec<u8>, String> {
    if let Ok(url) = url::Url::parse(source) {
        match url.scheme() {
            "http" | "https" => {
                return Err("remote images are not loaded automatically".to_string());
            }
            "file" => {}
            _ => return Err("unsupported image URL scheme".to_string()),
        }
    }
    let workspace = MARKDOWN_WORKSPACE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or_else(|| "workspace image loading is not configured".to_string())?;
    let base_url = url::Url::from_directory_path(&workspace)
        .map_err(|()| "workspace path cannot be represented as a file URL".to_string())?;
    let url = base_url
        .join(source)
        .map_err(|_| "invalid image URL".to_string())?;
    match url.scheme() {
        "http" | "https" => {
            return Err("remote images are not loaded automatically".to_string());
        }
        "file" => {}
        _ => return Err("unsupported image URL scheme".to_string()),
    }
    let path = url
        .to_file_path()
        .map_err(|()| "invalid local image path".to_string())?;
    let path = fs::canonicalize(path).map_err(|error| format!("cannot open image: {error}"))?;
    if !path.starts_with(&workspace) {
        return Err("local image is outside the active workspace".to_string());
    }
    let metadata = fs::metadata(&path).map_err(|error| format!("cannot inspect image: {error}"))?;
    if !metadata.is_file() {
        return Err("local image is not a regular file".to_string());
    }
    if metadata.len() > MAX_MARKDOWN_IMAGE_BYTES as u64 {
        return Err("image exceeds the encoded byte safety limit".to_string());
    }
    let bytes = fs::read(path).map_err(|error| format!("cannot read image: {error}"))?;
    Ok(bytes)
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let prefix = bytes
        .get(..bytes.len().min(4096))
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or_default()
        .trim_start_matches('\u{feff}')
        .trim_start();
    prefix.starts_with("<svg")
        || prefix.starts_with("<?xml") && prefix.to_ascii_lowercase().contains("<svg")
}

fn validate_encoded_image_size(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_MARKDOWN_IMAGE_BYTES {
        return Err("image exceeds the encoded byte safety limit".to_string());
    }
    let dimensions =
        imagesize::blob_size(bytes).map_err(|_| "unsupported image data".to_string())?;
    let width = u64::try_from(dimensions.width).unwrap_or(u64::MAX);
    let height = u64::try_from(dimensions.height).unwrap_or(u64::MAX);
    if width > u64::from(MAX_IMAGE_DIMENSION) || height > u64::from(MAX_IMAGE_DIMENSION) {
        return Err("image dimensions exceed the safety limit".to_string());
    }
    if width.saturating_mul(height) > MAX_IMAGE_PIXELS {
        return Err("image decoded pixels exceed the safety limit".to_string());
    }
    Ok(())
}

fn image_artifact(bytes: &[u8]) -> Result<Arc<MathArtifact>, String> {
    validate_encoded_image_size(bytes)?;
    let config = layout_config();
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    config.cell_width.hash(&mut hasher);
    config.cell_height.hash(&mut hasher);
    let id = hasher.finish();
    if let Some(artifact) = ARTIFACT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
    {
        return Ok(artifact);
    }
    let image = image::load_from_memory(bytes).map_err(|error| error.to_string())?;
    cache_dynamic_image(id, image, config)
}

fn svg_artifact(bytes: &[u8]) -> Result<Arc<MathArtifact>, String> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err("SVG exceeds the encoded byte safety limit".to_string());
    }
    let config = layout_config();
    let mut hasher = DefaultHasher::new();
    b"svg".hash(&mut hasher);
    bytes.hash(&mut hasher);
    config.cell_width.hash(&mut hasher);
    config.cell_height.hash(&mut hasher);
    let id = hasher.finish();
    if let Some(artifact) = ARTIFACT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
    {
        return Ok(artifact);
    }

    static FONT_DATABASE: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
        let mut database = resvg::usvg::fontdb::Database::new();
        database.load_system_fonts();
        Arc::new(database)
    });
    let options = resvg::usvg::Options {
        fontdb: Arc::clone(&FONT_DATABASE),
        resources_dir: None,
        ..resvg::usvg::Options::default()
    };
    let tree = resvg::usvg::Tree::from_data_nested(bytes, &options)
        .map_err(|error| format!("invalid SVG: {error}"))?;
    let size = tree.size();
    let width = size.width().ceil();
    let height = size.height().ceil();
    if !width.is_finite()
        || !height.is_finite()
        || width < 1.0
        || height < 1.0
        || width > MAX_IMAGE_DIMENSION as f32
        || height > MAX_IMAGE_DIMENSION as f32
        || (width as u64).saturating_mul(height as u64) > MAX_IMAGE_PIXELS
    {
        return Err("SVG dimensions exceed the safety limit".to_string());
    }
    let width = width as u32;
    let height = height as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| "cannot allocate SVG raster surface".to_string())?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel =
                    u8::try_from((u32::from(*channel) * 255 / alpha).min(255)).unwrap_or(255);
            }
        }
    }
    let image = image::RgbaImage::from_raw(width, height, rgba)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| "invalid SVG raster buffer".to_string())?;
    cache_dynamic_image(id, image, config)
}

fn cache_dynamic_image(
    id: u64,
    image: DynamicImage,
    config: MathLayoutConfig,
) -> Result<Arc<MathArtifact>, String> {
    let width = image
        .width()
        .div_ceil(u32::from(config.cell_width))
        .clamp(1, u32::from(u16::MAX)) as u16;
    let height = image
        .height()
        .div_ceil(u32::from(config.cell_height))
        .clamp(1, u32::from(u16::MAX)) as u16;
    let artifact = Arc::new(MathArtifact {
        id,
        image,
        size: Size::new(width, height),
    });
    ARTIFACT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(Arc::clone(&artifact));
    Ok(artifact)
}

pub(crate) fn unicode_formula(source: &str, display: bool) -> Vec<String> {
    let source = source.trim();
    if source.is_empty() {
        return Vec::new();
    }
    if source.len() > MAX_FORMULA_BYTES || latex_nesting_exceeds_limit(source) {
        return explicit_latex_fallback(source);
    }

    let config = layout_config();
    let key = UnicodeMathCacheKey {
        source: source.to_string(),
        display,
        cell_width: config.cell_width,
        cell_height: config.cell_height,
        foreground: config.foreground,
        background: config.background,
    };
    if let Some(lines) = UNICODE_MATH_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        return lines.as_ref().clone();
    }

    let normalized = normalize_unicode_latex(source);
    let parsed = parse_equation(&normalized);
    let lines = if !source_requires_source_fallback(&normalized) && math_ast_within_limits(&parsed)
    {
        let ast = normalize_unicode_math_ast(parsed);
        if math_ast_is_supported(&ast) {
            bounded_semantic_unicode(&ast, display)
                .unwrap_or_else(|| explicit_latex_fallback(source))
        } else {
            explicit_latex_fallback(source)
        }
    } else {
        explicit_latex_fallback(source)
    };

    UNICODE_MATH_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, Arc::new(lines.clone()));
    lines
}

fn source_requires_source_fallback(source: &str) -> bool {
    LATEX_INDEXED_ROOT.is_match(source)
        || LATEX_COMMAND_DELIMITER.is_match(source)
        || LATEX_ARROW_WITH_LOWER_LABEL.is_match(source)
}

fn bounded_semantic_unicode(ast: &EqNode, display: bool) -> Option<Vec<String>> {
    let mut lines = term_maths::layout::layout(ast)
        .to_string()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !display
        && lines.len() > 1
        && let Some(compact) = compact_inline_math(ast)
    {
        lines = vec![compact];
    }
    let width = lines
        .iter()
        .map(|line| unicode_width::UnicodeWidthStr::width(line.as_str()))
        .max()
        .unwrap_or(0);
    let cells = width.saturating_mul(lines.len());
    (!lines.is_empty()
        && width <= MAX_UNICODE_GRID_WIDTH
        && lines.len() <= MAX_UNICODE_GRID_HEIGHT
        && cells <= MAX_UNICODE_GRID_CELLS)
        .then_some(lines)
}

fn compact_inline_math(node: &EqNode) -> Option<String> {
    let rendered = term_maths::layout::layout(node).to_string();
    if !rendered.contains('\n')
        && unicode_width::UnicodeWidthStr::width(rendered.as_str()) <= MAX_UNICODE_GRID_WIDTH
    {
        return Some(rendered);
    }
    let compact = match node {
        EqNode::Text(text) | EqNode::TextBlock(text) => text.clone(),
        EqNode::Space(points) => {
            if *points > 0.0 {
                " ".to_string()
            } else {
                String::new()
            }
        }
        EqNode::Seq(nodes) => nodes
            .iter()
            .map(compact_inline_math)
            .collect::<Option<Vec<_>>>()?
            .join(""),
        EqNode::Sup(base, upper) => format!(
            "{}{}",
            compact_inline_atom(base)?,
            compact_script(upper, true)?
        ),
        EqNode::Sub(base, lower) => format!(
            "{}{}",
            compact_inline_atom(base)?,
            compact_script(lower, false)?
        ),
        EqNode::SupSub(base, upper, lower) => format!(
            "{}{}{}",
            compact_inline_atom(base)?,
            compact_script(upper, true)?,
            compact_script(lower, false)?
        ),
        EqNode::Frac(numerator, denominator) => format!(
            "{}⁄{}",
            compact_inline_atom(numerator)?,
            compact_inline_atom(denominator)?
        ),
        EqNode::Sqrt(body) => format!("√{}", compact_inline_atom(body)?),
        EqNode::BigOp {
            symbol,
            lower,
            upper,
        } => {
            let mut value = symbol.clone();
            if let Some(upper) = upper {
                value.push_str(&compact_script(upper, true)?);
            }
            if let Some(lower) = lower {
                value.push_str(&compact_script(lower, false)?);
            }
            value
        }
        EqNode::Limit { name, lower } => {
            let mut value = name.clone();
            if let Some(lower) = lower {
                value.push_str(&compact_script(lower, false)?);
            }
            value
        }
        EqNode::MathFont { .. } | EqNode::Accent(_, _) => return None,
        EqNode::Delimited {
            left,
            right,
            content,
        } => format!("{left}{}{right}", compact_inline_math(content)?),
        EqNode::Matrix { .. }
        | EqNode::Cases { .. }
        | EqNode::Binom(_, _)
        | EqNode::Brace { .. }
        | EqNode::StackRel { .. } => return None,
    };
    (unicode_width::UnicodeWidthStr::width(compact.as_str()) <= MAX_UNICODE_GRID_WIDTH)
        .then_some(compact)
}

fn compact_inline_atom(node: &EqNode) -> Option<String> {
    let value = compact_inline_math(node)?;
    if matches!(node, EqNode::Seq(nodes) if nodes.len() > 1) {
        Some(format!("({value})"))
    } else {
        Some(value)
    }
}

fn compact_script(node: &EqNode, superscript: bool) -> Option<String> {
    let value = compact_inline_math(node)?;
    let mapped = positional_unicode_text(&value, superscript);
    mapped.or_else(|| {
        Some(if superscript {
            format!("^({value})")
        } else {
            format!("_({value})")
        })
    })
}

pub(crate) fn positional_unicode_text(source: &str, superscript: bool) -> Option<String> {
    source
        .chars()
        .map(|ch| positional_unicode_char(ch, superscript))
        .collect()
}

fn positional_unicode_char(ch: char, superscript: bool) -> Option<char> {
    Some(if superscript {
        match ch {
            '0' => '⁰',
            '1' => '¹',
            '2' => '²',
            '3' => '³',
            '4' => '⁴',
            '5' => '⁵',
            '6' => '⁶',
            '7' => '⁷',
            '8' => '⁸',
            '9' => '⁹',
            '+' => '⁺',
            '-' => '⁻',
            '=' => '⁼',
            '(' => '⁽',
            ')' => '⁾',
            'a' => 'ᵃ',
            'b' => 'ᵇ',
            'c' => 'ᶜ',
            'd' => 'ᵈ',
            'e' => 'ᵉ',
            'f' => 'ᶠ',
            'g' => 'ᵍ',
            'h' => 'ʰ',
            'i' => 'ⁱ',
            'j' => 'ʲ',
            'k' => 'ᵏ',
            'l' => 'ˡ',
            'm' => 'ᵐ',
            'n' => 'ⁿ',
            'o' => 'ᵒ',
            'p' => 'ᵖ',
            'r' => 'ʳ',
            's' => 'ˢ',
            't' => 'ᵗ',
            'u' => 'ᵘ',
            'v' => 'ᵛ',
            'w' => 'ʷ',
            'x' => 'ˣ',
            'y' => 'ʸ',
            'z' => 'ᶻ',
            _ => return None,
        }
    } else {
        match ch {
            '0' => '₀',
            '1' => '₁',
            '2' => '₂',
            '3' => '₃',
            '4' => '₄',
            '5' => '₅',
            '6' => '₆',
            '7' => '₇',
            '8' => '₈',
            '9' => '₉',
            '+' => '₊',
            '-' => '₋',
            '=' => '₌',
            '(' => '₍',
            ')' => '₎',
            'a' => 'ₐ',
            'e' => 'ₑ',
            'h' => 'ₕ',
            'i' => 'ᵢ',
            'j' => 'ⱼ',
            'k' => 'ₖ',
            'l' => 'ₗ',
            'm' => 'ₘ',
            'n' => 'ₙ',
            'o' => 'ₒ',
            'p' => 'ₚ',
            'r' => 'ᵣ',
            's' => 'ₛ',
            't' => 'ₜ',
            'u' => 'ᵤ',
            'v' => 'ᵥ',
            'x' => 'ₓ',
            _ => return None,
        }
    })
}

fn latex_nesting_exceeds_limit(source: &str) -> bool {
    let mut depth = 0_usize;
    let mut escaped = false;
    for ch in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '{' | '[' => {
                depth = depth.saturating_add(1);
                if depth > MAX_MATH_NESTING {
                    return true;
                }
            }
            '}' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

fn math_ast_within_limits(root: &EqNode) -> bool {
    let mut stack = vec![(root, 1_usize)];
    let mut count = 0_usize;
    while let Some((node, depth)) = stack.pop() {
        count = count.saturating_add(1);
        if count > MAX_MATH_AST_NODES || depth > MAX_MATH_NESTING {
            return false;
        }
        let next_depth = depth.saturating_add(1);
        match node {
            EqNode::Seq(nodes) => stack.extend(nodes.iter().map(|node| (node, next_depth))),
            EqNode::Sup(base, script)
            | EqNode::Sub(base, script)
            | EqNode::Frac(base, script)
            | EqNode::Binom(base, script) => {
                stack.push((base, next_depth));
                stack.push((script, next_depth));
            }
            EqNode::SupSub(base, upper, lower) => {
                stack.push((base, next_depth));
                stack.push((upper, next_depth));
                stack.push((lower, next_depth));
            }
            EqNode::Sqrt(body) | EqNode::Accent(body, _) => stack.push((body, next_depth)),
            EqNode::BigOp { lower, upper, .. } => {
                if let Some(lower) = lower {
                    stack.push((lower, next_depth));
                }
                if let Some(upper) = upper {
                    stack.push((upper, next_depth));
                }
            }
            EqNode::Limit { lower, .. } => {
                if let Some(lower) = lower {
                    stack.push((lower, next_depth));
                }
            }
            EqNode::MathFont { content, .. } | EqNode::Delimited { content, .. } => {
                stack.push((content, next_depth));
            }
            EqNode::Matrix { rows, .. } => {
                if rows.len() > 256 || rows.iter().map(Vec::len).sum::<usize>() > 1_024 {
                    return false;
                }
                for cell in rows.iter().flatten() {
                    stack.push((cell, next_depth));
                }
            }
            EqNode::Cases { rows } => {
                if rows.len() > 256 {
                    return false;
                }
                for (value, condition) in rows {
                    stack.push((value, next_depth));
                    if let Some(condition) = condition {
                        stack.push((condition, next_depth));
                    }
                }
            }
            EqNode::Brace { content, label, .. } => {
                stack.push((content, next_depth));
                if let Some(label) = label {
                    stack.push((label, next_depth));
                }
            }
            EqNode::StackRel {
                base, annotation, ..
            } => {
                stack.push((base, next_depth));
                stack.push((annotation, next_depth));
            }
            EqNode::Text(_) | EqNode::TextBlock(_) | EqNode::Space(_) => {}
        }
    }
    true
}

fn math_ast_is_supported(root: &EqNode) -> bool {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            EqNode::Text(text) | EqNode::TextBlock(text) if contains_raw_latex_markup(text) => {
                return false;
            }
            EqNode::Seq(nodes) => stack.extend(nodes),
            EqNode::Sup(base, script)
            | EqNode::Sub(base, script)
            | EqNode::Frac(base, script)
            | EqNode::Binom(base, script) => {
                stack.push(base);
                stack.push(script);
            }
            EqNode::SupSub(base, upper, lower) => {
                stack.push(base);
                stack.push(upper);
                stack.push(lower);
            }
            EqNode::Sqrt(body) | EqNode::Accent(body, _) => stack.push(body),
            EqNode::BigOp { lower, upper, .. } => {
                stack.extend(lower.iter().map(Box::as_ref));
                stack.extend(upper.iter().map(Box::as_ref));
            }
            EqNode::Limit { lower, .. } => stack.extend(lower.iter().map(Box::as_ref)),
            EqNode::MathFont { content, .. } | EqNode::Delimited { content, .. } => {
                stack.push(content)
            }
            EqNode::Matrix { rows, .. } => stack.extend(rows.iter().flatten()),
            EqNode::Cases { rows } => {
                for (value, condition) in rows {
                    stack.push(value);
                    stack.extend(condition.iter());
                }
            }
            EqNode::Brace { content, label, .. } => {
                stack.push(content);
                stack.extend(label.iter().map(Box::as_ref));
            }
            EqNode::StackRel {
                base, annotation, ..
            } => {
                stack.push(base);
                stack.push(annotation);
            }
            EqNode::Text(_) | EqNode::TextBlock(_) | EqNode::Space(_) => {}
        }
    }
    true
}

fn contains_raw_latex_markup(text: &str) -> bool {
    text.contains('\\')
}

fn explicit_latex_fallback(source: &str) -> Vec<String> {
    let compact = source.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = compact
        .chars()
        .take(MAX_EXPLICIT_FALLBACK_CHARS)
        .collect::<String>();
    if compact.chars().count() > MAX_EXPLICIT_FALLBACK_CHARS {
        preview.push('…');
    }
    vec![format!("⟦LaTeX: {preview}⟧")]
}

/// Adapt standard LaTeX accepted by the native RaTeX renderer to the smaller
/// command surface understood by the Unicode terminal fallback.
fn normalize_unicode_latex(source: &str) -> String {
    let source = LATEX_FRACTION_ALIAS.replace_all(source, r"\frac");
    let source = LATEX_UNICODE_ALIAS.replace_all(&source, |captures: &regex::Captures<'_>| {
        match captures.get(1).map(|value| value.as_str()) {
            Some("odot") => "⊙",
            Some("complement") => "∁",
            Some("mid") => "∣",
            Some("doteq") => "≐",
            Some("nearrow") => "↗",
            Some("nwarrow") => "↖",
            Some("searrow") => "↘",
            Some("swarrow") => "↙",
            Some("updownarrow") => "↕",
            Some("longmapsto") => "⟼",
            Some("llbracket") => "⟦",
            Some("rrbracket") => "⟧",
            Some("dbinom" | "tbinom") => r"\binom",
            Some("operatorname" | "mathit") => r"\mathrm",
            Some("mathscr") => r"\mathcal",
            Some("widehat") => r"\hat",
            Some("widetilde") => r"\tilde",
            Some("bmod") => "mod",
            Some("pmod") => "mod ",
            Some("lg") => r"\log",
            Some(
                "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl"
                | "biggr" | "Biggl" | "Biggr",
            ) => "",
            Some(_) | None => unreachable!("regex only captures known compatibility aliases"),
        }
    });
    let source = LATEX_ROW_SPACING.replace_all(&source, r"\\");
    let source = LATEX_ALIGNED_AT_BEGIN.replace_all(&source, r"\begin{matrix}");
    let source = LATEX_ARRAY_BEGIN.replace_all(&source, r"\begin{matrix}");
    let source = LATEX_MATRIX_LIKE_BEGIN.replace_all(&source, r"\begin{matrix}");
    let source = LATEX_MATRIX_LIKE_END.replace_all(&source, r"\end{matrix}");
    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_unicode_math_ast(node: EqNode) -> EqNode {
    match node {
        EqNode::Text(text) => unicode_big_operator(&text).map_or_else(
            || {
                unicode_terminal_symbol(&text).map_or(EqNode::Text(text), |symbol| {
                    EqNode::Text(symbol.to_string())
                })
            },
            |symbol| EqNode::BigOp {
                symbol: symbol.to_string(),
                lower: None,
                upper: None,
            },
        ),
        EqNode::Space(_) | EqNode::TextBlock(_) => node,
        EqNode::Seq(nodes) => normalize_unicode_sequence(nodes),
        EqNode::Sup(base, upper) => attach_big_operator_limits(
            normalize_unicode_math_ast(*base),
            Some(normalize_unicode_math_ast(*upper)),
            None,
        ),
        EqNode::Sub(base, lower) => attach_big_operator_limits(
            normalize_unicode_math_ast(*base),
            None,
            Some(normalize_unicode_math_ast(*lower)),
        ),
        EqNode::SupSub(base, upper, lower) => attach_big_operator_limits(
            normalize_unicode_math_ast(*base),
            Some(normalize_unicode_math_ast(*upper)),
            Some(normalize_unicode_math_ast(*lower)),
        ),
        EqNode::Frac(numerator, denominator) => EqNode::Frac(
            Box::new(normalize_unicode_math_ast(*numerator)),
            Box::new(normalize_unicode_math_ast(*denominator)),
        ),
        EqNode::Sqrt(body) => EqNode::Sqrt(Box::new(normalize_unicode_math_ast(*body))),
        EqNode::BigOp {
            symbol,
            lower,
            upper,
        } => EqNode::BigOp {
            symbol,
            lower: lower.map(|node| Box::new(normalize_unicode_math_ast(*node))),
            upper: upper.map(|node| Box::new(normalize_unicode_math_ast(*node))),
        },
        EqNode::Accent(body, kind) => {
            EqNode::Accent(Box::new(normalize_unicode_math_ast(*body)), kind)
        }
        EqNode::Limit { name, lower } => EqNode::Limit {
            name,
            lower: lower.map(|node| Box::new(normalize_unicode_math_ast(*node))),
        },
        EqNode::MathFont { kind, content } => EqNode::MathFont {
            kind,
            content: Box::new(normalize_unicode_math_ast(*content)),
        },
        EqNode::Delimited {
            left,
            right,
            content,
        } => EqNode::Delimited {
            left,
            right,
            content: Box::new(normalize_unicode_math_ast(*content)),
        },
        EqNode::Matrix { kind, rows } => EqNode::Matrix {
            kind,
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(normalize_unicode_math_ast).collect())
                .collect(),
        },
        // term-maths inserts the English word "if" between both cases
        // columns. LaTeX does not imply that word, so preserve the source
        // columns in a brace-delimited plain matrix instead.
        EqNode::Cases { rows } => EqNode::Delimited {
            left: "{".to_string(),
            right: String::new(),
            content: Box::new(EqNode::Matrix {
                kind: MatrixKind::Plain,
                rows: rows
                    .into_iter()
                    .map(|(value, condition)| {
                        let mut row = vec![normalize_unicode_math_ast(value)];
                        if let Some(condition) = condition {
                            row.push(normalize_unicode_math_ast(condition));
                        }
                        row
                    })
                    .collect(),
            }),
        },
        // term-maths 1.0 gives Binom's two-row body and its delimiters
        // different baselines, which shifts the bottom argument outside the
        // parentheses. A one-column parenthesized matrix has the same visual
        // semantics and keeps every row on the same baseline.
        EqNode::Binom(top, bottom) => EqNode::Matrix {
            kind: MatrixKind::Paren,
            rows: vec![
                vec![normalize_unicode_math_ast(*top)],
                vec![normalize_unicode_math_ast(*bottom)],
            ],
        },
        EqNode::Brace {
            content,
            label,
            over,
        } => EqNode::Brace {
            content: Box::new(normalize_unicode_math_ast(*content)),
            label: label.map(|node| Box::new(normalize_unicode_math_ast(*node))),
            over,
        },
        EqNode::StackRel {
            base,
            annotation,
            over,
        } => EqNode::StackRel {
            base: Box::new(normalize_unicode_math_ast(*base)),
            annotation: Box::new(normalize_unicode_math_ast(*annotation)),
            over,
        },
    }
}

fn normalize_unicode_sequence(nodes: Vec<EqNode>) -> EqNode {
    let mut nodes = nodes.into_iter().map(normalize_unicode_math_ast).peekable();
    let mut normalized = Vec::new();
    while let Some(node) = nodes.next() {
        let command = match &node {
            EqNode::Text(text) => Some(text.as_str()),
            _ => None,
        };
        match command {
            Some(r"\xrightarrow" | r"\xleftarrow") => {
                let Some(annotation) = nodes.next() else {
                    normalized.push(node);
                    continue;
                };
                normalized.push(EqNode::StackRel {
                    base: Box::new(EqNode::Text(
                        if command == Some(r"\xrightarrow") {
                            "→"
                        } else {
                            "←"
                        }
                        .to_string(),
                    )),
                    annotation: Box::new(annotation),
                    over: true,
                });
            }
            Some(r"\pmod" | r"\pod") => {
                let Some(modulus) = nodes.next() else {
                    normalized.push(node);
                    continue;
                };
                normalized.push(EqNode::Seq(vec![
                    EqNode::Text("  (mod ".to_string()),
                    modulus,
                    EqNode::Text(")".to_string()),
                ]));
            }
            Some(r"\tag") => {
                if let Some(tag) = nodes.next() {
                    normalized.push(EqNode::Seq(vec![
                        EqNode::Text("  (".to_string()),
                        tag,
                        EqNode::Text(")".to_string()),
                    ]));
                }
            }
            Some(r"\label") => {
                let _ = nodes.next();
            }
            _ => normalized.push(node),
        }
    }
    match normalized.len() {
        0 => EqNode::Text(String::new()),
        1 => normalized.pop().expect("single normalized node exists"),
        _ => EqNode::Seq(normalized),
    }
}

fn attach_big_operator_limits(
    base: EqNode,
    upper: Option<EqNode>,
    lower: Option<EqNode>,
) -> EqNode {
    match base {
        EqNode::BigOp {
            symbol,
            lower: existing_lower,
            upper: existing_upper,
        } if existing_upper.is_none() && existing_lower.is_none() => EqNode::BigOp {
            symbol,
            lower: lower.map(Box::new),
            upper: upper.map(Box::new),
        },
        base => match (upper, lower) {
            (Some(upper), Some(lower)) => {
                EqNode::SupSub(Box::new(base), Box::new(upper), Box::new(lower))
            }
            (Some(upper), None) => EqNode::Sup(Box::new(base), Box::new(upper)),
            (None, Some(lower)) => EqNode::Sub(Box::new(base), Box::new(lower)),
            (None, None) => base,
        },
    }
}

fn unicode_big_operator(command: &str) -> Option<&'static str> {
    match command {
        r"\bigcap" => Some("⋂"),
        r"\bigcup" => Some("⋃"),
        r"\bigwedge" => Some("⋀"),
        r"\bigvee" => Some("⋁"),
        r"\bigsqcup" => Some("⨆"),
        r"\bigodot" => Some("⨀"),
        r"\bigoplus" => Some("⨁"),
        r"\bigotimes" => Some("⨂"),
        r"\biguplus" => Some("⨄"),
        r"\iiiint" => Some("⨌"),
        r"\oiint" => Some("∯"),
        r"\oiiint" => Some("∰"),
        _ => None,
    }
}

fn unicode_terminal_symbol(command: &str) -> Option<&'static str> {
    match command {
        r"\blacksquare" | r"\qed" | r"\QED" => Some("∎"),
        r"\square" => Some("□"),
        r"\mod" | r"\bmod" => Some(" mod "),
        r"\displaystyle"
        | r"\textstyle"
        | r"\scriptstyle"
        | r"\scriptscriptstyle"
        | r"\limits"
        | r"\nolimits"
        | r"\notag"
        | r"\nonumber" => Some(""),
        _ => None,
    }
}

pub(crate) struct MathGraphicsRenderer {
    picker: Picker,
    protocols: HashMap<(u64, u16, u16), SlicedProtocol>,
    generation: u64,
}

impl MathGraphicsRenderer {
    pub(crate) fn new(config: MathGraphicsConfig) -> Option<Self> {
        config.is_native().then(|| Self {
            picker: config.picker,
            protocols: HashMap::new(),
            generation: 0,
        })
    }

    pub(crate) fn sync_generation(&mut self, generation: u64) {
        if self.generation != generation {
            self.generation = generation;
            self.protocols.clear();
        }
    }

    pub(crate) fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        scroll: usize,
        placements: &[TranscriptMathPlacement],
    ) {
        for placement in placements {
            let y = placement.line as i64 - scroll as i64;
            if y >= i64::from(area.height) || y + i64::from(placement.artifact.size.height) <= 0 {
                continue;
            }
            let key = (
                placement.artifact.id,
                placement.size.width,
                placement.size.height,
            );
            if !self.protocols.contains_key(&key) {
                if self.protocols.len() >= MAX_PROTOCOLS {
                    self.protocols.clear();
                }
                let Ok(protocol) = SlicedProtocol::new(
                    &self.picker,
                    placement.artifact.image.clone(),
                    Some(placement.size),
                ) else {
                    continue;
                };
                self.protocols.insert(key, protocol);
            }
            let Some(protocol) = self.protocols.get(&key) else {
                continue;
            };
            let x = i16::try_from(placement.column).unwrap_or(i16::MAX);
            let y = i16::try_from(y).unwrap_or(if y < 0 { i16::MIN } else { i16::MAX });
            frame.render_widget(
                SlicedImage::new(protocol, SignedPosition::from((x, y))),
                area,
            );
        }
    }
}

fn foreground_for_background(background: TerminalRgb) -> [u8; 3] {
    if background.is_light() {
        [28, 28, 28]
    } else {
        [235, 235, 235]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            pixels.pixels().all(|pixel| pixel[3] == 255),
            "formula backing must be opaque on every graphics protocol"
        );
        let (darkest, brightest) = pixels.pixels().fold((u8::MAX, u8::MIN), |range, pixel| {
            let luminance =
                ((u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2])) / 3) as u8;
            (range.0.min(luminance), range.1.max(luminance))
        });
        assert!(
            brightest.saturating_sub(darkest) >= 128,
            "formula ink must contrast with its backing color"
        );
    }

    #[test]
    fn terminal_background_query_drives_formula_contrast() {
        let capabilities = vec![Capability::Kitty, Capability::Background(248, 249, 250)];
        let background = terminal_background_from_capabilities(&capabilities)
            .expect("OSC 11 response should be retained");
        assert_eq!(background, TerminalRgb::new(248, 249, 250));
        assert_eq!(foreground_for_background(background), [28, 28, 28]);
        assert_eq!(
            foreground_for_background(TerminalRgb::new(18, 18, 20)),
            [235, 235, 235]
        );
    }

    #[test]
    fn layered_transports_can_disable_a_selected_native_protocol() {
        assert_eq!(
            downgraded_native_protocol(ProtocolType::Kitty, false),
            Some(ProtocolType::Kitty)
        );
        assert_eq!(downgraded_native_protocol(ProtocolType::Kitty, true), None);
        assert_eq!(
            downgraded_native_protocol(ProtocolType::Halfblocks, false),
            None
        );
    }

    #[test]
    fn disabled_native_graphics_never_queries_the_terminal() {
        let queried = std::cell::Cell::new(false);
        let picker = picker_for_graphics_policy(false, || {
            queried.set(true);
            Picker::halfblocks()
        });

        assert!(!queried.get());
        assert_eq!(picker.protocol_type(), ProtocolType::Halfblocks);

        let picker = picker_for_graphics_policy(true, || {
            queried.set(true);
            Picker::halfblocks()
        });
        assert!(queried.get());
        assert_eq!(picker.protocol_type(), ProtocolType::Halfblocks);
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
        assert!(
            !contains_raw_latex_markup(&rendered),
            "raw LaTeX leaked:\n{rendered}"
        );
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
    fn unsupported_semantic_math_uses_a_bounded_source_fallback() {
        let rendered = unicode_formula(r"\operatorname{rank}(A)=\sqrt[3]{8}", true).join("\n");
        assert!(
            rendered.starts_with("⟦LaTeX: "),
            "unsupported semantic nodes should retain readable source:\n{rendered}"
        );
        assert!(
            rendered.contains(r"\operatorname{rank}(A)=\sqrt[3]{8}"),
            "fallback lost the original formula:\n{rendered}"
        );
        assert!(
            !rendered
                .chars()
                .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
            "math fallback must never emit Braille raster cells:\n{rendered}"
        );

        let repeated = unicode_formula(r"\operatorname{rank}(A)=\sqrt[3]{8}", true).join("\n");
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
    fn markdown_images_never_fetch_remote_urls() {
        let error = render_markdown_image("https://example.com/tracker.png")
            .expect_err("remote image must stay inert");
        assert!(error.contains("not loaded automatically"));
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
