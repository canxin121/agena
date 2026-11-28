use super::*;
use agena_testkit::{Reply, RequestMatch};

#[derive(Default)]
struct OutcomePeer {
    outcomes: Mutex<Vec<Value>>,
}

async fn memvault_rpc(
    State(peer): State<Arc<OutcomePeer>>,
    Json(request): Json<Value>,
) -> Response {
    let method = request["method"].as_str().unwrap_or_default();
    if request.get("id").is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "memvault-outcome-fixture", "version": "1.0"}
        }),
        "tools/list" => json!({"tools":[{
            "name": "infer_outcome",
            "description": "Record an observed session outcome",
            "inputSchema": {"type": "object"}
        }]}),
        "tools/call" if request["params"]["name"] == "infer_outcome" => {
            peer.outcomes
                .lock()
                .unwrap()
                .push(request["params"]["arguments"].clone());
            json!({"content":[{"type":"text","text":"recorded"}]})
        }
        _ => {
            return Json(json!({
                "jsonrpc": "2.0",
                "id": request["id"],
                "error": {"code": -32601, "message": "Unsupported fixture method"},
            }))
            .into_response();
        }
    };
    Json(json!({"jsonrpc": "2.0", "id": request["id"], "result": result})).into_response()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_deepseek_memvault_infers_the_durable_session_outcome_once() -> TestResult {
    const OPENING: &str = "Wait, prepare the release checklist.";
    const CORRECTION: &str = "No, keep the previous release target.";
    const FINAL_TEXT: &str = "The previous release target is retained.";
    let peer = Arc::new(OutcomePeer::default());
    let server_peer = Arc::clone(&peer);

    fixture_without_memory()
        .env("AGENA_SANDBOX", "disabled")
        .env_relative("PATH", "empty-bin")
        .expect_plugin("deepseek.fixture-memvault-outcome")
        .builtin_plugin(
            "agena.mcp",
            json!({"runtime":{"reconnect":{"enabled":false}}}),
        )
        .before_start(move |case| async move {
            let server = case
                .serve_http_on(
                    native_fixture_address()?,
                    Router::new()
                        .route("/mcp", post(memvault_rpc))
                        .with_state(server_peer),
                )
                .await?;
            let package = case.paths().root.join("memvault");
            tokio::fs::create_dir_all(&package).await?;
            tokio::fs::create_dir_all(case.paths().root.join("empty-bin")).await?;
            tokio::fs::write(
                package.join("package.json"),
                br#"{"name":"@fixture/memvault-outcome","dsh":{"bundle":{"patch":"bundle.yml"}}}"#,
            )
            .await?;
            tokio::fs::write(
                package.join("bundle.yml"),
                serde_json::to_vec(&json!([{"insert":[{
                    "name":"@dreamor/dsh-memvault",
                    "config":{
                        "mode":"attach",
                        "url":server.url("/mcp"),
                        "agentId":"fixture-release-agent",
                        "injectOnAssemble":false,
                        "extractOnTurnEnd":false,
                        "inferOutcomeOnSessionEnd":true
                    }
                }]}]))?,
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
            let session = case.user().new_session().await?;
            allow_permissions(&session).await?;
            let model = case.model();
            model
                .expect(RequestMatch::user_text("Earlier task to clear"))
                .reply(Reply::text("Earlier task complete"));
            session
                .send("Earlier task to clear")
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            session.client().clear_session(session.id(), true).await?;
            ensure!(peer.outcomes.lock().unwrap().is_empty());

            let first = model
                .expect(RequestMatch::user_text(OPENING))
                .reply(Reply::tool_call(
                    "outcome-missing-file",
                    "fs.read",
                    json!({"file_path":"missing.txt"}),
                ));
            let second = model
                .expect(RequestMatch::tool_output("outcome-missing-file"))
                .after(&first)
                .reply(Reply::tool_call(
                    "outcome-retry-file",
                    "fs.read",
                    json!({"file_path":"still-missing.txt"}),
                ));
            model
                .expect(RequestMatch::tool_output("outcome-retry-file"))
                .after(&second)
                .reply(Reply::text("The checklist is drafted."));
            session
                .send(OPENING)
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            let id = session.id();
            case.restart().await?;
            let session = case.user().attach_session(id).await?;
            case.model()
                .expect(RequestMatch::user_text(CORRECTION))
                .reply(Reply::text(FINAL_TEXT));
            session
                .send(CORRECTION)
                .await?
                .wait_settled()
                .await?
                .assert_succeeded()?;
            ensure!(peer.outcomes.lock().unwrap().is_empty());

            let http = case.http().await;
            let route = format!("/api/v1/sessions/{id}");
            let first = http.request(reqwest::Method::DELETE, &route)?;
            let second = http.request(reqwest::Method::DELETE, &route)?;
            let (first, second) = tokio::join!(http.execute(first), http.execute(second));
            let results = [first?, second?];
            ensure!(
                results
                    .iter()
                    .filter(|result| result.status.is_success())
                    .count()
                    == 1,
                "{results:?}"
            );
            let outcomes = peer.outcomes.lock().unwrap();
            ensure!(outcomes.len() == 1, "{outcomes:?}");
            let record = &outcomes[0];
            ensure!(record["agent_id"] == "fixture-release-agent", "{record}");
            ensure!(
                record["evidence"]
                    == json!({
                        "task": OPENING,
                        "retry_hits": 1,
                        "rejection_hits": 0,
                        "correction_hits": 1,
                        "terminal": "resolved",
                    }),
                "{record}"
            );
            let context = record["judge_context"].as_str().context("judge context")?;
            ensure!(
                context.contains(CORRECTION) && context.contains(FINAL_TEXT),
                "{context}"
            );
            ensure!(!context.contains("Earlier task"), "{context}");
            Ok(())
        })
        .await
}
