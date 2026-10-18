//! Exercise the public archive transport with real kernel state and bounded I/O.
use std::{path::Path, time::Duration};

use agena_domain::CommandId;
use agena_storage::kernel::{EntryPayload, ExecutionPhase, ExportReport, KernelError, RequestId};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::io::ReaderStream;

use super::*;

const PACKET_BYTES: usize = 64 * 1024;
const PAYLOAD_BYTES: usize = 18 * 1024 * 1024;

async fn session_count(server: &TestServer) -> u64 {
    application_for_test(&server.runtime)
        .session_kernel()
        .unwrap()
        .reader
        .workspace_statistics(&[server.workspace_id])
        .await
        .unwrap()[&server.workspace_id]
        .total
}

async fn large_text(application: &Application, session_id: i64) -> (usize, [u8; 32]) {
    let (_, attempt, round) =
        kernel_fixture::attempt(application, session_id, "large public archive fixture").await;
    let text = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let mut checksum = Sha256::new();
    let mut bytes = 0;
    for index in 0..PAYLOAD_BYTES / PACKET_BYTES {
        // Include escaping and multibyte text while keeping each admitted
        // packet bounded. The checksum is calculated before transport begins.
        let marker = format!("packet {index:06}: \\\"\t界\n");
        let packet = marker.repeat(PACKET_BYTES / marker.len());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            match text.append(&packet).await {
                Ok(_) => break,
                Err(KernelError::Busy) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(error) => panic!("large archive fixture append failed: {error}"),
            }
        }
        checksum.update(packet.as_bytes());
        bytes += packet.len();
    }
    text.finish(RequestId::new()).await.unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    (bytes, checksum.finalize().into())
}

async fn raw_import(server: &TestServer, request_id: CommandId, path: &Path) -> reqwest::Response {
    let source = tokio::fs::File::open(path).await.unwrap();
    reqwest::Client::new()
        .post(format!("{}/api/v1/sessions/import", server.url))
        .header("content-type", "application/x-ndjson")
        .header("idempotency-key", request_id.to_string())
        .body(reqwest::Body::wrap_stream(ReaderStream::with_capacity(
            source,
            PACKET_BYTES,
        )))
        .send()
        .await
        .unwrap()
}

async fn stalled_export(server: &TestServer, session_id: i64) -> TcpStream {
    let mut socket = TcpStream::connect(server.url.trim_start_matches("http://"))
        .await
        .unwrap();
    socket
        .write_all(
            format!(
                "GET /api/v1/sessions/{session_id}/export HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = BufReader::new(socket);
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut line = String::new();
        response.read_line(&mut line).await.unwrap();
        assert!(line.starts_with("HTTP/1.1 200 "), "{line}");
        let mut header_bytes = line.len();
        loop {
            line.clear();
            assert!(response.read_line(&mut line).await.unwrap() > 0);
            header_bytes += line.len();
            assert!(header_bytes < 16 * 1024);
            if line == "\r\n" {
                break;
            }
        }
    })
    .await
    .expect("archive headers must arrive before its stalled body");
    response.into_inner()
}

/// Independently verify the digest over the original bytes, including every
/// newline. Keep only one bounded record and require exactly one final footer.
async fn verify_archive(path: &Path) -> (ExportReport, u64) {
    let mut source = BufReader::new(tokio::fs::File::open(path).await.unwrap());
    let mut checksum = Sha256::new();
    let mut observed = ExportReport {
        entries: 0,
        records: 0,
        bytes: 0,
        sha256: [0; 32],
    };
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = source.read_until(b'\n', &mut line).await.unwrap();
        assert!(count > 0, "archive ended without a success footer");
        assert_eq!(line.last(), Some(&b'\n'));
        assert!(line.len() < 512 * 1024, "fixture record was not bounded");
        let record: serde_json::Value = serde_json::from_slice(&line).unwrap();
        if record["kind"] == "footer" {
            let footer: ExportReport = serde_json::from_value(record["data"].clone()).unwrap();
            observed.sha256 = checksum.finalize().into();
            assert_eq!(footer, observed, "archive footer disagrees with raw JSONL");
            line.clear();
            assert_eq!(source.read_until(b'\n', &mut line).await.unwrap(), 0);
            return (footer, observed.bytes);
        }
        observed.entries += u64::from(record["kind"] == "entry");
        observed.records += 1;
        observed.bytes += count as u64;
        checksum.update(&line);
    }
}

