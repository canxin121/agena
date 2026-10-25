use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use agena_plugin_sdk::hooks::{EventEnvelope, EventFilter};
use agena_plugin_sdk::host_api::*;
use agena_plugin_sdk::{InitContext, Plugin, Result, ToolInvokeInput, ToolInvokeOutput};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
mod more;

struct Host {
    connection: Mutex<BuiltinProviderConnection>,
    resolutions: Mutex<usize>,
    network: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl HostClient for Host {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> Result<EventSubscription> {
        Ok(EventSubscription { id: "test".into() })
    }
    async fn read_config(&self, _: Option<String>) -> Result<Value> {
        unreachable!()
    }
    async fn invoke_tool(&self, _: String, _: Value) -> Result<ToolInvokeOutput> {
        unreachable!()
    }
    async fn check_path_permission(&self, _: PathPermissionQuery) -> Result<PermissionQuery> {
        unreachable!()
    }
    async fn require_path_permission(&self, request: PathPermissionQuery) -> Result<()> {
        if request.path.ends_with("denied-input") {
            return Err(agena_plugin_sdk::PluginError::from_kind(
                agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                "input access denied",
            ));
        }
        Ok(())
    }
    async fn require_network_permission(&self, url: String) -> Result<()> {
        if url.contains("/deny-network/") {
            return Err(agena_plugin_sdk::PluginError::from_kind(
                agena_plugin_sdk::PluginErrorKind::PolicyDenied,
                "network access denied",
            ));
        }
        self.network.lock().unwrap().push(url);
        Ok(())
    }
    async fn builtin_provider_connection(&self) -> Result<BuiltinProviderConnection> {
        *self.resolutions.lock().unwrap() += 1;
        Ok(self.connection.lock().unwrap().clone())
    }
}

fn plugins() -> Vec<Box<dyn Plugin>> {
    vec![
        Box::new(super::chatgpt::ChatGptToolsPlugin::new()),
        Box::new(super::claude::ClaudeToolsPlugin::new()),
        Box::new(super::gemini::GeminiToolsPlugin::new()),
    ]
}

async fn initialize(plugin: &dyn Plugin, root: &Path, host: Arc<Host>) {
    let manifest = plugin.manifest();
    plugin
        .init(
            InitContext {
                agena_version: "test".into(),
                workspace_root: root.into(),
                plugin_id: agena_plugin_sdk::PluginKey::new(&manifest.namespace, &manifest.name)
                    .unwrap(),
                host_callback_url: None,
                host_callback_token: None,
                settings: json!({}),
                protocol_version: agena_plugin_sdk::rpc::PROTOCOL_VERSION,
            },
            host,
        )
        .await
        .unwrap();
}

async fn invoke(
    plugin: &dyn Plugin,
    root: &Path,
    name: &str,
    input: Value,
) -> Result<ToolInvokeOutput> {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        plugin.tool_invoke(ToolInvokeInput {
            tool_name: name.into(),
            session_id: 1,
            call_id: 2,
            workspace_root: root.to_string_lossy().into_owned(),
            input,
        }),
    )
    .await
    .unwrap()
}

async fn server(response: Value) -> (String, tokio::task::JoinHandle<(String, Vec<u8>)>) {
    server_bytes("application/json", response.to_string().into_bytes()).await
}

async fn server_bytes(
    mime: &str,
    response: Vec<u8>,
) -> (String, tokio::task::JoinHandle<(String, Vec<u8>)>) {
    let mime = mime.to_owned();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let (header, body) = loop {
            let mut bytes = [0u8; 8192];
            let size = socket.read(&mut bytes).await.unwrap();
            assert_ne!(size, 0);
            request.extend_from_slice(&bytes[..size]);
            if let Some(offset) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let header = String::from_utf8(request[..offset].to_vec()).unwrap();
                let size: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::parse)
                    })
                    .unwrap_or(Ok(0))
                    .unwrap();
                if request.len() >= offset + 4 + size {
                    break (header, request[offset + 4..offset + 4 + size].to_vec());
                }
            }
        };
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).as_bytes()).await.unwrap();
        socket.write_all(&response).await.unwrap();
        (header, body)
    });
    (base, handle)
}

