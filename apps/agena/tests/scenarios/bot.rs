//! Operator Bot uses real scoped plugin calls and ordinary project sessions.
use super::fixture;
use agena_testkit::{Reply, RequestMatch, TestCase, TestResult, UserSession};
use anyhow::{Context, Result, ensure};
use reqwest::Method;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

pub(super) async fn operation(
    case: &TestCase,
    session: &UserSession,
    message: &str,
    mut input: Value,
) -> Result<Value> {
    let call = format!("bot-call-{message}");
    let retry = input
        .get("_test_retry")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    input.as_object_mut().expect("object").remove("_test_retry");
    input["user_quote"] = json!(message);
    if input.get("request_key").is_none() {
        input["request_key"] = json!(message);
    }
    let output = Arc::new(Mutex::new(None));
    let capture = output.clone();
    let model = case.model();
    let start = model
        .expect(RequestMatch::user_text(message))
        .reply(Reply::tool_call(&call, "bot.sessions", input.clone()));
    let call2 = call.clone();
    model
        .expect(RequestMatch::tool_output(&call))
        .after(&start)
        .check(move |request| {
            let value: Value =
                serde_json::from_str(&request.tool_output_text(&call2).context("tool output")?)?;
            *capture.lock().unwrap() = Some(value);
            Ok(())
        })
        .reply(if retry {
            Reply::tool_call(format!("{call}-retry"), "bot.sessions", input)
        } else {
            Reply::text(format!("operation finished: {message}"))
        });
    if retry {
        let output = output.clone();
        let retry_call = format!("{call}-retry");
        let matched_call = retry_call.clone();
        model
            .expect(RequestMatch::tool_output(&retry_call))
            .check(move |request| {
                let value: Value = serde_json::from_str(
                    &request
                        .tool_output_text(&matched_call)
                        .context("retry output")?,
                )?;
                let before = output.lock().unwrap();
                let before = before.as_ref().context("first receipt")?;
                ensure!(
                    field(before, "execution_id") == field(&value, "execution_id"),
                    "retry created another execution"
                );
                ensure!(value["phase"] == "succeeded", "{value}");
                Ok(())
            })
            .reply(Reply::text(format!("operation finished: {message}")));
    }
    session
        .send(message)
        .await?
        .wait_settled()
        .await?
        .assert_succeeded()?;
    let value = output.lock().unwrap().take().context("operation output")?;
    Ok(value)
}

