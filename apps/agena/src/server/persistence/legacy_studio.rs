//! One-time migration for data created by the retired Studio product.
//!
//! This is the only active source owner allowed to mention the legacy product
//! identity. The migration is conservative: canonical data wins, and a
//! successful run is marked so it never overwrites new server state.

use std::path::{Path, PathBuf};

const MIGRATION_MARKER: &str = ".legacy-studio-migrated-v1";

pub(crate) async fn migrate_once(canonical_root: &Path) -> Result<(), String> {
    tokio::fs::create_dir_all(canonical_root)
        .await
        .map_err(|error| error.to_string())?;
    let marker = canonical_root.join(MIGRATION_MARKER);
    if tokio::fs::try_exists(&marker).await.unwrap_or(false) {
        return Ok(());
    }

    for legacy_root in legacy_roots() {
        if !tokio::fs::try_exists(&legacy_root).await.unwrap_or(false) {
            continue;
        }
        migrate_file_if_missing(
            &legacy_root.join("agena-studio.db"),
            &canonical_root.join("agena.db"),
        )
        .await?;
        migrate_file_if_missing(
            &legacy_root.join("server-settings.json"),
            &canonical_root.join("server-settings.json"),
        )
        .await?;
        migrate_directory_if_missing(&legacy_root.join("ui"), &canonical_root.join("ui")).await?;
        migrate_directory_if_missing(
            &legacy_root.join("terminal"),
            &canonical_root.join("terminal"),
        )
        .await?;
    }

    tokio::fs::write(marker, b"completed\n")
        .await
        .map_err(|error| error.to_string())
}

fn legacy_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = home_dir() {
        roots.push(home.join(".config").join("agena-studio"));
        roots.push(home.join("agena").join("server"));
    }
    roots
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

async fn migrate_file_if_missing(source: &Path, destination: &Path) -> Result<(), String> {
    if !tokio::fs::try_exists(source).await.unwrap_or(false)
        || tokio::fs::try_exists(destination).await.unwrap_or(false)
    {
        return Ok(());
    }
    tokio::fs::copy(source, destination)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

async fn migrate_directory_if_missing(source: &Path, destination: &Path) -> Result<(), String> {
    if !tokio::fs::try_exists(source).await.unwrap_or(false)
        || tokio::fs::try_exists(destination).await.unwrap_or(false)
    {
        return Ok(());
    }
    copy_directory(source, destination).await
}

async fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    tokio::fs::create_dir_all(destination)
        .await
        .map_err(|error| error.to_string())?;
    let mut entries = tokio::fs::read_dir(source)
        .await
        .map_err(|error| error.to_string())?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .await
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            Box::pin(copy_directory(&source_path, &destination_path)).await?;
        } else {
            migrate_file_if_missing(&source_path, &destination_path).await?;
        }
    }
    Ok(())
}
