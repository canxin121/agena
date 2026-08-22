//! Forward rustc invocations while exposing the real target build-std artifacts
//! to build scripts that invoke rustc directly.
//!
//! Cargo adds the target `core`/`std` artifacts to ordinary rustc commands. A
//! build script such as autocfg can instead invoke the RUSTC executable itself
//! and omit Cargo's `--extern` arguments. That is especially visible for
//! Windows build-std target specs, whose standard library is built in the
//! target directory rather than being present in the host sysroot. This
//! wrapper only augments an explicit Windows/Cygwin build-std target invocation
//! and only with artifacts discovered below the target's real build directory.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn target_argument(args: &[OsString]) -> Option<String> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--target" {
            return args
                .get(index + 1)
                .and_then(|value| value.to_str())
                .map(str::to_owned);
        }
        if let Some(value) = arg
            .to_str()
            .and_then(|value| value.strip_prefix("--target="))
        {
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
        if let Some(value) = arg
            .to_str()
            .and_then(|value| value.strip_prefix("--crate-name="))
        {
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

fn has_cfg_feature(args: &[OsString], feature: &str) -> bool {
    let expected = format!("feature=\"{feature}\"");
    args.iter().any(|arg| {
        arg.to_str().is_some_and(|value| {
            value == expected
                || value == format!("--cfg={expected}")
                || value == format!("--cfg {expected}")
        })
    })
}

fn source_path(args: &[OsString]) -> Option<&Path> {
    args.iter().find_map(|arg| {
        let path = Path::new(arg);
        path.extension()
            .is_some_and(|extension| extension == "rs")
            .then_some(path)
    })
}

fn is_no_std_crate(args: &[OsString]) -> bool {
    let Some(source_path) = source_path(args) else {
        return false;
    };
    let Ok(source) = fs::read_to_string(source_path) else {
        return false;
    };
    if source.contains("#![no_std]") {
        return true;
    }

    // A number of crates used by build-std spell this as
    // `cfg_attr(not(feature = "std"), no_std)`.  Rustc receives the feature
    // cfgs in the same invocation, so this remains correct when the crate is
    // built with its optional `std` feature enabled.
    let conditional_no_std = source.contains("no_std")
        && (source.contains("not(feature = \"std\")")
            || source.contains("not(feature=\"std\")")
            || source.contains("not (feature = \"std\")"));
    conditional_no_std && !has_cfg_feature(args, "std")
}

fn is_rust_sysroot_build() -> bool {
    // build-std compiles several Rust sysroot crates through the same RUSTC
    // wrapper.  Their build scripts have crate-name `build_script_build`, so
    // checking only the rustc crate name cannot distinguish the std build
    // script from an ordinary dependency build script.  The manifest path is
    // supplied by Cargo and is a stable, target-independent way to identify
    // the real Rust source tree.  Let all of those sysroot invocations pass
    // through; Cargo owns their exact dependency paths and extern arguments.
    let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") else {
        return false;
    };
    let manifest_dir = manifest_dir.to_string_lossy().to_ascii_lowercase();
    manifest_dir.contains("rustlib")
        && manifest_dir.contains("src")
        && manifest_dir.contains("library")
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
        let standard_library_name = ["core-", "libcore-", "std-", "libstd-"]
            .iter()
            .any(|prefix| name.starts_with(prefix));
        if (name.ends_with(".rlib") || name.ends_with(".rmeta")) && standard_library_name {
            if let Some(parent) = path.parent() {
                if !directories.iter().any(|candidate| candidate == parent) {
                    directories.push(parent.to_owned());
                }
            }
            artifacts.push(path);
        }
    }
}

fn discover_artifacts(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut directories = Vec::new();
    let mut artifacts = Vec::new();
    collect_artifacts(root, &mut directories, &mut artifacts);
    directories.sort();
    (directories, artifacts)
}

fn find_artifact(artifacts: &[PathBuf], crate_name: &str, extension: &str) -> Option<PathBuf> {
    artifacts
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| {
                    (name.starts_with(&format!("{crate_name}-"))
                        || name.starts_with(&format!("lib{crate_name}-")))
                        && name.ends_with(extension)
                })
        })
        .min_by(|left, right| left.as_os_str().cmp(right.as_os_str()))
        .cloned()
}

fn wait_for_artifact(root: &Path, crate_name: &str) -> Option<(Vec<PathBuf>, PathBuf)> {
    // Cargo can start a build-script probe while build-std is still compiling
    // core/std in a sibling job. Wait for the real artifact to be published,
    // but keep a hard bound so a broken build-std invocation still fails.
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let (directories, artifacts) = discover_artifacts(root);
        if let Some(artifact) = find_artifact(&artifacts, crate_name, ".rlib")
            .or_else(|| find_artifact(&artifacts, crate_name, ".rmeta"))
        {
            return Some((directories, artifact));
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
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
    let is_windows_or_cygwin_build_std = target.as_deref().is_some_and(|value| {
        value.ends_with("-windows-msvc")
            || value.ends_with("-windows-gnu")
            || value.ends_with("-pc-cygwin")
    });

    let mut command = Command::new(real_rustc);
    command.args(&args);

    if is_windows_or_cygwin_build_std {
        // Cargo's own Windows/Cygwin build-std compilation already has its exact externs, and
        // normal target compilation does too. Only direct probe crates need
        // these additions. Never inject host or synthetic standard-library
        // artifacts, and never try to bootstrap core/std while they are being
        // built (their output does not exist yet).
        let building_rust_sysroot =
            matches!(crate_name(&args), Some("core" | "std" | "alloc")) || is_rust_sysroot_build();
        let is_print_query = args.iter().any(|arg| {
            arg.to_str()
                .is_some_and(|value| value == "--print" || value.starts_with("--print="))
        });
        let (_, artifacts) = discover_artifacts(&build_root);
        let cargo_supplied_standard_library =
            has_extern(&args, "core") || has_extern(&args, "std") || has_extern(&args, "alloc");
        if !building_rust_sysroot && !is_print_query && !cargo_supplied_standard_library {
            // Cargo target invocations already carry the exact build-std --extern paths. Adding
            // the probe output directories to those commands makes rustc discover a second copy
            // of core and reports duplicate inherent methods (for example in hashbrown). A
            // direct build-script probe is identifiable by the absence of all standard-library
            // externs, so augment only that narrow case.
            let mut standard_artifacts = Vec::new();
            for standard_crate in ["core", "std"] {
                if standard_crate == "std" && is_no_std_crate(&args) {
                    continue;
                }
                if has_extern(&args, standard_crate) {
                    continue;
                }
                let found = find_artifact(&artifacts, standard_crate, ".rlib")
                    .or_else(|| find_artifact(&artifacts, standard_crate, ".rmeta"))
                    .map(|artifact| (Vec::new(), artifact))
                    .or_else(|| wait_for_artifact(&build_root, standard_crate));
                let Some((_, artifact)) = found else {
                    panic!(
                        "target build-std artifact for {standard_crate} was not found below {} after waiting 300 seconds",
                        build_root.display()
                    );
                };
                standard_artifacts.push((standard_crate, artifact));
            }
            for (standard_crate, artifact) in standard_artifacts {
                if let Some(directory) = artifact.parent() {
                    command
                        .arg("-L")
                        .arg(format!("dependency={}", directory.display()));
                }
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
