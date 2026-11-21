//! Pace request starts per engine while allowing slow requests to overlap.

use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};

use agena_async::permission_wait::Deadline;
use moka::{
    future::Cache,
    ops::compute::{CompResult, Op},
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore, SemaphorePermit};
use tokio::time::Instant;

use super::*;

const PAGE_DELAY: Duration = Duration::from_secs(2);
const REQUESTS_PER_ENGINE: usize = 4;
const CACHE_TTL: Duration = Duration::from_secs(120);
const CACHE_BYTES_PER_ENGINE: u64 = 4 * 1024 * 1024;
const MIN_CACHE_ENTRY_BYTES: u32 = (CACHE_BYTES_PER_ENGINE / 32) as u32;
static SEARCH_PARSERS: agena_async::BlockingPool = agena_async::BlockingPool::new(8);

#[derive(Clone, Hash, PartialEq, Eq)]
struct ClientKey {
    user_agent: String,
    route: String,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct SearchKey {
    query: String,
    limit: usize,
    max_pages: usize,
    client: ClientKey,
    allow_render: bool,
}

fn cache_weight(key: &SearchKey, response: &WebSearchResponse) -> u32 {
    let bytes = key.query.len()
        + key.client.user_agent.len()
        + key.client.route.len()
        + response
            .results
            .iter()
            .map(|row| {
                row.title.len() + row.description.len() + row.url.len() + row.source.len() + 256
            })
            .sum::<usize>()
        + 512;
    MIN_CACHE_ENTRY_BYTES.max(bytes.min(u32::MAX as usize) as u32)
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
struct EngineState {
    clients: VecDeque<(ClientKey, transport::SearchClient)>,
    cooldown: Option<(Instant, SearchIssue)>,
}

struct EngineSession {
    engine: WebSearchEngine,
    state: Mutex<EngineState>,
    next_request: AsyncMutex<Option<Instant>>,
    requests: Semaphore,
    cache: Cache<SearchKey, WebSearchResponse>,
}

impl EngineSession {
    fn new(engine: WebSearchEngine) -> Self {
        Self {
            engine,
            state: Mutex::default(),
            next_request: AsyncMutex::default(),
            requests: Semaphore::new(REQUESTS_PER_ENGINE),
            cache: Cache::builder()
                .time_to_live(CACHE_TTL)
                .max_capacity(CACHE_BYTES_PER_ENGINE)
                .weigher(cache_weight)
                .build(),
        }
    }

    fn client(
        &self,
        key: &ClientKey,
        route: &crate::WebHttpRoute,
    ) -> Result<transport::SearchClient, CrawlError> {
        let mut state = self.state.lock().expect("search engine state");
        let client =
            if let Some(position) = state.clients.iter().position(|(cached, _)| cached == key) {
                state
                    .clients
                    .remove(position)
                    .expect("cached search client")
                    .1
            } else {
                let client = transport::client(self.engine, &key.user_agent, route)?;
                if state.clients.len() >= 4 {
                    state.clients.pop_front();
                }
                client
            };
        state.clients.push_back((key.clone(), client.clone()));
        Ok(client.for_invocation(route))
    }

    fn check_cooldown(&self) -> Result<(), CrawlError> {
        let state = self.state.lock().expect("search engine state");
        if let Some((until, issue)) = &state.cooldown
            && let Some(remaining) = until.checked_duration_since(Instant::now())
        {
            let mut issue = issue.clone();
            issue.retry_after_secs = Some(remaining.as_secs().saturating_add(1));
            issue.message = format!("engine cooling down after {}", issue.message);
            return Err(issue.error(self.engine));
        }
        Ok(())
    }

    fn cool_down(&self, issue: &mut SearchIssue) {
        let delay = issue.cooldown();
        issue.retry_after_secs = Some(delay.as_secs());
        let until = Instant::now() + delay;
        let mut state = self.state.lock().expect("search engine state");
        if state
            .cooldown
            .as_ref()
            .is_none_or(|(previous, _)| *previous < until)
        {
            state.cooldown = Some((until, issue.clone()));
        }
    }

    async fn admit(&self, delay: Duration) -> Result<SemaphorePermit<'_>, CrawlError> {
        self.check_cooldown()?;
        let slot = self
            .requests
            .acquire()
            .await
            .expect("search requests are never closed");
        let mut next = self.next_request.lock().await;
        self.check_cooldown()?;
        if let Some(ready) = *next {
            tokio::time::sleep_until(ready).await;
            self.check_cooldown()?;
        }
        // Reserve the next start before connecting so cancellation cannot cause a burst.
        *next = Some(Instant::now() + delay);
        Ok(slot)
    }
}

/// The caller must authorize the selected origin before every call, including
/// cache hits. Different engines proceed concurrently; calls to one engine
/// share pacing, cookies, cooldowns and duplicate-query work.
pub struct WebSearchCoordinator {
    engines: [EngineSession; 8],
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
            engines: WebSearchEngine::ALL.map(EngineSession::new),
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
        F: Fn(transport::SearchClient, usize, Option<transport::SearchRequest>) -> FetchFuture,
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
        let deadline = Deadline::new(options.timeout);
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
        let session = &self.engines[index];
        let route = match &options.route {
            Some(route) => route.clone(),
            None => crate::WebHttpRoute::for_url(&url::Url::parse(engine.permission_url())?, None)?,
        };
        let key = SearchKey {
            query: query.to_owned(),
            limit: options.limit.clamp(1, MAX_SEARCH_RESULTS),
            max_pages: options.max_pages.clamp(1, MAX_SEARCH_PAGES),
            client: ClientKey {
                user_agent: options.user_agent.clone(),
                route: route.cache_key(),
            },
            allow_render,
        };
        if let Some(mut response) = session.cache.get(&key).await {
            response.cache_hit = true;
            return Ok(response);
        }
        // Only identical searches wait for one another. Failures and partial
        // results belong to the invoking call, including permission failures.
        let mut uncached = None;
        let outcome_slot = &mut uncached;
        let query_key = &key;
        let query_deadline = &deadline;
        let result = deadline
            .run(session.cache.entry_by_ref(&key).and_compute_with(|entry| {
                Box::pin(async move {
                    if entry.is_some() {
                        return Op::Nop;
                    }
                    match self
                        .retrieve(session, query_key, &route, query_deadline, fetch, render)
                        .await
                    {
                        Ok(response)
                            if !response.results.is_empty() && response.warnings.is_empty() =>
                        {
                            Op::Put(response)
                        }
                        outcome => {
                            *outcome_slot = Some(outcome);
                            Op::Nop
                        }
                    }
                })
            }))
            .await
            .map_err(|_| timeout().error(engine))?;
        match result {
            CompResult::Unchanged(entry) => {
                let mut response = entry.into_value();
                response.cache_hit = true;
                Ok(response)
            }
            CompResult::Inserted(entry) => Ok(entry.into_value()),
            CompResult::StillNone(_) => uncached.expect("uncached search outcome"),
            _ => unreachable!("search cache entries are only inserted"),
        }
    }

