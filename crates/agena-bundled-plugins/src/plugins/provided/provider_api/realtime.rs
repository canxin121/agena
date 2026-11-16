//! Bounded voice turns over the vendor WebSocket APIs. Inputs are explicit
//! files/text; this does not open microphones or execute returned tool calls.
use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio::time::{Instant, timeout_at};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
pub struct RealtimeInput {
    /// Defaults to current session model; explicitly pass a compatible OpenAI
    /// Realtime or Google Gemini Live model when the current model is unsuitable.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    /// Explicit raw signed 16-bit little-endian mono PCM file, not WAV/MP3.
    #[serde(default)]
    pub audio_path: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    /// Input sample rate. OpenAI requires 24000; Gemini permits 16000 or 24000.
    #[serde(default = "sample_rate")]
    pub input_sample_rate: u32,
    /// Official session/setup fields. model and output audio format are managed.
    #[serde(default)]
    pub session: BTreeMap<String, Value>,
    #[serde(default)]
    pub output_path: Option<String>,
    /// Complete turn deadline, 1..50 seconds. Partial audio is retained on timeout.
    #[serde(default = "default_wait")]
    pub duration_seconds: u32,
}
fn sample_rate() -> u32 {
    24000
}

fn websocket_url(service: &Service<'_>, model: &str) -> Result<url::Url> {
    if let Some(override_url) = &service.connection.realtime_ws_url {
        let mut url = url::Url::parse(override_url)
            .map_err(|_| invalid("invalid configured realtime URL"))?;
        if !matches!(url.scheme(), "ws" | "wss")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid(
                "configured realtime URL must use WS/WSS and contain no userinfo, query or fragment",
            ));
        }
        if service.provider == "chatgpt" {
            url.query_pairs_mut().append_pair("model", model);
        }
        return Ok(url);
    }

    let mut url = url::Url::parse(&service.connection.base_url)
        .map_err(|_| invalid("invalid provider URL"))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid(
            "provider URL must not contain credentials, a query or a fragment",
        ));
    }
    if service.provider == "gemini" {
        let path = url.path().trim_end_matches('/');
        let prefix = path
            .strip_suffix("/v1beta")
            .ok_or_else(|| invalid("Gemini Live requires a v1beta provider connection"))?;
        url.set_path(&format!(
            "{prefix}/ws/google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent"
        ));
    } else {
        url = url::Url::parse(&official_service::endpoint(
            &service.connection.base_url,
            "realtime",
        )?)
        .map_err(|_| invalid("invalid provider URL"))?;
        url.query_pairs_mut().append_pair("model", model);
    }
    let scheme = match url.scheme() {
        "https" => "wss",
        "http" => "ws",
        _ => return Err(invalid("provider must use HTTP or HTTPS")),
    };
    url.set_scheme(scheme)
        .map_err(|_| invalid("invalid WebSocket scheme"))?;
    Ok(url)
}