fn connection(base: String, model: Option<&str>) -> BuiltinProviderConnection {
    BuiltinProviderConnection {
        provider_id: "gateway-a".into(),
        messages_url: None,
        realtime_ws_url: None,
        base_url: base,
        headers: BTreeMap::from([
            ("x-gateway-token".into(), "provider-secret".into()),
            ("x-session-id".into(), "1".into()),
        ]),
        model: model.map(str::to_owned),
        timeout_secs: 5,
        connect_timeout_secs: 2,
        receipt_key: "provider-secret".into(),
    }
}

#[tokio::test]
async fn provider_tools_follow_the_live_connection_and_model_override() {
    for plugin in plugins() {
        let name = plugin.manifest().name;
        let (version, endpoint, tool) = match name.as_str() {
            "chatgpt" => ("v1", "responses", "web_search"),
            "claude" => ("v1", "messages", "web_search"),
            _ => ("v1beta", "interactions", "google_search"),
        };
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host {
            connection: Mutex::new(connection(
                "http://127.0.0.1:1/uninitialized".into(),
                Some("old-model"),
            )),
            resolutions: Mutex::new(0),
            network: Mutex::new(Vec::new()),
        });
        initialize(plugin.as_ref(), root.path(), host.clone()).await;
        assert_eq!(*host.resolutions.lock().unwrap(), 0);
        for (route, requested, expected) in [
            ("first", None, "provider-model"),
            ("second", Some("team/override"), "team/override"),
        ] {
            let (base, received) =
                server(json!({"status":"completed","output_text":"done","stop_reason":"end_turn"}))
                    .await;
            *host.connection.lock().unwrap() =
                connection(format!("{base}/{route}/{version}"), Some("provider-model"));
            let output = invoke(
                plugin.as_ref(),
                root.path(),
                tool,
                json!({"prompt":"test", "model":requested}),
            )
            .await
            .unwrap();
            assert!(
                !serde_json::to_string(&output)
                    .unwrap()
                    .contains("provider-secret")
            );
            let (header, body) = received.await.unwrap();
            assert!(
                header.starts_with(&format!("POST /{route}/{version}/{endpoint} HTTP/1.1")),
                "{header}"
            );
            assert!(
                header
                    .to_ascii_lowercase()
                    .contains("x-gateway-token: provider-secret"),
                "{header}"
            );
            assert!(header.to_ascii_lowercase().contains("x-session-id: 1"));
            assert!(!header.to_ascii_lowercase().contains("authorization:"));
            let body: Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["model"], expected);
        }
        assert_eq!(*host.resolutions.lock().unwrap(), 2);
        assert_eq!(host.network.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn uploads_need_no_model_and_handles_stay_bound_to_the_provider() {
    for plugin in plugins().into_iter().take(2) {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("input.txt"), "explicit input").unwrap();
        let (base, received) = server(json!({"id":"file-test","status":"processed"})).await;
        let host = Arc::new(Host {
            connection: Mutex::new(connection(format!("{base}/v1"), None)),
            resolutions: Mutex::new(0),
            network: Mutex::new(Vec::new()),
        });
        initialize(plugin.as_ref(), root.path(), host.clone()).await;
        let output = invoke(
            plugin.as_ref(),
            root.path(),
            "file_upload",
            json!({"path":"input.txt"}),
        )
        .await
        .unwrap();
        let (header, body) = received.await.unwrap();
        assert!(header.starts_with("POST /v1/files HTTP/1.1"), "{header}");
        assert!(
            header
                .to_ascii_lowercase()
                .contains("x-gateway-token: provider-secret"),
            "{header}"
        );
        assert!(String::from_utf8_lossy(&body).contains("explicit input"));
        let handle = output.payload.unwrap()["handle"]
            .as_str()
            .unwrap()
            .to_owned();
        // Even a provider sharing the URL and key cannot reuse this receipt.
        host.connection.lock().unwrap().provider_id = "gateway-b".into();
        let error = invoke(
            plugin.as_ref(),
            root.path(),
            "file_status",
            json!({"handle":handle}),
        )
        .await
        .unwrap_err();
        assert!(error.failure.user.fallback.contains("not owned"));
        assert_eq!(host.network.lock().unwrap().len(), 1);
    }
}

