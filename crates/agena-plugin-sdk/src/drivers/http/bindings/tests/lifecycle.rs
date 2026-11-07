use super::*;

#[tokio::test]
async fn fork_keeps_an_admitted_old_request_and_saved_host_client_on_its_original_callback() {
    let mut fixture = Fixture::start().await;
    let wire = fixture.wire.clone();
    let old = tokio::spawn(async move { wire.invoke("old", 11, json!({"hold":true})).await });
    fixture.started(11).await;
    // This must finish while the old request still owns its admission.
    fixture.fork().await;
    assert!(!old.is_finished());
    assert_eq!(
        ok(fixture.wire.invoke("new", 12, json!({})).await)["output_text"],
        "new"
    );
    fixture.control.release.add_permits(1);
    assert_eq!(ok(old.await.unwrap())["output_text"], "old");
    assert_eq!(fixture.control.initialized.load(Ordering::SeqCst), 1);

    for (peer, id) in [(&fixture.old, "old"), (&fixture.new, "new")] {
        let records = peer.requests.lock().unwrap();
        let request = records
            .iter()
            .find(|request| request.method == method::HOST_CONFIG_READ)
            .unwrap();
        let context = &request.params.as_ref().unwrap()["context"];
        assert_eq!(context["http_binding_id"], id);
        assert_eq!(context["authority_token"], format!("authority-{id}"));
    }
    assert!(!fixture.wire.retire("old").await.last);
    assert!(fixture.wire.retire("new").await.last);
    ok(fixture
        .wire
        .call("new", method::META_SHUTDOWN, json!({}))
        .await);
    fixture.finish().await;
}

#[tokio::test]
async fn retiring_one_binding_cancels_only_its_requests_and_preserves_the_successor() {
    let mut fixture = Fixture::start().await;
    fixture.fork().await;
    let wire = fixture.wire.clone();
    let old = tokio::spawn(async move { wire.invoke("old", 21, json!({"hold":true})).await });
    fixture.started(21).await;
    let wire = fixture.wire.clone();
    let new = tokio::spawn(async move { wire.invoke("new", 22, json!({"hold":true})).await });
    fixture.started(22).await;

    let retired = fixture.wire.retire("old").await;
    assert!(!retired.last && !retired.instance_stopped);
    error(old.await.unwrap());
    assert!(
        !new.is_finished(),
        "old retirement must not cancel the new request"
    );
    error(fixture.wire.invoke("old", 23, json!({})).await);
    assert_eq!(fixture.wire.retire("old").await, retired);

    fixture.control.release.add_permits(1);
    assert_eq!(ok(new.await.unwrap())["output_text"], "new");
    assert!(fixture.wire.retire("new").await.last);
    ok(fixture
        .wire
        .call("new", method::META_SHUTDOWN, json!({}))
        .await);
    fixture.finish().await;
}

#[tokio::test]
async fn reverse_retirement_restores_process_default_and_shutdown_alone_can_use_the_last_binding() {
    let fixture = Fixture::start().await;
    fixture.fork().await;
    assert_eq!(
        ok(fixture
            .wire
            .invoke("old", 31, json!({"process":true}))
            .await)["output_text"],
        "new"
    );
    let retired = fixture.wire.retire("new").await;
    assert!(!retired.last);
    assert_eq!(fixture.wire.retire("new").await, retired);
    error(fixture.wire.invoke("new", 32, json!({})).await);
    assert_eq!(
        ok(fixture
            .wire
            .invoke("old", 33, json!({"process":true}))
            .await)["output_text"],
        "old"
    );

    let unknown = error(
        fixture
            .wire
            .call(
                "unknown",
                method::META_HOST_RETIRE,
                json!({"binding_id":"unknown"}),
            )
            .await,
    );
    let unknown: PluginError = serde_json::from_value(unknown.data.unwrap()).unwrap();
    assert_eq!(unknown.kind, PluginErrorKind::HostUnavailable);
    assert!(unknown.diagnostic_message().contains("binding"));
    assert!(fixture.wire.retire("old").await.last);
    let saved = fixture.control.saved_hosts.lock().unwrap()[0].clone();
    assert_eq!(
        saved.read_config(None).await.unwrap_err().kind,
        PluginErrorKind::HostUnavailable
    );
    ok(fixture
        .wire
        .call("old", method::META_SHUTDOWN, json!({}))
        .await);
    assert_eq!(
        *fixture.control.shutdown_markers.lock().unwrap(),
        vec!["old"]
    );
    assert_eq!(fixture.control.shutdowns.load(Ordering::SeqCst), 1);
    assert_eq!(
        saved.read_config(None).await.unwrap_err().kind,
        PluginErrorKind::HostUnavailable
    );
    ok(fixture
        .wire
        .call("old", method::META_SHUTDOWN, json!({}))
        .await);
    assert_eq!(fixture.control.shutdowns.load(Ordering::SeqCst), 1);
    fixture.finish().await;
}

