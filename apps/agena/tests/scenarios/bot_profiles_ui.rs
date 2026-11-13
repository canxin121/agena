//! Integrated real Vue/CDP and native terminal/private-tmux acceptance.
//! Both clients use the same isolated server, registry and scripted provider.
use std::{path::PathBuf, time::Duration};

use agena_testkit::{ModelOptions, Reply, RequestMatch, TestResult};
use anyhow::{Context, ensure};
use serde_json::json;

use super::{
    bot_profiles::{create, profile},
    fixture,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires local Chrome, Node, Web dependencies and tmux; real multi-Bot acceptance"]
async fn bot_profiles_ui_web_and_tui_select_edit_and_continue_two_named_bots() -> TestResult {
    fixture().without_password().case_timeout(Duration::from_secs(660))
        .model_options(ModelOptions {models:vec!["fake-model".into(),"multi-bot-model".into()],default_timeout:Duration::from_secs(30),..Default::default()})
        .configure_global(|config| {
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["fake-model"]["input"]=json!(["text", "image"]);
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["multi-bot-model"]=json!({"context_window_tokens":131072,"max_output_tokens":8192,"native_compaction":false,"agena_tools":{"mode":"provider_protocol"}});
        })
        .run(|case|async move {
            let a=create(&case,"ui-profile-a","Aurora acceptance",Some(json!({"instructions":"A UI persona"}))).await?;
            let b=create(&case,"ui-profile-b","Borealis acceptance",Some(json!({"instructions":"B UI persona"}))).await?;
            let project=case.user().new_session_with(agena_testkit::SessionOptions { title:"Project sidebar sentinel 732".into(), ..Default::default() }).await?;
            let model=case.model();
            let wa=model.expect(RequestMatch::user_text("multi-web-a-message").model("fake-model"))
                .reply(Reply::text("multi-web-a-reply: Aurora's independent conversation."))
                .with_timeout(Duration::from_secs(480));
            let wb=model.expect(RequestMatch::user_text("multi-web-b-message").model("multi-bot-model"))
                .check(|r|{ensure!(!r.body.to_string().contains("multi-web-a-reply"),"Web B inherited A's history");Ok(())})
                .reply(Reply::text("multi-web-b-reply: Borealis uses its selected model."))
                .with_timeout(Duration::from_secs(480));
            let ta=model.expect(RequestMatch::user_text("multi-tui-a-message").model("fake-model"))
                .check(|r|{let b=r.body.to_string();ensure!(b.contains("multi-web-a-reply") && !b.contains("multi-web-b-reply"),"TUI A did not continue A's Web history");Ok(())})
                .reply(Reply::text("multi-tui-a-reply: same Aurora history."))
                .with_timeout(Duration::from_secs(600));
            let tb=model.expect(RequestMatch::user_text("multi-tui-b-message").model("multi-bot-model"))
                .check(|r|{let b=r.body.to_string();ensure!(b.contains("multi-web-b-reply") && !b.contains("multi-web-a-reply") && !b.contains("multi-tui-a-reply"),"TUI B did not continue B's independent history");Ok(())})
                .reply(Reply::text("multi-tui-b-reply: same Borealis history."))
                .with_timeout(Duration::from_secs(600));
            let repository=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            let evidence=std::env::temp_dir().join(format!("agena-multi-bot-ui-evidence-{}",uuid::Uuid::new_v4()));
            let http=case.http().await;
            let mut failures = Vec::new();
            for (runtime,script) in [("node","scripts/agena/tests/probe_multi_bot_web.cjs"),("python3","scripts/agena/tests/probe_multi_bot_tui.py")] {
                let output=tokio::time::timeout(Duration::from_secs(300),tokio::process::Command::new(runtime)
                    .arg(repository.join(script)).env("BOT_BACKEND",http.base_url())
                    .env("BOT_PROFILE_A",serde_json::to_string(&a)?).env("BOT_PROFILE_B",serde_json::to_string(&b)?)
                    .env("BOT_PROJECT_SESSION",project.id().to_string()).env("BOT_PROJECT_PATH",case.workspace()).env("BOT_EVIDENCE",&evidence)
                    .env("AGENA_TEST_BINARY",env!("CARGO_BIN_EXE_agena"))
                    .current_dir(case.workspace()).kill_on_drop(true).output()).await.context("multi-Bot UI probe exceeded its deadline")??;
                println!("Multi-Bot evidence: {}\n{}",evidence.display(),String::from_utf8_lossy(&output.stdout));
                if !output.status.success() { failures.push(format!("{script} failed:\n{}\n{}\nEvidence: {}",String::from_utf8_lossy(&output.stderr),String::from_utf8_lossy(&output.stdout),evidence.display())); }
            }
            ensure!(failures.is_empty(), "{}", failures.join("\n"));
            wa.finished().await?;wb.finished().await?;ta.finished().await?;tb.finished().await?;
            let pa=profile(&case,a.id).await?;let pb=profile(&case,b.id).await?;
            ensure!(pa.name=="Aurora Web edited" && pa.persona.instructions=="Web edited A personality persisted");
            ensure!(pb.name=="Borealis TUI edited" && pb.persona.instructions=="TUI edited B personality persisted");
            // Identity and persona are the Bot's; its conversations are separate rows.
            Ok(())
        }).await
}