#[test]
fn provider_plugin_contracts_remove_connection_settings_and_cloud_prefixes() {
    let mut count = 0;
    for plugin in plugins() {
        let manifest = plugin.manifest();
        let settings = serde_json::to_string(&manifest.settings).unwrap();
        for field in ["base_url", "api_key_env", "timeout_secs", "image_model"] {
            assert!(!settings.contains(field), "{field}: {settings}");
        }
        for tool in manifest.tools {
            assert!(!tool.name.starts_with("cloud_"));
            count += 1;
        }
    }
    assert_eq!(count, 74);
}

#[test]
fn model_tools_document_the_session_default_override_and_vendor_requirement() {
    let mut model_tools = 0;
    for plugin in plugins() {
        let mut manifest = plugin.manifest();
        super::provider_tool_docs::localize_provider_manifest(&mut manifest);
        let vendor = match manifest.name.as_str() {
            "chatgpt" => "OpenAI",
            "claude" => "Anthropic Claude",
            "gemini" => "Google Gemini",
            _ => unreachable!(),
        };
        for tool in &manifest.tools {
            let has_model = tool.contract.input_schema["properties"]
                .get("model")
                .is_some();
            if matches!(
                tool.name.as_str(),
                "file_upload" | "file_status" | "file_delete"
            ) {
                assert!(!has_model);
                continue;
            }
            assert!(
                has_model,
                "{}.{} must accept model",
                manifest.name, tool.name
            );
            let help = tool.docs.help.as_deref().unwrap();
            assert!(help.contains("current session model"));
            assert!(help.contains("explicitly pass model"));
            assert!(help.contains(vendor));
            let chinese = tool.docs.translations["zh-CN"].help.as_deref().unwrap();
            assert!(chinese.contains("默认使用当前会话模型"));
            assert!(chinese.contains("显式传入 model"));
            assert!(chinese.contains(vendor));
            model_tools += 1;
        }
    }
    assert_eq!(model_tools, 65);
}

#[test]
fn gemini_model_alias_cannot_escape_the_provider_path() {
    let endpoint = super::official_service::gemini_model_endpoint(
        "https://gateway.example/proxy/v1beta",
        "models/team/alias?token=ignored#fragment",
    )
    .unwrap();
    assert_eq!(
        endpoint,
        "https://gateway.example/proxy/v1beta/models/team%2Falias%3Ftoken=ignored%23fragment:generateContent"
    );
}

