use std::{env, fs, path::PathBuf};

use crate::{
    clipboard::{PastedImageInfo, paste_image_to_temp_png},
    iterm2,
    terminal::{SuspendReason, TerminalContext},
};

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
    fn available(&self, context: &TerminalContext) -> bool;
    fn suspend_reason(&self) -> Option<SuspendReason>;
    fn acquire(&self) -> Result<AttachmentAcquisition, String>;
}

#[derive(Debug, Default)]
pub struct ClipboardImageSource;

impl AttachmentSource for ClipboardImageSource {
    fn available(&self, context: &TerminalContext) -> bool {
        context
            .capabilities
            .clipboard_read_native
            .is_supported_or_unknown()
    }

    fn suspend_reason(&self) -> Option<SuspendReason> {
        None
    }

    fn acquire(&self) -> Result<AttachmentAcquisition, String> {
        let (path, info) = paste_image_to_temp_png().map_err(|error| error.to_string())?;
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

#[derive(Debug)]
pub struct Iterm2UploadSource {
    destination: PathBuf,
}

impl Iterm2UploadSource {
    pub fn new() -> Self {
        Self {
            destination: env::temp_dir()
                .join(format!("agena-iterm-upload-{}", uuid::Uuid::new_v4())),
        }
    }
}

impl AttachmentSource for Iterm2UploadSource {
    fn available(&self, context: &TerminalContext) -> bool {
        iterm2::upload_utility().is_some()
            && (!context.in_multiplexer()
                || context.capabilities.iterm2_file_transfer.is_supported())
    }

    fn suspend_reason(&self) -> Option<SuspendReason> {
        Some(SuspendReason::FileUpload)
    }

    fn acquire(&self) -> Result<AttachmentAcquisition, String> {
        fs::create_dir_all(&self.destination)
            .map_err(|error| format!("could not create iTerm2 upload directory: {error}"))?;
        let result = iterm2::request_upload(&self.destination)
            .and_then(|()| iterm2::uploaded_regular_files(&self.destination));
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
                cleanup_root: Some(self.destination.clone()),
            }),
            Err(error) => {
                let _ = fs::remove_dir_all(&self.destination);
                Err(error)
            }
        }
    }
}

pub fn acquire_from_source(
    source: &dyn AttachmentSource,
    context: &TerminalContext,
    terminal: &mut crate::terminal::TerminalRuntime,
) -> anyhow::Result<Result<AttachmentAcquisition, String>> {
    if !source.available(context) {
        return Ok(Err(
            "this attachment source is unavailable in the current terminal".to_string(),
        ));
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
    fn iterm_upload_destinations_are_unique_and_isolated() {
        let first = Iterm2UploadSource::new();
        let second = Iterm2UploadSource::new();
        assert_ne!(first.destination, second.destination);
        assert!(first.destination.starts_with(env::temp_dir()));
    }
}