    async fn retrieve<F, FetchFuture, R, RenderFuture>(
        &self,
        session: &EngineSession,
        key: &SearchKey,
        route: &crate::WebHttpRoute,
        deadline: &Deadline,
        fetch: F,
        render: R,
    ) -> Result<WebSearchResponse, CrawlError>
    where
        F: Fn(transport::SearchClient, usize, Option<transport::SearchRequest>) -> FetchFuture,
        FetchFuture: Future<Output = Result<WebSearchPage, CrawlError>>,
        R: Fn(url::Url, Duration) -> RenderFuture,
        RenderFuture: Future<Output = Result<Option<WebSearchPage>, CrawlError>>,
    {
        let engine = session.engine;
        session.check_cooldown()?;
        let client = session.client(&key.client, route)?;
        let timeout = || {
            SearchIssue::new(
                SearchIssueKind::Timeout,
                "search deadline exceeded, including queueing, pacing and all pages",
            )
            .error(engine)
        };
        let mut response = WebSearchResponse::default();
        let mut seen = HashSet::new();
        let mut continuation = None;
        for page_index in 0..key.max_pages {
            let mut contacted = false;
            let operation = async {
                let slot = session.admit(self.page_delay).await?;
                contacted = true;
                let page = fetch(client.clone(), page_index, continuation.take()).await?;
                let final_url = page.final_url.clone();
                let want_next = page_index + 1 < key.max_pages;
                let parsed = parse_page(engine, page, want_next).await;
                drop(slot);
                if key.allow_render
                    && matches!(&parsed, Err(CrawlError::SearchUnavailable { issue, .. }) if issue.kind == SearchIssueKind::JavascriptRequired)
                {
                    contacted = false;
                    let _slot = session.admit(self.page_delay).await?;
                    contacted = true;
                    if let Some(rendered) = render(final_url, deadline.remaining()).await? {
                        response.rendered = true;
                        if rendered.html.len() > MAX_SEARCH_RESPONSE_BYTES {
                            return Err(CrawlError::ResponseTooLarge {
                                maximum_bytes: MAX_SEARCH_RESPONSE_BYTES,
                            });
                        }
                        return parse_page(engine, rendered, want_next).await;
                    }
                }
                parsed
            };
            let outcome = deadline
                .run(operation)
                .await
                .unwrap_or_else(|_| Err(timeout()));
            let rows = match outcome {
                Ok((rows, next)) => {
                    continuation = next;
                    rows
                }
                Err(error @ CrawlError::AccessRejected(_)) => return Err(error),
                Err(error) => {
                    let mut issue = SearchIssue::from_error(error);
                    if contacted && issue.kind != SearchIssueKind::Timeout {
                        session.cool_down(&mut issue);
                    }
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
                    if response.results.len() >= key.limit {
                        break;
                    }
                }
            }
            if response.results.len() >= key.limit
                || response.results.len() == before
                || (engine == WebSearchEngine::DuckDuckGo && continuation.is_none())
            {
                break;
            }
        }
        Ok(response)
    }
}

