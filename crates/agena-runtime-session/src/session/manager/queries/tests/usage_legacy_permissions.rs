use super::*;
use agena_domain::{ModelRef, RequestPerformance, UsageStatsQuery};
use agena_provider::{
    AttributedCompletionUsage, CompletionUsage, PROVIDER_TOOL_USAGE_METADATA_KEY,
};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use serde_json::{Value, json};

async fn persist_accounting(manager: &SessionManager, session: SessionId, scale: u64) {
    let execution = send(manager, session).await;
    let attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "historical-service-call".into(),
                name: "accounting-fixture".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let tool_performance = RequestPerformance {
        duration_ms: 1_000 * scale,
        ttft_ms: Some(100 * scale),
        generation_ms: Some(500 * scale),
        generated_tokens: Some(10 * scale),
    };
    let attributed = AttributedCompletionUsage {
        provider_id: "tool-provider".into(),
        model_id: "tool-model".into(),
        operation: "completion".into(),
        request_id: Some(format!("historical-tool-{}", session.get())),
        usage: Box::new(CompletionUsage {
            requests: 1,
            input_tokens: 50 * scale,
            output_tokens: 8 * scale,
            reasoning_tokens: 2 * scale,
            recorded_cost: 0.25 * scale as f64,
            recorded_cost_available: true,
            performance: vec![tool_performance],
            request_performance: Some(tool_performance),
            ..Default::default()
        }),
    };
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        PROVIDER_TOOL_USAGE_METADATA_KEY.into(),
        json!(serde_json::to_string(&attributed).unwrap()),
    );
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({ "metadata": metadata }),
        },
    )
    .await
    .unwrap();

    let performance = RequestPerformance {
        duration_ms: 1_000 * scale,
        ttft_ms: Some(100 * scale),
        generation_ms: Some(800 * scale),
        generated_tokens: Some(12 * scale),
    };
    let mut state =
        KernelProviderState::new(ModelRef::new("direct-provider", "direct-model"), json!({}));
    state.usage = Some(CompletionUsage {
        requests: 1,
        input_tokens: 100 * scale,
        output_tokens: 10 * scale,
        reasoning_tokens: 2 * scale,
        cache_read_tokens: 5 * scale,
        cache_write_tokens: 3 * scale,
        recorded_cost: 0.125 * scale as f64,
        recorded_cost_available: true,
        performance: vec![performance],
        request_performance: Some(performance),
        ..Default::default()
    });
    round
        .finish(
            RequestId::new(),
            Some(serde_json::to_value(state).unwrap()),
            true,
        )
        .await
        .unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
}