#[tokio::test]
async fn resource_apis_use_provider_auth_and_the_correct_model_protocol() {
    let cases = [
        (
            0,
            "embeddings",
            json!({"model":"embedding-model","parameters":{"input":"hello"}}),
            "/v1/embeddings",
            Some("embedding-model"),
        ),
        (
            1,
            "inference",
            json!({"prompt":"make a chart","parameters":{"container":{"skills":[{"type":"anthropic","skill_id":"xlsx","version":"latest"}]},"tools":[{"type":"code_execution_20260521","name":"code_execution"}]}}),
            "/v1/messages",
            Some("current-model"),
        ),
        (
            2,
            "embeddings",
            json!({"model":"team/embed","parameters":{"content":{"parts":[{"text":"hello"}]}}}),
            "/v1beta/models/team%2Fembed:embedContent",
            None,
        ),
        (
            2,
            "interactions",
            json!({"action":"create","parameters":{"agent":"deep-research-preview-04-2026","input":"research","background":true}}),
            "/v1beta/interactions",
            None,
        ),
    ];
    for (index, tool, args, path, expected_model) in cases {
        let plugin = plugins().remove(index);
        let root = tempfile::tempdir().unwrap();
        let (base, received) =
            server(json!({"id":"resource_1","status":"completed","output_text":"ok"})).await;
        let host = Arc::new(Host {
            connection: Mutex::new(connection(
                format!("{base}/{}", if index == 2 { "v1beta" } else { "v1" }),
                Some("current-model"),
            )),
            resolutions: Mutex::new(0),
            network: Mutex::new(Vec::new()),
        });
        initialize(plugin.as_ref(), root.path(), host.clone()).await;
        let output = invoke(plugin.as_ref(), root.path(), tool, args)
            .await
            .unwrap();
        assert!(
            !serde_json::to_string(&output)
                .unwrap()
                .contains("provider-secret")
        );
        let (header, body) = received.await.unwrap();
        assert!(
            header.starts_with(&format!("POST {path} HTTP/1.1")),
            "{header}"
        );
        assert!(
            header
                .to_lowercase()
                .contains("x-gateway-token: provider-secret")
        );
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body.get("model").and_then(Value::as_str), expected_model);
        if tool == "inference" {
            assert_eq!(body["container"]["skills"][0]["skill_id"], "xlsx");
            assert_eq!(body["stream"], false);
        }
    }
}

#[tokio::test]
async fn generated_files_and_audio_download_actual_bytes_without_a_model() {
    for (index, tool, args, path, mime, bytes) in [
        (
            0,
            "containers",
            json!({"action":"download_file","id":"cntr_1","item_id":"cfile_1"}),
            "/v1/containers/cntr_1/files/cfile_1/content",
            "text/csv",
            b"a,b\n1,2\n".to_vec(),
        ),
        (
            1,
            "files",
            json!({"action":"download","id":"file_1"}),
            "/v1/files/file_1/content",
            "application/pdf",
            b"%PDF-test".to_vec(),
        ),
        (
            0,
            "speech",
            json!({"model":"speech-model","prompt":"hello","parameters":{"voice":"alloy"}}),
            "/v1/audio/speech",
            "audio/wav",
            b"RIFF-test-audio".to_vec(),
        ),
    ] {
        let plugin = plugins().remove(index);
        let root = tempfile::tempdir().unwrap();
        let (base, received) = server_bytes(mime, bytes.clone()).await;
        let host = Arc::new(Host {
            connection: Mutex::new(connection(format!("{base}/v1"), None)),
            resolutions: Mutex::new(0),
            network: Mutex::new(Vec::new()),
        });
        initialize(plugin.as_ref(), root.path(), host).await;
        let output = invoke(plugin.as_ref(), root.path(), tool, args)
            .await
            .unwrap();
        let (header, _) = received.await.unwrap();
        assert!(header.contains(path), "{header}");
        assert_eq!(output.attachments.len(), 1);
        let agena_plugin_sdk::attachment::AttachmentSource::LocalPath { path } =
            &output.attachments[0].source
        else {
            panic!("not persisted")
        };
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        assert_eq!(output.attachments[0].mime, mime);
    }
}

#[tokio::test]
async fn generic_file_upload_accepts_binary_datasets_and_default_model_is_not_sent() {
    let plugin = plugins().remove(0);
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("dataset.xlsx"),
        b"PK\x03\x04\x00binary-dataset",
    )
    .unwrap();
    let (base, received) = server(json!({"id":"file_data"})).await;
    let host = Arc::new(Host {
        connection: Mutex::new(connection(
            format!("{base}/v1"),
            Some("unrelated-chat-model"),
        )),
        resolutions: Mutex::new(0),
        network: Mutex::new(Vec::new()),
    });
    initialize(plugin.as_ref(), root.path(), host).await;
    invoke(
        plugin.as_ref(),
        root.path(),
        "files",
        json!({"action":"upload","path":"dataset.xlsx","parameters":{"purpose":"assistants"}}),
    )
    .await
    .unwrap();
    let (header, body) = received.await.unwrap();
    assert!(header.starts_with("POST /v1/files HTTP/1.1"));
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("binary-dataset"));
    assert!(body.contains("assistants"));
    assert!(!body.contains("unrelated-chat-model"));
}

