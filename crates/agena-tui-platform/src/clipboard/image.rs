use std::path::{Path, PathBuf};

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
use tempfile::Builder;

#[cfg(target_os = "linux")]
use std::process::Command;

#[cfg(target_os = "linux")]
use super::path::{convert_windows_path_to_wsl, is_probably_wsl};

#[derive(Debug, Clone)]
/// Error pasting an image from the clipboard.
pub enum PasteImageError {
    ClipboardUnavailable(String),
    NoImage(String),
    EncodeFailed(String),
    IoError(String),
}

impl std::fmt::Display for PasteImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ClipboardUnavailable(message) => write!(f, "clipboard unavailable: {message}"),
            Self::NoImage(message) => write!(f, "no image on clipboard: {message}"),
            Self::EncodeFailed(message) => write!(f, "image encode failed: {message}"),
            Self::IoError(message) => write!(f, "io error: {message}"),
        }
    }
}

impl std::error::Error for PasteImageError {}

impl PasteImageError {
    fn clipboard_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::ClipboardUnavailable(agena_failure::diagnostic::format_error_chain(error))
    }

    fn no_image_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::NoImage(agena_failure::diagnostic::format_error_chain(error))
    }

    fn encode_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::EncodeFailed(agena_failure::diagnostic::format_error_chain(error))
    }

    fn io_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::IoError(agena_failure::diagnostic::format_error_chain(error))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Encoded format of a pasted image.
pub enum EncodedImageFormat {
    Png,
    Jpeg,
    Other,
}

impl EncodedImageFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Other => "IMG",
        }
    }
}

#[derive(Debug, Clone)]
/// Information about a pasted image.
pub struct PastedImageInfo {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
/// Error reading filesystem entries advertised by the native clipboard.
pub struct ClipboardFilesError(pub String);

impl std::fmt::Display for ClipboardFilesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl std::error::Error for ClipboardFilesError {}

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
pub fn clipboard_file_list() -> Result<Vec<PathBuf>, ClipboardFilesError> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| {
        ClipboardFilesError(agena_failure::diagnostic::format_error_chain(&error))
    })?;
    clipboard
        .get()
        .file_list()
        .map_err(|error| ClipboardFilesError(agena_failure::diagnostic::format_error_chain(&error)))
}

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
pub fn clipboard_file_list() -> Result<Vec<PathBuf>, ClipboardFilesError> {
    Err(ClipboardFilesError(
        "clipboard file-list read is unsupported on this platform".to_owned(),
    ))
}

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
pub fn paste_image_to_temp_png() -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    match paste_image_as_png() {
        Ok((png, info)) => {
            let mut file = Builder::new()
                .prefix("agena-clipboard-")
                .suffix(".png")
                .tempfile()
                .map_err(|error| PasteImageError::io_error(&error))?;
            std::io::Write::write_all(file.as_file_mut(), &png)
                .map_err(|error| PasteImageError::io_error(&error))?;
            let (_, path) = file
                .keep()
                .map_err(|error| PasteImageError::io_error(&error.error))?;
            Ok((path, info))
        }
        Err(error) => {
            #[cfg(target_os = "linux")]
            {
                try_wsl_clipboard_fallback(&error).or(Err(error))
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err(error)
            }
        }
    }
}

#[cfg(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
))]
pub fn paste_image_to_temp_png() -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    Err(PasteImageError::ClipboardUnavailable(
        "clipboard image paste is unsupported on this platform".to_string(),
    ))
}

#[cfg(not(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos"
)))]
fn paste_image_as_png() -> Result<(Vec<u8>, PastedImageInfo), PasteImageError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| PasteImageError::clipboard_error(&error))?;

    let files = clipboard
        .get()
        .file_list()
        .map_err(|error| PasteImageError::clipboard_error(&error))
        .unwrap_or_default();

    let image = if let Some(path) = files.into_iter().find(|path| {
        path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    matches!(
                        ext.to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
                    )
                })
    }) {
        decode_clipboard_image_file(&path)?
    } else {
        let image = clipboard
            .get_image()
            .map_err(|error| PasteImageError::no_image_error(&error))?;
        let (width, height) =
            checked_rgba_dimensions(image.width, image.height, image.bytes.len())?;
        let rgba = image::RgbaImage::from_raw(width, height, image.bytes.into_owned())
            .ok_or_else(|| PasteImageError::EncodeFailed("invalid RGBA clipboard buffer".into()))?;
        image::DynamicImage::ImageRgba8(rgba)
    };
    let png = encode_clipboard_png(&image)?;

    Ok((
        png,
        PastedImageInfo {
            width: image.width(),
            height: image.height(),
        },
    ))
}

