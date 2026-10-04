//! Bounded ordinary HTTP retrieval. Every redirect and robots request is
//! authorized before connecting, using the exact DNS addresses just checked.
use super::*;
use std::net::SocketAddr;

const ROBOTS_BYTES: usize = 512 * 1024;
const REDIRECTS: usize = 10;

#[derive(Debug)]
pub(super) struct Source {
    pub final_url: url::Url,
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub truncated: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub decoding_errors: bool,
}

async fn send<F, Fut>(
    url: &url::Url,
    options: &SpiderFetchOptions,
    resolve: &F,
) -> SdkResult<reqwest::Response>
where
    F: Fn(url::Url) -> Fut,
    Fut: Future<Output = SdkResult<Vec<SocketAddr>>>,
{
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(PluginError::invalid_params(
            "fetch URLs must use HTTP(S), without URL credentials",
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| PluginError::invalid_params("fetch URL has no host"))?;
    let addresses = resolve(url.clone()).await?;
    if addresses.is_empty() {
        return Err(PluginError::internal(
            "authorized fetch DNS resolution returned no addresses",
        ));
    }
    let client = reqwest::Client::builder()
        .user_agent(&options.user_agent)
        .timeout(options.timeout)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|error| PluginError::internal_error(&error))?;
    client
        .get(url.clone())
        .header(
            "Accept",
            "text/html, text/plain, application/xhtml+xml, application/json;q=0.9, */*;q=0.1",
        )
        .send()
        .await
        .map_err(|error| PluginError::internal_error(&error.without_url()))
}

fn redirect(response: &reqwest::Response) -> SdkResult<Option<url::Url>> {
    if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| PluginError::internal("HTTP redirect has no valid Location header"))?;
    let mut next = response
        .url()
        .join(location)
        .map_err(|error| PluginError::invalid_params_error(&error))?;
    next.set_fragment(None);
    Ok(Some(next))
}

async fn bounded_body(
    mut response: reqwest::Response,
    maximum: usize,
) -> SdkResult<(Vec<u8>, bool)> {
    let mut body = Vec::with_capacity(maximum.min(64 * 1024));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| PluginError::internal_error(&error.without_url()))?
    {
        let remaining = maximum.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            return Ok((body, true));
        }
    }
    Ok((body, false))
}

async fn robots<F, Fut>(
    target: &url::Url,
    options: &SpiderFetchOptions,
    resolve: &F,
) -> SdkResult<Option<String>>
where
    F: Fn(url::Url) -> Fut,
    Fut: Future<Output = SdkResult<Vec<SocketAddr>>>,
{
    let mut url = target
        .join("/robots.txt")
        .map_err(|error| PluginError::invalid_params_error(&error))?;
    url.set_query(None);
    let mut visited = BTreeSet::new();
    for _ in 0..=REDIRECTS {
        if !visited.insert(url.to_string()) {
            break;
        }
        let response = send(&url, options, resolve).await?;
        if let Some(next) = redirect(&response)? {
            url = next;
            continue;
        }
        let status = response.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(PluginError::invalid_params(
                "robots.txt denies access to this origin",
            ));
        }
        if status.is_client_error() {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(PluginError::internal(format!(
                "robots.txt returned HTTP {}; fetch did not continue",
                status.as_u16()
            )));
        }
        let (bytes, truncated) = bounded_body(response, ROBOTS_BYTES).await?;
        if truncated {
            return Err(PluginError::internal(
                "robots.txt exceeds the 512 KiB parsing budget; fetch did not continue",
            ));
        }
        return Ok(Some(String::from_utf8_lossy(&bytes).into_owned()));
    }
    Err(PluginError::internal(
        "robots.txt redirect loop or redirect budget exceeded",
    ))
}

pub(super) async fn fetch<F, Fut>(
    initial: &url::Url,
    options: &SpiderFetchOptions,
    resolve: F,
) -> SdkResult<Source>
where
    F: Fn(url::Url) -> Fut,
    Fut: Future<Output = SdkResult<Vec<SocketAddr>>>,
{
    let operation = async {
        let mut url = initial.clone();
        let mut visited = BTreeSet::new();
        let mut policies = BTreeMap::new();
        for _ in 0..=REDIRECTS {
            if !visited.insert(url.to_string()) {
                break;
            }
            if options.respect_robots_txt {
                let origin = url.origin().ascii_serialization();
                if !policies.contains_key(&origin) {
                    policies.insert(origin.clone(), robots(&url, options, &resolve).await?);
                }
                if let Some(Some(parser)) = policies.get(&origin)
                    && !agena_web::robots_allows(parser, &options.user_agent, url.as_str())
                {
                    return Err(PluginError::invalid_params(
                        "robots.txt disallows the requested page",
                    ));
                }
            }
            let response = send(&url, options, &resolve).await?;
            if let Some(next) = redirect(&response)? {
                url = next;
                continue;
            }
            let status = response.status().as_u16();
            let header = |name| {
                response
                    .headers()
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned)
            };
            let content_type = header(reqwest::header::CONTENT_TYPE).unwrap_or_default();
            let etag = header(reqwest::header::ETAG);
            let last_modified = header(reqwest::header::LAST_MODIFIED);
            let (bytes, truncated) = bounded_body(response, options.max_body_bytes).await?;
            let (body, decoding_errors) = agena_web::decode_response_body(&bytes, &content_type);
            if body.contains('\0') || bytes.starts_with(b"%PDF-") {
                return Err(PluginError::invalid_params(
                    "page body is binary; download it explicitly and use an appropriate document reader",
                ));
            }
            return Ok(Source {
                final_url: url,
                status,
                content_type,
                body,
                truncated,
                etag,
                last_modified,
                decoding_errors,
            });
        }
        Err(PluginError::internal(
            "fetch redirect loop or ten-redirect budget exceeded",
        ))
    };
    tokio::time::timeout(options.timeout, operation)
        .await
        .map_err(|_| {
            PluginError::internal(
                "fetch deadline exceeded, including robots, DNS, redirects and body streaming",
            )
        })?
}

pub(super) async fn extract(
    source: Source,
    requested: url::Url,
    backend: agena_web::ExtractionBackend,
) -> SdkResult<FetchedPage> {
    let permit = crate::BLOCKING_PLUGIN_WORKERS
        .acquire()
        .await
        .map_err(|error| PluginError::internal_error(&error))?;
    let decoding_errors = source.decoding_errors;
    let (mut page, body) = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let page = agena_web::extract_page_from_body(
            &requested,
            &source.final_url,
            &source.content_type,
            source.status,
            source.truncated,
            false,
            &source.body,
            source.etag,
            source.last_modified,
        );
        (page, source.body)
    })
    .await
    .map_err(|error| PluginError::internal_error(&error))?;
    if decoding_errors {
        page.warnings.push(
            "Response decoding replaced invalid byte sequences (possibly at a truncated boundary)."
                .into(),
        );
    }
    agena_web::extract_with_backend(page, &body, backend)
        .await
        .map_err(crawl_error_to_plugin)
}

#[cfg(test)]
mod tests;