pub async fn invoke(service: Service<'_>, input: RealtimeInput) -> Result<ToolInvokeOutput> {
    if !(1..=50).contains(&input.duration_seconds) {
        return Err(invalid("duration_seconds must be 1..50"));
    }
    if input.text.as_deref().is_none_or(|s| s.trim().is_empty()) && input.audio_path.is_none() {
        return Err(invalid("provide text or an explicit raw PCM audio_path"));
    }
    if input.session.contains_key("model") {
        return Err(invalid("use the top-level model override"));
    }
    if input.text.as_ref().is_some_and(|t| t.len() > 64000) {
        return Err(invalid("text exceeds 64000 bytes"));
    }
    if input.input_sample_rate != 24000
        && !(service.provider == "gemini" && input.input_sample_rate == 16000)
    {
        return Err(invalid("unsupported PCM input sample rate"));
    }
    let model = official_service::configured_model(
        input.model.clone(),
        service.connection.model.as_deref(),
        "realtime",
    )?;
    let audio = if let Some(path) = &input.audio_path {
        let bytes = authorized_input(
            &service,
            path,
            input.expected_sha256.as_deref(),
            10 * 1024 * 1024,
        )
        .await?;
        if bytes.len() % 2 != 0
            || bytes.starts_with(b"RIFF")
            || bytes.starts_with(b"ID3")
            || bytes.starts_with(b"OggS")
        {
            return Err(invalid(
                "audio_path must contain raw 16-bit mono PCM; convert containers explicitly",
            ));
        }
        Some(bytes)
    } else {
        None
    };
    let url = websocket_url(&service, &model)?;
    let mut permission_url = url.clone();
    permission_url
        .set_scheme(if url.scheme() == "wss" {
            "https"
        } else {
            "http"
        })
        .map_err(|_| invalid("invalid permission URL"))?;
    let addresses = official_service::authorized_provider_addresses(
        service.host,
        permission_url.as_str(),
        std::time::Duration::from_secs(service.connection.connect_timeout_secs.max(1).min(30)),
    )
    .await?;
    let headers = official_service::provider_headers(&service.connection)?;
    let worker_input = RealtimeWorkerInput {
        provider: service.provider.to_owned(),
        url: url.to_string(),
        headers: headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.as_bytes().to_vec()))
            .collect(),
        addresses,
        model: model.clone(),
        text: input.text.clone(),
        audio_base64: audio.map(|bytes| STANDARD.encode(bytes)),
        input_sample_rate: input.input_sample_rate,
        session: input.session.clone(),
        duration_seconds: (input.duration_seconds as u64)
            .min(service.connection.timeout_secs.max(1)),
        connect_timeout_seconds: service.connection.connect_timeout_secs.clamp(1, 30),
    };
    let value = crate::worker::execute_bundled_worker(
        service.host.as_ref(),
        "provider.realtime",
        serde_json::to_value(worker_input).map_err(|error| PluginError::internal_error(&error))?,
        agena_plugin_sdk::host_api::HostProcessResources {
            network: vec![permission_url.origin().ascii_serialization()],
            ..Default::default()
        },
    )
    .await?;
    let result: RealtimeWorkerOutput = serde_json::from_value(value)
        .map_err(|_| PluginError::internal("invalid voice worker response"))?;
    let pcm = result.validated_pcm()?;
    let RealtimeWorkerOutput {
        complete,
        submitted,
        transcripts,
        events,
        usage_response,
        warnings,
        ..
    } = result;
    let mut attachments = Vec::new();
    if !pcm.is_empty() {
        attachments.push(
            save_binary(
                &service,
                input.output_path.as_deref(),
                "audio/wav",
                wav(pcm, 24000),
            )
            .await?,
        );
    }
    Ok(ToolInvokeOutput::from_parts(
        "Realtime voice",
        if complete {
            "Voice turn completed"
        } else {
            "Voice turn incomplete"
        },
        transcripts,
        Some(
            json!({"provider":service.provider,"provider_id":service.connection.provider_id,"model":model,"completed":complete,
            "execution_location":"vendor_cloud","local_workspace_automatically_available":false,
            "input_submitted":submitted,"warnings":warnings,"events":events,"local_actions_executed":false,"safe_to_reexecute_automatically":false}),
        ),
        official_service::resource_usage(
            &service.connection,
            service.provider,
            "realtime",
            &model,
            &usage_response,
            true,
        )?,
        attachments,
    ))
}

/// Only explicit target addresses, protocol fields and target request headers
/// cross this boundary. Host services and provider configuration stay outside.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RealtimeWorkerInput {
    provider: String,
    url: String,
    headers: Vec<(String, Vec<u8>)>,
    addresses: Vec<std::net::SocketAddr>,
    model: String,
    text: Option<String>,
    audio_base64: Option<String>,
    input_sample_rate: u32,
    session: BTreeMap<String, Value>,
    duration_seconds: u64,
    connect_timeout_seconds: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RealtimeWorkerOutput {
    complete: bool,
    submitted: bool,
    pcm_base64: String,
    transcripts: String,
    events: Vec<Value>,
    usage_response: Value,
    warnings: Vec<String>,
}

impl RealtimeWorkerOutput {
    fn validated_pcm(&self) -> Result<Vec<u8>> {
        if self.pcm_base64.len() > MAX_INLINE_BINARY.div_ceil(3) * 4
            || self.transcripts.len() > 128 * 1024
            || self.events.len() > 256
            || self
                .events
                .iter()
                .map(|event| event.to_string().len())
                .sum::<usize>()
                > 128 * 1024
            || self.usage_response.to_string().len() > 8 * 1024 * 1024
            || self.warnings.len() > 16
            || self.warnings.iter().any(|warning| warning.len() > 4096)
        {
            return Err(PluginError::internal(
                "voice worker response exceeds limits",
            ));
        }
        let pcm = STANDARD
            .decode(&self.pcm_base64)
            .map_err(|_| PluginError::internal("invalid voice worker audio"))?;
        if pcm.len() > MAX_INLINE_BINARY || pcm.len() % 2 != 0 {
            return Err(PluginError::internal("invalid voice worker PCM length"));
        }
        Ok(pcm)
    }
}

