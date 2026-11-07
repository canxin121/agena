use super::*;

async fn clock_fixture() -> Result<(tempfile::TempDir, TestClock)> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("clock");
    tokio::fs::write(&path, "1700000000000").await?;
    let clock = TestClock::new(path, Arc::new(Mutex::new(())));
    Ok((dir, clock))
}

#[tokio::test]
async fn exact_scheduler_clock_supports_instant_jumps_and_rejects_negative_advance() -> Result<()> {
    let (_dir, clock) = clock_fixture().await?;
    let original = clock.now().await?;
    assert_eq!(original.timestamp_millis(), 1_700_000_000_000);
    let next = clock.advance(chrono::Duration::days(12)).await?;
    assert_eq!(next - original, chrono::Duration::days(12));
    assert_eq!(clock.now().await?, next);
    assert_eq!(clock.elapsed().await?, Duration::from_secs(12 * 86400));
    assert!(
        clock
            .advance(chrono::Duration::milliseconds(-1))
            .await
            .is_err()
    );
    assert!(clock.advance_to(original).await.is_err());
    assert_eq!(clock.now().await?, next);
    Ok(())
}

#[tokio::test]
async fn separately_acquired_case_handles_cannot_lose_concurrent_advances() -> Result<()> {
    let (_dir, clock) = clock_fixture().await?;
    let original = clock.now().await?;
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..32 {
        // This mirrors separate case.clock() calls, not only Clone.
        let handle = TestClock::new(clock.path.clone(), clock.gate.clone());
        tasks.spawn(async move { handle.advance(chrono::Duration::minutes(15)).await });
    }
    while let Some(result) = tasks.join_next().await {
        result??;
    }
    assert_eq!(clock.now().await?, original + chrono::Duration::hours(8));
    assert_eq!(clock.elapsed().await?, Duration::from_secs(8 * 3600));
    assert_eq!(clock.read().await?.revision, 32);
    Ok(())
}

#[tokio::test]
async fn invalid_precision_and_overflow_leave_clock_file_unchanged() -> Result<()> {
    let (_dir, clock) = clock_fixture().await?;
    let now = clock.now().await?;
    let before = tokio::fs::read(&clock.path).await?;
    assert!(
        clock
            .advance(chrono::Duration::nanoseconds(1))
            .await
            .is_err()
    );
    assert!(
        clock
            .freeze(now + chrono::Duration::nanoseconds(1))
            .await
            .is_err()
    );
    assert!(clock.sleep(Duration::from_nanos(1)).await.is_err());
    assert_eq!(tokio::fs::read(&clock.path).await?, before);

    let maximum =
        DateTime::from_timestamp_millis(DateTime::<Utc>::MAX_UTC.timestamp_millis()).unwrap();
    clock.set(maximum).await?;
    let before = tokio::fs::read(&clock.path).await?;
    assert!(clock.advance(chrono::Duration::days(1)).await.is_err());
    assert_eq!(tokio::fs::read(&clock.path).await?, before);

    let mut state = clock.read().await?;
    state.now_ms = now.timestamp_millis();
    state.elapsed_ms = u64::MAX;
    tokio::fs::write(&clock.path, serde_json::to_vec(&state)?).await?;
    let before = tokio::fs::read(&clock.path).await?;
    assert!(
        clock
            .advance(chrono::Duration::milliseconds(1))
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(&clock.path).await?, before);

    state.elapsed_ms = 0;
    state.revision = u64::MAX;
    tokio::fs::write(&clock.path, serde_json::to_vec(&state)?).await?;
    let before = tokio::fs::read(&clock.path).await?;
    assert!(
        clock
            .advance(chrono::Duration::milliseconds(1))
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read(&clock.path).await?, before);
    Ok(())
}

#[tokio::test]
async fn frozen_clock_wakes_its_sleepers_and_timeout_at_the_exact_virtual_deadline() -> Result<()> {
    let (_dir, clock) = clock_fixture().await?;
    let baseline = clock.now().await?;
    let (started, registered) = tokio::sync::oneshot::channel();
    let timer_clock = clock.clone();
    let timeout = tokio::spawn(async move {
        timer_clock
            .timeout(Duration::from_secs(3 * 3600), async move {
                let _ = started.send(());
                std::future::pending::<()>().await;
            })
            .await
    });
    // timeout records its virtual deadline before polling the wrapped future.
    tokio::time::timeout(Duration::from_secs(2), registered).await??;
    let sleeper_clock = clock.clone();
    let wall_sleep = tokio::spawn(async move {
        sleeper_clock
            .sleep_until(baseline + chrono::Duration::hours(3))
            .await
    });
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert_eq!(clock.now().await?, baseline);
    assert!(!timeout.is_finished());
    assert!(!wall_sleep.is_finished());

    clock
        .advance(chrono::Duration::hours(3) - chrono::Duration::milliseconds(1))
        .await?;
    assert!(!timeout.is_finished());
    assert!(!wall_sleep.is_finished());
    clock.advance(chrono::Duration::milliseconds(1)).await?;
    let error = tokio::time::timeout(Duration::from_secs(2), timeout)
        .await??
        .unwrap_err();
    assert!(error.to_string().contains("virtual timeout"));
    tokio::time::timeout(Duration::from_secs(2), wall_sleep).await???;
    // Absolute deadlines remain reliable if their future starts after a jump.
    clock
        .sleep_until_elapsed(Duration::from_secs(3 * 3600))
        .await?;
    assert_eq!(clock.elapsed().await?, Duration::from_secs(3 * 3600));
    Ok(())
}

#[tokio::test]
async fn wall_rewinds_do_not_rewind_duration_timers() -> Result<()> {
    let (_dir, clock) = clock_fixture().await?;
    let baseline = clock.now().await?;
    clock.advance(chrono::Duration::hours(1)).await?;
    clock.freeze(baseline - chrono::Duration::days(1)).await?;
    assert_eq!(clock.elapsed().await?, Duration::from_secs(3600));
    clock.sleep_until_elapsed(Duration::from_secs(3600)).await?;
    clock.advance_to(baseline).await?;
    assert_eq!(clock.now().await?, baseline);
    assert_eq!(clock.elapsed().await?, Duration::from_secs(25 * 3600));
    Ok(())
}

#[tokio::test]
async fn ready_operation_wins_a_simultaneous_virtual_timeout() -> Result<()> {
    let (_dir, clock) = clock_fixture().await?;
    assert_eq!(clock.timeout(Duration::ZERO, async { 42 }).await?, 42);
    clock.sleep(Duration::ZERO).await?;
    Ok(())
}

#[tokio::test]
async fn invalid_persisted_clock_never_falls_back_to_wall_time() -> Result<()> {
    let (_dir, clock) = clock_fixture().await?;
    for raw in ["", "not-a-time", "9223372036854775807", "{\"version\":2}"] {
        tokio::fs::write(&clock.path, raw).await?;
        assert!(
            clock.now().await.is_err(),
            "accepted invalid clock state {raw:?}"
        );
        assert!(clock.advance(chrono::Duration::hours(1)).await.is_err());
        assert_eq!(tokio::fs::read_to_string(&clock.path).await?, raw);
    }
    Ok(())
}
