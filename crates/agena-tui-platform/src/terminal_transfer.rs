//! Terminal file transfer helpers.

use std::path::Path;

use crate::{iterm2, kitty, provider_error::ProviderError, terminal::TerminalContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadProvider {
    Iterm2,
    Kitty,
}

impl DownloadProvider {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Iterm2 => "iTerm2",
            Self::Kitty => "Kitty",
        }
    }
}

pub fn download_providers(context: &TerminalContext) -> Vec<DownloadProvider> {
    let mut providers = Vec::new();
    if context.capabilities.iterm2_file_transfer.is_operational()
        && iterm2::download_utility().is_some()
    {
        providers.push(DownloadProvider::Iterm2);
    }
    if context.capabilities.kitty_file_transfer.is_operational()
        && kitty::transfer_utility().is_some()
    {
        providers.push(DownloadProvider::Kitty);
    }
    providers
}

pub fn request_download(provider: DownloadProvider, path: &Path) -> Result<(), ProviderError> {
    match provider {
        DownloadProvider::Iterm2 => iterm2::request_download(path),
        DownloadProvider::Kitty => kitty::request_download(path),
    }
}