async fn persisted_configuration(manager: &SessionManager, session: SessionId) -> String {
    manager
        .kernel_runtime
        .database
        .query_one(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            "SELECT configuration_json FROM agena_sessions WHERE id=?",
            [session.get().into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "configuration_json")
        .unwrap()
}

#[tokio::test]
async fn usage_stats_preserves_legacy_permission_history_and_subagent_filters() {
    let (manager, root) = fixture().await;
    let workspace = manager
        .kernel()
        .reader
        .metadata(root)
        .await
        .unwrap()
        .workspace_id
        .unwrap();
    // This fixture's workspace is an identity, not a directory on disk.
    manager.workspace.set(workspace).unwrap();
    let other_workspace = manager
        .kernel_runtime
        .workspaces
        .ensure_id("/query-tests-other")
        .await
        .unwrap();
    let legacy_layers = [
        json!({"permission": {"path": {"workspace": {"read": "allow", "write": "ask"}}}}),
        json!({"permission": {"path": {"external": {"read": "auto", "write": "auto"}}}}),
        json!({"permission_ceiling": {"path": {"rules": {"/legacy": {"read": "ask", "write": "ask"}}}}}),
        json!({"permission_ceiling_layers": [{"path": {"workspace": {"read": "allow", "write": "auto"}}}]}),
    ];
    let mut sessions = Vec::new();
    for (index, layer) in legacy_layers.into_iter().enumerate() {
        let is_subagent = index == 1;
        let session = if index == 0 {
            root
        } else {
            manager
                .kernel()
                .sessions
                .create_configured(
                    RequestId::new(),
                    if index == 2 {
                        other_workspace
                    } else {
                        workspace
                    },
                    format!("Legacy accounting {index}"),
                    SessionConfiguration {
                        parent_session_id: is_subagent.then_some(root),
                        relation_kind: if is_subagent {
                            SessionRelationKind::Subagent
                        } else {
                            SessionRelationKind::Root
                        },
                        agent: is_subagent.then(|| SessionAgentConfiguration {
                            origin_message_id: None,
                            name: "legacy_accounting".into(),
                            status: AgentStatus::Created,
                            started_at_ms: None,
                            finished_at_ms: None,
                            failure: None,
                            limits: Default::default(),
                        }),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .value
                .id
        };
        let scale = index as u64 + 1;
        persist_accounting(&manager, session, scale).await;
        sessions.push((session, scale, is_subagent, layer));
    }

    let mut history = Vec::new();
    for (session, scale, is_subagent, layer) in sessions {
        // Inject the historical wire format only into this in-memory database.
        // New execution/configuration writes must continue to reject it.
        let mut config: Value =
            serde_json::from_str(&persisted_configuration(&manager, session).await).unwrap();
        for (key, value) in layer.as_object().unwrap() {
            config[key] = value.clone();
        }
        let encoded = serde_json::to_string(&config).unwrap();
        manager
            .kernel_runtime
            .database
            .execute(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "UPDATE agena_sessions SET configuration_json=? WHERE id=?",
                [encoded.clone().into(), session.get().into()],
            ))
            .await
            .unwrap();
        let error = manager
            .kernel()
            .reader
            .configuration(session)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("move ask/auto"), "{error}");
        history.push((session, scale, is_subagent, encoded));
    }

    let all = manager
        .usage_stats(UsageStatsQuery::custom(None, None))
        .await
        .unwrap();
    assert_eq!(all.totals.sessions, 4);
    assert_eq!(all.totals.requests, 8);
    assert_eq!(all.totals.input_tokens, 1_500);
    assert_eq!(all.totals.output_tokens, 180);
    assert_eq!(all.totals.reasoning_tokens, 40);
    assert_eq!(all.totals.cache_read_tokens, 50);
    assert_eq!(all.totals.cache_write_tokens, 30);
    assert_eq!(all.totals.total_tokens, 1_800);
    assert_eq!(all.totals.total_cost_usd, 3.75);
    assert_eq!(all.totals.performance.request_count, 8);
    assert_eq!(all.totals.performance.generation_tokens, 220);
    assert_eq!(all.totals.performance.total_duration_ms, 20_000);
    assert_eq!(
        all.totals.performance.end_to_end_tokens_per_second,
        Some(11.0)
    );
    assert_eq!(all.by_provider.len(), 2);
    assert_eq!(all.by_model.len(), 2);
    for (provider, tokens, cost, speed) in [
        ("direct-provider", 1_200, 1.25, 15.0),
        ("tool-provider", 600, 2.5, 20.0),
    ] {
        let row = all
            .by_provider
            .iter()
            .find(|row| row.provider_id == provider)
            .unwrap();
        assert_eq!(row.totals.total_tokens, tokens);
        assert_eq!(row.totals.total_cost_usd, cost);
        assert_eq!(row.totals.requests, 4);
        assert_eq!(row.totals.performance.request_count, 4);
        assert_eq!(row.totals.performance.tokens_per_second, Some(speed));
    }
    for (session, scale, is_subagent, _) in &history {
        let row = all
            .by_session
            .iter()
            .find(|row| row.session_id == session.get())
            .unwrap();
        assert_eq!(row.is_subagent, *is_subagent);
        assert_eq!(row.totals.total_tokens, 180 * scale);
        assert_eq!(row.totals.total_cost_usd, 0.375 * *scale as f64);
        assert_eq!(row.totals.performance.request_count, 2);
    }

    let without_subagents = manager
        .usage_stats(UsageStatsQuery::custom(None, None).with_filters(
            vec![],
            vec![],
            vec![],
            false,
        ))
        .await
        .unwrap();
    assert_eq!(without_subagents.totals.sessions, 3);
    assert_eq!(without_subagents.totals.requests, 6);
    assert_eq!(without_subagents.totals.total_tokens, 1_440);
    assert_eq!(without_subagents.totals.total_cost_usd, 3.0);
    assert_eq!(without_subagents.totals.performance.request_count, 6);
    assert_eq!(without_subagents.totals.performance.generation_tokens, 176);
    assert!(
        without_subagents
            .by_session
            .iter()
            .all(|row| !row.is_subagent)
    );
    for row in &without_subagents.by_provider {
        assert_eq!(row.totals.requests, 3);
        assert_eq!(row.totals.performance.request_count, 3);
    }
    for (session, _, _, encoded) in history {
        assert_eq!(persisted_configuration(&manager, session).await, encoded);
    }
    manager.kernel().clone().shutdown().await.unwrap();
}