#[tokio::test]
async fn realtime_uses_current_provider_waits_for_setup_and_saves_bounded_wav() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    for index in [0, 2] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mock = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket=tokio_tungstenite::accept_hdr_async(stream,move |request:&tokio_tungstenite::tungstenite::handshake::server::Request,response| {
                assert_eq!(request.headers().get("x-gateway-token").unwrap(),"provider-secret");
                assert_eq!(request.headers().get("x-agena-provider-api").unwrap(),if index==0 {"chatgpt"} else {"gemini"});
                if index==0 {assert_eq!(request.uri().path(),"/v1/realtime");assert_eq!(request.uri().query(),Some("model=voice-model"));}
                else {assert_eq!(request.uri().path(),"/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent");}
                Ok(response)
            }).await.unwrap();
            let setup: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            if index == 0 {
                assert_eq!(setup["session"]["model"], "voice-model");
            } else {
                assert_eq!(setup["setup"]["model"], "models/voice-model");
            }
            socket
                .send(Message::Text(
                    if index == 0 {
                        json!({"type":"session.updated"})
                    } else {
                        json!({"setupComplete":{}})
                    }
                    .to_string()
                    .into(),
                ))
                .await
                .unwrap();
            let submitted: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            if index == 0 {
                assert_eq!(submitted["type"], "conversation.item.create");
                let request: Value =
                    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(request["type"], "response.create");
                for event in [
                    json!({"type":"response.output_audio.delta","delta":"AQIDBA=="}),
                    json!({"type":"response.output_audio_transcript.delta","delta":"hello"}),
                    json!({"type":"response.done","response":{"status":"completed","usage":{"input_tokens":5,"output_tokens":8}}}),
                ] {
                    socket
                        .send(Message::Text(event.to_string().into()))
                        .await
                        .unwrap();
                }
            } else {
                assert_eq!(submitted["clientContent"]["turnComplete"], true);
                socket.send(Message::Text(json!({"serverContent":{"modelTurn":{"parts":[{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"AQIDBA=="}}]},"outputTranscription":{"text":"hello"},"turnComplete":true},"usageMetadata":{"promptTokenCount":5,"candidatesTokenCount":8}}).to_string().into())).await.unwrap();
            }
            let _ = socket.next().await;
        });
        let plugin = plugins().remove(index);
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host {
            connection: Mutex::new(connection(
                format!("http://{addr}/{}", if index == 0 { "v1" } else { "v1beta" }),
                Some("unrelated-model"),
            )),
            resolutions: Mutex::new(0),
            network: Mutex::new(Vec::new()),
        });
        initialize(plugin.as_ref(), root.path(), host.clone()).await;
        let output = invoke(
            plugin.as_ref(),
            root.path(),
            "realtime",
            json!({"model":"voice-model","text":"hi","duration_seconds":3}),
        )
        .await
        .unwrap();
        mock.await.unwrap();
        let serialized = serde_json::to_string(&output).unwrap();
        assert!(serialized.contains("hello"));
        assert!(serialized.contains("\"completed\":true"));
        assert!(!serialized.contains("AQIDBA=="));
        assert!(!serialized.contains("provider-secret"));
        assert_eq!(host.network.lock().unwrap().len(), 1);
        assert_eq!(output.attachments.len(), 1);
        let agena_plugin_sdk::attachment::AttachmentSource::LocalPath { path } =
            &output.attachments[0].source
        else {
            panic!("no audio file")
        };
        let wav = std::fs::read(path).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[44..], &[1, 2, 3, 4]);
        assert!(
            output
                .metadata
                .contains_key(agena_provider::PROVIDER_TOOL_USAGE_METADATA_KEY)
        );
    }
}

