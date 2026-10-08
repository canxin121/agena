//! Measure the production application/SSE/client delivery path on loopback.
//! SQL accounting is measured separately by content_stream_probe.
use agena_api_server::{AppState, router};
use agena_application::Application;
use agena_client::AgenaClient;
use agena_domain::{ContentCursor, ContentState, SessionRelationKind};
use agena_runtime::{RuntimeBootstrapRequest, bootstrap_application_services};
use agena_storage::store::{NewPart, NewSession, PartDelta, PartRole, PartState};
use serde_json::json;
use std::error::Error;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
        scheduler_database_path: Some(directory.path().join("scheduler.db")),
        initialize_schema: true,
        ..Default::default()
    })
    .await?;
    let services = runtime.application_services();
    let store = services
        .session_store
        .as_ref()
        .expect("session store")
        .clone();
    let workspace_id = services
        .repositories
        .as_ref()
        .expect("repositories")
        .workspace
        .ensure_id(directory.path().to_str().expect("UTF-8 fixture path"))
        .await?;
    let application = Application::from_composed_runtime_services(services)?;
    let session = store
        .create_session(NewSession {
            workspace_id,
            parent_id: None,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "SSE delivery probe".into(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await?;
    let run = store
        .submit_user_run(
            session.id,
            vec![NewPart {
                state: PartState::InProgress,
                ..NewPart::pending("text", PartRole::Assistant, json!({"text":""}))
            }],
            None,
        )
        .await?;
    let part = &run.parts[1];
    let writer = store
        .contents()
        .open(session.id, part.part_id, agena_domain::ContentKind::Text)
        .await?;
    let reference = writer.resource().reference();
    store
        .update_part(
            session.id,
            part.part_id,
            PartDelta {
                content: Some(json!({"text":"","resources":[reference]})),
                ..Default::default()
            },
        )
        .await?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = AgenaClient::new(format!("http://{}", listener.local_addr()?).as_str())?;
    let server = tokio::spawn(async move {
        axum::serve(listener, router(AppState::from_application(application))).await
    });
    let starts = Arc::new(Mutex::new(Vec::<Instant>::new()));
    let mut readers = Vec::new();
    for _ in 0..observers {
        let mut stream = client
            .stream_content(
                session.id,
                reference.resource_id,
                Some(ContentCursor {
                    epoch: writer.resource().cursor.epoch,
                    sequence: 0,
                }),
                64 * 1024,
            )
            .await?;
        let starts = starts.clone();
        readers.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            let mut final_cursor = 0;
            while let Some(page) = stream.recv().await {
                let page = page.expect("SSE page");
                assert!(!page.gap);
                let now = Instant::now();
                for chunk in &page.chunks {
                    latencies.push(
                        now.duration_since(
                            starts.lock().unwrap()[chunk.cursor.sequence as usize - 1],
                        )
                        .as_secs_f64()
                            * 1000.0,
                    );
                }
                final_cursor = page.next_cursor.sequence;
            }
            assert_eq!(final_cursor, 100);
            assert_eq!(latencies.len(), 100);
            latencies
        }));
    }
    for index in 0..100 {
        starts.lock().unwrap().push(Instant::now());
        writer
            .append_text(&format!("{index:03} {}\n", "x".repeat(1019)))
            .await?;
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    writer.finalize(ContentState::Complete).await?;
    let mut latencies = Vec::new();
    for reader in readers {
        latencies.extend(tokio::time::timeout(Duration::from_secs(10), reader).await??);
    }
    latencies.sort_unstable_by(f64::total_cmp);
    println!(
        "{}",
        json!({
            "scope":"ContentWriter / application delivery / production loopback SSE / Rust client",
            "observers":observers,"source_records":100,"samples":latencies.len(),
            "delivery_p50_ms":latencies[latencies.len()/2],
            "delivery_p95_ms":latencies[latencies.len()*95/100],
            "delivery_max_ms":latencies[latencies.len()-1]
        })
    );
    server.abort();
    Ok(())
}
