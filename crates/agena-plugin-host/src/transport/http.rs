//! HTTP transport — POSTs JSON-RPC envelopes to a remote plugin server.

use async_trait::async_trait;
use reqwest::Client;
use url::Url;

use crate::config::HttpAuth;
use crate::error::TransportError;
use crate::sdk::PluginError;
use crate::sdk::rpc::{JsonRpcVersion, Request, RequestId, Response, ResponsePayload};
use crate::transport::PluginTransport;

pub struct HttpTransport {
    client: Client,
    url: Url,
    auth_header: Option<String>,
    next_id: tokio::sync::Mutex<i64>,
}

impl HttpTransport {
    pub fn new(
        url: Url,
        auth: HttpAuth,
        env_lookup: &(dyn Fn(&str) -> Option<String> + Send + Sync),
    ) -> Self {
        let auth_header = match auth {
            HttpAuth::None => None,
            HttpAuth::Bearer { token, token_env } => {
                let resolved = token.or_else(|| token_env.as_deref().and_then(env_lookup));
                resolved.map(|t| format!("Bearer {t}"))
            }
            HttpAuth::Basic {
                username,
                password,
                password_env,
            } => {
                let pwd = password.or_else(|| password_env.as_deref().and_then(env_lookup));
                let pwd = pwd.unwrap_or_default();
                use std::fmt::Write;
                let mut creds = String::new();
                let _ = write!(creds, "{username}:{pwd}");
                let encoded = base64_encode(creds.as_bytes());
                Some(format!("Basic {encoded}"))
            }
        };
        Self {
            client: Client::new(),
            url,
            auth_header,
            next_id: tokio::sync::Mutex::new(1),
        }
    }

    async fn send(&self, req: &Request) -> Result<Response, TransportError> {
        let mut builder = self.client.post(self.url.clone()).json(req);
        if let Some(h) = &self.auth_header {
            builder = builder.header("authorization", h);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| TransportError::Io(e.to_string()))?;
        let body: Response = resp
            .json()
            .await
            .map_err(|e| TransportError::Rpc(e.to_string()))?;
        Ok(body)
    }
}

#[async_trait]
impl PluginTransport for HttpTransport {
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, TransportError> {
        let id = {
            let mut g = self.next_id.lock().await;
            let v = *g;
            *g += 1;
            v
        };
        let req = Request {
            jsonrpc: JsonRpcVersion,
            id: RequestId::Num(id),
            method: method.to_string(),
            params: Some(params),
        };
        let resp = self.send(&req).await?;
        match resp.payload {
            ResponsePayload::Ok { result } => Ok(result),
            ResponsePayload::Err { error } => {
                let pe: Option<PluginError> = error
                    .data
                    .as_ref()
                    .and_then(|d| serde_json::from_value(d.clone()).ok());
                let pe = pe.unwrap_or_else(|| PluginError {
                    code: crate::sdk::PluginErrorCode::Generic,
                    message: error.message,
                    hook: None,
                    plugin: None,
                    data: error.data,
                });
                Err(TransportError::Plugin(pe))
            }
        }
    }
}

// minimal base64 (RFC 4648) to avoid pulling another dep
fn base64_encode(data: &[u8]) -> String {
    const ALPH: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        let n = ((data[i] as u32) << 16) | ((data[i + 1] as u32) << 8) | (data[i + 2] as u32);
        out.push(ALPH[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPH[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPH[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPH[(n & 0x3F) as usize] as char);
        i += 3;
    }
    if i < data.len() {
        let rem = data.len() - i;
        let mut n: u32 = (data[i] as u32) << 16;
        if rem == 2 {
            n |= (data[i + 1] as u32) << 8;
        }
        out.push(ALPH[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPH[((n >> 12) & 0x3F) as usize] as char);
        if rem == 2 {
            out.push(ALPH[((n >> 6) & 0x3F) as usize] as char);
            out.push('=');
        } else {
            out.push('=');
            out.push('=');
        }
    }
    out
}