pub(crate) async fn execute_worker(mut value: Value) -> Result<Value> {
    if let Some(object) = value.as_object_mut() {
        object.remove("_agena_permission_snapshot");
    }
    let input: RealtimeWorkerInput =
        serde_json::from_value(value).map_err(|_| invalid("invalid voice worker request"))?;
    if !matches!(input.provider.as_str(), "chatgpt" | "gemini")
        || !(1..=50).contains(&input.duration_seconds)
        || !(1..=30).contains(&input.connect_timeout_seconds)
        || input.text.as_ref().is_some_and(|text| text.len() > 64000)
        || input.session.contains_key("model")
        || (input.input_sample_rate != 24000
            && !(input.provider == "gemini" && input.input_sample_rate == 16000))
        || input.headers.len() > 128
        || input
            .headers
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum::<usize>()
            > 64 * 1024
    {
        return Err(invalid("invalid voice worker protocol limits"));
    }
    let url = url::Url::parse(&input.url).map_err(|_| invalid("invalid voice worker URL"))?;
    let port = url.port_or_known_default();
    if !matches!(url.scheme(), "ws" | "wss")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || input.addresses.is_empty()
        || input
            .addresses
            .iter()
            .any(|address| Some(address.port()) != port)
    {
        return Err(invalid(
            "voice worker requires approved addresses for its WS/WSS target",
        ));
    }
    let audio = input
        .audio_base64
        .as_deref()
        .map(|encoded| {
            if encoded.len() > (10_usize * 1024 * 1024).div_ceil(3) * 4 {
                return Err(invalid("voice worker audio exceeds limit"));
            }
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| invalid("invalid voice worker audio"))?;
            if bytes.len() > 10 * 1024 * 1024
                || bytes.len() % 2 != 0
                || bytes.starts_with(b"RIFF")
                || bytes.starts_with(b"ID3")
                || bytes.starts_with(b"OggS")
            {
                return Err(invalid("voice worker requires raw 16-bit mono PCM"));
            }
            Ok(bytes)
        })
        .transpose()?;
    if input
        .text
        .as_deref()
        .is_none_or(|text| text.trim().is_empty())
        && audio.is_none()
    {
        return Err(invalid("voice worker requires text or PCM input"));
    }
    let model = input.model.clone();
    let gemini = input.provider == "gemini";
    let mut session = Value::Object(input.session.clone().into_iter().collect());
    for path in [
        "/generationConfig",
        "/audio",
        "/audio/input",
        "/audio/output",
    ] {
        if session
            .pointer(path)
            .is_some_and(|v| !v.is_object() && !v.is_null())
        {
            return Err(invalid(
                "session audio/generationConfig fields must be objects",
            ));
        }
    }
    if gemini {
        session["model"] = json!(format!(
            "models/{}",
            model.strip_prefix("models/").unwrap_or(&model)
        ));
        session["generationConfig"]["responseModalities"] = json!(["AUDIO"]);
        session["outputAudioTranscription"] = json!({});
    } else {
        session["type"] = json!("realtime");
        session["model"] = json!(model);
        session["output_modalities"] = json!(["audio"]);
        session["audio"]["input"]["format"] = json!({"type":"audio/pcm","rate":24000});
        session["audio"]["input"]["turn_detection"] = Value::Null;
        session["audio"]["output"]["format"] = json!({"type":"audio/pcm","rate":24000});
    }
    let setup = if gemini {
        json!({"setup":session})
    } else {
        json!({"type":"session.update","session":session})
    };
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| invalid("invalid voice worker request"))?;
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in &input.headers {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| invalid("invalid voice worker header"))?;
        let value = reqwest::header::HeaderValue::from_bytes(value)
            .map_err(|_| invalid("invalid voice worker header"))?;
        headers.append(name, value);
    }
    request.headers_mut().extend(headers);
    request.headers_mut().insert(
        "x-agena-provider-api",
        input
            .provider
            .parse()
            .map_err(|_| invalid("invalid provider protocol"))?,
    );
    let deadline = Instant::now() + std::time::Duration::from_secs(input.duration_seconds);
    let connect_deadline = deadline
        .min(Instant::now() + std::time::Duration::from_secs(input.connect_timeout_seconds));
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(8 * 1024 * 1024))
        .max_frame_size(Some(8 * 1024 * 1024));
    let (mut socket, _) = timeout_at(connect_deadline, async {
        let stream = tokio::net::TcpStream::connect(input.addresses.as_slice())
            .await
            .map_err(tokio_tungstenite::tungstenite::Error::Io)?;
        // The URL supplies Host and TLS SNI; all TCP destinations were approved
        // before this worker was started. No worker DNS lookup is performed.
        tokio_tungstenite::client_async_tls_with_config(request, stream, Some(config), None).await
    })
    .await
    .map_err(|_| invalid("voice connection deadline exceeded"))?
    .map_err(|_| PluginError::internal("voice WebSocket connection failed"))?;
    send(&mut socket, setup.to_string().into(), deadline).await?;
    let mut submitted = false;
    let mut complete = false;
    let mut pcm = Vec::new();
    let mut transcripts = String::new();
    let mut events = Vec::new();
    let mut event_bytes = 0usize;
    let mut usage_response = Value::Null;
    let mut warnings = Vec::new();
    loop {
        let frame = match tokio::time::timeout_at(deadline, socket.next()).await {
            Ok(Some(Ok(frame))) => frame,
            Ok(None) => break,
            Ok(Some(Err(_))) => {
                warnings.push("Voice connection closed before confirmed completion.");
                break;
            }
            Err(_) => {
                warnings.push("Turn deadline reached; returned audio/transcript may be partial. Do not automatically repeat the paid request.");
                break;
            }
        };
        let bytes = match frame {
            Message::Text(t) => t.as_bytes().to_vec(),
            Message::Binary(b) => b.to_vec(),
            Message::Close(_) => break,
            _ => continue,
        };
        let mut event: Value = serde_json::from_slice(&bytes)
            .map_err(|_| PluginError::internal("invalid voice event JSON"))?;
        if event.get("error").is_some() {
            return Err(PluginError::internal(format!(
                "provider voice error: {}",
                official_service::truncate_text(&event["error"].to_string(), 2048)
            )));
        }
        let kind = event["type"].as_str().unwrap_or("").to_owned();
        let ready = if gemini {
            event.get("setupComplete").is_some() || event.get("setup_complete").is_some()
        } else {
            kind == "session.updated"
        };
        if ready && !submitted {
            if let Some(audio) = &audio {
                for chunk in audio.chunks(32 * 1024) {
                    let data = STANDARD.encode(chunk);
                    let message = if gemini {
                        json!({"realtimeInput":{"audio":{"data":data,"mimeType":format!("audio/pcm;rate={}",input.input_sample_rate)}}})
                    } else {
                        json!({"type":"input_audio_buffer.append","audio":data})
                    };
                    send(&mut socket, message.to_string().into(), deadline).await?;
                }
                let message = if gemini {
                    json!({"realtimeInput":{"audioStreamEnd":true}})
                } else {
                    json!({"type":"input_audio_buffer.commit"})
                };
                send(&mut socket, message.to_string().into(), deadline).await?;
            }
            if let Some(text) = &input.text {
                let message = if gemini {
                    json!({"clientContent":{"turns":[{"role":"user","parts":[{"text":text}]}],"turnComplete":true}})
                } else {
                    json!({"type":"conversation.item.create","item":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}})
                };
                send(&mut socket, message.to_string().into(), deadline).await?;
            }
            if !gemini {
                send(
                    &mut socket,
                    json!({"type":"response.create"}).to_string().into(),
                    deadline,
                )
                .await?;
            }
            submitted = true;
        }
        if gemini {
            if let Some(parts) = event
                .pointer("/serverContent/modelTurn/parts")
                .and_then(Value::as_array)
            {
                for part in parts {
                    if part
                        .pointer("/inlineData/mimeType")
                        .and_then(Value::as_str)
                        .is_some_and(|m| m.starts_with("audio/pcm"))
                        && let Some(data) = part.pointer("/inlineData/data").and_then(Value::as_str)
                    {
                        append_pcm(&mut pcm, data)?;
                    }
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        transcripts.push_str(text);
                    }
                }
            }
            if let Some(text) = event
                .pointer("/serverContent/outputTranscription/text")
                .and_then(Value::as_str)
            {
                transcripts.push_str(text);
            }
            complete = event
                .pointer("/serverContent/turnComplete")
                .and_then(Value::as_bool)
                == Some(true);
        } else {
            if matches!(
                kind.as_str(),
                "response.audio.delta" | "response.output_audio.delta"
            ) && let Some(delta) = event.get("delta").and_then(Value::as_str)
            {
                append_pcm(&mut pcm, delta)?;
                event["delta"] = json!("[saved as attachment]");
            }
            if matches!(
                kind.as_str(),
                "response.audio_transcript.delta"
                    | "response.output_audio_transcript.delta"
                    | "response.text.delta"
                    | "response.output_text.delta"
            ) && let Some(delta) = event.get("delta").and_then(Value::as_str)
            {
                transcripts.push_str(delta);
            }
            if matches!(kind.as_str(), "response.done" | "response.completed") {
                let status = event
                    .pointer("/response/status")
                    .and_then(Value::as_str)
                    .unwrap_or("completed");
                complete = status == "completed";
                if !complete {
                    warnings
                        .push("Provider ended the voice response without successful completion.");
                }
                usage_response = event
                    .get("response")
                    .cloned()
                    .unwrap_or_else(|| event.clone());
            }
        }
        if gemini && event.get("usageMetadata").is_some() {
            usage_response = event.clone();
        }
        let ended = matches!(kind.as_str(), "response.done" | "response.completed");
        let mut redacted = Vec::new();
        collect_binary(&mut event, &mut redacted, 0);
        event_bytes = event_bytes.saturating_add(event.to_string().len());
        if events.len() < 256 && event_bytes <= 128 * 1024 {
            events.push(event);
        }
        if transcripts.len() > 128 * 1024 {
            return Err(invalid("voice transcript exceeds limit"));
        }
        if complete || ended {
            break;
        }
    }
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), socket.close(None)).await;
    serde_json::to_value(RealtimeWorkerOutput {
        complete,
        submitted,
        pcm_base64: STANDARD.encode(pcm),
        transcripts,
        events,
        usage_response,
        warnings: warnings.into_iter().map(str::to_owned).collect(),
    })
    .map_err(|error| PluginError::internal_error(&error))
}