async fn resource(case: &TestCase, id: i64) -> Result<Value> {
    case.http()
        .await
        .get(&format!("/api/v1/sessions/{id}"))
        .await?
        .require_success()?
        .json()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_ensure_is_authenticated_concurrent_persistent_and_preserves_user_settings()
-> TestResult {
    fixture()
        .run(|case| async move {
            let ordinary = case.user().new_session().await?;
            let http = case.http().await;
            http.send_json(
                Method::PUT,
                &format!("/api/v1/sessions/{}", ordinary.id()),
                &json!({"title":"Bot"}),
            )
            .await?
            .require_success()?;
            let unauthorized = http
                .anonymous()
                .send_bytes(Method::POST, "/api/v1/bot/session", None, Vec::new())
                .await?;
            ensure!(unauthorized.status.as_u16() == 401);
            let client = case.client().await;
            let mut requests = Vec::new();
            for _ in 0..16 {
                let client = client.clone();
                requests.push(tokio::spawn(
                    async move { client.ensure_bot_session().await },
                ));
            }
            let mut ids = Vec::new();
            for request in requests {
                ids.push(request.await??.session_id);
            }
            let id = ids[0];
            ensure!(id > 0 && id != ordinary.id() && ids.iter().all(|v| *v == id));
            let body: Value = http
                .send_bytes(Method::POST, "/api/v1/bot/session", None, Vec::new())
                .await?
                .require_success()?
                .json()?;
            ensure!(body["session_id"] == id);
            let bot_id = body["bot_id"]
                .as_str()
                .context("default Bot registry UUID")?;
            uuid::Uuid::parse_str(bot_id)?;
            let profile = client.get_bot(bot_id.parse()?).await?;
            // A Bot owns many conversations; the binding names the Bot, not a
            // session, and the session resource names its owner.
            ensure!(body["name"] == profile.name);
            ensure!(resource(&case, id).await?["owner_bot_id"] == json!(bot_id));
            http.send_json(
                Method::PUT,
                &format!("/api/v1/sessions/{id}"),
                &json!({"title":"My operator","favorite":true}),
            )
            .await?
            .require_success()?;
            let before = resource(&case, id).await?;
            ensure!(client.ensure_bot_session().await?.session_id == id);
            ensure!(resource(&case, id).await? == before);
            case.restart().await?;
            ensure!(case.client().await.ensure_bot_session().await?.session_id == id);
            let after = resource(&case, id).await?;
            ensure!(after["title"] == "My operator" && after["favorite"] == true);
            Ok(())
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_creates_visible_projects_routes_work_and_allows_bot_and_human_continuation_without_tasks()
-> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).run(|case| async move {
        let id=case.client().await.ensure_bot_session().await?.session_id;
        let bot=case.user().attach_session(id).await?;
        let project=case.workspace().join("second-project");
        tokio::fs::create_dir(&project).await?;
        let registration=operation(&case,&bot,"register-project",json!({"action":"register_project","path":project})).await?;
        ensure!(registration["phase"]=="succeeded","{registration}");
        let workspace=field(&registration,"workspace_id").context("registered project ID")?.as_i64().context("workspace integer")?;
        let projects=operation(&case,&bot,"list-projects",json!({"action":"projects","limit":1})).await?;
        ensure!(projects["phase"]=="succeeded");
        // Repeat the same create in one real user turn: the lost receipt retry
        // must return the original normal session rather than another root.
        let model=case.model();
        let input=json!({"action":"create","workspace_id":workspace,"title":"Visible project work","user_quote":"create-project-session","request_key":"create-one"});
        let capture:Arc<Mutex<Vec<Value>>>=Default::default();
        let first=model.expect(RequestMatch::user_text("create-project-session"))
            .check(|request| {ensure!(request.body.to_string().contains("## Named operator Bot"));Ok(())})
            .reply(Reply::tool_call("create-first","bot.sessions",input.clone()));
        let output=capture.clone();
        let second=model.expect(RequestMatch::tool_output("create-first")).after(&first).check(move|r|{
            let v:Value=serde_json::from_str(&r.tool_output_text("create-first").context("first create")?)?;
            ensure!(v["phase"]=="succeeded","{v}");output.lock().unwrap().push(v);Ok(())
        }).reply(Reply::tool_call("create-retry","bot.sessions",input));
        let output=capture.clone();
        model.expect(RequestMatch::tool_output("create-retry")).after(&second).check(move|r|{
            let v:Value=serde_json::from_str(&r.tool_output_text("create-retry").context("retry create")?)?;
            ensure!(v["phase"]=="succeeded","{v}");output.lock().unwrap().push(v);Ok(())
        }).reply(Reply::text("created normal session"));
        bot.send("create-project-session").await?.wait_settled().await?.assert_succeeded()?;
        let values=capture.lock().unwrap().clone();
        let target=field(&values[0],"session_id").context("created session")?.as_i64().context("ID")?;
        ensure!(field(&values[1],"session_id")==Some(json!(target)));
        let normal=resource(&case,target).await?;
        ensure!(normal["workspace_id"]==workspace && normal["parent_id"].is_null() && normal["is_subagent"]==false);
        let listed=operation(&case,&bot,"list-project-sessions",json!({"action":"sessions","workspace_id":workspace})).await?;
        ensure!(listed.to_string().contains("Visible project work"));
        let work=model.expect(RequestMatch::new("project Bot instruction",|r| {
            let body=r.body.to_string();body.contains("Operator Bot instruction (Bot session") && body.contains("routed-project-write") && !body.contains("## Named operator Bot")
        })).check(|request| {let body=request.body.to_string();ensure!(!body.contains("For a direct request for substantial multi-step work"));Ok(())})
          .reply(Reply::tool_call("project-write","fs.write",json!({"path":"bot-result.txt","content":"routed-project-evidence","create_parents":true})));
        let work_done=model.expect(RequestMatch::tool_output("project-write")).after(&work)
            .check_tool_output_contains("project-write","succeeded").reply(Reply::text("project result ready"));
        let sent=operation(&case,&bot,"send-to-project",json!({"action":"send","session_id":target,"instruction":"routed-project-write: write bot-result.txt","_test_retry":true})).await?;
        ensure!(sent["phase"]=="succeeded","{sent}");
        work_done.finished().await?;
        case.wait_file_bytes("workspace/second-project/bot-result.txt",b"routed-project-evidence").await?;
        ensure!(!tokio::fs::try_exists(case.workspace().join("bot-result.txt")).await?);
        let user=case.user().attach_session(target).await?;
        model.expect(RequestMatch::user_text("human-direct-followup")).check(|r|{ensure!(r.body.to_string().contains("project result ready"));Ok(())})
            .reply(Reply::text("human continuation succeeded"));
        user.send("human-direct-followup").await?.wait_settled().await?.assert_succeeded()?;
        let continued=model.expect(RequestMatch::new("Bot continues human conversation",|r| {
            let body=r.body.to_string();body.contains("routed-project-followup") && body.contains("Operator Bot instruction (Bot session") && !body.contains("## Named operator Bot")
        })).check(|r|{ensure!(r.body.to_string().contains("human continuation succeeded"));Ok(())}).reply(Reply::text("Bot continuation succeeded"));
        let followup=operation(&case,&bot,"continue-project",json!({"action":"send","session_id":target,"instruction":"routed-project-followup"})).await?;
        ensure!(followup["phase"]=="succeeded");continued.finished().await?;
        user.wait_text_contains("Bot continuation succeeded").await?;
        let read=operation(&case,&bot,"read-project",json!({"action":"read","session_id":target,"limit":2})).await?;
        ensure!(read["phase"]=="succeeded" && field(&read,"url")==Some(json!(format!("/chat?sessionId={target}"))));
        ensure!(read.to_string().contains("Bot continuation succeeded"),"read omitted actual assistant text: {read}");
        ensure!(case.http().await.get("/api/v1/tasks").await?.status == 404);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_authority_rejects_ordinary_callers_self_targets_bad_quotes_and_conflicts() -> TestResult
{
    fixture().case_timeout(Duration::from_secs(120)).run(|case| async move {
        let ordinary=case.user().new_session().await?;
        let id=case.client().await.ensure_bot_session().await?.session_id;
        let bot=case.user().attach_session(id).await?;
        let denied=operation(&case,&ordinary,"not-an-operator",json!({"action":"projects"})).await?;
        ensure!(denied["phase"]=="failed" && denied.to_string().contains("active registered Bot"),"{denied}");
        let denied=operation(&case,&bot,"self-target",json!({"action":"send","session_id":id,"instruction":"loop"})).await?;
        // A Bot conversation is refused as a project target before any deeper
        // ownership check runs, because it has no project scope.
        ensure!(denied["phase"]=="failed" && denied.to_string().contains("not project targets"),"{denied}");
        let denied=operation(&case,&bot,"missing-target",json!({"action":"read","session_id":9223372036854775807_i64})).await?;
        ensure!(denied["phase"]=="failed");
        let denied=operation(&case,&bot,"malformed-cursor",json!({"action":"read","session_id":ordinary.id(),"cursor":"not a cursor"})).await?;
        ensure!(denied["phase"]=="failed");
        let workspace=resource(&case,ordinary.id()).await?["workspace_id"].as_i64().context("workspace")?;
        let model=case.model();
        let bad=model.expect(RequestMatch::user_text("real-user-quote")).reply(Reply::tool_call("bad-quote","bot.sessions",json!({"action":"create","workspace_id":workspace,"title":"forged","user_quote":"not-in-current-request","request_key":"forged"})));
        model.expect(RequestMatch::tool_output("bad-quote")).after(&bad).check_tool_output_contains("bad-quote","failed").reply(Reply::text("quotation rejected"));
        bot.send("real-user-quote").await?.wait_settled().await?.assert_succeeded()?;
        let before=resource(&case,ordinary.id()).await?;
        let version=before["version"].as_i64().context("version")?;
        let rename=operation(&case,&bot,"rename-project",json!({"action":"rename","session_id":ordinary.id(),"title":"renamed by Bot","expected_version":version})).await?;
        ensure!(rename["phase"]=="succeeded","{rename}");
        let stale=operation(&case,&bot,"stale-rename",json!({"action":"rename","session_id":ordinary.id(),"title":"must not overwrite","expected_version":version})).await?;
        ensure!(stale["phase"]=="failed");
        ensure!(resource(&case,ordinary.id()).await?["title"]=="renamed by Bot");
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_uses_target_queue_permissions_model_and_guarded_stop() -> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).run(|case| async move {
        let client=case.client().await;
        let bot=case.user().attach_session(client.ensure_bot_session().await?.session_id).await?;
        let target=case.user().new_session().await?;
        let model=case.model();
        // Give the ordinary target its own persisted model choice before routing.
        model.expect(RequestMatch::user_text("target-model-choice")).reply(Reply::text("selection recorded"));
        target.send("target-model-choice").await?.wait_settled().await?.assert_succeeded()?;
        let selection_before:Value=case.http().await.get(&format!("/api/v1/sessions/{}/state",target.id())).await?.require_success()?.json()?;
        let first=model.expect(RequestMatch::new("held routed execution",|r|{
            let body=r.body.to_string();body.contains("Operator Bot instruction (Bot session") && body.contains("queue-routed-first") && !body.contains("## Named operator Bot")
        })).reply(Reply::text("held-project-result").held());
        let sent=operation(&case,&bot,"start-held-project",json!({"action":"send","session_id":target.id(),"instruction":"queue-routed-first"})).await?;
        ensure!(sent["phase"]=="succeeded","{sent}");first.observed().await?;
        let status=operation(&case,&bot,"inspect-running-project",json!({"action":"status","session_id":target.id()})).await?;
        ensure!(status["phase"]=="succeeded" && status.to_string().contains("running"),"{status}");
        let execution=field(&sent,"execution_id").context("execution")?;
        let generation=field(&sent,"generation").context("generation")?;
        let stopped=operation(&case,&bot,"stop-observed-project",json!({"action":"stop","session_id":target.id(),"execution_id":execution,"expected_generation":generation})).await?;
        ensure!(stopped["phase"]=="succeeded","{stopped}");
        first.disconnected().await?;
        // A direct human continuation remains normal activity after a Bot stop.
        model.expect(RequestMatch::user_text("user-takes-over-after-stop")).reply(Reply::text("takeover accepted"));
        target.send("user-takes-over-after-stop").await?.wait_settled().await?.assert_succeeded()?;
        let selection_after:Value=case.http().await.get(&format!("/api/v1/sessions/{}/state",target.id())).await?.require_success()?.json()?;
        for key in ["model_provider_id","model_adapter_id","model_id","model_thinking_mode","model_speed_mode","model_verbosity"] {
            ensure!(selection_before["execution"][key]==selection_after["execution"][key],"Bot replaced target selection {key}");
        }
        let path=case.workspace().join("bot-denied.txt");
        super::path_permission(&target,&path,"deny").await?;
        let denied=model.expect(RequestMatch::new("routed permission test",|r|{
            let body=r.body.to_string();body.contains("Operator Bot instruction (Bot session") && body.contains("queue-routed-denied") && !body.contains("## Named operator Bot")
        })).reply(Reply::tool_call("routed-denied-write","fs.write",json!({"path":"bot-denied.txt","content":"forbidden"})));
        let done=model.expect(RequestMatch::tool_output("routed-denied-write")).after(&denied).check_tool_output_contains("routed-denied-write","failed").reply(Reply::text("target permission preserved"));
        let sent=operation(&case,&bot,"send-permission-test",json!({"action":"send","session_id":target.id(),"instruction":"queue-routed-denied"})).await?;
        ensure!(sent["phase"]=="succeeded");done.finished().await?;
        ensure!(!tokio::fs::try_exists(path).await?);
        Ok(())
    }).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bot_routed_project_uses_existing_subagents_and_subagents_cannot_manage_projects()
-> TestResult {
    fixture().case_timeout(Duration::from_secs(120)).run(|case| async move {
        let bot_id=case.client().await.ensure_bot_session().await?.session_id;
        let bot=case.user().attach_session(bot_id).await?;
        let target=case.user().new_session().await?;
        let model=case.model();
        let spawn=model.expect(RequestMatch::new("routed subagent request",|r| {
            let body=r.body.to_string();body.contains("Operator Bot instruction (Bot session") && body.contains("route-existing-subagent") && !body.contains("## Named operator Bot")
        })).reply(Reply::tool_call("project-spawn","agent.spawn",json!({"name":"bot_regression_child","prompt":"subagent-bot-work-marker"})));
        model.expect(RequestMatch::tool_output("project-spawn")).after(&spawn).check_tool_output_contains("project-spawn","succeeded").reply(Reply::text("project waiting for its Subagent"));
        let child=model.expect(RequestMatch::user_text("subagent-bot-work-marker"))
            .check(|r| {ensure!(!r.body.to_string().contains("## Named operator Bot"));Ok(())})
            .reply(Reply::tool_call("child-bot-denied","bot.sessions",json!({"action":"projects"})));
        let child_done=model.expect(RequestMatch::tool_output("child-bot-denied")).after(&child)
            .check_tool_output_contains("child-bot-denied","failed").reply(Reply::text("subagent-bot-evidence").held());
        let report=model.expect(RequestMatch::new("project receives existing Subagent result",|r|{
            let body=r.body.to_string();body.contains("subagent-bot-evidence") && body.contains("route-existing-subagent") && !body.contains("## Named operator Bot")
        })).reply(Reply::text("ordinary project received Subagent evidence"));
        let result=operation(&case,&bot,"route-subagent-work",json!({"action":"send","session_id":target.id(),"instruction":"route-existing-subagent"})).await?;
        ensure!(result["phase"]=="succeeded","{result}");
        child_done.observed().await?;
        let status=operation(&case,&bot,"inspect-project-subagent",json!({"action":"status","session_id":target.id()})).await?;
        let children=field(&status,"subagents").context("subagent list")?;
        let child_id=children[0]["session_id"].as_i64().context("normal Subagent ID")?;
        let denied=operation(&case,&bot,"deny-subagent-target",json!({"action":"read","session_id":child_id})).await?;
        ensure!(denied["phase"]=="failed");
        child_done.release();report.finished().await?;
        target.wait_text_contains("ordinary project received Subagent evidence").await?;
        ensure!(case.http().await.get("/api/v1/tasks").await?.status == 404);
        Ok(())
    }).await
}
