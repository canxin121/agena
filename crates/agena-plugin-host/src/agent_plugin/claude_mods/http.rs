use std::{collections::BTreeMap, net::SocketAddr, time::Duration};

use agena_plugin_sdk::host_api::{HostClient, HostProcessResources, PermissionQuery};
use agena_process::sandbox_http::{self, HttpOptions};
use reqwest::{
    Method, Request, Response, StatusCode,
    header::{self, HeaderMap, HeaderName, HeaderValue},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

const MAX_BODY: usize = 256 * 1024;
const MAX_REDIRECTS: usize = 5;

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct HttpInit {
    method: Option<String>,
    headers: BTreeMap<String, String>,
    body: Option<String>,
    timeout_ms: Option<u64>,
    redirect: Redirect,
}

#[derive(Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Redirect {
    #[default]
    Follow,
    Manual,
}

pub(super) async fn fetch(
    host: &dyn HostClient,
    raw_url: String,
    init: HttpInit,
    cancel: CancellationToken,
) -> Result<Value, String> {
    let deadline = Duration::from_millis(init.timeout_ms.unwrap_or(30_000).min(600_000));
    let request = fetch_inner(host, raw_url, init, cancel.clone());
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("Claude mod HTTP request was cancelled".into()),
        result = tokio::time::timeout(deadline, request) => {
            result.map_err(|_| "Claude mod HTTP request timed out".to_owned())?
        }
    }
}

async fn fetch_inner(
    host: &dyn HostClient,
    raw_url: String,
    init: HttpInit,
    cancel: CancellationToken,
) -> Result<Value, String> {
    let mut url = Url::parse(&raw_url).map_err(|_| "invalid Claude mod HTTP URL")?;
    let mut method = init
        .method
        .map(|method| Method::from_bytes(method.to_ascii_uppercase().as_bytes()))
        .transpose()
        .map_err(|_| "invalid Claude mod HTTP method")?
        .unwrap_or(Method::GET);
    let mut headers = request_headers(init.headers)?;
    let mut body = init.body;
    if body.as_ref().is_some_and(|body| body.len() > MAX_BODY) {
        return Err("Claude mod HTTP request body exceeds the byte limit".into());
    }

    for redirect in 0..=MAX_REDIRECTS {
        validate_url(&url)?;
        let response = send(
            host,
            &url,
            method.clone(),
            headers.clone(),
            body.as_deref(),
            cancel.clone(),
        )
        .await?;
        let status = response.status();
        if init.redirect == Redirect::Manual
            || !matches!(
                status,
                StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                    | StatusCode::TEMPORARY_REDIRECT
                    | StatusCode::PERMANENT_REDIRECT
            )
            || !response.headers().contains_key(header::LOCATION)
        {
            return read_response(response).await;
        }
        if redirect == MAX_REDIRECTS {
            return Err("Claude mod HTTP request exceeded the redirect limit".into());
        }
        let location = response.headers()[header::LOCATION]
            .to_str()
            .map_err(|_| "invalid Claude mod HTTP redirect")?;
        let next = url
            .join(location)
            .map_err(|_| "invalid Claude mod HTTP redirect")?;
        if next.origin() != url.origin() {
            // Plugin headers can carry arbitrary credentials. Do not forward
            // them to an origin that the plugin did not explicitly address.
            headers = HeaderMap::new();
        }
        if (status == StatusCode::SEE_OTHER && method != Method::HEAD)
            || (matches!(status, StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND)
                && method == Method::POST)
        {
            method = Method::GET;
            body = None;
            headers.remove(header::CONTENT_TYPE);
            headers.remove(header::CONTENT_ENCODING);
        }
        url = next;
    }
    unreachable!("the redirects loop either returns a response or reaches its limit")
}

fn validate_url(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Claude mod HTTP requires an HTTP(S) URL without user information".into());
    }
    Ok(())
}

fn request_headers(definitions: BTreeMap<String, String>) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for (name, value) in definitions {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "invalid Claude mod HTTP header name")?;
        if name == header::HOST
            || name == header::CONTENT_LENGTH
            || name == header::TRANSFER_ENCODING
            || name.as_str().starts_with("proxy-")
        {
            return Err("Claude mod HTTP request contains a host-managed header".into());
        }
        let value =
            HeaderValue::from_str(&value).map_err(|_| "invalid Claude mod HTTP header value")?;
        headers.insert(name, value);
    }
    Ok(headers)
}

async fn send(
    host: &dyn HostClient,
    url: &Url,
    method: Method,
    headers: HeaderMap,
    body: Option<&str>,
    cancel: CancellationToken,
) -> Result<Response, String> {
    let target = url.to_string();
    let failure = |error: agena_plugin_sdk::PluginError| error.failure.user.fallback;
    host.require_pinned_http_permission(target.clone())
        .await
        .map_err(failure)?;
    let prepared = host
        .prepare_worker_sandbox(HostProcessResources {
            network: vec![url.origin().ascii_serialization()],
            ..Default::default()
        })
        .await
        .map_err(failure)?;
    let policy = serde_json::from_value(prepared.policy)
        .map_err(|_| "invalid Claude mod HTTP sandbox policy")?;
    let hostname = url.host_str().ok_or("Claude mod HTTP target has no host")?;
    let port = url
        .port_or_known_default()
        .ok_or("Claude mod HTTP target has no port")?;
    let addresses = tokio::net::lookup_host((hostname.trim_matches(['[', ']']), port))
        .await
        .map_err(|_| "Claude mod HTTP DNS lookup failed")?
        .collect::<std::collections::BTreeSet<SocketAddr>>();
    let mut permitted = Vec::new();
    for address in addresses.into_iter().take(128) {
        if matches!(
            host.check_resolved_network_permission(target.clone(), address.to_string())
                .await
                .map_err(failure)?,
            PermissionQuery::Allow {}
        ) {
            permitted.push(address);
        }
    }
    if permitted.is_empty() {
        return Err("no resolved Claude mod HTTP endpoint is permitted".into());
    }

    let mut request = Request::new(method, url.clone());
    *request.headers_mut() = headers;
    *request.body_mut() = body.map(|body| body.to_owned().into());
    sandbox_http::execute(
        request,
        HttpOptions {
            policy,
            pinned_addresses: permitted,
            max_request_bytes: MAX_BODY as u64,
            max_response_bytes: Some(MAX_BODY as u64),
            cancellation: Some(cancel),
            ..Default::default()
        },
    )
    .await
    .map_err(|error| format!("Claude mod HTTP request failed: {}", error.kind()))
}

async fn read_response(response: Response) -> Result<Value, String> {
    let status = response.status();
    let headers = response
        .headers()
        .keys()
        .map(|name| {
            let values = response.headers().get_all(name);
            let values = values
                .iter()
                .map(|value| String::from_utf8_lossy(value.as_bytes()))
                .collect::<Vec<_>>()
                .join(", ");
            (name.as_str().to_owned(), values)
        })
        .collect::<BTreeMap<_, _>>();
    let text = response
        .text()
        .await
        .map_err(|_| "Claude mod HTTP body could not be read")?;
    Ok(json!({"status":status.as_u16(),"ok":status.is_success(),"headers":headers,"text":text}))
}
