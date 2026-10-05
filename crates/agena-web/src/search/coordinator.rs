//! One shared queue and cookie jar per engine, with bounded result reuse and cooldowns.

use std::collections::VecDeque;
use std::sync::LazyLock;

use tokio::sync::Mutex;
use tokio::time::{Instant, timeout_at};

use super::*;

const PAGE_DELAY: Duration = Duration::from_secs(2);
const CACHE_TTL: Duration = Duration::from_secs(120);
const CACHE_ENTRIES_PER_ENGINE: usize = 32;
const CACHE_BYTES_PER_ENGINE: usize = 4 * 1024 * 1024;

fn cache_weight(key: &str, response: &WebSearchResponse) -> usize {
    key.len()
        + response
            .results
            .iter()
            .map(|row| {
                row.title.len() + row.description.len() + row.url.len() + row.source.len() + 256
            })
            .sum::<usize>()
        + 512
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebSearchResponse {
    pub results: Vec<WebSearchResult>,
    pub warnings: Vec<SearchIssue>,
    pub pages_fetched: usize,
    pub cache_hit: bool,
    pub rendered: bool,
}

impl From<Vec<WebSearchResult>> for WebSearchResponse {
    fn from(results: Vec<WebSearchResult>) -> Self {
        Self {
            results,
            pages_fetched: 1,
            ..Self::default()
        }
    }
}

#[derive(Default)]
struct EngineSession {
    client: Option<(String, reqwest::Client)>,
    next_request: Option<Instant>,
    cooldown: Option<(Instant, SearchIssue)>,
    cache: VecDeque<(String, Instant, WebSearchResponse)>,
}

/// The caller must authorize the selected origin before every call, including
/// cache hits. Different engines proceed concurrently; calls to one engine
/// share pacing, cookies, cooldowns and duplicate-query work.
pub struct WebSearchCoordinator {
    engines: [Mutex<EngineSession>; 8],
    page_delay: Duration,
}

impl Default for WebSearchCoordinator {
    fn default() -> Self {
        Self::new(PAGE_DELAY)
    }
}

static DEFAULT_COORDINATOR: LazyLock<WebSearchCoordinator> =
    LazyLock::new(WebSearchCoordinator::default);

pub(super) async fn search(
    query: &str,
    options: &WebSearchOptions,
) -> Result<WebSearchResponse, CrawlError> {
    DEFAULT_COORDINATOR.search(query, options).await
}

impl WebSearchCoordinator {
    pub fn new(minimum_delay: Duration) -> Self {
        Self {
            engines: std::array::from_fn(|_| Mutex::new(EngineSession::default())),
            page_delay: minimum_delay.max(PAGE_DELAY),
        }
    }

    pub async fn search(
        &self,
        query: &str,
        options: &WebSearchOptions,
    ) -> Result<WebSearchResponse, CrawlError> {
        self.search_with_renderer(query, options, false, |_, _| async { Ok(None) })
            .await
    }

    /// Render only a confirmed JavaScript shell, at most once per result page.
    /// CAPTCHA, consent and rate limiting are never automatically retried in a
    /// browser. The renderer must authorize all of its own network requests.
    pub async fn search_with_renderer<R, RenderFuture>(
        &self,
        query: &str,
        options: &WebSearchOptions,
        allow_render: bool,
        render: R,
    ) -> Result<WebSearchResponse, CrawlError>
    where
        R: Fn(url::Url, Duration) -> RenderFuture,
        RenderFuture: Future<Output = Result<Option<WebSearchPage>, CrawlError>>,
    {
        self.run(
            query,
            options,
            allow_render,
            |client, page, continuation| {
                transport::fetch(
                    client,
                    options.engine,
                    query.trim().to_owned(),
                    page,
                    continuation,
                )
            },
            render,
        )
        .await
    }

    async fn run<F, FetchFuture, R, RenderFuture>(
        &self,
        query: &str,
        options: &WebSearchOptions,
        allow_render: bool,
        fetch: F,
        render: R,
    ) -> Result<WebSearchResponse, CrawlError>
    where
        F: Fn(reqwest::Client, usize, Option<transport::SearchRequest>) -> FetchFuture,
        FetchFuture: Future<Output = Result<WebSearchPage, CrawlError>>,
        R: Fn(url::Url, Duration) -> RenderFuture,
        RenderFuture: Future<Output = Result<Option<WebSearchPage>, CrawlError>>,
    {
        let query = query.trim();
        if query.is_empty() {
            return Err(CrawlError::InvalidInput(
                "search query must not be empty".into(),
            ));
        }
        let engine = options.engine;
        let deadline = Instant::now() + options.timeout;
        let timeout = || {
            SearchIssue::new(
                SearchIssueKind::Timeout,
                "search deadline exceeded, including queueing, pacing and all pages",
            )
        };
        let index = WebSearchEngine::ALL
            .iter()
            .position(|candidate| *candidate == engine)
            .unwrap();
        let mut session = timeout_at(deadline, self.engines[index].lock())
            .await
            .map_err(|_| timeout().error(engine))?;
        let limit = options.limit.clamp(1, MAX_SEARCH_RESULTS);
        let max_pages = options.max_pages.clamp(1, MAX_SEARCH_PAGES);
        let key =
            serde_json::to_string(&(query, limit, max_pages, &options.user_agent, allow_render))?;
        session.cache.retain(|(_, at, _)| at.elapsed() < CACHE_TTL);
        if let Some((_, _, response)) = session.cache.iter().find(|(cached, _, _)| cached == &key) {
            let mut response = response.clone();
            response.cache_hit = true;
            return Ok(response);
        }
        if let Some((until, issue)) = &session.cooldown {
            if *until > Instant::now() {
                let mut issue = issue.clone();
                issue.retry_after_secs = Some(
                    until
                        .duration_since(Instant::now())
                        .as_secs()
                        .saturating_add(1),
                );
                issue.message = format!(
                    "engine cooling down after {}; no new request was sent",
                    issue.message
                );
                return Err(issue.error(engine));
            }
            session.cooldown = None;
        }
        if session
            .client
            .as_ref()
            .is_none_or(|(agent, _)| agent != &options.user_agent)
        {
            session.client = Some((
                options.user_agent.clone(),
                transport::client(engine, &options.user_agent)?,
            ));
        }
        let client = session.client.as_ref().unwrap().1.clone();
        let mut response = WebSearchResponse::default();
        let mut seen = HashSet::new();
        let mut continuation = None;
        for page_index in 0..max_pages {
            let operation = async {
                if let Some(next) = session.next_request {
                    tokio::time::sleep_until(next).await;
                }
                // Set before issuing the request so cancellation cannot create
                // an immediate retry burst. The mutex drops on cancellation.
                session.next_request = Some(Instant::now() + self.page_delay);
                let page = fetch(client.clone(), page_index, continuation.take()).await?;
                if engine == WebSearchEngine::DuckDuckGo {
                    continuation = transport::duckduckgo_next(&page);
                }
                let parsed = status::parse_page(engine, &page);
                if allow_render
                    && matches!(&parsed, Err(CrawlError::SearchUnavailable { issue, .. }) if issue.kind == SearchIssueKind::JavascriptRequired)
                {
                    if let Some(next) = session.next_request {
                        tokio::time::sleep_until(next).await;
                    }
                    session.next_request = Some(Instant::now() + self.page_delay);
                    if let Some(rendered) = render(
                        page.final_url,
                        deadline.saturating_duration_since(Instant::now()),
                    )
                    .await?
                    {
                        response.rendered = true;
                        if rendered.html.len() > MAX_SEARCH_RESPONSE_BYTES {
                            return Err(CrawlError::ResponseTooLarge {
                                maximum_bytes: MAX_SEARCH_RESPONSE_BYTES,
                            });
                        }
                        return status::parse_page(engine, &rendered);
                    }
                }
                parsed
            };
            let outcome = timeout_at(deadline, operation)
                .await
                .unwrap_or_else(|_| Err(timeout().error(engine)));
            let rows = match outcome {
                Ok(rows) => rows,
                Err(error) => {
                    let mut issue = SearchIssue::from_error(error);
                    let delay = issue.cooldown();
                    issue.retry_after_secs = Some(delay.as_secs());
                    session.cooldown = Some((Instant::now() + delay, issue.clone()));
                    if response.results.is_empty() {
                        return Err(issue.error(engine));
                    }
                    response.warnings.push(issue);
                    break;
                }
            };
            response.pages_fetched += 1;
            let before = response.results.len();
            for mut row in rows {
                let Ok(url) = canonicalize_url(&row.url) else {
                    continue;
                };
                row.url = url.to_string();
                if seen.insert(row.url.clone()) {
                    // Also bounds memory used by the per-engine result cache.
                    row.title = row.title.chars().take(512).collect();
                    row.description = row.description.chars().take(2048).collect();
                    row.source = row.source.chars().take(512).collect();
                    if row.url.len() > 8192 {
                        continue;
                    }
                    response.results.push(row);
                    if response.results.len() >= limit {
                        break;
                    }
                }
            }
            if response.results.len() >= limit
                || response.results.len() == before
                || (engine == WebSearchEngine::DuckDuckGo && continuation.is_none())
            {
                break;
            }
        }
        // Do not cache blocked pages, partial pagination, or genuine empty
        // results. A transient empty state must not suppress a later retry.
        if !response.results.is_empty() && response.warnings.is_empty() {
            let weight = cache_weight(&key, &response);
            while !session.cache.is_empty()
                && (session.cache.len() >= CACHE_ENTRIES_PER_ENGINE
                    || session
                        .cache
                        .iter()
                        .map(|(key, _, response)| cache_weight(key, response))
                        .sum::<usize>()
                        + weight
                        > CACHE_BYTES_PER_ENGINE)
            {
                session.cache.pop_front();
            }
            if weight <= CACHE_BYTES_PER_ENGINE {
                session
                    .cache
                    .push_back((key, Instant::now(), response.clone()));
            }
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests;
