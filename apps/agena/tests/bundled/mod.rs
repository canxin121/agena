use std::{collections::BTreeSet, time::Duration};

use agena_testkit::{ModelOptions, TestAgena, TestAgenaBuilder, TestResult, UserSession};
use anyhow::ensure;
use serde_json::{Value, json};

mod assistant;
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
        // These scenarios script foreground model calls. Background extraction
        // has its own controlled-provider lifecycle tests.
        .configure_global(|config| {
            config["plugins"]["list"]["agena.memory"] = json!({
                "package":{"kind":"static"},
                "settings":{"maintenance":{"enabled":false}}
            });
        })
        .model_options(ModelOptions {
            default_timeout: Duration::from_secs(30),
            ..Default::default()
        })
}

fn native_fixture_address() -> anyhow::Result<std::net::IpAddr> {
    // Selecting a UDP route does not send a packet. Fixtures only bind and
    // connect to the resulting local interface, never to this documentation IP.
    let route = std::net::UdpSocket::bind((std::net::Ipv4Addr::UNSPECIFIED, 0))?;
    route.connect((std::net::Ipv4Addr::new(192, 0, 2, 1), 9))?;
    let address = route.local_addr()?.ip();
    ensure!(
        !address.is_loopback() && !address.is_unspecified(),
        "native network fixtures require a local non-loopback interface"
    );
    Ok(address)
}

fn fixture_without_memory() -> TestAgenaBuilder {
    fixture().configure_global(|config| {
        config["plugins"]["list"]["agena.memory"]["enabled"] = json!(false);
    })
}

fn native_fixture_network() -> anyhow::Result<Value> {
    let network = if cfg!(target_os = "macos") {
        json!({"default":"allow"})
    } else {
        let address = native_fixture_address()?;
        json!({
            "default":"deny",
            "allow":[{"to":address.to_string()}]
        })
    };
    Ok(network)
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
                    "workspace": "readwrite",
                    "external": "readwrite"
                },
                "network": native_fixture_network()?,
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
        assistant::CASES,
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