async fn verify_imported_text(
    application: &Application,
    session_id: i64,
    expected_bytes: usize,
    expected_digest: [u8; 32],
) {
    let entries = kernel_fixture::entries(application, session_id).await;
    let texts = entries
        .iter()
        .filter_map(|entry| match entry.payload {
            EntryPayload::Text { content_id } => Some(content_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(texts.len(), 1);
    let kernel = application.session_kernel().unwrap();
    let mut cursor = None;
    let mut checksum = Sha256::new();
    let mut bytes = 0;
    loop {
        let page = kernel
            .reader
            .text_range(
                session_id.try_into().unwrap(),
                texts[0],
                cursor,
                PACKET_BYTES,
            )
            .await
            .unwrap();
        assert!(!page.bytes.is_empty() || !page.has_more);
        checksum.update(&page.bytes);
        bytes += page.bytes.len();
        cursor = Some(page.next);
        if !page.has_more {
            break;
        }
    }
    let digest: [u8; 32] = checksum.finalize().into();
    assert_eq!((bytes, digest), (expected_bytes, expected_digest));
}

#[tokio::test]
async fn public_archive_streams_over_16_mib_with_backpressure_control_and_idempotent_import() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(server.url.as_str()).unwrap();
    let application = application_for_test(&server.runtime);
    let source = client
        .create_session(server.workspace_id, "large streaming archive", None)
        .await
        .unwrap();
    let (expected_bytes, expected_digest) = large_text(&application, source.id).await;
    assert!(expected_bytes > 16 * 1024 * 1024);
    let (execution, attempt, round) =
        kernel_fixture::attempt(&application, source.id, "cancel during slow export").await;
    let http = reqwest::Client::new();
    let export_url = format!("{}/api/v1/sessions/{}/export", server.url, source.id);

    // The two unread loopback responses exceed the socket and duplex buffers,
    // keeping both native export permits live without a timing assumption.
    // Own the first TCP connection directly so dropping it below closes the
    // transport rather than relying on a pooled client's body-drop policy.
    let first = stalled_export(&server, source.id).await;
    let mut second = http.get(&export_url).send().await.unwrap();
    assert_eq!(second.status(), reqwest::StatusCode::OK);
    assert_eq!(second.headers()["content-type"], "application/x-ndjson");
    let excess = tokio::time::timeout(Duration::from_secs(10), http.get(&export_url).send())
        .await
        .expect("export admission must fail without waiting for a slow consumer")
        .unwrap();
    assert!(!excess.status().is_success());

    let controls = async {
        for _ in 0..3 {
            let state = http
                .get(format!(
                    "{}/api/v1/sessions/{}/state",
                    server.url, source.id
                ))
                .send()
                .await
                .unwrap();
            assert_eq!(state.status(), reqwest::StatusCode::OK);
            let _: serde_json::Value = state.json().await.unwrap();
        }
        client
            .cancel_run(source.id, Some(execution.id))
            .await
            .unwrap();
    };
    tokio::time::timeout(Duration::from_secs(10), controls)
        .await
        .expect("control reads and cancellation progress while archives are backpressured");
    assert_eq!(
        application
            .session_kernel()
            .unwrap()
            .reader
            .execution(source.id.try_into().unwrap(), execution.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Cancelled
    );
    drop((round, attempt, first));

    // Disconnect must abort the producer and release its admission even while
    // the other client remains stalled. Successful replacement proves reuse.
    let mut rejections = 0_usize;
    let mut last_rejection = String::new();
    let replacement = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let mut response = http.get(&export_url).send().await.unwrap();
            if response.status().is_success() {
                break response;
            }
            rejections += 1;
            last_rejection = response.status().to_string();
            if let Some(body) = response.chunk().await.unwrap() {
                last_rejection.push_str(": ");
                last_rejection.push_str(&String::from_utf8_lossy(&body[..body.len().min(2048)]));
            }
            if response.status().is_server_error()
                && response.status() != reqwest::StatusCode::SERVICE_UNAVAILABLE
            {
                // The public problem deliberately scrubs SQL diagnostics.
                // Probe the same native snapshot path only after an unexpected
                // HTTP failure, so its operator error survives a test failure.
                let native = match application
                    .session_kernel()
                    .unwrap()
                    .reader
                    .history_snapshot(source.id.try_into().unwrap())
                    .await
                {
                    Ok(snapshot) => format!("snapshot admitted: {:?}", snapshot.info()),
                    Err(error) => format!("{error:?}"),
                };
                panic!("replacement archive failed: {last_rejection}; native: {native}");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|error| {
        panic!(
            "disconnected archive must release its export permit: {error}; {rejections} rejections; last response: {last_rejection}"
        )
    });
    drop(replacement);

    let files = tempfile::tempdir().unwrap();
    let archive_path = files.path().join("large.jsonl");
    let mut archive = tokio::fs::File::create(&archive_path).await.unwrap();
    let mut wire_bytes = 0;
    let mut chunks = 0_usize;
    while let Some(chunk) = second.chunk().await.unwrap() {
        archive.write_all(&chunk).await.unwrap();
        wire_bytes += chunk.len();
        chunks += 1;
        if chunks.is_multiple_of(32) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
    archive.flush().await.unwrap();
    drop(archive);
    assert!(wire_bytes > 16 * 1024 * 1024);
    assert!(chunks > 16, "HTTP response must arrive as multiple chunks");
    let (report, _) = verify_archive(&archive_path).await;
    assert!(report.entries >= 3);

    let before = session_count(&server).await;
    let key = CommandId::new();
    let imported = client
        .import_session_archive(key, tokio::fs::File::open(&archive_path).await.unwrap())
        .await
        .unwrap();
    assert_ne!(imported.session_id, source.id);
    assert_eq!(session_count(&server).await, before + 1);
    verify_imported_text(
        &application,
        imported.session_id,
        expected_bytes,
        expected_digest,
    )
    .await;
    let replayed = raw_import(&server, key, &archive_path).await;
    assert_eq!(replayed.status(), reqwest::StatusCode::OK);
    let replayed = replayed.bytes().await.unwrap();
    assert!(
        replayed.len() < 128,
        "large archive receipt must stay bounded"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&replayed).unwrap(),
        serde_json::json!({"session_id": imported.session_id})
    );
    assert_eq!(session_count(&server).await, before + 1);

    let different = client
        .create_session(server.workspace_id, "different archive digest", None)
        .await
        .unwrap();
    let other_path = files.path().join("other.jsonl");
    let mut other = tokio::fs::File::create(&other_path).await.unwrap();
    client
        .export_session_archive(different.id, &mut other)
        .await
        .unwrap();
    drop(other);
    let before_conflict = session_count(&server).await;
    let conflict = raw_import(&server, key, &other_path).await;
    assert_eq!(conflict.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(session_count(&server).await, before_conflict);
}

#[tokio::test]
async fn public_archive_rejects_invalid_or_interrupted_uploads_without_publishing_sessions() {
    let server = start_test_server("http://127.0.0.1:9").await;
    let client = AgenaClient::new(server.url.as_str()).unwrap();
    let application = application_for_test(&server.runtime);
    let source = client
        .create_session(server.workspace_id, "archive rejection fixture", None)
        .await
        .unwrap();
    kernel_fixture::completed_input(&application, source.id, "small archive input").await;
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("valid.jsonl");
    let mut archive = tokio::fs::File::create(&path).await.unwrap();
    client
        .export_session_archive(source.id, &mut archive)
        .await
        .unwrap();
    drop(archive);
    let (mut report, footer_offset) = verify_archive(&path).await;
    let baseline = session_count(&server).await;
    let http = reqwest::Client::new();
    let url = format!("{}/api/v1/sessions/import", server.url);
    for key in [None, Some("not-a-uuid")] {
        let mut request = http
            .post(&url)
            .header("content-type", "application/x-ndjson");
        if let Some(key) = key {
            request = request.header("idempotency-key", key);
        }
        assert_eq!(
            request.body("{}\n").send().await.unwrap().status(),
            reqwest::StatusCode::BAD_REQUEST
        );
    }
    let unknown = http
        .get(format!(
            "{}/api/v1/sessions/{}/export",
            server.url,
            i64::MAX
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), reqwest::StatusCode::NOT_FOUND);

    let truncated = files.path().join("truncated.jsonl");
    tokio::fs::copy(&path, &truncated).await.unwrap();
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&truncated)
        .await
        .unwrap();
    file.set_len(footer_offset).await.unwrap();
    drop(file);
    assert_eq!(
        raw_import(&server, CommandId::new(), &truncated)
            .await
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(session_count(&server).await, baseline);

    let corrupted = files.path().join("corrupted.jsonl");
    tokio::fs::copy(&truncated, &corrupted).await.unwrap();
    let mut file = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&corrupted)
        .await
        .unwrap();
    report.sha256[0] ^= 0xff;
    let mut footer =
        serde_json::to_vec(&serde_json::json!({"kind":"footer", "data":report})).unwrap();
    footer.push(b'\n');
    file.write_all(&footer).await.unwrap();
    file.flush().await.unwrap();
    drop(file);
    assert_eq!(
        raw_import(&server, CommandId::new(), &corrupted)
            .await
            .status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    assert_eq!(session_count(&server).await, baseline);

    // Close a real upload after delivering a complete first record while its
    // declared body is still incomplete. The same key must remain usable.
    let key = CommandId::new();
    let mut socket = TcpStream::connect(server.url.trim_start_matches("http://"))
        .await
        .unwrap();
    let mut source_file = BufReader::new(tokio::fs::File::open(&path).await.unwrap());
    let mut prefix = Vec::new();
    source_file.read_until(b'\n', &mut prefix).await.unwrap();
    let headers = format!(
        "POST /api/v1/sessions/import HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-ndjson\r\nIdempotency-Key: {key}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        prefix.len() + 1024
    );
    socket.write_all(headers.as_bytes()).await.unwrap();
    socket.write_all(&prefix).await.unwrap();
    socket.shutdown().await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut response))
        .await
        .expect("server must release the interrupted HTTP upload")
        .unwrap();
    drop(socket);
    assert_eq!(session_count(&server).await, baseline);
    let accepted = raw_import(&server, key, &path).await;
    assert_eq!(accepted.status(), reqwest::StatusCode::OK);
    let imported: agena_api::resource::SessionArchiveImportResource =
        accepted.json().await.unwrap();
    assert_ne!(imported.session_id, source.id);
    assert_eq!(session_count(&server).await, baseline + 1);
}

#[tokio::test]
async fn public_archive_sdk_rejects_clean_http_eof_without_a_complete_footer() {
    // A proxy can terminate a valid HTTP body cleanly after receiving only a
    // prefix. Transport EOF alone cannot attest archive success.
    let header = "{\"kind\":\"header\",\"data\":{}}\n";
    let header_digest: [u8; 32] = Sha256::digest(header.as_bytes()).into();
    let valid_footer = serde_json::json!({
        "kind":"footer", "data": {
            "entries":0, "records":1, "bytes":header.len(), "sha256":header_digest
        }
    });
    let mut cases = vec![
        header.to_owned(),
        format!("{header}{{\"kind\":\"footer\",\"data\":"),
        format!("{header}{{\"kind\":\"footer\",\"data\":{{}}}}\n"),
        format!("{header}{{\"kind\":\"footer\"}}\n"),
        format!("{header}{valid_footer}"),
        format!("{header}{valid_footer}\nx"),
        format!("{header}{valid_footer}\n\n"),
        format!("{header}{valid_footer}\n \n"),
        format!("{header}{valid_footer}\n{{\"kind\":\"entry\",\"data\":{{}}}}\n"),
    ];
    for field in ["entries", "records", "bytes", "sha256"] {
        let mut footer = valid_footer.clone();
        footer["data"].as_object_mut().unwrap().remove(field);
        cases.push(format!("{header}{footer}\n"));
    }
    for invalid_count in [
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!("0"),
        serde_json::json!(null),
    ] {
        let mut footer = valid_footer.clone();
        footer["data"]["records"] = invalid_count;
        cases.push(format!("{header}{footer}\n"));
    }
    for invalid_hash in [
        serde_json::json!(vec![0_u8; 31]),
        serde_json::json!(vec![0_u8; 33]),
        serde_json::json!("00000000000000000000000000000000"),
        serde_json::json!(vec![256_u16; 32]),
        serde_json::json!(vec![-1_i16; 32]),
    ] {
        let mut footer = valid_footer.clone();
        footer["data"]["sha256"] = invalid_hash;
        cases.push(format!("{header}{footer}\n"));
    }
    let mut cases = cases
        .into_iter()
        .map(|body| (body, false))
        .collect::<Vec<_>>();
    cases.push((format!("{header}{valid_footer}\n"), true));
    for (body, valid) in cases {
        let expected = body.clone();
        let app = axum::Router::new().route(
            "/api/v1/sessions/1/export",
            axum::routing::get(move || {
                let body = body.clone();
                async move { ([("content-type", "application/x-ndjson")], body) }
            }),
        );
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = AgenaClient::new(format!("http://{address}")).unwrap();
        let mut target = Vec::new();
        let result = client.export_session_archive(1, &mut target).await;
        server.abort();
        let _ = server.await;
        if valid {
            assert!(
                result.is_ok(),
                "complete typed footer must succeed: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(agena_client::ClientError::Protocol(_))),
                "clean EOF without a complete footer must fail: {result:?}"
            );
        }
        assert_eq!(target, expected.as_bytes());
    }
}
