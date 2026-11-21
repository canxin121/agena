use std::collections::VecDeque;

use base64::Engine as _;
use serde::Serialize;
use serde_json::{Value, json};

use super::{CdpClient, CdpEvent};

pub(crate) const MAX_BODY_BYTES: usize = 1024 * 1024;
const REQUEST_LIMIT: usize = 256;
const LOG_LIMIT: usize = 256;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct RequestKey {
    session: String,
    id: String,
}

pub(super) struct Request {
    key: RequestKey,
    loader_id: String,
    started: Option<f64>,
    pub(super) entry: NetworkEntry,
}

#[derive(Clone, Serialize)]
pub(super) struct NetworkEntry {
    seq: u64,
    request_id: u64,
    url: String,
    method: String,
    resource_type: String,
    frame_id: String,
    status: Option<u16>,
    mime_type: Option<String>,
    duration_ms: Option<f64>,
    state: &'static str,
    failure: Option<String>,
    redirects: u32,
}

#[derive(Clone)]
pub(crate) struct ResponseBody {
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

pub(super) struct BodySource {
    key: RequestKey,
    pub(super) entry: NetworkEntry,
    captured: Option<Result<ResponseBody, &'static str>>,
}

impl BodySource {
    pub(super) fn url(&self) -> &str {
        &self.entry.url
    }

    pub(super) fn is_html(&self) -> bool {
        matches!(
            self.entry.mime_type.as_deref(),
            Some("text/html" | "application/xhtml+xml")
        )
    }

    pub(super) fn entry_state(&self) -> &'static str {
        self.entry.state
    }

    pub(super) async fn read(&self, client: &CdpClient) -> Result<ResponseBody, &'static str> {
        match self.entry.state {
            "failed" => return Err("request_failed"),
            "finished" => {}
            _ => return Err("pending"),
        }
        if let Some(captured) = &self.captured {
            return captured.clone();
        }
        read_body(client, &self.key).await
    }
}

#[derive(Serialize)]
struct LogEntry {
    seq: u64,
    timestamp_ms: i64,
    kind: &'static str,
    level: String,
    message: Option<String>,
    url: Option<String>,
    line: Option<u64>,
    column: Option<u64>,
}

pub(super) struct Diagnostics {
    main_frame: String,
    seq: u64,
    requests: VecDeque<Request>,
    logs: VecDeque<LogEntry>,
    documents: VecDeque<(String, BodySource)>,
}

impl Diagnostics {
    pub(super) fn new(main_frame: &str) -> Self {
        Self {
            main_frame: main_frame.to_owned(),
            seq: 0,
            requests: VecDeque::new(),
            logs: VecDeque::new(),
            documents: VecDeque::new(),
        }
    }

    pub(super) fn record(
        &mut self,
        event: &CdpEvent,
        main: bool,
        capture_console: bool,
    ) -> Option<RequestKey> {
        match event.method.as_str() {
            "Page.frameNavigated" if main && event.params.pointer("/frame/parentId").is_none() => {
                let frame = &event.params["frame"];
                self.main_frame = string(&frame["id"], 128);
                None
            }
            "Network.requestWillBeSent"
            | "Network.responseReceived"
            | "Network.loadingFinished"
            | "Network.loadingFailed" => self.network_event(event, main),
            "Runtime.consoleAPICalled" | "Runtime.exceptionThrown" | "Log.entryAdded" => {
                self.log(event, capture_console);
                None
            }
            _ => None,
        }
    }

