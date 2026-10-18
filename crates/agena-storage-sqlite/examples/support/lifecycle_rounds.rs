//! B05: one immutable replay input per measured round, independent of history.

use super::*;

pub(super) async fn run(checkpoints: &str) -> AnyResult<()> {
    let mut checkpoints = checkpoints
        .split(',')
        .map(str::parse::<usize>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    checkpoints.sort_unstable();
    checkpoints.dedup();
    if checkpoints.is_empty()
        || checkpoints[0] == 0
        || checkpoints.last().is_some_and(|value| *value > 10000)
    {
        return Err("B05 expects positive, comma-separated round counts of at most 10000".into());
    }
    let root = owned_root()?;
    let kernel = open_kernel_runtime(&database_url(root.path())).await?;
    let inspect = inspect(root.path()).await?;
    let (session, execution, attempt) =
        start_attempt(&kernel, root.path(), "B05 fixed replay").await?;
    let anchor = attempt
        .begin_round(RequestId::new(), vec![execution.message_id])
        .await?
        .value;
    let text = anchor.begin_text(RequestId::new(), false).await?.value;
    text.append("the same frozen replay boundary for every measured round")
        .await?;
    let provider_state = json!({"opaque":ascii(64*1024,0xb0520261018)});
    let state_bytes = serde_json::to_vec(&provider_state)?.len();
    anchor
        .finish(RequestId::new(), Some(provider_state.clone()), true)
        .await?;
    let base = stored(&inspect, root.path()).await?;
    let initial = resources::sample();
    let sampler = Sampler::start();
    let started = Instant::now();
    let mut interval_start = Instant::now();
    let mut interval_usage = initial;
    let mut interval_metrics = kernel.diagnostics.snapshot();
    let mut previous_rounds = 0;
    emit(
        json!({"case":"b05","phase":"baseline","session":session,"checkpoints":checkpoints,"frozen_input_entries":2,"frozen_replays_per_measured_round":1,"provider_state_bytes_per_round":state_bytes,"database":base,"diagnostics":diagnostics(&kernel),"usage":initial}),
    )?;
    for number in 1..=*checkpoints.last().ok_or("missing round checkpoint")? {
        let round = attempt
            .begin_round(
                RequestId::new(),
                vec![execution.message_id, text.entry_id()],
            )
            .await?
            .value;
        {
            let prompt = round.prompt().await?;
            if prompt.entries.len() != 2
                || prompt.replays.len() != 1
                || prompt.replays[0].round_id != anchor.id()
                || prompt.replays[0].provider_state.as_ref() != Some(&provider_state)
            {
                return Err("B05 measured replay grew or lost its immutable anchor".into());
            }
        }
        round
            .finish(RequestId::new(), Some(provider_state.clone()), true)
            .await?;
        if checkpoints.binary_search(&number).is_ok() {
            let snapshot = stored(&inspect, root.path()).await?;
            if snapshot["provider_state_bytes"].as_u64()
                != Some((state_bytes * (number + 1)) as u64)
                || snapshot["round_inputs"].as_u64() != Some((1 + 2 * number) as u64)
                || snapshot["replay_edges"].as_u64() != Some(number as u64)
            {
                return Err("B05 round/replay storage cardinality mismatch".into());
            }
            let usage = resources::sample();
            let metrics = kernel.diagnostics.snapshot();
            emit(
                json!({"case":"b05","phase":"round_checkpoint","measured_rounds":number,"interval_rounds":number-previous_rounds,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"interval_ms":interval_start.elapsed().as_secs_f64()*1000.0,"interval_io":io_delta(interval_usage,usage),"usage":usage,"sampling":sampler.summary(),"diagnostics":diagnostics(&kernel),"interval_counters":metric_delta(&interval_metrics,&metrics)?,"database":snapshot,"provider_growth":"fixed 64 KiB opaque state per round; no provider-generated cumulative array"}),
            )?;
            previous_rounds = number;
            interval_start = Instant::now();
            interval_usage = usage;
            interval_metrics = metrics;
        }
    }
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await?;
    drop((text, anchor, attempt));
    let final_diagnostics = kernel.diagnostics.clone();
    kernel.ports.clone().shutdown().await?;
    inspect.close().await;
    drop(kernel);
    emit(
        json!({"case":"b05","phase":"closed","elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"sampling":sampler.summary(),"total_io":io_delta(initial,resources::sample()),"read_budget_after_close":final_diagnostics.read_budget(),"wal_checkpoint_after_close":closed_checkpoint(root.path()).await?}),
    )?;
    Ok(())
}
