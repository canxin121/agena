//! Immutable browser resources and errors shared by plugin page callers.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use agena_plugin_sdk::PluginWebContributions;
use agena_plugin_sdk::web::valid_web_asset_path;
use bytes::Bytes;

/// Bound the complete in-memory resource snapshot for one plugin.
const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ASSET_ENTRIES: usize = 4096;
const MAX_ASSET_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub struct PluginWebAsset {
    pub bytes: Bytes,
    pub content_type: &'static str,
}

#[derive(Debug)]
pub(crate) struct PluginWebBundle {
    pub revision: String,
    pub assets: BTreeMap<String, PluginWebAsset>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PluginWebError {
    #[error("The plugin page is no longer available.")]
    NotFound,
    #[error("The plugin changed. Reload this page before continuing.")]
    StaleGeneration,
    #[error("This operation is not declared by the plugin page.")]
    Forbidden,
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Plugin(#[source] Box<crate::sdk::PluginError>),
}

impl PluginWebBundle {
    /// Read a complete, bounded snapshot before plugin activation. Serving a
    /// revision never reopens mutable paths from a later filesystem state.
    pub fn load(package_root: Option<&Path>, web: &PluginWebContributions) -> Result<Self, String> {
        let package_root =
            package_root.ok_or("a plugin with Web pages requires an explicit package_root")?;
        if !package_root.is_absolute() {
            return Err("Web package_root must be an absolute path".into());
        }
        if !valid_web_asset_path(&web.assets.root) {
            return Err("Web assets.root must stay inside the package root".into());
        }
        let package_root = package_root
            .canonicalize()
            .map_err(|error| format!("cannot resolve Web package_root: {error}"))?;
        if !package_root.is_dir() {
            return Err("Web package_root must be a directory".into());
        }
        let mut root = package_root.clone();
        for component in web.assets.root.split('/') {
            root.push(component);
            let metadata = root
                .symlink_metadata()
                .map_err(|error| format!("cannot inspect Web assets root: {error}"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("Web assets.root must contain only real directories".into());
            }
        }
        let root = root
            .canonicalize()
            .map_err(|error| format!("cannot resolve Web assets root: {error}"))?;
        if !root.starts_with(&package_root) {
            return Err("Web assets.root escapes its package root".into());
        }

        let mut assets = BTreeMap::new();
        let mut pending = vec![(root.clone(), 0usize)];
        let mut total_bytes = 0usize;
        let mut entries_seen = 0usize;
        while let Some((directory, depth)) = pending.pop() {
            for entry in std::fs::read_dir(&directory)
                .map_err(|error| format!("cannot read Web assets directory: {error}"))?
            {
                let entry = entry.map_err(|error| format!("cannot read Web asset: {error}"))?;
                entries_seen += 1;
                if entries_seen > MAX_ASSET_ENTRIES {
                    return Err("Web assets exceed 4096 files and directories".into());
                }
                let path = entry.path();
                let metadata = path
                    .symlink_metadata()
                    .map_err(|error| format!("cannot inspect Web asset: {error}"))?;
                if metadata.file_type().is_symlink() {
                    return Err("Web assets must not contain symbolic links".into());
                }
                let resolved = path
                    .canonicalize()
                    .map_err(|error| format!("cannot resolve Web asset: {error}"))?;
                if !resolved.starts_with(&root) {
                    return Err("Web asset escapes its declared root".into());
                }
                if metadata.is_dir() {
                    if depth >= MAX_ASSET_DEPTH {
                        return Err("Web assets exceed the directory depth limit".into());
                    }
                    pending.push((resolved, depth + 1));
                    continue;
                }
                if !metadata.is_file() {
                    return Err("Web assets may contain only regular files and directories".into());
                }
                let relative = path
                    .strip_prefix(&root)
                    .map_err(|_| "Web asset escapes its declared root")?;
                let name = relative
                    .to_str()
                    .ok_or("Web asset paths must be UTF-8")?
                    .replace(std::path::MAIN_SEPARATOR, "/");
                if !valid_web_asset_path(&name) {
                    return Err(format!("invalid Web asset path `{name}`"));
                }
                let remaining = MAX_ASSET_BYTES.saturating_sub(total_bytes);
                let limit = MAX_FILE_BYTES.min(remaining);
                if metadata.len() > limit as u64 {
                    return Err("Web assets exceed the 16 MiB file or 64 MiB package limit".into());
                }
                let mut bytes = Vec::new();
                std::fs::File::open(&resolved)
                    .map_err(|error| format!("cannot open Web asset: {error}"))?
                    .take((limit + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map_err(|error| format!("cannot read Web asset: {error}"))?;
                if bytes.len() > limit {
                    return Err("Web assets exceed the 16 MiB file or 64 MiB package limit".into());
                }
                // Detect replacement with a symlink while building the snapshot.
                if path
                    .symlink_metadata()
                    .map_err(|error| format!("Web asset changed while loading: {error}"))?
                    .file_type()
                    .is_symlink()
                    || path
                        .canonicalize()
                        .map_err(|error| format!("Web asset changed while loading: {error}"))?
                        != resolved
                {
                    return Err("Web asset changed while loading".into());
                }
                total_bytes += bytes.len();
                assets.insert(
                    name.clone(),
                    PluginWebAsset {
                        bytes: Bytes::from(bytes),
                        content_type: asset_content_type(&name),
                    },
                );
            }
        }
        for page in &web.pages {
            if !assets.contains_key(&page.entry) {
                return Err(format!(
                    "Web page `{}` entry `{}` is missing",
                    page.id, page.entry
                ));
            }
            for style in &page.styles {
                if !assets.contains_key(style) {
                    return Err(format!(
                        "Web page `{}` stylesheet `{style}` is missing",
                        page.id
                    ));
                }
            }
        }

        let mut digest = blake3::Hasher::new();
        for (name, asset) in &assets {
            digest.update(&(name.len() as u64).to_le_bytes());
            digest.update(name.as_bytes());
            digest.update(&(asset.bytes.len() as u64).to_le_bytes());
            digest.update(&asset.bytes);
        }
        Ok(Self {
            revision: digest.finalize().to_hex().to_string(),
            assets,
        })
    }
}

fn asset_content_type(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "html" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agena_plugin_sdk::{PluginWebAssets, PluginWebPage};
    use std::path::PathBuf;

    struct Package(PathBuf);
    impl Package {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("agena-web-assets-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("web/dist/chunks")).unwrap();
            std::fs::write(
                root.join("web/dist/index.js"),
                b"export function mount() {}",
            )
            .unwrap();
            std::fs::write(
                root.join("web/dist/chunks/data.js"),
                b"export const value = 1",
            )
            .unwrap();
            Self(root)
        }
        fn declaration(&self) -> PluginWebContributions {
            PluginWebContributions {
                assets: PluginWebAssets {
                    root: "web/dist".into(),
                },
                pages: vec![PluginWebPage {
                    id: "home".into(),
                    title: "Home".into(),
                    entry: "index.js".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }
        }
    }
    impl Drop for Package {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn asset_revision_covers_chunks_and_old_snapshot_is_immutable() {
        let package = Package::new();
        let old = PluginWebBundle::load(Some(&package.0), &package.declaration()).unwrap();
        std::fs::write(
            package.0.join("web/dist/chunks/data.js"),
            b"export const value = 2",
        )
        .unwrap();
        let new = PluginWebBundle::load(Some(&package.0), &package.declaration()).unwrap();
        assert_ne!(old.revision, new.revision);
        assert_eq!(
            old.assets["chunks/data.js"].bytes.as_ref(),
            b"export const value = 1"
        );
        assert_eq!(
            new.assets["index.js"].content_type,
            "text/javascript; charset=utf-8"
        );
    }

    #[test]
    fn loading_requires_explicit_root_and_declared_entries() {
        let package = Package::new();
        assert!(PluginWebBundle::load(None, &package.declaration()).is_err());
        let mut web = package.declaration();
        web.assets.root = "../dist".into();
        assert!(PluginWebBundle::load(Some(&package.0), &web).is_err());
        web = package.declaration();
        web.pages[0].entry = "missing.js".into();
        assert!(
            PluginWebBundle::load(Some(&package.0), &web)
                .unwrap_err()
                .contains("missing")
        );
    }

    #[test]
    fn oversized_assets_fail_without_publishing_a_partial_bundle() {
        let package = Package::new();
        let file = std::fs::File::create(package.0.join("web/dist/large.bin")).unwrap();
        file.set_len((MAX_FILE_BYTES + 1) as u64).unwrap();
        assert!(PluginWebBundle::load(Some(&package.0), &package.declaration()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn asset_symlinks_and_symlink_roots_are_rejected() {
        let package = Package::new();
        std::os::unix::fs::symlink(
            package.0.join("web/dist/index.js"),
            package.0.join("web/dist/link.js"),
        )
        .unwrap();
        assert!(
            PluginWebBundle::load(Some(&package.0), &package.declaration())
                .unwrap_err()
                .contains("symbolic")
        );
        std::fs::remove_file(package.0.join("web/dist/link.js")).unwrap();
        std::os::unix::fs::symlink(package.0.join("web/dist"), package.0.join("linked")).unwrap();
        let mut web = package.declaration();
        web.assets.root = "linked".into();
        assert!(PluginWebBundle::load(Some(&package.0), &web).is_err());
    }
}
