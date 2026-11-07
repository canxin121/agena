use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::oneshot;

use super::{client, post_json, request};
use crate::model::{Reply, RequestMatch, ScriptedModel};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn injected_disconnect_is_a_network_error_before_body_and_midstream_and_is_scoped()
-> Result<()> {
    for sent_prefix in [false, true] {
        let owner = ScriptedModel::start().await?;
        let model = owner.handle();
        let victim = model.expect(RequestMatch::user_text("victim")).reply(
            Reply::raw_stream(
                200,
                [
                    "data: {\"prefix\":\"visible\"}\n\n",
                    "data: {\"terminal\":\"must not arrive\"}\n\n",
                ],
            )
            .hold_after_chunks(usize::from(sent_prefix)),
        );
        let survivor = model
            .expect(RequestMatch::user_text("survivor"))
            .reply(Reply::text("survives the unrelated socket cut").held());
        let url = format!("{}/responses", owner.base_url());
        let http = client()?;
        let survivor_http = http.clone();
        let survivor_url = url.clone();
        let survivor_task = tokio::spawn(async move {
            post_json(&survivor_http, &survivor_url, request("survivor", false)).await
        });
        survivor.observed().await?;

        let (first_tx, first_rx) = oneshot::channel();
        let victim_task = tokio::spawn(async move {
            let response = http.post(url).json(&request("victim", true)).send().await?;
            let mut chunks = response.bytes_stream();
            let mut first = Some(first_tx);
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk?;
                if let Some(first) = first.take() {
                    let _ = first.send(chunk);
                }
            }
            Ok::<(), reqwest::Error>(())
        });
        victim.observed().await?;
        if sent_prefix {
            let prefix = first_rx
                .await
                .context("first body bytes before disconnection")?;
            ensure!(std::str::from_utf8(&prefix)?.contains("visible"));
        }
        victim.disconnect().await?;
        // Repeated injection is idempotent after an observed forced disconnect.
        victim.disconnect().await?;
        let error = victim_task
            .await?
            .expect_err("socket cut must not look like a normal HTTP EOF");
        ensure!(
            !error.is_timeout(),
            "injection must cut the socket, not wait for an HTTP deadline"
        );
        assert!(victim.finished().await.is_err());
        assert!(
            !survivor_task.is_finished(),
            "unrelated held request must remain connected"
        );
        survivor.release();
        let response = survivor_task.await??;
        assert_eq!(
            response["output"][0]["content"][0]["text"],
            "survives the unrelated socket cut"
        );
        survivor.finished().await?;
        model.assert_finished()?;
        owner.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn completed_step_cannot_disconnect_a_later_reused_connection() -> Result<()> {
    let owner = ScriptedModel::start().await?;
    let model = owner.handle();
    let first = model
        .expect(RequestMatch::user_text("first"))
        .reply(Reply::text("done"));
    let second = model
        .expect(RequestMatch::user_text("second"))
        .after(&first)
        .reply(Reply::text("still connected").held());
    let http = client()?;
    let url = format!("{}/responses", owner.base_url());
    let _: Value = post_json(&http, &url, request("first", false)).await?;
    first.finished().await?;
    let pending =
        tokio::spawn(async move { post_json(&http, &url, request("second", false)).await });
    second.observed().await?;
    assert!(
        first
            .disconnect()
            .await
            .unwrap_err()
            .to_string()
            .contains("already finished")
    );
    second.release();
    let response = pending.await??;
    assert_eq!(
        response["output"][0]["content"][0]["text"],
        "still connected"
    );
    second.finished().await?;
    model.assert_finished()?;
    owner.shutdown().await
}
