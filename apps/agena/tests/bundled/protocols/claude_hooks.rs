use agena_api::commands::{Command, CommandResult, ForkSessionParams};
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult, Usage};
use anyhow::{Context, ensure};
use serde_json::{Value, json};

const PLUGIN: &str = "claude.fixture-session-start";
const SKILL: &str = "fixture-start-skill";
const SLASH_COMMAND: &str = "fixture-start-command";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_post_model_switch_tracks_reload_and_resume() -> TestResult {
    use agena_api::resource::{ModelRef, RunOptions};
    use agena_testkit::{EditOptions, SettingsLayer};

    const BASE: &str = "fake-model";
    const SAVED: &str = "claude-sonnet-4-6";
    const RELOADED: &str = "claude-opus-4-6";
    const MARKER: &str = "fixture-claude-model-switch";

    let model_options = |model: &str| RunOptions {
        model: Some(ModelRef {
            provider_id: "testkit".into(),
            adapter_id: Some("openai_responses".into()),
            model_id: model.into(),
        }),
        ..Default::default()
    };
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin("claude.fixture-model-switch")
        .configure_global(|config| {
            let models = &mut config["providers"]["testkit"]["adapters"]["openai_responses"]["models"];
            let definition = models[BASE].clone();
            models[SAVED] = definition.clone();
            models[RELOADED] = definition;
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.fs.write"] = json!("allow");
        })
        .before_start(|case| async move {
            let root = case.paths().root.join("claude-model-switch");
            tokio::fs::create_dir_all(root.join(".claude-plugin")).await?;
            tokio::fs::create_dir_all(root.join("hooks")).await?;
            tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
            tokio::fs::write(root.join(".claude-plugin/plugin.json"), br#"{"name":"fixture-model-switch"}"#).await?;
            tokio::fs::write(root.join("hooks/hooks.json"), br#"{"modules":["observe.mjs"]}"#).await?;
            tokio::fs::write(root.join("hooks/observe.mjs"), r#"
export function register(on) {
    on('classic.PreModelSwitch', async ($, event, next) => {
        await $.tool.call({ tool: 'agena.fs.write', path: `model-pre-${event.source}.json`,
            content: JSON.stringify(event) });
        return next(event);
    });
    on('classic.PostModelSwitch', async ($, event, next) => {
        const result = await next(event);
        const values = [event.source, event.from_model, event.to_model, event.requested_model ?? 'null'];
        return {...result, additionalContext: [`fixture-claude-model-switch ${values.join(' ')}`]};
    });
}
"#).await?;
            case.run_cli(["plugin", "install", root.to_str().context("package path")?, "--enable-hooks"])
                .await?.require_success()?;
            Ok(())
        })
        .run(move |case| async move {
            let inherited = case.user().new_session().await?;
            let saved = case.user().new_session().await?;
            let inherited_id = inherited.id();
            let saved_id = saved.id();
            let expect_request = |text: &str, model: &'static str, switch: Option<String>| {
                case.model().expect(RequestMatch::user_text(text)).check(move |request| {
                    ensure!(request.body["model"] == model, "{}", request.body["model"]);
                    let body = request.body.to_string();
                    if let Some(expected) = &switch {
                        ensure!(body.matches(expected).count() == 1, "expected one instance of {expected}");
                    } else {
                        ensure!(!body.contains(MARKER));
                    }
                    Ok(())
                }).reply(Reply::text(format!("answered using {model}")));
            };
            expect_request("initial inherited route", BASE, None);
            inherited.send("initial inherited route").await?.wait_settled().await?.assert_succeeded()?;
            ensure!(inherited.snapshot().await?.execution.selected_selection.model.is_none());

            saved.client().update_session_selection(saved_id, model_options(SAVED)).await?;
            expect_request("initial saved route", SAVED, Some(format!("{MARKER} sdk {BASE} {SAVED} {SAVED}")));
            saved.send("initial saved route").await?.wait_settled().await?.assert_succeeded()?;

            let settings = case.settings();
            settings.set(SettingsLayer::Global, "providers.default_selection.model", json!(RELOADED), EditOptions::default())
                .await?.require_success()?;
            settings.reload_and_wait().await?;
            expect_request("route changed by reload", RELOADED, Some(format!("{MARKER} auto {BASE} {RELOADED} null")));
            inherited.send("route changed by reload").await?.wait_settled().await?.assert_succeeded()?;
            expect_request("inherited route remains selected", RELOADED, None);
            inherited.send("inherited route remains selected").await?.wait_settled().await?.assert_succeeded()?;

            expect_request("inherited route remains selected", SAVED, None);
            let response = inherited.client().continue_run(inherited_id, model_options(SAVED)).await?;
            inherited.attach_run(response.receipt.context("one-off model receipt")?).wait_settled().await?.assert_succeeded()?;
            ensure!(inherited.snapshot().await?.execution.selected_selection.model.is_none());

            case.restart().await?;
            let inherited = case.user().attach_session(inherited_id).await?;
            expect_request("restore the inherited route", RELOADED, None);
            inherited.send("restore the inherited route").await?.wait_settled().await?.assert_succeeded()?;
            let saved = case.user().attach_session(saved_id).await?;
            expect_request("restore the saved route", SAVED, Some(format!("{MARKER} resume {RELOADED} {SAVED} {SAVED}")));
            saved.send("restore the saved route").await?.wait_settled().await?.assert_succeeded()?;
            expect_request("restored route stays selected", SAVED, None);
            saved.send("restored route stays selected").await?.wait_settled().await?.assert_succeeded()?;

            let pre: Value = serde_json::from_slice(&tokio::fs::read(case.workspace().join("model-pre-sdk.json")).await?)?;
            ensure!(pre["from_model"] == BASE && pre["to_model"] == SAVED, "{pre}");
            for source in ["auto", "resume"] {
                ensure!(!case.workspace().join(format!("model-pre-{source}.json")).exists());
            }

            // Persist an undelivered PostModelSwitch context across restart and deliver it once.
            let persisted = case.user().new_session().await?;
            let persisted_id = persisted.id();
            persisted
                .client()
                .update_session_selection(persisted_id, model_options(SAVED))
                .await?;
            // Wait until the background PostModelSwitch hook finishes and writes its context.
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            case.restart().await?;
            let persisted = case.user().attach_session(persisted_id).await?;
            expect_request(
                "deliver saved switch context after restart",
                SAVED,
                Some(format!("{MARKER} sdk {RELOADED} {SAVED} {SAVED}")),
            );
            persisted
                .send("deliver saved switch context after restart")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            expect_request("saved switch context consumed once", SAVED, None);
            persisted
                .send("saved switch context consumed once")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_observes_loaded_project_instructions() -> TestResult {
    use std::{path::Path, time::Duration};

    async fn observed(workspace: &Path, reason: &str) -> anyhow::Result<Value> {
        let path = workspace.join(format!("instructions-{reason}.json"));
        let payload = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(text) = tokio::fs::read_to_string(&path).await
                    && let Ok(payload) = serde_json::from_str::<Value>(&text)
                {
                    break payload;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .with_context(|| format!("wait for InstructionsLoaded ({reason})"))?;
        ensure!(payload["count"] == 1, "{payload}");
        Ok(payload["event"].clone())
    }

    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("claude.fixture-instructions")
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.fs.read"] = json!("allow");
            config["permission"]["tools"]["names"]["agena.fs.write"] = json!("allow");
        })
        .before_start(|case| async move {
            let package = case.paths().root.join("claude-instructions");
            tokio::fs::create_dir_all(package.join(".claude-plugin")).await?;
            tokio::fs::create_dir_all(package.join("hooks")).await?;
            for (path, body) in [
                ("CLAUDE.md", "root Claude guidance"),
                ("nested/CLAUDE.md", "nested Claude guidance"),
                ("nested/example.txt", "nested data"),
                ("native/AGENTS.md", "Agena project guidance"),
                ("native/CLAUDE.md", "lower-priority Claude guidance"),
                ("native/example.txt", "native data"),
            ] {
                let path = case.workspace().join(path);
                tokio::fs::create_dir_all(path.parent().expect("fixture file parent")).await?;
                tokio::fs::write(path, body).await?;
            }
            tokio::fs::write(
                package.join(".claude-plugin/plugin.json"),
                br#"{"name":"fixture-instructions"}"#,
            )
            .await?;
            tokio::fs::write(package.join("hooks/hooks.json"), br#"{"modules":["instructions.mjs"]}"#).await?;
            tokio::fs::write(package.join("hooks/instructions.mjs"), r#"
export function register(on) {
    const counts = new Map();
    on('classic.InstructionsLoaded', async ($, event, next) => {
        const reason = event.load_reason;
        const count = (counts.get(reason) ?? 0) + 1;
        counts.set(reason, count);
        await $.tool.call({tool:'agena.fs.write', path:`instructions-${reason}.json`,
            content:JSON.stringify({event, count})});
        const answer = await next(event);
        return {...answer, continue:false, additionalContext:['suppressed instruction hook output']};
    });
}
"#).await?;
            case.run_cli([
                "plugin",
                "install",
                package.to_str().context("package path")?,
                "--enable-hooks",
            ])
            .await?
            .require_success()?;
            Ok(())
        })
        .run(|case| async move {
            let root = case.workspace().canonicalize()?;
            let user = case.user().new_session().await?;
            case.model()
                .expect(RequestMatch::user_text("load project guidance"))
                .check(|request| {
                    let body = request.body.to_string();
                    ensure!(body.contains("root Claude guidance"));
                    ensure!(!body.contains("suppressed instruction hook output"));
                    Ok(())
                })
                .reply(Reply::text("root loaded"));
            user.send("load project guidance")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;

            let input = observed(&root, "session_start").await?;
            ensure!(input["hook_event_name"] == "InstructionsLoaded", "{input}");
            ensure!(
                input["file_path"] == json!(root.join("CLAUDE.md")),
                "{input}"
            );
            ensure!(
                input["memory_type"] == "Project" && input["load_reason"] == "session_start",
                "{input}"
            );
            ensure!(input.get("trigger_file_path").is_none(), "{input}");

            for (folder, text) in [
                ("nested", "nested Claude guidance"),
                ("native", "Agena project guidance"),
                ("nested", "nested Claude guidance"),
            ] {
                let read = user
                    .call_tool(
                        "fs.read",
                        json!({"file_path":format!("{folder}/example.txt")}),
                    )
                    .await?;
                read.assert_succeeded()?;
                ensure!(read.text().contains(text), "{}", read.text());
            }
            let input = observed(&root, "nested_traversal").await?;
            ensure!(
                input["file_path"] == json!(root.join("nested/CLAUDE.md")),
                "{input}"
            );
            ensure!(
                input["trigger_file_path"] == json!(root.join("nested/example.txt")),
                "{input}"
            );

            case.model()
                .expect(RequestMatch::new("manual compaction", |request| {
                    request
                        .body
                        .to_string()
                        .contains("Summarize the conversation for the next assistant continuation")
                }))
                .reply(Reply::text("Project instructions checkpoint"));
            let compact = user
                .client()
                .compact_session(user.id(), Default::default())
                .await?;
            user.attach_run(compact.receipt.context("compaction receipt")?)
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let input = observed(&root, "compact").await?;
            ensure!(
                input["file_path"] == json!(root.join("CLAUDE.md")),
                "{input}"
            );
            ensure!(input["load_reason"] == "compact", "{input}");
            observed(&root, "session_start").await?;
            observed(&root, "nested_traversal").await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_loads_project_rules_and_reports_provenance() -> TestResult {
    use std::{path::Path, time::Duration};

    async fn observe(root: &Path, reason: &str, file: &str) -> anyhow::Result<Value> {
        let path = root
            .join("hook-observations")
            .join(format!("observed-{reason}-{file}.json"));
        let payload = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(raw) = tokio::fs::read(&path).await
                    && let Ok(event) = serde_json::from_slice::<Value>(&raw)
                {
                    break event;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .with_context(|| format!("wait for InstructionsLoaded ({reason}, {file})"))?;
        ensure!(payload["count"] == 1, "{payload}");
        Ok(payload["event"].clone())
    }

    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin("claude.fixture-rules")
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            for tool in ["fs.read", "fs.read_many", "fs.write", "fs.replace", "shell.exec"] {
                config["permission"]["tools"]["names"][format!("agena.{tool}")] = json!("allow");
            }
        })
        .before_start(|case| async move {
            let workspace = case.workspace();
            for (name, content) in [
                (
                    ".claude/CLAUDE.md",
                    concat!(
                        "Shared Claude guidance\n\n",
                        "@guidance/welcome.md @guidance/welcome-link.md\n",
                        "\"@guidance/quoted.md\" and \u{60}@guidance/code.md\u{60}\n\n",
                        "~~~md\n@guidance/fenced.md\n~~~\n\n",
                        "<!-- Hidden maintainer note @guidance/comment.md -->\n\n",
                        "@guidance/denied.md @guidance/outside.md @../../outside-guidance.md",
                    ),
                ),
                (".claude/guidance/welcome.md", "Welcome guidance\n@Next\\ Steps.md @../CLAUDE.md"),
                (".claude/guidance/Next Steps.md", "Instructions with escaped spaces\n@third.md"),
                (".claude/guidance/third.md", "Third imported guidance\n@fourth.md"),
                (".claude/guidance/fourth.md", "Fourth imported guidance\n@fifth.md"),
                (".claude/guidance/fifth.md", "Unexpected fifth hop"),
                (".claude/guidance/quoted.md", "Unexpected quoted import"),
                (".claude/guidance/code.md", "Unexpected inline code import"),
                (".claude/guidance/fenced.md", "Unexpected fenced import"),
                (".claude/guidance/comment.md", "Unexpected comment import"),
                (".claude/guidance/denied.md", "Unexpected denied import"),
                ("CLAUDE.local.md", "Local Claude guidance"),
                (".claude/rules/general/start.md", "Eager recursive rule"),
                (
                    ".claude/rules/api/scope.md",
                    "---\npaths: 'api/**/*.{ts,tsx}, photos [2024/**'\nignored: ignored YAML field\n---\nScoped API rule\n@notes.md",
                ),
                (".claude/rules/api/notes.md", "---\npaths: ['never/*.none']\n---\nAPI rule included note"),
                ("api/.claude/rules/storage.md", "---\npaths:\n  - 'files/**/*.sql'\n---\nNested SQL write rule"),
                (".claude/rules/replacement.md", "---\npaths: ['edits/*.txt']\n---\nScoped replacement rule"),
                (".claude/rules/shell.md", "---\npaths: ['shell/view.txt']\n---\nScoped shell rule"),
                ("public/info.txt", "Public data"),
                ("api/index.tsx", "API data"),
                ("edits/existing.txt", "before edit"),
                ("shell/view.txt", "Shell data"),
                ("hook-observations/.claude/rules/hook-only.md", "---\npaths: ['*.json']\n---\nHook-only guidance"),
            ] {
                let path = workspace.join(name);
                tokio::fs::create_dir_all(path.parent().context("fixture file parent")?).await?;
                tokio::fs::write(path, content).await?;
            }
            let outside = workspace.parent().context("workspace parent")?.join("outside-guidance.md");
            tokio::fs::write(&outside, "Unexpected outside import").await?;
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&outside, workspace.join(".claude/guidance/outside.md"))?;
                std::os::unix::fs::symlink("welcome.md", workspace.join(".claude/guidance/welcome-link.md"))?;
                std::os::unix::fs::symlink("..", workspace.join(".claude/rules/general/cycle"))?;
            }

            let package = case.paths().root.join("claude-rules");
            tokio::fs::create_dir_all(package.join(".claude-plugin")).await?;
            tokio::fs::create_dir_all(package.join("hooks")).await?;
            tokio::fs::write(package.join(".claude-plugin/plugin.json"), br#"{"name":"fixture-rules"}"#).await?;
            tokio::fs::write(package.join("hooks/hooks.json"), br#"{"modules":["observe.mjs"]}"#).await?;
            tokio::fs::write(package.join("hooks/observe.mjs"), r#"
export function register(on) {
    const counts = new Map();
    on('classic.InstructionsLoaded', async ($, event, next) => {
        const file = event.file_path.split(/[\\/]/).at(-1);
        const path = 'hook-observations/observed-' + event.load_reason + '-' + file + '.json';
        const count = (counts.get(path) || 0) + 1;
        counts.set(path, count);
        await $.tool.call({tool:'agena.fs.write', path, content:JSON.stringify({event,count})});
        return next(event);
    });
}
"#).await?;
            case.run_cli(["plugin", "install", package.to_str().context("fixture package path")?, "--enable-hooks"])
                .await?
                .require_success()?;
            Ok(())
        })
        .run(|case| async move {
            let root = case.workspace().canonicalize()?;
            let user = case.user().new_session().await?;
            let mut policy = serde_json::to_value(user.snapshot().await?.execution.selected_permission)?;
            policy["path"]["rules"] = json!({root.join(".claude/guidance/denied.md").display().to_string(): "deny"});
            user.client().set_session_permission(user.id(), serde_json::from_value(policy)?).await?;

            case.model()
                .expect(RequestMatch::user_text("verify imported memory"))
                .check(|request| {
                    let body = request.body.to_string();
                    for expected in [
                        "Shared Claude guidance", "Welcome guidance", "Instructions with escaped spaces",
                        "Third imported guidance", "Fourth imported guidance", "Local Claude guidance",
                        "Eager recursive rule",
                    ] {
                        ensure!(body.matches(expected).count() == 1, "expected one copy of {expected}");
                    }
                    for absent in ["Unexpected ", "Hidden maintainer note", "Scoped API rule", "Scoped replacement rule", "Nested SQL write rule"] {
                        ensure!(!body.contains(absent), "unexpected guidance {absent}");
                    }
                    Ok(())
                })
                .reply(Reply::tool_call("public-read", "fs.read", json!({"file_path":"public/info.txt"})));
            case.model()
                .expect(RequestMatch::tool_output("public-read"))
                .check(|request| {
                    let output = request.tool_output_text("public-read").context("read result")?;
                    ensure!(output.contains("Public data") && !output.contains("Scoped API rule"), "{output}");
                    Ok(())
                })
                .reply(Reply::tool_call("scoped-read", "fs.read", json!({"file_path":"api/index.tsx"})));
            case.model()
                .expect(RequestMatch::tool_output("scoped-read"))
                .check(|request| {
                    let output = request.tool_output_text("scoped-read").context("scoped read result")?;
                    ensure!(output.contains("Scoped API rule") && output.contains("API rule included note"), "{output}");
                    ensure!(!output.contains("ignored YAML field"), "{output}");
                    Ok(())
                })
                .reply(Reply::text("verified imported memory"));
            user.send("verify imported memory").await?.wait_settled().await?.assert_succeeded()?;

            let start = observe(&root, "session_start", "start.md").await?;
            ensure!(start["file_path"] == json!(root.join(".claude/rules/general/start.md")), "{start}");
            let local = observe(&root, "session_start", "CLAUDE.local.md").await?;
            ensure!(local["memory_type"] == "Local", "{local}");
            let imported = observe(&root, "include", "Next Steps.md").await?;
            ensure!(imported["file_path"] == json!(root.join(".claude/guidance/Next Steps.md")), "{imported}");
            ensure!(imported["parent_file_path"] == json!(root.join(".claude/guidance/welcome.md")), "{imported}");
            ensure!(imported.get("trigger_file_path").is_none(), "{imported}");
            let scoped = observe(&root, "path_glob_match", "scope.md").await?;
            ensure!(scoped["globs"] == json!(["api/**/*.{ts,tsx}", "photos [2024/**"]), "{scoped}");
            ensure!(scoped["trigger_file_path"] == json!(root.join("api/index.tsx")), "{scoped}");
            let scoped_import = observe(&root, "include", "notes.md").await?;
            ensure!(scoped_import["parent_file_path"] == json!(root.join(".claude/rules/api/scope.md")), "{scoped_import}");
            ensure!(scoped_import["trigger_file_path"] == scoped["trigger_file_path"], "{scoped_import}");

            let partial = user.call_tool("fs.read_many", json!({"paths":["api/missing.ts", "public/info.txt"]})).await?;
            partial.assert_succeeded()?;
            ensure!(!partial.text().contains("Scoped API rule"), "{}", partial.text());

            let write = user.call_tool("fs.write", json!({
                "path":"api/files/query.sql", "content":"select 1", "create_parents":true
            })).await?;
            write.assert_succeeded()?;
            write.assert_text_contains("Nested SQL write rule")?;
            let sql = observe(&root, "path_glob_match", "storage.md").await?;
            ensure!(sql["trigger_file_path"] == json!(root.join("api/files/query.sql")), "{sql}");

            let edit = user.call_tool("fs.replace", json!({
                "path":"edits/existing.txt", "old":"before edit", "new":"after edit"
            })).await?;
            edit.assert_succeeded()?;
            edit.assert_text_contains("Scoped replacement rule")?;
            let edit = observe(&root, "path_glob_match", "replacement.md").await?;
            ensure!(edit["trigger_file_path"] == json!(root.join("edits/existing.txt")), "{edit}");

            #[cfg(unix)]
            {
                let pipeline = user.call_tool("shell.exec", json!({
                    "command":"cat view.txt | head -n 1", "workdir":"shell", "reads":["view.txt"]
                })).await?;
                pipeline.assert_succeeded()?;
                ensure!(!pipeline.text().contains("Scoped shell rule"), "{}", pipeline.text());
                let view = user.call_tool("shell.exec", json!({
                    "command":"head -n 1 -- view.txt", "workdir":"shell", "reads":["view.txt"]
                })).await?;
                view.assert_succeeded()?;
                view.assert_text_contains("Scoped shell rule")?;
                let shell = observe(&root, "path_glob_match", "shell.md").await?;
                ensure!(shell["trigger_file_path"] == json!(root.join("shell/view.txt")), "{shell}");
            }
            ensure!(!root.join("hook-observations/observed-path_glob_match-hook-only.md.json").exists());
            Ok(())
        })
        .await
}

async fn install(case: TestCase) -> TestResult {
    let package = case.paths().root.join("session-start-plugin");
    tokio::fs::create_dir(case.paths().root.join("empty-bin")).await?;
    tokio::fs::create_dir_all(case.workspace().join(".claude/skills").join(SKILL)).await?;
    tokio::fs::create_dir_all(case.workspace().join(".claude/commands")).await?;
    tokio::fs::create_dir_all(package.join(".claude-plugin")).await?;
    tokio::fs::create_dir_all(package.join("hooks")).await?;
    tokio::fs::write(
        package.join(".claude-plugin/plugin.json"),
        br#"{"name":"fixture-session-start"}"#,
    )
    .await?;
    tokio::fs::write(
        package.join("hooks/hooks.json"),
        br#"{"modules":["session-start.mjs"]}"#,
    )
    .await?;
    tokio::fs::write(
        package.join("hooks/session-start.mjs"),
        r#"
export function register(on) {
    on('classic.SessionStart', async ($, event, next) => {
        const answer = await next(event);
        const write = (path, content) => $.tool.call({tool:'agena.fs.write', path, content});
        await write('session-start-' + event.source + '.json', JSON.stringify(event));
        if (event.source === 'startup') {
            await write('.claude/skills/fixture-start-skill/SKILL.md',
                '---\nname: fixture-start-skill\ndescription: Installed during SessionStart.\n---\nStartup skill body');
            await write('.claude/commands/fixture-start-command.md',
                '---\ndescription: Installed during SessionStart.\n---\nStartup command body');
            return {...answer, reloadSkills: true, sessionTitle: 'Named by SessionStart'};
        }
        return {...answer, additionalContext: ['SessionStart context for ' + event.source]};
    });
}
"#,
    )
    .await?;
    case.run_cli([
        "plugin",
        "install",
        package.to_str().context("fixture package path")?,
        "--enable-hooks",
    ])
    .await?
    .require_success()?;
    Ok(())
}

async fn hook_input(case: &TestCase, source: &str) -> anyhow::Result<Value> {
    let path = case
        .workspace()
        .join(format!("session-start-{source}.json"));
    let input = tokio::fs::read(&path)
        .await
        .with_context(|| format!("read the {source} payload recorded by the plugin"))?;
    Ok(serde_json::from_slice(&input)?)
}

fn assert_cache_input(input: &Value, source: &str, context_tokens: u64) -> TestResult {
    ensure!(input["source"] == source, "{input}");
    ensure!(input["model"] == "fake-model", "{input}");
    ensure!(input["context_tokens"] == context_tokens, "{input}");
    ensure!(input["seconds_since_last_response"].is_u64(), "{input}");
    ensure!(input["prompt_cache_likely_expired"] == false, "{input}");
    let cost = input["estimated_cache_write_usd"]
        .as_f64()
        .with_context(|| format!("missing cache price estimate: {input}"))?;
    ensure!(
        (cost - context_tokens as f64 * 4.0 / 1_000_000.0).abs() < 1e-9,
        "{input}"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_session_start_reloads_skills_and_reports_resume_cache() -> TestResult {
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            config["permission"]["tools"]["names"]["agena.fs.write"] = json!("allow");
            config["permission"]["tools"]["names"]["agena.commands.refresh"] = json!("allow");
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["fake-model"]
                ["pricing"]["cache_write_usd_per_million_tokens"] = json!("4.0");
        })
        .before_start(install)
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let client = user.client();
            let startup = hook_input(&case, "startup").await?;
            ensure!(startup["source"] == "startup", "{startup}");
            ensure!(
                startup.get("seconds_since_last_response").is_none(),
                "{startup}"
            );
            let catalog = client.plugin_surface_catalog_for_session(user.id()).await?;
            let global_catalog = client.plugin_surface_catalog().await?;
            for name in [SKILL, SLASH_COMMAND] {
                ensure!(
                    catalog.catalog.commands.iter().any(|entry| {
                        entry.plugin_id.to_string() == "agena.commands" && entry.command.id == name
                    }),
                    "SessionStart did not publish {name} before the first model request"
                );
                ensure!(
                    global_catalog
                        .catalog
                        .commands
                        .iter()
                        .all(|entry| entry.command.id != name),
                    "SessionStart published the project entry {name} to the instance catalog"
                );
            }
            for (name, body) in [
                (SKILL, "Startup skill body"),
                (SLASH_COMMAND, "Startup command body"),
            ] {
                let result = user
                    .call_tool("agena.commands.get", json!({"name":name}))
                    .await?;
                result.assert_succeeded()?;
                result.assert_text_contains(body)?;
            }

            case.model()
                .expect(RequestMatch::user_text("record cache usage"))
                .reply(Reply::text("response with known usage").usage(Usage::tokens(12_300, 45)));
            let first = user
                .send("record cache usage")
                .await?
                .wait_settled()
                .await?;
            first.assert_assistant_text("response with known usage")?;
            let first_response = first
                .parts
                .iter()
                .find(|part| part.role == "assistant")
                .context("first response in the fork prefix")?
                .part_id;
            case.model()
                .expect(RequestMatch::user_text("record later cache usage"))
                .reply(Reply::text("newer response").usage(Usage::tokens(23_400, 56)));
            user.send("record later cache usage")
                .await?
                .wait_settled()
                .await?
                .assert_assistant_text("newer response")?;

            let fork = client
                .command(Command::ForkSession(ForkSessionParams {
                    session_id: user.id(),
                    title: Some("cache fixture fork".into()),
                    at_message_id: Some(first_response),
                    conversation_mode: None,
                    expected_version: None,
                }))
                .await?;
            ensure!(matches!(fork, CommandResult::Execution(_)));
            assert_cache_input(&hook_input(&case, "fork").await?, "fork", 12_345)?;

            case.restart().await?;
            let user = case.user().attach_session(user.id()).await?;
            case.model()
                .expect(RequestMatch::user_text("verify resume context"))
                .check(|request| {
                    ensure!(
                        request
                            .body
                            .to_string()
                            .contains("SessionStart context for resume"),
                        "first request after restart missed SessionStart context"
                    );
                    Ok(())
                })
                .reply(Reply::text("resumed"));
            user.send("verify resume context")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let resume = hook_input(&case, "resume").await?;
            ensure!(
                resume["session_title"] == "Named by SessionStart",
                "{resume}"
            );
            assert_cache_input(&resume, "resume", 23_456)
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_worktree_role_owns_checkout_across_restart() -> TestResult {
    run_worktree_role(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_role_preloads_skills_and_omits_initial_claude_md() -> TestResult {
    const PLUGIN: &str = "claude.role-options";
    const SKILL: &str = "ROLE_PRELOADED_SKILL";
    const CLAUDE_MD: &str = "PROJECT_CLAUDE_GUIDANCE";
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .expect_plugin(PLUGIN)
        .configure_global(|config| {
            config["permission"]["path"]["workspace"] = json!("readwrite");
            for tool in ["claude.role-options.run", "agena.agent.spawn"] {
                config["permission"]["tools"]["names"][tool] = json!("allow");
            }
        })
        .before_start(|case| async move {
            let root = case.workspace().join("role-plugin");
            for path in [".claude-plugin", "agents", "skills/review-guide"] {
                tokio::fs::create_dir_all(root.join(path)).await?;
            }
            tokio::fs::write(root.join(".claude-plugin/plugin.json"), r#"{
                "name":"role-options",
                "userConfig":{
                    "areas":{"type":"string","multiple":true,"default":["backend","frontend"]},
                    "credential":{"type":"string","sensitive":true,"default":"never-include-me"}
                }
            }"#).await?;
            tokio::fs::write(root.join("skills/review-guide/SKILL.md"), format!(
                "---\nname: review-guide\ndescription: Review conventions\n---\n{SKILL} from ${{CLAUDE_SKILL_DIR}} for ${{user_config.areas}}\n"
            )).await?;
            tokio::fs::write(root.join("agents/reviewer.md"), "---\nname: reviewer\ndescription: Review with a preloaded skill\npermissionMode: bypassPermissions\nmcpServers: [unused]\nskills: [role-options:review-guide]\nomitClaudeMd: true\n---\nReview the submitted work. The credential is ${user_config.credential}.\n").await?;
            tokio::fs::write(case.workspace().join("CLAUDE.md"), CLAUDE_MD).await?;
            case.run_cli(["plugin", "install", root.to_str().context("package path")?])
                .await?.require_success()?;
            Ok(())
        })
        .run(|case| async move {
            let parent = case.user().new_session().await?;
            case.model().expect(RequestMatch::user_text("host turn")).check(|request| {
                let body = request.body.to_string();
                ensure!(body.contains(CLAUDE_MD) && !body.contains(SKILL), "{body}");
                Ok(())
            }).reply(Reply::text("host answered"));
            parent.send("host turn").await?.wait_settled().await?.assert_succeeded()?;
            case.model().expect(RequestMatch::user_text("child review")).check(|request| {
                let body = request.body.to_string();
                ensure!(body.contains(SKILL) && body.contains("skills/review-guide"), "{body}");
                ensure!(!body.contains(CLAUDE_MD), "{body}");
                ensure!(body.contains("backend") && body.contains("frontend"), "{body}");
                ensure!(body.contains("[redacted]") && !body.contains("never-include-me"), "{body}");
                Ok(())
            }).reply(Reply::text("review done"));
            let result = parent.call_tool(format!("{PLUGIN}.run"), json!({"agent":"reviewer","prompt":"child review"})).await?;
            result.assert_succeeded()?;
            result.assert_text_contains("review done")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_claude_worktree_preserves_the_project_subdirectory() -> TestResult {
    run_worktree_role(true).await
}

async fn run_worktree_role(nested: bool) -> TestResult {
    const PLUGIN: &str = "claude.fixture-worktree";
    const PROMPT: &str = "Use the isolated checkout for this task";
    const MODEL: &str = "claude-sonnet-4-6";
    super::super::fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin(PLUGIN)
        .configure_global(move |config| {
            let models =
                &mut config["providers"]["testkit"]["adapters"]["openai_responses"]["models"];
            models[MODEL] = models["fake-model"].clone();
            config["permission"]["path"]["workspace"] = json!("readwrite");
            if nested {
                config["permission"]["path"]["external"] = json!("readonly");
            }
            for tool in [
                "claude.fixture-worktree.run",
                "agena.agent.spawn",
                "agena.fs.write",
            ] {
                config["permission"]["tools"]["names"][tool] = json!("allow");
            }
        })
        .before_start(move |case| async move {
            let repository = if nested {
                case.paths().root.as_path()
            } else {
                case.workspace()
            };
            let git = |args: &[&str]| -> anyhow::Result<()> {
                let output = std::process::Command::new("git")
                    .args(args)
                    .current_dir(repository)
                    .output()?;
                ensure!(
                    output.status.success(),
                    "git {args:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                Ok(())
            };
            git(&["init", "-q"])?;
            tokio::fs::write(case.workspace().join("tracked.txt"), "base checkout").await?;
            tokio::fs::write(
                repository.join(".gitignore"),
                "/.env.fixture\n/.env.link\n/.cache/\n/fixtures/\n/vendor/\ntracked.txt\n",
            )
            .await?;
            let tracked = if nested {
                "workspace/tracked.txt"
            } else {
                "tracked.txt"
            };
            git(&["add", "--force", "--", ".gitignore", tracked])?;
            git(&[
                "-c",
                "user.name=Worktree Fixture",
                "-c",
                "user.email=fixture@example.com",
                "commit",
                "-qm",
                "fixture",
            ])?;
            tokio::fs::write(case.workspace().join("tracked.txt"), "source-only edit").await?;
            tokio::fs::write(
                repository.join(".worktreeinclude"),
                "# Local checkout settings\n.env.fixture\n.env.link\ntracked.txt\nunignored.txt\nfixtures/\n!fixtures/excluded.json\n**/private.json\n**/vendor/runtime/config.json\n",
            )
            .await?;
            for directory in [".cache", "fixtures", "vendor/runtime"] {
                tokio::fs::create_dir_all(repository.join(directory)).await?;
            }
            for (path, text) in [
                (".env.fixture", "WORKTREE_COLOR=blue"),
                ("unignored.txt", "leave in the source checkout"),
                (".cache/private.json", "leave under an unmatched ignored directory"),
                ("fixtures/retained.json", "copy this local fixture"),
                ("fixtures/excluded.json", "leave this excluded fixture"),
                ("vendor/runtime/config.json", "copy this explicitly reached fixture"),
            ] {
                tokio::fs::write(repository.join(path), text).await?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(
                repository.join("unignored.txt"),
                repository.join(".env.link"),
            )?;
            let package = case.paths().root.join("worktree-plugin");
            tokio::fs::create_dir_all(package.join(".claude-plugin")).await?;
            tokio::fs::create_dir_all(package.join("agents")).await?;
            tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
            tokio::fs::write(
                package.join(".claude-plugin/plugin.json"),
                r#"{"name":"fixture-worktree"}"#,
            )
            .await?;
            tokio::fs::write(
                package.join("agents/reviewer.md"),
                "---
name: reviewer
description: Review an isolated checkout
model: sonnet
isolation: worktree
tools: [Read, Write]
---
Inspect the checkout in isolation.
",
            )
            .await?;
            case.run_cli([
                "plugin",
                "install",
                package.to_str().context("package path")?,
                "--enable-hooks",
            ])
            .await?
            .require_success()?;
            Ok(())
        })
        .run(move |case| async move {
            let parent = case.user().new_session().await?;
            case.model()
                .expect(RequestMatch::user_text(PROMPT))
                .check(|request| {
                    ensure!(request.body["model"] == MODEL, "{}", request.body["model"]);
                    Ok(())
                })
                .reply(Reply::text("isolated review complete"));
            let result = parent
                .call_tool(
                    format!("{PLUGIN}.run"),
                    json!({"agent":"reviewer","prompt":PROMPT}),
                )
                .await?;
            result.assert_succeeded()?;
            ensure!(
                result.text().contains("isolated review complete"),
                "{}",
                result.text()
            );

            let http = case.http().await;
            let children: Value = http
                .get(&format!("/api/v1/sessions?parent_id={}", parent.id()))
                .await?
                .require_success()?
                .json()?;
            let child = children["items"][0]["id"]
                .as_i64()
                .context("isolated child session")?;
            let mut trees = tokio::fs::read_dir(case.paths().data_dir.join("worktrees")).await?;
            let checkout = trees.next_entry().await?.context("owned checkout")?.path();
            ensure!(trees.next_entry().await?.is_none());
            let project = if nested {
                checkout.join("workspace")
            } else {
                checkout.clone()
            };
            ensure!(
                tokio::fs::read_to_string(project.join("tracked.txt")).await? == "base checkout"
            );
            for (path, expected) in [
                (".env.fixture", "WORKTREE_COLOR=blue"),
                ("fixtures/retained.json", "copy this local fixture"),
                ("vendor/runtime/config.json", "copy this explicitly reached fixture"),
            ] {
                ensure!(tokio::fs::read_to_string(checkout.join(path)).await? == expected, "{path}");
            }
            for path in [
                ".env.link",
                "unignored.txt",
                ".cache/private.json",
                "fixtures/excluded.json",
            ] {
                ensure!(!checkout.join(path).exists(), "unexpected worktree file {path}");
            }

            case.restart().await?;
            let isolated = case.user().attach_session(child).await?;
            let result = isolated
                .call_tool(
                    "fs.write",
                    json!({"path":"local.txt","content":"isolated change"}),
                )
                .await?;
            result.assert_succeeded()?;
            ensure!(
                tokio::fs::read_to_string(project.join("local.txt")).await? == "isolated change"
            );
            ensure!(!case.workspace().join("local.txt").exists());

            let http = case.http().await;
            let route = format!("/api/v1/sessions/{child}");
            let delete = || http.request(reqwest::Method::DELETE, &route);
            let refused = http.execute(delete()?).await?;
            ensure!(
                !refused.status.is_success() && project.join("local.txt").is_file(),
                "{refused:?}"
            );

            tokio::fs::remove_file(project.join("local.txt")).await?;
            let copied = checkout.join(".env.fixture");
            tokio::fs::write(&copied, "WORKTREE_COLOR=red!").await?;
            let refused = http.execute(delete()?).await?;
            ensure!(
                !refused.status.is_success() && copied.is_file(),
                "edited included file was eligible for removal: {refused:?}"
            );
            tokio::fs::write(&copied, "WORKTREE_COLOR=blue").await?;
            http.execute(delete()?).await?.require_success()?;
            ensure!(
                !checkout.exists(),
                "checkout retained after explicit removal"
            );
            ensure!(
                tokio::fs::read_to_string(case.workspace().join("tracked.txt")).await?
                    == "source-only edit"
            );
            Ok(())
        })
        .await
}
