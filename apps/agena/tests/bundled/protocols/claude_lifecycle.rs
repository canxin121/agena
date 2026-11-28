use std::time::Duration;

use agena_api::resource::{RuntimeBackgroundTaskKind, RuntimeBackgroundTaskStatus};
use agena_testkit::{Reply, RequestMatch, SettingsLayer, TestCase, TestResult};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

async fn install(case: TestCase, module: &str) -> TestResult {
    let package = case.paths().root.join("lifecycle-plugin");
    tokio::fs::create_dir_all(package.join(".claude-plugin")).await?;
    tokio::fs::create_dir_all(package.join("hooks")).await?;
    tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
    tokio::fs::write(
        package.join(".claude-plugin/plugin.json"),
        br#"{"name":"fixture-lifecycle"}"#,
    )
    .await?;
    tokio::fs::write(
        package.join("hooks/hooks.json"),
        br#"{"modules":["lifecycle.mjs"]}"#,
    )
    .await?;
    tokio::fs::write(package.join("hooks/lifecycle.mjs"), module).await?;
    case.run_cli([
        "plugin",
        "install",
        package.to_str().context("plugin package path")?,
        "--enable-hooks",
    ])
    .await?
    .require_success()?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_message_display_keeps_the_original_assistant_source() -> TestResult {
    const RAW: &str = "original first line\noriginal final fragment";
    const DISPLAY: &str = "formatted first line\nformatted final fragment";
    const HOOK: &str = r#"
export function register(on) {
    on('classic.MessageDisplay', async ($, event) => {
        await $.tool.call({tool:'agena.fs.write', path:'display-event-' + event.index + '.json',
            content:JSON.stringify(event)});
        return {
            displayContent: event.final ? 'formatted final fragment' : 'formatted first line\n',
            preventContinuation: true,
            systemMessage: 'fixture ignored message'
        };
    });
}

"#;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin("claude.fixture-lifecycle")
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.fs.write"] = json!("allow");
        })
        .before_start(|case| async move { install(case, HOOK).await })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            case.model()
                .expect(RequestMatch::user_text("show a Claude display projection"))
                .reply(Reply::text(RAW));
            let settled = user
                .send("show a Claude display projection")
                .await?
                .wait_settled()
                .await?;
            settled.assert_succeeded()?;
            settled.assert_assistant_text(DISPLAY)?;
            let text = settled
                .parts
                .iter()
                .find(|part| part.role == "assistant" && part.kind == "text")
                .context("assistant text part")?;
            let resource = serde_json::from_value(text.content["original_text_resource"].clone())?;
            let mut raw = Vec::new();
            user.client()
                .write_content_text(user.id(), &resource, &mut raw)
                .await?;
            ensure!(raw == RAW.as_bytes(), "stored source differs");

            let mut events = Vec::new();
            for index in [0, 1] {
                let path = case.workspace().join(format!("display-event-{index}.json"));
                events.push(serde_json::from_slice::<Value>(
                    &tokio::fs::read(path).await?,
                )?);
            }
            ensure!(!case.workspace().join("display-event-2.json").exists());
            ensure!(
                events[0]["index"] == 0 && events[0]["final"] == false,
                "{events:?}"
            );
            ensure!(
                events[1]["index"] == 1 && events[1]["final"] == true,
                "{events:?}"
            );
            ensure!(
                events[0]["message_id"] == events[1]["message_id"]
                    && events[0]["turn_id"] == events[1]["turn_id"],
                "{events:?}"
            );
            ensure!(
                events[0]["delta"] == "original first line\n"
                    && events[1]["delta"] == "original final fragment",
                "{events:?}"
            );
            ensure!(
                events[0]["session_id"] == user.id().to_string(),
                "{events:?}"
            );

            let id = user.id();
            case.restart().await?;
            let user = case.user().attach_session(id).await?;
            case.model()
                .expect(RequestMatch::user_text("verify original model context"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        body.contains("original first line")
                            && body.contains("original final fragment")
                    );
                    ensure!(
                        !body.contains("formatted first line")
                            && !body.contains("fixture ignored message")
                    );
                    Ok(())
                })
                .reply(Reply::text("second round worked"));
            user.send("verify original model context")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_stop_observes_only_current_pending_work() -> TestResult {
    const EXPRESSION: &str = "0 0 0 1 1 *";
    const HOOK: &str = r#"
export function register(on) {
    on('classic.Stop', (_$, event) => {
        if (event.stop_hook_active || event.last_assistant_message !== 'measure Claude Stop work') return {};
        return {additionalContext: ['Claude Stop work: ' + JSON.stringify({
            background_tasks:event.background_tasks, session_crons:event.session_crons,
        })]};
    });
}
"#;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("claude.fixture-lifecycle")
        .expect_plugin("agena.cron")
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            for tool in [
                "shell.spawn",
                "shell.stop",
                "cron.create",
                "cron.pause",
            ] {
                config["permission"]["tools"]["names"][format!("agena.{tool}")] = json!("allow");
            }
        })
        .before_start(|case| async move { install(case, HOOK).await })
        .run(|case| async move {
            let client = case.client().await;
            tokio::time::timeout(Duration::from_secs(30), async {
                while client
                    .runtime_status()
                    .await?
                    .model_catalog
                    .is_some_and(|catalog| catalog.refreshing)
                {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("startup model catalog refresh did not settle")??;
            let owner = case.user().new_session().await?;
            let other = case.user().new_session().await?;
            let (shell, command) = if cfg!(windows) {
                ("powershell", "while ($true) { Start-Sleep -Seconds 3600 }")
            } else {
                ("bash", "while true; do sleep 3600; done")
            };
            let spawned = owner
                .call_tool(
                    "shell.spawn",
                    json!({
                        "shell":shell,
                        "command":command,
                        "description":"fixture pending process"
                    }),
                )
                .await?;
            spawned.assert_succeeded()?;
            let process = spawned.data()?["process_id"]
                .as_str()
                .context("process id")?;

            let observation: TestResult = async {
            let create = |prompt: &str| {
                json!({"expression":EXPRESSION,"timezone":"UTC","prompt":prompt,"max_age_days":7})
            };
            let scheduled = owner
                .call_tool("cron.create", create("fixture waiting job"))
                .await?;
            scheduled.assert_succeeded()?;
            let scheduled_id = scheduled.data()?["id"].as_str().context("scheduled id")?;
            let paused = owner
                .call_tool("cron.create", create("fixture paused job"))
                .await?;
            paused.assert_succeeded()?;
            let paused_id = paused.data()?["id"].as_str().context("paused id")?;
            owner
                .call_tool("cron.pause", json!({"id":paused_id}))
                .await?
                .assert_succeeded()?;

            for (session, message) in [(&owner, "check owner work"), (&other, "check other work")] {
                let expected = if session.id() == owner.id() {
                    json!({"background_tasks":[{
                        "id":process,"type":"shell","status":"running",
                        "description":"fixture pending process","command":command,
                    }],"session_crons":[{
                        "id":scheduled_id,"schedule":EXPRESSION,
                        "prompt":"fixture waiting job","recurring":true,
                    }]})
                } else {
                    json!({"background_tasks":[],"session_crons":[]})
                };
                let initial = case.model().expect(RequestMatch::user_text(message))
                    .reply(Reply::text("measure Claude Stop work"));
                case.model()
                    .expect(RequestMatch::new("Claude Stop work", |request| stop_work_text(&request.body).is_some()))
                    .after(&initial)
                    .check(move |request| {
                        let snapshot = stop_work_text(&request.body).context("Claude Stop work in continuation")?;
                        let value = <Value as serde::Deserialize>::deserialize(
                            &mut serde_json::Deserializer::from_str(snapshot),
                        )?;
                        ensure!(value == expected, "expected {expected}, got {value}");
                        Ok(())
                    })
                    .reply(Reply::text("finished after observing pending work"));
                let settled = session.send(message).await?.wait_settled().await?;
                settled.assert_succeeded()?;
                settled.assert_text_contains("finished after observing pending work")?;
            }
            Ok(())
            }.await;
            let stopped = owner
                .call_tool("shell.stop", json!({"process_id":process}))
                .await;
            observation?;
            stopped?.assert_succeeded()?;
            Ok(())
        })
        .await
}

fn stop_work_text(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => text.split_once("Claude Stop work: ").map(|(_, work)| work),
        Value::Array(items) => items.iter().find_map(stop_work_text),
        Value::Object(items) => items.values().find_map(stop_work_text),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_directory_added_uses_committed_permissions_and_watches() -> TestResult {
    const PLUGIN: &str = "claude.fixture-lifecycle";
    const DIAGNOSTIC: &str = "directory-added debug-only marker";
    const HOOK: &str = r#"
export function register(on) {
    const watchPath = __WATCH_PATH__;
    let count = 0;
    let recordedWatch = false;
    on('classic.SessionStart', () => ({watchPaths:[watchPath]}));
    on('classic.DirectoryAdded', async ($, event) => {
        const before = await $.tool.call({tool:'agena.fs.read', file_path:watchPath, mode:'text'});
        const stat = await $.tool.call({tool:'agena.fs.stat', path:watchPath, hash:true});
        await $.tool.call({tool:'agena.fs.write', path:watchPath, content:'changed by DirectoryAdded',
            expected_sha256:stat.result.sha256});
        await $.tool.call({tool:'agena.fs.write', path:'directory-added-' + (++count) + '.json',
            content:JSON.stringify({event, before:before.text})});
        return {
            systemMessage:'directory-added debug-only marker',
            additionalContext:['directory-added unexpected model context'],
            block:'directory-added unexpected veto',
            preventContinuation:true,
        };
    });
    on('classic.FileChanged', async ($, event) => {
        if (!recordedWatch) {
            recordedWatch = true;
            await $.tool.call({tool:'agena.fs.write', path:'registered-watch.json', content:JSON.stringify(event)});
        }
        return {};
    });
}
"#;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["path"]["external"] = json!("deny");
            for tool in ["read", "stat", "write"] {
                config["permission"]["tools"]["names"][format!("agena.fs.{tool}")] = json!("allow");
            }
        })
        .before_start(|case| async move {
            let repository = case.paths().root.join("additional repository");
            tokio::fs::create_dir_all(repository.join("nested")).await?;
            let watched = tokio::fs::canonicalize(repository)
                .await?
                .join("settings.txt");
            tokio::fs::write(&watched, "settings before registration").await?;
            let module = HOOK.replace("__WATCH_PATH__", &serde_json::to_string(&watched)?);
            install(case, &module).await
        })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let repository =
                tokio::fs::canonicalize(case.paths().root.join("additional repository")).await?;
            let watched = repository.join("settings.txt");
            let canonical_workspace = tokio::fs::canonicalize(case.workspace()).await?;
            user.call_tool(
                "agena.fs.read",
                json!({"file_path": watched, "mode":"text"}),
            )
            .await?
            .assert_failure_kind(agena_failure::ModelFeedbackKind::PermissionDenied)?;
            let route = format!("/api/v1/sessions/{}/working-directories", user.id());
            let http = case.http().await;
            let register = |value: Value| {
                let http = &http;
                let route = &route;
                async move {
                    let response: Value = http
                        .send_json(reqwest::Method::POST, route, &value)
                        .await?
                        .require_success()?
                        .json()?;
                    Ok::<_, anyhow::Error>(response)
                }
            };
            let owner_id = user.id().to_string();
            let cli = case
                .run_cli([
                    "sessions",
                    "working-directories",
                    &owner_id,
                    "--add",
                    "../additional repository",
                ])
                .await?;
            let added: Value = serde_json::from_str(&cli.require_success()?.stdout)?;
            ensure!(
                added["added"] == true && added["directory"] == json!(repository),
                "{added}"
            );
            let duplicate = register(json!({"path":repository})).await?;
            ensure!(duplicate["added"] == false, "{duplicate}");
            for path in [canonical_workspace.clone(), repository.join("nested")] {
                let covered = register(json!({"path": path})).await?;
                ensure!(covered["added"] == false && covered["directories"] == json!([repository]));
            }
            let missing = http
                .send_json(
                    reqwest::Method::POST,
                    &route,
                    &json!({"path":repository.join("missing")}),
                )
                .await?;
            ensure!(missing.status == reqwest::StatusCode::BAD_REQUEST);
            let cli = case
                .run_cli(["sessions", "working-directories", &owner_id])
                .await?;
            let listed: Value = serde_json::from_str(&cli.require_success()?.stdout)?;
            ensure!(listed == json!([repository]), "{listed}");

            let receipt = case.workspace().join("directory-added-1.json");
            let watch_event = case.workspace().join("registered-watch.json");
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let logs = user.client().plugin_logs(PLUGIN, None, 50).await?;
                    if receipt.is_file()
                        && watch_event.is_file()
                        && logs
                            .logs
                            .iter()
                            .any(|entry| entry.message == DIAGNOSTIC && entry.level == "debug")
                    {
                        return Ok::<_, anyhow::Error>(());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .context("DirectoryAdded and FileChanged observers did not finish")??;
            let receipt: Value = serde_json::from_slice(&tokio::fs::read(&receipt).await?)?;
            let event = &receipt["event"];
            ensure!(
                event["hook_event_name"] == "DirectoryAdded"
                    && event["source"] == "register_repo_root"
                    && event["session_id"] == user.id().to_string()
                    && event["directory"] == json!(repository)
                    && event["cwd"] == json!(canonical_workspace),
                "{event}"
            );
            ensure!(
                receipt["before"]
                    .as_str()
                    .is_some_and(|text| text.contains("settings before registration")),
                "{receipt}"
            );
            ensure!(tokio::fs::read_to_string(&watched).await? == "changed by DirectoryAdded");
            let changed: Value = serde_json::from_slice(&tokio::fs::read(&watch_event).await?)?;
            ensure!(
                changed["event"] == "change" && changed["file_path"] == json!(watched),
                "{changed}"
            );
            let other = case.user().new_session().await?;
            other
                .call_tool("agena.fs.read", json!({"file_path":watched, "mode":"text"}))
                .await?
                .assert_failure_kind(agena_failure::ModelFeedbackKind::PermissionDenied)?;
            ensure!(
                other
                    .client()
                    .list_working_directories(other.id())
                    .await?
                    .is_empty()
            );

            case.model()
                .expect(RequestMatch::user_text("inspect the registration result"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        !body.contains(DIAGNOSTIC) && !body.contains("directory-added unexpected")
                    );
                    Ok(())
                })
                .reply(Reply::text("registration remained in effect"));
            user.send("inspect the registration result")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            ensure!(!case.workspace().join("directory-added-2.json").exists());
            let session_id = user.id();
            case.restart().await?;
            let restored = case.user().attach_session(session_id).await?;
            ensure!(
                restored
                    .client()
                    .list_working_directories(session_id)
                    .await?
                    == [repository]
            );
            restored
                .call_tool("agena.fs.read", json!({"file_path":watched, "mode":"text"}))
                .await?
                .assert_text_contains("changed by DirectoryAdded")?;
            Ok(())
        })
        .await
}

