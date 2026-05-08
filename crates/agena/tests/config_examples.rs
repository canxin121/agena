//! Ensure the shipped example configs (`config.example.toml` and
//! `config.full.toml`) parse against the live `RawConfig`/`ConfigLoader`.
//! These files double as documentation; if a field is renamed or removed,
//! this test catches the drift.

use std::fs;
use std::io::Write;

use agena::config::{ConfigLoader, LoadConfigRequest, ProcessEnvironment};

fn try_load(content: &str, label: &str) {
    let mut tmp = tempfile::NamedTempFile::new().expect("create tmp file");
    tmp.write_all(content.as_bytes()).expect("write config");
    let path = tmp.path().to_path_buf();
    let loader = ConfigLoader::new(ProcessEnvironment);
    let request = LoadConfigRequest {
        config_path: Some(path),
        mode: None,
        overrides: Vec::new(),
    };
    if let Err(err) = loader.load(&request) {
        panic!("{label} failed to parse: {err:#}");
    }
}

#[test]
fn config_example_toml_parses() {
    let content = include_str!("../../../config.example.toml");
    try_load(content, "config.example.toml");
}

#[test]
fn config_full_toml_parses() {
    let content = include_str!("../../../config.full.toml");
    try_load(content, "config.full.toml");
}

#[test]
fn config_example_toml_exists_on_disk() {
    let metadata = fs::metadata(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config.example.toml"
    ))
    .expect("config.example.toml present at repo root");
    assert!(metadata.is_file());
}
