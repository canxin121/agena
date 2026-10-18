//! Golden user-visible content/replay contracts through public kernel ports.
use agena_domain::{
    CommandOutputStream, ComposerDocument, ComposerNode, TerminalAttributes, TerminalCellRun,
    TerminalSnapshot,
};
use agena_storage::kernel::*;
use agena_storage_sqlite::open_kernel_runtime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
fn input(text: &str) -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text { text: text.into() }])
}
fn screen(text: &str) -> TerminalSnapshot {
    TerminalSnapshot {
        rows: 2,
        cols: 80,
        cursor_row: 0,
        cursor_col: 0,
        cursor_visible: true,
        alternate_screen: false,
        bracketed_paste: true,
        application_cursor: false,
        cells: vec![
            TerminalCellRun {
                row: 0,
                col: 0,
                text: text.into(),
                attributes: TerminalAttributes::default(),
            },
            TerminalCellRun {
                row: 1,
                col: 0,
                text: "unchanged".into(),
                attributes: TerminalAttributes::default(),
            },
        ],
    }
}
#[tokio::test]
async fn golden_content_export_and_pty_checks_match_memory_and_file_kernels() {
    for memory in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("contracts.db");
        let url = if memory {
            "sqlite::memory:".to_owned()
        } else {
            format!("sqlite://{}?mode=rwc", path.display())
        };
        let kernel = open_kernel_runtime(&url).await.unwrap();
        let workspace = kernel
            .workspaces
            .create(root.path().to_string_lossy().into_owned())
            .await
            .unwrap();
        let p = &kernel.ports;
        let session = p
            .sessions
            .create(RequestId::new(), workspace.id, "golden".into())
            .await
            .unwrap()
            .value
            .id;
        let sent = p
            .sessions
            .send(RequestId::new(), session, input("golden input 界"))
            .await
            .unwrap()
            .value;
        let attempt = p
            .executor
            .claim(RequestId::new(), session, sent.id)
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
                    call_id: "golden".into(),
                    name: "fixture".into(),
                    input: json!({"raw":"  \r\n"}),
                },
            )
            .await
            .unwrap()
            .value;
        let log = tool.begin_log(RequestId::new()).await.unwrap().value;
        let fixture: Value =
            serde_json::from_str(include_str!("fixtures/kernel-content-golden.json")).unwrap();
        for line in fixture["log"].as_array().unwrap() {
            let stream = if line["stream"] == "stdout" {
                CommandOutputStream::Stdout
            } else {
                CommandOutputStream::Stderr
            };
            log.append(stream, line["text"].as_str().unwrap())
                .await
                .unwrap();
        }
        let terminal = tool.begin_terminal(RequestId::new()).await.unwrap().value;
        let base = terminal.snapshot(screen("first")).await.unwrap();
        let mut patch = screen("next 界");
        patch.cells.truncate(1);
        terminal.patch(base, patch, vec![0]).await.unwrap();
        log.finish(RequestId::new()).await.unwrap();
        terminal.finish(RequestId::new()).await.unwrap();
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({"text":"  exact\r\n\u{1b}[31m"}),
            },
        )
        .await
        .unwrap();
        let replay: Value =
            serde_json::from_str(include_str!("fixtures/kernel-replay-golden.json")).unwrap();
        round
            .finish(RequestId::new(), Some(replay.clone()), true)
            .await
            .unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        let page = p
            .reader
            .event_page(session, log.id(), None, 32)
            .await
            .unwrap();
        assert_eq!(page.records.len(), 4);
        for (actual, expected) in page.records.iter().zip(fixture["log"].as_array().unwrap()) {
            let EventPayload::Log {
                channel: stream,
                text,
                ..
            } = &actual.payload
            else {
                panic!("log codec changed")
            };
            assert_eq!(text, expected["text"].as_str().unwrap());
            assert_eq!(
                *stream,
                if expected["stream"] == "stdout" {
                    CommandOutputStream::Stdout
                } else {
                    CommandOutputStream::Stderr
                }
            );
        }
        let snapshot = p
            .reader
            .event_snapshot(session, terminal.id())
            .await
            .unwrap();
        assert_eq!(snapshot.terminal.as_deref(), Some(&screen("next 界")));
        assert_eq!(
            p.reader
                .round(session, round.id())
                .await
                .unwrap()
                .provider_state,
            Some(replay.clone())
        );
        let frozen = p.reader.history_snapshot(session).await.unwrap();
        let mut archive = Vec::new();
        let report = frozen.export_jsonl(&mut archive).await.unwrap();
        assert_eq!(
            report.sha256,
            <[u8; 32]>::from(Sha256::digest(&archive[..report.bytes as usize]))
        );
        let rows = archive
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.first().unwrap()["kind"], "header");
        assert_eq!(rows.last().unwrap()["kind"], "footer");
        assert!(rows.iter().any(|row| {
            row["kind"] == "round"
                && row["data"]["provider_state_json"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .as_ref()
                    == Some(&replay)
        }));
        drop((frozen, snapshot, page));
        p.clone().shutdown().await.unwrap();
    }
}
