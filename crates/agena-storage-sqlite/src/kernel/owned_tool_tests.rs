use serde_json::json;
use std::collections::BTreeSet;

use super::{
    tests::{document, fixture_with_diagnostics},
    *,
};

async fn seed_tools(
    ports: &KernelPorts,
    unrelated: usize,
    result_bytes: usize,
) -> (SessionId, Vec<EntryId>) {
    let session = ports
        .sessions
        .create(RequestId::new(), 1, "owned tools".into())
        .await
        .unwrap()
        .value
        .id;
    let user = ports
        .sessions
        .send(RequestId::new(), session, document("file work"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, user.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let mut tools = Vec::new();
    for index in 0..3 {
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: format!("edit-{index}"),
                    name: "fs.write".into(),
                    input: json!({"path":format!("file-{index}.txt")}),
                },
            )
            .await
            .unwrap()
            .value;
        tool.complete(RequestId::new(), ToolResult { phase:ToolPhase::Succeeded, value:json!({"path":format!("file-{index}.txt"),"record":"r".repeat(result_bytes)}) }).await.unwrap();
        tools.push(
            ports
                .reader
                .tool(session, tool.id())
                .await
                .unwrap()
                .entry_id,
        );
    }
    let text = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    text.append(&"unrelated assistant text ".repeat(10_000))
        .await
        .unwrap();
    round
        .finish(
            RequestId::new(),
            Some(json!({"opaque":"opaque bytes never used for files ".repeat(5_000)})),
            true,
        )
        .await
        .unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let body = "ordinary user payload ".repeat(8_000);
    for _ in 0..unrelated {
        ports
            .sessions
            .send(RequestId::new(), session, document(&body))
            .await
            .unwrap();
    }
    (session, tools)
}

#[tokio::test]
async fn owned_tool_pages_skip_large_ordinary_history_and_bound_only_matching_payloads() {
    for memory in [true, false] {
        let diagnostics = KernelDiagnostics::default();
        let (ports, _admin, _root, _url) =
            fixture_with_diagnostics(memory, diagnostics.clone()).await;
        let (session, expected) = seed_tools(&ports, 48, 400 * 1024).await;
        let start = diagnostics.snapshot();
        let first = ports
            .reader
            .owned_tool_entries(session, None, 128)
            .await
            .unwrap();
        assert_eq!(first.entries.len(), 2);
        assert!(first.has_more);
        assert!(
            first.ptr_eq(
                &ports
                    .reader
                    .owned_tool_entries(session, None, 128)
                    .await
                    .unwrap()
            )
        );
        let middle = diagnostics.snapshot();
        assert_eq!(
            middle.owned_tool_entry_rows - start.owned_tool_entry_rows,
            2
        );
        assert_eq!(
            middle.owned_tool_candidate_rows - start.owned_tool_candidate_rows,
            3
        );
        assert!(middle.owned_tool_admitted_bytes - start.owned_tool_admitted_bytes <= 1024 * 1024);
        assert!(middle.owned_tool_entry_body_bytes - start.owned_tool_entry_body_bytes < 1024);
        assert_eq!(middle.history_queries, start.history_queries);
        assert_eq!(middle.prompt_queries, start.prompt_queries);
        let second = ports
            .reader
            .owned_tool_entries(session, first.next, 128)
            .await
            .unwrap();
        assert_eq!(second.entries.len(), 1);
        assert!(!second.has_more);
        let ids = first
            .entries
            .iter()
            .chain(&second.entries)
            .map(|entry| {
                assert_eq!(entry.origin_session_id, session);
                assert!(matches!(entry.payload, EntryPayload::Tool { .. }));
                entry.id
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, expected);
        let fork = ports
            .sessions
            .fork(RequestId::new(), session, expected[0])
            .await
            .unwrap()
            .value
            .id;
        assert!(
            ports
                .reader
                .owned_tool_entries(fork, None, 128)
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        assert!(
            !ports
                .reader
                .history(fork, None, 128)
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn owned_tool_cursors_reject_foreign_or_changed_membership_and_control_facts() {
    for memory in [true, false] {
        let (ports, _admin, _root, _url) = tests::fixture(memory).await;
        let (session, _) = seed_tools(&ports, 0, 1).await;
        let first = ports
            .reader
            .owned_tool_entries(session, None, 1)
            .await
            .unwrap();
        let cursor = first.next.unwrap();
        let other = ports
            .sessions
            .create(RequestId::new(), 1, "other".into())
            .await
            .unwrap()
            .value
            .id;
        assert!(matches!(
            ports
                .reader
                .owned_tool_entries(other, Some(cursor), 1)
                .await,
            Err(KernelError::Ownership)
        ));
        let changed_membership = OwnedToolCursor {
            membership: cursor.membership.next().unwrap(),
            ..cursor
        };
        assert!(matches!(
            ports
                .reader
                .owned_tool_entries(session, Some(changed_membership), 1)
                .await,
            Err(KernelError::Conflict(_))
        ));
        ports
            .sessions
            .update_metadata(
                RequestId::new(),
                session,
                MetadataChange {
                    title: Some("changed while projecting".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            ports
                .reader
                .owned_tool_entries(session, Some(cursor), 1)
                .await,
            Err(KernelError::Conflict(_))
        ));
        let updated = ports
            .reader
            .owned_tool_entries(session, None, 1)
            .await
            .unwrap();
        assert!(updated.control_version > first.control_version);
        assert_eq!(
            updated.metadata.membership_generation,
            first.metadata.membership_generation
        );
        ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn owned_tool_preflight_uses_partial_covering_indexes_without_cold_json_columns() {
    for memory in [true, false] {
        let (ports, admin, _root, _url) = tests::fixture(memory).await;
        let plan = sqlx::query(&format!(
            "EXPLAIN QUERY PLAN {}",
            owned_tools::PREFLIGHT_SQL
        ))
        .bind(1i64)
        .bind(0i64)
        .bind(129i64)
        .fetch_all(admin.get_sqlite_connection_pool())
        .await
        .unwrap();
        let details = plan
            .iter()
            .map(|row| row.try_get::<String, _>("detail").unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            details.contains("idx_agena_owned_tool_entries"),
            "{details}"
        );
        assert!(details.contains("idx_agena_tool_bytes"), "{details}");
        assert!(!details.contains("USE TEMP B-TREE"), "{details}");
        let roots = sqlx::query_scalar::<_, i64>(
            "SELECT rootpage FROM sqlite_schema WHERE name IN ('agena_entries','agena_tool_calls')",
        )
        .fetch_all(admin.get_sqlite_connection_pool())
        .await
        .unwrap()
        .into_iter()
        .collect::<BTreeSet<_>>();
        let vm = sqlx::query(&format!("EXPLAIN {}", owned_tools::PREFLIGHT_SQL))
            .bind(1i64)
            .bind(0i64)
            .bind(129i64)
            .fetch_all(admin.get_sqlite_connection_pool())
            .await
            .unwrap();
        let cold_cursors = vm
            .iter()
            .filter(|row| {
                row.try_get::<String, _>("opcode").unwrap() == "OpenRead"
                    && roots.contains(&row.try_get::<i64, _>("p2").unwrap())
            })
            .map(|row| row.try_get::<i64, _>("p1").unwrap())
            .collect::<BTreeSet<_>>();
        assert!(
            !vm.iter().any(
                |row| row.try_get::<String, _>("opcode").unwrap() == "Column"
                    && cold_cursors.contains(&row.try_get::<i64, _>("p1").unwrap())
            ),
            "preflight loads a cold table column: {details}"
        );
        ports.shutdown().await.unwrap();
    }
}