fn append_pcm(output: &mut Vec<u8>, data: &str) -> Result<()> {
    if data.len() > 12 * 1024 * 1024 {
        return Err(invalid("voice audio frame exceeds limit"));
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| invalid("invalid voice audio base64"))?;
    if output.len().saturating_add(bytes.len()) > MAX_INLINE_BINARY {
        return Err(invalid("voice audio output exceeds limit"));
    }
    output.extend(bytes);
    Ok(())
}
pub(super) fn wav(pcm: Vec<u8>, rate: u32) -> Vec<u8> {
    let mut output = Vec::with_capacity(44 + pcm.len());
    output.extend(b"RIFF");
    output.extend((36 + pcm.len() as u32).to_le_bytes());
    output.extend(b"WAVEfmt ");
    output.extend(16u32.to_le_bytes());
    output.extend(1u16.to_le_bytes());
    output.extend(1u16.to_le_bytes());
    output.extend(rate.to_le_bytes());
    output.extend((rate * 2).to_le_bytes());
    output.extend(2u16.to_le_bytes());
    output.extend(16u16.to_le_bytes());
    output.extend(b"data");
    output.extend((pcm.len() as u32).to_le_bytes());
    output.extend(pcm);
    output
}

async fn send<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    text: tokio_tungstenite::tungstenite::Utf8Bytes,
    deadline: Instant,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    timeout_at(deadline,socket.send(Message::Text(text))).await
        .map_err(|_|ambiguous("Voice send deadline exceeded; acceptance may be unknown. Do not automatically repeat."))?
        .map_err(|_|ambiguous("Voice send failed; acceptance may be unknown. Do not automatically repeat."))
}

