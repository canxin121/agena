use std::{
    path::{Path, PathBuf},
    process::Command,
};

use tempfile::Builder;

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct ClipboardTextError(pub String);

impl std::fmt::Display for ClipboardTextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl std::error::Error for ClipboardTextError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub struct PastedImageInfo {
    pub width: u32,
    pub height: u32,
}

#[cfg(not(target_os = "android"))]
pub fn paste_image_to_temp_png() -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    match paste_image_as_png() {
        Ok((png, info)) => {
            let file = Builder::new()
                .prefix("agena-clipboard-")
                .suffix(".png")
                .tempfile()
                .map_err(|error| PasteImageError::IoError(error.to_string()))?;
            std::fs::write(file.path(), png)
                .map_err(|error| PasteImageError::IoError(error.to_string()))?;
            let (_file, path) = file
                .keep()
                .map_err(|error| PasteImageError::IoError(error.error.to_string()))?;
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

#[cfg(not(target_os = "android"))]
pub fn set_clipboard_text(text: &str) -> Result<(), ClipboardTextError> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| ClipboardTextError(error.to_string()))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| ClipboardTextError(error.to_string()))
}

#[cfg(target_os = "android")]
pub fn set_clipboard_text(_text: &str) -> Result<(), ClipboardTextError> {
    Err(ClipboardTextError(
        "clipboard text copy is unsupported on Android".to_string(),
    ))
}

#[cfg(target_os = "android")]
pub fn paste_image_to_temp_png() -> Result<(PathBuf, PastedImageInfo), PasteImageError> {
    Err(PasteImageError::ClipboardUnavailable(
        "clipboard image paste is unsupported on Android".to_string(),
    ))
}

#[cfg(not(target_os = "android"))]
fn paste_image_as_png() -> Result<(Vec<u8>, PastedImageInfo), PasteImageError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| PasteImageError::ClipboardUnavailable(error.to_string()))?;

    let files = clipboard
        .get()
        .file_list()
        .map_err(|error| PasteImageError::ClipboardUnavailable(error.to_string()))
        .unwrap_or_default();

    let image = if let Some(image) = files.into_iter().find_map(|path| image::open(path).ok()) {
        image
    } else {
        let image = clipboard
            .get_image()
            .map_err(|error| PasteImageError::NoImage(error.to_string()))?;
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
        .map_err(|error| PasteImageError::EncodeFailed(error.to_string()))?;

    Ok((
        png,
        PastedImageInfo {
            width: image.width(),
            height: image.height(),
        },
    ))
}

pub fn normalize_pasted_path(pasted: &str) -> Option<PathBuf> {
    let pasted = pasted.trim();
    if pasted.is_empty() {
        return None;
    }

    if let Ok(url) = url::Url::parse(pasted)
        && url.scheme() == "file"
    {
        return url.to_file_path().ok();
    }

    let looks_like_windows_path = {
        let drive = pasted
            .chars()
            .next()
            .map(|char| char.is_ascii_alphabetic())
            .unwrap_or(false)
            && pasted.get(1..2) == Some(":")
            && pasted
                .get(2..3)
                .map(|component| component == "\\" || component == "/")
                .unwrap_or(false);
        let unc = pasted.starts_with("\\\\");
        drive || unc
    };
    if looks_like_windows_path {
        #[cfg(target_os = "linux")]
        {
            if is_probably_wsl()
                && let Some(converted) = convert_windows_path_to_wsl(pasted)
            {
                return Some(converted);
            }
        }
        return Some(PathBuf::from(pasted));
    }

    let parts: Vec<String> = shlex::Shlex::new(pasted).collect();
    if parts.len() == 1 {
        return parts.into_iter().next().map(PathBuf::from);
    }

    let trimmed_quotes = pasted
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            pasted
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        });
    trimmed_quotes.map(PathBuf::from)
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
pub fn is_probably_wsl() -> bool {
    if let Ok(version) = std::fs::read_to_string("/proc/version") {
        let lower = version.to_ascii_lowercase();
        if lower.contains("microsoft") || lower.contains("wsl") {
            return true;
        }
    }

    std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some()
}

#[cfg(target_os = "linux")]
fn convert_windows_path_to_wsl(path: &str) -> Option<PathBuf> {
    if path.starts_with("\\\\") {
        return None;
    }
    let drive = path.chars().next()?.to_ascii_lowercase();
    if !drive.is_ascii_lowercase() || path.get(1..2) != Some(":") {
        return None;
    }

    let mut out = PathBuf::from(format!("/mnt/{drive}"));
    for component in path
        .get(2..)?
        .trim_start_matches(['\\', '/'])
        .split(['\\', '/'])
        .filter(|component| !component.is_empty())
    {
        out.push(component);
    }
    Some(out)
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
    let Ok((width, height)) = image::image_dimensions(&path) else {
        return Err(error.clone());
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
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
            Ok(_) | Err(_) => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn normalize_file_url() {
        let path = normalize_pasted_path("file:///tmp/example.png").expect("file url");
        assert_eq!(path, PathBuf::from("/tmp/example.png"));
    }

    #[test]
    fn normalize_shell_escaped_single_path() {
        let path = normalize_pasted_path("/home/user/My\\ File.png").expect("shell path");
        assert_eq!(path, PathBuf::from("/home/user/My File.png"));
    }

    #[test]
    fn normalize_quoted_path() {
        let path = normalize_pasted_path("\"/home/user/My File.png\"").expect("quoted path");
        assert_eq!(path, PathBuf::from("/home/user/My File.png"));
    }

    #[test]
    fn pasted_image_format_detects_common_extensions() {
        assert_eq!(
            pasted_image_format(Path::new("/tmp/a.PNG")),
            EncodedImageFormat::Png
        );
        assert_eq!(
            pasted_image_format(Path::new("/tmp/a.jpeg")),
            EncodedImageFormat::Jpeg
        );
        assert_eq!(
            pasted_image_format(Path::new("/tmp/a.bin")),
            EncodedImageFormat::Other
        );
    }
}
