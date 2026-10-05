//! Classify search responses before interpreting missing cards as empty results.

use super::*;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchIssueKind {
    RateLimited,
    VerificationRequired,
    ConsentRequired,
    JavascriptRequired,
    UnexpectedPage,
    HttpError,
    TransportError,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIssue {
    pub kind: SearchIssueKind,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
}

impl fmt::Display for SearchIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some(seconds) = self.retry_after_secs {
            write!(f, "; retry after {seconds}s, use another engine meanwhile")?;
        }
        Ok(())
    }
}

impl SearchIssue {
    pub(super) fn new(kind: SearchIssueKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            http_status: None,
            retry_after_secs: None,
        }
    }

    pub(super) fn error(self, engine: WebSearchEngine) -> CrawlError {
        CrawlError::SearchUnavailable {
            provider: engine.label(),
            issue: self,
        }
    }

    pub(super) fn from_error(error: CrawlError) -> Self {
        match error {
            CrawlError::SearchUnavailable { issue, .. } => issue,
            CrawlError::Http(error) => Self::new(
                SearchIssueKind::TransportError,
                format!(
                    "search request failed: {}",
                    agena_failure::diagnostic::format_error_chain(&error.without_url())
                ),
            ),
            other => Self::new(SearchIssueKind::TransportError, other.to_string()),
        }
    }

    pub(super) fn cooldown(&self) -> Duration {
        let seconds = self.retry_after_secs.unwrap_or(match self.kind {
            SearchIssueKind::RateLimited => 60,
            SearchIssueKind::VerificationRequired | SearchIssueKind::ConsentRequired => 300,
            SearchIssueKind::JavascriptRequired => 60,
            SearchIssueKind::UnexpectedPage => 30,
            _ => 10,
        });
        Duration::from_secs(seconds.clamp(2, 86_400))
    }
}

/// Raw search DOM, also used by the optional caller-owned browser renderer.
#[derive(Debug, Clone)]
pub struct WebSearchPage {
    pub final_url: Url,
    pub status: u16,
    pub html: String,
    pub retry_after_secs: Option<u64>,
}

