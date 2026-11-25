//! System prompts, durable memory and proactive contact in project conversations.
use super::fixture;
use agena_testkit::{ModelHandle, Reply, RequestMatch, SessionOptions, TestResult, UserSession};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::time::Duration;

async fn ask(
    source: &UserSession,
    model: &ModelHandle,
    text: &str,
    id: &str,
    tool: &str,
    input: Value,
    expected: &str,
) -> Result<()> {
    let start = model
        .expect(RequestMatch::user_text(text))
        .reply(Reply::tool_call(id, tool, input));
    model
        .expect(RequestMatch::tool_output(id))
        .after(&start)
        .check_tool_output_contains(id, expected)
        .reply(Reply::text(format!("assistant-action-complete-{id}")));
    source
        .send(text)
        .await?
        .wait_settled()
        .await?
        .assert_succeeded()?;
    Ok(())
}
fn notice(marker: &'static str) -> RequestMatch {
    RequestMatch::new(format!("scheduled contact {marker}"), move |r| {
        let b = r.body.to_string();
        b.contains("Scheduled assistant contact") && b.contains(marker)
    })
}

fn instructions(request: &agena_testkit::ModelRequest) -> Result<String> {
    request
        .body
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("model request has no instructions")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delegated_sessions_discover_profiles_and_load_readonly_research_context() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(120))
        .configure_global(|config| {
            config["sandbox"] = json!(false);
            config["profiles"] = json!({
                "research-check": {
                    "extends": "explore",
                    "description": "Trace the research fixture",
                    "context": {"preload_skills":["research-fixture"]}
                },
                "private-check": {"extends":"explore","hidden":true},
                "primary-check": {"extends":"main"}
            });
        })
        .seed_file("workspace/AGENTS.md", b"PROJECT_GUIDANCE_PROBE".to_vec())
        .seed_file("workspace/research.txt", b"READ_ONLY_RESEARCH_PROBE".to_vec())
        .seed_file(
            "workspace/.agena/skills/research-fixture/SKILL.md",
            b"---\nname: research-fixture\ndescription: Inspect the research fixture\n---\n\nRESEARCH_SKILL_PRELOAD_PROBE\n".to_vec(),
        )
        .run(|case| async move {
            let model = case.model();
            let parent = case.user().new_session().await?;
            let research_spawn = model
                .expect(RequestMatch::user_text("coordinate-research-probe"))
                .check(|request| {
                    let prompt = instructions(request)?;
                    ensure!(prompt.contains("- `research-check` (read-only)"));
                    ensure!(!prompt.contains("- `private-check`") && !prompt.contains("- `primary-check`"));
                    ensure!(prompt.contains("PROJECT_GUIDANCE_PROBE"));
                    Ok(())
                })
                .reply(Reply::tool_call("readonly-spawn", "agent.spawn", json!({
                    "name":"research_probe", "profile":"research-check",
                    "prompt":"nested-research-probe"
                })));
            let wait_help = model
                .expect(RequestMatch::tool_output("readonly-spawn"))
                .after(&research_spawn)
                .check_tool_output_contains("readonly-spawn", "research-check")
                .reply(Reply::function_call("wait-research-help", "tools_help", json!({"tool":"agent.wait"})));
            let wait = model
                .expect(RequestMatch::tool_output("wait-research-help"))
                .after(&wait_help)
                .reply(Reply::function_call(
                    "wait-research", "tools_call", json!({"tool":"agent.wait","input":{"timeout_ms":30000}})
                ));
            let read = model
                .expect(RequestMatch::user_text("nested-research-probe"))
                .check(|request| {
                    let prompt = instructions(request)?;
                    ensure!(prompt.contains("Selected profile: `research-check`"));
                    ensure!(prompt.contains("RESEARCH_SKILL_PRELOAD_PROBE"));
                    ensure!(!prompt.contains("PROJECT_GUIDANCE_PROBE"));
                    ensure!(!prompt.contains("# Available subagent profiles"));
                    Ok(())
                })
                .reply(Reply::tool_call("nested-read", "fs.read", json!({"file_path":"research.txt"})));
            let blocked_write = model
                .expect(RequestMatch::tool_output("nested-read"))
                .after(&read)
                .check_tool_output_contains("nested-read", "READ_ONLY_RESEARCH_PROBE")
                .reply(Reply::tool_call("nested-write", "fs.write", json!({
                    "path":"unexpected-research.txt", "content":"unexpected"
                })));
            let research_result = model
                .expect(RequestMatch::tool_output("nested-write"))
                .after(&blocked_write)
                .check_tool_output_contains("nested-write", "\"phase\":\"failed\"")
                .reply(Reply::text("nested-research-finished").held());
            let coordinator_result = model
                .expect(RequestMatch::tool_output("wait-research"))
                .after(&wait)
                .check(|request| {
                    ensure!(request.body.to_string().contains("nested-research-finished"));
                    Ok(())
                })
                .reply(Reply::text("research-coordinator-finished"));
            model
                .expect(RequestMatch::new("coordinator delivery", |request| {
                    request.body.to_string().contains("research-coordinator-finished")
                }))
                .after(&coordinator_result)
                .reply(Reply::text("received-nested-research"));
            parent.call_tool("agent.spawn", json!({
                "name":"research_coordinator", "profile":"worker", "prompt":"coordinate-research-probe"
            })).await?.assert_succeeded()?;
            wait.observed().await?;
            research_result.observed().await?;
            research_result.release();
            parent.wait_text_contains("received-nested-research").await?;
            ensure!(!case.workspace().join("unexpected-research.txt").exists());
            Ok(())
        })
        .await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_verification_uses_shell_exits_and_reports_missing_evidence() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(120))
        .configure_global(|config| {
            config["sandbox"] = json!(false);
        })
        .run(|case| async move {
            let parent = case.user().new_session().await?;
            let model = case.model();
            let early_reply = model
                .expect(RequestMatch::user_text("verify-recorded-outcomes"))
                .check(|request| {
                    ensure!(instructions(request)?.contains("echo verification-proof"));
                    Ok(())
                })
                .reply(Reply::text("I claim all checks passed"));
            let pass = model
                .expect(RequestMatch::new("missing check reminder", |request| {
                    request
                        .body
                        .to_string()
                        .contains("no recorded shell.exec results")
                }))
                .after(&early_reply)
                .reply(Reply::tool_call(
                    "verify-pass",
                    "shell.exec",
                    json!({
                        "command":"echo verification-proof"
                    }),
                ));
            let fail = model
                .expect(RequestMatch::tool_output("verify-pass"))
                .after(&pass)
                .check_tool_output_contains("verify-pass", "verification-proof")
                .reply(Reply::tool_call(
                    "verify-fail",
                    "shell.exec",
                    json!({"command":"false"}),
                ));
            let finish = model
                .expect(RequestMatch::tool_output("verify-fail"))
                .after(&fail)
                .check_tool_output_contains("verify-fail", "exit_code")
                .reply(Reply::text("recorded-checks-finished").held());
            model
                .expect(RequestMatch::new("verification delivery", |request| {
                    request
                        .body
                        .to_string()
                        .contains("recorded-checks-finished")
                }))
                .after(&finish)
                .check(|request| {
                    let prompt = request.body.to_string();
                    for expected in [
                        "PASS `echo verification-proof` (exit 0, output resource",
                        "FAIL `false` (exit 1, output resource",
                        "BLOCKED `echo missing-proof` (no matching shell.exec result)",
                    ] {
                        ensure!(
                            prompt.contains(expected),
                            "missing {expected:?} in delivered output"
                        );
                    }
                    Ok(())
                })
                .reply(Reply::text("received-recorded-verification"));
            parent.call_tool("agent.spawn", json!({
                "name":"recorded_checks", "profile":"verifier", "prompt":"verify-recorded-outcomes",
                "verification_commands":["echo verification-proof","false","echo missing-proof"]
            })).await?.assert_succeeded()?;
            finish.observed().await?;
            finish.release();
            parent
                .wait_text_contains("received-recorded-verification")
                .await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_worktree_uses_a_pinned_git_base_and_is_reused_for_followup() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(120))
        .configure_global(|config| {
            config["sandbox"] = json!(false);
            config["profiles"] = json!({
                "isolated-worker":{"extends":"worker","isolation":"worktree"}
            });
        })
        .seed_file("workspace/isolation.txt", b"committed-isolation-probe".to_vec())
        .run(|case| async move {
            let git = |args: &[&str]| -> Result<String> {
                let result = std::process::Command::new("git")
                    .current_dir(case.workspace()).args(args).output()?;
                ensure!(result.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&result.stderr));
                Ok(String::from_utf8(result.stdout)?.trim().to_owned())
            };
            git(&["init", "-q"])?;
            git(&["add", "isolation.txt"])?;
            git(&["-c", "user.name=Agena Test", "-c", "user.email=agena-test@example.invalid",
                "-c", "commit.gpgsign=false", "commit", "-qm", "Base checkout"])?;
            let initial = git(&["rev-parse", "HEAD"])?;
            let original = case.workspace().join("isolation.txt");
            tokio::fs::write(&original, "uncommitted-parent-probe").await?;
            let parent = case.user().new_session().await?;
            let model = case.model();
            let read = model
                .expect(RequestMatch::user_text("worktree-initial-probe"))
                .check(|request| {
                    ensure!(instructions(request)?.contains("Use the assigned Git worktree"));
                    Ok(())
                })
                .reply(Reply::tool_call("worktree-read", "fs.read", json!({"file_path":"isolation.txt"})));
            let write = model
                .expect(RequestMatch::tool_output("worktree-read"))
                .after(&read)
                .check_tool_output_contains("worktree-read", "committed-isolation-probe")
                .reply(Reply::tool_call("worktree-write", "fs.write", json!({
                    "path":"child-result.txt", "content":"isolated-child-probe"
                })));
            let finished = model
                .expect(RequestMatch::tool_output("worktree-write"))
                .after(&write)
                .check_tool_output_contains("worktree-write", "\"phase\":\"succeeded\"")
                .reply(Reply::text("worktree-probe-finished").held());
            model
                .expect(RequestMatch::new("worktree delivery", |request| {
                    request.body.to_string().contains("worktree-probe-finished")
                }))
                .after(&finished)
                .check(|request| {
                    ensure!(request.body.to_string().contains("1 changed paths"));
                    Ok(())
                })
                .reply(Reply::text("received-worktree-result"));
            let spawned = parent.call_tool("agent.spawn", json!({
                "name":"isolated_probe", "profile":"isolated-worker", "prompt":"worktree-initial-probe"
            })).await?;
            spawned.assert_succeeded()?;
            let chosen = &spawned.data()?["worktree"];
            ensure!(chosen["base_commit"] == initial);
            let checkout = std::path::PathBuf::from(chosen["path"].as_str().context("missing checkout path")?);
            ensure!(checkout.starts_with(case.data_dir()));
            finished.observed().await?;
            finished.release();
            parent.wait_text_contains("received-worktree-result").await?;
            ensure!(tokio::fs::read_to_string(checkout.join("child-result.txt")).await? == "isolated-child-probe");
            ensure!(tokio::fs::read_to_string(&original).await? == "uncommitted-parent-probe");
            ensure!(!case.workspace().join("child-result.txt").exists());

            let followup = model
                .expect(RequestMatch::user_text("worktree-followup-probe"))
                .reply(Reply::tool_call("followup-read", "fs.read", json!({"file_path":"child-result.txt"})));
            let followup_done = model
                .expect(RequestMatch::tool_output("followup-read"))
                .after(&followup)
                .check_tool_output_contains("followup-read", "isolated-child-probe")
                .reply(Reply::text("worktree-followup-finished").held());
            model
                .expect(RequestMatch::new("followup delivery", |request| {
                    request.body.to_string().contains("worktree-followup-finished")
                }))
                .after(&followup_done)
                .reply(Reply::text("received-worktree-followup"));
            let reused = parent.call_tool("agent.followup", json!({
                "target":"isolated_probe", "message":"worktree-followup-probe"
            })).await?;
            reused.assert_succeeded()?;
            ensure!(reused.data()?["worktree"] == *chosen);
            ensure!(reused.data()?["agent_id"] == spawned.data()?["agent_id"]);
            followup_done.observed().await?;
            followup_done.release();
            parent.wait_text_contains("received-worktree-followup").await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delegated_planner_and_verifier_use_their_prompt_fragments_and_permissions() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(120))
        .configure_global(|config| {
            config["sandbox"] = json!(false);
            config["resource_guard"] = json!(true);
        })
        .seed_file("workspace/role-check.txt", b"role-check-evidence\n".to_vec())
        .run(|case| async move {
            let parent = case.user().new_session().await?;
            parent
                .client()
                .set_session_permission(
                    parent.id(),
                    serde_json::from_value(json!({
                        "path": {"workspace": "readwrite"},
                        "tools": {
                            "names": {"agena.shell.exec": "allow"},
                            "rules": {"shell.*": {"*parent-blocked-check*": "deny"}}
                        }
                    }))?,
                )
                .await?;

            let (shell, read_command, write_command) = if cfg!(windows) {
                (
                    "powershell",
                    "Get-Content role-check.txt",
                    "Set-Content role-check.txt unexpected-write",
                )
            } else {
                (
                    "bash",
                    "cat role-check.txt",
                    "printf unexpected-write > role-check.txt",
                )
            };
            let read = json!({
                "shell": shell, "command": read_command, "reads": ["role-check.txt"], "writes": [], "network": []
            });
            let write = json!({
                "shell": shell, "command": write_command, "reads": [], "writes": ["role-check.txt"], "network": []
            });
            let blocked = json!({
                "shell": shell, "command": "echo parent-blocked-check", "reads": [], "writes": [], "network": []
            });
            let model = case.model();
            for (role, heading, method, method_heading, checks) in [
                (
                    "planner",
                    "# Planner role",
                    "agena.profile.planning",
                    "# Planning and acceptance criteria",
                    vec![
                        ("fs.read", json!({"file_path": "role-check.txt"}), "succeeded", "role-check-evidence"),
                        ("shell.exec", read.clone(), "failed", "denied"),
                    ],
                ),
                (
                    "verifier",
                    "# Verifier role",
                    "agena.profile.verification",
                    "# Verification and evidence",
                    vec![
                        ("shell.exec", read, "succeeded", "role-check-evidence"),
                        ("shell.exec", write, "failed", "denied"),
                        ("shell.exec", blocked, "failed", "denied"),
                    ],
                ),
            ] {
                let assignment = format!("start-{role}-probe");
                let report = format!("{role}-probe-completed");
                let delivered = format!("received-{role}-probe");
                let mut next = model.expect(RequestMatch::user_text(&assignment)).check(move |r| {
                    let prompt = instructions(r)?;
                    ensure!(prompt.contains(heading) && prompt.contains(method_heading));
                    ensure!(prompt.contains("# Delegated session"));
                    Ok(())
                });
                for (index, (tool, input, phase, evidence)) in checks.into_iter().enumerate() {
                    let call_id = format!("{role}-check-{index}");
                    let check = next.reply(Reply::tool_call(&call_id, tool, input));
                    next = model.expect(RequestMatch::tool_output(&call_id)).after(&check).check(move |r| {
                        let output = r.tool_output_text(&call_id).context("missing check output")?;
                        let envelope: Value = serde_json::from_str(&output)?;
                        ensure!(envelope["phase"] == phase && output.contains(evidence), "{output}");
                        Ok(())
                    });
                }
                let reply = next.reply(Reply::text(&report).held());
                let parent_report = model
                    .expect(RequestMatch::new("delegated check completion", move |r| r.body.to_string().contains(&report)))
                    .after(&reply)
                    .reply(Reply::text(&delivered));

                let spawned = parent.call_tool("agent.spawn", json!({
                    "name": role, "profile": role, "prompt": assignment
                })).await?;
                spawned.assert_succeeded()?;
                ensure!(spawned.data()?["profile"] == role);
                let child = spawned.data()?["agent_id"].as_i64().context("missing child session ID")?;
                let preview = parent.client().session_prompt_preview(child).await?;
                ensure!(preview.fragments.iter().any(|item| item.id.as_str() == method));
                reply.observed().await?;
                reply.release();
                parent_report.finished().await?;
                parent.wait_text_contains(&delivered).await?;
            }
            ensure!(tokio::fs::read_to_string(case.workspace().join("role-check.txt")).await? == "role-check-evidence\n");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn system_prompt_preview_and_provider_apply_the_same_required_order() -> TestResult {
    fixture()
        .configure_global_with(|config, _, model| {
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"][model.model_id()]
                ["agena_tools"]["mode"] = json!("disabled");
            config["prompts"] = json!({
                "order": ["inherited", "agena.system.markdown"],
                "library": [
                    {"id": "inherited", "body": "INHERITED PROMPT MARKER"},
                    {"id": "selected", "body": "SELECTED PROMPT MARKER"}
                ]
            });
        })
        .run(|case| async move {
            let user = case.user().new_session().await?;
            let client = user.client();
            let builtins = client.builtin_prompts().await?;
            ensure!(builtins.len() == 3 && builtins.iter().all(|item| !item.reusable));
            let identity = builtins
                .iter()
                .find(|item| item.id.as_str() == "agena.system.identity")
                .context("missing shared identity instructions")?;
            ensure!(!identity.body.contains("# User-facing session"));
            ensure!(!identity.body.contains("# Delegated session"));
            let protocol = builtins.iter()
                .find(|item| item.id.as_str() == "agena.system.tools")
                .context("missing tool protocol")?;
            ensure!(serde_json::to_value(protocol.when)? == json!("always"));
            let catalog = client.builtin_prompt_catalog().await?;
            let reusable: Vec<_> = catalog
                .iter()
                .filter(|item| item.reusable)
                .map(|item| item.id.as_str())
                .collect();
            ensure!(reusable == [
                "agena.profile.investigation",
                "agena.profile.implementation",
                "agena.profile.read-only",
                "agena.profile.planning",
                "agena.profile.verification",
            ]);
            for invalid in [
                json!({"disabled": ["agena.system.identity"]}),
                json!({"inline": [{"id": "agena.system.identity", "body": "Override"}]}),
                json!({"order": ["agena.system.unknown"]}),
            ] {
                ensure!(
                    client
                        .set_session_prompts(user.id(), Some(serde_json::from_value(invalid)?))
                        .await
                        .is_err()
                );
            }
            client
                .set_session_prompts(
                    user.id(),
                    Some(serde_json::from_value(json!({
                        "mode": "replace",
                        "order": ["agena.system.markdown", "selected", "agena.system.rich-output", "agena.system.identity"]
                    }))?),
                )
                .await?;
            let preview = client.session_prompt_preview(user.id()).await?;
            ensure!(!preview.text.contains("INHERITED PROMPT MARKER"));
            ensure!(preview.text.contains(&protocol.body));
            ensure!(!preview.text.contains("# Available capabilities"));
            let relationship = preview.fragments.iter()
                .find(|part| part.id.as_str() == "agena.runtime.session-relation")
                .context("missing runtime session relationship")?;
            ensure!(relationship.scope.as_str() == "runtime");
            ensure!(relationship.text.starts_with("# User-facing session"));
            ensure!(!preview.text.contains("# Delegated session"));
            for item in &builtins {
                ensure!(
                    preview
                        .fragments
                        .iter()
                        .any(|fragment| fragment.id == item.id),
                    "missing built-in {}",
                    item.id
                );
            }
            let position = |id| {
                preview
                    .fragments
                    .iter()
                    .filter(|fragment| !fragment.id.as_str().starts_with("agena.runtime."))
                    .position(|fragment| fragment.id.as_str() == id)
            };
            let delivery = position("agena.system.communication").context("missing delivery instructions")?;
            ensure!(position("selected").map(|index| index + 1) == Some(delivery));
            ensure!(position("agena.system.identity") == Some(delivery + 1));

            case.model()
                .expect(RequestMatch::user_text("Check prompt order"))
                .check(move |request| {
                    ensure!(request.body.get("tools").and_then(Value::as_array).is_none_or(Vec::is_empty));
                    ensure!(
                        instructions(request)?.starts_with(&preview.text),
                        "provider prompt diverged from preview"
                    );
                    Ok(())
                })
                .reply(Reply::text("Prompt order checked."));
            user.send("Check prompt order")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_system_prompt_stays_byte_identical_across_turns_and_carries_no_tool_contracts()
-> TestResult {
    fixture()
        .run(|case| async move {
            let model = case.model();
            let user = case.user().new_session().await?;
            let mut seen = Vec::new();
            for marker in ["prompt-stability-first", "prompt-stability-second"] {
                model
                    .expect(RequestMatch::user_text(marker))
                    .check(|request| {
                        let prompt = instructions(request)?;
                        // Contracts arrive through tools_help, and plugin-private
                        // data through the tool that owns it; neither belongs in a
                        // cached system prefix.
                        for absent in [
                            "\"$defs\"",
                            "input schema",
                            "memory_instance_id",
                            "memory_checkpoint_entry_id",
                            "utc_now",
                            "Agent roster",
                            "Current assistant presentation preferences",
                        ] {
                            ensure!(!prompt.contains(absent), "prompt carries {absent}");
                        }
                        ensure!(
                            prompt.contains("## Memory settings and reminders"),
                            "reminder guidance missing from the prompt"
                        );
                        Ok(())
                    })
                    .reply(Reply::text(format!("reply to {marker}")));
                user.send(marker)
                    .await?
                    .wait_settled()
                    .await?
                    .assert_succeeded()?;
            }
            for request in model.requests() {
                if request.body.get("instructions").is_some() {
                    seen.push(instructions(&request)?);
                }
            }
            ensure!(seen.len() >= 2, "captured requests: {}", seen.len());
            ensure!(
                seen.iter().all(|prompt| *prompt == seen[0]),
                "the system prompt changed between turns, invalidating the provider prefix cache"
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn project_conversations_use_system_prompts_and_only_save_policy() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(120))
        .seed_file("alternate-workspace/readme.txt", b"alternate".to_vec())
        .configure_global(|config| {
            config["prompts"] = json!({
                "order": ["presentation"],
                "inline": [{"id": "presentation", "body": "Use the name ConfiguredAssistant."}]
            });
        })
        .run(|case| async move {
            let first = case.user().new_session().await?;
            let http = case.http().await;
            let path = format!("/api/v1/sessions/{}/assistant", first.id());
            for (field, value) in [
                ("name", "Legacy assistant"),
                ("address_user", "Legacy user"),
                ("tone", "warm"),
                ("language", "English"),
                ("instructions", "RETIRED-PRESENTATION-INSTRUCTIONS"),
            ] {
                let mut patch = json!({});
                patch[field] = json!(value);
                let response = http
                    .send_json(
                        reqwest::Method::PUT,
                        &path,
                        &json!({"request_key": field, "expected_revision": 0, "patch": patch}),
                    )
                    .await?;
                ensure!(
                    response.status == reqwest::StatusCode::UNPROCESSABLE_ENTITY,
                    "retired field {field} was accepted"
                );
            }
            let model = case.model();
            ask(
                &first,
                &model,
                "Save a new style for this project",
                "reject-style",
                "companion.manage",
                json!({
                    "action": "configure",
                    "user_quote": "Save a new style for this project",
                    "expected_revision": 0,
                    "patch": {"instructions": "RETIRED-PRESENTATION-INSTRUCTIONS"},
                }),
                "failed",
            )
            .await?;
            ask(
                &first,
                &model,
                "Set the reminder timezone to Europe/Paris",
                "save-timezone",
                "companion.manage",
                json!({
                    "action": "configure",
                    "user_quote": "Set the reminder timezone to Europe/Paris",
                    "expected_revision": 0,
                    "patch": {"timezone": "Europe/Paris"},
                }),
                "saved",
            )
            .await?;
            let next = case.user().new_session().await?;
            model
                .expect(RequestMatch::user_text("Check the ordinary system prompt"))
                .check(|r| {
                    let prompt = instructions(r)?;
                    ensure!(prompt.contains("Use the name ConfiguredAssistant."));
                    ensure!(!prompt.contains("RETIRED-PRESENTATION-INSTRUCTIONS"));
                    ensure!(!prompt.contains("Europe/Paris"));
                    Ok(())
                })
                .reply(Reply::text("System prompt checked."));
            next.send("Check the ordinary system prompt")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let other = case
                .user()
                .new_session_in_with(
                    case.paths().root.join("alternate-workspace"),
                    SessionOptions::default(),
                )
                .await?;
            let other_settings = other.client().assistant_snapshot(other.id()).await?;
            ensure!(other_settings.profile.preferences.timezone == "UTC");
            case.crash().await?;
            case.restart().await?;
            let restored = case.user().attach_session(next.id()).await?;
            let snapshot = restored.client().assistant_snapshot(restored.id()).await?;
            ensure!(snapshot.profile.preferences.timezone == "Europe/Paris");
            ensure!(snapshot.profile.revision == 1);
            let json: Value = case
                .http()
                .await
                .get(&path)
                .await?
                .require_success()?
                .json()?;
            for field in ["name", "address_user", "tone", "language", "instructions"] {
                ensure!(json["profile"]["preferences"].get(field).is_none());
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn natural_memory_recall_correction_mute_and_forget_use_real_scoped_storage() -> TestResult {
    fixture().case_timeout(Duration::from_secs(180)).run(|case|async move{
        let model=case.model();let source=case.user().new_session().await?;
        ask(&source,&model,"记住我的报告偏好：先给结论","remember-pref","memory.write",json!({"name":"report-style","content":"user-memory-original-7e81: reports begin with the conclusion.","kind":"preference","description":"报告偏好"}),"user-memory-original-7e81").await?;
        let fresh=case.user().new_session().await?;
        model.expect(RequestMatch::user_text("你好-memory-recall")).check(|r|{ensure!(r.body.to_string().contains("user-memory-original-7e81"),"saved personal preference was not recalled");Ok(())}).reply(Reply::text("会先给结论。"));
        fresh.send("你好-memory-recall").await?.wait_settled().await?.assert_succeeded()?;
        let snapshot=source.client().assistant_snapshot(source.id()).await?;
        let memory=snapshot.memories.iter().find(|m|m.name=="report-style").context("saved memory")?;
        ask(&source,&model,"纠正一下，报告先列行动项","correct-pref","memory.write",json!({"name":"report-style","content":"user-memory-corrected-68ab: reports start with action items.","kind":"preference","expected_sha256":memory.sha256}),"user-memory-corrected-68ab").await?;
        let corrected=case.user().new_session().await?;
        model.expect(RequestMatch::user_text("新的会话-memory-corrected")).check(|r|{let b=r.body.to_string();ensure!(b.contains("user-memory-corrected-68ab")&&!b.contains("user-memory-original-7e81"));Ok(())}).reply(Reply::text("先列行动项。"));
        corrected.send("新的会话-memory-corrected").await?.wait_settled().await?.assert_succeeded()?;
        ask(&source,&model,"先关闭自动记忆","mute-memory","companion.manage",json!({"action":"configure","user_quote":"先关闭自动记忆","expected_revision":0,"patch":{"memory_enabled":false}}),"saved").await?;
        let muted=case.user().new_session().await?;
        model.expect(RequestMatch::user_text("无记忆会话-memory-muted")).check(|r|{ensure!(!r.body.to_string().contains("user-memory-corrected-68ab"));Ok(())}).reply(Reply::text("不使用自动召回。"));
        muted.send("无记忆会话-memory-muted").await?.wait_settled().await?.assert_succeeded()?;
        let bypass=model.expect(RequestMatch::user_text("保持记忆关闭，再检查是否能保存"))
            .reply(Reply::tool_call("raw-memory-mute-guard","memory.write",json!({"name":"must-not-save","content":"raw bypass rejected","kind":"preference"})));
        model.expect(RequestMatch::tool_output("raw-memory-mute-guard")).after(&bypass)
            .check_tool_output_contains("raw-memory-mute-guard","memory is disabled").reply(Reply::text("记忆仍然关闭，没有保存。"));
        muted.send("保持记忆关闭，再检查是否能保存").await?.wait_settled().await?.assert_succeeded()?;
        let snapshot=source.client().assistant_snapshot(source.id()).await?;
        ensure!(snapshot.memories.len()==1,"muting must not pretend to delete saved memory");
        let memory=&snapshot.memories[0];
        ask(&source,&model,"删除刚才保存的报告偏好","forget-pref","memory.delete",json!({"name":memory.name,"expected_sha256":memory.sha256}),"deleted").await?;
        ensure!(source.client().assistant_snapshot(source.id()).await?.memories.is_empty());
        case.crash().await?;case.restart().await?;
        let restored=case.user().attach_session(source.id()).await?;let state=restored.client().assistant_snapshot(restored.id()).await?;
        ensure!(state.memories.is_empty()&&!state.profile.preferences.memory_enabled);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scheduled_contact_arrives_in_original_chat_and_cannot_authorize_new_actions() -> TestResult
{
    fixture().case_timeout(Duration::from_secs(100)).run(|case|async move{
        let model=case.model();let source=case.user().new_session().await?;
        let start=model.expect(RequestMatch::user_text("两秒后提醒我喝水"))
            .reply(Reply::tool_call("schedule-water","companion.manage",json!({"action":"remind","user_quote":"两秒后提醒我喝水","reminder":{"text":"water-reminder-proof-392a","after_seconds":2}})));
        model.expect(RequestMatch::tool_output("schedule-water")).after(&start).check_tool_output_contains("schedule-water","scheduled").reply(Reply::text("已经安排，提醒会回到这里。"));
        let recursive=model.expect(notice("water-reminder-proof-392a")).reply(Reply::tool_call("reminder-cannot-mutate","companion.manage",json!({"action":"configure","user_quote":"两秒后提醒我喝水","expected_revision":0,"patch":{"proactive_enabled":false}})));
        let done=model.expect(RequestMatch::tool_output("reminder-cannot-mutate")).after(&recursive).check_tool_output_contains("reminder-cannot-mutate","current direct user").reply(Reply::text("到你安排的时间了，记得喝水。water-reminder-delivered"));
        source.send("两秒后提醒我喝水").await?.wait_settled().await?.assert_succeeded()?;done.finished().await?;
        source.wait_text_contains("water-reminder-delivered").await?;
        let snapshot=source.client().assistant_snapshot(source.id()).await?;
        ensure!(snapshot.profile.revision == 0 && snapshot.profile.preferences.proactive_enabled);
        ensure!(snapshot.reminders.len()==1&&snapshot.reminders[0].delivered==1);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reminder_survives_restart_and_cancellation_works_from_natural_chat() -> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).run(|case|async move{
        let model=case.model();let source=case.user().new_session().await?;
        ask(&source,&model,"三秒后问我是否完成检查","restart-reminder","companion.manage",json!({"action":"remind","user_quote":"三秒后问我是否完成检查","reminder":{"text":"restart-check-in-e825","kind":"check_in","after_seconds":3}}),"scheduled").await?;
        source.client().set_message_queue_paused(source.id(),true).await?;
        let original=source.client().assistant_snapshot(source.id()).await?.reminders[0].id;
        case.crash().await?;case.restart().await?;
        let restored=case.user().attach_session(source.id()).await?;
        let done=model.expect(notice("restart-check-in-e825")).reply(Reply::text("你之前安排了这次跟进：检查完成了吗？restart-check-in-delivered"));
        restored.client().set_message_queue_paused(restored.id(),false).await?;done.finished().await?;
        restored.wait_text_contains("restart-check-in-delivered").await?;
        let snapshot=restored.client().assistant_snapshot(restored.id()).await?;
        ensure!(snapshot.reminders[0].id==original&&snapshot.reminders[0].delivered==1);
        ask(&restored,&model,"一分钟后提醒我核对报告","cancel-create","companion.manage",json!({"action":"remind","user_quote":"一分钟后提醒我核对报告","reminder":{"text":"cancel-this-reminder","after_seconds":60}}),"scheduled").await?;
        let snapshot=restored.client().assistant_snapshot(restored.id()).await?;let pending=snapshot.reminders.iter().find(|r|r.spec.text=="cancel-this-reminder").unwrap();
        ask(&restored,&model,"取消刚才核对报告的提醒","cancel-reminder","companion.manage",json!({"action":"cancel_reminder","user_quote":"取消刚才核对报告的提醒","reminder_id":pending.id,"expected_revision":pending.revision}),"cancelled").await?;
        let snapshot=restored.client().assistant_snapshot(restored.id()).await?;ensure!(snapshot.reminders.iter().any(|r|r.id==pending.id&&r.state==agena_api::assistant::ReminderState::Cancelled));
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn assistant_api_preserves_operator_auth_and_stale_memory_edits_are_rejected() -> TestResult {
    use reqwest::Method;
    fixture().password("companion-fixture-password").run(|case|async move{
        let source=case.user().new_session().await?;let http=case.http().await;
        ensure!(http.anonymous().get(&format!("/api/v1/sessions/{}/assistant",source.id())).await?.status.as_u16()==401);
        let name = "偏好".repeat(32);
        let body=json!({"name":name,"body":"original","description":"","kind":"preference","expected_sha256":null});
        let path=format!("/api/v1/sessions/{}/assistant/memory",source.id());
        let saved:Value=http.send_json(Method::POST,&path,&body).await?.require_success()?.json()?;
        let stale=http.send_json(Method::POST,&path,&json!({"name":name,"kind":"preference","body":"must-not-overwrite","expected_sha256":"wrong-revision"})).await?;
        ensure!(stale.status.as_u16()==409);
        let memory=source.client().assistant_snapshot(source.id()).await?.memories.pop().context("memory")?;
        ensure!(memory.body.trim()=="original"&&memory.sha256==saved["sha256"]);
        let updated: Value = http.send_json(Method::POST, &path, &json!({
            "name": name, "kind": "preference", "body": "updated", "expected_sha256": memory.sha256
        })).await?.require_success()?.json()?;
        ensure!(updated["name"] == name && updated["body"].as_str().is_some_and(|body| body.trim() == "updated"));
        http.send_json(Method::POST, &path, &json!({
            "name": name, "expected_sha256": updated["sha256"]
        })).await?.require_success()?;
        ensure!(source.client().assistant_snapshot(source.id()).await?.memories.is_empty());
        Ok(())
    }).await
}
