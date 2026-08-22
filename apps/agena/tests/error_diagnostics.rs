use std::process::Command;

#[test]
fn server_bootstrap_prints_the_complete_configuration_error_chain() {
    let root = tempfile::tempdir().expect("temporary isolated home");
    let home = root.path().join("home");
    let workspace = root.path().join("workspace");
    std::fs::create_dir_all(home.join("agena")).expect("create config directory");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    std::fs::write(
        home.join("agena/agena.json"),
        r#"{"providers":{"default":"legacy"}}"#,
    )
    .expect("write deliberately retired configuration");

    let output = Command::new(env!("CARGO_BIN_EXE_agena"))
        .args([
            "server",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            "--workspace",
        ])
        .arg(&workspace)
        .env("HOME", &home)
        .env_remove("USERPROFILE")
        .env_remove("RUST_LOG")
        .env_remove("AGENA_SERVER_TOKEN")
        .env_remove("AGENA_SERVER_PASSWORD")
        .env_remove("AGENA_DATABASE_URL")
        .env_remove("AGENA_DATABASE_PATH")
        .output()
        .expect("run agena server");

    assert!(
        !output.status.success(),
        "retired provider default unexpectedly started the server"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Error: configuration error:"), "{stderr}");
    assert!(stderr.contains("failed to build agena runtime"), "{stderr}");
    assert!(stderr.contains("config validation failed"), "{stderr}");
    assert!(stderr.contains("providers.default"), "{stderr}");
    assert!(stderr.contains("is no longer supported"), "{stderr}");
    assert!(stderr.contains("select a model explicitly"), "{stderr}");
    assert!(!stderr.contains("Internal(\""), "{stderr}");
    assert_ne!(stderr.trim(), "Error: failed to build agena runtime");
}

#[test]
fn process_boundary_source_does_not_regress_to_debug_or_outer_only_rendering() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let main = std::fs::read_to_string(manifest.join("src/main.rs")).expect("read main source");
    let process_error =
        std::fs::read_to_string(manifest.join("src/error.rs")).expect("read process error source");
    let server =
        std::fs::read_to_string(manifest.join("src/server/mod.rs")).expect("read server source");

    assert!(main.contains("fn main() -> ExitCode"));
    assert!(main.contains("eprintln!(\"Error: {error}\")"));
    assert!(!main.contains("fn main() -> error::Result<()>"));
    assert!(process_error.contains("format_error_chain"));
    assert!(!server.contains("AgenaProcessError::Internal(error.to_string())"));
}
