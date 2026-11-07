//! Plugin project authoring is tested through the *real Agena CLI*, in a
//! case-owned HOME/data directory, without contacting a remote SDK registry.

use std::{ffi::OsString, path::Path};

use super::fixture;
use agena_testkit::TestResult;
use anyhow::{Context, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn path_arg(path: &Path) -> OsString {
    path.as_os_str().to_os_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn marketplace_cli_scaffolds_validates_and_packages_a_standalone_stdio_plugin() -> TestResult
{
    fixture()
        .run(|case| async move {
            let root = case.paths().root.join("my-plugins/example");
            let args = vec![
                "plugin".into(),
                "init".into(),
                path_arg(&root),
                "--id".into(),
                "qa.echo".into(),
                "--kind".into(),
                "stdio".into(),
                "--description".into(),
                "Standalone example".into(),
                "--repository".into(),
                "https://github.com/example/echo".into(),
                "--skip-lockfile".into(),
            ];
            case.run_cli(args.clone()).await?.require_success()?;
            ensure!(root.join("src/main.rs").is_file());
            ensure!(
                !root.join("Cargo.lock").exists(),
                "skip-lockfile unexpectedly fetched dependencies"
            );
            ensure!(root.join("agena-plugin.toml").is_file());
            ensure!(root.join(".github/workflows").is_dir());
            let cargo = tokio::fs::read_to_string(root.join("Cargo.toml")).await?;
            let source = tokio::fs::read_to_string(root.join("src/main.rs")).await?;
            ensure!(
                cargo.contains("agena-plugin-sdk") && cargo.contains("src/main.rs"),
                "{cargo}"
            );
            ensure!(
                source.contains("namespace = \"qa\"") && source.contains("name = \"echo\""),
                "{source}"
            );
            let manifest = tokio::fs::read_to_string(root.join("agena-plugin.toml")).await?;
            ensure!(manifest.contains("id = \"qa.echo\"") && manifest.contains("kind = \"stdio\""));

            let validate = case
                .run_cli(vec![
                    "plugin".into(),
                    "validate".into(),
                    path_arg(&root),
                    "--format".into(),
                    "json".into(),
                ])
                .await?;
            validate.require_success()?;
            let result: Value = serde_json::from_str(&validate.stdout)
                .context("product plugin validation did not return JSON")?;
            ensure!(
                result["ok"] == true && result["target_kind"] == "plugin_project",
                "{result}"
            );

            // Packaging tests filesystem layout, project manifest parsing, archive
            // production, and digest emission. No fake plugin is loaded by Agena.
            let artifact = case
                .seed_file("my-plugins/example/dist-bin/echo", b"offline test artifact")
                .await?;
            let dist = root.join("dist");
            let packaged = case
                .run_cli(vec![
                    "plugin".into(),
                    "package".into(),
                    "--manifest".into(),
                    path_arg(&root.join("agena-plugin.toml")),
                    "--artifact".into(),
                    path_arg(&artifact),
                    "--target".into(),
                    "offline-system-test".into(),
                    "--output".into(),
                    path_arg(&dist),
                ])
                .await?;
            packaged.require_success()?;
            let archive = dist.join("qa-echo-v0.1.0-offline-system-test.tar.gz");
            let fragment = dist.join("agena-plugin-release.offline-system-test.json");
            ensure!(
                archive.is_file() && fragment.is_file(),
                "package did not produce archive and fragment: {}",
                packaged.stdout
            );
            let bytes = tokio::fs::read(&archive).await?;
            ensure!(bytes.len() > b"offline test artifact".len());
            let digest = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            ensure!(
                packaged.stdout.contains(&digest),
                "CLI output did not report actual archive digest"
            );
            let fragment_json: Value = serde_json::from_slice(&tokio::fs::read(&fragment).await?)?;
            ensure!(fragment_json.to_string().contains(&digest));

            // Verify the reverse direction as well: the actual archive
            // produced by `agena plugin package` must be accepted by the
            // production installer, extracted under the private cache and
            // referenced by the runtime configuration. This catches overly
            // restrictive archive security rules that reject valid packages.
            let local_catalog = json!({
                "version": 1,
                "marketplace": {"name": "Packaged Archive Fixture", "description": "Offline test registry"},
                "plugins": [{
                    "id": "qa.echo", "versions": [{
                        "version": "0.1.0", "kind": "stdio", "platform": "any",
                        "url": format!("file://{}", archive.display()),
                        "sha256": digest,
                        "archive": {"format": "tar_gz", "entrypoint": "echo"},
                    }]
                }],
            });
            let catalog = case.seed_file(
                "my-plugins/example/local-archive-registry.json",
                serde_json::to_vec(&local_catalog)?,
            ).await?;
            let install = case.run_cli(vec![
                "plugin".into(), "install".into(), "qa.echo".into(),
                "--registry".into(), path_arg(&catalog),
                "--registry-id".into(), "archive-fixture".into(),
            ]).await?;
            install.require_success()?;
            let current: Value = serde_json::from_slice(
                &tokio::fs::read(&case.paths().global_config).await?
            )?;
            let entrypoint = current["plugins"]["list"]["qa.echo"]["package"]["command"]
                .as_str().context("archive installer did not publish the extracted entrypoint")?;
            ensure!(tokio::fs::read(entrypoint).await? == b"offline test artifact",
                "archive install did not preserve source artifact bytes");
            case.run_cli(["plugin", "uninstall", "qa.echo"]).await?.require_success()?;

            let no_clobber = case.run_cli(args).await?;
            no_clobber.require_failure()?;
            ensure!(root.join("src/main.rs").is_file());
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn marketplace_cli_cdylib_template_and_invalid_requests_leave_no_unowned_changes()
-> TestResult {
    fixture()
        .run(|case| async move {
            let root = case.paths().root.join("my-plugins/dynamic");
            case.run_cli(vec![
                "plugin".into(),
                "init".into(),
                path_arg(&root),
                "--id".into(),
                "qa.dynamic".into(),
                "--kind".into(),
                "cdylib".into(),
                "--skip-lockfile".into(),
            ])
            .await?
            .require_success()?;
            ensure!(root.join("src/lib.rs").is_file());
            ensure!(!root.join("src/main.rs").exists());
            let cargo = tokio::fs::read_to_string(root.join("Cargo.toml")).await?;
            ensure!(cargo.contains("cdylib") && cargo.contains("abi_stable"));
            let validation = case
                .run_cli(vec!["plugin".into(), "validate".into(), path_arg(&root)])
                .await?;
            validation.require_success()?;

            let invalid = case.paths().root.join("my-plugins/invalid-id");
            let bad_id = case
                .run_cli(vec![
                    "plugin".into(),
                    "init".into(),
                    path_arg(&invalid),
                    "--id".into(),
                    "../invalid".into(),
                    "--skip-lockfile".into(),
                ])
                .await?;
            bad_id.require_failure()?;
            ensure!(
                !invalid.exists(),
                "invalid plugin id unexpectedly created a project"
            );

            let invalid_repo = case.paths().root.join("my-plugins/invalid-repository");
            let bad_repo = case
                .run_cli(vec![
                    "plugin".into(),
                    "init".into(),
                    path_arg(&invalid_repo),
                    "--id".into(),
                    "qa.badrepo".into(),
                    "--repository".into(),
                    "http://example.com/unsafe".into(),
                    "--skip-lockfile".into(),
                ])
                .await?;
            bad_repo.require_failure()?;
            ensure!(
                !invalid_repo.exists(),
                "invalid repository unexpectedly created a project"
            );

            let before = tokio::fs::read(root.join("Cargo.toml")).await?;
            case.seed_file(
                "my-plugins/dynamic/custom-notes.txt",
                b"must survive force update",
            )
            .await?;
            let rerun = case
                .run_cli(vec![
                    "plugin".into(),
                    "init".into(),
                    path_arg(&root),
                    "--id".into(),
                    "qa.dynamic".into(),
                    "--kind".into(),
                    "cdylib".into(),
                    "--force".into(),
                    "--skip-lockfile".into(),
                ])
                .await?;
            rerun.require_success()?;
            ensure!(tokio::fs::read(root.join("Cargo.toml")).await? == before);
            ensure!(
                tokio::fs::read(root.join("custom-notes.txt")).await?
                    == b"must survive force update"
            );
            ensure!(
                case.run_cli([
                    "plugin",
                    "init",
                    "--data-dir",
                    "/tmp/someone-else",
                    "--id",
                    "qa.reject"
                ])
                .await
                .is_err(),
                "CLI fixture accepted a user data-dir override"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn marketplace_cli_local_registry_dry_run_verified_install_and_uninstall() -> TestResult {
    fixture().run(|case| async move {
        let payload = b"#!/bin/sh\nexit 0\n";
        let artifact = case.seed_file("marketplace/artifacts/test-stdio", payload).await?;
        let digest = Sha256::digest(payload).iter().map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let index = json!({
            "version": 1,
            "marketplace": {"name":"Local Agena Test Registry", "description":"Offline registry"},
            "plugins": [{
                "id":"qa.localplugin", "name":"Offline fixture",
                "versions": [{
                    "version":"0.1.0", "platform":"any", "kind":"stdio",
                    "url": format!("file://{}", artifact.display()),
                    "sha256": digest,
                    "args": [], "env": {}, "settings": null
                }]
            }]
        });
        let registry = case.seed_file("marketplace/catalog.json", serde_json::to_vec_pretty(&index)?).await?;
        let args = vec![
            "plugin".into(), "install".into(), "qa.localplugin".into(),
            "--registry".into(), path_arg(&registry),
            "--registry-id".into(), "testkit-local".into(),
        ];
        let original = tokio::fs::read(&case.paths().global_config).await?;
        let mut dry_args = args.clone();
        dry_args.push("--dry-run".into());
        let dry = case.run_cli(dry_args).await?;
        dry.require_success()?;
        ensure!(dry.stdout.contains("DRY-RUN"), "{dry:?}");
        ensure!(tokio::fs::read(&case.paths().global_config).await? == original,
            "dry-run unexpectedly changed the config");
        let listed = case.run_cli(["plugin", "list-installed"]).await?;
        listed.require_success()?;
        ensure!(listed.stdout.contains("no plugins installed"), "{listed:?}");

        // SHA-256 alone must not satisfy an explicit signature policy.
        // A rejected install must preserve both config and installed records.
        let mut require_signed = args.clone();
        require_signed.push("--require-signature".into());
        let unsigned = case.run_cli(require_signed).await?;
        unsigned.require_failure()?;
        ensure!(tokio::fs::read(&case.paths().global_config).await? == original,
            "signature-required failure mutated runtime config");
        let after_unsigned = case.run_cli(["plugin", "list-installed"]).await?;
        after_unsigned.require_success()?;
        ensure!(after_unsigned.stdout.contains("no plugins installed"),
            "unsigned artifact was partially installed: {after_unsigned:?}");

        let mut bad_index = index.clone();
        bad_index["plugins"][0]["versions"][0]["sha256"] = json!("0".repeat(64));
        let bad_registry = case.seed_file("marketplace/bad-catalog.json", serde_json::to_vec(&bad_index)?).await?;
        let bad = case.run_cli(vec![
            "plugin".into(), "install".into(), "qa.localplugin".into(),
            "--registry".into(), path_arg(&bad_registry),
            "--registry-id".into(), "bad-catalog".into(),
        ]).await?;
        bad.require_failure()?;
        ensure!(tokio::fs::read(&case.paths().global_config).await? == original,
            "bad digest changed runtime config");

        let installed = case.run_cli(args.clone()).await?;
        installed.require_success()?;
        ensure!(installed.stdout.contains("Installed qa.localplugin v0.1.0"), "{installed:?}");
        let doc: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
        ensure!(doc["plugins"]["list"]["qa.localplugin"]["package"]["kind"] == "stdio",
            "installed config not a real stdio package: {doc}");
        let installed_path = doc["plugins"]["list"]["qa.localplugin"]["package"]["command"]
            .as_str().context("installed plugin command missing")?;
        ensure!(tokio::fs::read(installed_path).await? == payload);
        let listing = case.run_cli(["plugin", "list-installed"]).await?;
        listing.require_success()?;
        ensure!(listing.stdout.contains("qa.localplugin v0.1.0"), "{listing:?}");

        let duplicate = case.run_cli(args).await?;
        duplicate.require_failure()?;
        let removed = case.run_cli(["plugin", "uninstall", "qa.localplugin"]).await?;
        removed.require_success()?;
        let after: Value = serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
        ensure!(after["plugins"]["list"].get("qa.localplugin").is_none(), "{after}");
        let empty = case.run_cli(["plugin", "list-installed"]).await?;
        empty.require_success()?;
        ensure!(empty.stdout.contains("no plugins installed"));
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn marketplace_cli_dependency_failure_rolls_back_and_cascade_removes_dependents() -> TestResult
{
    fixture()
        .run(|case| async move {
            let dependency = b"#!/bin/sh\nexit 10\n";
            let root_plugin = b"#!/bin/sh\nexit 11\n";
            let dependency_file = case
                .seed_file("marketplace/artifacts/dependency", dependency)
                .await?;
            let root_file = case
                .seed_file("marketplace/artifacts/root", root_plugin)
                .await?;
            let digest = |bytes: &[u8]| -> String {
                Sha256::digest(bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect()
            };
            let mut index = json!({
                "version": 1,
                "marketplace": {"name": "Dependency Registry", "description": "Offline graph"},
                "plugins": [
                    {"id":"qa.dependency", "versions": [{
                        "version":"0.1.0", "kind":"stdio", "platform":"any",
                        "url":format!("file://{}", dependency_file.display()),
                        "sha256": digest(dependency)
                    }]},
                    {"id":"qa.root", "versions": [{
                        "version":"0.2.0", "kind":"stdio", "platform":"any",
                        "url":format!("file://{}", root_file.display()),
                        "sha256": "0".repeat(64),
                        "dependencies": [{"plugin_id":"qa.dependency", "version_req":">=0.1.0"}]
                    }]}
                ]
            });
            let registry = case
                .seed_file(
                    "marketplace/dependency-catalog.json",
                    serde_json::to_vec(&index)?,
                )
                .await?;
            let args = vec![
                "plugin".into(),
                "install".into(),
                "qa.root".into(),
                "--registry".into(),
                path_arg(&registry),
                "--registry-id".into(),
                "local-graph".into(),
                "--refresh".into(),
            ];

            let before = tokio::fs::read(&case.paths().global_config).await?;
            let bad = case.run_cli(args.clone()).await?;
            bad.require_failure()?;
            ensure!(
                tokio::fs::read(&case.paths().global_config).await? == before,
                "failed dependency transaction left a partial Agena config"
            );
            let empty = case.run_cli(["plugin", "list-installed"]).await?;
            empty.require_success()?;
            ensure!(
                empty.stdout.contains("no plugins installed"),
                "rollback left partially installed dependencies: {empty:?}"
            );

            index["plugins"][1]["versions"][0]["sha256"] = json!(digest(root_plugin));
            case.seed_file(
                "marketplace/dependency-catalog.json",
                serde_json::to_vec(&index)?,
            )
            .await?;
            case.run_cli(args).await?.require_success()?;
            let listing = case.run_cli(["plugin", "list-installed"]).await?;
            listing.require_success()?;
            ensure!(
                listing.stdout.contains("qa.dependency v0.1.0"),
                "{listing:?}"
            );
            ensure!(listing.stdout.contains("qa.root v0.2.0"), "{listing:?}");

            let refused = case
                .run_cli(["plugin", "uninstall", "qa.dependency"])
                .await?;
            refused.require_failure()?;
            let still_installed = case.run_cli(["plugin", "list-installed"]).await?;
            still_installed.require_success()?;
            ensure!(
                still_installed.stdout.contains("qa.dependency")
                    && still_installed.stdout.contains("qa.root")
            );

            case.run_cli(["plugin", "uninstall", "qa.dependency", "--cascade"])
                .await?
                .require_success()?;
            let removed = case.run_cli(["plugin", "list-installed"]).await?;
            removed.require_success()?;
            ensure!(
                removed.stdout.contains("no plugins installed"),
                "{removed:?}"
            );
            let after: Value =
                serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
            ensure!(
                after["plugins"]["list"].get("qa.dependency").is_none()
                    && after["plugins"]["list"].get("qa.root").is_none(),
                "{after}"
            );
            Ok(())
        })
        .await
}
