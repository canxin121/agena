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
    official_service::validate_provider_endpoint(service.host, permission_url.as_str()).await?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| invalid("invalid WebSocket request"))?;
    request
        .headers_mut()
        .extend(official_service::provider_headers(&service.connection)?);
    request.headers_mut().insert(
        "x-agena-provider-api",
        service
            .provider
            .parse()
            .map_err(|_| invalid("invalid provider protocol"))?,
    );
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(
            (input.duration_seconds as u64).min(service.connection.timeout_secs.max(1)),
        );
    let connect_deadline = deadline.min(
        tokio::time::Instant::now()
            + std::time::Duration::from_secs(service.connection.connect_timeout_secs.max(1)),
    );
    let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(8 * 1024 * 1024))
        .max_frame_size(Some(8 * 1024 * 1024));
    let (mut socket, _) = tokio::time::timeout_at(
        connect_deadline,
        tokio_tungstenite::connect_async_with_config(request, Some(config), false),
    )
    .await
    .map_err(|_| invalid("voice connection deadline exceeded"))?
    .map_err(|_| PluginError::internal("voice WebSocket connection failed"))?;
    let gemini = service.provider == "gemini";
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
                    {
                        if let Some(data) = part.pointer("/inlineData/data").and_then(Value::as_str)
                        {
                            append_pcm(&mut pcm, data)?;
                        }
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
            ) {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    append_pcm(&mut pcm, delta)?;
                    event["delta"] = json!("[saved as attachment]");
                }
            }
            if matches!(
                kind.as_str(),
                "response.audio_transcript.delta"
                    | "response.output_audio_transcript.delta"
                    | "response.text.delta"
                    | "response.output_text.delta"
            ) {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    transcripts.push_str(delta);
                }
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