    fn network_event(&mut self, event: &CdpEvent, main: bool) -> Option<RequestKey> {
        let p = &event.params;
        let key = RequestKey {
            session: event.session_id.as_deref()?.to_owned(),
            id: p["requestId"].as_str()?.to_owned(),
        };
        let index = self.requests.iter().position(|entry| entry.key == key);
        let timestamp = p["timestamp"].as_f64();
        self.seq += 1;
        let mut request = match index {
            Some(index) => self.requests.remove(index)?,
            None if event.method == "Network.requestWillBeSent" => Request {
                key: key.clone(),
                loader_id: String::new(),
                started: timestamp,
                entry: NetworkEntry {
                    seq: self.seq,
                    request_id: self.seq,
                    url: String::new(),
                    method: String::new(),
                    resource_type: String::new(),
                    frame_id: String::new(),
                    status: None,
                    mime_type: None,
                    duration_ms: None,
                    state: "pending",
                    failure: None,
                    redirects: 0,
                },
            },
            _ => return None,
        };
        request.entry.seq = self.seq;
        match event.method.as_str() {
            "Network.requestWillBeSent" => {
                if p.get("redirectResponse").is_some() {
                    request.entry.redirects += 1;
                }
                request.loader_id = string(&p["loaderId"], 128);
                request.entry.url = url_text(&p["request"]["url"]);
                request.entry.method = string(&p["request"]["method"], 24);
                request.entry.frame_id = string(&p["frameId"], 128);
                request.entry.resource_type = string(&p["type"], 40);
                request.entry.status = None;
                request.entry.mime_type = None;
                request.entry.state = "pending";
            }
            "Network.responseReceived" => {
                request.entry.resource_type = string(&p["type"], 40);
                request.entry.status = p["response"]["status"].as_f64().map(|status| status as u16);
                request.entry.mime_type = p["response"]["mimeType"]
                    .as_str()
                    .map(|mime| prefix(mime, 128));
                request.entry.state = "response";
            }
            "Network.loadingFinished" => request.entry.state = "finished",
            "Network.loadingFailed" => {
                request.entry.state = "failed";
                request.entry.failure = Some(string(&p["errorText"], 256));
            }
            _ => {}
        }
        request.entry.duration_ms = timestamp
            .zip(request.started)
            .map(|(now, start)| ((now - start).max(0.0) * 1_000.0).round());
        let mut capture = None;
        if main
            && request.entry.resource_type == "Document"
            && request.entry.frame_id == self.main_frame
        {
            let source = BodySource {
                key: key.clone(),
                entry: request.entry.clone(),
                captured: None,
            };
            if source.entry.state == "finished" && source.is_html() {
                capture = Some(key);
            }
            if let Some((_, entry)) = self
                .documents
                .iter_mut()
                .find(|(loader, _)| loader == &request.loader_id)
            {
                *entry = source;
            } else {
                if self.documents.len() == 2 {
                    self.documents.pop_front();
                }
                self.documents
                    .push_back((request.loader_id.clone(), source));
            }
        }
        if self.requests.len() == REQUEST_LIMIT {
            self.requests.pop_front();
        }
        self.requests.push_back(request);
        capture
    }

    pub(super) fn save_body(
        &mut self,
        key: &RequestKey,
        result: Result<ResponseBody, &'static str>,
    ) {
        if let Some((_, source)) = self
            .documents
            .iter_mut()
            .find(|(_, source)| &source.key == key)
        {
            source.captured = Some(result);
        }
    }

    pub(super) fn document(&self, loader_id: &str) -> Option<BodySource> {
        self.documents
            .iter()
            .find(|(loader, _)| loader == loader_id)
            .map(|(_, source)| BodySource {
                key: source.key.clone(),
                entry: source.entry.clone(),
                captured: source.captured.clone(),
            })
    }

    pub(super) fn request(&self, request_id: u64) -> Option<BodySource> {
        let request = self
            .requests
            .iter()
            .find(|request| request.entry.request_id == request_id)?;
        let captured = self
            .documents
            .iter()
            .find(|(_, source)| source.key == request.key)
            .and_then(|(_, source)| source.captured.clone());
        Some(BodySource {
            key: request.key.clone(),
            entry: request.entry.clone(),
            captured,
        })
    }

    pub(super) fn requests(&self, since: u64, limit: usize, filter: Option<&str>) -> Value {
        let mut matching = self
            .requests
            .iter()
            .map(|r| &r.entry)
            .filter(|entry| entry.seq > since && filter.is_none_or(|s| entry.url.contains(s)));
        let entries: Vec<_> = matching.by_ref().take(limit).collect();
        let has_more = matching.next().is_some();
        json!({
            "requests": entries,
            "next_seq": if has_more { entries.last().map_or(since, |entry| entry.seq) } else { self.seq.max(since) },
            "has_more": has_more,
            "oldest_retained_seq": self.requests.front().map(|entry| entry.entry.seq),
        })
    }

