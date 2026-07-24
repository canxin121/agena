use std::{fs, path::PathBuf};

use crate::{
    clipboard::{PastedImageInfo, paste_image_to_temp_png},
    iterm2, kitty,
    provider_error::ProviderError,
    terminal::{TerminalContext, TerminalRuntime},
};
use agena_tui::terminal_lifecycle::SuspendReason;

const MAX_UPLOADED_FILES: usize = 32;
const MAX_UPLOAD_DIRECTORIES: usize = 64;
const MAX_UPLOAD_DEPTH: usize = 16;
const MAX_UPLOAD_FILE_BYTES: u64 = 50 * 1024 * 1024;
const MAX_UPLOAD_TOTAL_BYTES: u64 = 200 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 32_768;
const MAX_IMAGE_PIXELS: u64 = 100_000_000;

#[derive(Debug)]
pub struct AcquiredAttachment {
    pub path: PathBuf,
    pub temporary: bool,
    pub image_info: Option<PastedImageInfo>,
}

#[derive(Debug)]
pub struct AttachmentAcquisition {
    pub items: Vec<AcquiredAttachment>,
    pub cleanup_root: Option<PathBuf>,
}

pub trait AttachmentSource {
    fn label(&self) -> &'static str;
    fn available(&self, context: &TerminalContext) -> bool;
    fn suspend_reason(&self) -> Option<SuspendReason>;
    fn acquire(&self) -> Result<AttachmentAcquisition, ProviderError>;
}

#[derive(Debug, Default)]
pub struct ClipboardImageSource;

impl AttachmentSource for ClipboardImageSource {
    fn label(&self) -> &'static str {
        "clipboard"
    }

    fn available(&self, context: &TerminalContext) -> bool {
        context.capabilities.clipboard_read_native.is_operational()
    }

    fn suspend_reason(&self) -> Option<SuspendReason> {
        None
    }

    fn acquire(&self) -> Result<AttachmentAcquisition, ProviderError> {
        let (path, info) = paste_image_to_temp_png().map_err(|error| {
            ProviderError::Unsupported(format!("native clipboard image is unavailable: {error}"))
        })?;
        if let Err(error) = validate_image_dimensions(info.width, info.height) {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(AttachmentAcquisition {
            items: vec![AcquiredAttachment {
                path,
                temporary: true,
                image_info: Some(info),
            }],
            cleanup_root: None,
        })
    }
}

#[derive(Debug, Default)]
pub struct KittyClipboardImageSource;

impl KittyClipboardImageSource {
    pub fn new() -> Self {
        Self
    }

    pub fn provider_available(context: &TerminalContext) -> bool {
        context.capabilities.kitty_rich_clipboard.is_operational()
            && kitty::clipboard_utility().is_some()
    }
}

impl AttachmentSource for KittyClipboardImageSource {
    fn label(&self) -> &'static str {
        "Kitty clipboard"
    }

    fn available(&self, context: &TerminalContext) -> bool {
        Self::provider_available(context)
    }

    fn suspend_reason(&self) -> Option<SuspendReason> {
        Some(SuspendReason::ClipboardRead)
    }

    fn acquire(&self) -> Result<AttachmentAcquisition, ProviderError> {
        let root = secure_temp_root("agena-kitty-clipboard-")?;
        let destination = root.join("clipboard.png");
        if let Err(error) = kitty::request_clipboard_image(destination.as_path()) {
            let _ = fs::remove_dir_all(&root);
            return Err(error);
        }
        let files = match inspect_transfer_tree(root.as_path(), self.label(), true) {
            Ok(files) => files,
            Err(error) => {
                let _ = fs::remove_dir_all(&root);
                return Err(error);
            }
        };
        if files.len() != 1 || files[0] != destination {
            let _ = fs::remove_dir_all(&root);
            return Err(ProviderError::Protocol(
                "Kitty clipboard did not produce exactly one regular image file".to_owned(),
            ));
        }
        let (width, height) = image::image_dimensions(&destination).map_err(|error| {
            let _ = fs::remove_dir_all(&root);
            ProviderError::Unsupported(format!(
                "Kitty clipboard did not provide a supported raster image: {error}"
            ))
        })?;
        validate_image_dimensions(width, height).inspect_err(|_| {
            let _ = fs::remove_dir_all(&root);
        })?;
        Ok(AttachmentAcquisition {
            items: vec![AcquiredAttachment {
                path: destination,
                temporary: true,
                image_info: Some(PastedImageInfo { width, height }),
            }],
            cleanup_root: Some(root),
        })
    }
}

pub fn acquire_clipboard_image(
    context: &TerminalContext,
    terminal: &mut TerminalRuntime,
) -> anyhow::Result<Result<AttachmentAcquisition, ProviderError>> {
    let native = ClipboardImageSource;
    let kitty = KittyClipboardImageSource::new();
    let mut providers: Vec<&dyn AttachmentSource> = Vec::new();
    if context.capabilities.clipboard_read_native.is_operational() {
        providers.push(&native);
    }
    if kitty.available(context) {
        providers.push(&kitty);
    }
    let mut failures = Vec::new();
    for source in providers {
        match acquire_from_source(source, context, terminal)? {
            Ok(acquisition) => return Ok(Ok(acquisition)),
            Err(error) if error.allows_fallback() => {
                failures.push(format!("{}: {error}", source.label()));
            }
            Err(error) => return Ok(Err(error)),
        }
    }
    Ok(Err(ProviderError::Unsupported(if failures.is_empty() {
        "no compatible clipboard image provider is available".to_owned()
    } else {
        failures.join("; ")
    })))
}