pub(super) fn parse_page(
    engine: WebSearchEngine,
    page: &WebSearchPage,
) -> Result<Vec<WebSearchResult>, CrawlError> {
    let doc = Html::parse_document(&page.html);
    let fail = |kind, message: &str| {
        let mut issue = SearchIssue::new(kind, message);
        issue.http_status = Some(page.status);
        issue.retry_after_secs = page.retry_after_secs;
        issue.error(engine)
    };
    if page.status == 429 {
        return Err(fail(
            SearchIssueKind::RateLimited,
            "rate_limited: search website returned HTTP 429",
        ));
    }
    let title = doc
        .select(&selector("title"))
        .next()
        .map(|el| normalize_whitespace(&el.text().collect::<String>()).to_lowercase())
        .unwrap_or_default();
    let path = page.final_url.path();
    if path.starts_with("/showcaptcha") || path.starts_with("/sorry") || path.contains("/captcha")
        || ["百度安全验证", "are you not a robot?", "are you a robot?", "verify you are human", "robot or human?"].contains(&title.as_str())
        || doc.select(&selector(".CheckboxCaptcha, #captcha-form, form[action*='captcha'], form[action*='anomaly.js'], .g-recaptcha, .anomaly-modal, #b_captcha, #b_captchaSection, #challenge-form")).next().is_some()
    {
        return Err(fail(SearchIssueKind::VerificationRequired, "captcha_required: search website requires human verification; this is not an empty result"));
    }
    if page
        .final_url
        .host_str()
        .is_some_and(|host| host.starts_with("consent."))
        || doc
            .select(&selector(
                "form[action*='consent.google'], form[action*='consent.yahoo']",
            ))
            .next()
            .is_some()
    {
        return Err(fail(
            SearchIssueKind::ConsentRequired,
            "consent_required: search website returned a consent page",
        ));
    }
    if !(200..300).contains(&page.status) {
        return Err(fail(
            SearchIssueKind::HttpError,
            &format!(
                "search website returned HTTP {}; no usable results were received",
                page.status
            ),
        ));
    }
    if doc
        .select(&selector("a[href*='/httpservice/retry/enablejs']"))
        .next()
        .is_some()
        || doc.select(&selector("noscript")).any(|el| {
            el.text()
                .any(|text| text.contains("/httpservice/retry/enablejs"))
        })
    {
        return Err(fail(
            SearchIssueKind::JavascriptRequired,
            "javascript_required: search website requires browser rendering",
        ));
    }
    let rows = match engine {
        WebSearchEngine::Bing => parse_bing_results(&page.html, MAX_SEARCH_RESULTS),
        WebSearchEngine::DuckDuckGo => parse_duckduckgo_results(&page.html, MAX_SEARCH_RESULTS),
        WebSearchEngine::Baidu => parse_baidu_results(&page.html, MAX_SEARCH_RESULTS),
        _ => public_html::parse_results(engine, &doc, MAX_SEARCH_RESULTS),
    };
    if !rows.is_empty() {
        return Ok(rows);
    }
    let empty = selector(
        ".no-results, .no-results__message, .result--no-result, .no-results-message, .NoResults, .serp-error, #topstuff, .api_noresult_wrap, .not_found, .b_no, #b_results .b_msg, .nors, .norsSuggest",
    );
    if doc.select(&empty).any(|el| {
        let text = normalize_whitespace(&el.text().collect::<Vec<_>>().join(" ")).to_lowercase();
        [
            "no results",
            "did not match any documents",
            "nothing found",
            "ничего не нашлось",
            "没有找到",
            "未找到",
            "找不到",
            "没有与",
            "검색결과가 없습니다",
            "검색 결과가 없습니다",
        ]
        .iter()
        .any(|message| text.contains(message))
    }) {
        return Ok(Vec::new());
    }
    // Baidu can return only a JS HTTPS->HTTP downgrade shim. Never call it a
    // successful empty search or silently follow that downgrade.
    if engine == WebSearchEngine::Baidu
        && page.html.len() < 2048
        && page.html.contains("location.replace")
        && page.html.contains("http://")
    {
        return Err(fail(
            SearchIssueKind::UnexpectedPage,
            "unexpected_redirect: Baidu returned an HTTP downgrade shell instead of search results",
        ));
    }
    if engine == WebSearchEngine::Bing && path == "/" {
        return Err(fail(
            SearchIssueKind::UnexpectedPage,
            "unexpected_search_homepage: Bing returned its homepage instead of search results",
        ));
    }
    Err(fail(
        SearchIssueKind::UnexpectedPage,
        "unrecognized_results_page: no usable result cards or explicit empty state; this does not mean no matching pages exist",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(engine: WebSearchEngine, html: &str) -> WebSearchPage {
        WebSearchPage {
            final_url: Url::parse(engine.permission_url()).unwrap(),
            status: 200,
            html: html.into(),
            retry_after_secs: None,
        }
    }

    #[test]
    fn all_eight_engines_distinguish_unknown_pages_and_challenges_from_explicit_empty_results() {
        for engine in WebSearchEngine::ALL {
            for html in [
                "<html><body></body></html>",
                "<title>Search</title><form><input name=q></form>",
            ] {
                assert!(
                    parse_page(engine, &response(engine, html)).is_err(),
                    "{engine}"
                );
            }
            for html in [
                "<form id=captcha-form>Verify</form>",
                "<div class=anomaly-modal>Unfortunately bots use DuckDuckGo too.</div>",
                "<title>百度安全验证</title>",
                "<title>Are you not a robot?</title>",
            ] {
                assert!(
                    matches!(
                        parse_page(engine, &response(engine, html)),
                        Err(CrawlError::SearchUnavailable {
                            issue: SearchIssue {
                                kind: SearchIssueKind::VerificationRequired,
                                ..
                            },
                            ..
                        })
                    ),
                    "{engine}"
                );
            }
            assert!(
                parse_page(
                    engine,
                    &response(engine, "<div class=no-results>No results found</div>")
                )
                .unwrap()
                .is_empty()
            );
        }
    }

    #[test]
    fn bing_homepage_baidu_downgrade_and_google_shell_are_never_successful_empty_searches() {
        let mut bing = response(
            WebSearchEngine::Bing,
            "<form id=sb_form action=/search></form>",
        );
        bing.final_url = Url::parse("https://www.bing.com/?q=fixture").unwrap();
        assert!(
            parse_page(WebSearchEngine::Bing, &bing)
                .unwrap_err()
                .to_string()
                .contains("unexpected_search_homepage")
        );
        let baidu = response(
            WebSearchEngine::Baidu,
            r#"<script>location.replace(location.href.replace("https://", "http://"));</script>"#,
        );
        assert!(
            parse_page(WebSearchEngine::Baidu, &baidu)
                .unwrap_err()
                .to_string()
                .contains("unexpected_redirect")
        );
        let google = response(
            WebSearchEngine::Google,
            r#"<noscript><a href="/httpservice/retry/enablejs">Enable JavaScript</a></noscript>"#,
        );
        assert!(matches!(
            parse_page(WebSearchEngine::Google, &google),
            Err(CrawlError::SearchUnavailable {
                issue: SearchIssue {
                    kind: SearchIssueKind::JavascriptRequired,
                    ..
                },
                ..
            })
        ));
    }

    #[test]
    fn challenges_are_not_results_even_when_a_stale_card_is_present() {
        let html = r#"<div class=anomaly-modal>Verification</div><div class=result><a class=result__a href=https://example.com>Old result</a></div>"#;
        assert!(
            parse_page(
                WebSearchEngine::DuckDuckGo,
                &response(WebSearchEngine::DuckDuckGo, html)
            )
            .is_err()
        );
        let html = r#"<script>const translation = "captcha verification";</script><div class=result><a class=result__a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fguide">Guide to captcha</a><a class=result__snippet>Discusses verification.</a></div>"#;
        let rows = parse_page(
            WebSearchEngine::DuckDuckGo,
            &response(WebSearchEngine::DuckDuckGo, html),
        )
        .unwrap();
        assert_eq!(rows[0].url, "https://example.com/guide");
    }
}
