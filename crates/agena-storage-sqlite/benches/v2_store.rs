//! Runnable production-SQLite microbenchmarks required by P7.
//!
//! This intentionally uses a tiny custom harness instead of adding a benchmark
//! framework dependency. It reports wall-clock latency for the three mandated
//! v2 paths: warm facade reads, indexed usage aggregation, and D10 bounded
//! streaming (including the run-end tail flush).

use std::error::Error;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agena_domain::SessionRelationKind;
use agena_storage::WorkspaceRepository;
use agena_storage::store::{
    NewPart, NewSession, PartDelta, PartRole, PartState, PartVisibility, RunOutcome, SessionFacade,
    SessionStore, UsageQuery, UsageRecord,
};
use agena_storage_sqlite::{SeaWorkspaceRepository, SqliteEngine, initialize_schema};
use sea_orm::Database;
use serde_json::json;

const READ_ITERATIONS: usize = 1_000;
const USAGE_ROWS: usize = 2_000;
const USAGE_ITERATIONS: usize = 200;
const STREAM_SAMPLES: usize = 25;
const STREAM_DELTAS: usize = 65;
const STREAM_FLUSH_THRESHOLD: usize = 8;

fn main() -> Result<(), Box<dyn Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let database_path = directory.path().join("v2-store-bench.db");
    let db = Arc::new(
        Database::connect(format!("sqlite://{}?mode=rwc", database_path.display())).await?,
    );
    initialize_schema(&db).await?;
    let workspace_id = SeaWorkspaceRepository::new(Arc::clone(&db))
        .ensure_id("/bench/workspace")
        .await?;
    let facade = SessionFacade::new(SqliteEngine::new(db), "bench-owner", 32)
        .with_streaming_flush_delta_count(STREAM_FLUSH_THRESHOLD);
    let session_id = facade
        .create_session(NewSession {
            workspace_id,
            parent_id: None,
            relation_kind: SessionRelationKind::Root,
            cutoff_part_id: None,
            title: "v2 store benchmark".to_owned(),
            task_id: None,
            config_json: None,
            provider_anchors_json: None,
        })
        .await?
        .id;

    seed_read_session(&facade, session_id).await?;
    benchmark_warm_read(&facade, session_id).await?;
    seed_usage(&facade, workspace_id, session_id).await?;
    benchmark_indexed_usage(&facade, workspace_id).await?;
    benchmark_streaming(&facade, session_id).await?;
    Ok(())
}

async fn seed_read_session(
    facade: &SessionFacade<SqliteEngine>,
    session_id: i64,
) -> Result<(), Box<dyn Error>> {
    let parts = (0..256)
        .map(|index| NewPart {
            kind: "text".to_owned(),
            role: PartRole::User,
            content: json!({"text": format!("seed part {index}")}),
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            state: PartState::Completed,
        })
        .collect();
    let run_id = facade
        .submit_user_message(session_id, "bench-owner", parts, None)
        .await?
        .run_id;
    facade
        .complete_run(
            session_id,
            "bench-owner",
            run_id,
            RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
        )
        .await?;
    // Populate and validate the facade cache before measuring the warm path.
    assert_eq!(facade.load(session_id).await?.parts.len(), 257);
    Ok(())
}

async fn benchmark_warm_read(
    facade: &SessionFacade<SqliteEngine>,
    session_id: i64,
) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    for _ in 0..READ_ITERATIONS {
        black_box(facade.load(session_id).await?);
    }
    report("read/warm_facade", started.elapsed(), READ_ITERATIONS);
    Ok(())
}

