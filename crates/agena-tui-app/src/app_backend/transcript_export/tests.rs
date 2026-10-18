use super::*;
use agena_domain::{
    ContentChunk, ContentDocument, ContentId, ContentInput, ContentPage, ContentPayload,
    ContentResource, ContentState, DocumentMutation, TerminalAttributes, TerminalCellRun,
    TerminalSnapshot, ViewBlock,
};
use serde_json::{Value, json};
use tokio::{io::AsyncReadExt, net::TcpListener, task::JoinHandle};

const SESSION_ID: i64 = 7;

async fn serve(responses: Vec<(String, Value)>) -> (TuiBackend, JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let backend = TuiBackend::remote_mock_at(&format!("http://{}", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        for (expected, body) in responses {
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut request = Vec::new();
            let mut bytes = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut bytes).await.unwrap();
                assert!(read > 0);
                request.extend_from_slice(&bytes[..read]);
            }
            let request = String::from_utf8(request).unwrap();
            let path = request.split_whitespace().nth(1).unwrap();
            assert!(path.contains(&expected), "expected {expected}, got {path}");
            let body = body.to_string();
            stream.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()
            ).as_bytes()).await.unwrap();
        }
    });
    (backend, server)
}

fn part(id: i64, run_id: i64, role: &str, boundary: bool, content: Value) -> PartResource {
    PartResource {
        part_id: id,
        run_id: Some(run_id),
        message_boundary: boundary,
        kind: "text".into(),
        role: role.into(),
        state: "completed".into(),
        content,
        origin_session_id: SESSION_ID,
        ..Default::default()
    }
}

fn parts_page(parts: Vec<PartResource>, next: Option<&str>) -> Value {
    json!({"session_id": SESSION_ID, "version": 1, "parts": parts, "runs": [],
        "page": {"returned": parts.len(), "has_more": next.is_some(), "next_cursor": next}})
}

fn resource(kind: ContentKind) -> ContentResource {
    let id = ContentId::new();
    let cursor = ContentCursor {
        epoch: id.0,
        sequence: 0,
    };
    ContentResource {
        resource_id: id,
        owner_session_id: SESSION_ID,
        part_id: 2,
        kind,
        state: ContentState::Active,
        cursor,
        committed_cursor: cursor,
        total_bytes: None,
        dropped_bytes: 0,
        dropped_packets: 0,
        committed_dropped_bytes: 0,
        committed_dropped_packets: 0,
        retained_ranges: vec![],
        capture_error: None,
    }
}

fn page(
    resource: &ContentResource,
    head: u64,
    records: Vec<(u64, ContentPayload)>,
    has_more: bool,
) -> ContentPage {
    let mut resource = resource.clone();
    resource.cursor.sequence = head;
    resource.committed_cursor = resource.cursor;
    let next_cursor = ContentCursor {
        sequence: records.last().map_or(0, |(sequence, _)| *sequence),
        ..resource.cursor
    };
    ContentPage {
        chunks: records
            .into_iter()
            .map(|(sequence, payload)| ContentChunk {
                cursor: ContentCursor {
                    sequence,
                    ..resource.cursor
                },
                captured_at_ms: 0,
                payload,
            })
            .collect(),
        resource,
        next_cursor,
        has_more,
        gap: false,
    }
}

fn export_request(destination: PathBuf) -> TranscriptExportRequest {
    TranscriptExportRequest {
        session_id: SESSION_ID,
        session_title: "Complete history".into(),
        execution: None,
        i18n: I18n::english(),
        markdown: true,
        destination: Some(destination),
        pager_context: String::new(),
    }
}

