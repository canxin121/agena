#![cfg(all(unix, feature = "signing"))]

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agena_plugin_host::config::{ConfiguredPlugin, PluginPackage, RestartMode, RestartPolicy};
use agena_plugin_host::host::HostHandle;
use agena_plugin_host::loader::{PreparedPlugin, prepare_entry};
use agena_plugin_host::sdk::{PluginManifest, host_api::NoopHostClient, rpc::method};
use agena_plugin_host::status::{PluginRunState, PluginStatus};
use sha2::{Digest, Sha256};

const FIXTURE: &str = include_str!("fixtures/stdio_plugin.py");

struct Fixture {
    root: tempfile::TempDir,
    executable: PathBuf,
    starts: PathBuf,
    host: Arc<HostHandle>,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let executable = bin.join("agena-checksum-fixture");
        std::fs::write(&executable, FIXTURE).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let starts = root.path().join("starts");
        let host = Arc::new(HostHandle::new(Arc::new(NoopHostClient)));
        host.status_registry().set(PluginStatus::initial(
            &"test.checked".parse().unwrap(),
            "stdio",
        ));
        Self {
            root,
            executable,
            starts,
            host,
        }
    }

    async fn prepare(
        &self,
        command: &str,
        digest: String,
    ) -> Result<PreparedPlugin, agena_plugin_host::HostError> {
        let configured = ConfiguredPlugin {
            package: PluginPackage::Stdio {
                command: command.into(),
                args: vec![],
                env: BTreeMap::from([
                    ("PATH".into(), "bin".into()),
                    (
                        "AGENA_TEST_STARTS".into(),
                        self.starts.to_str().unwrap().into(),
                    ),
                    (
                        "AGENA_TEST_MANIFEST".into(),
                        serde_json::to_string(&PluginManifest::new("test", "checked", "1.0.0"))
                            .unwrap(),
                    ),
                ]),
                cwd: Some(self.root.path().to_owned()),
                restart: RestartPolicy {
                    policy: RestartMode::Always,
                    max_retries: 1,
                    min_backoff: agena_plugin_host::config::DurationSpec(Duration::from_millis(1)),
                    max_backoff: agena_plugin_host::config::DurationSpec(Duration::from_millis(1)),
                },
                sha256: Some(digest),
            },
            ..ConfiguredPlugin::default()
        };
        prepare_entry(
            "test.checked",
            &configured,
            &mut HashMap::new(),
            Arc::clone(&self.host),
            self.root.path(),
            &|_| None,
            &BTreeMap::new(),
        )
        .await
    }
}

#[tokio::test]
async fn loader_rejects_checksum_mismatch_for_path_and_relative_commands() {
    for command in ["agena-checksum-fixture", "./bin/agena-checksum-fixture"] {
        let fixture = Fixture::new();
        let result = fixture.prepare(command, "00".repeat(32)).await;
        if let Ok(prepared) = &result {
            prepared.transport.close().await.unwrap();
        }
        let error = result
            .err()
            .expect("configured checksum must be verified before executing");
        assert!(error.to_string().contains("sha256 mismatch"), "{error}");
        assert!(
            !fixture.starts.exists(),
            "mismatched executable must never start"
        );
    }
}

#[tokio::test]
async fn loader_verifies_the_executable_in_the_plugins_working_directory() {
    for command in ["agena-checksum-fixture", "./bin/agena-checksum-fixture"] {
        let fixture = Fixture::new();
        let prepared = fixture
            .prepare(command, hex::encode(Sha256::digest(FIXTURE.as_bytes())))
            .await
            .unwrap();
        let response = prepared
            .transport
            .dispatch(method::META_PING, serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(response, serde_json::json!({"ok": true}));
        prepared.transport.close().await.unwrap();
        assert_eq!(
            std::fs::read_to_string(&fixture.starts).unwrap(),
            "started\n"
        );
    }
}

#[tokio::test]
async fn supervised_restart_rejects_an_executable_changed_after_initial_verification() {
    let fixture = Fixture::new();
    let prepared = fixture
        .prepare(
            "agena-checksum-fixture",
            hex::encode(Sha256::digest(FIXTURE.as_bytes())),
        )
        .await
        .unwrap();
    prepared
        .transport
        .dispatch(method::META_PING, serde_json::json!({}))
        .await
        .unwrap();
    let replacement = fixture.executable.with_extension("replacement");
    std::fs::write(&replacement, format!("{FIXTURE}\n# different executable\n")).unwrap();
    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::rename(replacement, &fixture.executable).unwrap();
    assert!(
        prepared
            .transport
            .dispatch("test/exit", serde_json::json!({}))
            .await
            .is_err()
    );
    let failed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = fixture
                .host
                .status_registry()
                .get(&"test.checked".parse().unwrap())
                .unwrap();
            if status.state == PluginRunState::Failed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
    prepared.transport.close().await.unwrap();
    failed.expect("checksum failure must terminate the supervised restart");
    assert_eq!(
        std::fs::read_to_string(&fixture.starts).unwrap(),
        "started\n"
    );
}