#[tokio::test]
async fn invalid_forks_headers_and_body_selectors_cannot_change_callback_destinations() {
    let fixture = Fixture::start().await;
    for (next_id, expected) in [
        ("old", fixture.old.binding.clone()),
        ("new", fixture.new.binding.clone()),
        ("invalid/next", fixture.old.binding.clone()),
    ] {
        error(
            fixture
                .wire
                .call(
                    "old",
                    method::META_HOST_FORK,
                    serde_json::to_value(HttpCallbackFork {
                        expected_id: "old".into(),
                        expected,
                        next_id: next_id.into(),
                        next: fixture.new.binding.clone(),
                    })
                    .unwrap(),
                )
                .await,
        );
        assert_eq!(
            ok(fixture.wire.invoke("old", 41, json!({})).await)["output_text"],
            "old"
        );
    }
    fixture.fork().await;
    // Body selectors are echo-only. A different or unknown body value cannot
    // override the independent, validated transport header.
    for spoof in ["new", "http://127.0.0.1/attacker", "not-registered"] {
        let result = fixture
            .wire
            .send(
                Wire::headers("owner-a", Some("old")),
                method::HOOK_TOOL_INVOKE,
                invocation(42, json!({})),
                Some(HostCallbackContext {
                    http_binding_id: Some(spoof.into()),
                    ..Default::default()
                }),
            )
            .await;
        assert_eq!(ok(result)["output_text"], "old");
    }
    error(
        fixture
            .wire
            .send(
                Wire::headers("owner-a", None),
                method::HOOK_TOOL_INVOKE,
                invocation(43, json!({})),
                Some(HostCallbackContext {
                    http_binding_id: Some("old".into()),
                    ..Default::default()
                }),
            )
            .await,
    );
    for id in ["unknown", "invalid/id"] {
        error(fixture.wire.invoke(id, 44, json!({})).await);
    }
    let mut repeated = Wire::headers("owner-a", Some("old"));
    repeated.append(BINDING_HEADER, "new".parse().unwrap());
    error(
        fixture
            .wire
            .send(
                repeated,
                method::HOOK_TOOL_INVOKE,
                invocation(45, json!({})),
                None,
            )
            .await,
    );

    // Destructive compatibility rebind must not silently move the retained old
    // generation when more than one generation is active.
    error(
        fixture
            .wire
            .call(
                "old",
                method::META_HOST_REBIND,
                json!({
                    "expected":fixture.new.binding, "next":fixture.old.binding,
                }),
            )
            .await,
    );
    assert_eq!(
        ok(fixture.wire.invoke("new", 46, json!({})).await)["output_text"],
        "new"
    );
    assert!(!fixture.wire.retire("old").await.last);
    assert!(fixture.wire.retire("new").await.last);
    ok(fixture
        .wire
        .call("new", method::META_SHUTDOWN, json!({}))
        .await);
    fixture.finish().await;
}

#[tokio::test]
async fn successful_instance_replacement_keeps_exact_binding_retirement_evidence() {
    let fixture = Fixture::start().await;
    fixture.fork().await;
    ok(fixture
        .initialize("owner-b", "successor", &fixture.new.binding)
        .await);
    for id in ["old", "new"] {
        let retired = fixture.wire.retire(id).await;
        assert!(retired.instance_stopped && !retired.last);
    }
    error(
        fixture
            .wire
            .call(
                "never-registered",
                method::META_HOST_RETIRE,
                json!({"binding_id":"never-registered"}),
            )
            .await,
    );
    error(
        fixture
            .wire
            .send(
                Wire::headers("unknown-owner", Some("old")),
                method::META_HOST_RETIRE,
                json!({"binding_id":"old"}),
                None,
            )
            .await,
    );
    assert_eq!(
        ok(fixture
            .wire
            .send(
                Wire::headers("owner-b", Some("successor")),
                method::HOOK_TOOL_INVOKE,
                invocation(51, json!({})),
                None
            )
            .await)["output_text"],
        "new"
    );
    ok(fixture
        .wire
        .send(
            Wire::headers("owner-b", Some("successor")),
            method::META_SHUTDOWN,
            json!({}),
            None,
        )
        .await);
    fixture.finish().await;
}