async fn seed_usage(
    facade: &SessionFacade<SqliteEngine>,
    workspace_id: i64,
    session_id: i64,
) -> Result<(), Box<dyn Error>> {
    for index in 0..USAGE_ROWS {
        facade
            .record_usage(UsageRecord {
                workspace_id,
                session_id,
                run_id: None,
                provider_id: if index % 2 == 0 {
                    "anthropic".to_owned()
                } else {
                    "openai".to_owned()
                },
                model_id: if index % 3 == 0 {
                    "large".to_owned()
                } else {
                    "small".to_owned()
                },
                created_at_ms: 1_000_000 + index as i64,
                input_tokens: 100,
                output_tokens: 40,
                reasoning_tokens: 10,
                cache_write_tokens: 0,
                cache_read_tokens: 5,
                tool_use_tokens: 0,
                other_tokens: 0,
                total_cost_micros: 250,
                recorded_cost_micros: None,
                cost_estimate_incomplete: false,
                detail_json: None,
            })
            .await?;
    }
    Ok(())
}

async fn benchmark_indexed_usage(
    facade: &SessionFacade<SqliteEngine>,
    workspace_id: i64,
) -> Result<(), Box<dyn Error>> {
    let query = UsageQuery {
        workspace_id: Some(workspace_id),
        after_ms: Some(1_000_250),
        before_ms: Some(1_001_750),
        ..Default::default()
    };
    let expected_calls = facade.usage_stats(query.clone()).await?.total_calls;
    assert_eq!(expected_calls, 1_500);
    let started = Instant::now();
    for _ in 0..USAGE_ITERATIONS {
        let stats = facade.usage_stats(query.clone()).await?;
        black_box(stats);
    }
    report(
        "usage/indexed_workspace_range",
        started.elapsed(),
        USAGE_ITERATIONS,
    );
    Ok(())
}

async fn benchmark_streaming(
    facade: &SessionFacade<SqliteEngine>,
    session_id: i64,
) -> Result<(), Box<dyn Error>> {
    let mut measured = Duration::ZERO;
    for _ in 0..STREAM_SAMPLES {
        let run_id = facade
            .submit_user_message(
                session_id,
                "bench-owner",
                vec![NewPart {
                    kind: "text".to_owned(),
                    role: PartRole::Assistant,
                    content: json!({"text": ""}),
                    summary: None,
                    visibility: PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::InProgress,
                }],
                None,
            )
            .await?
            .run_id;
        let streamed_part_id = facade
            .load(session_id)
            .await?
            .parts
            .into_iter()
            .find(|part| part.run_id == Some(run_id))
            .expect("streamed part")
            .part_id;

        let started = Instant::now();
        for _ in 0..STREAM_DELTAS {
            facade
                .update_part(
                    session_id,
                    "bench-owner",
                    streamed_part_id,
                    PartDelta {
                        content_text_delta: Some("x".to_owned()),
                        ..Default::default()
                    },
                )
                .await?;
        }
        facade
            .complete_run(
                session_id,
                "bench-owner",
                run_id,
                RunOutcome {
                    status: PartState::Completed,
                    abort_reason: None,
                    content: None,
                    provider_state: None,
                },
            )
            .await?;
        measured += started.elapsed();

        let persisted = facade
            .load(session_id)
            .await?
            .parts
            .into_iter()
            .find(|part| part.part_id == streamed_part_id)
            .expect("persisted streamed part");
        assert_eq!(
            persisted.content["text"].as_str().unwrap().len(),
            STREAM_DELTAS
        );
        assert_eq!(
            persisted.revision,
            1 + STREAM_DELTAS.div_ceil(STREAM_FLUSH_THRESHOLD) as i64,
            "D10 persists one part revision per bounded checkpoint"
        );
    }
    report(
        "streaming/65_deltas_plus_completion",
        measured,
        STREAM_SAMPLES,
    );
    Ok(())
}

fn report(name: &str, elapsed: Duration, iterations: usize) {
    let nanos_per_iteration = elapsed.as_nanos() / iterations as u128;
    println!(
        "v2_store/{name}: {nanos_per_iteration} ns/op ({iterations} iterations, {:.3?} total)",
        elapsed
    );
}
