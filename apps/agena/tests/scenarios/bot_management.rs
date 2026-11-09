//! Human management parity through the real server, provider, tools and client API.
use super::{bot::operation, fixture};
use agena_api::resource::{ModelRef, RunOptions, UserInputReplyKind};
use agena_domain::session_kernel::ExecutionPhase;
use agena_testkit::{
    ModelOptions, Reply, RequestMatch, SessionOptions, TestCase, TestResult, UserSession,
};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

// Tool envelopes may contain the model JSON followed by a human-readable suffix.
// Keep the same tiny general parser as bot.rs; the actual test engine is testkit.
fn field(value: &Value, name: &str) -> Option<Value> {
    if let Some(value) = value.get(name) {
        return Some(value.clone());
    }
    match value {
        Value::Object(object) => object.values().find_map(|v| field(v, name)),
        Value::Array(items) => items.iter().find_map(|v| field(v, name)),
        Value::String(text) => serde_json::Deserializer::from_str(text)
            .into_iter::<Value>()
            .next()
            .and_then(|parsed| parsed.ok())
            .and_then(|v| field(&v, name)),
        _ => None,
    }
}

fn required(value: &Value, name: &str) -> Result<Value> {
    field(value, name).with_context(|| format!("missing {name}: {value}"))
}

fn succeeded(value: &Value) -> Result<()> {
    ensure!(value["phase"] == "succeeded", "{value}");
    Ok(())
}

fn failed(value: &Value) -> Result<()> {
    ensure!(value["phase"] == "failed", "{value}");
    Ok(())
}

async fn resource(case: &TestCase, id: i64) -> Result<Value> {
    case.http()
        .await
        .get(&format!("/api/v1/sessions/{id}"))
        .await?
        .require_success()?
        .json()
}

async fn native_list(case: &TestCase, workspace: i64) -> Result<Vec<Value>> {
    let value: Value = case
        .http()
        .await
        .get(&format!(
            "/api/v1/sessions?workspace_id={workspace}&limit=50"
        ))
        .await?
        .require_success()?
        .json()?;
    required(&value, "items")?
        .as_array()
        .cloned()
        .context("native session list")
}

async fn operator(case: &TestCase) -> Result<UserSession> {
    let id = case.client().await.ensure_bot_session().await?.session_id;
    case.user().attach_session(id).await
}

async fn status(case: &TestCase, bot: &UserSession, target: i64, marker: &str) -> Result<Value> {
    let result = operation(
        case,
        bot,
        marker,
        json!({"action":"status","session_id":target}),
    )
    .await?;
    succeeded(&result)?;
    Ok(result)
}

fn version(value: &Value) -> Result<i64> {
    required(value, "version")?
        .as_i64()
        .context("metadata version")
}

fn control_version(value: &Value) -> Result<i64> {
    required(value, "control_version")?
        .as_i64()
        .context("control version")
}

