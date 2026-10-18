//! Production-kernel microbenchmarks. Every fixture owns a temporary database.
use agena_domain::{ComposerDocument, ComposerNode};
use agena_storage::kernel::{ExecutionPhase, RequestId};
use agena_storage_sqlite::open_kernel_runtime;
use std::{error::Error, hint::black_box, time::Instant};
const READ_ITERATIONS: usize = 1_000;
fn input(text: impl Into<String>) -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text { text: text.into() }])
}
fn main() -> Result<(), Box<dyn Error>> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}
async fn run() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("kernel-bench.db");
    let runtime = open_kernel_runtime(&format!("sqlite://{}?mode=rwc", path.display())).await?;
    let workspace = runtime
        .workspaces
        .create(directory.path().to_string_lossy().into_owned())
        .await?;
    let ports = &runtime.ports;
    let session = ports
        .sessions
        .create(RequestId::new(), workspace.id, "kernel benchmark".into())
        .await?
        .value
        .id;
    for index in 0..256 {
        let execution = ports
            .sessions
            .send(
                RequestId::new(),
                session,
                input(format!("seed input {index}")),
            )
            .await?
            .value;
        ports
            .sessions
            .cancel(RequestId::new(), session, execution.id)
            .await?;
    }
    assert_eq!(ports.reader.summary(session).await?.user_message_count, 256);
    let started = Instant::now();
    for _ in 0..READ_ITERATIONS {
        black_box(ports.reader.history(session, None, 32).await?);
    }
    report("history/warm_bounded_32", started, READ_ITERATIONS);
    let started = Instant::now();
    for _ in 0..READ_ITERATIONS {
        black_box(ports.reader.summary(session).await?);
    }
    report("summary/indexed", started, READ_ITERATIONS);
    let mut measured = std::time::Duration::ZERO;
    for _ in 0..25 {
        let execution = ports
            .sessions
            .send(RequestId::new(), session, input("stream benchmark"))
            .await?
            .value;
        let attempt = ports
            .executor
            .claim(RequestId::new(), session, execution.id)
            .await?
            .value;
        let round = attempt.begin_current_round(RequestId::new()).await?.value;
        let text = round.begin_text(RequestId::new(), false).await?.value;
        let started = Instant::now();
        for _ in 0..65 {
            text.append("x").await?;
        }
        text.finish(RequestId::new()).await?;
        round.finish(RequestId::new(), None, true).await?;
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await?;
        measured += started.elapsed();
        assert_eq!(
            ports
                .reader
                .text_range(session, text.id(), None, 1024)
                .await?
                .bytes,
            b"x".repeat(65)
        );
    }
    println!(
        "kernel/streaming/65_deltas_plus_completion: {} ns/op",
        measured.as_nanos() / 25
    );
    ports.clone().shutdown().await?;
    Ok(())
}
fn report(name: &str, started: Instant, iterations: usize) {
    println!(
        "kernel/{name}: {} ns/op ({iterations} iterations)",
        started.elapsed().as_nanos() / iterations as u128
    );
}