#[tokio::test]
async fn copy_reads_the_target_reply_across_history_pages_and_preserves_source_bytes() {
    let source = resource(ContentKind::Text);
    let text = "  中文 λ\tfirst  \n\nlast  ";
    let marker = part(
        2,
        2,
        "assistant",
        true,
        json!({"text": "", "resources": [source.reference()]}),
    );
    let mut maintenance = part(4, 4, "assistant", true, json!({"run_kind": "compaction"}));
    maintenance.kind = "compaction".into();
    let (backend, server) = serve(vec![
        (
            "/parts?limit=200".into(),
            parts_page(
                vec![
                    part(3, 2, "assistant", false, json!({"text": " tail\t"})),
                    maintenance,
                ],
                Some("older"),
            ),
        ),
        (
            "cursor=older".into(),
            parts_page(
                vec![
                    part(1, 1, "user", true, json!({"text": "old user"})),
                    marker.clone(),
                ],
                None,
            ),
        ),
        (
            format!("/content/{}?", source.resource_id),
            serde_json::to_value(page(
                &source,
                text.len() as u64,
                vec![(
                    text.len() as u64,
                    ContentPayload::Text { text: text.into() },
                )],
                false,
            ))
            .unwrap(),
        ),
    ])
    .await;
    assert_eq!(
        backend
            .last_assistant_message_text(SESSION_ID)
            .await
            .unwrap()
            .as_deref(),
        Some(format!("{text}\n tail\t").as_str())
    );
    assert_eq!(marker.content["text"], "");
    server.await.unwrap();
}

#[tokio::test]
async fn export_reads_all_history_and_only_exposes_a_complete_staged_file() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("report.md");
    std::fs::write(&destination, "existing report").unwrap();
    let source = resource(ContentKind::Text);
    let text = "  complete 中文 λ\t正文  \n\nfinal whitespace  ";
    let (backend, server) = serve(vec![
        (
            "/parts?limit=200".into(),
            parts_page(
                vec![part(
                    2,
                    2,
                    "assistant",
                    true,
                    json!({"text": "", "resources": [source.reference()]}),
                )],
                Some("older"),
            ),
        ),
        (
            "cursor=older".into(),
            parts_page(
                vec![part(
                    1,
                    1,
                    "user",
                    true,
                    json!({"text": "  older page input  "}),
                )],
                None,
            ),
        ),
        (
            format!("/content/{}?", source.resource_id),
            serde_json::to_value(page(
                &source,
                text.len() as u64,
                vec![(
                    text.len() as u64,
                    ContentPayload::Text { text: text.into() },
                )],
                false,
            ))
            .unwrap(),
        ),
    ])
    .await;
    let staged = backend
        .prepare_transcript_export(export_request(destination.clone()))
        .await
        .unwrap()
        .unwrap();
    let body = std::fs::read_to_string(staged.path()).unwrap();
    assert!(body.contains("  older page input  "));
    assert!(body.contains(text));
    assert_eq!(body.matches(text).count(), 1);
    assert_eq!(
        std::fs::read_to_string(destination).unwrap(),
        "existing report"
    );
    let staged_path = staged.path().to_owned();
    drop(staged);
    assert!(!staged_path.exists());
    server.await.unwrap();
}

fn document(text: &str) -> ContentDocument {
    ContentDocument {
        blocks: vec![ViewBlock::Text {
            id: Some("body".into()),
            text: text.into(),
        }],
    }
}

fn screen(text: &str) -> TerminalSnapshot {
    TerminalSnapshot {
        rows: 1,
        cols: 40,
        cursor_row: 0,
        cursor_col: 0,
        cursor_visible: false,
        alternate_screen: false,
        bracketed_paste: false,
        application_cursor: false,
        cells: vec![TerminalCellRun {
            row: 0,
            col: 0,
            text: text.into(),
            attributes: TerminalAttributes::default(),
        }],
    }
}

