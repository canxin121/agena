use std::{
    collections::{HashMap, VecDeque, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex, RwLock},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use image::DynamicImage;
use ratatui::{
    Frame,
    layout::{Rect, Size},
};
use ratatui_image::{
    picker::{Picker, ProtocolType},
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};
use ratex_layout::{LayoutOptions, layout, to_display_list};
use ratex_parser::parser::parse;
use ratex_render::{RenderOptions, render_to_png};
use ratex_types::{color::Color, math_style::MathStyle};

const MAX_FORMULA_BYTES: usize = 16 * 1024;
const MAX_MARKDOWN_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_ARTIFACTS: usize = 128;
const MAX_CACHED_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_PROTOCOLS: usize = 256;

#[derive(Debug, Clone, Copy)]
pub(crate) struct MathLayoutConfig {
    pub(crate) native_graphics: bool,
    pub(crate) cell_width: u16,
    pub(crate) cell_height: u16,
    pub(crate) foreground: [u8; 3],
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

static LAYOUT_CONFIG: LazyLock<RwLock<MathLayoutConfig>> =
    LazyLock::new(|| RwLock::new(MathLayoutConfig::default()));
static GRAPHICS_CONFIG: LazyLock<RwLock<Option<MathGraphicsConfig>>> =
    LazyLock::new(|| RwLock::new(None));
static MARKDOWN_WORKSPACE: LazyLock<RwLock<Option<PathBuf>>> = LazyLock::new(|| RwLock::new(None));

#[derive(Clone, Debug)]
pub(crate) struct MathGraphicsConfig {
    picker: Picker,
}

impl MathGraphicsConfig {
    pub(crate) fn query(foreground: [u8; 3]) -> Self {
        let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
        let font = picker.font_size();
        let config = MathLayoutConfig {
            native_graphics: picker.protocol_type() != ProtocolType::Halfblocks,
            cell_width: font.width.max(1),
            cell_height: font.height.max(1),
            foreground,
        };
        *LAYOUT_CONFIG
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
        let graphics = Self { picker };
        *GRAPHICS_CONFIG
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(graphics.clone());
        graphics
    }

    pub(crate) fn is_native(&self) -> bool {
        self.picker.protocol_type() != ProtocolType::Halfblocks
    }

    pub(crate) fn protocol_name(&self) -> &'static str {
        match self.picker.protocol_type() {
            ProtocolType::Kitty => "kitty",
            ProtocolType::Sixel => "sixel",
            ProtocolType::Iterm2 => "iterm2",
            ProtocolType::Halfblocks => "unicode",
        }
    }
}

pub(crate) fn configured_graphics() -> Option<MathGraphicsConfig> {
    GRAPHICS_CONFIG
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

pub(crate) fn layout_config() -> MathLayoutConfig {
    *LAYOUT_CONFIG
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
            background_color: Color::new(0.0, 0.0, 0.0, 0.0),
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
    image_artifact(&bytes)
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
    validate_encoded_image_size(&bytes)?;
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
    validate_encoded_image_size(&bytes)?;
    Ok(bytes)
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

pub(crate) fn unicode_formula(source: &str) -> Vec<String> {
    let lines = term_maths::render(source.trim())
        .to_string()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        vec![source.trim().to_string()]
    } else {
        lines
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

pub(crate) fn foreground_for_background(
    background: Option<agena_tui_components::TerminalRgb>,
) -> [u8; 3] {
    let Some(background) = background else {
        return [235, 235, 235];
    };
    let luminance = 0.2126 * f32::from(background.red)
        + 0.7152 * f32::from(background.green)
        + 0.0722 * f32::from(background.blue);
    if luminance > 145.0 {
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
    }

    #[test]
    fn unicode_fallback_is_two_dimensional_for_a_fraction() {
        let lines = unicode_formula(r"\frac{a}{b}");
        assert!(lines.len() >= 2);
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
}
