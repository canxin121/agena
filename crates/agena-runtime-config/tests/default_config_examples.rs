use std::path::Path;

use agena_runtime_config::{RawConfig, validate_config_text};
use serde_json::json;

fn assert_supported_defaults(name: &str, text: &str) {
    let document: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(document["sandbox"], json!(true), "{name}");
    assert_eq!(document["resource_guard"], json!(true), "{name}");
    validate_config_text(Path::new(name), text).unwrap_or_else(|error| panic!("{name}: {error}"));
    let config = serde_json::from_str::<RawConfig>(text)
        .unwrap()
        .resolve()
        .unwrap();
    let permission = serde_json::to_value(config.permission).unwrap();
    assert_eq!(permission["path"]["workspace"], "readwrite", "{name}");
    assert_eq!(permission["path"]["external"], "deny", "{name}");
    assert_eq!(permission["network"], json!({"default":"allow"}), "{name}");
    assert!(config.sandbox, "{name}");
    assert!(config.resource_guard, "{name}");
}

#[test]
fn example_config_uses_supported_project_and_network_defaults() {
    assert_supported_defaults(
        "config.example.json",
        include_str!("../../../config.example.json"),
    );
}

#[test]
fn full_config_uses_supported_project_and_network_defaults() {
    assert_supported_defaults(
        "config.full.json",
        include_str!("../../../config.full.json"),
    );
}