#[tokio::test]
async fn typed_export_keeps_large_mixed_text_and_projects_final_document_and_terminal_at_initial_heads()
 {
    let document_source = resource(ContentKind::Document);
    let terminal_source = resource(ContentKind::Terminal);
    let prefix = "完整 mixed log  \n".repeat(40_000);
    assert!(prefix.len() > 512 * 1024);
    let first = page(
        &document_source,
        3,
        vec![
            (
                1,
                ContentPayload::DocumentEvent {
                    input: Box::new(ContentInput::Text {
                        text: prefix.clone(),
                    }),
                    checkpoint: Some(Box::default()),
                },
            ),
            (
                2,
                ContentPayload::DocumentEvent {
                    input: Box::new(ContentInput::Structured {
                        event: DocumentMutation::Insert {
                            after: None,
                            block: document("obsolete document").blocks.remove(0),
                        },
                    }),
                    checkpoint: Some(Box::new(document("obsolete document"))),
                },
            ),
        ],
        true,
    );
    let second = page(
        &document_source,
        4,
        vec![
            (
                3,
                ContentPayload::DocumentEvent {
                    input: Box::new(ContentInput::Structured {
                        event: DocumentMutation::Replace {
                            block: document("final 文档").blocks.remove(0),
                        },
                    }),
                    checkpoint: Some(Box::new(document("final 文档"))),
                },
            ),
            (
                4,
                ContentPayload::DocumentEvent {
                    input: Box::new(ContentInput::Text {
                        text: "late tail outside snapshot".into(),
                    }),
                    checkpoint: None,
                },
            ),
        ],
        false,
    );
    let terminal_first = page(
        &terminal_source,
        2,
        vec![(
            1,
            ContentPayload::Terminal {
                screen: screen("obsolete terminal"),
            },
        )],
        true,
    );
    let terminal_second = page(
        &terminal_source,
        3,
        vec![
            (
                2,
                ContentPayload::TerminalPatch {
                    base_cursor: ContentCursor {
                        sequence: 1,
                        ..terminal_source.cursor
                    },
                    screen: screen("final 终端"),
                    rows_changed: vec![0],
                },
            ),
            (
                3,
                ContentPayload::Terminal {
                    screen: screen("late terminal"),
                },
            ),
        ],
        false,
    );
    let (backend, server) = serve(vec![
        (
            document_source.resource_id.to_string(),
            serde_json::to_value(first).unwrap(),
        ),
        ("after=2".into(), serde_json::to_value(second).unwrap()),
        (
            terminal_source.resource_id.to_string(),
            serde_json::to_value(terminal_first).unwrap(),
        ),
        (
            "after=1".into(),
            serde_json::to_value(terminal_second).unwrap(),
        ),
    ])
    .await;
    let file = NamedTempFile::new().unwrap();
    let mut output = tokio::fs::File::from_std(file.reopen().unwrap());
    backend
        .write_export_resource(
            SESSION_ID,
            &document_source.reference(),
            &mut output,
            &I18n::english(),
        )
        .await
        .unwrap();
    backend
        .write_export_resource(
            SESSION_ID,
            &terminal_source.reference(),
            &mut output,
            &I18n::english(),
        )
        .await
        .unwrap();
    output.flush().await.unwrap();
    let body = std::fs::read_to_string(file.path()).unwrap();
    assert!(body.starts_with(&prefix));
    assert!(body.contains("final 文档"));
    assert!(body.contains("final 终端"));
    assert!(!body.contains("obsolete"));
    assert!(!body.contains("late"));
    server.await.unwrap();
}

#[tokio::test]
async fn an_export_gap_discards_the_partial_file_and_preserves_the_destination() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("report.md");
    std::fs::write(&destination, "existing report").unwrap();
    let source = resource(ContentKind::Document);
    let mut lost = page(
        &source,
        1,
        vec![(
            1,
            ContentPayload::DocumentEvent {
                input: Box::new(ContentInput::Text {
                    text: "partial".into(),
                }),
                checkpoint: Some(Box::default()),
            },
        )],
        false,
    );
    lost.gap = true;
    let (backend, server) = serve(vec![
        (
            "/parts?limit=200".into(),
            parts_page(
                vec![part(
                    2,
                    2,
                    "assistant",
                    true,
                    json!({"text": "", "resources": [source.reference()]}),
                )],
                None,
            ),
        ),
        (
            source.resource_id.to_string(),
            serde_json::to_value(lost).unwrap(),
        ),
    ])
    .await;
    let error = backend
        .prepare_transcript_export(export_request(destination.clone()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("retention gap"));
    assert_eq!(
        std::fs::read_to_string(destination).unwrap(),
        "existing report"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    server.await.unwrap();
}
