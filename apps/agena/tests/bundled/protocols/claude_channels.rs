use agena_testkit::{Reply, RequestMatch, TestCase, TestResult};
use anyhow::{Context, ensure};
use serde_json::{Value, json};
use std::time::Duration;

const PLUGIN: &str = "claude.fixture-channel";
const CHANNEL: &str = "plugin:claude.fixture-channel:events";

async fn install_channel(case: TestCase) -> TestResult {
    let package = case.paths().root.join("claude-channel");
    tokio::fs::create_dir_all(package.join(".claude-plugin")).await?;
    tokio::fs::write(
        package.join(".claude-plugin/plugin.json"),
        serde_json::to_vec(&json!({
            "name":"fixture-channel",
            "channels": [{"server":"events","displayName":"Fixture events","userConfig":{
                "room":{"type":"string","title":"Room","description":"Fixture sender","default":"fixture-room"},
                "credential":{"type":"string","title":"Token","description":"Optional credential","sensitive":true,
                    "default":"do-not-render-fixture-token"},
            }}]
        }))?,
    )
    .await?;
    tokio::fs::write(
        package.join(".mcp.json"),
        serde_json::to_vec(&json!({
            "mcpServers": {
                "events": {
                    "command":"python3",
                    "args":["${CLAUDE_PLUGIN_ROOT}/peer.py", "${CLAUDE_PROJECT_DIR}/channel-trigger"],
                    "env":{"FIXTURE_ROOM":"${user_config.room}"},
                },
                "implicit": {
                    "command":"python3",
                    "args":["${CLAUDE_PLUGIN_ROOT}/peer.py", "${CLAUDE_PROJECT_DIR}/implicit-trigger"],
                    "env":{"FIXTURE_ROOM":"implicit-room"},
                }
            }
        }))?,
    )
    .await?;
    tokio::fs::write(
        package.join("peer.py"),
        include_str!("claude_channel_peer.py"),
    )
    .await?;
    let skill = package.join("skills/configure");
    tokio::fs::create_dir_all(&skill).await?;
    let (shell, command) = if cfg!(windows) {
        ("powershell", "Write-Output $env:CLAUDE_CONFIG_DIR")
    } else {
        ("bash", "printf '%s' \"$CLAUDE_CONFIG_DIR\"")
    };
    tokio::fs::write(
        skill.join("SKILL.md"),
        format!("---\nname: configure\ndescription: Fixture channel setup\nshell: {shell}\n---\nChannel configuration directory is !`{command}`\n"),
    )
    .await?;
    case.run_cli([
        "plugin",
        "install",
        package.to_str().context("fixture path")?,
    ])
    .await?
    .require_success()?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_channel_resolves_plugin_selector_and_requires_process_sandbox()
-> TestResult {
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin(PLUGIN)
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["path"]["external"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.shell.exec"] = json!("allow");
        })
        .before_start(install_channel)
        .run(|case| async move {
            let owner = case.user().new_session().await?;
            let route = format!("/api/v1/sessions/{}/claude-channels", owner.id());
            let http = case.http().await;
            let catalog = case
                .run_cli(["sessions", "channels", &owner.id().to_string(), "--catalog"])
                .await?;
            catalog.require_success()?;
            ensure!(!catalog.stdout.contains("do-not-render-fixture-token"));
            let catalog: Vec<Value> = serde_json::from_str(&catalog.stdout)?;
            ensure!(catalog.len() == 2, "{catalog:?}");
            ensure!(
                catalog[0]["server"] == CHANNEL && catalog[0]["display_name"] == "Fixture events"
            );
            ensure!(catalog[0]["registered"] == true && catalog[0]["declared"] == true);
            ensure!(catalog[1]["server"] == "plugin:claude.fixture-channel:implicit");
            ensure!(catalog[1]["registered"] == true && catalog[1]["declared"] == false);
            let options = catalog[0]["user_options"]
                .as_array()
                .context("channel option catalog")?;
            ensure!(
                options
                    .iter()
                    .any(|option| option["key"] == "room" && option["configured"] == true)
            );
            ensure!(options.iter().any(|option| option["key"] == "credential"
                && option["sensitive"] == true
                && option["configured"] == false));
            let configured = owner
                .client()
                .invoke_plugin_command(PLUGIN, "configure", json!({}), Some(owner.id()), None, "")
                .await?
                .result;
            ensure!(
                configured.status == agena_plugin_sdk::CommandStatus::Succeeded,
                "{configured:?}"
            );
            let [agena_plugin_sdk::CommandHostEffect::InsertPrompt { prompt }] =
                configured.effects.as_slice()
            else {
                anyhow::bail!("expected one shell-expanded configure effect: {configured:?}");
            };
            let directory = prompt
                .trim()
                .strip_prefix("Channel configuration directory is ")
                .context("channel config directory in skill output")?;
            ensure!(std::path::Path::new(directory).is_absolute(), "{directory}");
            ensure!(
                std::path::Path::new(directory).starts_with(&case.paths().data_dir),
                "{directory}"
            );
            ensure!(std::path::Path::new(directory).is_dir(), "{directory}");
            let failed = http
                .send_json(
                    reqwest::Method::PUT,
                    &route,
                    &json!({"action":"enable","server":CHANNEL}),
                )
                .await?;
            ensure!(
                !failed.status.is_success() && failed.text()?.contains("process sandboxing"),
                "expected selector resolution and sandbox denial, got {failed:?}"
            );
            let channels: Vec<Value> = http.get(&route).await?.require_success()?.json()?;
            ensure!(channels.is_empty(), "{channels:?}");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires native process sandboxing for imported stdio servers"]
async fn imported_claude_channel_wakes_only_the_selected_session_and_relays_permission()
-> TestResult {
    use agena_testkit::{EditOptions, SettingsLayer};

    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "enabled")
        .expect_plugin(PLUGIN)
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime":{"reconnect":{"enabled":false}}}),
        )
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["path"]["external"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.fs.write"] = json!("ask");
            config["permission"]["tools"]["names"]["agena.mcp.tools.search"] = json!("allow");
            config["permission"]["tools"]["names"]["agena.mcp.tools.call"] = json!("allow");
        })
        .before_start(install_channel)
        .run(|case| async move {
            let owner = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            let owner_id = owner.id().to_string();
            let other_route = format!("/api/v1/sessions/{}/claude-channels", other.id());
            let http = case.http().await;
            let other_channels: Vec<Value> =
                http.get(&other_route).await?.require_success()?.json()?;
            ensure!(other_channels.is_empty(), "{other_channels:?}");

            case.model()
                .expect(RequestMatch::user_text("fixture channel alert"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(body.contains("channel") && body.contains("fixture-room"));
                    Ok(())
                })
                .reply(Reply::text("The channel notice was received."));
            case.run_cli([
                "sessions",
                "channels",
                &owner_id,
                "--enable",
                CHANNEL,
                "--permission-relay",
            ])
            .await?
            .require_success()?;
            let trigger = case.workspace().join("channel-trigger");
            tokio::fs::write(&trigger, "ready").await?;
            tokio::time::timeout(Duration::from_secs(15), async {
                while case.model().requests().is_empty() {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await?;
            // A new MCP connection cannot publish an event until the trigger
            // returns, so only the bound channel peer can accept the reply.
            tokio::fs::remove_file(&trigger).await?;
            let found = owner
                .call_tool("mcp.tools.search", json!({"query": "fixture channel"}))
                .await?;
            found.assert_succeeded()?;
            let server = found.data()?["results"]
                .as_array()
                .context("fixture channel search results")?
                .iter()
                .find_map(|tool| {
                    tool["server"]
                        .as_str()
                        .filter(|server| tool["name"] == "reply" && server.ends_with("-events"))
                })
                .context("fixture channel reply tool")?;
            let reply = owner
                .call_tool(
                    "mcp.tools.call",
                    json!({"server":server,"name":"reply", "arguments":{
                        "chat_id":"fixture-room","text":"received"
                    }}),
                )
                .await?;
            reply.assert_succeeded()?;
            reply.assert_text_contains("fixture reply: received")?;
            tokio::fs::write(&trigger, "ready").await?;

            let output = owner
                .call_tool(
                    "fs.write",
                    json!({"path":"channel-approved.txt","content":"approved by fixture"}),
                )
                .await?;
            output.assert_succeeded()?;
            ensure!(
                tokio::fs::read_to_string(case.workspace().join("channel-approved.txt")).await?
                    == "approved by fixture"
            );
            case.model()
                .expect(RequestMatch::user_text("fixture channel after reconnect"))
                .reply(Reply::text("The reconnected channel notice was received."));
            tokio::fs::write(
                case.workspace().join("channel-trigger.restart"),
                "disconnect",
            )
            .await?;
            tokio::time::timeout(Duration::from_secs(15), async {
                while !case.model().requests().iter().any(|request| {
                    request
                        .body
                        .to_string()
                        .contains("fixture channel after reconnect")
                }) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await?;
            const RELOADED: &str = "fixture channel after runtime reload";
            case.model()
                .expect(RequestMatch::user_text(RELOADED))
                .reply(Reply::text("The channel is still active in this session."));
            tokio::fs::write(trigger.with_extension("message"), RELOADED).await?;
            case.settings().reload_and_wait().await?;
            tokio::time::timeout(Duration::from_secs(20), async {
                while !case
                    .model()
                    .requests()
                    .iter()
                    .any(|request| request.body.to_string().contains(RELOADED))
                {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await?;
            let owner_route = format!("/api/v1/sessions/{owner_id}/claude-channels");
            let channels: Vec<Value> = http.get(&owner_route).await?.require_success()?.json()?;
            ensure!(
                channels == [json!({"server":CHANNEL,"permission_relay":true,"connected":true})]
            );
            let other_channels: Vec<Value> =
                http.get(&other_route).await?.require_success()?.json()?;
            ensure!(other_channels.is_empty(), "{other_channels:?}");
            case.settings()
                .set(
                    SettingsLayer::Global,
                    "plugins.list.\"claude.fixture-channel\".enabled",
                    json!(false),
                    EditOptions {
                        reload: false,
                        ..Default::default()
                    },
                )
                .await?
                .require_success()?;
            case.settings().reload_and_wait().await?;
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let channels: Vec<Value> =
                        http.get(&owner_route).await?.require_success()?.json()?;
                    if channels.is_empty() {
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await??;
            case.run_cli(["sessions", "channels", &owner_id, "--disable", CHANNEL])
                .await?
                .require_success()?;
            let owner_channels: Vec<Value> =
                http.get(&owner_route).await?.require_success()?.json()?;
            ensure!(owner_channels.is_empty(), "{owner_channels:?}");
            Ok(())
        })
        .await
}
