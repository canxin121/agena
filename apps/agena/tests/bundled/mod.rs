use std::{collections::BTreeSet, time::Duration};

use agena_testkit::{ModelOptions, TestAgena, TestAgenaBuilder, TestResult, UserSession};
use anyhow::ensure;
use serde_json::{Value, json};

mod core;
mod notifications;
mod protocols;
mod runtime;
mod settings_provider;

fn fixture() -> TestAgenaBuilder {
    TestAgena::builder(env!("CARGO_BIN_EXE_agena"))
        .startup_timeout(Duration::from_secs(60))
        .case_timeout(Duration::from_secs(240))
        .shutdown_timeout(Duration::from_secs(20))
        .model_options(ModelOptions {
            default_timeout: Duration::from_secs(30),
            ..Default::default()
        })
}

/// Explicit policy for these local, case-owned behavioral fixtures. It is not
/// hidden inside call_tool: permission-specific tests select their own policy.
async fn allow_permissions(user: &UserSession) -> TestResult {
    let mut names = serde_json::Map::new();
    for plugin in agena_bundled_plugins::bundled_capability_manifest().plugins {
        for tool in plugin.tools {
            names.insert(tool.canonical_name.clone(), json!("allow"));
            if let Some(short) = tool.canonical_name.strip_prefix("agena.") {
                names.insert(short.to_owned(), json!("allow"));
            }
        }
    }
    user.client()
        .set_session_permission(
            user.id(),
            serde_json::from_value(json!({
                "path": {
                    "workspace": {"read": "allow", "write": "allow"},
                    "external": {"read": "allow", "write": "allow"}
                },
                "network": {"loopback": "allow", "private": "deny", "internet": "deny"},
                "tools": {"default": "allow", "names": Value::Object(names)}
            }))?,
        )
        .await?;
    Ok(())
}

/// Function pointers tie each inventory entry to a compiled, executable Rust
/// behavior test. Compare with registration entries, including tool-less and
/// conditional plugins, rather than only the tool documentation catalog.
type BundledBehaviorCase = (&'static str, fn() -> TestResult);

#[test]
fn every_registered_builtin_has_a_behavior_test() -> TestResult {
    let groups: &[&[BundledBehaviorCase]] = &[
        core::CASES,
        runtime::CASES,
        protocols::CASES,
        settings_provider::CASES,
        notifications::CASES,
    ];
    let mut covered = BTreeSet::new();
    for (id, test) in groups.iter().flat_map(|group| group.iter()) {
        ensure!(
            covered.insert((*id).to_owned()),
            "duplicate plugin coverage entry: {id}"
        );
        // Retain a typed reference without running the test twice; libtest owns
        // discovery, execution and per-test reporting.
        let _test: fn() -> TestResult = *test;
    }
    let registered: BTreeSet<_> = agena_bundled_plugins::plugins::sources::bundled_plugin_entries()
        .into_keys()
        .collect();
    ensure!(
        registered == covered,
        "bundled plugin test coverage drift: missing {:?}, obsolete {:?}",
        registered.difference(&covered).collect::<Vec<_>>(),
        covered.difference(&registered).collect::<Vec<_>>()
    );
    Ok(())
}