#[derive(Debug, Default)]
pub struct Iterm2UploadSource;

impl Iterm2UploadSource {
    pub fn new() -> Self {
        Self
    }

    pub fn provider_available(context: &TerminalContext) -> bool {
        iterm2::upload_utility().is_some()
            && context.capabilities.iterm2_file_transfer.is_operational()
    }
}

impl AttachmentSource for Iterm2UploadSource {
    fn label(&self) -> &'static str {
        "iTerm2"
    }

    fn available(&self, context: &TerminalContext) -> bool {
        Self::provider_available(context)
    }

    fn suspend_reason(&self) -> Option<SuspendReason> {
        Some(SuspendReason::FileUpload)
    }

    fn acquire(&self) -> Result<AttachmentAcquisition, ProviderError> {
        let destination = secure_temp_root("agena-iterm-upload-")?;
        let mut guard = || monitor_transfer_tree(destination.as_path(), self.label());
        let result = iterm2::request_upload(destination.as_path(), &mut guard)
            .and_then(|()| uploaded_regular_files(destination.as_path(), self.label()));
        transfer_acquisition(destination, result)
    }
}

#[derive(Debug)]
pub struct KittyUploadSource {
    local_sources: Vec<String>,
}

impl KittyUploadSource {
    pub fn new(local_sources: Vec<String>) -> Self {
        Self { local_sources }
    }

    pub fn provider_available(context: &TerminalContext) -> bool {
        context.capabilities.kitty_file_transfer.is_operational()
            && kitty::transfer_utility().is_some()
    }
}

impl AttachmentSource for KittyUploadSource {
    fn label(&self) -> &'static str {
        "Kitty"
    }

    fn available(&self, context: &TerminalContext) -> bool {
        Self::provider_available(context) && !self.local_sources.is_empty()
    }

    fn suspend_reason(&self) -> Option<SuspendReason> {
        Some(SuspendReason::FileUpload)
    }

    fn acquire(&self) -> Result<AttachmentAcquisition, ProviderError> {
        let destination = secure_temp_root("agena-kitty-upload-")?;
        let mut guard = || monitor_transfer_tree(destination.as_path(), self.label());
        let result = kitty::request_upload(&self.local_sources, destination.as_path(), &mut guard)
            .and_then(|()| uploaded_regular_files(destination.as_path(), self.label()));
        transfer_acquisition(destination, result)
    }
}

fn transfer_acquisition(
    destination: PathBuf,
    result: Result<Vec<PathBuf>, ProviderError>,
) -> Result<AttachmentAcquisition, ProviderError> {
    match result {
        Ok(files) => Ok(AttachmentAcquisition {
            items: files
                .into_iter()
                .map(|path| AcquiredAttachment {
                    path,
                    temporary: true,
                    image_info: None,
                })
                .collect(),
            cleanup_root: Some(destination),
        }),
        Err(error) => {
            let _ = fs::remove_dir_all(destination);
            Err(error)
        }
    }
}

fn secure_temp_root(prefix: &str) -> Result<PathBuf, ProviderError> {
    for _ in 0..16 {
        let path = std::env::temp_dir().join(format!("{prefix}{}", uuid::Uuid::new_v4()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(ProviderError::Io(error)),
        }
    }
    Err(ProviderError::Io(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate a unique private attachment directory",
    )))
}

fn uploaded_regular_files(
    destination: &std::path::Path,
    provider: &str,
) -> Result<Vec<PathBuf>, ProviderError> {
    inspect_transfer_tree(destination, provider, true)
}

fn monitor_transfer_tree(
    destination: &std::path::Path,
    provider: &str,
) -> Result<(), ProviderError> {
    inspect_transfer_tree(destination, provider, false).map(drop)
}

