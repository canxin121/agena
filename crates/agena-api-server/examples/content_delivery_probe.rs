//! Measure the production application/SSE/client delivery path on loopback.
//! Native text positions count bytes; logical append markers survive physical
//! block coalescing and arbitrary network page boundaries.
use agena_api_server::{AppState, router};
use agena_application::Application;
use agena_client::AgenaClient;
use agena_domain::{ComposerDocument, ComposerNode, ContentCursor, ContentPayload};
use agena_runtime::{RuntimeBootstrapRequest, bootstrap_application_services};
use agena_storage::kernel::{ExecutionPhase, RequestId};
use serde_json::json;
use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    for observers in [1, 8] {
        measure(observers).await?;
    }
    Ok(())
}
async fn measure(observers: usize) -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let runtime = bootstrap_application_services(RuntimeBootstrapRequest {
        workspace_root: Some(directory.path().to_path_buf()),
        config_path: Some(directory.path().join("global.json")),
        database_path: Some(directory.path().join("sessions.db")),
        ..Default::default()
    })
    .await?;
    let services = runtime.application_services();
    let workspace = services
        .repositories
        .as_ref()
        .expect("repositories")
        .workspace
        .ensure_id(directory.path().to_str().expect("UTF-8 fixture path"))
        .await?;
    let kernel = services.session_kernel.clone().expect("session kernel");
    let application = Application::from_composed_runtime_services(services)?;
    let session = kernel
        .sessions
        .create(RequestId::new(), workspace, "SSE delivery probe".into())
        .await?
        .value
        .id;
    let sent = kernel
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "isolated SSE delivery measurement".into(),
            }]),
        )
        .await?
        .value;
    let attempt = kernel
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await?
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![sent.message_id])
        .await?
        .value;
    let writer = round.begin_text(RequestId::new(), false).await?.value;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = AgenaClient::new(format!("http://{}", listener.local_addr()?).as_str())?;
    let server = tokio::spawn(async move {
        axum::serve(listener, router(AppState::from_application(application))).await
    });
    let markers = Arc::new(Mutex::new(Vec::<(u64, Instant)>::new()));
    let mut expected = blake3::Hasher::new();
    for index in 0..100 {
        expected.update(format!("{index:03} {}\n", "x".repeat(1019)).as_bytes());
    }
    let expected = expected.finalize();
    let mut readers = Vec::new();
    for _ in 0..observers {
        let mut stream = client
            .stream_content(
                session.get(),
                writer.id(),
                Some(ContentCursor {
                    epoch: writer.epoch().0,
                    sequence: 0,
                }),
                64 * 1024,
            )
            .await?;
        let markers = markers.clone();
        readers.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            let mut final_cursor = 0;
            let mut sampled = 0usize;
            let mut checksum = blake3::Hasher::new();
            let mut received_bytes = 0_usize;
            let mut pages = 0_usize;
            while let Some(page) = stream.recv().await {
                let page = page.expect("SSE page");
                assert!(!page.gap);
                assert_eq!(page.resource.dropped_bytes, 0);
                for chunk in &page.chunks {
                    let ContentPayload::Text { text } = &chunk.payload else {
                        panic!("text delivery changed its payload kind");
                    };
                    checksum.update(text.as_bytes());
                    received_bytes += text.len();
                    assert_eq!(chunk.cursor.sequence, received_bytes as u64);
                }
                assert_eq!(page.next_cursor.sequence, received_bytes as u64);
                pages += 1;
                let now = Instant::now();
                final_cursor = page.next_cursor.sequence;
                let markers = markers.lock().unwrap();
                while sampled < markers.len() && markers[sampled].0 <= final_cursor {
                    latencies.push(now.duration_since(markers[sampled].1).as_secs_f64() * 1000.0);
                    sampled += 1;
                }
            }
            assert_eq!(final_cursor, 100 * 1024);
            assert_eq!(sampled, 100);
            assert_eq!(received_bytes, 100 * 1024);
            assert_eq!(checksum.finalize(), expected);
            (
                latencies,
                json!({"received_bytes":received_bytes,"blake3":checksum.finalize().to_hex().as_str(),"pages":pages,"final_cursor":final_cursor,"marker_samples":sampled,"client_visible_busy_errors":0,"client_reconnections":0}),
            )
        }));
    }
    let mut byte_end = 0u64;
    for index in 0..100 {
        let text = format!("{index:03} {}\n", "x".repeat(1019));
        byte_end += text.len() as u64;
        markers.lock().unwrap().push((byte_end, Instant::now()));
        writer.append(&text).await?;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    writer.finish(RequestId::new()).await?;
    let mut latencies = Vec::new();
    let mut integrity = Vec::new();
    for reader in readers {
        let (samples, verification) =
            tokio::time::timeout(Duration::from_secs(10), reader).await??;
        latencies.extend(samples);
        integrity.push(verification);
    }
    round.finish(RequestId::new(), None, true).await?;
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await?;
    latencies.sort_unstable_by(f64::total_cmp);
    println!(
        "{}",
        json!({"scope":"TextOutput / application delivery / production loopback SSE / Rust client","observers":observers,"logical_appends":100,"source_bytes":byte_end,"samples":latencies.len(),"delivery_p50_ms":latencies[latencies.len()/2],"delivery_p95_ms":latencies[latencies.len()*95/100],"delivery_max_ms":latencies[latencies.len()-1],"observer_integrity":integrity,"client_visible_busy_errors":0,"client_reconnections":0,"server_internal_busy_retries":null})
    );
    server.abort();
    let _ = server.await;
    runtime.shutdown();
    Ok(())
}
