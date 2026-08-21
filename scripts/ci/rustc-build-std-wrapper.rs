//! Forward rustc invocations while exposing the real target build-std artifacts
//! to build scripts that invoke rustc directly.
//!
//! Cargo adds the target `core`/`std` artifacts to ordinary rustc commands. A
//! build script such as autocfg can instead invoke the RUSTC executable itself
//! and omit Cargo's `--extern` arguments. That is especially visible for the
//! Win7 target specs, whose standard library is built in the target directory
//! rather than being present in the host sysroot. This wrapper only augments an
//! explicit Win7 target invocation and only with artifacts discovered below the
//! target's real build directory.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn target_argument(args: &[OsString]) -> Option<String> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--target" {
            return args
                .get(index + 1)
                .and_then(|value| value.to_str())
                .map(str::to_owned);
        }
        if let Some(value) = arg.to_str().and_then(|value| value.strip_prefix("--target=")) {
            return Some(value.to_owned());
        }
    }
    None
}

fn crate_name(args: &[OsString]) -> Option<&str> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--crate-name" {
            return args.get(index + 1).and_then(|value| value.to_str());
        }
        if let Some(value) = arg.to_str().and_then(|value| value.strip_prefix("--crate-name=")) {
            return Some(value);
        }
    }
    None
}

fn has_extern(args: &[OsString], crate_name: &str) -> bool {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--extern" {
            if let Some(value) = args.get(index + 1).and_then(|value| value.to_str()) {
                if value == crate_name || value.starts_with(&format!("{crate_name}=")) {
                    return true;
                }
            }
        } else if let Some(value) = arg
            .to_str()
            .and_then(|value| value.strip_prefix("--extern="))
        {
            if value == crate_name || value.starts_with(&format!("{crate_name}=")) {
                return true;
            }
        }
    }
    false
}

fn collect_artifacts(root: &Path, directories: &mut Vec<PathBuf>, artifacts: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_artifacts(&path, directories, artifacts);
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if (name.ends_with(".rlib") || name.ends_with(".rmeta"))
            && (name.starts_with("libcore-") || name.starts_with("libstd-"))
        {
            if let Some(parent) = path.parent() {
                if !directories.iter().any(|candidate| candidate == parent) {
                    directories.push(parent.to_owned());
                }
            }
            artifacts.push(path);
        }
    }
}

fn find_artifact(artifacts: &[PathBuf], crate_name: &str, extension: &str) -> Option<PathBuf> {
    artifacts
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| {
                    name.starts_with(&format!("lib{crate_name}-")) && name.ends_with(extension)
                })
        })
        .min_by(|left, right| left.as_os_str().cmp(right.as_os_str()))
        .cloned()
}

fn main() {
    let real_rustc = env::var_os("AGENA_REAL_RUSTC")
        .expect("AGENA_REAL_RUSTC is required by the build-std rustc wrapper");
    let build_root = PathBuf::from(
        env::var_os("AGENA_BUILD_STD_ROOT")
            .expect("AGENA_BUILD_STD_ROOT is required by the build-std rustc wrapper"),
    );
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let target = target_argument(&args);
    let is_win7 = target.as_deref().is_some_and(|value| {
        value.ends_with("-win7-windows-msvc") || value.ends_with("-win7-windows-gnu")
    });

    let mut command = Command::new(real_rustc);
    command.args(&args);

    if is_win7 {
        let mut directories = Vec::new();
        let mut artifacts = Vec::new();
        collect_artifacts(&build_root, &mut directories, &mut artifacts);
        directories.sort();
        for directory in directories {
            command.arg("-L").arg(format!("dependency={}", directory.display()));
        }

        // Cargo's own build-std compilation already has its exact externs, and
        // normal target compilation does too. Only direct probe crates need
        // these additions. Never inject host or synthetic standard-library
        // artifacts, and never try to bootstrap core/std while they are being
        // built (their output does not exist yet).
        let building_std = matches!(crate_name(&args), Some("core" | "std" | "alloc"));
        let is_print_query = args.iter().any(|arg| arg == "--print");
        if !building_std && !is_print_query {
            for standard_crate in ["core", "std"] {
                if has_extern(&args, standard_crate) {
                    continue;
                }
                let artifact = find_artifact(&artifacts, standard_crate, ".rlib")
                    .or_else(|| find_artifact(&artifacts, standard_crate, ".rmeta"))
                    .unwrap_or_else(|| {
                        panic!(
                            "target build-std artifact for {standard_crate} was not found below {}",
                            build_root.display()
                        )
                    });
                command
                    .arg("--extern")
                    .arg(format!("{standard_crate}={}", artifact.display()));
            }
        }
    }

    let status = command
        .status()
        .expect("failed to execute the real Rust compiler");
    std::process::exit(status.code().unwrap_or(1));
}
