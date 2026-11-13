//! Service-level checks for global Bots against disposable real server state.
use std::time::Duration;

use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::{Context, ensure};
use reqwest::Method;
use serde_json::{Value, json};

use super::{
    bot_profiles::{create, ensure_named},
    fixture,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_global_registry_does_not_register_or_inherit_project() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(120))
        .seed_file("workspace/AGENTS.md", b"PROJECT-GUIDANCE-ONLY-785".to_vec())
        .seed_file("workspace/scope.txt", b"project-relative-marker".to_vec())
        .run(|case| async move {
            let http = case.http().await;
            let initial: Value = http.get("/api/v1/workspaces").await?.require_success()?.json()?;
            // TestKit starts the server with an explicit --workspace. Remove
            // that fixture registration before exercising the empty repository.
            for project in initial["items"].as_array().context("workspace list")? {
                let id = project["id"].as_i64().context("fixture project id")?;
                http.execute(http.request(Method::DELETE, &format!("/api/v1/workspaces/{id}"))?).await?.require_success()?;
            }
            let empty: Value = http.get("/api/v1/workspaces").await?.require_success()?.json()?;
            ensure!(empty["items"].as_array().context("empty workspace list")?.is_empty(), "fixture cleanup failed: {empty}");
            let a = create(&case, "global-a", "Global Alpha", Some(json!({"instructions":"GLOBAL-PERSONA-541"}))).await?;
            let b = create(&case, "global-b", "Global Beta", None).await?;
            let _: Value = http.get("/api/v1/bots").await?.require_success()?.json()?;
            let _: Value = http.get(&format!("/api/v1/bots/{}", a.id)).await?.require_success()?.json()?;
            let a_session = ensure_named(&case, a.id).await?.session_id;
            let b_session = ensure_named(&case, b.id).await?.session_id;
            let preview = serde_json::to_value(case.client().await.session_prompt_preview(a_session).await?)?.to_string();
            ensure!(!preview.contains(case.workspace().to_string_lossy().as_ref()) && !preview.contains("PROJECT-GUIDANCE-ONLY-785"), "global prompt preview leaked project context: {preview}");
            let after: Value = http.get("/api/v1/workspaces").await?.require_success()?.json()?;
            ensure!(after["items"].as_array().context("workspace list")?.is_empty(), "Bot registry registered a project: {after}");
            for (bot, bot_session) in [(&a, a_session), (&b, b_session)] {
                let resource: Value = http.get(&format!("/api/v1/sessions/{bot_session}")).await?.require_success()?.json()?;
                ensure!(resource.get("workspace_id") == Some(&Value::Null), "Bot has a project: {resource}");
                ensure!(resource["owner_bot_id"] == json!(bot.id.to_string()), "Bot session lost its owner: {resource}");
                let session = case.user().attach_session(bot_session).await?;
                let state = session.snapshot().await?;
                ensure!(state.session.workspace_id.is_none());
                ensure!(state.execution.effective_workspace_root.is_none(), "Bot inherited a root");
            }
            case.crash().await?;
            case.restart().await?;
            let http = case.http().await;
            let restored: Value = http.get("/api/v1/workspaces").await?.require_success()?.json()?;
            let restored_items = restored["items"].as_array().context("workspace list after crash")?;
            ensure!(restored_items.len() <= 1 && restored_items.iter().all(|p| p["path"] == case.workspace().to_string_lossy().as_ref()), "restart added a synthetic project: {restored}");
            for (bot, bot_session) in [(&a, a_session), (&b, b_session)] {
                ensure!(super::bot_profiles::profile(&case, bot.id).await? == *bot, "Bot identity or persona changed after crash");
                ensure!(case.user().attach_session(bot_session).await?.snapshot().await?.session.workspace_id.is_none());
            }
            let project = case.user().new_session().await?;
            let project_id = project.snapshot().await?.session.workspace_id.context("project association")?;
            let listed: Value = http.get(&format!("/api/v1/sessions?workspace_id={project_id}&limit=50")).await?.require_success()?.json()?;
            let items = listed["items"].as_array().context("project sessions")?;
            ensure!(items.iter().any(|s| s["id"] == project.id()));
            ensure!(!items.iter().any(|s| s["id"] == a_session || s["id"] == b_session), "Bot appeared in project sessions");
            let bot = case.user().attach_session(a_session).await?;
            let project_path = case.workspace().to_string_lossy().into_owned();
            case.model().expect(RequestMatch::user_text("global-provider-scope")).check(move |r| {
                let body = r.body.to_string();
                ensure!(body.contains("GLOBAL-PERSONA-541"), "Bot persona missing from provider request");
                ensure!(!body.contains("PROJECT-GUIDANCE-ONLY-785"), "project AGENTS leaked into global prompt");
                ensure!(!body.contains(&project_path), "project path leaked into global prompt");
                Ok(())
            }).reply(Reply::text("global provider checked"));
            bot.send("global-provider-scope").await?.wait_settled().await?.assert_succeeded()?;
            ensure!(bot.snapshot().await?.session.workspace_id.is_none(), "selecting project moved Bot");
            let relative = bot.call_tool("fs.read", json!({"file_path":"scope.txt"})).await?;
            relative.assert_failed()?;
            let target = case.workspace().join("global-approved-write.txt");
            super::path_permission(&bot, &target, "ask").await?;
            let writing = bot.start_tool("fs.write", json!({"path":target,"content":"global-approved-marker"})).await?;
            let approval = writing.wait_permission().await?;
            ensure!(!target.exists(), "absolute write happened before approval");
            approval.mark_presented().await?;
            approval.allow_once().await?;
            writing.wait_settled().await?.assert_succeeded()?;
            ensure!(tokio::fs::read_to_string(&target).await? == "global-approved-marker");
            let denied_target = case.workspace().join("global-denied-write.txt");
            super::path_permission(&bot, &denied_target, "deny").await?;
            bot.call_tool("fs.write", json!({"path":denied_target,"content":"forbidden"})).await?.assert_failed()?;
            ensure!(!denied_target.exists(), "denied global write had an effect");
            let shell_target = case.workspace().join("global-shell-no-cwd.txt");
            let command = format!("printf bad > '{}'", shell_target.display());
            let missing_cwd = bot.call_tool("shell.exec", json!({"command":command,"reads":[],"writes":[],"network":[]})).await?;
            missing_cwd.assert_failed()?;
            ensure!(!shell_target.exists(), "shell without cwd had an effect");
            bot.client().set_session_permission(bot.id(), serde_json::from_value(json!({
                "tools":{"default":"allow"},
                "path":{"external":{"read":"allow","write":"allow"},"rules":{
                    (case.workspace().to_string_lossy().as_ref()):{"read":"ask","write":"allow"}
                }}
            }))?).await?;
            let bypass = bot.call_tool("fs.read", json!({"file_path":"scope.txt"})).await?;
            bypass.assert_failed()?;
            let shell = bot.start_tool("shell.exec", json!({
                "command":"printf global-shell-approved > global-shell-approved.txt",
                "workdir":case.workspace(),"reads":[],"writes":["global-shell-approved.txt"],"network":[]
            })).await?;
            let shell_approval = shell.wait_permission().await?;
            let shell_output = case.workspace().join("global-shell-approved.txt");
            ensure!(!shell_output.exists(), "shell ran before cwd approval");
            shell_approval.mark_presented().await?;
            shell_approval.allow_once().await?;
            shell.wait_settled().await?.assert_succeeded()?;
            ensure!(tokio::fs::read_to_string(shell_output).await? == "global-shell-approved");
            ensure!(bot.snapshot().await?.execution.effective_workspace_root.is_none(), "shell cwd became durable Bot root");
            let project_read = project.call_tool("fs.read", json!({"file_path":"scope.txt"})).await?;
            project_read.assert_succeeded()?;
            project_read.assert_text_contains("project-relative-marker")?;
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_global_attachments_are_owned_by_exact_session() -> TestResult {
    fixture()
        .run(|case| async move {
            let a = create(&case, "asset-a", "Asset Alpha", None).await?;
            let b = create(&case, "asset-b", "Asset Beta", None).await?;
            let a_session = ensure_named(&case, a.id).await?.session_id;
            let b_session = ensure_named(&case, b.id).await?.session_id;
            let client = case.client().await;
            let uploaded = client
                .upload_session_file(
                    a_session,
                    "evidence.txt",
                    "Z2xvYmFsLWFzc2V0LW93bmVk",
                    Some("text/plain"),
                )
                .await?;
            ensure!(uploaded.workspace_id.is_none() && uploaded.session_id == Some(a_session));
            ensure!(std::path::Path::new(&uploaded.path).is_absolute());
            let (_, bytes) = client
                .download_session_media_file(a_session, &uploaded.path)
                .await?;
            ensure!(bytes == b"global-asset-owned");
            ensure!(
                client
                    .download_session_media_file(b_session, &uploaded.path)
                    .await
                    .is_err(),
                "another Bot read the attachment"
            );
            let project = case.user().new_session().await?;
            ensure!(
                client
                    .download_session_media_file(project.id(), &uploaded.path)
                    .await
                    .is_err(),
                "project session read a Bot attachment"
            );
            let project_id = project
                .snapshot()
                .await?
                .session
                .workspace_id
                .context("project association")?;
            let ordinary = client
                .upload_workspace_file(
                    project_id,
                    "ordinary.txt",
                    "cHJvamVjdC1hc3NldA==",
                    Some("text/plain"),
                )
                .await?;
            ensure!(ordinary.workspace_id == Some(project_id));
            let (_, bytes) = client
                .download_workspace_file(project_id, &ordinary.path)
                .await?;
            ensure!(bytes == b"project-asset");
            case.restart().await?;
            let (_, bytes) = case
                .client()
                .await
                .download_session_media_file(a_session, &uploaded.path)
                .await?;
            ensure!(bytes == b"global-asset-owned");
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_global_native_subagent_inherits_scope_permissions_and_reports() -> TestResult {
    fixture().case_timeout(Duration::from_secs(180)).run(|case| async move {
        let profile = create(&case, "global-child", "Global parent", None).await?;
        let bot_session = ensure_named(&case, profile.id).await?.session_id;
        let bot = case.user().attach_session(bot_session).await?;
        let forbidden = case.workspace().join("subagent-denied.txt");
        super::path_permission(&bot, &forbidden, "deny").await?;
        let model = case.model();
        let spawn = model.expect(RequestMatch::user_text("global-spawn-request"))
            .reply(Reply::tool_call("global-spawn", "agent.spawn", json!({"name":"global_child","prompt":"global-child-prompt"})));
        model.expect(RequestMatch::tool_output("global-spawn")).after(&spawn)
            .check_tool_output_contains("global-spawn", "succeeded").reply(Reply::text("waiting for global child"));
        let child_start = model.expect(RequestMatch::user_text("global-child-prompt"))
            .reply(Reply::tool_call("global-child-denied", "fs.write", json!({"path":forbidden,"content":"forbidden"})));
        let child = model.expect(RequestMatch::tool_output("global-child-denied")).after(&child_start)
            .check_tool_output_contains("global-child-denied", "failed").reply(Reply::text("global child evidence").held());
        let report = model.expect(RequestMatch::new("global child result", |r| r.body.to_string().contains("global child evidence")))
            .reply(Reply::text("global parent received child evidence")).with_timeout(Duration::from_secs(80));
        bot.send("global-spawn-request").await?.wait_settled().await?.assert_succeeded()?;
        child.observed().await?;
        let output = tokio::process::Command::new("python3").arg("-c").arg(
            "import pathlib,sqlite3,sys; p=pathlib.Path(sys.argv[1]).resolve(); c=sqlite3.connect(p.as_uri()+'?mode=ro',uri=True); print(c.execute('SELECT count(*) FROM agena_sessions WHERE parent_session_id=? AND relation_kind=? AND workspace_id IS NULL',(int(sys.argv[2]),'subagent' )).fetchone()[0])"
        ).arg(case.paths().data_dir.join("agena.db")).arg(bot.id().to_string()).output().await?;
        ensure!(output.status.success(), "query child scope: {}", String::from_utf8_lossy(&output.stderr));
        ensure!(String::from_utf8_lossy(&output.stdout).trim().parse::<usize>()? >= 1, "no global child associated with Bot");
        ensure!(!forbidden.exists());
        child.release();
        report.finished().await?;
        bot.wait_text_contains("global parent received child evidence").await?;
        Ok(())
    }).await
}