fn inspect_transfer_tree(
    destination: &std::path::Path,
    provider: &str,
    require_files: bool,
) -> Result<Vec<PathBuf>, ProviderError> {
    let mut files = Vec::new();
    let mut directories = 0_usize;
    let mut total_bytes = 0_u64;
    let mut pending = vec![(destination.to_path_buf(), 0_usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_UPLOAD_DEPTH {
            return Err(ProviderError::Unsupported(format!(
                "{provider} upload exceeds the maximum directory depth of {MAX_UPLOAD_DEPTH}"
            )));
        }
        for entry in fs::read_dir(&directory).map_err(|error| {
            ProviderError::Io(std::io::Error::new(
                error.kind(),
                format!("could not inspect {provider} upload directory: {error}"),
            ))
        })? {
            let entry = entry.map_err(ProviderError::Io)?;
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(ProviderError::Io(error)),
            };
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(ProviderError::Unsupported(format!(
                    "{provider} upload contained a symbolic link, which is not allowed: {}",
                    path.display()
                )));
            }
            if file_type.is_dir() {
                directories = directories.saturating_add(1);
                if directories > MAX_UPLOAD_DIRECTORIES {
                    return Err(ProviderError::Unsupported(format!(
                        "{provider} upload contains more than {MAX_UPLOAD_DIRECTORIES} directories"
                    )));
                }
                pending.push((path, depth.saturating_add(1)));
            } else if file_type.is_file() {
                if files.len() >= MAX_UPLOADED_FILES {
                    return Err(ProviderError::Unsupported(format!(
                        "{provider} upload contains more than {MAX_UPLOADED_FILES} files"
                    )));
                }
                let bytes = metadata.len();
                if bytes > MAX_UPLOAD_FILE_BYTES {
                    return Err(ProviderError::Unsupported(format!(
                        "{} exceeds the {} MiB per-file upload limit",
                        path.display(),
                        MAX_UPLOAD_FILE_BYTES / 1024 / 1024
                    )));
                }
                total_bytes = total_bytes.saturating_add(bytes);
                if total_bytes > MAX_UPLOAD_TOTAL_BYTES {
                    return Err(ProviderError::Unsupported(format!(
                        "{provider} upload exceeds the {} MiB total size limit",
                        MAX_UPLOAD_TOTAL_BYTES / 1024 / 1024
                    )));
                }
                files.push(path);
            } else {
                return Err(ProviderError::Unsupported(format!(
                    "{provider} upload contained an unsupported special file: {}",
                    path.display()
                )));
            }
        }
    }
    if require_files && files.is_empty() {
        return Err(ProviderError::Unsupported(format!(
            "no files were received through {provider}"
        )));
    }
    files.sort();
    Ok(files)
}

fn validate_image_dimensions(width: u32, height: u32) -> Result<(), ProviderError> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION || pixels > MAX_IMAGE_PIXELS {
        return Err(ProviderError::Unsupported(format!(
            "clipboard image dimensions {width}×{height} exceed the safety limit"
        )));
    }
    Ok(())
}

pub fn acquire_from_source(
    source: &dyn AttachmentSource,
    context: &TerminalContext,
    terminal: &mut TerminalRuntime,
) -> anyhow::Result<Result<AttachmentAcquisition, ProviderError>> {
    if !source.available(context) {
        return Ok(Err(ProviderError::Unsupported(
            "this attachment source is unavailable in the current terminal".to_owned(),
        )));
    }
    match source.suspend_reason() {
        Some(reason) => terminal.with_suspended(reason, || source.acquire()),
        None => Ok(source.acquire()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secure_upload_roots_are_unique() {
        let first = secure_temp_root("agena-test-upload-").expect("first root");
        let second = secure_temp_root("agena-test-upload-").expect("second root");
        assert_ne!(first, second);
        let _ = fs::remove_dir_all(first);
        let _ = fs::remove_dir_all(second);
    }

    #[cfg(unix)]
    #[test]
    fn secure_upload_roots_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = secure_temp_root("agena-test-private-").expect("private root");
        let mode = fs::metadata(&root).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn uploaded_regular_files_recurse_and_sort() {
        let directory = tempfile::tempdir().expect("temporary upload directory");
        fs::write(directory.path().join("b.txt"), "b").expect("write first file");
        fs::create_dir(directory.path().join("nested")).expect("create nested directory");
        fs::write(directory.path().join("nested/a.txt"), "a").expect("write nested file");
        let files = uploaded_regular_files(directory.path(), "test").expect("collect files");
        let relative = files
            .iter()
            .map(|path| {
                path.strip_prefix(directory.path())
                    .expect("relative upload path")
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(relative, ["b.txt", "nested/a.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn uploaded_regular_files_reject_symbolic_links() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().expect("temporary upload directory");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        symlink(outside.path(), directory.path().join("escape")).expect("create upload symlink");
        let error =
            uploaded_regular_files(directory.path(), "test").expect_err("symlink must be rejected");
        assert!(error.to_string().contains("symbolic link"));
    }

    #[test]
    fn uploaded_regular_files_enforce_count_limit() {
        let directory = tempfile::tempdir().expect("temporary upload directory");
        for index in 0..=MAX_UPLOADED_FILES {
            fs::write(directory.path().join(format!("{index:02}.txt")), "x")
                .expect("write upload file");
        }
        let error = uploaded_regular_files(directory.path(), "test")
            .expect_err("oversized transfer must be rejected");
        assert!(error.to_string().contains("more than 32"));
    }

    #[test]
    fn uploaded_regular_files_enforce_total_size_limit() {
        let directory = tempfile::tempdir().expect("temporary upload directory");
        let path = directory.path().join("large.bin");
        let file = fs::File::create(&path).expect("large file");
        file.set_len(MAX_UPLOAD_FILE_BYTES + 1)
            .expect("set sparse file length");
        let error = uploaded_regular_files(directory.path(), "test")
            .expect_err("large transfer must be rejected");
        assert!(error.to_string().contains("per-file upload limit"));
    }
}