const MAX_CLIPBOARD_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_CLIPBOARD_PIXELS: usize = 40_000_000;
fn checked_rgba_dimensions(
    width: usize,
    height: usize,
    length: usize,
) -> Result<(u32, u32), PasteImageError> {
    let expected = width.checked_mul(height).and_then(|n| n.checked_mul(4));
    if width == 0
        || height == 0
        || width > 16384
        || height > 16384
        || width
            .checked_mul(height)
            .is_none_or(|n| n > MAX_CLIPBOARD_PIXELS)
        || expected != Some(length)
    {
        return Err(PasteImageError::EncodeFailed(
            "clipboard RGBA size is invalid or exceeds the 40 megapixel/16384 pixel safety limit"
                .into(),
        ));
    }
    Ok((width as u32, height as u32))
}
fn decode_clipboard_image_file(path: &Path) -> Result<image::DynamicImage, PasteImageError> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|e| PasteImageError::io_error(&e))?;
    if !file
        .metadata()
        .map_err(|e| PasteImageError::io_error(&e))?
        .is_file()
    {
        return Err(PasteImageError::EncodeFailed(
            "clipboard image is not a regular file".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CLIPBOARD_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| PasteImageError::io_error(&e))?;
    if bytes.len() > MAX_CLIPBOARD_IMAGE_BYTES {
        return Err(PasteImageError::EncodeFailed(
            "clipboard image exceeds the 20 MiB compressed input limit".into(),
        ));
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| PasteImageError::io_error(&e))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| PasteImageError::encode_error(&e))?;
    let count = (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or_else(|| PasteImageError::EncodeFailed("image dimension overflow".into()))?;
    checked_rgba_dimensions(w as usize, h as usize, count)?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| PasteImageError::io_error(&e))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some((MAX_CLIPBOARD_PIXELS * 4) as u64);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| PasteImageError::encode_error(&e))
}
fn encode_clipboard_png(image: &image::DynamicImage) -> Result<Vec<u8>, PasteImageError> {
    struct Limited(std::io::Cursor<Vec<u8>>);
    impl std::io::Write for Limited {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.position().saturating_add(bytes.len() as u64)
                > MAX_CLIPBOARD_IMAGE_BYTES as u64
            {
                return Err(std::io::Error::other(
                    "encoded clipboard image exceeds 20 MiB",
                ));
            }
            std::io::Write::write(&mut self.0, bytes)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl std::io::Seek for Limited {
        fn seek(&mut self, from: std::io::SeekFrom) -> std::io::Result<u64> {
            let position = std::io::Seek::seek(&mut self.0, from)?;
            if position > MAX_CLIPBOARD_IMAGE_BYTES as u64 {
                return Err(std::io::Error::other("PNG seek exceeds buffer limit"));
            }
            Ok(position)
        }
    }
    let mut output = Limited(std::io::Cursor::new(Vec::new()));
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|e| PasteImageError::encode_error(&e))?;
    Ok(output.0.into_inner())
}

pub fn pasted_image_format(path: &Path) -> EncodedImageFormat {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => EncodedImageFormat::Png,
        Some("jpg") | Some("jpeg") => EncodedImageFormat::Jpeg,
        _ => EncodedImageFormat::Other,
    }
}

#[cfg(target_os = "linux")]
fn try_wsl_clipboard_fallback(
    error: &PasteImageError,
) -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    if !is_probably_wsl()
        || !matches!(
            error,
            PasteImageError::ClipboardUnavailable(_) | PasteImageError::NoImage(_)
        )
    {
        return Err(error.clone());
    }

    let Some(win_path) = try_dump_windows_clipboard_image() else {
        return Err(error.clone());
    };
    let Some(path) = convert_windows_path_to_wsl(win_path.as_str()) else {
        return Err(error.clone());
    };
    let (width, height) = match image::image_dimensions(&path) {
        Ok(dimensions) => dimensions,
        Err(dimension_error) => {
            tracing::debug!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "the WSL clipboard fallback produced an unreadable image",
                    &dimension_error,
                ),
                "discarding an unusable WSL clipboard image fallback"
            );
            return Err(error.clone());
        }
    };

    Ok((path, PastedImageInfo { width, height }))
}

#[cfg(target_os = "linux")]
fn try_dump_windows_clipboard_image() -> Option<String> {
    let script = r#"[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; $img = Get-Clipboard -Format Image; if ($img -ne $null) { $p=[System.IO.Path]::GetTempFileName(); $p = [System.IO.Path]::ChangeExtension($p,'png'); $img.Save($p,[System.Drawing.Imaging.ImageFormat]::Png); Write-Output $p } else { exit 1 }"#;

    for command in ["powershell.exe", "pwsh", "powershell"] {
        match Command::new(command)
            .args(["-NoProfile", "-Command", script])
            .output()
        {
            Ok(output) if output.status.success() => match String::from_utf8(output.stdout) {
                Ok(path) if !path.trim().is_empty() => return Some(path.trim().to_owned()),
                Ok(_) => tracing::debug!(
                    clipboard_command = command,
                    "Windows clipboard helper succeeded without returning an image path"
                ),
                Err(error) => tracing::debug!(
                    clipboard_command = command,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "Windows clipboard helper returned a non-UTF-8 image path",
                        &error,
                    ),
                    "discarding an invalid Windows clipboard helper response"
                ),
            },
            Ok(output) => tracing::debug!(
                clipboard_command = command,
                status = %output.status,
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "Windows clipboard helper did not return an image"
            ),
            Err(error) => tracing::debug!(
                clipboard_command = command,
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to launch the Windows clipboard helper",
                    &error,
                ),
                "Windows clipboard helper is unavailable"
            ),
        }
    }
    None
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    #[test]
    fn rgba_dimensions_are_checked_before_integer_casts_or_copies() {
        assert!(checked_rgba_dimensions(usize::MAX, 2, 4).is_err());
        assert!(checked_rgba_dimensions(1, 1, 3).is_err());
        assert!(checked_rgba_dimensions(0, 1, 0).is_err());
        assert!(checked_rgba_dimensions(16384, 16384, 16384 * 16384 * 4).is_err());
        assert_eq!(checked_rgba_dimensions(2, 2, 16).unwrap(), (2, 2));
    }
    #[test]
    fn synthetic_clipboard_png_roundtrips_without_reading_real_clipboard() {
        let dir = tempfile::tempdir().unwrap();
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([10, 20, 30, 255]),
        ));
        let encoded = encode_clipboard_png(&image).unwrap();
        let path = dir.path().join("clipboard.png");
        std::fs::write(&path, encoded).unwrap();
        let decoded = decode_clipboard_image_file(&path).unwrap();
        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 2);
    }
}
