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
            let file = Builder::new()
                .prefix("agena-clipboard-")
                .suffix(".png")
                .tempfile()
                .map_err(|error| PasteImageError::io_error(&error))?;
            std::fs::write(file.path(), png).map_err(|error| PasteImageError::io_error(&error))?;
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

    let image = if let Some(image) = files.into_iter().find_map(|path| image::open(path).ok()) {
        image
    } else {
        let image = clipboard
            .get_image()
            .map_err(|error| PasteImageError::no_image_error(&error))?;
        let width = image.width as u32;
        let height = image.height as u32;
        let Some(rgba) = image::RgbaImage::from_raw(width, height, image.bytes.into_owned()) else {
            return Err(PasteImageError::EncodeFailed(
                "invalid RGBA clipboard buffer".to_string(),
            ));
        };
        image::DynamicImage::ImageRgba8(rgba)
    };

    let mut png = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut png);
    image
        .write_to(&mut cursor, image::ImageFormat::Png)
        .map_err(|error| PasteImageError::encode_error(&error))?;

    Ok((
        png,
        PastedImageInfo {
            width: image.width(),
            height: image.height(),
        },
    ))
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