#[cfg(test)]
mod worker_tests {
    use super::*;

    #[tokio::test]
    async fn realtime_worker_rejects_unpinned_target_before_connecting() {
        let input = json!({
            "provider":"chatgpt", "url":"wss://example.invalid/realtime?model=fixture",
            "headers":[["authorization", b"Bearer private-fixture".to_vec()]],
            "addresses":[], "model":"fixture", "text":"hello", "audio_base64":null,
            "input_sample_rate":24000, "session":{}, "duration_seconds":1,
            "connect_timeout_seconds":1,
            "_agena_permission_snapshot":null
        });
        let error = execute_worker(input).await.unwrap_err();
        assert!(error.to_string().contains("approved addresses"));
        assert!(!error.to_string().contains("private-fixture"));
    }

    #[test]
    fn realtime_worker_audio_is_validated_before_host_publication() {
        let mut output = RealtimeWorkerOutput {
            complete: true,
            submitted: true,
            pcm_base64: STANDARD.encode([0, 0]),
            transcripts: String::new(),
            events: Vec::new(),
            usage_response: Value::Null,
            warnings: Vec::new(),
        };
        assert_eq!(output.validated_pcm().unwrap(), [0, 0]);
        output.pcm_base64 = STANDARD.encode([0]);
        assert!(output.validated_pcm().is_err());
        output.pcm_base64 = "invalid base64".into();
        assert!(output.validated_pcm().is_err());
    }
}
