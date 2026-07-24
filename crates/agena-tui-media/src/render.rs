use super::*;

pub fn render_markdown_svg(source: &str) -> Result<Arc<MathArtifact>, String> {
    svg_artifact(source.as_bytes())
}

/// Construct a data URL for an attachment only after proving that decoding it
/// cannot exceed the transcript image byte budget. This prevents an untrusted
/// plugin payload from being duplicated into a second unbounded allocation
/// before the normal image decoder sees it.
pub fn bounded_image_data_url(mime: &str, payload: &str) -> Result<String, String> {
    validate_base64_image_payload_size(payload)?;
    let trimmed = mime.trim();
    let mime = trimmed
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("data:"))
        .and_then(|_| trimmed.get(5..))
        .unwrap_or(trimmed)
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    let subtype = mime
        .split_once('/')
        .filter(|(family, subtype)| {
            family.eq_ignore_ascii_case("image")
                && !subtype.is_empty()
                && subtype.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '+' | '-')
                })
        })
        .map_or("png", |(_, subtype)| subtype);
    Ok(format!("data:image/{subtype};base64,{payload}"))
}

pub(super) fn decode_data_image(source: &str) -> Result<Vec<u8>, String> {
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
    validate_base64_image_payload_size(payload)?;
    let bytes = BASE64_STANDARD
        .decode(payload)
        .map_err(|_| "invalid base64 image data".to_string())?;
    Ok(bytes)
}

fn validate_base64_image_payload_size(payload: &str) -> Result<(), String> {
    let estimated_len = payload.len().saturating_mul(3).div_ceil(4);
    if estimated_len > MAX_MARKDOWN_IMAGE_BYTES {
        return Err("image exceeds the encoded byte safety limit".to_string());
    }
    Ok(())
}

pub(super) fn read_workspace_image(source: &str) -> Result<Vec<u8>, String> {
    if let Ok(url) = url::Url::parse(source) {
        match url.scheme() {
            "http" | "https" => {
                return Err("remote images are not loaded automatically".to_string());
            }
            "file" => {}
            _ => return Err("unsupported image URL scheme".to_string()),
        }
    }
    let workspace = RENDER_CONTEXT_STACK
        .with(|stack| {
            stack
                .borrow()
                .last()
                .and_then(|context| context.workspace.clone())
        })
        .ok_or_else(|| "workspace image loading is not configured".to_string())?;
    let base_url = url::Url::from_directory_path(workspace.as_path())
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
    if !path.starts_with(workspace.as_path()) {
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

pub(super) fn looks_like_svg(bytes: &[u8]) -> bool {
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

pub(super) fn image_artifact(bytes: &[u8]) -> Result<Arc<MathArtifact>, String> {
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

pub(super) fn svg_artifact(bytes: &[u8]) -> Result<Arc<MathArtifact>, String> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err("SVG exceeds the encoded byte safety limit".to_string());
    }
    let config = layout_config();
    let appearance_style = svg_appearance_style(bytes, config.foreground);
    let mut hasher = DefaultHasher::new();
    b"svg".hash(&mut hasher);
    bytes.hash(&mut hasher);
    config.cell_width.hash(&mut hasher);
    config.cell_height.hash(&mut hasher);
    // SVG `currentColor` may inherit Agena's appearance foreground. Include
    // the actual injected style so theme-sensitive SVGs cannot reuse stale
    // pixels while authored root colors stay independent of the TUI theme.
    appearance_style.hash(&mut hasher);
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
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: resvg::usvg::ImageHrefResolver::default_data_resolver(),
            // Markdown SVGs are untrusted transcript content. Preserve embedded
            // data images, but never let an `<image href>` read from disk.
            resolve_string: Box::new(|_, _| None),
        },
        style_sheet: appearance_style,
        ..resvg::usvg::Options::default()
    };
    // `from_data_nested` silently replaces the caller's stylesheet, so use a
    // resolver with the same no-external-resource policy and retain our
    // appearance defaults through the normal parser.
    let tree = resvg::usvg::Tree::from_data(bytes, &options)
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

fn svg_appearance_style(bytes: &[u8], foreground: [u8; 3]) -> Option<String> {
    // Many icon SVGs intentionally use `currentColor`. usvg otherwise resolves
    // a missing SVG `color` property to black, which disappears on dark
    // terminals. Internal styles and inline `style` attributes are parsed
    // after this injected default. A presentation `color` attribute is parsed
    // before it, so omit the default when the SVG root already supplies one.
    let text = std::str::from_utf8(bytes).ok()?;
    let document = roxmltree::Document::parse(text).ok()?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" || root.attribute("color").is_some() {
        return None;
    }
    Some(format!(
        "svg {{ color: rgb({}, {}, {}); }}",
        foreground[0], foreground[1], foreground[2]
    ))
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
        row_layout_weights: Vec::new(),
    });
    ARTIFACT_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(Arc::clone(&artifact));
    Ok(artifact)
}

pub fn unicode_formula(source: &str, display: bool) -> Vec<String> {
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
    };
    if let Some(lines) = UNICODE_MATH_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        return lines.as_ref().clone();
    }

    let lines = unicode_math::parse(source, display)
        .filter(math_ast_within_limits)
        .and_then(|ast| bounded_semantic_unicode(&ast, display))
        .unwrap_or_else(|| explicit_latex_fallback(source));

    UNICODE_MATH_CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, Arc::new(lines.clone()));
    lines
}

pub fn semantic_math_row_heights(source: &str, display: bool) -> Option<Vec<usize>> {
    if source.is_empty() || source.len() > MAX_FORMULA_BYTES || latex_nesting_exceeds_limit(source)
    {
        return None;
    }
    unicode_math::semantic_row_heights(source, display)
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

pub fn positional_unicode_text(source: &str, superscript: bool) -> Option<String> {
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

pub struct MathGraphicsRenderer {
    picker: Picker,
    protocols: HashMap<(u64, u16, u16), SlicedProtocol>,
    generation: u64,
}

impl MathGraphicsRenderer {
    pub fn new(config: MathGraphicsConfig) -> Option<Self> {
        config.is_native().then(|| Self {
            picker: config.picker,
            protocols: HashMap::new(),
            generation: 0,
        })
    }

    pub fn sync_generation(&mut self, generation: u64) {
        if self.generation != generation {
            self.generation = generation;
            self.protocols.clear();
        }
    }

    pub fn render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        scroll: usize,
        placements: &[TranscriptMathPlacement],
    ) {
        for placement in placements {
            let y = placement.line as i64 - scroll as i64;
            if y >= i64::from(area.height) || y + i64::from(placement.size.height) <= 0 {
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
                let Ok(protocol) = SlicedProtocol::new_with_resize(
                    &self.picker,
                    placement.artifact.image.clone(),
                    placement.size,
                    Resize::Scale(None),
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

pub(super) fn foreground_for_background(background: TerminalRgb) -> [u8; 3] {
    let foreground = formula_foreground_for_background(background);
    [foreground.red, foreground.green, foreground.blue]
}

pub fn formula_foreground_for_background(background: TerminalRgb) -> TerminalRgb {
    if background.is_light() {
        TerminalRgb::new(28, 28, 28)
    } else {
        TerminalRgb::new(235, 235, 235)
    }
}
