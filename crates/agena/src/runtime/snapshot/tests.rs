use super::build_profile_agent;
use crate::config::{ConfigLoader, LoadConfigRequest, ProcessEnvironment};
use crate::permission::{AccessKind, PermissionDecision};
use std::path::Path;

#[test]
fn build_profile_agent_uses_only_profile_permission() {
    let config_path = std::env::temp_dir().join(format!(
        "agena-runtime-snapshot-empty-test-{}.json",
        std::process::id()
    ));
    std::fs::write(&config_path, "{}").expect("test config should be written");

    let resolution = ConfigLoader::<ProcessEnvironment>::new(ProcessEnvironment)
        .load(&LoadConfigRequest {
            config_path: Some(config_path.clone()),
            ..LoadConfigRequest::default()
        })
        .expect("default config should load");

    let agent = build_profile_agent(
        "build",
        crate::agent::PermissionConfig::default(),
        &resolution,
    );

    assert_eq!(
        agent.authorize_path_access(
            AccessKind::Write,
            Path::new("/workspace"),
            Path::new("/workspace/file.txt"),
        ),
        PermissionDecision::Allow
    );
}