async fn parse_page(
    engine: WebSearchEngine,
    page: WebSearchPage,
    want_next: bool,
) -> Result<(Vec<WebSearchResult>, Option<transport::SearchRequest>), CrawlError> {
    SEARCH_PARSERS
        .run(move || status::parse_page(engine, &page, want_next))
        .await
        .map_err(|error| CrawlError::Io(std::io::Error::other(error)))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::Notify;

    fn page() -> WebSearchPage {
        WebSearchPage {
            final_url: url::Url::parse(BING_BASE).unwrap(),
            status: 200,
            html: "<ol id='b_results'><li class='b_algo'><h2><a href='https://example.org/'>Example</a></h2></li></ol>".into(),
            retry_after_secs: None,
        }
    }

    fn coordinator() -> WebSearchCoordinator {
        WebSearchCoordinator {
            page_delay: Duration::from_millis(35),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn identical_searches_coalesce_and_another_route_does_its_own_fetch() {
        let coordinator = coordinator();
        let first_route =
            crate::WebHttpRoute::default().with_addresses(vec!["192.0.2.1:443".parse().unwrap()]);
        let options = WebSearchOptions {
            route: Some(first_route),
            ..Default::default()
        };
        let requested = AtomicUsize::new(0);
        let fetch = |_, _, _| async {
            requested.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(page())
        };
        let render = |_, _| async { Ok(None) };
        let responses = futures_util::future::join_all(
            (0..16).map(|_| coordinator.run("fixture", &options, false, fetch, render)),
        )
        .await;
        assert_eq!(
            responses
                .into_iter()
                .filter(|response| !response.as_ref().unwrap().cache_hit)
                .count(),
            1
        );
        assert_eq!(requested.load(Ordering::Relaxed), 1);
        let other = WebSearchOptions {
            route: Some(
                crate::WebHttpRoute::default()
                    .with_addresses(vec!["192.0.2.2:443".parse().unwrap()]),
            ),
            ..options
        };
        assert!(
            !coordinator
                .run("fixture", &other, false, fetch, render)
                .await
                .unwrap()
                .cache_hit
        );
        assert_eq!(requested.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn slow_search_does_not_hold_the_engine_until_its_response_arrives() {
        let coordinator = coordinator();
        let options = WebSearchOptions {
            timeout: Duration::from_secs(2),
            route: Some(crate::WebHttpRoute::default()),
            ..Default::default()
        };
        let started = Notify::new();
        let released = Notify::new();
        let render = |_, _| async { Ok(None) };
        let slow = coordinator.run(
            "slow",
            &options,
            false,
            |_, _, _| async {
                started.notify_one();
                released.notified().await;
                Ok(page())
            },
            render,
        );
        let fast = async {
            started.notified().await;
            let after_slow_start = Instant::now();
            let result = coordinator
                .run(
                    "fast",
                    &options,
                    false,
                    |_, _, _| async {
                        assert!(after_slow_start.elapsed() >= Duration::from_millis(20));
                        released.notify_one();
                        Ok(page())
                    },
                    render,
                )
                .await;
            assert_eq!(result.unwrap().results.len(), 1);
        };
        let (slow, ()) = tokio::join!(slow, fast);
        assert_eq!(slow.unwrap().results.len(), 1);
    }

    #[tokio::test]
    async fn rate_limit_cooldown_prevents_another_request() {
        let coordinator = coordinator();
        let options = WebSearchOptions {
            route: Some(crate::WebHttpRoute::default()),
            ..Default::default()
        };
        let render = |_, _| async { Ok(None) };
        for query in ["first", "second"] {
            let response = coordinator
                .run(
                    query,
                    &options,
                    false,
                    |_, _, _| async {
                        assert_eq!(query, "first");
                        Ok(WebSearchPage {
                            status: 429,
                            retry_after_secs: Some(90),
                            ..page()
                        })
                    },
                    render,
                )
                .await;
            assert!(
                matches!(response, Err(CrawlError::SearchUnavailable { issue, .. }) if issue.kind == SearchIssueKind::RateLimited && issue.retry_after_secs.is_some_and(|delay| delay >= 89))
            );
        }
    }
}