    pub(super) fn logs(&self, since: u64, limit: usize) -> Value {
        let mut matching = self.logs.iter().filter(|entry| entry.seq > since);
        let entries: Vec<_> = matching.by_ref().take(limit).collect();
        let has_more = matching.next().is_some();
        json!({
            "logs": entries,
            "next_seq": if has_more { entries.last().map_or(since, |entry| entry.seq) } else { self.seq.max(since) },
            "has_more": has_more,
            "oldest_retained_seq": self.logs.front().map(|entry| entry.seq),
        })
    }

    fn log(&mut self, event: &CdpEvent, capture_text: bool) {
        self.seq += 1;
        let p = &event.params;
        let (kind, level, message, location) = match event.method.as_str() {
            "Runtime.consoleAPICalled" => {
                let message = capture_text.then(|| console_text(&p["args"]));
                (
                    "console",
                    string(&p["type"], 32),
                    message,
                    &p["stackTrace"]["callFrames"][0],
                )
            }
            "Runtime.exceptionThrown" => {
                let details = &p["exceptionDetails"];
                let summary = if capture_text {
                    details
                        .pointer("/exception/description")
                        .and_then(Value::as_str)
                } else {
                    details
                        .pointer("/exception/className")
                        .and_then(Value::as_str)
                };
                let message = summary
                    .or_else(|| details["text"].as_str())
                    .map(|text| prefix(text, 1024));
                ("exception", "error".into(), message, details)
            }
            _ => {
                let entry = &p["entry"];
                let message = capture_text.then(|| string(&entry["text"], 1024));
                ("browser", string(&entry["level"], 32), message, entry)
            }
        };
        if self.logs.len() == LOG_LIMIT {
            self.logs.pop_front();
        }
        self.logs.push_back(LogEntry {
            seq: self.seq,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
            kind,
            level,
            message,
            url: location["url"].as_str().map(|_| url_text(&location["url"])),
            line: location["lineNumber"].as_u64().map(|line| line + 1),
            column: location["columnNumber"].as_u64().map(|column| column + 1),
        });
    }
}

pub(super) async fn read_body(
    client: &CdpClient,
    key: &RequestKey,
) -> Result<ResponseBody, &'static str> {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client.send_command(
            "Network.getResponseBody",
            Some(json!({"requestId": key.id})),
            Some(&key.session),
        ),
    )
    .await
    .map_err(|_| "unavailable")?
    .map_err(|error| {
        if error.contains("evicted") {
            "evicted"
        } else {
            "unavailable"
        }
    })?;
    let body = response["body"].as_str().ok_or("unavailable")?;
    let decoded;
    let text = if response["base64Encoded"] == true {
        decoded = base64::engine::general_purpose::STANDARD
            .decode(body)
            .map_err(|_| "unavailable")?;
        std::str::from_utf8(&decoded).map_err(|_| "non_text")?
    } else {
        body
    };
    let capped = super::utf8_prefix(text, MAX_BODY_BYTES);
    Ok(ResponseBody {
        text: capped.to_owned(),
        truncated: capped.len() < text.len(),
    })
}

fn url_text(value: &Value) -> String {
    let raw = value.as_str().unwrap_or_default();
    if raw.starts_with("data:") {
        return "data:".into();
    }
    match url::Url::parse(raw) {
        Ok(mut url) if !url.username().is_empty() || url.password().is_some() => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            prefix(url.as_str(), 2048)
        }
        _ => prefix(raw, 2048),
    }
}

fn prefix(text: &str, max_bytes: usize) -> String {
    super::utf8_prefix(text, max_bytes).to_owned()
}

fn string(value: &Value, limit: usize) -> String {
    prefix(value.as_str().unwrap_or_default(), limit)
}

pub(crate) fn console_text(args: &Value) -> String {
    args.as_array()
        .into_iter()
        .flatten()
        .take(16)
        .map(|arg| {
            let value = arg["value"]
                .as_str()
                .or_else(|| arg["description"].as_str());
            value.map_or_else(
                || prefix(&arg["value"].to_string(), 256),
                |s| prefix(s, 256),
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}