async fn wait_for_failed_reload(case: &TestCase, expected: usize) -> TestResult {
    let client = case.client().await;
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let status = client.runtime_status().await?;
            let failed = status
                .background_tasks
                .iter()
                .filter(|task| {
                    task.kind == RuntimeBackgroundTaskKind::RuntimeReload
                        && task.status == RuntimeBackgroundTaskStatus::Failed
                })
                .count();
            if failed >= expected {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(75)).await;
        }
    })
    .await
    .context("watched reload did not settle")?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_config_change_activates_for_an_already_open_session() -> TestResult {
    const PLUGIN: &str = "claude.fixture-lifecycle";
    const HOOK: &str = r#"
export function register(on) {
    let recorded = false;
    on('classic.ConfigChange', async ($, event) => {
        if (event.source !== 'local_settings') return {};
        if (!recorded) {
            recorded = true;
            await $.tool.call({tool:'agena.fs.write', path:'late-config-hook.json', content:JSON.stringify(event)});
        }
        return {block:'fixture rejected the local change'};
    });
}
"#;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.fs.write"] = json!("allow");
        })
        .before_start(|case| async move {
            install(case.clone(), HOOK).await?;
            let config_path = &case.paths().global_config;
            let mut config: Value = serde_json::from_slice(&tokio::fs::read(config_path).await?)?;
            let plugin = &mut config["plugins"]["list"][PLUGIN];
            ensure!(plugin["enabled"] == true, "{plugin}");
            plugin["enabled"] = json!(false);
            tokio::fs::write(config_path, serde_json::to_vec(&config)?).await?;
            tokio::fs::create_dir_all(case.workspace().join(".claude")).await?;
            Ok(())
        })
        .run(|case| async move {
            let owner = case.user().new_session().await?;
            let settings = case.settings();
            settings
                .set(
                    SettingsLayer::Global,
                    &format!("plugins.list.\"{PLUGIN}\".enabled"),
                    json!(true),
                    agena_testkit::EditOptions {
                        reload: false,
                        ..Default::default()
                    },
                )
                .await?
                .require_success()?;
            settings.reload_and_wait().await?;
            let workspace = tokio::fs::canonicalize(case.workspace()).await?;
            let local = workspace.join(".claude/settings.local.json");
            tokio::fs::write(&local, br#"{"claudeMdExcludes":["**/CLAUDE.md"]}"#).await?;
            wait_for_failed_reload(&case, 1).await?;
            let receipt = tokio::fs::read(workspace.join("late-config-hook.json")).await?;
            let event: Value = serde_json::from_slice(&receipt)?;
            ensure!(
                event["session_id"] == owner.id().to_string()
                    && event["source"] == "local_settings"
                    && event["file_path"] == json!(local),
                "{event}"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_config_change_can_veto_watched_project_reload() -> TestResult {
    const HOOK: &str = r#"
export function register(on) {
    let changes = 0;
    on('classic.ConfigChange', async ($, event) => {
        if (event.source !== 'project_settings') return {};
        changes++;
        await $.tool.call({tool:'agena.fs.write', path:'config-event-' + changes + '.json',
            content:JSON.stringify(event)});
        return changes <= 2 ? {block:'fixture declined the project change'} : {};
    });
}
"#;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin("claude.fixture-lifecycle")
        .workspace_config(json!({"session": {"max_turns": 11}}))
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.fs.write"] = json!("allow");
        })
        .before_start(|case| async move { install(case, HOOK).await })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let settings = case.settings();
            let before = settings
                .wait_loaded("/config/session/max_turns", json!(11))
                .await?;
            case.write_config(
                SettingsLayer::Workspace,
                json!({"session": {"max_turns": 12}}),
            )
            .await?;
            wait_for_failed_reload(&case, 1).await?;
            let event: Value = serde_json::from_slice(
                &tokio::fs::read(case.workspace().join("config-event-1.json")).await?,
            )?;
            ensure!(
                event["source"] == "project_settings" && event["hook_event_name"] == "ConfigChange",
                "{event}"
            );
            ensure!(
                event["file_path"] == json!(case.paths().workspace_config),
                "{event}"
            );
            ensure!(event["session_id"] == user.id().to_string(), "{event}");
            settings
                .wait_loaded("/config/session/max_turns", json!(11))
                .await?;

            // Another watched path must not publish the previously vetoed
            // project file as part of a later runtime reload.
            let mut global = tokio::fs::read(&case.paths().global_config).await?;
            global.push(b'\n');
            tokio::fs::write(&case.paths().global_config, global).await?;
            wait_for_failed_reload(&case, 2).await?;
            ensure!(case.workspace().join("config-event-2.json").is_file());
            settings
                .wait_loaded("/config/session/max_turns", json!(11))
                .await?;

            case.write_config(
                SettingsLayer::Workspace,
                json!({"session": {"max_turns": 13}}),
            )
            .await?;
            let after = settings
                .wait_loaded("/config/session/max_turns", json!(13))
                .await?;
            ensure!(after.generation > before.generation);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_project_settings_filter_instructions_and_honor_vetoes() -> TestResult {
    const HOOK: &str = r#"
export function register(on) {
    const counts = new Map();
    on('classic.ConfigChange', async ($, event) => {
        if (event.source !== 'project_settings' && event.source !== 'local_settings') return {};
        const count = (counts.get(event.source) || 0) + 1;
        counts.set(event.source, count);
        await $.tool.call({tool:'agena.fs.write', path:'native-config-' + event.source + '-' + count + '.json',
            content:JSON.stringify(event)});
        return event.source === 'local_settings' && count === 1 ? {block:'fixture veto'} : {};
    });
}
"#;
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin("claude.fixture-lifecycle")
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["path"]["external"] = json!("deny");
            config["permission"]["tools"]["names"]["agena.fs.write"] = json!("allow");
        })
        .before_start(|case| async move {
            let workspace = case.workspace();
            let shared = json!({"claudeMdExcludes":[
                "**/.claude/rules/shared.md", "**/.claude/content/unloaded.md", "**/.claude/rules/alias.md"
            ]});
            let local = json!({"claudeMdExcludes":[
                "**/.claude/rules/local.md", "**/.claude/content/target.md"
            ]});
            for (name, content) in [
                ("CLAUDE.md", "Native primary instructions\n\n@.claude/content/unloaded.md".into()),
                (".claude/CLAUDE.md", "Native alternate instructions".into()),
                (".claude/content/unloaded.md", "Native excluded import".into()),
                (".claude/content/target.md", "Native resolved-target instructions".into()),
                (".claude/content/alias-target.md", "Native alias-excluded instructions".into()),
                (".claude/rules/shared.md", "Native shared rule".into()),
                (".claude/rules/local.md", "Native local rule".into()),
                (".claude/rules/kept.md", "Native retained rule".into()),
                (".claude/settings.json", shared.to_string()),
                (".claude/settings.local.json", local.to_string()),
            ] {
                let path = workspace.join(name);
                tokio::fs::create_dir_all(path.parent().context("native fixture parent")?).await?;
                tokio::fs::write(path, content).await?;
            }
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink("../content/target.md", workspace.join(".claude/rules/link.md"))?;
                std::os::unix::fs::symlink("../content/alias-target.md", workspace.join(".claude/rules/alias.md"))?;
            }
            install(case, HOOK).await
        })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let workspace = tokio::fs::canonicalize(case.workspace()).await?;
            let shared = workspace.join(".claude/settings.json");
            let local = workspace.join(".claude/settings.local.json");
            let prompt = |text: &'static str, local_included: bool, alternate: bool| {
                let case = &case;
                let user = &user;
                async move {
                    case.model().expect(RequestMatch::user_text(text)).check(move |request| {
                        let body = request.body.to_string();
                        ensure!(body.contains("Native retained rule"));
                        ensure!(body.contains("Native primary instructions") != alternate);
                        ensure!(body.contains("Native alternate instructions") == alternate);
                        ensure!(body.contains("Native local rule") == local_included);
                        if cfg!(unix) {
                            ensure!(body.contains("Native resolved-target instructions") == local_included);
                        }
                        for hidden in ["Native shared rule", "Native excluded import", "Native alias-excluded instructions"] {
                            ensure!(!body.contains(hidden), "loaded excluded guidance {hidden}");
                        }
                        Ok(())
                    }).reply(Reply::text("guidance checked"));
                    user.send(text).await?.wait_settled().await?.assert_succeeded()
                }
            };
            prompt("check native settings at startup", false, false).await?;
            tokio::fs::write(&local, b"{}").await?;
            wait_for_failed_reload(&case, 1).await?;
            let event: Value = serde_json::from_slice(&tokio::fs::read(workspace.join("native-config-local_settings-1.json")).await?)?;
            ensure!(event["file_path"] == json!(local) && event["source"] == "local_settings");
            prompt("check native settings after a veto", false, false).await?;

            let wait_published = |before: u64| {
                let user = &user;
                async move {
                    tokio::time::timeout(Duration::from_secs(20), async {
                        loop {
                            let current = user.client().runtime_status().await?.generation;
                            if current > before { return Ok::<_, anyhow::Error>(current); }
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }).await.context("native settings were not published")?
                }
            };
            let generation = user.client().runtime_status().await?.generation;
            tokio::fs::write(&local, br#"{"claudeMdExcludes":[]}"#).await?;
            let generation = wait_published(generation).await?;
            prompt("check approved local settings", true, false).await?;

            let mut changed: Value = serde_json::from_slice(&tokio::fs::read(&shared).await?)?;
            changed["claudeMdExcludes"].as_array_mut().context("shared exclusion array")?
                .push(json!("**/workspace/CLAUDE.md"));
            tokio::fs::write(&shared, serde_json::to_vec(&changed)?).await?;
            wait_published(generation).await?;
            let event: Value = serde_json::from_slice(&tokio::fs::read(workspace.join("native-config-project_settings-1.json")).await?)?;
            ensure!(event["file_path"] == json!(shared) && event["source"] == "project_settings");
            prompt("check approved project settings", true, true).await?;

            let other_root = case.paths().root.join("another project");
            let rules = other_root.join(".claude/rules");
            tokio::fs::create_dir_all(&rules).await?;
            tokio::fs::write(rules.join("shared.md"), "Other project's instructions").await?;
            let other = case.user().new_session_in(&other_root).await?;
            case.model().expect(RequestMatch::user_text("check settings isolation")).check(|request| {
                ensure!(request.body.to_string().contains("Other project's instructions"));
                Ok(())
            }).reply(Reply::text("isolated"));
            other.send("check settings isolation").await?.wait_settled().await?.assert_succeeded()?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_role_memory_background_teammate_idle_and_mods_extensions() -> TestResult {
    const PLUGIN: &str = "claude.fixture-ext";
    const PNG_2X1: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAAD0lEQVR4nGP4z8DA8J8BAAf/Af8Bf4mnAAAAAElFTkSuQmCC";
    let module = format!(
        r#"
export function register(on) {{
    on('session.start', async ($, e, next) => {{
        await $.tool.register({{ name: 'preview', description: 'Original preview description' }});
        await $.tool.register({{ name: 'hidden_tool', description: 'Must not be offered' }});
        return next(e);
    }});
    on('tool.offer', ($, e, next) => {{
        if (e.tool === 'mcp__fixture-ext__hidden_tool') return {{ offer: false }};
        return next(e);
    }});
    on('tool.describe', {{ tool: 'mcp__fixture-ext__preview' }}, () => ({{
        description: 'Rewritten preview tool summary',
        whenToUse: 'Use when returning a final preview image',
    }}));
    on('tool.run', {{ tool: 'mcp__fixture-ext__preview' }}, async ($) => {{
        const missing = await $.store.get('cas_key', {{ version: true }});
        if (missing !== undefined) throw new Error('expected missing initial key');
        const created = await $.store.set('cas_key', {{ step: 1 }}, {{ ifVersion: 0 }});
        if (!created.ok || created.version !== 1) throw new Error('expected version 1 create');
        const conflict = await $.store.set('cas_key', {{ step: 99 }}, {{ ifVersion: 0 }});
        if (conflict.ok || conflict.version !== 1) throw new Error('expected CAS conflict');
        const read1 = await $.store.get('cas_key', {{ version: true }});
        if (read1.version !== 1 || read1.value.step !== 1) throw new Error('expected version 1 read');
        const updated = await $.store.set('cas_key', {{ step: 2 }}, {{ ifVersion: 1 }});
        if (!updated.ok || updated.version !== 2) throw new Error('expected version 2 update');
        const badDel = await $.store.delete('cas_key', {{ ifVersion: 1 }});
        if (badDel.ok) throw new Error('expected stale delete failure');
        const okDel = await $.store.delete('cas_key', {{ ifVersion: 2 }});
        if (!okDel.ok) throw new Error('expected matching delete success');
        const afterDel = await $.store.get('cas_key');
        if (afterDel !== undefined) throw new Error('expected deleted key');
        return {{
            content: [
                {{ type: 'text', text: 'CAS_VERIFIED_AND_ENDED' }},
                {{ type: 'image', source: {{ type: 'base64', media_type: 'image/png', data: '{PNG_2X1}' }} }},
            ],
            isEnd: true,
        }};
    }});
    on('classic.TeammateIdle', (_$, event) => {{
        if (event.stop_hook_active) return {{}};
        return {{ block: 'teammate-idle-followup:' + event.teammate_name }};
    }});
}}
"#
    );
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .configure_global(|config| {
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["fake-model"]
                ["input"] = json!(["image", "text"]);
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.agent.spawn"] = json!("allow");
            config["permission"]["tools"]["names"][format!("{PLUGIN}.run")] = json!("allow");
            config["permission"]["tools"]["names"][format!("{PLUGIN}.mcp__fixture-ext__preview")] =
                json!("allow");
        })
        .before_start(move |case| {
            let module = module.clone();
            async move {
                let package = case.paths().root.join("ext-plugin");
                tokio::fs::create_dir_all(package.join(".claude-plugin")).await?;
                tokio::fs::create_dir_all(package.join("agents")).await?;
                tokio::fs::create_dir_all(package.join("hooks")).await?;
                tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
                tokio::fs::write(
                    package.join(".claude-plugin/plugin.json"),
                    br#"{"name":"fixture-ext"}"#,
                )
                .await?;
                tokio::fs::write(
                    package.join("agents/researcher.md"),
                    "---\nname: researcher\ndescription: Background researcher\nmemory: project\nbackground: true\n---\nResearch deeply.",
                )
                .await?;
                tokio::fs::write(
                    package.join("hooks/hooks.json"),
                    br#"{"modules":["ext.mjs"]}"#,
                )
                .await?;
                tokio::fs::write(package.join("hooks/ext.mjs"), module).await?;
                let memory_dir = case.workspace().join(".claude/agent-memory/researcher");
                tokio::fs::create_dir_all(&memory_dir).await?;
                tokio::fs::write(
                    memory_dir.join("MEMORY.md"),
                    "Remembered project architecture fact\n",
                )
                .await?;
                case.run_cli([
                    "plugin",
                    "install",
                    package.to_str().context("plugin package path")?,
                    "--enable-hooks",
                ])
                .await?
                .require_success()?;
                Ok(())
            }
        })
        .run(|case| async move {
            let parent = case.user().new_session().await?;

            // 1. Background Claude agent role returns immediately, preloads MEMORY.md,
            //    and triggers classic.TeammateIdle on idle transition.
            let first_child_round = case
                .model()
                .expect(RequestMatch::user_text("collect background findings"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(
                        body.contains("Remembered project architecture fact"),
                        "expected preloaded project MEMORY.md in child prompt"
                    );
                    ensure!(
                        body.contains(".claude/agent-memory/researcher"),
                        "expected memory directory hint in child prompt"
                    );
                    Ok(())
                })
                .reply(Reply::text("initial background draft"));
            case.model()
                .expect(RequestMatch::new("teammate idle continuation", |request| {
                    request
                        .body
                        .to_string()
                        .contains("teammate-idle-followup:plugin_")
                }))
                .after(&first_child_round)
                .reply(Reply::text("background findings verified after idle hook"));

            let launched = parent
                .call_tool(
                    format!("{PLUGIN}.run"),
                    json!({"agent": "researcher", "prompt": "collect background findings"}),
                )
                .await?;
            launched.assert_succeeded()?;
            let launch_data = launched.data()?;
            ensure!(
                launch_data["background"] == true
                    && launch_data["status"] == "running"
                    && launch_data["agent_id"].as_i64().is_some(),
                "{launch_data}"
            );
            let child_id = launch_data["agent_id"].as_i64().context("child agent id")?;
            let child = case.user().attach_session(child_id).await?;
            child
                .wait_state("background Claude agent completed", |state| {
                    state.execution.agent_status
                        == Some(agena_api::resource::AgentStatus::Completed)
                })
                .await?;
            let child_text = child.assistant_text().await?;
            ensure!(
                child_text.contains("background findings verified after idle hook"),
                "expected background child final assistant reply after TeammateIdle continuation, got: {child_text:?}"
            );

            // 2. tool.offer hides hidden_tool, tool.describe rewrites preview summary,
            //    and tool.run verifies versioned $.store CAS, image content, and isEnd: true.
            let list_round = case
                .model()
                .expect(RequestMatch::user_text("inspect mod catalog and end turn"))
                .reply(Reply::function_call(
                    "list-1",
                    "tools_list",
                    json!({"plugin": PLUGIN}),
                ));
            let help_round = case
                .model()
                .expect(RequestMatch::tool_output("list-1"))
                .after(&list_round)
                .check(|request| {
                    let output = request.tool_output_text("list-1").context("tools_list output")?;
                    ensure!(output.contains("mcp__fixture-ext__preview"), "{output}");
                    ensure!(
                        output.contains("Rewritten preview tool summary"),
                        "expected tool.describe summary override: {output}"
                    );
                    ensure!(
                        !output.contains("hidden_tool"),
                        "expected tool.offer to hide hidden_tool: {output}"
                    );
                    Ok(())
                })
                .reply(Reply::function_call(
                    "help-1",
                    "tools_help",
                    json!({"tool": "mcp__fixture-ext__preview"}),
                ));
            case.model()
                .expect(RequestMatch::tool_output("help-1"))
                .after(&help_round)
                .check(|request| {
                    let output = request.tool_output_text("help-1").context("tools_help output")?;
                    ensure!(
                        output.contains("Use when returning a final preview image"),
                        "expected tool.describe help override: {output}"
                    );
                    Ok(())
                })
                .reply(Reply::function_call(
                    "call-1",
                    "tools_call",
                    json!({"tool": "mcp__fixture-ext__preview", "input": {}}),
                ));
            // Note: NO fourth model expectation is registered! If isEnd: true failed
            // to stop the turn after call-1, the fake model server would fail the test.
            let settled = parent
                .send("inspect mod catalog and end turn")
                .await?
                .wait_settled()
                .await?;
            settled.assert_succeeded()?;
            let parts = parent.parts().await?;
            let preview_part = parts
                .iter()
                .rev()
                .find(|part| {
                    part.kind == "tool_call"
                        && part
                            .content
                            .get("tool_api_call")
                            .and_then(|call| {
                                serde_json::from_value::<agena_domain::ToolApiCall>(call.clone())
                                    .ok()
                            })
                            .map(|call| Value::from(call.arguments))
                            .is_some_and(|arguments| {
                                arguments.get("tool").and_then(Value::as_str)
                                    == Some("mcp__fixture-ext__preview")
                            })
                })
                .with_context(|| {
                    format!(
                        "expected mcp__fixture-ext__preview tool_call part in parent parts: {:?}",
                        parts
                            .iter()
                            .filter(|part| part.kind == "tool_call")
                            .map(|part| (&part.part_id, &part.content))
                            .collect::<Vec<_>>()
                    )
                })?;
            let detail = parent
                .client()
                .session_tool_detail(
                    parent.id(),
                    preview_part.part_id,
                    agena_api::live::ToolDetailSection::Output,
                )
                .await?;
            let detail_text = detail.value.to_string();
            ensure!(
                detail_text.contains("CAS_VERIFIED_AND_ENDED"),
                "expected CAS_VERIFIED_AND_ENDED in preview tool detail: {detail_text}"
            );
            ensure!(
                detail_text.contains("image/png"),
                "expected image/png attachment in preview tool detail: {detail_text}"
            );
            Ok(())
        })
        .await
}
