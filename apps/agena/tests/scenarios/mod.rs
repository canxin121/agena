use std::time::Duration;

use agena_testkit::{ModelOptions, TestAgena, TestAgenaBuilder, UserSession};
use anyhow::Result;
use serde_json::json;

mod assistant;
mod assistant_ui;
mod bot;
mod bot_global;
mod bot_management;
mod bot_operator_ui;
mod bot_profiles;
mod bot_profiles_observation;
mod bot_profiles_ui;
mod bot_resilience;
mod channel_access;
mod clock;
mod control;
mod conversation;
mod coverage;
mod fault_lifecycle;
mod git;
mod integrations;
mod interaction;
mod lifecycle;
mod marketplace;
mod native;
mod plugins;
mod queue;
mod rewind;
mod settings;
mod terminal;
mod tools;
mod usage;
mod workbench_fs;

fn fixture() -> TestAgenaBuilder {
    TestAgena::builder(env!("CARGO_BIN_EXE_agena"))
        .startup_timeout(Duration::from_secs(60))
        .case_timeout(Duration::from_secs(60))
        .shutdown_timeout(Duration::from_secs(15))
        // Scripted foreground scenarios do not consume background model calls.
        // Dedicated memory lifecycle tests exercise automatic maintenance.
        .configure_global(|config| {
            config["plugins"]["list"]["agena.memory"] = json!({
                "package": {"kind": "static"},
                "settings": {"maintenance": {"enabled": false}}
            });
        })
        .model_options(ModelOptions {
            default_timeout: Duration::from_secs(20),
            ..Default::default()
        })
}

async fn workspace_permission(user: &UserSession, write: &str) -> Result<()> {
    user.client()
        .set_session_permission(
            user.id(),
            serde_json::from_value(json!({
                "path": {"workspace": {"read": "allow", "write": write}}
            }))?,
        )
        .await?;
    Ok(())
}

async fn path_permission(user: &UserSession, path: &std::path::Path, write: &str) -> Result<()> {
    // Fixture files live in temp storage. Product <tmp>/** allow rules would
    // otherwise outrank the broad workspace mode. An exact rule exercises the
    // requested policy through the real evaluator.
    user.client().set_session_permission(user.id(), serde_json::from_value(json!({
        "path": {"rules": {(path.to_string_lossy().as_ref()): {"read": "allow", "write": write}}}
    }))?).await?;
    Ok(())
}
