//! A compiled-in source fingerprint identifies the tool-runtime source that
//! produced this binary, not whatever checkout happens to be on disk later.
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
fn files(root: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("read build fingerprint directory");
        let path = entry.path();
        let kind = entry.file_type().expect("read fingerprint file type");
        if kind.is_dir() {
            files(&path, output)
        } else if kind.is_file() {
            output.push(path);
        }
    }
}
fn main() {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let crates = manifest.parent().expect("crates directory");
    let mut hash = Sha256::new();
    let mut scopes = Vec::new();
    for name in [
        "agena-bundled-plugins",
        "agena-runtime-tools",
        "agena-runtime-contracts",
        "agena-process",
        "agena-scheduler",
        "agena-lsp",
    ] {
        let root = crates.join(name);
        if !root.join("src").is_dir() {
            continue;
        }
        scopes.push(name);
        let mut paths = Vec::new();
        files(&root.join("src"), &mut paths);
        paths.push(root.join("Cargo.toml"));
        paths.sort();
        println!("cargo:rerun-if-changed={}", root.join("src").display());
        println!(
            "cargo:rerun-if-changed={}",
            root.join("Cargo.toml").display()
        );
        for path in paths {
            hash.update(name.as_bytes());
            hash.update(
                path.strip_prefix(&root)
                    .expect("scoped path")
                    .to_string_lossy()
                    .as_bytes(),
            );
            hash.update([0]);
            hash.update(fs::read(&path).expect("read source for build fingerprint"));
            hash.update([0]);
        }
    }
    println!(
        "cargo:rustc-env=AGENA_TOOL_SOURCE_FINGERPRINT={}",
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    println!(
        "cargo:rustc-env=AGENA_TOOL_SOURCE_SCOPE={}",
        scopes.join(",")
    );
    println!(
        "cargo:rustc-env=AGENA_TOOL_BUILD_TARGET={}",
        std::env::var("TARGET").expect("target triple")
    );
}
