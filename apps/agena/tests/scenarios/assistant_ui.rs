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
                .reply(Reply::tool_call("ui-memory", "memory.write", json!({"name":"ui-report-style","content":"assistant-ui-memory-original: put the conclusion first","description":"Report preferences","kind":"preference"})));
            model.expect(RequestMatch::tool_output("ui-memory")).after(&memory).check_tool_output_contains("ui-memory", "assistant-ui-memory-original")
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
            command.arg(repository().join(if terminal {"scripts/agena/tests/probe_assistant_tui.py"} else {"packages/agena-web/scripts/probe-assistant.cjs"}))
                .env("ASSISTANT_BACKEND", http.base_url()).env("ASSISTANT_SOURCE", source.id().to_string())
                .env("ASSISTANT_EVIDENCE", &evidence).env("ASSISTANT_PROBE", "1")
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
        command.arg(repository().join(if terminal{"scripts/agena/tests/probe_assistant_tui.py"}else{"packages/agena-web/scripts/probe-assistant.cjs"}))
            .env("ASSISTANT_BACKEND",http.base_url()).env("ASSISTANT_SOURCE",source.id().to_string()).env("ASSISTANT_EVIDENCE",&evidence)
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
            title:"Channel access fixture".into(),provider:"example.webhook_chat".into(),mode:IntegrationMode::Channel,config:json!({"conversation":"ui-room"}),resource:"ui-room".into(),thread:None,event_types:vec!["message".into()],actors:vec![],instruction:String::new(),poll_seconds:1,expires_at_ms:chrono::Utc::now().timestamp_millis()+3600000,daily_limit:100,total_limit:200,max_execution_seconds:60,use_source_permissions:true,channel_policy:Some(policy)
        }}).await?;
        let http=case.http().await;
        let evidence=std::env::temp_dir().join(format!("agena-channel-access-{}-evidence-{}",if terminal{"tui"}else{"web"},uuid::Uuid::new_v4()));
        let mut command=tokio::process::Command::new(if terminal{"python3"}else{"node"});
        command.arg(repository().join(if terminal{"scripts/agena/tests/probe_assistant_tui.py"}else{"packages/agena-web/scripts/probe-assistant.cjs"}))
            .env("ASSISTANT_BACKEND",http.base_url()).env("ASSISTANT_SOURCE",source.id().to_string()).env("ASSISTANT_EVIDENCE",&evidence).env("CHANNEL_ACCESS_PROBE","1")
            .env("AGENA_TEST_BINARY",env!("CARGO_BIN_EXE_agena")).current_dir(case.workspace()).kill_on_drop(true);
        let output=tokio::time::timeout(Duration::from_secs(150),command.output()).await.context("channel access UI timed out")??;
        println!("Channel access UI evidence: {}\n{}",evidence.display(),String::from_utf8_lossy(&output.stdout));
        ensure!(output.status.success(),"channel access UI failed: {}",String::from_utf8_lossy(&output.stderr));
        Ok(())
    }).await
}
