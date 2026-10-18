use std::{
    collections::HashMap,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use agena_domain::{
    ContentChunk, ContentCursor, ContentId, ContentKind, ContentPage, ContentPayload, ContentRange,
    ContentRef, ContentResource, ContentState, ContentTextPage, ContentTextPosition,
    ContentTextSlice,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

use super::{AgenaClient, COMPLETE_TEXT_PAGE_BYTES, ClientError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    None,
    Gap,
    Reset,
    Stall,
    Short,
    Skip,
}

enum Body {
    Native {
        text: String,
        initial_bytes: usize,
    },
    Log {
        records: Vec<String>,
        initial_records: usize,
    },
}

struct Source {
    id: ContentId,
    epoch: uuid::Uuid,
    body: Body,
    fault: Fault,
    calls: AtomicUsize,
}

impl Source {
    fn native(initial: String, late: &str, fault: Fault) -> Arc<Self> {
        let initial_bytes = initial.len();
        Arc::new(Self {
            id: ContentId::new(),
            epoch: uuid::Uuid::new_v4(),
            body: Body::Native {
                text: initial + late,
                initial_bytes,
            },
            fault,
            calls: AtomicUsize::new(0),
        })
    }

    fn log(mut records: Vec<String>, late: &str, fault: Fault) -> Arc<Self> {
        let initial_records = records.len();
        records.push(late.to_owned());
        Arc::new(Self {
            id: ContentId::new(),
            epoch: uuid::Uuid::new_v4(),
            body: Body::Log {
                records,
                initial_records,
            },
            fault,
            calls: AtomicUsize::new(0),
        })
    }

    fn reference(&self) -> ContentRef {
        ContentRef {
            resource_id: self.id,
            kind: match self.body {
                Body::Native { .. } => ContentKind::Text,
                Body::Log { .. } => ContentKind::Log,
            },
        }
    }

    fn resource(&self, session: i64, call: usize) -> ContentResource {
        let head = match &self.body {
            Body::Native {
                text,
                initial_bytes,
            } => {
                if call == 0 {
                    *initial_bytes
                } else {
                    text.len()
                }
            }
            Body::Log {
                records,
                initial_records,
            } => {
                if call == 0 {
                    *initial_records
                } else {
                    records.len()
                }
            }
        } as u64;
        let epoch = if call > 0 && self.fault == Fault::Reset {
            uuid::Uuid::from_u128(1)
        } else {
            self.epoch
        };
        let cursor = ContentCursor {
            epoch,
            sequence: head,
        };
        ContentResource {
            resource_id: self.id,
            owner_session_id: session,
            part_id: 37,
            kind: self.reference().kind,
            state: ContentState::Active,
            cursor,
            committed_cursor: cursor,
            total_bytes: matches!(self.body, Body::Native { .. }).then_some(head),
            dropped_bytes: 0,
            dropped_packets: 0,
            committed_dropped_bytes: 0,
            committed_dropped_packets: 0,
            retained_ranges: if head == 0 {
                Vec::new()
            } else {
                vec![ContentRange {
                    first: 1,
                    last: head,
                }]
            },
            capture_error: None,
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
struct ReadQuery {
    epoch: Option<uuid::Uuid>,
    after: Option<u64>,
    #[serde(default)]
    offset: usize,
    max_bytes: usize,
}

#[derive(Clone)]
struct HttpState {
    sources: Arc<HashMap<ContentId, Arc<Source>>>,
    requests: Arc<Mutex<Vec<(ContentId, ReadQuery)>>>,
}

struct Fixture {
    client: AgenaClient,
    state: HttpState,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn fixture(sources: &[Arc<Source>]) -> Fixture {
    let state = HttpState {
        sources: Arc::new(
            sources
                .iter()
                .map(|source| (source.id, source.clone()))
                .collect(),
        ),
        requests: Arc::new(Mutex::new(Vec::new())),
    };
    let router = Router::new()
        .route("/api/v1/sessions/{session}/content/{id}", get(native_page))
        .route(
            "/api/v1/sessions/{session}/content/{id}/text",
            get(log_page),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Fixture {
        client: AgenaClient::new(format!("http://{address}")).unwrap(),
        state,
        server,
    }
}

async fn native_page(
    State(state): State<HttpState>,
    Path((session, id)): Path<(i64, ContentId)>,
    Query(query): Query<ReadQuery>,
) -> Json<ContentPage> {
    state.requests.lock().unwrap().push((id, query.clone()));
    let source = &state.sources[&id];
    let call = source.calls.fetch_add(1, Ordering::Relaxed);
    let resource = source.resource(session, call);
    let Body::Native { text, .. } = &source.body else {
        panic!("native endpoint for a Log fixture")
    };
    let start = query.after.unwrap_or(0) as usize;
    let mut end =
        (start + query.max_bytes.clamp(4, 1024 * 1024)).min(resource.cursor.sequence as usize);
    if call > 0 && source.fault == Fault::Stall {
        end = start;
    }
    if call > 0 && source.fault == Fault::Short {
        end = start + 10;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let next = ContentCursor {
        sequence: end as u64,
        ..resource.cursor
    };
    let chunks = if end == start {
        Vec::new()
    } else {
        vec![ContentChunk {
            cursor: ContentCursor {
                sequence: next.sequence + u64::from(call > 0 && source.fault == Fault::Skip),
                ..next
            },
            captured_at_ms: 123,
            payload: ContentPayload::Text {
                text: text[start..end].to_owned(),
            },
        }]
    };
    let has_more =
        end < resource.cursor.sequence as usize && !(call > 0 && source.fault == Fault::Short);
    Json(ContentPage {
        resource,
        chunks,
        next_cursor: next,
        has_more,
        gap: call > 0 && source.fault == Fault::Gap,
    })
}

async fn log_page(
    State(state): State<HttpState>,
    Path((session, id)): Path<(i64, ContentId)>,
    Query(query): Query<ReadQuery>,
) -> Json<ContentTextPage> {
    state.requests.lock().unwrap().push((id, query.clone()));
    let source = &state.sources[&id];
    let call = source.calls.fetch_add(1, Ordering::Relaxed);
    let resource = source.resource(session, call);
    let Body::Log { records, .. } = &source.body else {
        panic!("Log endpoint for a native fixture")
    };
    let mut next = ContentTextPosition {
        after: ContentCursor {
            sequence: query.after.unwrap_or(0),
            ..resource.cursor
        },
        offset: query.offset,
    };
    let mut slices = Vec::new();
    let mut bytes = 0;
    for (index, text) in records
        .iter()
        .enumerate()
        .take(resource.cursor.sequence as usize)
        .skip(next.after.sequence as usize)
    {
        let start = if index == query.after.unwrap_or(0) as usize {
            query.offset
        } else {
            0
        };
        let mut end = (start + query.max_bytes.clamp(4, 1024 * 1024) - bytes).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end == start && start < text.len() {
            break;
        }
        let cursor = ContentCursor {
            sequence: index as u64 + 1,
            ..resource.cursor
        };
        slices.push(ContentTextSlice {
            cursor,
            captured_at_ms: 123,
            offset: start + usize::from(call > 0 && source.fault == Fault::Skip),
            stream: None,
            text: text[start..end].to_owned(),
        });
        bytes += end - start;
        if end < text.len() {
            next.offset = end;
            break;
        }
        next = ContentTextPosition {
            after: cursor,
            offset: 0,
        };
    }
    let has_more = next.after.sequence < resource.cursor.sequence;
    Json(ContentTextPage {
        resource,
        slices,
        next_position: next,
        has_more,
        gap: false,
    })
}

fn part(sources: &[Arc<Source>], inline: &str) -> agena_api::part::PartResource {
    agena_api::part::PartResource {
        part_id: 37,
        content: serde_json::json!({ "resources": sources.iter().map(|source| source.reference()).collect::<Vec<_>>(), "text": inline }),
        ..Default::default()
    }
}

#[tokio::test]
async fn complete_part_text_streams_more_than_eight_mib_with_utf8_and_a_fixed_initial_head() {
    // The first 64 KiB page ends immediately before a split three-byte scalar.
    // The last request has a one-byte budget; the server clamps that to four
    // and returns part of the late append, which must stay outside the snapshot.
    let mut text = "a".repeat(COMPLETE_TEXT_PAGE_BYTES - 1);
    text.push_str("界🙂");
    text.push_str(&"z".repeat(8 * 1024 * 1024 - 7));
    text.push('!');
    assert!(text.len() > 8 * 1024 * 1024);
    let source = Source::native(text.clone(), "LATE APPEND MUST BE EXCLUDED", Fault::None);
    let second = Source::native("\nsecond 界🙂".into(), "", Fault::None);
    let fixture = fixture(&[source.clone(), second.clone()]).await;
    let inline = "\ninline tail é";
    let part = part(&[source.clone(), second], inline);
    let mut output = Vec::new();
    let sink: &mut (dyn AsyncWrite + Unpin + Send) = &mut output;
    fixture
        .client
        .write_part_text(7, &part, sink)
        .await
        .unwrap();
    text.push_str("\nsecond 界🙂");
    text.push_str(inline);
    assert_eq!(output, text.as_bytes());
    let requests = fixture.state.requests.lock().unwrap();
    assert!(requests.len() > 128);
    assert!(
        requests
            .iter()
            .all(|(_, query)| query.max_bytes <= COMPLETE_TEXT_PAGE_BYTES)
    );
    assert!(requests.iter().any(|(_, query)| query.max_bytes == 1));
    assert!(
        requests
            .iter()
            .filter(|(id, _)| *id == source.id)
            .skip(1)
            .all(|(_, query)| query.epoch == Some(source.epoch))
    );
}

#[tokio::test]
async fn complete_part_text_reports_gap_reset_stalls_and_short_reads_with_partial_output() {
    for (fault, diagnostic) in [
        (Fault::Gap, "retention gap"),
        (Fault::Reset, "epoch reset"),
        (Fault::Stall, "no progress"),
        (Fault::Short, "initial source head"),
        (Fault::Skip, "not contiguous"),
    ] {
        let text = "p".repeat(3 * COMPLETE_TEXT_PAGE_BYTES + 1);
        let source = Source::native(text.clone(), "", fault);
        let fixture = fixture(std::slice::from_ref(&source)).await;
        let mut output = Vec::new();
        let error = fixture
            .client
            .write_part_text(7, &part(&[source], ""), &mut output)
            .await
            .unwrap_err();
        assert!(
            matches!(error, ClientError::Protocol(_)),
            "{fault:?}: {error:?}"
        );
        assert!(
            error.operator_diagnostic().contains(diagnostic),
            "{fault:?}: {error:?}"
        );
        assert_eq!(output, text.as_bytes()[..COMPLETE_TEXT_PAGE_BYTES]);
    }
}

#[tokio::test]
async fn complete_log_text_resumes_large_utf8_records_and_stops_at_its_initial_record_head() {
    let mut large = "a".repeat(COMPLETE_TEXT_PAGE_BYTES - 1);
    large.push_str("界🙂");
    large.push_str(&"é".repeat(COMPLETE_TEXT_PAGE_BYTES));
    let records = vec![large, String::new(), "\nfinal 界🙂".into()];
    let expected = records.concat();
    let source = Source::log(records, "LATE LOG RECORD MUST BE EXCLUDED", Fault::None);
    let fixture = fixture(std::slice::from_ref(&source)).await;
    let mut output = Vec::new();
    fixture
        .client
        .write_content_text(7, &source.reference(), &mut output)
        .await
        .unwrap();
    assert_eq!(output, expected.as_bytes());
    let requests = fixture.state.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|(_, query)| query.after == Some(0) && query.offset > 0)
    );
    assert!(
        requests
            .iter()
            .all(|(_, query)| query.max_bytes == COMPLETE_TEXT_PAGE_BYTES)
    );
    assert!(
        requests
            .iter()
            .skip(1)
            .all(|(_, query)| query.epoch == Some(source.epoch))
    );
}

#[tokio::test]
async fn complete_log_text_rejects_a_mismatched_partial_record_offset() {
    let source = Source::log(
        vec!["large".repeat(COMPLETE_TEXT_PAGE_BYTES)],
        "",
        Fault::Skip,
    );
    let fixture = fixture(std::slice::from_ref(&source)).await;
    let mut output = Vec::new();
    let error = fixture
        .client
        .write_content_text(7, &source.reference(), &mut output)
        .await
        .unwrap_err();
    assert!(error.operator_diagnostic().contains("not contiguous"));
    assert_eq!(output.len(), COMPLETE_TEXT_PAGE_BYTES);
}

struct FailingSink {
    bytes: Vec<u8>,
    limit: usize,
}

impl AsyncWrite for FailingSink {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let length = bytes.len().min(self.limit - self.bytes.len());
        if length == 0 {
            return Poll::Ready(Err(std::io::Error::other("injected sink failure")));
        }
        self.bytes.extend_from_slice(&bytes[..length]);
        Poll::Ready(Ok(length))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn complete_text_preserves_sink_errors_and_requires_typed_state_projection() {
    let source = Source::native("body".repeat(COMPLETE_TEXT_PAGE_BYTES), "", Fault::None);
    let fixture = fixture(std::slice::from_ref(&source)).await;
    let mut sink = FailingSink {
        bytes: Vec::new(),
        limit: 17,
    };
    let error = fixture
        .client
        .write_content_text(7, &source.reference(), &mut sink)
        .await
        .unwrap_err();
    assert!(
        error
            .operator_diagnostic()
            .contains("injected sink failure")
    );
    assert_eq!(sink.bytes, b"bodybodybodybodyb");
    assert_eq!(source.calls.load(Ordering::Relaxed), 1);
    let mut output = Vec::new();
    for kind in [
        ContentKind::Document,
        ContentKind::Structured,
        ContentKind::Terminal,
    ] {
        let error = fixture
            .client
            .write_content_text(
                7,
                &ContentRef {
                    resource_id: source.id,
                    kind,
                },
                &mut output,
            )
            .await
            .unwrap_err();
        assert!(
            error
                .operator_diagnostic()
                .contains("typed document or terminal projection")
        );
    }
    assert!(output.is_empty());
    assert_eq!(source.calls.load(Ordering::Relaxed), 1);
    sink.flush().await.unwrap();
}
