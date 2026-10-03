//! A persistent macOS app identity for the installed launchd service.
//!
//! Linker/ad-hoc signatures identify one specific build by its cdhash. Replacing
//! that binary invalidates TCC's remembered grants (Apple TN3127). Keep a tiny
//! signed supervisor, including its Info.plist and seal, unchanged in runtime
//! data and spawn the replaceable server as its child. Never exec the server,
//! re-sign an existing host, or invent a weaker designated requirement.

use std::{
    fs,
    io::IsTerminal as _,
    os::unix::fs::{DirBuilderExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail, ensure};

pub(super) const BUNDLE_ID: &str = "com.agena.permission-host";
const HOST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/agena-permission-host"));
const INFO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.agena.permission-host</string>
  <key>CFBundleName</key><string>Agena</string>
  <key>CFBundleDisplayName</key><string>Agena</string>
  <key>CFBundleExecutable</key><string>Agena</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>LSUIElement</key><true/>
  <key>NSDesktopFolderUsageDescription</key><string>Agena accesses files in the workspaces you choose.</string>
  <key>NSDocumentsFolderUsageDescription</key><string>Agena accesses files in the workspaces you choose.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>Agena accesses files in the workspaces you choose.</string>
  <key>NSRemovableVolumesUsageDescription</key><string>Agena accesses your workspaces on external drives.</string>
  <key>NSNetworkVolumesUsageDescription</key><string>Agena accesses your workspaces on network drives.</string>
</dict></plist>
"#;

pub(super) fn bundle_path() -> Result<PathBuf> {
    let record = super::server_record::record_path();
    let parent = record
        .parent()
        .context("server record path has no parent")?;
    Ok(parent.join("macos").join("Agena.app"))
}

fn executable(bundle: &Path) -> PathBuf {
    bundle.join("Contents/MacOS/Agena")
}

fn validate(bundle: &Path) -> Result<()> {
    ensure!(
        !fs::symlink_metadata(bundle)?.file_type().is_symlink(),
        "the macOS permission host must not be a symlink: {}",
        bundle.display()
    );
    let signature = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "-R"])
        .arg(format!("=identifier \"{BUNDLE_ID}\""))
        .arg(bundle)
        .output()
        .context("failed to verify the macOS permission host signature")?;
    ensure!(
        signature.status.success(),
        "macOS permission host signature is invalid at {}: {}. Restore its original backup; replacing it requires Full Disk Access again",
        bundle.display(),
        String::from_utf8_lossy(&signature.stderr).trim()
    );
    let protocol = Command::new(executable(bundle))
        .arg("--agena-host-protocol")
        .output()
        .context("failed to inspect the macOS permission host protocol")?;
    ensure!(
        protocol.status.success() && protocol.stdout == b"1\n",
        "unsupported macOS permission host at {}; it was preserved to retain its authorization",
        bundle.display()
    );
    Ok(())
}

/// Install only when absent. Even a newly built host with different bytes must
/// reuse the originally authorized app on upgrade, rollback, and reinstall.
fn ensure_bundle_at(bundle: &Path) -> Result<bool> {
    if bundle.try_exists()? {
        validate(bundle)?;
        return Ok(false);
    }
    let parent = bundle.parent().context("permission host has no parent")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    let staged_bundle = staging.path().join("Agena.app");
    let host = executable(&staged_bundle);
    fs::create_dir_all(host.parent().unwrap())?;
    fs::write(&host, HOST)?;
    fs::set_permissions(&host, fs::Permissions::from_mode(0o700))?;
    fs::write(staged_bundle.join("Contents/Info.plist"), INFO)?;
    let signed = Command::new("/usr/bin/codesign")
        .args(["--sign", "-", "--timestamp=none", "--identifier", BUNDLE_ID])
        .arg(&staged_bundle)
        .output()
        .context("failed to sign the macOS permission host")?;
    ensure!(
        signed.status.success(),
        "failed to sign macOS permission host: {}",
        String::from_utf8_lossy(&signed.stderr)
    );
    validate(&staged_bundle)?;
    if let Err(error) = fs::rename(&staged_bundle, bundle) {
        // A concurrent install may have won. Validate and preserve that copy.
        if bundle.try_exists()? {
            validate(bundle)?;
            return Ok(false);
        }
        return Err(error).context("failed to install the macOS permission host");
    }
    Ok(true)
}

pub(super) fn ensure_host() -> Result<PathBuf> {
    let bundle = bundle_path()?;
    if ensure_bundle_at(&bundle)? {
        print_instructions(&bundle);
        // Interactive first install only. Service restarts/updates never open
        // settings or probe protected data to guess the current TCC state.
        if std::io::stdout().is_terminal()
            && std::env::var_os("AGENA_MACOS_OPEN_PERMISSIONS").as_deref()
                != Some(std::ffi::OsStr::new("0"))
            && let Err(error) = open_settings(&bundle)
        {
            eprintln!(
                "Could not open macOS settings: {error}. Run `agena server permissions` later."
            );
        }
    }
    Ok(executable(&bundle))
}

fn print_instructions(bundle: &Path) {
    println!(
        "macOS file access: enable Agena in System Settings > Privacy & Security > Full Disk Access.\n\
         Add this app with the + button (Cmd+Shift+G accepts its path): {}\n\
         This app is preserved across normal upgrades. After granting access, restart the server:\n\
         agena server stop && agena server start\n\
         Reopen setup at any time: agena server permissions",
        bundle.display()
    );
}

fn open_settings(bundle: &Path) -> Result<()> {
    for args in [
        vec![std::ffi::OsStr::new("-R"), bundle.as_os_str()],
        vec![std::ffi::OsStr::new(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
        )],
    ] {
        let status = Command::new("/usr/bin/open")
            .args(args)
            .stdin(Stdio::null())
            .status()?;
        ensure!(status.success(), "macOS open failed with {status}");
    }
    Ok(())
}

pub(super) fn setup() -> Result<()> {
    if !super::user_service::is_installed() {
        bail!(
            "install the macOS user service with `agena server install` first; direct terminal launches use the terminal's privacy identity"
        );
    }
    let bundle = bundle_path()?;
    ensure_bundle_at(&bundle)?;
    print_instructions(&bundle);
    open_settings(&bundle)
}

#[cfg(test)]
mod tests;
