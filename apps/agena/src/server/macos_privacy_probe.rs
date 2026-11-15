//! Public macOS privacy APIs, executed as a child of the stable permission host.
//! This entry point runs before loading runtime configuration or credentials.

use std::{
    ffi::c_void, fs::OpenOptions, io::Write as _, os::unix::fs::OpenOptionsExt as _, path::Path,
    ptr,
};

use anyhow::{Context as _, ensure};
use serde::{Deserialize, Serialize};

pub(super) const ARGUMENT: &str = "--agena-macos-permission-probe";
pub(super) const REPORT_ARGUMENT: &str = "--report";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PermissionProbe {
    pub schema_version: u32,
    /// None means there was no local graphical session to inspect.
    pub accessibility: Option<bool>,
    pub screen_recording: Option<bool>,
}

impl PermissionProbe {
    pub(super) fn unknown() -> Self {
        Self {
            schema_version: 1,
            accessibility: None,
            screen_recording: None,
        }
    }

    pub(super) fn desktop_permissions_granted(&self) -> bool {
        self.accessibility == Some(true) && self.screen_recording == Some(true)
    }
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    static kAXTrustedCheckOptionPrompt: *const c_void;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const c_void;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFRelease(value: *const c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

fn inspect(request: bool) -> anyhow::Result<PermissionProbe> {
    // These APIs are called synchronously on the process main thread. The
    // dictionary borrows immortal framework constants; no retain callbacks are
    // needed. AX prompting is asynchronous, so the parent launches a fresh
    // instance to recheck after the person finishes in System Settings.
    let accessibility = unsafe {
        if request {
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                ptr::null(),
                ptr::null(),
            );
            ensure!(!options.is_null(), "cannot allocate macOS privacy options");
            let granted = AXIsProcessTrustedWithOptions(options) != 0;
            CFRelease(options);
            granted
        } else {
            AXIsProcessTrustedWithOptions(ptr::null()) != 0
        }
    };
    let screen_recording = unsafe {
        let granted = CGPreflightScreenCaptureAccess();
        if request && !granted {
            CGRequestScreenCaptureAccess()
        } else {
            granted
        }
    };
    Ok(PermissionProbe {
        schema_version: 1,
        accessibility: Some(accessibility),
        screen_recording: Some(screen_recording),
    })
}

fn write_report(path: &Path, report: &PermissionProbe) -> anyhow::Result<()> {
    ensure!(
        path.is_absolute(),
        "macOS privacy report path must be absolute"
    );
    // The parent owns a mode-0700 temporary directory. create_new rejects an
    // existing file or symlink, and the child's result is never group-readable.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .context("cannot create private macOS privacy report")?;
    serde_json::to_writer(&mut file, report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn try_run_cli() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(ARGUMENT)) {
        return None;
    }
    let request = match args.next().as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("request") => true,
        Some("check") => false,
        _ => {
            eprintln!("macOS permission probe expects request or check");
            return Some(2);
        }
    };
    if args.next().as_deref() != Some(std::ffi::OsStr::new(REPORT_ARGUMENT)) {
        eprintln!("macOS permission probe requires --report with a private absolute path");
        return Some(2);
    }
    let Some(report_path) = args.next().map(std::path::PathBuf::from) else {
        eprintln!("macOS permission probe requires a report path");
        return Some(2);
    };
    if args.next().is_some() || !report_path.is_absolute() {
        eprintln!("invalid macOS permission probe arguments");
        return Some(2);
    }
    match inspect(request).and_then(|report| write_report(&report_path, &report)) {
        Ok(()) => Some(0),
        Err(error) => {
            eprintln!("macOS permission probe failed: {error:#}");
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    #[test]
    fn report_is_private_exclusive_and_never_replaces_a_symlink() {
        let directory = tempfile::tempdir().unwrap();
        let report = PermissionProbe {
            schema_version: 1,
            accessibility: Some(true),
            screen_recording: Some(false),
        };
        let path = directory.path().join("report.json");
        write_report(&path, &report).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(write_report(&path, &report).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);

        let link = directory.path().join("symlink.json");
        symlink(&path, &link).unwrap();
        assert!(write_report(&link, &report).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn headless_status_is_unknown_instead_of_granted_or_denied() {
        let report = PermissionProbe::unknown();
        assert!(!report.desktop_permissions_granted());
        let value = serde_json::to_value(report).unwrap();
        assert!(value["accessibility"].is_null());
        assert!(value["screen_recording"].is_null());
    }
}