#[tokio::test]
async fn knowledge_and_media_workflows_preserve_vendor_payloads_and_redact_all_outputs() {
    for (index, tool, args, path, response) in [
        (
            0,
            "knowledge",
            json!({"action":"add_file","id":"vs_test","parameters":{"file_id":"file_test"}}),
            "/v1/vector_stores/vs_test/files",
            json!({"id":"file_test","status":"in_progress"}),
        ),
        (
            2,
            "knowledge",
            json!({"action":"import_file","id":"store","parameters":{"fileName":"files/dataset"}}),
            "/v1beta/fileSearchStores/store:importFile",
            json!({"name":"fileSearchStores/store/operations/import"}),
        ),
        (
            2,
            "speech",
            json!({"prompt":"hi","parameters":{"generation_config":{"speech_config":[{"voice":"Kore"}]}}}),
            "/v1beta/interactions",
            json!({"id":"audio1","steps":[{"content":[{"type":"audio","mime_type":"audio/pcm;rate=24000","data":"AQIDBA=="},{"type":"audio","mime_type":"audio/wav","data":"d2F2"}]}]}),
        ),
        (
            1,
            "batches",
            json!({"action":"create","parameters":{"requests":[{"custom_id":"one","params":{"messages":[],"max_tokens":20}}]}}),
            "/v1/messages/batches",
            json!({"id":"batch_test","processing_status":"in_progress"}),
        ),
    ] {
        let plugin = plugins().remove(index);
        let root = tempfile::tempdir().unwrap();
        let (base, received) = server(response).await;
        let host = Arc::new(Host {
            connection: Mutex::new(connection(
                format!("{base}/{}", if index == 2 { "v1beta" } else { "v1" }),
                Some("current-model"),
            )),
            resolutions: Mutex::new(0),
            network: Mutex::new(Vec::new()),
        });
        initialize(plugin.as_ref(), root.path(), host).await;
        let output = invoke(plugin.as_ref(), root.path(), tool, args)
            .await
            .unwrap();
        let (header, body) = received.await.unwrap();
        assert!(
            header.starts_with(&format!("POST {path} HTTP/1.1")),
            "{header}"
        );
        let body: Value = serde_json::from_slice(&body).unwrap();
        if tool == "speech" {
            assert_eq!(body["response_format"]["type"], "audio");
            assert_eq!(body["model"], "current-model");
            assert_eq!(output.attachments.len(), 2);
            assert_eq!(output.attachments[0].mime, "audio/wav");
            let serialized = serde_json::to_string(&output).unwrap();
            assert!(!serialized.contains("AQIDBA=="));
        } else if tool == "batches" {
            assert_eq!(body["requests"][0]["params"]["model"], "current-model");
            assert!(body.get("model").is_none());
        } else {
            assert!(body.get("model").is_none());
            assert!(
                serde_json::to_string(&output)
                    .unwrap()
                    .contains("\"pending\":true")
            );
        }
    }
}

#[tokio::test]
async fn resource_wait_retains_pending_id_when_polling_fails_without_resubmitting() {
    let plugin = plugins().remove(0);
    let root = tempfile::tempdir().unwrap();
    let (base, received) = server(json!({"id":"resp_pending","status":"in_progress"})).await;
    let host = Arc::new(Host {
        connection: Mutex::new(connection(format!("{base}/v1"), None)),
        resolutions: Mutex::new(0),
        network: Mutex::new(Vec::new()),
    });
    initialize(plugin.as_ref(), root.path(), host).await;
    let output = invoke(
        plugin.as_ref(),
        root.path(),
        "responses",
        json!({"action":"wait","id":"resp_pending","wait_seconds":3}),
    )
    .await
    .unwrap();
    assert!(
        received
            .await
            .unwrap()
            .0
            .starts_with("GET /v1/responses/resp_pending ")
    );
    let output = serde_json::to_string(&output).unwrap();
    assert!(output.contains("resp_pending"));
    assert!(output.contains("\"pending\":true"));
    assert!(output.contains("poll_warning"));
}

