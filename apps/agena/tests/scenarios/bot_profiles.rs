//! Named Bot regressions against an authenticated, disposable real service.
//! Provider requests and tool calls use the existing scripted model, never a
//! production provider. Registry mutations are exclusively public HTTP APIs.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_api::bot::{BotList, BotPersona, BotProfile, BotSession};
use agena_testkit::{Reply, RequestMatch, TestCase, TestHttpResponse, TestResult, UserSession};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde_json::{Value, json};

use super::{bot::operation, fixture};

pub(super) async fn create(
    case: &TestCase,
    key: &str,
    name: &str,
    persona: Option<Value>,
) -> Result<BotProfile> {
    case.http()
        .await
        .send_json(
            Method::POST,
            "/api/v1/bots",
            &json!({
                "request_key":key,"name":name,"persona":persona
            }),
        )
        .await?
        .require_success()?
        .json()
}
pub(super) async fn profile(case: &TestCase, id: uuid::Uuid) -> Result<BotProfile> {
    case.http()
        .await
        .get(&format!("/api/v1/bots/{id}"))
        .await?
        .require_success()?
        .json()
}
async fn list(case: &TestCase, archived: bool) -> Result<BotList> {
    case.http()
        .await
        .get(&format!("/api/v1/bots?include_archived={archived}"))
        .await?
        .require_success()?
        .json()
}
async fn ensure_named(case: &TestCase, id: uuid::Uuid) -> Result<BotSession> {
    case.http()
        .await
        .send_bytes(
            Method::POST,
            &format!("/api/v1/bots/{id}/session"),
            None,
            vec![],
        )
        .await?
        .require_success()?
        .json()
}
async fn resource(case: &TestCase, id: i64) -> Result<Value> {
    case.http()
        .await
        .get(&format!("/api/v1/sessions/{id}"))
        .await?
        .require_success()?
        .json()
}
fn rejected(response: TestHttpResponse, meanings: &[&str]) -> Result<()> {
    // A JSON error inside HTTP 200, redirect, missing route, or server panic is
    // not an acceptable validation/conflict response.
    ensure!(
        response.status.is_client_error(),
        "expected HTTP 4xx: {} {}",
        response.status,
        response.text()?
    );
    let body = response.text()?.to_lowercase();
    ensure!(
        meanings.iter().any(|s| body.contains(s)),
        "missing meaningful error {meanings:?}: {body}"
    );
    Ok(())
}
fn phase(value: &Value, expected: &str) -> Result<()> {
    ensure!(value["phase"] == expected, "expected {expected}: {value}");
    Ok(())
}
fn field(value: &Value, name: &str) -> Option<Value> {
    if let Some(v) = value.get(name) {
        return Some(v.clone());
    }
    match value {
        Value::Object(v) => v.values().find_map(|v| field(v, name)),
        Value::Array(v) => v.iter().find_map(|v| field(v, name)),
        Value::String(s) => serde_json::Deserializer::from_str(s)
            .into_iter::<Value>()
            .next()
            .and_then(Result::ok)
            .and_then(|v| field(&v, name)),
        _ => None,
    }
}
async fn archive(case: &TestCase, p: &BotProfile, key: &str) -> Result<BotProfile> {
    case.http()
        .await
        .send_json(
            Method::POST,
            &format!("/api/v1/bots/{}/archive", p.id),
            &json!({
                "request_key":key,"expected_revision":p.revision
            }),
        )
        .await?
        .require_success()?
        .json()
}
async fn delete_session(case: &TestCase, id: i64) -> Result<()> {
    let version = resource(case, id).await?["version"]
        .as_i64()
        .context("metadata revision")?;
    let http = case.http().await;
    http.execute(
        http.request(Method::DELETE, &format!("/api/v1/sessions/{id}"))?
            .header("if-match", format!("\"{version}\"")),
    )
    .await?
    .require_success()?;
    Ok(())
}
async fn companion(
    case: &TestCase,
    source: &UserSession,
    text: &str,
    mut input: Value,
) -> Result<Value> {
    input["user_quote"] = json!(text);
    let call = format!("companion-{text}");
    let output = Arc::new(Mutex::new(None));
    let capture = output.clone();
    let matched_call = call.clone();
    let model = case.model();
    let start = model
        .expect(RequestMatch::user_text(text))
        .reply(Reply::tool_call(&call, "companion.manage", input));
    model
        .expect(RequestMatch::tool_output(&call))
        .after(&start)
        .check(move |r| {
            *capture.lock().unwrap() = Some(serde_json::from_str(
                &r.tool_output_text(&matched_call)
                    .context("companion result")?,
            )?);
            Ok(())
        })
        .reply(Reply::text(format!("finished-{text}")));
    source
        .send(text)
        .await?
        .wait_settled()
        .await?
        .assert_succeeded()?;
    let result = output
        .lock()
        .unwrap()
        .take()
        .context("captured companion result")?;
    Ok(result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_registry_auth_validation_concurrency_and_receipts() -> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).run(|case|async move {
        let http = case.http().await;
        rejected(http.anonymous().get("/api/v1/bots").await?, &["auth","unauthorized"])?;
        rejected(http.anonymous().send_json(Method::POST,"/api/v1/bots",&json!({"request_key":"anon","name":"No"})).await?, &["auth","unauthorized"])?;
        let a = create(&case,"create-alpha"," Alpha ",None).await?;
        ensure!(a.name == "Alpha" && a.id != uuid::Uuid::nil() && a.session_id > 0 && !a.archived);
        ensure!(a.persona == BotPersona::default(), "fresh Bot inherited another persona");
        let replay = create(&case,"create-alpha"," Alpha ",None).await?;
        ensure!(replay == a, "lost acknowledgment retry changed registry receipt");
        rejected(http.send_json(Method::POST,"/api/v1/bots",&json!({"request_key":"duplicate-alpha","name":" aLPHa "})).await?, &["name","exist","unique"])?;
        for (index,name) in ["", "   ", "bad\nname", "bad\u{0001}name", &"x".repeat(1000)].into_iter().enumerate() {
            rejected(http.send_json(Method::POST,"/api/v1/bots",&json!({"request_key":format!("bad-{index}"),"name":name})).await?, &["name","invalid","empty","control","byte"])?;
        }
        rejected(http.send_json(Method::POST,"/api/v1/bots",&json!({"request_key":"too-long-persona","name":"Long","persona":{"instructions":"x".repeat(4097)}})).await?, &["instruction","4096","byte"])?;
        // Every removed persona field must fail parsing on both endpoints, even
        // when it contains its former default value. Axum's typed JSON parser
        // rejects unknown fields with HTTP 422 before any profile write.
        for (key, value) in [("address_user", json!("")), ("tone", json!("balanced")), ("language", json!("auto"))] {
            let mut persona = json!({"instructions":"strict-check"});
            persona[key] = value;
            let response = http.send_json(Method::POST,"/api/v1/bots",&json!({"request_key":format!("legacy-create-{key}"),"name":format!("Legacy {key}"),"persona":persona})).await?;
            let status = response.status;
            let body = response.text()?;
            ensure!(status == reqwest::StatusCode::UNPROCESSABLE_ENTITY && body.contains("unknown field") && body.contains(key), "legacy create {key}: {status} {body}");
            let response = http.send_json(Method::PUT,&format!("/api/v1/bots/{}",a.id),&json!({"request_key":format!("legacy-update-{key}"),"expected_revision":a.revision,"name":a.name,"persona":persona})).await?;
            let status = response.status;
            let body = response.text()?;
            ensure!(status == reqwest::StatusCode::UNPROCESSABLE_ENTITY && body.contains("unknown field") && body.contains(key), "legacy update {key}: {status} {body}");
        }
        ensure!(profile(&case,a.id).await? == a, "rejected legacy update changed Bot");
        let mut jobs = Vec::new();
        for i in 0..12 {
            let case = case.clone();
            jobs.push(tokio::spawn(async move {
                let p = create(&case,"concurrent-beta","Beta",None).await?;
                let default = case.client().await.ensure_bot_session().await?;
                ensure!(default.bot_id != uuid::Uuid::nil());
                Ok::<_,anyhow::Error>((i,p,default))
            }));
        }
        let mut b = None;
        let mut default = None;
        for job in jobs {
            let (_,p,d) = job.await??;
            if let Some(ref before) = b { ensure!(before == &p, "concurrent create produced duplicate identities"); }
            if let Some(ref before) = default { ensure!(before == &d, "clients raced to different defaults"); }
            b = Some(p); default = Some(d);
        }
        let b = b.context("Beta")?;
        ensure!(b.id != a.id && b.session_id != a.session_id);
        let registry = list(&case,false).await?;
        ensure!(registry.items.iter().filter(|p|p.name == "Beta").count() == 1);
        ensure!(registry.default_bot_id == Some(default.context("default")?.bot_id));
        for bot in &registry.items {
            ensure!(resource(&case,bot.session_id).await?["workspace_id"].is_null(), "registry session is not global");
        }
        // Inspect the disposable fixture database to retain the orphan check:
        // public project lists deliberately exclude global sessions.
        let count = tokio::process::Command::new("python3").arg("-c").arg(
            "import pathlib,sqlite3,sys; p=pathlib.Path(sys.argv[1]).resolve(); c=sqlite3.connect(p.as_uri()+'?mode=ro',uri=True); print(c.execute('SELECT count(*) FROM agena_sessions WHERE workspace_id IS NULL').fetchone()[0])"
        ).arg(case.paths().data_dir.join("agena.db")).output().await?;
        ensure!(count.status.success(), "read fixture sessions: {}", String::from_utf8_lossy(&count.stderr));
        ensure!(String::from_utf8_lossy(&count.stdout).trim().parse::<usize>()? == registry.items.len(), "create/default retries produced orphan native sessions");
        let mut contenders=Vec::new();
        for (index,name) in ["Collision name"," collision NAME "].into_iter().enumerate() {
            let http=http.clone();
            contenders.push(tokio::spawn(async move {http.send_json(Method::POST,"/api/v1/bots",&json!({"request_key":format!("name-race-{index}"),"name":name})).await}));
        }
        let mut winners=0;
        for contender in contenders {let response=contender.await??;if response.status.is_success(){winners+=1;}else{rejected(response,&["name","exist","unique"])?;}}
        ensure!(winners==1,"concurrent normalized duplicate names escaped uniqueness");
        let before = profile(&case,a.id).await?;
        let update = json!({"request_key":"rename-alpha","expected_revision":before.revision,"name":"Alpha revised","persona":{"instructions":"DIRECT-REST-INSTRUCTIONS-710"}});
        let updated: BotProfile = http.send_json(Method::PUT,&format!("/api/v1/bots/{}",a.id),&update).await?.require_success()?.json()?;
        ensure!(updated.revision > before.revision && updated.session_id == a.session_id && updated.persona.instructions == "DIRECT-REST-INSTRUCTIONS-710");
        let direct_session = case.user().attach_session(updated.session_id).await?;
        case.model().expect(RequestMatch::user_text("direct-rest-prompt-check")).check(|r| {
            let body = r.body.to_string();
            ensure!(body.contains("Alpha revised") && body.contains("DIRECT-REST-INSTRUCTIONS-710"), "directly updated Bot instructions missing from model request");
            Ok(())
        }).reply(Reply::text("direct instructions checked"));
        direct_session.send("direct-rest-prompt-check").await?.wait_settled().await?.assert_succeeded()?;
        let retry: BotProfile = http.send_json(Method::PUT,&format!("/api/v1/bots/{}",a.id),&update).await?.require_success()?.json()?;
        ensure!(retry == updated, "idempotent update retried stale CAS instead of receipt");
        let mut stale = update.clone(); stale["request_key"] = json!("stale-alpha");
        rejected(http.send_json(Method::PUT,&format!("/api/v1/bots/{}",a.id),&stale).await?, &["revision","conflict","changed"])?;
        // Two clients editing the same revision must produce exactly one winner.
        let mut jobs = Vec::new();
        for i in 0..2 {
            let http = http.clone(); let updated = updated.clone();
            jobs.push(tokio::spawn(async move {http.send_json(Method::PUT,&format!("/api/v1/bots/{}",updated.id),&json!({"request_key":format!("race-update-{i}"),"expected_revision":updated.revision,"name":format!("Alpha winner {i}"),"persona":updated.persona})).await}));
        }
        let mut won = 0;
        for job in jobs { let r = job.await??; if r.status.is_success() { won += 1; } else { rejected(r,&["revision","conflict","changed"])?; } }
        ensure!(won == 1, "concurrent stale writes were both accepted or both lost");
        ensure!(profile(&case,b.id).await? == b);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_persist_persona_and_history_without_project_overlay() -> TestResult {
    fixture().case_timeout(Duration::from_secs(150)).run(|case|async move {
        let bot = create(&case, "persist-persona", "Persistent persona", Some(json!({"instructions":"persistent-instructions-marker-42"}))).await?;
        let session = case.user().attach_session(bot.session_id).await?;
        case.model().expect(RequestMatch::user_text("persistent-history-marker")).reply(Reply::text("persistent-history-preserved"));
        session.send("persistent-history-marker").await?.wait_settled().await?.assert_succeeded()?;
        case.crash().await?;
        case.restart().await?;
        ensure!(profile(&case,bot.id).await? == bot, "restart changed Bot identity or persona");
        ensure!(resource(&case,bot.session_id).await?["workspace_id"].is_null());
        let ordinary = case.user().new_session().await?;
        let snapshot = ordinary.client().assistant_snapshot(ordinary.id()).await?;
        case.http().await.send_json(Method::PUT,&format!("/api/v1/sessions/{}/assistant",ordinary.id()),&json!({"request_key":"workspace-after-restart","expected_revision":snapshot.profile.revision,"patch":{"name":"Later workspace name","instructions":"later-workspace-instructions"}})).await?.require_success()?;
        let fresh = create(&case,"fresh-after-restart","Fresh independent",None).await?;
        ensure!(fresh.persona == BotPersona::default());
        case.restart().await?;
        ensure!(profile(&case,bot.id).await? == bot, "workspace change/restart rewrote persona");
        let restored = case.user().attach_session(bot.session_id).await?;
        case.model().expect(RequestMatch::user_text("check-persistent-transcript")).check(|r| {
            let body = r.body.to_string();
            ensure!(body.contains("persistent-history-preserved") && body.contains("persistent-instructions-marker-42") && !body.contains("later-workspace-instructions")); Ok(())
        }).reply(Reply::text("persistence checked"));
        restored.send("check-persistent-transcript").await?.wait_settled().await?.assert_succeeded()?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_personas_replace_workspace_and_natural_edit_is_local() -> TestResult {
    fixture().case_timeout(Duration::from_secs(150)).run(|case|async move {
        let ordinary = case.user().new_session().await?;
        let workspace_before = ordinary.client().assistant_snapshot(ordinary.id()).await?;
        case.http().await.send_json(Method::PUT,&format!("/api/v1/sessions/{}/assistant",ordinary.id()),&json!({"request_key":"conflicting-workspace","expected_revision":workspace_before.profile.revision,"patch":{"name":"WorkspaceOnlyPersona","instructions":"WORKSPACE-CONFLICT-427","tone":"direct"}})).await?.require_success()?;
        let workspace = ordinary.client().assistant_snapshot(ordinary.id()).await?.profile;
        let a = create(&case,"persona-a","AuroraPersona",Some(json!({"instructions":"A-PRIVATE-INSTRUCTIONS-431"}))).await?;
        let b = create(&case,"persona-b","BorealisPersona",Some(json!({"instructions":"B-PRIVATE-INSTRUCTIONS-862"}))).await?;
        let sa = case.user().attach_session(a.session_id).await?;
        let sb = case.user().attach_session(b.session_id).await?;
        for (session,marker,name,instructions,other_name,other_instructions,reply) in [
            (&sa,"persona-check-a","AuroraPersona","A-PRIVATE-INSTRUCTIONS-431","BorealisPersona","B-PRIVATE-INSTRUCTIONS-862","A-PRIVATE-TRANSCRIPT-214"),
            (&sb,"persona-check-b","BorealisPersona","B-PRIVATE-INSTRUCTIONS-862","AuroraPersona","A-PRIVATE-INSTRUCTIONS-431","B-PRIVATE-TRANSCRIPT-563"),
        ] {
            case.model().expect(RequestMatch::user_text(marker)).check(move |r| {
                let body = r.body.to_string();
                ensure!(body.contains(name) && body.contains(instructions), "own persona absent from real provider request");
                for absent in [other_name,other_instructions,"WorkspaceOnlyPersona","WORKSPACE-CONFLICT-427"] {ensure!(!body.contains(absent),"foreign persona leaked: {absent}");}
                Ok(())
            }).reply(Reply::text(reply));
            session.send(marker).await?.wait_settled().await?.assert_succeeded()?;
        }
        let permission_before=serde_json::to_value(&sa.snapshot().await?.execution.selected_permission)?;
        let current = profile(&case,a.id).await?;
        let changed = companion(&case,&sa,"change-only-my-persona",json!({"action":"configure","expected_revision":current.revision,"patch":{"name":"AuroraRenamed","instructions":"A-EDITED-INSTRUCTIONS-519"}})).await?;
        phase(&changed,"succeeded")?;
        let edited = profile(&case,a.id).await?;
        ensure!(edited.name == "AuroraRenamed" && edited.persona.instructions == "A-EDITED-INSTRUCTIONS-519" && edited.revision > current.revision);
        ensure!(profile(&case,b.id).await? == b);
        ensure!(ordinary.client().assistant_snapshot(ordinary.id()).await?.profile == workspace);
        let forbidden = companion(&case,&sa,"reject-shared-memory-setting",json!({"action":"configure","expected_revision":edited.revision,"patch":{"memory_enabled":!workspace.preferences.memory_enabled}})).await?;
        phase(&forbidden,"failed")?;
        ensure!(
            forbidden.to_string().to_lowercase().contains("instance")
                || forbidden.to_string().to_lowercase().contains("global"),
            "shared setting needs explicit global-policy guidance: {forbidden}"
        );
        ensure!(ordinary.client().assistant_snapshot(ordinary.id()).await?.profile == workspace);
        for (session,marker,own,foreign) in [(&sa,"history-isolation-a","A-PRIVATE-TRANSCRIPT-214","B-PRIVATE-TRANSCRIPT-563"),(&sb,"history-isolation-b","B-PRIVATE-TRANSCRIPT-563","A-PRIVATE-TRANSCRIPT-214")] {
            case.model().expect(RequestMatch::user_text(marker)).check(move |r| {
                let body=r.body.to_string();ensure!(body.contains(own) && !body.contains(foreign),"cross-Bot transcript contamination");Ok(())
            }).reply(Reply::text("isolated history checked"));
            session.send(marker).await?.wait_settled().await?.assert_succeeded()?;
        }
        phase(&companion(&case,&sa,"reject-duplicate-natural-name",json!({"action":"configure","expected_revision":edited.revision,"patch":{"name":" BorealisPersona "}})).await?,"failed")?;
        ensure!(profile(&case,a.id).await?==edited && profile(&case,b.id).await?==b);
        // Even neutral or unchanged legacy presentation fields are forbidden on Bots.
        for (key, value) in [("address_user", json!("")), ("tone", json!("balanced")), ("language", json!("auto"))] {
            let mut patch = json!({});
            patch[key] = value;
            phase(&companion(&case,&sa,&format!("reject-legacy-{key}"),json!({"action":"configure","expected_revision":edited.revision,"patch":patch})).await?,"failed")?;
            ensure!(profile(&case,a.id).await? == edited, "legacy {key} changed Bot");
        }
        phase(&companion(&case,&sa,"reject-noop-shared-policy",json!({"action":"configure","expected_revision":edited.revision,"patch":{"memory_enabled":workspace.preferences.memory_enabled}})).await?,"failed")?;
        ensure!(profile(&case,a.id).await? == edited, "rejected instance policy update changed Bot");
        case.model().expect(RequestMatch::user_text("edited-instructions-effective")).check(|r| {
            let body = r.body.to_string();
            ensure!(body.contains("AuroraRenamed") && body.contains("A-EDITED-INSTRUCTIONS-519") && !body.contains("B-PRIVATE-INSTRUCTIONS-862"), "edited instructions missing or foreign instructions leaked");
            Ok(())
        }).reply(Reply::text("edited instructions checked"));
        sa.send("edited-instructions-effective").await?.wait_settled().await?.assert_succeeded()?;
        ensure!(ordinary.client().assistant_snapshot(ordinary.id()).await?.profile==workspace);
        ensure!(serde_json::to_value(&sa.snapshot().await?.execution.selected_permission)?==permission_before,"persona edits changed tool permissions");
        let snap = sa.client().assistant_snapshot(sa.id()).await?;
        ensure!(snap.profile.preferences.name == "AuroraRenamed" && snap.profile.revision == edited.revision);
        case.model().expect(RequestMatch::user_text("ordinary-workspace-still-effective")).check(|r| {
            let body=r.body.to_string();ensure!(body.contains("WORKSPACE-CONFLICT-427") && !body.contains("A-EDITED-INSTRUCTIONS-519") && !body.contains("B-PRIVATE-INSTRUCTIONS-862"));Ok(())
        }).reply(Reply::text("ordinary project settings preserved"));
        ordinary.send("ordinary-workspace-still-effective").await?.wait_settled().await?.assert_succeeded()?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_reminder_inspection_and_control_are_source_owned() -> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).run(|case|async move {
        let a = create(&case,"reminder-a","Reminder A",None).await?;
        let b = create(&case,"reminder-b","Reminder B",None).await?;
        let sa = case.user().attach_session(a.session_id).await?;
        let sb = case.user().attach_session(b.session_id).await?;
        for (session,marker) in [(&sa,"source-a-reminder"),(&sb,"source-b-reminder")] {
            phase(&companion(&case,session,marker,json!({"action":"remind","reminder":{"text":marker,"after_seconds":3600}})).await?,"succeeded")?;
        }
        let ar = sa.client().assistant_snapshot(sa.id()).await?.reminders;
        let br = sb.client().assistant_snapshot(sb.id()).await?.reminders;
        ensure!(ar.len() == 1 && br.len() == 1 && ar[0].source_session_id == sa.id() && br[0].source_session_id == sb.id() && ar[0].id != br[0].id);
        let foreign = companion(&case,&sa,"deny-foreign-reminder-cancel",json!({"action":"cancel_reminder","reminder_id":br[0].id,"expected_revision":br[0].revision})).await?;
        phase(&foreign,"failed")?;
        let http = case.http().await;
        rejected(http.send_json(Method::POST,&format!("/api/v1/sessions/{}/assistant/reminders/{}",sa.id(),br[0].id),&json!({"action":"cancel","expected_revision":br[0].revision})).await?, &["source","conversation","not found","scope"])?;
        ensure!(sb.client().assistant_snapshot(sb.id()).await?.reminders[0].revision == br[0].revision);
        phase(&companion(&case,&sa,"cancel-own-reminder",json!({"action":"cancel_reminder","reminder_id":ar[0].id,"expected_revision":ar[0].revision})).await?,"succeeded")?;
        ensure!(sa.client().assistant_snapshot(sa.id()).await?.reminders[0].state == agena_api::assistant::ReminderState::Cancelled);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_restart_and_deleted_session_recovery_remain_independent() -> TestResult {
    fixture()
        .case_timeout(Duration::from_secs(150))
        .run(|case| async move {
            let a = create(
                &case,
                "recover-a",
                "Recovery A",
                Some(json!({"instructions":"recovery-persona-a"})),
            )
            .await?;
            let b = create(&case, "recover-b", "Recovery B", None).await?;
            let sb = case.user().attach_session(b.session_id).await?;
            case.model()
                .expect(RequestMatch::user_text("b-history-before-recovery"))
                .reply(Reply::text("b-history-survives-a-deletion"));
            sb.send("b-history-before-recovery")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            case.http()
                .await
                .send_json(
                    Method::PUT,
                    &format!("/api/v1/sessions/{}", b.session_id),
                    &json!({"title":"Human custom title B"}),
                )
                .await?
                .require_success()?;
            let b_session = resource(&case, b.session_id).await?;
            delete_session(&case, a.session_id).await?;
            let mut jobs = Vec::new();
            for _ in 0..10 {
                let case = case.clone();
                jobs.push(tokio::spawn(async move { ensure_named(&case, a.id).await }));
            }
            let mut replacement = None;
            for job in jobs {
                let next = job.await??;
                ensure!(
                    next.bot_id == a.id
                        && next.session_id != a.session_id
                        && next.session_id != b.session_id
                );
                if let Some(id) = replacement {
                    ensure!(
                        next.session_id == id,
                        "recovery exploded into orphan conversations"
                    );
                }
                replacement = Some(next.session_id);
            }
            let replacement = replacement.context("replacement")?;
            let recovered = profile(&case, a.id).await?;
            ensure!(
                recovered.id == a.id
                    && recovered.persona == a.persona
                    && recovered.name == a.name
                    && recovered.session_id == replacement
            );
            ensure!(
                profile(&case, b.id).await? == b
                    && resource(&case, b.session_id).await? == b_session
            );
            case.crash().await?;
            case.restart().await?;
            ensure!(
                ensure_named(&case, a.id).await?.session_id == replacement
                    && ensure_named(&case, b.id).await?.session_id == b.session_id
            );
            let sb = case.user().attach_session(b.session_id).await?;
            case.model()
                .expect(RequestMatch::user_text("b-history-after-restart"))
                .check(|r| {
                    ensure!(
                        r.body.to_string().contains("b-history-survives-a-deletion")
                            && !r.body.to_string().contains("recovery-persona-a")
                    );
                    Ok(())
                })
                .reply(Reply::text("B intact"));
            sb.send("b-history-after-restart")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let sa = case.user().attach_session(replacement).await?;
            let denied = operation(
                &case,
                &sb,
                "deny-replaced-bot-id",
                json!({"action":"status","session_id":a.session_id}),
            )
            .await?;
            phase(&denied, "failed")?;
            let denied = operation(
                &case,
                &sa,
                "deny-other-bot-after-recovery",
                json!({"action":"read","session_id":b.session_id}),
            )
            .await?;
            phase(&denied, "failed")?;
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_archive_requires_idle_cas_revokes_authority_and_never_resurrects()
-> TestResult {
    fixture().case_timeout(Duration::from_secs(180)).run(|case|async move {
        let a=create(&case,"archive-a","Archive A",None).await?;
        let a:BotProfile=case.http().await.send_json(Method::PUT,&format!("/api/v1/bots/{}",a.id),&json!({"request_key":"archive-positive-stale-revision","expected_revision":a.revision,"name":"Archive A revised","persona":a.persona})).await?.require_success()?.json()?;
        let b=create(&case,"archive-b","Archive B",None).await?;
        let sa=case.user().attach_session(a.session_id).await?;
        let ordinary=case.user().new_session().await?;
        let workspace_before=ordinary.client().assistant_snapshot(ordinary.id()).await?.profile;
        let held=case.model().expect(RequestMatch::user_text("archive-running-gate")).reply(Reply::text("running gate released").held());
        let running=sa.send("archive-running-gate").await?;held.observed().await?;
        rejected(case.http().await.send_json(Method::POST,&format!("/api/v1/bots/{}/archive",a.id),&json!({"request_key":"archive-running","expected_revision":a.revision})).await?, &["idle","running","active","busy"])?;
        held.release();running.wait_settled().await?.assert_succeeded()?;
        // A native interactive request is another non-idle state.
        let directory=case.workspace().join("archive-pending-project");tokio::fs::create_dir(&directory).await?;
        sa.client().set_session_permission(sa.id(),serde_json::from_value(json!({"path":{"rules":{(directory.to_string_lossy().as_ref()):{"read":"ask","write":"deny"}}}}))?).await?;
        let model=case.model();
        let start=model.expect(RequestMatch::user_text("archive-pending-gate")).reply(Reply::tool_call("pending-archive-register","bot.sessions",json!({"action":"register_project","path":directory,"user_quote":"archive-pending-gate","request_key":"archive-pending-register"})));
        model.expect(RequestMatch::tool_output("pending-archive-register")).after(&start).reply(Reply::text("pending gate released"));
        let run=sa.send("archive-pending-gate").await?;let card=run.wait_permission().await?;
        rejected(case.http().await.send_json(Method::POST,&format!("/api/v1/bots/{}/archive",a.id),&json!({"request_key":"archive-pending","expected_revision":a.revision})).await?, &["idle","pending","active","busy"])?;
        card.allow_once().await?;run.wait_settled().await?.assert_succeeded()?;
        sa.client().set_message_queue_paused(sa.id(),true).await?;
        let queued=sa.send("archive-queued-gate").await?;
        ensure!(queued.status().await?.phase==agena_domain::session_kernel::ExecutionPhase::Queued);
        rejected(case.http().await.send_json(Method::POST,&format!("/api/v1/bots/{}/archive",a.id),&json!({"request_key":"archive-queued","expected_revision":a.revision})).await?, &["idle","queued","active","busy"])?;
        case.model().expect(RequestMatch::user_text("archive-queued-gate")).reply(Reply::text("queued gate released"));
        sa.client().set_message_queue_paused(sa.id(),false).await?;
        queued.wait_settled().await?.assert_succeeded()?;
        let current=profile(&case,a.id).await?;
        rejected(case.http().await.send_json(Method::POST,&format!("/api/v1/bots/{}/archive",a.id),&json!({"request_key":"archive-stale","expected_revision":current.revision-1})).await?, &["revision","conflict","changed"])?;
        let archived=archive(&case,&current,"archive-idle").await?;
        ensure!(archived.archived && archived.session_id==a.session_id && archived.revision>current.revision);
        ensure!(archive(&case,&current,"archive-idle").await?==archived,"archive retry lost receipt");
        ensure!(profile(&case,a.id).await?==archived && !list(&case,false).await?.items.iter().any(|p|p.id==a.id));
        rejected(case.http().await.send_bytes(Method::POST,&format!("/api/v1/bots/{}/session",a.id),None,vec![]).await?, &["archiv","inactive"])?;
        // Archived history rejects model/tool admission before any provider
        // round; the ordinary profile endpoint must reject fallthrough too.
        rejected(case.http().await.send_json(Method::POST,&format!("/api/v1/sessions/{}/messages",sa.id()),&json!({"document":[{"type":"text","text":"archived cannot request bot.sessions or companion.manage"}]})).await?, &["archiv","read-only"])?;
        rejected(case.http().await.send_json(Method::PUT,&format!("/api/v1/sessions/{}/assistant",sa.id()),&json!({"request_key":"archived-profile-fallthrough","expected_revision":workspace_before.revision,"patch":{"name":"MUST NOT CHANGE WORKSPACE"}})).await?, &["archiv","reserved","bot"])?;
        ensure!(ordinary.client().assistant_snapshot(ordinary.id()).await?.profile==workspace_before);
        let sb=case.user().attach_session(b.session_id).await?;
        phase(&operation(&case,&sb,"deny-archived-target",json!({"action":"read","session_id":a.session_id})).await?,"failed")?;
        let active=list(&case,false).await?.items;
        for (index,p) in active.iter().enumerate() {
            archive(&case,p,&format!("archive-all-{}",p.id)).await?;
            ensure!(list(&case,false).await?.default_bot_id==active.get(index+1).map(|p|p.id),"archive did not resolve oldest remaining active default");
        }
        let empty=list(&case,false).await?;ensure!(empty.items.is_empty() && empty.default_bot_id.is_none());
        rejected(case.http().await.send_bytes(Method::POST,"/api/v1/bot/session",None,vec![]).await?, &["create","active","archiv"])?;
        case.restart().await?;
        ensure!(list(&case,false).await?.items.is_empty() && list(&case,true).await?.items.iter().any(|p|p.id==a.id));
        let fresh=create(&case,"explicit-after-all-archive","Explicit new Bot",None).await?;
        ensure!(fresh.id!=a.id && fresh.id!=b.id && case.client().await.ensure_bot_session().await?.bot_id==fresh.id);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_both_manage_projects_but_never_each_other() -> TestResult {
    fixture().case_timeout(Duration::from_secs(240)).run(|case|async move {
        let a=create(&case,"management-a","Management A",None).await?;
        let b=create(&case,"management-b","Management B",None).await?;
        let sa=case.user().attach_session(a.session_id).await?;
        let sb=case.user().attach_session(b.session_id).await?;
        let project=case.user().new_session().await?;
        let workspace=resource(&case,project.id()).await?["workspace_id"].as_i64().context("project workspace")?;
        for (source,other,tag) in [(&sa,&b,"a"),(&sb,&a,"b")] {
            let created=operation(&case,source,&format!("create-project-{tag}"),json!({"action":"create","workspace_id":workspace,"title":format!("Ordinary project {tag}"),"_test_retry":true})).await?;phase(&created,"succeeded")?;
            let target=field(&created,"session_id").and_then(|v|v.as_i64()).context("created ordinary project")?;
            let before=resource(&case,target).await?;
            ensure!(before["workspace_id"] == workspace && resource(&case,source.id()).await?["workspace_id"].is_null(), "project operation relocated Bot or created a global target");
            phase(&operation(&case,source,&format!("rename-project-{tag}"),json!({"action":"rename","session_id":target,"title":format!("Renamed project {tag}"),"expected_version":before["version"]})).await?,"succeeded")?;
            ensure!(resource(&case,target).await?["title"]==format!("Renamed project {tag}"));
            for action in ["read","status","export","models","queue"] {
                phase(&operation(&case,source,&format!("deny-cross-{tag}-{action}"),json!({"action":action,"session_id":other.session_id})).await?,"failed")?;
            }
            let before_other=resource(&case,other.session_id).await?;
            phase(&operation(&case,source,&format!("deny-cross-rename-{tag}"),json!({"action":"rename","session_id":other.session_id,"title":"forged foreign title","expected_version":before_other["version"]})).await?,"failed")?;
            ensure!(resource(&case,other.session_id).await?==before_other);
            let listed=operation(&case,source,&format!("discover-projects-{tag}"),json!({"action":"sessions","workspace_id":workspace})).await?;phase(&listed,"succeeded")?;
            ensure!(!listed.to_string().contains("Management A") && !listed.to_string().contains("Management B"),"Bot profiles appeared as ordinary project targets: {listed}");

        }
        // Ordinary sessions remain unauthorized even with a matching title and
        // selected permissions. Native child boundaries have a separate scenario.
        let ordinary=case.user().new_session().await?;
        case.http().await.send_json(Method::PUT,&format!("/api/v1/sessions/{}",ordinary.id()),&json!({"title":"Management A"})).await?.require_success()?;
        phase(&operation(&case,&ordinary,"ordinary-not-registry-member",json!({"action":"projects"})).await?,"failed")?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_reserved_descendants_stay_hidden_after_archive_and_replacement() -> TestResult
{
    fixture()
        .case_timeout(Duration::from_secs(150))
        .run(|case| async move {
            let a = create(&case, "descendant-a", "Descendant A", None).await?;
            let b = create(&case, "descendant-b", "Descendant B", None).await?;
            let sa = case.user().attach_session(a.session_id).await?;
            let sb = case.user().attach_session(b.session_id).await?;
            case.model()
                .expect(RequestMatch::user_text("reserved-parent-history"))
                .reply(Reply::text("RESERVED-PARENT-TRANSCRIPT-957"));
            sa.send("reserved-parent-history")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let http = case.http().await;
            let version = resource(&case, a.session_id).await?["version"]
                .as_i64()
                .context("fork version")?;
            let descendant: Value = http
                .execute(
                    http.request(
                        Method::POST,
                        &format!("/api/v1/sessions/{}/fork", a.session_id),
                    )?
                    .header("if-match", format!("\"{version}\""))
                    .json(&json!({"title":"Reserved descendant marker"})),
                )
                .await?
                .require_success()?
                .json()?;
            let child = descendant["session"]["id"]
                .as_i64()
                .context("forked descendant id")?;
            let c = create(&case, "descendant-c", "Recovery neighbor", None).await?;
            let sc = case.user().attach_session(c.session_id).await?;
            case.model()
                .expect(RequestMatch::user_text("reserved-replaced-parent-history"))
                .reply(Reply::text("RESERVED-REPLACED-TRANSCRIPT-268"));
            sc.send("reserved-replaced-parent-history")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let version = resource(&case, c.session_id).await?["version"]
                .as_i64()
                .context("replacement fork version")?;
            let fork: Value = http
                .execute(
                    http.request(
                        Method::POST,
                        &format!("/api/v1/sessions/{}/fork", c.session_id),
                    )?
                    .header("if-match", format!("\"{version}\""))
                    .json(&json!({"title":"Replaced descendant marker"})),
                )
                .await?
                .require_success()?
                .json()?;
            let replaced_child = fork["session"]["id"]
                .as_i64()
                .context("replacement fork id")?;
            for (index, state) in ["active", "archived", "replaced"].into_iter().enumerate() {
                if state == "archived" {
                    archive(
                        &case,
                        &profile(&case, a.id).await?,
                        "archive-reserved-parent",
                    )
                    .await?;
                }
                if state == "replaced" {
                    delete_session(&case, c.session_id).await?;
                    ensure_named(&case, c.id).await?;
                }
                for target in [a.session_id, child, c.session_id, replaced_child] {
                    let result = operation(
                        &case,
                        &sb,
                        &format!("deny-reserved-{index}-{target}"),
                        json!({"action":"read","session_id":target}),
                    )
                    .await?;
                    phase(&result, "failed")?;
                    ensure!(
                        !result
                            .to_string()
                            .contains("RESERVED-PARENT-TRANSCRIPT-957")
                            && !result
                                .to_string()
                                .contains("RESERVED-REPLACED-TRANSCRIPT-268"),
                        "denial leaked a hidden transcript"
                    );
                }
                let listed = operation(
                    &case,
                    &sb,
                    &format!("list-reserved-{index}"),
                    json!({"action":"sessions"}),
                )
                .await?;
                phase(&listed, "succeeded")?;
                ensure!(
                    !listed.to_string().contains("Reserved descendant marker")
                        && !listed.to_string().contains("Replaced descendant marker"),
                    "discovery exposed reserved Bot descendant"
                );
            }
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_reminder_delivery_stays_in_original_bot_with_bounded_cadence() -> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).run(|case|async move {
        let a=create(&case,"delivery-a","Delivery A",None).await?;
        let b=create(&case,"delivery-b","Delivery B",None).await?;
        let sa=case.user().attach_session(a.session_id).await?;
        let sb=case.user().attach_session(b.session_id).await?;
        let model=case.model();
        // Register before scheduling: the actual native scheduler polls every
        // 30 seconds. No sleep-based assumption or second foreground fake turn.
        let delivered=model.expect(RequestMatch::new("A source-owned scheduled contact",|r|{
            let body=r.body.to_string();body.contains("Scheduled assistant contact") && body.contains("delivery-source-a-584")
        })).check(|r|{ensure!(!r.body.to_string().contains("Delivery B"));Ok(())})
            .reply(Reply::text("source-a-delivery-confirmed")).with_timeout(Duration::from_secs(80));
        phase(&companion(&case,&sa,"schedule-only-source-a",json!({"action":"remind","reminder":{"text":"delivery-source-a-584","after_seconds":1}})).await?,"succeeded")?;
        delivered.finished().await?;
        tokio::time::timeout(Duration::from_secs(15),sa.wait_text_contains("source-a-delivery-confirmed")).await.context("delivered source transcript did not settle")??;
        ensure!(sa.client().assistant_snapshot(sa.id()).await?.reminders.iter().any(|r|r.spec.text=="delivery-source-a-584" && r.delivered==1));
        ensure!(sb.client().assistant_snapshot(sb.id()).await?.reminders.is_empty());
        model.expect(RequestMatch::user_text("delivery-b-no-foreign-contact")).check(|r|{
            let body=r.body.to_string();ensure!(!body.contains("source-a-delivery-confirmed") && !body.contains("delivery-source-a-584"),"other Bot received A's contact/context");Ok(())
        }).reply(Reply::text("B untouched by A's contact"));
        sb.send("delivery-b-no-foreign-contact").await?.wait_settled().await?.assert_succeeded()?;
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_profiles_both_route_native_subagents_with_hidden_source_and_target_guards()
-> TestResult {
    fixture().case_timeout(Duration::from_secs(240)).run(|case|async move {
        let a=create(&case,"native-a","Native A",None).await?;
        let b=create(&case,"native-b","Native B",None).await?;
        let sa=case.user().attach_session(a.session_id).await?;
        let sb=case.user().attach_session(b.session_id).await?;
        for (source,tag) in [(&sa,"a"),(&sb,"b")] {
            let target=case.user().new_session().await?.id();
            // Existing native Subagent execution, routed by each Bot in turn.
            let marker=format!("native-work-{tag}");let prompt=format!("native-child-{tag}");
            let prompt_match=prompt.clone();
            let start=case.model().expect(RequestMatch::new(format!("routed native {tag}"),move |r|{let body=r.body.to_string();body.contains("Operator Bot instruction (Bot session") && body.contains(&marker)})).reply(Reply::tool_call(format!("spawn-{tag}"),"agent.spawn",json!({"name":format!("profile_child_{tag}"),"prompt":prompt})));
            case.model().expect(RequestMatch::tool_output(format!("spawn-{tag}"))).after(&start).reply(Reply::text("project waiting for native child"));
            let child_start=case.model().expect(RequestMatch::user_text(prompt_match)).reply(Reply::tool_call(format!("native-child-denied-{tag}"),"bot.sessions",json!({"action":"projects"})));
            let child=case.model().expect(RequestMatch::tool_output(format!("native-child-denied-{tag}"))).after(&child_start).check_tool_output_contains(format!("native-child-denied-{tag}"),"failed").reply(Reply::text(format!("native-child-evidence-{tag}")).held());
            phase(&operation(&case,source,&format!("send-native-{tag}"),json!({"action":"send","session_id":target,"instruction":format!("native-work-{tag}")})).await?,"succeeded")?;
            child.observed().await?;
            let status=operation(&case,source,&format!("inspect-native-{tag}"),json!({"action":"status","session_id":target})).await?;
            let children=field(&status,"subagents").context("native Subagents")?;
            let child_id=children[0]["session_id"].as_i64().context("native child id")?;
            phase(&operation(&case,source,&format!("deny-hidden-child-{tag}"),json!({"action":"read","session_id":child_id})).await?,"failed")?;
            let report_marker=format!("native-child-evidence-{tag}");
            let report=case.model().expect(RequestMatch::new(format!("native child completion {tag}"),move|r|r.body.to_string().contains(&report_marker))).reply(Reply::text(format!("project completed {tag}"))).with_timeout(Duration::from_secs(80));
            child.release();report.finished().await?;
            case.user().attach_session(target).await?.wait_text_contains(&format!("project completed {tag}")).await?;
        }
        Ok(())
    }).await
}
