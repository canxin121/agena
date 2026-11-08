//! Opt-in real UI acceptance against an isolated product process and scripted
//! local provider. No production data or service is used.
use std::{path::PathBuf, time::Duration};

use agena_testkit::{Reply, RequestMatch, TestResult};
use anyhow::{Context, ensure};

use super::fixture;

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires local Chrome and installed Web dependencies; runs real Vue + HTTP"]
async fn web_task_board_real_browser_workflow() -> TestResult {
    fixture()
        .without_password()
        .case_timeout(Duration::from_secs(300))
        .run(|case| async move {
            let model = case.model();
            let first = model
                .expect(RequestMatch::user_text("browser-task-marker"))
                .reply(Reply::text(
                    "browser-evidence-one: verified output from the real worker",
                ));
            let second = model
                .expect(RequestMatch::user_text("browser-followup-marker"))
                .after(&first)
                .reply(Reply::text("browser-evidence-two: feedback verified"));
            let source = case.user().new_session().await?;
            let http = case.http().await;
            let evidence = std::env::temp_dir()
                .join(format!("agena-task-web-evidence-{}", uuid::Uuid::new_v4()));
            let output = tokio::time::timeout(
                Duration::from_secs(240),
                tokio::process::Command::new(
                    std::env::var("TASKS_JS_RUNTIME").unwrap_or_else(|_| "node".into()),
                )
                .arg(repository().join("packages/agena-web/scripts/probe-tasks.cjs"))
                .env("TASKS_BACKEND", http.base_url())
                .env("TASKS_SOURCE", source.id().to_string())
                .env("TASKS_EVIDENCE", &evidence)
                .kill_on_drop(true)
                .output(),
            )
            .await
            .context("browser probe timed out")??;
            println!(
                "Browser evidence: {}\n{}",
                evidence.display(),
                String::from_utf8_lossy(&output.stdout)
            );
            ensure!(
                output.status.success(),
                "browser probe failed: {}\n{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            first.finished().await?;
            second.finished().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires tmux; drives the real TUI in a private terminal"]
async fn terminal_task_board_real_keyboard_workflow() -> TestResult {
    fixture()
        .without_password()
        .case_timeout(Duration::from_secs(180))
        .run(|case| async move {
            let model = case.model();
            let first = model
                .expect(RequestMatch::user_text("tui-task-marker"))
                .reply(Reply::text("tui-evidence-one: verification complete"));
            let second = model
                .expect(RequestMatch::user_text("tui-followup-marker"))
                .after(&first)
                .reply(Reply::text("tui-evidence-two: feedback verified"));
            let source = case.user().new_session().await?;
            let http = case.http().await;
            let evidence = std::env::temp_dir()
                .join(format!("agena-task-tui-evidence-{}", uuid::Uuid::new_v4()));
            let output = tokio::time::timeout(
                Duration::from_secs(150),
                tokio::process::Command::new("python3")
                    .arg(repository().join("scripts/agena/tests/probe_superagent_tui.py"))
                    .env("TASKS_BACKEND", http.base_url())
                    .env("TASKS_SOURCE", source.id().to_string())
                    .env("TASKS_EVIDENCE", &evidence)
                    .env("AGENA_TEST_BINARY", env!("CARGO_BIN_EXE_agena"))
                    .current_dir(case.workspace())
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            .context("terminal probe timed out")??;
            println!(
                "TUI evidence: {}\n{}",
                evidence.display(),
                String::from_utf8_lossy(&output.stdout)
            );
            ensure!(
                output.status.success(),
                "terminal probe failed: {}\n{}",
                String::from_utf8_lossy(&output.stderr),
                String::from_utf8_lossy(&output.stdout)
            );
            first.finished().await?;
            second.finished().await?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome; exercises natural chat with no Task UI operations"]
async fn web_natural_conversation_workflow_without_task_controls() -> TestResult {
    natural_conversation_ui(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires tmux; exercises natural chat with no Task workbench"]
async fn terminal_natural_conversation_workflow_without_task_controls() -> TestResult {
    natural_conversation_ui(true).await
}

async fn natural_conversation_ui(terminal: bool) -> TestResult {
    fixture().without_password().case_timeout(Duration::from_secs(300))
        .model_options(agena_testkit::ModelOptions { default_timeout: Duration::from_secs(120), ..Default::default() })
        .run(move |case| async move {
            use serde_json::{Value,json};
            let model=case.model();
            let start=model.expect(RequestMatch::user_text("请后台检查测试并把结果告诉我"))
                .reply(Reply::tool_call("natural-ui-start","work.manage",json!({"action":"start","instruction":"natural-ui-worker-first: check tests and provide evidence","user_quote":"请后台检查测试并把结果告诉我"})));
            let acknowledged=model.expect(RequestMatch::tool_output("natural-ui-start")).after(&start)
                .reply(Reply::text("开始检查，结果会直接发回这里。"));
            let worker=model.expect(RequestMatch::user_text("natural-ui-worker-first"))
                .reply(Reply::text("natural-ui-evidence-first: checked the requested tests").held());
            let first_report=model.expect(RequestMatch::new("first automatic report",|request| {
                let text=request.body.to_string(); text.contains("Background work update") && text.contains("natural-ui-evidence-first")
            })).after(&worker).reply(Reply::text("检查已完成，结果已验证。natural-ui-delivery-one"));
            let source=case.user().new_session().await?;
            let source_id=source.id();
            let http=case.http().await;
            let driver_http=http.clone();
            let driver=async move {
                acknowledged.finished().await?;
                worker.observed().await?;
                let rows: Vec<Value>=driver_http.get(&format!("/api/v1/tasks?source_session_id={source_id}")).await?.require_success()?.json()?;
                let task=rows.first().context("natural UI task created by ordinary conversation")?;
                let id=task["id"].as_str().context("natural UI task identity")?;
                let update=model.expect(RequestMatch::user_text("再补一下取消请求的测试"))
                    .reply(Reply::tool_call("natural-ui-update","work.manage",json!({"action":"update","task_id":id,"instruction":"natural-ui-worker-followup: cover cancellation","user_quote":"再补一下取消请求的测试"})));
                let ack2=model.expect(RequestMatch::tool_output("natural-ui-update")).after(&update)
                    .reply(Reply::text("收到，原来的执行会话继续补测。"));
                let next=model.expect(RequestMatch::user_text("natural-ui-worker-followup"))
                    .reply(Reply::text("natural-ui-evidence-followup: cancellation tests passed").held());
                let final_report=model.expect(RequestMatch::new("follow-up automatic report",|request| {
                    let text=request.body.to_string();text.contains("Background work update")&&text.contains("natural-ui-evidence-followup")
                })).after(&next).reply(Reply::text("补充测试已经通过，无需另外验收。natural-ui-delivery-two"));
                worker.release();
                first_report.finished().await?;
                ack2.finished().await?;
                next.observed().await?;
                next.release();
                final_report.finished().await?;
                Ok::<(),anyhow::Error>(())
            };
            let evidence=std::env::temp_dir().join(format!("agena-natural-{}-evidence-{}",if terminal {"tui"} else {"web"},uuid::Uuid::new_v4()));
            let mut command=tokio::process::Command::new(if terminal { "python3" } else { "node" });
            command.arg(repository().join(if terminal { "scripts/agena/tests/probe_superagent_tui.py" } else { "packages/agena-web/scripts/probe-tasks.cjs" }))
                .env("TASKS_BACKEND",http.base_url()).env("TASKS_SOURCE",source_id.to_string())
                .env("TASKS_EVIDENCE",&evidence).env("CONVERSATION_WORK_PROBE","1")
                .env("AGENA_TEST_BINARY",env!("CARGO_BIN_EXE_agena"))
                .current_dir(case.workspace()).kill_on_drop(true);
            let probe=async { Ok::<_,anyhow::Error>(tokio::time::timeout(Duration::from_secs(240),command.output()).await.context("natural conversation UI probe timed out")??) };
            let (output,model_result)=tokio::join!(probe,driver);
            let output=output?;
            println!("Natural conversation UI evidence: {}\n{}",evidence.display(),String::from_utf8_lossy(&output.stdout));
            ensure!(output.status.success(),"natural conversation UI failed: {}\n{}",String::from_utf8_lossy(&output.stderr),String::from_utf8_lossy(&output.stdout));
            model_result?;
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires local Chrome; natural assistant and real management UI"]
async fn web_assistant_personality_memory_and_proactive_contact() -> TestResult {
    assistant_ui(false).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires tmux; natural assistant and native settings workbench"]
async fn terminal_assistant_personality_memory_and_proactive_contact() -> TestResult {
    assistant_ui(true).await
}

async fn assistant_ui(terminal: bool) -> TestResult {
    fixture().without_password().case_timeout(Duration::from_secs(300))
        .model_options(agena_testkit::ModelOptions { default_timeout: Duration::from_secs(120), ..Default::default() })
        .run(move |case| async move {
            use serde_json::json;
            let model = case.model();
            let persona = model.expect(RequestMatch::user_text("以后叫你小鱼，叫我阿林"))
                .reply(Reply::tool_call("ui-persona", "companion.manage", json!({"action":"configure","user_quote":"以后叫你小鱼，叫我阿林","expected_revision":0,"patch":{"name":"小鱼","address_user":"阿林","tone":"warm"}})));
            model.expect(RequestMatch::tool_output("ui-persona")).after(&persona).check_tool_output_contains("ui-persona", "saved")
                .reply(Reply::text("阿林，已经保存这个称呼。assistant-ui-persona-saved"));
            let memory = model.expect(RequestMatch::user_text("记住我希望报告先给结论"))
                .reply(Reply::tool_call("ui-memory", "companion.manage", json!({"action":"remember","user_quote":"记住我希望报告先给结论","name":"ui-report-style","content":"assistant-ui-memory-original: put the conclusion first","description":"Report preferences","memory_type":"user"})));
            model.expect(RequestMatch::tool_output("ui-memory")).after(&memory).check_tool_output_contains("ui-memory", "Saved memory")
                .reply(Reply::text("已经保存报告偏好。assistant-ui-memory-saved"));
            let reminder = model.expect(RequestMatch::user_text("两秒后提醒我核对结果"))
                .reply(Reply::tool_call("ui-reminder", "companion.manage", json!({"action":"remind","user_quote":"两秒后提醒我核对结果","reminder":{"text":"assistant-ui-due-contact","after_seconds":2}})));
            let ack = model.expect(RequestMatch::tool_output("ui-reminder")).after(&reminder).check_tool_output_contains("ui-reminder", "scheduled")
                .reply(Reply::text("已经安排提醒。assistant-ui-reminder-scheduled"));
            model.expect(RequestMatch::new("scheduled assistant UI contact", |request| {
                let text = request.body.to_string(); text.contains("Scheduled assistant contact") && text.contains("assistant-ui-due-contact")
            })).after(&ack).reply(Reply::text("阿林，到你安排的时间了，请核对结果。assistant-ui-contact-delivered"));
            let later = model.expect(RequestMatch::user_text("一小时后再提醒我一次"))
                .reply(Reply::tool_call("ui-later", "companion.manage", json!({"action":"remind","user_quote":"一小时后再提醒我一次","reminder":{"text":"assistant-ui-future-contact","after_seconds":3600}})));
            model.expect(RequestMatch::tool_output("ui-later")).after(&later).check_tool_output_contains("ui-later", "scheduled")
                .reply(Reply::text("一小时后的提醒已安排。assistant-ui-later-scheduled"));
            let source = case.user().new_session().await?;
            let http = case.http().await;
            let evidence = std::env::temp_dir().join(format!("agena-assistant-{}-evidence-{}", if terminal {"tui"} else {"web"}, uuid::Uuid::new_v4()));
            let mut command = tokio::process::Command::new(if terminal {"python3"} else {"node"});
            command.arg(repository().join(if terminal {"scripts/agena/tests/probe_superagent_tui.py"} else {"packages/agena-web/scripts/probe-tasks.cjs"}))
                .env("TASKS_BACKEND", http.base_url()).env("TASKS_SOURCE", source.id().to_string())
                .env("TASKS_EVIDENCE", &evidence).env("ASSISTANT_PROBE", "1")
                .env("AGENA_TEST_BINARY", env!("CARGO_BIN_EXE_agena"))
                .current_dir(case.workspace()).kill_on_drop(true);
            let output = tokio::time::timeout(Duration::from_secs(240), command.output()).await.context("assistant UI probe timed out")??;
            println!("Assistant UI evidence: {}\n{}", evidence.display(), String::from_utf8_lossy(&output.stdout));
            ensure!(output.status.success(), "assistant UI failed: {}\n{}", String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome; validates external integration authorization UI"]
async fn web_plugin_integrations_authorization_workflow() -> TestResult {
    integration_ui(false).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires tmux; validates native integration authorization UI"]
async fn terminal_plugin_integrations_authorization_workflow() -> TestResult {
    integration_ui(true).await
}

async fn integration_ui(terminal: bool) -> TestResult {
    fixture().without_password().case_timeout(Duration::from_secs(180)).before_start(|case|async move{
        use axum::{Json,Router,routing::get};use serde_json::{Value,json};
        let peer=case.serve_http(Router::new().route("/repos/owner/repository/issues",get(||async{Json(json!([]))}))).await?;
        let mut config:Value=serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
        config["plugins"]["list"]["example.github_issues"]=json!({"package":{"kind":"stdio","command":"python3","args":[repository().join("examples/integration-plugins/github_issues.py")],"env":{}},"settings":{"api_base":peer.url(""),"allow_loopback_http":true,"token_env":"AGENA_UNUSED_FIXTURE_CREDENTIAL"}});
        case.write_config(agena_testkit::SettingsLayer::Global,config).await
    }).run(move|case|async move{
        let source=case.user().new_session().await?;let http=case.http().await;
        let evidence=std::env::temp_dir().join(format!("agena-integrations-{}-evidence-{}",if terminal{"tui"}else{"web"},uuid::Uuid::new_v4()));
        let mut command=tokio::process::Command::new(if terminal{"python3"}else{"node"});
        command.arg(repository().join(if terminal{"scripts/agena/tests/probe_superagent_tui.py"}else{"packages/agena-web/scripts/probe-tasks.cjs"}))
            .env("TASKS_BACKEND",http.base_url()).env("TASKS_SOURCE",source.id().to_string()).env("TASKS_EVIDENCE",&evidence)
            .env("INTEGRATIONS_PROBE","1").env("AGENA_TEST_BINARY",env!("CARGO_BIN_EXE_agena")).current_dir(case.workspace()).kill_on_drop(true);
        let output=tokio::time::timeout(Duration::from_secs(150),command.output()).await.context("integration UI timed out")??;
        println!("Integration UI evidence: {}\n{}",evidence.display(),String::from_utf8_lossy(&output.stdout));
        ensure!(output.status.success(),"integration UI failed: {}",String::from_utf8_lossy(&output.stderr));
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Chrome; actual channel role editor and stale-policy guard"]
async fn web_channel_access_policy_editor_and_conflict() -> TestResult {
    channel_access_ui(false).await
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires tmux; actual channel role policy editor"]
async fn terminal_channel_access_policy_editor_and_conflict() -> TestResult {
    channel_access_ui(true).await
}

async fn channel_access_ui(terminal: bool) -> TestResult {
    fixture().without_password().case_timeout(Duration::from_secs(180)).before_start(|case|async move{
        use serde_json::{Value,json};
        let mut config:Value=serde_json::from_slice(&tokio::fs::read(&case.paths().global_config).await?)?;
        config["plugins"]["list"]["example.webhook_chat"]=json!({"package":{"kind":"stdio","command":"python3","args":[repository().join("examples/integration-plugins/webhook_chat.py")],"env":{"AGENA_CHAT_BRIDGE_SECRET":"channel-access-ui-local-fixture-secret-32"}},"settings":{"database_path":case.workspace().join("access-ui-bridge.db"),"listen_port":0,"spool_only":true}});
        case.write_config(agena_testkit::SettingsLayer::Global,config).await
    }).run(move|case|async move{
        use agena_api::integrations::*;use serde_json::json;
        let source=case.user().new_session().await?;
        let policy:ChannelAccessPolicy=serde_json::from_value(json!({"identity":{"tenant":"ui-tenant","account":"ui-bot","kind":"group"},"members":[{"actor":"owner-user","role":"owner"}],"per_user_daily_limit":50}))?;
        source.client().create_integration(source.id(),&CreateIntegration{request_key:"access-ui".into(),spec:IntegrationSpec{
            title:"Channel access fixture".into(),provider:"example.webhook_chat".into(),mode:IntegrationMode::Channel,config:json!({"conversation":"ui-room"}),resource:"ui-room".into(),thread:None,event_types:vec!["message".into()],actors:vec![],instruction:String::new(),poll_seconds:1,expires_at_ms:chrono::Utc::now().timestamp_millis()+3600000,daily_limit:100,total_limit:200,max_task_seconds:60,use_source_permissions:true,channel_policy:Some(policy)
        }}).await?;
        let http=case.http().await;
        let evidence=std::env::temp_dir().join(format!("agena-channel-access-{}-evidence-{}",if terminal{"tui"}else{"web"},uuid::Uuid::new_v4()));
        let mut command=tokio::process::Command::new(if terminal{"python3"}else{"node"});
        command.arg(repository().join(if terminal{"scripts/agena/tests/probe_superagent_tui.py"}else{"packages/agena-web/scripts/probe-tasks.cjs"}))
            .env("TASKS_BACKEND",http.base_url()).env("TASKS_SOURCE",source.id().to_string()).env("TASKS_EVIDENCE",&evidence).env("CHANNEL_ACCESS_PROBE","1")
            .env("AGENA_TEST_BINARY",env!("CARGO_BIN_EXE_agena")).current_dir(case.workspace()).kill_on_drop(true);
        let output=tokio::time::timeout(Duration::from_secs(150),command.output()).await.context("channel access UI timed out")??;
        println!("Channel access UI evidence: {}\n{}",evidence.display(),String::from_utf8_lossy(&output.stdout));
        ensure!(output.status.success(),"channel access UI failed: {}",String::from_utf8_lossy(&output.stderr));
        Ok(())
    }).await
}
