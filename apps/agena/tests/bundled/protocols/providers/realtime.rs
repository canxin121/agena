use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_testkit::{FixtureWebSocketUpgrade, TestResult};
use anyhow::{Context, Result, ensure};
use axum::{
    Router,
    extract::{
        OriginalUri, State,
        ws::{Message, WebSocket},
    },
    http::HeaderMap,
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{API_KEY, Vendor, peer::*};

#[derive(Clone, Copy)]
enum Finish {
    Completed,
    Failed,
    Deadline,
}
#[derive(Clone, Copy)]
struct Plan {
    vendor: Vendor,
    finish: Finish,
}
#[derive(Default)]
struct WsData {
    plans: VecDeque<Plan>,
    finished: Vec<Result<(), String>>,
    active: usize,
}
#[derive(Clone, Default)]
struct WsPeer {
    data: Arc<Mutex<WsData>>,
    changed: Arc<Notify>,
}
impl WsPeer {
    fn complete(&self, result: Result<(), String>) {
        let mut data = self.data.lock().unwrap();
        data.active -= 1;
        data.finished.push(result);
        drop(data);
        self.changed.notify_waiters();
    }
    fn expect(&self, vendor: Vendor, finish: Finish) {
        self.data
            .lock()
            .unwrap()
            .plans
            .push_back(Plan { vendor, finish });
    }
    fn router(&self) -> Router {
        Router::new()
            .route("/v1/realtime", get(upgrade))
            .route(
                "/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent",
                get(upgrade),
            )
            .with_state(self.clone())
    }
    async fn finished(&self, count: usize) -> TestResult {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let notified = self.changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                {
                    let data = self.data.lock().unwrap();
                    for result in &data.finished {
                        if let Err(error) = result {
                            anyhow::bail!("{error}");
                        }
                    }
                    if data.finished.len() == count {
                        ensure!(
                            data.active == 0 && data.plans.is_empty(),
                            "unclosed websocket peer"
                        );
                        return Ok(());
                    }
                }
                notified.await;
            }
        })
        .await
        .context("websocket peer did not close")?
    }
}
async fn upgrade(
    State(peer): State<WsPeer>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    ws: FixtureWebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket, cancel| async move {
        let plan = {
            let mut data = peer.data.lock().unwrap();
            data.active += 1;
            data.plans.pop_front()
        };
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(anyhow::anyhow!(
                "WebSocket fixture stopped before its expected exchange completed"
            )),
            result = tokio::time::timeout(Duration::from_secs(8), async move {
                let plan = plan.context("unexpected WebSocket connection")?;
                exchange(socket, plan, uri, headers).await
            }) => result.context("WebSocket exchange timed out").and_then(|result| result),
        };
        peer.complete(
            result
                .as_ref()
                .map(|_| ())
                .map_err(|error| format!("{error:#}")),
        );
        result
    })
}
async fn receive(socket: &mut WebSocket) -> Result<Value> {
    let message = socket
        .next()
        .await
        .context("peer closed before expected frame")??;
    Ok(serde_json::from_str(message.to_text()?)?)
}
async fn send(socket: &mut WebSocket, value: Value) -> Result<()> {
    socket.send(Message::Text(value.to_string().into())).await?;
    Ok(())
}
async fn exchange(
    mut socket: WebSocket,
    plan: Plan,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Result<()> {
    let gemini = matches!(plan.vendor, Vendor::Gemini);
    ensure!(
        headers
            .get("x-agena-provider-api")
            .context("missing vendor protocol header")?
            == plan.vendor.name()
    );
    let expected = if gemini {
        API_KEY.to_owned()
    } else {
        format!("Bearer {API_KEY}")
    };
    ensure!(
        headers
            .get(plan.vendor.auth_header())
            .and_then(|v| v.to_str().ok())
            == Some(expected.as_str())
    );
    ensure!(headers.get("x-session-id").is_some());
    if !gemini {
        ensure!(uri.path() == "/v1/realtime" && uri.query() == Some("model=voice-model"));
    }
    let setup = receive(&mut socket).await?;
    ensure!(
        if gemini {
            setup["setup"]["model"] == "models/voice-model"
        } else {
            setup["type"] == "session.update" && setup["session"]["model"] == "voice-model"
        },
        "bad setup: {setup}"
    );
    // Hold the protocol acknowledgment. Input arriving in this interval would
    // prove the client submitted paid work before the provider accepted setup.
    ensure!(
        tokio::time::timeout(Duration::from_millis(40), socket.next())
            .await
            .is_err(),
        "input was sent before setup acknowledgment"
    );
    send(
        &mut socket,
        if gemini {
            json!({"setupComplete":{}})
        } else {
            json!({"type":"session.updated"})
        },
    )
    .await?;
    let input = receive(&mut socket).await?;
    if gemini {
        ensure!(input["clientContent"]["turnComplete"] == true);
        ensure!(input["clientContent"]["turns"][0]["parts"][0]["text"] == "hi");
        send(
            &mut socket,
            json!({"serverContent":{"modelTurn":{"parts":[
            {"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"AQIDBA=="}}]},
            "outputTranscription":{"text":"hello-voice"},"turnComplete":true},
            "usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":8}}),
        )
        .await?;
    } else {
        ensure!(input["type"] == "conversation.item.create");
        ensure!(receive(&mut socket).await?["type"] == "response.create");
        send(
            &mut socket,
            json!({"type":"response.output_audio.delta","delta":"AQIDBA=="}),
        )
        .await?;
        send(
            &mut socket,
            json!({"type":"response.output_audio_transcript.delta","delta":"hello-voice"}),
        )
        .await?;
        match plan.finish {
            Finish::Completed => {
                send(
                    &mut socket,
                    json!({"type":"response.done","response":{
                "status":"completed","usage":{"input_tokens":5,"output_tokens":8}}}),
                )
                .await?
            }
            Finish::Failed => {
                send(
                    &mut socket,
                    json!({"type":"response.done","response":{"status":"failed"}}),
                )
                .await?
            }
            Finish::Deadline => {}
        }
    }
    // The tool owns termination, including its configured deadline; the fixture
    // waits for close/EOF and never leaves an unobserved upgraded task behind.
    match socket.next().await {
        None | Some(Ok(Message::Close(_))) => {}
        Some(Err(error)) => return Err(error.into()),
        Some(Ok(_)) => anyhow::bail!("unexpected frame after voice turn"),
    }
    let _ = socket.close().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn realtime_waits_for_setup_and_persists_complete_audio_and_usage() -> TestResult {
    let peer = Peer::default();
    let websocket = WsPeer::default();
    scripted_routes(&peer, websocket.router())
        .run(move |case| async move {
            let user = session(&case).await?;
            for (index, vendor) in [Vendor::ChatGpt, Vendor::Gemini].into_iter().enumerate() {
                websocket.expect(vendor, Finish::Completed);
                let result = user
                    .call_tool(
                        format!("{}.realtime", vendor.name()),
                        json!({"model":"voice-model","text":"hi","duration_seconds":3}),
                    )
                    .await?;
                websocket.finished(index + 1).await?;
                result.assert_succeeded()?;
                ensure!(
                    result.data()?["completed"] == true
                        && result.data()?["input_submitted"] == true,
                    "voice turn did not complete after setup: {}",
                    result.data()?
                );
                ensure!(result.data()?.to_string().contains("hello-voice"));
                ensure!(
                    !result.output.to_string().contains("AQIDBA==")
                        && !result.output.to_string().contains(API_KEY)
                );
                let wav = attachment_bytes(&user, &result, 0, "audio/wav").await?;
                ensure!(wav.starts_with(b"RIFF") && wav.get(44..) == Some([1, 2, 3, 4].as_slice()));
            }
            let totals = user
                .client()
                .usage_stats(
                    agena_domain::UsagePeriod::AllTime,
                    None,
                    None,
                    &["testkit".into()],
                    &["voice-model".into()],
                    &[user.id()],
                    true,
                    0,
                )
                .await?
                .totals;
            ensure!(
                totals.input_tokens == 10 && totals.output_tokens == 16 && totals.requests == 2,
                "{totals:?}"
            );
            peer.assert_finished()
        })
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_and_timed_out_voice_turns_keep_partial_audio_without_claiming_completion()
-> TestResult {
    let peer = Peer::default();
    let websocket = WsPeer::default();
    scripted_routes(&peer, websocket.router())
        .run(move |case| async move {
            let user = session(&case).await?;
            for (index, finish) in [Finish::Failed, Finish::Deadline].into_iter().enumerate() {
                websocket.expect(Vendor::ChatGpt, finish);
                let result = user
                    .call_tool(
                        "chatgpt.realtime",
                        json!({"model":"voice-model","text":"hi","duration_seconds":1}),
                    )
                    .await?;
                websocket.finished(index + 1).await?;
                result.assert_succeeded()?;
                ensure!(
                    result.data()?["completed"] == false
                        && result.data()?["input_submitted"] == true,
                    "partial voice turn lost its submitted-input state: {}",
                    result.data()?
                );
                let warning = result.data()?["warnings"].to_string();
                ensure!(
                    warning.contains(match finish {
                        Finish::Failed => "without successful completion",
                        Finish::Deadline => "deadline reached",
                        Finish::Completed => unreachable!(),
                    }),
                    "{warning}"
                );
                let wav = attachment_bytes(&user, &result, 0, "audio/wav").await?;
                ensure!(wav.starts_with(b"RIFF") && wav.get(44..) == Some([1, 2, 3, 4].as_slice()));
            }
            peer.assert_finished()
        })
        .await
}