fn delete_input(target: i64, observed: &Value) -> Result<Value> {
    Ok(json!({"action":"delete","session_id":target,
        "expected_version":version(observed)?,
        "expected_control_version":control_version(observed)?,
        "expected_entry_count":required(observed,"entry_count")?}))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_finds_duplicate_titles_and_explicit_flags_match_native_lists() -> TestResult
{
    fixture().case_timeout(Duration::from_secs(150))
        .seed_file("workspace/mgmt-project-alpha/keep.txt", b"alpha".to_vec())
        .seed_file("workspace/mgmt-project-beta/keep.txt", b"beta".to_vec())
        .run(|case| async move {
            let bot = operator(&case).await?;
            let options = || SessionOptions { title: "Identical release plan".into(), ..Default::default() };
            let alpha = case.user().new_session_in_with(case.workspace().join("mgmt-project-alpha"), options()).await?;
            let beta = case.user().new_session_in_with(case.workspace().join("mgmt-project-beta"), options()).await?;
            let alpha_before = resource(&case, alpha.id()).await?;
            let beta_before = resource(&case, beta.id()).await?;
            let workspace = alpha_before["workspace_id"].as_i64().context("alpha project")?;
            let permissions = alpha.snapshot().await?.execution.selected_permission;

            let projects = operation(&case, &bot, "find-management-projects", json!({"action":"projects","query":"mgmt-project"})).await?;
            succeeded(&projects)?;
            let projects = required(&projects, "projects")?;
            let projects = projects.as_array().context("project candidates")?;
            ensure!(projects.len() == 2, "{projects:?}");
            for expected in [&alpha_before, &beta_before] {
                ensure!(projects.iter().any(|p| p["workspace_id"] == expected["workspace_id"]), "wrong project candidates");
            }
            let exact = operation(&case, &bot, "find-exact-alpha-project", json!({"action":"projects","query":case.workspace().join("mgmt-project-alpha")})).await?;
            succeeded(&exact)?;
            let exact = required(&exact, "projects")?;
            ensure!(exact.as_array().context("exact projects")?.len() == 1 && exact[0]["workspace_id"] == workspace, "{exact}");
            let detail = operation(&case, &bot, "inspect-alpha-project", json!({"action":"project","workspace_id":workspace})).await?;
            succeeded(&detail)?;
            ensure!(detail.to_string().contains("mgmt-project-alpha"));

            let all = operation(&case, &bot, "find-duplicate-session-titles", json!({"action":"sessions","query":"Identical release plan","limit":1})).await?;
            succeeded(&all)?;
            let mut matches = required(&all, "sessions")?.as_array().cloned().context("global matches")?;
            ensure!(matches.len() == 1 && required(&all,"has_more")? == true, "{all}");
            let cursor = required(&all, "next_cursor")?;
            let rest = operation(&case, &bot, "find-duplicate-session-next-page", json!({"action":"sessions","query":"Identical release plan","limit":1,"cursor":cursor})).await?;
            succeeded(&rest)?;
            matches.extend(required(&rest,"sessions")?.as_array().context("second matches")?.clone());
            ensure!(matches.len() == 2 && matches.iter().any(|s| s["session_id"] == alpha.id() && s["workspace_id"] == workspace)
                && matches.iter().any(|s| s["session_id"] == beta.id() && s["workspace_id"] == beta_before["workspace_id"]), "{matches:?}");
            let wrong_scope = operation(&case, &bot, "reject-session-cursor-different-query", json!({"action":"sessions","query":"different title","cursor":cursor})).await?;
            failed(&wrong_scope)?;
            let scoped = operation(&case, &bot, "locate-alpha-same-title", json!({"action":"sessions","workspace_id":workspace,"query":"Identical release plan"})).await?;
            succeeded(&scoped)?;
            let rows = required(&scoped,"sessions")?;
            ensure!(rows.as_array().context("scoped rows")?.len() == 1 && rows[0]["session_id"] == alpha.id(), "{rows}");

            let organized = operation(&case, &bot, "favorite-and-pin-alpha", json!({"action":"organize","session_id":alpha.id(),"favorite":true,"pinned":true,"expected_version":version(&alpha_before)?,"_test_retry":true})).await?;
            succeeded(&organized)?;
            let after = resource(&case, alpha.id()).await?;
            ensure!(after["favorite"] == true && after["pinned"] == true && after["title"] == alpha_before["title"]);
            let native = native_list(&case, workspace).await?;
            ensure!(native.iter().any(|s| s["id"] == alpha.id() && s["favorite"] == true && s["pinned"] == true));
            let filtered = operation(&case, &bot, "find-favorite-pinned-alpha", json!({"action":"sessions","query":"Identical release plan","favorite":true,"pinned":true})).await?;
            succeeded(&filtered)?;
            let rows = required(&filtered,"sessions")?;
            ensure!(rows.as_array().context("flag filter")?.len() == 1 && rows[0]["session_id"] == alpha.id());
            let unpin = operation(&case, &bot, "explicitly-unpin-alpha", json!({"action":"organize","session_id":alpha.id(),"pinned":false,"expected_version":version(&after)?})).await?;
            succeeded(&unpin)?;
            let final_state = resource(&case, alpha.id()).await?;
            ensure!(final_state["favorite"] == true && final_state["pinned"] == false);
            let stale = operation(&case, &bot, "stale-organize-must-preserve-human-flags", json!({"action":"organize","session_id":alpha.id(),"favorite":false,"expected_version":version(&after)?})).await?;
            failed(&stale)?;
            ensure!(resource(&case, alpha.id()).await? == final_state);
            ensure!(resource(&case, beta.id()).await? == beta_before);
            ensure!(serde_json::to_value(alpha.snapshot().await?.execution.selected_permission)? == serde_json::to_value(permissions)?);
            Ok(())
        }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_fork_preserves_history_model_permissions_and_human_continuation()
-> TestResult {
    fixture().case_timeout(Duration::from_secs(150)).run(|case| async move {
        let bot = operator(&case).await?;
        let source = case.user().new_session_with(SessionOptions { title:"Fork source with history".into(), ..Default::default() }).await?;
        let forbidden = case.workspace().join("fork-must-not-write.txt");
        super::path_permission(&source, &forbidden, "deny").await?;
        source.client().update_session_selection(source.id(), RunOptions {
            model:Some(ModelRef::new_with_adapter("testkit","openai_responses","fake-model")),
            ..Default::default()
        }).await?;
        case.model().expect(RequestMatch::user_text("source-history-evidence")).reply(Reply::text("source assistant context preserved"));
        source.send("source-history-evidence").await?.wait_settled().await?.assert_succeeded()?;
        let before = source.snapshot().await?;
        let source_parts = source.parts().await?;
        let forked = operation(&case, &bot, "fork-source-once", json!({"action":"fork","session_id":source.id(),"title":"Review branch","_test_retry":true})).await?;
        succeeded(&forked)?;
        let id = required(&forked,"session_id")?.as_i64().context("fork ID")?;
        ensure!(id != source.id() && id != bot.id());
        let fork = case.user().attach_session(id).await?;
        let fork_resource = resource(&case,id).await?;
        ensure!(fork_resource["workspace_id"] == json!(before.session.workspace_id) && fork_resource["parent_id"] == source.id()
            && fork_resource["relation_kind"] == "fork" && fork_resource["is_subagent"] == false && fork_resource["title"] == "Review branch", "{fork_resource}");
        let children: Vec<_> = native_list(&case,before.session.workspace_id.context("fork project workspace")?).await?.into_iter().filter(|s|s["parent_id"] == source.id() && s["relation_kind"] == "fork").collect();
        ensure!(children.len() == 1 && children[0]["id"] == id, "retry duplicated the fork: {children:?}");
        let inherited = fork.snapshot().await?;
        for key in ["model_provider_id","model_adapter_id","model_id","model_thinking_mode","model_speed_mode","model_verbosity","model_parallel_tool_calls","selected_permission","effective_permission","permission_ceiling"] {
            let original = serde_json::to_value(&before.execution)?;
            let actual = serde_json::to_value(&inherited.execution)?;
            ensure!(original[key] == actual[key], "fork changed {key}: {} -> {}",original[key],actual[key]);
        }
        let parts = fork.parts().await?;
        for original in source_parts.iter().filter(|p| p.kind == "text") {
            ensure!(parts.iter().any(|p| p.part_id == original.part_id && p.content == original.content), "fork omitted inherited entry {}",original.part_id);
        }
        ensure!(fork.assistant_text().await?.contains("source assistant context preserved"));
        let routed = case.model().expect(RequestMatch::new("Bot sends ordinary fork",|r| {
            r.body.to_string().contains("fork-routed-permission-evidence") && r.body.to_string().contains("Operator Bot instruction (Bot session")
        })).check(|r| {ensure!(r.body.to_string().contains("source assistant context preserved"));Ok(())})
            .reply(Reply::tool_call("fork-denied-write","fs.write",json!({"path":"fork-must-not-write.txt","content":"forbidden"})));
        let done = case.model().expect(RequestMatch::tool_output("fork-denied-write")).after(&routed)
            .check_tool_output_contains("fork-denied-write","failed").reply(Reply::text("fork permission stayed denied"))
            .with_timeout(Duration::from_secs(45));
        let sent = operation(&case,&bot,"send-review-branch",json!({"action":"send","session_id":id,"instruction":"fork-routed-permission-evidence"})).await?;
        succeeded(&sent)?;
        done.finished().await?;
        fork.wait_text_contains("fork permission stayed denied").await?;
        ensure!(!tokio::fs::try_exists(forbidden).await?);
        case.model().expect(RequestMatch::user_text("human-continues-review-branch"))
            .check(|r| {let body=r.body.to_string();ensure!(body.contains("source assistant context preserved") && body.contains("fork permission stayed denied"));ensure!(!body.contains("## Named operator Bot"));Ok(())})
            .reply(Reply::text("human continued the same fork"));
        fork.send("human-continues-review-branch").await?.wait_settled().await?.assert_assistant_text("human continued the same fork")?;
        ensure!(!source.assistant_text().await?.contains("human continued the same fork"));
        let evidence = operation(&case,&bot,"read-fork-tool-and-human-evidence",json!({"action":"read","session_id":id,"limit":8})).await?;
        succeeded(&evidence)?;
        ensure!(evidence.to_string().contains("fs.write") && evidence.to_string().contains("failed") && evidence.to_string().contains("human continued the same fork"), "read omitted real tool/human evidence: {evidence}");
        let exported = operation(&case,&bot,"export-native-review-branch",json!({"action":"export","session_id":id})).await?;
        succeeded(&exported)?;
        let url = required(&exported,"export_url")?.as_str().context("native export URL")?.to_owned();
        ensure!(url == format!("/api/v1/sessions/{id}/export"));
        let archive = case.http().await.get(&url).await?.require_success()?;
        let archive = String::from_utf8(archive.body)?;
        let records = archive.lines().map(serde_json::from_str::<Value>).collect::<std::result::Result<Vec<_>,_>>()?;
        ensure!(records.first().context("archive header")?["kind"] == "header" && archive.contains("human-continues-review-branch"));
        for original in source_parts.iter().filter(|p|p.kind == "text") {
            ensure!(records.iter().any(|record|record["kind"] == "entry" && record["data"]["facts"]["entry"]["id"] == original.part_id), "native export omitted inherited entry {}",original.part_id);
        }
        ensure!(records.iter().any(|record|record["kind"] == "content_block"), "native export omitted committed content");
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_surviving_fork_keeps_inherited_text_and_continues_after_source_deletion()
-> TestResult {
    fixture().case_timeout(Duration::from_secs(180)).run(|case| async move {
        let bot = operator(&case).await?;
        let source = case.user().new_session_with(SessionOptions {title:"Disposable fork source".into(),..Default::default()}).await?;
        let inherited = format!("surviving-fork-history-start\n{}\nsurviving-fork-history-end", "继承🌍 human-visible branch evidence.\n".repeat(180));
        case.model().expect(RequestMatch::user_text("prepare-surviving-fork-history")).reply(Reply::text(inherited.clone()));
        source.send("prepare-surviving-fork-history").await?.wait_settled().await?.assert_assistant_text(&inherited)?;
        let entry = source.parts().await?.into_iter().find(|p|p.kind == "text" && p.role == "assistant").context("inherited assistant entry")?;
        let forked = operation(&case,&bot,"create-surviving-management-fork",json!({"action":"fork","session_id":source.id(),"title":"Surviving ordinary fork"})).await?;
        succeeded(&forked)?;
        let id = required(&forked,"session_id")?.as_i64().context("surviving fork ID")?;
        let fork = case.user().attach_session(id).await?;
        let permission = serde_json::to_value(fork.snapshot().await?.execution.selected_permission)?;
        let source_resource = resource(&case,source.id()).await?;
        let http = case.http().await;
        let deletion = http.request(Method::DELETE,&format!("/api/v1/sessions/{}",source.id()))?
            .header("if-match",format!("\"{}\"",version(&source_resource)?));
        http.execute(deletion).await?.require_success()?;
        ensure!(http.get(&format!("/api/v1/sessions/{}",source.id())).await?.status == 404);
        ensure!(resource(&case,id).await?["parent_id"] == source.id());
        let listed = operation(&case,&bot,"find-fork-after-native-source-delete",json!({"action":"sessions","query":"Surviving ordinary fork"})).await?;
        succeeded(&listed)?;
        let rows = required(&listed,"sessions")?;
        ensure!(rows.as_array().context("surviving fork discovery")?.len() == 1 && rows[0]["session_id"] == id,"global discovery lost a live ordinary fork: {listed}");
        let observed = status(&case,&bot,id,"inspect-fork-after-native-source-delete").await?;
        ensure!(observed.to_string().contains("surviving-fork-history-start"));
        let mut recovered = String::new();
        let mut offset = 0_u64;
        for page in 0..8 {
            let chunk = operation(&case,&bot,&format!("page-inherited-fork-after-source-delete-{page}"),json!({"action":"read_text","session_id":id,"entry_id":entry.part_id,"byte_offset":offset,"max_bytes":4097})).await?;
            succeeded(&chunk)?;
            ensure!(required(&chunk,"origin_session_id")? == source.id() && required(&chunk,"session_id")? == id,"inherited text lost its target or provenance: {chunk}");
            recovered.push_str(required(&chunk,"text")?.as_str().context("inherited text page")?);
            if required(&chunk,"has_more")? == false {break;}
            let navigation = required(&chunk,"next")?;
            ensure!(navigation["session_id"] == id && navigation["entry_id"] == entry.part_id,"inherited paging escaped the visible fork: {navigation}");
            let next = required(&chunk,"next_byte_offset")?.as_u64().context("inherited next offset")?;
            ensure!(next > offset && next == recovered.len() as u64);
            offset = next;
        }
        ensure!(recovered == inherited,"source deletion lost inherited human-visible fork text");
        let routed = case.model().expect(RequestMatch::new("Bot continues surviving ordinary fork",|request| {
            let body=request.body.to_string();body.contains("bot-follows-surviving-fork") && body.contains("Operator Bot instruction (Bot session") && !body.contains("## Named operator Bot")
        })).check(|request| {ensure!(request.body.to_string().contains("surviving-fork-history-end"));Ok(())})
            .reply(Reply::text("surviving fork accepted operator continuation")).with_timeout(Duration::from_secs(45));
        let sent = operation(&case,&bot,"send-surviving-management-fork",json!({"action":"send","session_id":id,"instruction":"bot-follows-surviving-fork"})).await?;
        succeeded(&sent)?;
        routed.finished().await?;
        fork.wait_text_contains("surviving fork accepted operator continuation").await?;
        case.model().expect(RequestMatch::user_text("human-follows-surviving-fork"))
            .check(|request| {let body=request.body.to_string();ensure!(body.contains("surviving-fork-history-end") && body.contains("surviving fork accepted operator continuation"));Ok(())})
            .reply(Reply::text("human continued surviving ordinary fork"));
        fork.send("human-follows-surviving-fork").await?.wait_settled().await?.assert_assistant_text("human continued surviving ordinary fork")?;
        let current = resource(&case,id).await?;
        let organized = operation(&case,&bot,"favorite-surviving-management-fork",json!({"action":"organize","session_id":id,"favorite":true,"expected_version":version(&current)?})).await?;
        succeeded(&organized)?;
        ensure!(resource(&case,id).await?["favorite"] == true && serde_json::to_value(fork.snapshot().await?.execution.selected_permission)? == permission);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_read_status_and_utf8_text_paging_return_actual_evidence() -> TestResult {
    fixture().case_timeout(Duration::from_secs(180)).run(|case| async move {
        let bot = operator(&case).await?;
        let target = case.user().new_session().await?;
        let text = format!("management-long-start\n{}\nmanagement-long-end", "段落🌍: evidence with human context.\n".repeat(750));
        let user_text = format!("management-long-transcript\n{}\nlong-user-instruction-end", "用户上下文🌍 with exact requested evidence.\n".repeat(80));
        case.model().expect(RequestMatch::user_text("management-long-transcript")).reply(Reply::text(text.clone()));
        target.send(user_text.clone()).await?.wait_settled().await?.assert_assistant_text(&text)?;
        let read = operation(&case,&bot,"read-management-transcript",json!({"action":"read","session_id":target.id(),"limit":10})).await?;
        succeeded(&read)?;
        ensure!(read.to_string().contains("management-long-start"),"read has no actual text: {read}");
        ensure!(required(&read,"url")? == format!("/chat?sessionId={}",target.id()));
        let entry = target.parts().await?.into_iter().find(|p|p.kind == "text" && p.role == "assistant").context("native assistant text entry")?;
        let mut offset = 0_u64;
        let mut reconstructed = String::new();
        for page in 0..32 {
            let chunk = operation(&case,&bot,&format!("page-management-text-{page}"),json!({"action":"read_text","session_id":target.id(),"entry_id":entry.part_id,"byte_offset":offset,"max_bytes":4097})).await?;
            succeeded(&chunk)?;
            let bytes = required(&chunk,"text")?.as_str().context("text page")?.to_owned();
            ensure!(bytes.len() <= 4097 && !bytes.contains('\u{fffd}'), "invalid UTF-8 boundary or byte budget");
            reconstructed.push_str(&bytes);
            if required(&chunk,"has_more")? == false { break; }
            let next = required(&chunk,"next_byte_offset")?.as_u64().context("next text offset")?;
            ensure!(next > offset && next == reconstructed.len() as u64, "paging skipped or repeated text: {chunk}");
            offset = next;
        }
        ensure!(reconstructed == text, "text paging did not reconstruct the complete native response");
        let user_entry = target.parts().await?.into_iter().find(|p|p.kind == "text" && p.role == "user").context("native user entry")?;
        let user_read = operation(&case,&bot,"read-long-native-human-instruction",json!({"action":"read_text","session_id":target.id(),"entry_id":user_entry.part_id,"max_bytes":16000})).await?;
        succeeded(&user_read)?;
        ensure!(required(&user_read,"text")? == user_text && required(&user_read,"has_more")? == false, "long user instruction was not retrievable: {user_read}");
        let foreign = case.user().new_session().await?;
        let rejected = operation(&case,&bot,"reject-foreign-text-entry",json!({"action":"read_text","session_id":foreign.id(),"entry_id":entry.part_id,"max_bytes":4097})).await?;
        failed(&rejected)?;
        ensure!(!rejected.to_string().contains("management-long-start"));
        let observed = status(&case,&bot,target.id(),"inspect-completed-management-work").await?;
        let native = target.snapshot().await?;
        ensure!(version(&observed)? == native.session.version && control_version(&observed)? == native.latest_event_seq.context("native control revision")?);
        ensure!(required(&observed,"entry_count")?.as_u64().context("entry count")? >= 2);
        ensure!(observed.to_string().contains("management-long-start"),"status omitted last response evidence: {observed}");
        let ask = case.model().expect(RequestMatch::user_text("management-needs-human-decision"))
            .reply(Reply::tool_call("management-question","interaction.ask",json!({"questions":[{"header":"Review","question":"Which release should continue?","options":[{"label":"Stable","description":"Use stable."},{"label":"Preview","description":"Use preview."}],"multiple":false}]})));
        case.model().expect(RequestMatch::tool_output("management-question")).after(&ask)
            .check_tool_output_contains("management-question","Stable").reply(Reply::text("human decision consumed"));
        let pending = target.send("management-needs-human-decision").await?;
        let card = pending.wait_user_input().await?;
        let blocked = status(&case,&bot,target.id(),"inspect-human-blocked-management-work").await?;
        ensure!(blocked.to_string().contains("Which release should continue?") && required(&blocked,"pending_interaction_count")? == 1, "status hid actionable pending question: {blocked}");
        let interactions = required(&blocked,"pending_interactions")?;
        ensure!(interactions[0]["requires_human"] == true && interactions[0]["detail"]["type"] == "input" && interactions[0]["url"] == format!("/chat?sessionId={}",target.id()));
        ensure!(!target.snapshot().await?.session.state.pending_interactive_requests().is_empty(), "Bot answered a human decision");
        card.answer(BTreeMap::from([("0".into(),vec!["Stable".into()])])).await?;
        pending.wait_settled().await?.assert_assistant_text("human decision consumed")?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_models_configure_real_provider_and_reject_invalid_or_stale_changes()
-> TestResult {
    fixture().case_timeout(Duration::from_secs(150))
        .model_options(ModelOptions { models:vec!["fake-model".into(),"management-model".into()], default_timeout:Duration::from_secs(20), ..Default::default() })
        .configure_global(|config| {
            config["providers"]["testkit"]["adapters"]["openai_responses"]["models"]["management-model"] =
                json!({"context_window_tokens":131072,"max_output_tokens":8192,"native_compaction":false,"agena_tools":{"mode":"provider_protocol"}});
        }).run(|case| async move {
            let bot = operator(&case).await?;
            let target = case.user().new_session().await?;
            super::path_permission(&target,&case.workspace().join("model-policy.txt"),"deny").await?;
            let before = target.snapshot().await?;
            let models = operation(&case,&bot,"list-actual-management-models",json!({"action":"models","session_id":target.id()})).await?;
            succeeded(&models)?;
            let inventory = models.to_string();
            ensure!(inventory.contains("testkit") && inventory.contains("openai_responses") && inventory.contains("fake-model") && inventory.contains("management-model"), "actual fixture provider missing: {models}");
            ensure!(!inventory.contains("agena-testkit") && !inventory.contains("api_key"), "model inventory exposed provider credentials");
            let selected = json!({"provider_id":"testkit","adapter_id":"openai_responses","model_id":"management-model"});
            let changed = operation(&case,&bot,"configure-management-model",json!({"action":"configure","session_id":target.id(),"model":selected,"parallel_tool_calls":false,"expected_version":before.session.version,"_test_retry":true})).await?;
            succeeded(&changed)?;
            let after = target.snapshot().await?;
            ensure!(after.execution.model_provider_id.as_deref() == Some("testkit") && after.execution.model_adapter_id.as_deref() == Some("openai_responses") && after.execution.model_id.as_deref() == Some("management-model") && after.execution.model_parallel_tool_calls == Some(false), "selection did not reach native client: {:?}",after.execution);
            ensure!(serde_json::to_value(&after.execution.selected_permission)? == serde_json::to_value(&before.execution.selected_permission)?);
            let invalid = operation(&case,&bot,"reject-unconfigured-management-model",json!({"action":"configure","session_id":target.id(),"model":{"provider_id":"testkit","adapter_id":"openai_responses","model_id":"invented-management-model"},"parallel_tool_calls":true,"expected_version":after.session.version})).await?;
            failed(&invalid)?;
            let unchanged = target.snapshot().await?;
            ensure!(unchanged.session.version == after.session.version && serde_json::to_value(&unchanged.execution)? == serde_json::to_value(&after.execution)?, "invalid model partially changed native selection");
            let invalid_mode = operation(&case,&bot,"reject-invalid-management-mode",json!({"action":"configure","session_id":target.id(),"verbosity":"invented-verbosity","expected_version":after.session.version})).await?;
            failed(&invalid_mode)?;
            ensure!(target.snapshot().await?.session.version == after.session.version);
            target.client().set_session_favorite(target.id(),true).await?;
            let human = target.snapshot().await?;
            let stale = operation(&case,&bot,"reject-stale-model-management",json!({"action":"configure","session_id":target.id(),"model":{"provider_id":"testkit","adapter_id":"openai_responses","model_id":"fake-model"},"expected_version":after.session.version})).await?;
            failed(&stale)?;
            let preserved = target.snapshot().await?;
            ensure!(preserved.session.version == human.session.version && preserved.session.favorite && preserved.execution.model_id.as_deref() == Some("management-model"));
            case.model().expect(RequestMatch::user_text("human-uses-bot-model-selection").model("management-model"))
                .check(|request| {ensure!(request.body["parallel_tool_calls"] == false,"configured parallel preference lost: {}",request.body);Ok(())})
                .reply(Reply::text("actual configured model answered"));
            target.send("human-uses-bot-model-selection").await?.wait_settled().await?.assert_assistant_text("actual configured model answered")?;
            let current = resource(&case,target.id()).await?;
            let reset = operation(&case,&bot,"reset-management-model-to-project-default",json!({"action":"configure","session_id":target.id(),"reset_model":true,"expected_version":version(&current)?})).await?;
            succeeded(&reset)?;
            case.model().expect(RequestMatch::user_text("human-uses-reset-project-default").model("fake-model")).reply(Reply::text("project default answered"));
            target.send("human-uses-reset-project-default").await?.wait_settled().await?.assert_assistant_text("project default answered")?;
            ensure!(target.snapshot().await?.session.favorite);
            Ok(())
        }).await
}

async fn queue(case: &TestCase, bot: &UserSession, target: i64, marker: &str) -> Result<Value> {
    let result = operation(
        case,
        bot,
        marker,
        json!({"action":"queue","session_id":target}),
    )
    .await?;
    succeeded(&result)?;
    Ok(result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_native_queue_pause_edit_remove_resume_and_human_followup() -> TestResult {
    fixture().case_timeout(Duration::from_secs(180))
        .seed_file("workspace/queue-attachment.txt",b"retained queued attachment".to_vec())
        .run(|case| async move {
        let bot = operator(&case).await?;
        let target = case.user().new_session().await?;
        let initial = target.snapshot().await?;
        let first = case.model().expect(RequestMatch::user_text("management-queue-active"))
            .reply(Reply::text("management active completed").held());
        let active = target.send("management-queue-active").await?;
        first.observed().await?;
        let document = serde_json::from_value(json!([
            {"type":"text","text":"management queued original"},
            {"type":"activity","activity":{"id":uuid::Uuid::new_v4(),"payload":{
                "activity_type":"resource","kind":"file","name":"queue-attachment.txt","media_type":"text/plain",
                "reference":{"reference_type":"workspace_path","path":"queue-attachment.txt"}
            }}}
        ]))?;
        let edited = target.send_document(document,RunOptions::default()).await?;
        let removed = target.send("management queued remove me").await?;
        ensure!(edited.status().await?.phase == ExecutionPhase::Queued && removed.status().await?.phase == ExecutionPhase::Queued);
        let observed = queue(&case,&bot,target.id(),"inspect-native-management-queue").await?;
        for marker in ["management queued original","management queued remove me"] {
            ensure!(observed.to_string().contains(marker),"queue omitted accepted human text: {observed}");
        }
        ensure!(observed.to_string().contains(&edited.receipt().execution_id.to_string()) && observed.to_string().contains(&removed.receipt().execution_id.to_string()));
        let paused = operation(&case,&bot,"pause-native-management-queue",json!({"action":"queue_pause","session_id":target.id(),"paused":true,"expected_control_version":control_version(&observed)?,"_test_retry":true})).await?;
        succeeded(&paused)?;
        ensure!(target.snapshot().await?.queue_paused);
        let observed = queue(&case,&bot,target.id(),"inspect-paused-management-queue").await?;
        let wrong_generation = operation(&case,&bot,"reject-newer-queue-generation",json!({"action":"queue_edit","session_id":target.id(),"execution_id":edited.receipt().execution_id.to_string(),"expected_generation":edited.receipt().attempt_generation+1,"expected_control_version":control_version(&observed)?,"instruction":"must never replace human queue"})).await?;
        failed(&wrong_generation)?;
        let native_edit = target.client().queued_message_for_edit(target.id(),edited.receipt().execution_id.into()).await?;
        ensure!(serde_json::to_string(&native_edit.document)?.contains("management queued original"));
        let attachment_before = native_edit.document.0.iter().filter(|node|matches!(node,agena_domain::ComposerNode::Activity{..})).cloned().collect::<Vec<_>>();
        ensure!(attachment_before.len() == 1);
        let changed = operation(&case,&bot,"edit-native-management-queue",json!({"action":"queue_edit","session_id":target.id(),"execution_id":edited.receipt().execution_id.to_string(),"expected_generation":edited.receipt().attempt_generation,"expected_control_version":control_version(&observed)?,"instruction":"management queued edited","_test_retry":true})).await?;
        succeeded(&changed)?;
        let native_edit = target.client().queued_message_for_edit(target.id(),edited.receipt().execution_id.into()).await?;
        ensure!(serde_json::to_string(&native_edit.document)?.contains("management queued edited") && !serde_json::to_string(&native_edit.document)?.contains("management queued original"));
        let attachment_after = native_edit.document.0.iter().filter(|node|matches!(node,agena_domain::ComposerNode::Activity{..})).cloned().collect::<Vec<_>>();
        ensure!(attachment_after == attachment_before,"Bot queue text edit dropped or changed attachments");
        let stale = operation(&case,&bot,"reject-stale-queue-remove",json!({"action":"queue_remove","session_id":target.id(),"execution_id":removed.receipt().execution_id.to_string(),"expected_generation":removed.receipt().attempt_generation,"expected_control_version":control_version(&observed)?})).await?;
        failed(&stale)?;
        ensure!(removed.status().await?.phase == ExecutionPhase::Queued);
        let observed = queue(&case,&bot,target.id(),"inspect-edited-management-queue").await?;
        let deleted = operation(&case,&bot,"remove-native-management-queue-entry",json!({"action":"queue_remove","session_id":target.id(),"execution_id":removed.receipt().execution_id.to_string(),"expected_generation":removed.receipt().attempt_generation,"expected_control_version":control_version(&observed)?,"_test_retry":true})).await?;
        succeeded(&deleted)?;
        ensure!(removed.status().await?.phase == ExecutionPhase::Cancelled);
        let native = target.snapshot().await?;
        ensure!(native.queue_paused && native.queued_messages.len() == 1 && native.queued_messages[0].execution_id == edited.receipt().execution_id);
        first.release();
        active.wait_settled().await?.assert_assistant_text("management active completed")?;
        ensure!(edited.status().await?.phase == ExecutionPhase::Queued);
        let resumed = case.model().expect(RequestMatch::user_text("management queued edited"))
            .check(|request| {let body=request.body.to_string();ensure!(body.contains("management active completed") && body.contains("queue-attachment.txt") && !request.user_text().contains("management queued original"));Ok(())})
            .reply(Reply::text("edited native queue consumed once").held())
            .with_timeout(Duration::from_secs(45));
        let observed = queue(&case,&bot,target.id(),"inspect-queue-before-native-resume").await?;
        let result = operation(&case,&bot,"resume-native-management-queue",json!({"action":"queue_pause","session_id":target.id(),"paused":false,"expected_control_version":control_version(&observed)?,"_test_retry":true})).await?;
        succeeded(&result)?;
        resumed.observed().await?;
        let native = target.snapshot().await?;
        ensure!(!native.queue_paused && native.queued_messages.is_empty());
        resumed.release();
        edited.wait_settled().await?.assert_assistant_text("edited native queue consumed once")?;
        case.model().expect(RequestMatch::user_text("human-after-management-queue"))
            .check(|request| {ensure!(request.body.to_string().contains("edited native queue consumed once"));Ok(())})
            .reply(Reply::text("human followup after queue management"));
        target.send("human-after-management-queue").await?.wait_settled().await?.assert_assistant_text("human followup after queue management")?;
        ensure!(removed.status().await?.phase == ExecutionPhase::Cancelled);
        let final_state = target.snapshot().await?;
        ensure!(final_state.queued_messages.is_empty() && !final_state.queue_paused);
        ensure!(serde_json::to_value(&initial.execution.selected_permission)? == serde_json::to_value(&final_state.execution.selected_permission)?);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_delete_uses_native_decline_cancel_and_exact_human_confirmation()
-> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(180))
        .seed_file(
            "workspace/deletion-survivor.txt",
            b"project files survive conversation deletion".to_vec(),
        )
        .run(|case| async move {
            let bot = operator(&case).await?;
            let bot_permission =
                serde_json::to_value(bot.snapshot().await?.execution.selected_permission)?;
            let unrelated = case
                .user()
                .new_session_with(SessionOptions {
                    title: "Delete candidate".into(),
                    ..Default::default()
                })
                .await?;
            let unrelated_before = resource(&case, unrelated.id()).await?;
            for decision in ["decline", "cancel", "confirm"] {
                let target = case
                    .user()
                    .new_session_with(SessionOptions {
                        title: "Delete candidate".into(),
                        ..Default::default()
                    })
                    .await?;
                let before = resource(&case, target.id()).await?;
                let observed = status(
                    &case,
                    &bot,
                    target.id(),
                    &format!("observe-delete-{decision}"),
                )
                .await?;
                let mut input = delete_input(target.id(), &observed)?;
                if decision == "confirm" {
                    input["_test_retry"] = json!(true);
                }
                let task_case = case.clone();
                let task_bot = bot.clone();
                let marker = format!("delete-with-human-{decision}");
                let task =
                    tokio::spawn(
                        async move { operation(&task_case, &task_bot, &marker, input).await },
                    );
                let card = bot.wait_user_input().await?;
                let request = card
                    .request
                    .request
                    .as_user_input()
                    .context("native delete input card")?;
                let card_text = serde_json::to_string(request)?;
                ensure!(
                    card_text.contains("Delete candidate")
                        && card_text.contains(&target.id().to_string())
                        && card_text.contains(&case.workspace().to_string_lossy().to_string()),
                    "approval did not identify exact target: {card_text}"
                );
                ensure!(
                    request.auto_resolution_ms != Some(0),
                    "delete card cannot auto-confirm"
                );
                ensure!(
                    resource(&case, target.id()).await? == before,
                    "deletion occurred before human confirmation"
                );
                card.mark_presented().await?;
                match decision {
                    "cancel" => {
                        card.reply_user_input(
                            UserInputReplyKind::Cancel,
                            BTreeMap::new(),
                            Some("Human cancelled deletion".into()),
                        )
                        .await?;
                    }
                    "decline" => {
                        card.answer(BTreeMap::from([(
                            "0".into(),
                            vec!["Keep conversation".into()],
                        )]))
                        .await?;
                    }
                    _ => {
                        card.answer(BTreeMap::from([(
                            "0".into(),
                            vec!["Delete conversation".into()],
                        )]))
                        .await?;
                    }
                }
                let result = task.await??;
                succeeded(&result)?;
                let list = native_list(
                    &case,
                    before["workspace_id"]
                        .as_i64()
                        .context("delete workspace")?,
                )
                .await?;
                if decision == "confirm" {
                    ensure!(
                        required(&result, "deleted")? == true
                            && required(&result, "soft_delete")? == true,
                        "{result}"
                    );
                    ensure!(
                        case.http()
                            .await
                            .get(&format!("/api/v1/sessions/{}", target.id()))
                            .await?
                            .status
                            == 404
                    );
                    ensure!(
                        !list.iter().any(|s| s["id"] == target.id()),
                        "deleted conversation remains in native list"
                    );
                } else {
                    ensure!(
                        required(&result, "deleted")? == false
                            && required(&result, "outcome")? == "cancelled",
                        "{result}"
                    );
                    ensure!(resource(&case, target.id()).await? == before);
                    ensure!(list.iter().any(|s| s["id"] == target.id()));
                }
                ensure!(
                    bot.snapshot()
                        .await?
                        .session
                        .state
                        .pending_interactive_requests()
                        .is_empty()
                );
                ensure!(
                    resource(&case, unrelated.id()).await? == unrelated_before,
                    "same-title neighbor was changed"
                );
                case.wait_file_bytes(
                    "workspace/deletion-survivor.txt",
                    b"project files survive conversation deletion",
                )
                .await?;
            }
            ensure!(
                serde_json::to_value(bot.snapshot().await?.execution.selected_permission)?
                    == bot_permission
            );
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_delete_conflicts_with_new_human_metadata_and_running_work() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(180))
        .run(|case| async move {
            let bot = operator(&case).await?;
            for mutation in ["metadata", "message"] {
                let target = case
                    .user()
                    .new_session_with(SessionOptions {
                        title: format!("Guarded delete {mutation}"),
                        ..Default::default()
                    })
                    .await?;
                let observed = status(
                    &case,
                    &bot,
                    target.id(),
                    &format!("observe-racing-delete-{mutation}"),
                )
                .await?;
                let input = delete_input(target.id(), &observed)?;
                let task_case = case.clone();
                let task_bot = bot.clone();
                let marker = format!("request-racing-delete-{mutation}");
                let task =
                    tokio::spawn(
                        async move { operation(&task_case, &task_bot, &marker, input).await },
                    );
                let card = bot.wait_user_input().await?;
                let mut pending = None;
                let mut gate = None;
                if mutation == "metadata" {
                    target
                        .client()
                        .set_session_favorite(target.id(), true)
                        .await?;
                    target
                        .client()
                        .set_session_pinned(target.id(), true)
                        .await?;
                } else {
                    let response = case
                        .model()
                        .expect(RequestMatch::user_text(
                            "human-new-work-during-delete-approval",
                        ))
                        .reply(Reply::text("new human work survived stale delete").held());
                    pending = Some(target.send("human-new-work-during-delete-approval").await?);
                    response.observed().await?;
                    gate = Some(response);
                }
                let before_reply = resource(&case, target.id()).await?;
                card.answer(BTreeMap::from([(
                    "0".into(),
                    vec!["Delete conversation".into()],
                )]))
                .await?;
                let result = task.await??;
                failed(&result)?;
                ensure!(
                    result.to_string().to_lowercase().contains("conflict")
                        || result.to_string().to_lowercase().contains("changed"),
                    "stale approval must report conflict: {result}"
                );
                ensure!(
                    resource(&case, target.id()).await? == before_reply,
                    "stale approval changed or deleted newer human work"
                );
                ensure!(
                    native_list(
                        &case,
                        before_reply["workspace_id"].as_i64().context("workspace")?
                    )
                    .await?
                    .iter()
                    .any(|s| s["id"] == target.id())
                );
                if let (Some(response), Some(run)) = (gate, pending) {
                    ensure!(
                        run.status().await?.phase == ExecutionPhase::Running,
                        "stale delete cancelled newer human execution"
                    );
                    response.release();
                    run.wait_settled()
                        .await?
                        .assert_assistant_text("new human work survived stale delete")?;
                } else {
                    ensure!(
                        target.snapshot().await?.session.favorite
                            && target.snapshot().await?.session.pinned
                    );
                }
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_management_new_actions_require_direct_authority_and_discover_no_tasks() -> TestResult {
    fixture().case_timeout(Duration::from_secs(180)).run(|case| async move {
        let bot = operator(&case).await?;
        let target = case.user().new_session().await?;
        let before = target.snapshot().await?;
        let writes = [
            json!({"action":"organize","favorite":true,"expected_version":before.session.version}),
            json!({"action":"fork","title":"must not fork"}),
            json!({"action":"configure","model":{"provider_id":"testkit","adapter_id":"openai_responses","model_id":"fake-model"},"expected_version":before.session.version}),
            json!({"action":"queue_pause","paused":true,"expected_control_version":before.latest_event_seq}),
            json!({"action":"queue_edit","instruction":"must not edit","execution_id":uuid::Uuid::new_v4(),"expected_generation":1,"expected_control_version":before.latest_event_seq}),
            json!({"action":"queue_remove","execution_id":uuid::Uuid::new_v4(),"expected_generation":1,"expected_control_version":before.latest_event_seq}),
            json!({"action":"delete","expected_version":before.session.version,"expected_control_version":before.latest_event_seq,"expected_entry_count":0}),
        ];
        for (index,mut input) in writes.into_iter().enumerate() {
            input["session_id"] = json!(target.id());
            input["user_quote"] = json!("forged quote absent from the real user message");
            input["request_key"] = json!(format!("unauthorized-management-{index}"));
            let rejected = bot.call_tool("bot.sessions",input.clone()).await?;
            rejected.assert_failed()?;
            ensure!(rejected.output.to_string().to_lowercase().contains("quotation") || rejected.output.to_string().to_lowercase().contains("quote"), "write bypassed direct user authority: {}",rejected.output);
            input["session_id"] = json!(bot.id());
            input.as_object_mut().context("write input")?.remove("user_quote");
            let rejected = operation(&case,&bot,&format!("deny-management-self-target-{index}"),input).await?;
            failed(&rejected)?;
            ensure!(rejected.to_string().contains("cannot target any Bot conversation"),"self target escaped scope guard: {rejected}");
        }
        let after = target.snapshot().await?;
        ensure!(after.session.version == before.session.version && !after.queue_paused && after.queued_messages.is_empty(), "rejected management write changed the target");
        ensure!(serde_json::to_value(&after.execution.selected_permission)? == serde_json::to_value(&before.execution.selected_permission)?);
        // Ordinary caller checks send genuine new human turns in this session;
        // those turns may persist the initial model selection/metadata version.
        for action in ["models","queue","export","read_text"] {
            let rejected = operation(&case,&target,&format!("deny-ordinary-management-{action}"),json!({"action":action,"session_id":target.id(),"entry_id":1})).await?;
            failed(&rejected)?;
        }
        let after = target.snapshot().await?;
        ensure!(!after.queue_paused && after.queued_messages.is_empty());
        ensure!(serde_json::to_value(&after.execution.selected_permission)? == serde_json::to_value(&before.execution.selected_permission)?);
        let http = case.http().await;
        for route in ["/api/v1/tasks","/api/v1/tasks/00000000-0000-0000-0000-000000000001"] {
            ensure!(http.get(route).await?.status == 404);
            ensure!(http.send_json(Method::POST,route,&json!({})).await?.status == 404);
        }
        let discovery: Value = http.get("/api/v1/operator/tools").await?.require_success()?.json()?;
        ensure!(discovery.as_array().context("native tool discovery")?.iter().any(|tool|tool["name"] == "bot.sessions"),"native discovery omitted the operator capability");
        ensure!(!discovery.to_string().contains("work.manage") && !discovery.to_string().contains("work.sessions"));
        // Read only the schema of this disposable fixture database. No user DB,
        // storage mutation or parallel execution/status table is involved.
        let schema = tokio::process::Command::new("python3").arg("-c").arg(
            "import json,pathlib,sqlite3,sys\np=pathlib.Path(sys.argv[1]).resolve()\nc=sqlite3.connect(p.as_uri()+'?mode=ro',uri=True)\nprint(json.dumps([r[0] for r in c.execute(\"SELECT name FROM sqlite_schema WHERE type='table'\")]))"
        ).arg(case.paths().data_dir.join("agena.db")).output().await?;
        ensure!(schema.status.success(),"read fixture schema: {}",String::from_utf8_lossy(&schema.stderr));
        let tables: Vec<String> = serde_json::from_slice(&schema.stdout)?;
        ensure!(tables.iter().any(|name|name == "agena_sessions") && tables.iter().any(|name|name == "agena_executions"));
        ensure!(!tables.iter().any(|name|name.to_lowercase().contains("task") || name.to_lowercase().contains("worker")),"retired tables found: {tables:?}");
        Ok(())
    }).await
}