#[tokio::test]
async fn resource_uploads_and_requests_stop_at_permission_denial() {
    for (base, args, expected) in [
        (
            "http://127.0.0.1:1/v1",
            json!({"action":"upload","path":"denied-input"}),
            "input access denied",
        ),
        (
            "http://127.0.0.1:1/deny-network/v1",
            json!({"action":"list"}),
            "network access denied",
        ),
    ] {
        let plugin = plugins().remove(0);
        let root = tempfile::tempdir().unwrap();
        let host = Arc::new(Host {
            connection: Mutex::new(connection(base.into(), None)),
            resolutions: Mutex::new(0),
            network: Mutex::new(Vec::new()),
        });
        initialize(plugin.as_ref(), root.path(), host).await;
        let err = invoke(plugin.as_ref(), root.path(), "files", args)
            .await
            .unwrap_err();
        assert_eq!(err.failure.user.fallback, expected);
        assert_eq!(err.kind, agena_plugin_sdk::PluginErrorKind::PolicyDenied);
    }
}

#[tokio::test]
async fn realtime_retains_partial_audio_and_does_not_report_failed_turns_as_completed() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    for failed in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mock = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = socket.next().await.unwrap().unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"session.updated"}).to_string().into(),
                ))
                .await
                .unwrap();
            for _ in 0..2 {
                let _ = socket.next().await.unwrap().unwrap();
            }
            socket
                .send(Message::Text(
                    json!({"type":"response.output_audio.delta","delta":"AQIDBA=="})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            if failed {
                socket
                    .send(Message::Text(
                        json!({"type":"response.done","response":{"status":"failed"}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
            }
            let _ = socket.next().await;
        });
        let root = tempfile::tempdir().unwrap();
        let plugin = plugins().remove(0);
        let host = Arc::new(Host {
            connection: Mutex::new(connection(format!("http://{addr}/v1"), Some("voice-model"))),
            resolutions: Mutex::new(0),
            network: Mutex::new(Vec::new()),
        });
        initialize(plugin.as_ref(), root.path(), host).await;
        let output = invoke(
            plugin.as_ref(),
            root.path(),
            "realtime",
            json!({"text":"hi","duration_seconds":1}),
        )
        .await
        .unwrap();
        mock.await.unwrap();
        let serialized = serde_json::to_string(&output).unwrap();
        assert!(serialized.contains("\"completed\":false"));
        assert!(serialized.contains("\"input_submitted\":true"));
        assert_eq!(output.attachments.len(), 1);
        assert!(serialized.contains(if failed {
            "without successful completion"
        } else {
            "Turn deadline reached"
        }));
    }
}

#[tokio::test]
async fn managed_session_events_decode_sse_and_keep_the_session_id() {
    let plugin = plugins().remove(0);
    let root = tempfile::tempdir().unwrap();
    let (base, received) = server_bytes(
        "text/event-stream",
        b"event: output\ndata: {\"type\":\"message\",\"text\":\"hello\"}\n\ndata: [DONE]\n\n"
            .to_vec(),
    )
    .await;
    let host = Arc::new(Host {
        connection: Mutex::new(connection(format!("{base}/v1"), None)),
        resolutions: Mutex::new(0),
        network: Mutex::new(Vec::new()),
    });
    initialize(plugin.as_ref(), root.path(), host).await;
    let output = invoke(
        plugin.as_ref(),
        root.path(),
        "agents",
        json!({"action":"events","resource":"sessions","id":"session_test"}),
    )
    .await
    .unwrap();
    assert!(
        received
            .await
            .unwrap()
            .0
            .starts_with("GET /v1/agents/sessions/session_test/events ")
    );
    let output = serde_json::to_value(output).unwrap();
    let output = output.to_string();
    assert!(output.contains("hello"));
    assert!(output.contains("session_test"));
    assert!(output.contains("stream_ended"));
}
