fn main() {
    println!("cargo:rerun-if-changed=src/server/macos_permission_host.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // Embed a tiny native supervisor so installing a release never requires
    // Xcode/clang. Its installed copy is deliberately preserved across upgrades:
    // macOS TCC grants belong to this stable code identity, not the changing
    // Rust server executable.
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
        .join("agena-permission-host");
    let compiler = cc::Build::new()
        .cargo_metadata(false)
        .opt_level(2)
        .debug(false)
        .get_compiler();
    let status = compiler
        .to_command()
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg("src/server/macos_permission_host.c")
        .arg("-o")
        .arg(output)
        .status()
        .expect("failed to compile the macOS permission host");
    assert!(status.success(), "macOS permission host compilation failed");
}