#[tokio::test]
async fn failed_instance_cleanup_does_not_claim_successful_binding_retirement() {
    let fixture = Fixture::start().await;
    fixture.fork().await;
    fixture.control.fail_shutdown.store(true, Ordering::SeqCst);
    let failed = error(
        fixture
            .initialize("owner-b", "successor", &fixture.new.binding)
            .await,
    );
    let failed: PluginError = serde_json::from_value(failed.data.unwrap()).unwrap();
    assert_eq!(failed.kind, PluginErrorKind::Internal);
    assert!(
        failed
            .diagnostic_message()
            .contains("deliberate generation cleanup failure")
    );
    for id in ["old", "new"] {
        error(
            fixture
                .wire
                .call(id, method::META_HOST_RETIRE, json!({"binding_id":id}))
                .await,
        );
    }
    fixture.control.fail_shutdown.store(false, Ordering::SeqCst);
    ok(fixture
        .wire
        .call("new", method::META_SHUTDOWN, json!({}))
        .await);
    assert!(fixture.wire.retire("old").await.instance_stopped);
    fixture.finish().await;
}

#[tokio::test]
async fn completed_retirement_receipts_are_bounded_and_never_admit_evicted_bindings() {
    // Capacity is a protocol resource bound, independent of an HTTP client's
    // timing. Live generations are never evicted to make room for another.
    let bindings = CallbackBindings::new(
        Some("binding-0".into()),
        HttpCallbackBinding {
            url: None,
            token: None,
        },
        CancellationToken::new(),
    )
    .unwrap();
    for index in 0..=RETIRED_CAPACITY {
        bindings
            .fork(HttpCallbackFork {
                expected_id: format!("binding-{index}"),
                expected: HttpCallbackBinding {
                    url: None,
                    token: None,
                },
                next_id: format!("binding-{}", index + 1),
                next: HttpCallbackBinding {
                    url: None,
                    token: None,
                },
            })
            .unwrap();
        assert!(
            !bindings
                .retire(&format!("binding-{index}"))
                .await
                .unwrap()
                .last
        );
    }
    assert!(bindings.retire("binding-0").await.is_err());
    assert!(bindings.admit(Some("binding-0")).await.is_err());
    assert!(bindings.retire("never-registered").await.is_err());
    assert!(!bindings.retire("binding-1").await.unwrap().last);
    assert!(
        bindings
            .retire(&format!("binding-{}", RETIRED_CAPACITY + 1))
            .await
            .unwrap()
            .last
    );
}

#[tokio::test]
async fn abort_before_a_delayed_fork_fences_its_id_without_claiming_retirement() {
    let fixture = Fixture::start().await;
    let fenced: HttpCallbackRetired = serde_json::from_value(ok(fixture
        .wire
        .call(
            "new",
            method::META_HOST_RETIRE,
            json!({"binding_id":"new", "abort_uncommitted":true}),
        )
        .await))
    .unwrap();
    assert!(fenced.fenced_uncommitted && !fenced.last && !fenced.instance_stopped);
    // Deterministic wire ordering: rollback arrives before the delayed fork.
    error(
        fixture
            .wire
            .call(
                "old",
                method::META_HOST_FORK,
                serde_json::to_value(HttpCallbackFork {
                    expected_id: "old".into(),
                    expected: fixture.old.binding.clone(),
                    next_id: "new".into(),
                    next: fixture.new.binding.clone(),
                })
                .unwrap(),
            )
            .await,
    );
    error(
        fixture
            .wire
            .call("new", method::META_HOST_RETIRE, json!({"binding_id":"new"}))
            .await,
    );
    assert_eq!(
        ok(fixture
            .wire
            .invoke("old", 91, json!({"process":true}))
            .await)["output_text"],
        "old"
    );
    assert_eq!(fixture.control.initialized.load(Ordering::SeqCst), 1);
    assert!(fixture.wire.retire("old").await.last);
    ok(fixture
        .wire
        .call("old", method::META_SHUTDOWN, json!({}))
        .await);
    fixture.finish().await;
}

#[tokio::test]
async fn abort_fences_never_evict_and_capacity_closes_only_future_forks() {
    let bindings = CallbackBindings::new(
        Some("live".into()),
        HttpCallbackBinding {
            url: None,
            token: None,
        },
        CancellationToken::new(),
    )
    .unwrap();
    for index in 0..=RETIRED_CAPACITY {
        assert!(
            bindings
                .abort_fork(&format!("aborted-{index}"))
                .await
                .unwrap()
                .fenced_uncommitted
        );
    }
    let failed = bindings
        .fork(HttpCallbackFork {
            expected_id: "live".into(),
            expected: HttpCallbackBinding {
                url: None,
                token: None,
            },
            next_id: "otherwise-valid".into(),
            next: HttpCallbackBinding {
                url: None,
                token: None,
            },
        })
        .unwrap_err();
    assert!(failed.diagnostic_message().contains("capacity"));
    drop(bindings.admit(Some("live")).await.unwrap());
    assert!(bindings.retire("aborted-0").await.is_err());
    assert!(bindings.retire("live").await.unwrap().last);
}
